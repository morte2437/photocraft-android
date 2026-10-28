use super::{JobEvent, JobOutcome, PhotocraftApp, Started, Value, json, prefs};

/// View › Rulers follows `unitsAndRulers.showRulers` when the preference changes (at launch, from
/// `prefs.set` or a reset), so documents open with the rulers as last set.
pub(super) fn sync_rulers(app: &mut PhotocraftApp) {
    let want = app.session.prefs().units_and_rulers.show_rulers;
    if app.prefs_rt.show_rulers != Some(want) {
        app.ui.extras.rulers = want;
        app.prefs_rt.show_rulers = Some(want);
    }
}

/// View › Rulers was toggled: remember it for the next launch (Photoshop keeps the last state).
pub(crate) fn remember_rulers(app: &mut PhotocraftApp, on: bool) {
    if app.session.prefs().units_and_rulers.show_rulers != on {
        app.session.prefs.edit(|p| p.units_and_rulers.show_rulers = on);
    }
    app.prefs_rt.show_rulers = Some(on);
}

// ------------------------------------------------------------------ lifecycle

/// Load saved preferences once, when the app is created. Recovery starts during frame upkeep.
pub fn load(app: &mut PhotocraftApp) {
    if app.prefs_rt.loaded {
        return;
    }
    app.prefs_rt.loaded = true;
    let mut invalid = Vec::new();
    let mut stored = None;
    if let Some(text) = app.services.load_prefs.as_mut().and_then(|f| f()) {
        match app.session.load_prefs_json(&text) {
            Ok(found) => {
                invalid = found;
                stored = serde_json::from_str::<Value>(&text).ok();
            }
            Err(e) => {
                let template = "Preferences were reset: {error}";
                app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
            }
        }
    }
    crate::dock::restore(app);
    crate::brush_picker::restore(app);
    crate::last_brush::restore(app);
    sync_rulers(app);
    crate::tool_memory::restore(app);
    crate::view_cmds::restore(app);
    app.sync_recent();
    app.prefs_rt.saved_rev = app.session.prefs.rev();
    let loaded = app.session.prefs_value();
    app.prefs_rt.saved_value = Some(match stored {
        Some(stored) => stored_base(&loaded, &stored),
        None => loaded,
    });
    if !invalid.is_empty() {
        repair_invalid(app, &invalid);
    }
}

/// The save base after loading `stored`: what was loaded, with the keys the shift tool switch
/// migration may change (#5647) put back as the file had them, the way [`repair_invalid`] puts
/// back unreadable values. The migrated value and the new version then differ from the base, so
/// the next save writes them rather than keeping the old value under a newer version. Every other
/// key keeps its loaded value, so the three-way merge leaves another instance's changes alone,
/// also for keys the file doesn't have yet. A file from a newer version keeps its version.
fn stored_base(loaded: &Value, stored: &Value) -> Value {
    let mut base = loaded.clone();
    let Some(obj) = base.as_object_mut() else {
        return base;
    };
    match stored.get("version") {
        Some(v) if v.as_u64().is_some_and(|v| v > prefs::PREFS_VERSION) => {}
        Some(v) => {
            obj.insert("version".into(), v.clone());
        }
        None => {
            obj.remove("version");
        }
    }
    if let (Some(v), Some(tools)) = (stored.pointer("/tools/useShiftKeyForToolSwitch"), obj.get_mut("tools").and_then(Value::as_object_mut)) {
        tools.insert("useShiftKeyForToolSwitch".into(), v.clone());
    }
    base
}

/// Some saved values couldn't be read and took their defaults (#6611): say which, and write
/// the defaults over them on the next save so the next launch reads a clean file. The save
/// base holds the unreadable values, so the three-way merge sees them as changed here and
/// replaces (or removes) exactly those, keeping everything else in the stored file.
fn repair_invalid(app: &mut PhotocraftApp, invalid: &[prefs::InvalidPref]) {
    if let Some(base) = app.prefs_rt.saved_value.as_mut() {
        for bad in invalid {
            let Some((key, parent)) = bad.keys.split_last() else {
                continue;
            };
            if let Some(obj) = parent.iter().try_fold(&mut *base, |cur, k| cur.get_mut(k.as_str())).and_then(Value::as_object_mut) {
                obj.insert(key.clone(), bad.value.clone());
            }
        }
    }
    app.session.prefs.edit(|_| ());
    let paths = invalid.iter().map(prefs::InvalidPref::path).collect::<Vec<_>>().join(", ");
    let (title, line) = (
        "Some preferences were reset",
        "The preferences file held values PhotoCraft can't read for {paths}. Those settings are back to their defaults; the rest of your preferences loaded.",
    );
    // Notices keep their English source and are translated when drawn.
    let id = crate::notices::post(app, title, vec![line.into()], false, None);
    if let Some(notice) = app.ui.notices.iter_mut().find(|n| n.id == id) {
        notice.args.push(("paths".into(), paths));
    }
}

