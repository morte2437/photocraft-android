use super::basic::reset_color_balance;
use super::gradient_map::{gradient_presets, is_gray, swatches};
use super::{Adjustment, EditorCx, HistSource, LayerId, PhotocraftApp, RichText, Tokens, Value, adjust_params, editor, json, needs_histogram, space_of, tone};

/// Root of an adjustment layer's Properties editor view state in egui memory.
pub fn layer_mem(id: LayerId) -> egui::Id {
    egui::Id::unique(("adjust-layer", id.0))
}

/// Kinds whose Properties edits follow Photoshop's history and Reset (measured for Color Balance,
/// Photoshop 25.4): consecutive edits of the layer make one history step, and Reset first returns
/// to the settings the layer had when the panel started showing it.
fn photoshop_session(kind: &str) -> bool {
    kind == "colorBalance"
}

/// The `coalesce` key of a layer's Properties edits (one history step while they follow each
/// other, like Photoshop's "Modify … Layer").
pub fn properties_coalesce(id: LayerId) -> String {
    format!("properties-adjustment:{}", id.0)
}

/// A layer's Properties session: document, last pass drawn, the settings the panel found, and
/// whether the panel edited them since (or since the last Reset).
type Session = (u64, u64, Value, bool);

fn session_key(id: LayerId) -> egui::Id {
    layer_mem(id).with("session")
}

/// Remember the settings `id` (in document `doc`) had when the panel started showing it: kept
/// while it is drawn pass after pass, taken anew after a pass without it (another layer,
/// document or panel state shown).
fn track_session(ctx: &egui::Context, doc: u64, id: LayerId, committed: &Value) {
    let pass = ctx.cumulative_pass_nr();
    let kept: Option<Session> = ctx.data(|d| d.get_temp(session_key(id)));
    let (start, edited) = match kept {
        Some((d, last, v, e)) if d == doc && last.saturating_add(1) >= pass => (v, e),
        _ => (committed.clone(), false),
    };
    ctx.data_mut(|d| d.insert_temp(session_key(id), (doc, pass, start, edited)));
}

fn set_edited(ctx: &egui::Context, id: LayerId, edited: bool) {
    let kept: Option<Session> = ctx.data(|d| d.get_temp(session_key(id)));
    if let Some((doc, pass, start, _)) = kept {
        ctx.data_mut(|d| d.insert_temp(session_key(id), (doc, pass, start, edited)));
    }
}

/// The settings layer `id` of document `doc` had when the Properties panel started showing it,
/// if the panel has edited them since (or since the last Reset).
pub fn session_start(ctx: &egui::Context, doc: u64, id: LayerId) -> Option<Value> {
    let kept: Option<Session> = ctx.data(|d| d.get_temp(session_key(id)));
    kept.filter(|k| k.0 == doc && k.3).map(|k| k.2)
}

/// The Properties panel's Reset for an adjustment layer whose kind follows Photoshop's session
/// ([`photoshop_session`]): the `layer.setAdjustment` params, or None for other kinds. Edited in
/// the panel since it started showing the layer (or since the last Reset): back to the settings it
/// found (Tone choice kept); otherwise the defaults (Color Balance: [`reset_color_balance`]).
/// Either way part of the layer's edit step.
pub fn properties_reset(ctx: &egui::Context, doc: u64, id: LayerId, adj: &Adjustment) -> Option<Value> {
    let kind = photocraft_engine::commands::adjustment_kind(adj);
    if !photoshop_session(kind) {
        return None;
    }
    let mut v = match session_start(ctx, doc, id) {
        Some(start) => start,
        None => {
            let mut v = adjust_params::to_params(adj);
            reset_color_balance(ctx, layer_mem(id), &mut v);
            v
        }
    };
    set_edited(ctx, id, false);
    v["layer"] = json!(id.0);
    v["coalesce"] = json!(properties_coalesce(id));
    Some(v)
}

/// The Properties-panel editor of an adjustment layer: previews live (`app.live_adjust`) while a
/// control moves and commits one `layer.setAdjustment` per gesture.
pub fn layer_editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &Adjustment) {
    let t = Tokens::get(ui.ctx());
    let kind = photocraft_engine::commands::adjustment_kind(adj);
    match adj {
        Adjustment::SelectiveColor { .. } => return crate::adjust_ui::selective_color_editor(app, ui, id, adj),
        Adjustment::ColorLookup { .. } => return crate::adjust_ui::color_lookup_editor(app, ui, id, adj),
        Adjustment::Invert => {
            ui.label(RichText::new(tl!("Invert has no settings.")).color(t.text_faint));
            return;
        }
        Adjustment::Unsupported { psd_key, .. } => {
            let template = "This adjustment ({key}) is kept as imported and can't be edited yet.";
            ui.label(RichText::new(crate::i18n::fmt(tl!(template), &[("key", psd_key)])).color(t.text_faint));
            return;
        }
        _ => {}
    }
    let committed = adjust_params::to_params(adj);
    let session = photoshop_session(kind);
    if session && let Some(doc) = app.session.active().map(|s| s.doc.id.0) {
        track_session(ui.ctx(), doc, id, &committed);
    }
    let mut values = match &app.live_adjust {
        Some((l, v)) if *l == id => v.clone(),
        _ => committed.clone(),
    };
    let gray = app.session.active().is_some_and(|s| is_gray(s.doc.mode));
    let hist = needs_histogram(kind).then(|| tone::histograms(app, HistSource::BelowLayer(id), space_of(&values)));
    let cx = EditorCx {
        mem: layer_mem(id),
        hist,
        gray,
        swatches: swatches(app),
        gradients: gradient_presets(app, kind),
        dialog: false,
        dynamic_color_sliders: app.session.prefs().interface.dynamic_color_sliders,
        isolated_curve: None,
    };
    if matches!(kind, "levels" | "curves") {
        crate::layer_picker::controls(app, ui, id);
    }
    let e = editor(ui, kind, &mut values, &cx);
    if e.import_gradients {
        crate::preset_files_ui::import_gradients_dialog(app);
    }
    if e.changed {
        app.live_adjust = Some((id, values.clone()));
    }
    if e.commit {
        if values != committed {
            let rev = app.session.active().map(|s| s.revision);
            let mut p = values;
            p["layer"] = json!(id.0);
            if session {
                p["coalesce"] = json!(properties_coalesce(id));
                set_edited(ui.ctx(), id, true);
            }
            if let Err(err) = app.run("layer.setAdjustment", p) {
                app.ui.status = err;
            }
            tone::keep_after_commit(app, id, rev);
        }
        app.live_adjust = None;
    }
}