/// Discover recovery entries after construction, then decode one at a time without blocking
/// the window. Only the completed job's apply step touches the session.
pub(super) fn recovery(app: &mut PhotocraftApp) {
    if !app.prefs_rt.recovery_started {
        app.prefs_rt.recovery_started = true;
        if app.session.prefs().file_handling.recover_on_launch
            && let Some(recover) = app.services.recover.as_mut()
        {
            app.prefs_rt.recovery_queue.extend(recover());
        }
    }
    start_next_recovery(app);
}

fn start_next_recovery(app: &mut PhotocraftApp) {
    while app.prefs_rt.recovery_pending.is_none() {
        let Some(r) = app.prefs_rt.recovery_queue.pop_front() else { return };
        let label = format!("Recovering {}", r.name);
        let started = app.session.start_job(
            "file.recover",
            json!({}),
            &label,
            false,
            move |ctx| {
                ctx.check()?;
                ctx.progress(0.0, "Loading recovery data");
                let doc = (r.load)().map_err(photocraft_engine::EngineError::Other)?;
                ctx.check()?;
                Ok(doc)
            },
            move |s, doc| {
                let active = s.active().map(|st| st.doc.id);
                let first = s.documents().is_empty();
                s.add_document(doc, r.path);
                let st = s.active_mut().ok_or(photocraft_engine::EngineError::NoDocument)?;
                st.saved_revision = 0;
                let (id, revision) = (st.doc.id, st.revision);
                if let Some(index) = active.and_then(|id| s.documents().iter().position(|st| st.doc.id == id)) {
                    s.set_active(index);
                }
                Ok(json!({"documentId": id.0, "revision": revision, "firstDocument": first}))
            },
        );
        match started {
            Ok(Started::Job(job)) => app.prefs_rt.recovery_pending = Some((job, r.key)),
            Ok(Started::Done(v)) => finish_recovery(app, &r.key, &v),
            Err(e) => crate::notices::error(app, format!("{label}: {e}")),
        }
    }
}

fn finish_recovery(app: &mut PhotocraftApp, key: &str, v: &Value) {
    let recovered = v
        .get("documentId")
        .and_then(Value::as_u64)
        .zip(v.get("revision").and_then(Value::as_u64))
        .and_then(|(id, revision)| app.session.documents().iter().find(|st| st.doc.id.0 == id).map(|st| (st.doc.id, revision, st.doc.name.clone())));
    let Some((id, revision, name)) = recovered else {
        crate::notices::error(app, "The recovered document is no longer open".into());
        return;
    };
    // Track the admitted snapshot's revision, even if another job has already edited it.
    app.prefs_rt.autosaved.insert(id, revision);
    if let Some(adopt) = app.services.adopt_autosave.as_mut() {
        adopt(id.0, key);
    }
    if v.get("firstDocument").and_then(Value::as_bool) == Some(true) {
        app.ui.chrome.home = None;
    }
    app.sync_views();
    let template = "Recovered {name}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    app.ui.status_error = false;
}

/// Recovery uses the existing job polling/progress/cancel path. Its successful result adopts
/// the original entry on the UI thread, using identity rather than a mutable tab position.
pub(crate) fn on_recovery_event(app: &mut PhotocraftApp, e: &JobEvent) -> bool {
    if !app.prefs_rt.recovery_pending.as_ref().is_some_and(|(job, _)| *job == e.id) {
        return false;
    }
    let Some((_, key)) = app.prefs_rt.recovery_pending.take() else { return false };
    match &e.outcome {
        JobOutcome::Done(v) => finish_recovery(app, &key, v),
        JobOutcome::Failed(err) => crate::notices::error(app, format!("{}: {err}", e.label)),
        JobOutcome::Cancelled => {
            // Native bundle decoding cannot stop midway. Cancel admission and the rest of the
            // batch so another large decoder isn't started while this worker finishes.
            app.prefs_rt.recovery_queue.clear();
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", tl!(&e.label))]);
            app.ui.status_error = false;
        }
    }
    start_next_recovery(app);
    true
}

/// Attach the brush preset store once its background load finishes, and surface write
/// failures in the status bar.
pub(super) fn presets_store(app: &mut PhotocraftApp) {
    if let Some(rx) = &app.services.preset_store {
        match rx.try_recv() {
            Ok(opened) => {
                app.services.preset_store = None;
                let warnings = app.session.attach_preset_store(opened);
                if let Some(w) = warnings.first() {
                    app.ui.status = if warnings.len() == 1 {
                        w.clone()
                    } else {
                        crate::i18n::fmt(
                            &crate::i18n::trn(
                                crate::i18n::current(),
                                (warnings.len() - 1) as u64,
                                "{warning} (+{n} more brush preset warning)",
                                "{warning} (+{n} more brush preset warnings)",
                            ),
                            &[("warning", w)],
                        )
                    };
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.services.preset_store = None,
        }
    }
    if let Some(w) = app.session.preset_store.as_mut().map(|s| s.take_warnings()).and_then(|w| w.into_iter().next()) {
        app.ui.status = w;
    }
}
