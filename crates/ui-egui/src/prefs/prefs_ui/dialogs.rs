use super::shortcuts::shortcut_items;
use super::{BTreeMap, DialogKind, Map, PhotocraftApp, RichText, SECTIONS, Tokens, Value, json, prefs};

// ------------------------------------------------------------------ menu routing

/// Shell front ends for Edit-menu commands invoked without parameters (dialogs, pickers).
/// `None` when `id` isn't handled here.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|o| o.is_empty());
    if !empty {
        return None;
    }
    if let Some(section) = id.strip_prefix("edit.preferences.") {
        return Some(Ok(json!({"dialog": open_preferences(app, section)})));
    }
    let dialog = |d: Option<u64>| Some(d.map(|d| json!({"dialog": d})).ok_or_else(|| "not available".to_string()));
    match id {
        "edit.keyboardShortcuts" => Some(Ok(json!({"dialog": open_shortcuts(app, 0)}))),
        "edit.menus" => Some(Ok(json!({"dialog": open_shortcuts(app, 1)}))),
        "edit.toolbar" => Some(Ok(json!({"dialog": open_shortcuts(app, 2)}))),
        "edit.colorSettings" => {
            let d = crate::filter_dialog::open(app, "edit.colorSettings");
            let cur = serde_json::to_value(&app.session.color.settings).unwrap_or_default();
            // What Monitor Profile resolves to, so a fallback to sRGB is visible (#569): read the
            // displays again now, and the dialog refreshes this line as it draws.
            crate::monitor_status::read_now(app);
            let note = crate::monitor_status::note(app);
            if let Some(dm) = d.and_then(|d| app.ui.dialog_mut(d)) {
                for (k, v) in cur.as_object().into_iter().flatten() {
                    if dm.fields.contains_key(k) {
                        dm.fields.insert(k.clone(), v.clone());
                    }
                }
                dm.fields.insert("__note".into(), json!(note));
            }
            dialog(d)
        }
        "edit.fade" | "edit.findAndReplaceText" | "edit.defineBrushPreset" | "edit.defineCustomShape" | "edit.autoAlignLayers" | "edit.autoBlendLayers" => {
            if !app.session.is_enabled(id) {
                return Some(Err(photocraft_engine::commands::find(id)
                    .map_or("not available".into(), |c| format!("{} is not available right now", c.label.trim_end_matches('…')))));
            }
            let d = crate::filter_dialog::open(app, id);
            if let Some(dm) = d.and_then(|d| app.ui.dialog_mut(d)) {
                // Optional parameters the generic dialog can't express keep their defaults.
                for k in ["reference", "path"] {
                    dm.fields.remove(k);
                }
            }
            dialog(d)
        }
        // Photoshop's Content-Aware Fill workspace (caf_ui); the engine command with params
        // still runs directly.
        "edit.contentAwareFill" => Some(crate::caf_ui::open(app).map(|_| json!({"contentAwareFill": app.distort.describe()["contentAwareFill"]}))),
        // Photoshop has no dialog: the menu opens the scale box (with params: the engine).
        crate::cas_transform::COMMAND => Some(crate::cas_transform::begin(app, ctx)),
        "edit.presets.presetManager" => {
            Some(Ok(json!({"dialog": open_kind(app, "presets", "Preset Manager", json!({"kind": "brushes", "selected": 0, "newName": ""}))})))
        }
        "edit.presets.exportImportPresets" => Some(Ok(
            json!({"dialog": open_kind(app, "presetsIO", "Export/Import Presets", json!({"action": "export", "brushes": true, "customShapes": true, "tools": true}))}),
        )),
        _ => None,
    }
}

fn open_kind(app: &mut PhotocraftApp, kind: &str, label: &str, fields: Value) -> u64 {
    let mut f = Map::new();
    f.insert("__prefsui".into(), json!(kind));
    f.insert("__label".into(), json!(label));
    if let Value::Object(m) = fields {
        f.extend(m);
    }
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Is this a dialog rendered by this module?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__prefsui")
}

/// Only Preferences supports applying changes without closing the dialog.
pub fn is_preferences(fields: &Map<String, Value>) -> bool {
    fields.get("__prefsui").and_then(Value::as_str) == Some("prefs")
}

pub(super) fn preference_values(p: &prefs::Preferences) -> Map<String, Value> {
    let values = p.to_json();
    SECTIONS.iter().filter_map(|(id, _)| Some((id.to_string(), values.get(id)?.clone()))).collect()
}

/// Compare the editable sections, excluding settings managed outside Preferences.
pub fn preferences_changed(app: &PhotocraftApp, fields: &Map<String, Value>) -> bool {
    is_preferences(fields) && fields.get("values").and_then(Value::as_object).is_some_and(|values| *values != preference_values(app.session.prefs()))
}

/// Commit the working copy and keep the current section open. Failure leaves the draft intact.
pub fn apply(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    let d = app.ui.dialogs.iter().find(|d| d.id == id).ok_or_else(|| format!("no dialog {id}"))?;
    if d.kind != DialogKind::Command || !is_preferences(&d.fields) {
        return Err("Apply is only available for Preferences".into());
    }
    let fields = d.fields.clone();
    let result = confirm(app, &fields)?;
    let values = Value::Object(preference_values(app.session.prefs()));
    if let Some(d) = app.ui.dialog_mut(id) {
        // Use the validated values as the next draft, including any normalisation by prefs.set.
        d.fields.insert("values".into(), values);
    }
    Ok(result)
}

/// The size Preferences opens at: room for the widest section and about 30 settings rows. The
/// dialog's resize grip changes it, and its contents follow (#7420).
pub const PREFERENCES_SIZE: egui::Vec2 = egui::vec2(1040.0, 800.0);

/// The height of the settings area (section list and fields) when `available` is left below the
/// search field: everything but the dialog's button row (`item_spacing` + 8 pt + the buttons) and
/// a little slack, never less than a few rows.
pub(super) fn settings_height(available: f32, item_spacing: f32, button_height: f32) -> f32 {
    (available - (item_spacing + 8.0 + button_height + 4.0)).max(160.0)
}

/// Max dialog width for our dialogs. Preferences fills the width it is given (`available`, its
/// resizable area).
pub fn width(fields: &Map<String, Value>, available: f32) -> Option<f32> {
    match fields.get("__prefsui").and_then(Value::as_str)? {
        "prefs" => Some(available.max(380.0)),
        "shortcuts" => Some(720.0),
        _ => Some(460.0),
    }
}

/// Open Edit › Preferences on `section`.
pub fn open_preferences(app: &mut PhotocraftApp, section: &str) -> u64 {
    let section = if SECTIONS.iter().any(|(id, _)| *id == section) { section } else { "general" };
    let working = preference_values(app.session.prefs());
    let order = field_order(app.session.prefs(), &working);
    open_kind(app, "prefs", "Preferences", json!({"section": section, "values": working, "__order": order}))
}

/// Each section's keys in declaration order (JSON objects sort their keys; the serialised text
/// keeps the struct's order, which groups related settings like Photoshop's dialog).
fn field_order(p: &prefs::Preferences, sections: &Map<String, Value>) -> Value {
    let text = serde_json::to_string(p).unwrap_or_default();
    let mut out = Map::new();
    for (id, v) in sections {
        let Some(start) = text.find(&format!("\"{id}\":{{")) else { continue };
        let body = &text[start..];
        let end = body.find('}').unwrap_or(body.len());
        let region = &body[..end];
        let mut keys: Vec<(usize, String)> =
            v.as_object().into_iter().flatten().map(|(k, _)| (region.find(&format!("\"{k}\":")).unwrap_or(usize::MAX), k.clone())).collect();
        keys.sort();
        out.insert(id.clone(), json!(keys.into_iter().map(|(_, k)| k).collect::<Vec<_>>()));
    }
    Value::Object(out)
}

/// Open Keyboard Shortcuts and Menus on a tab (0 shortcuts, 1 menus, 2 toolbar).
pub fn open_shortcuts(app: &mut PhotocraftApp, tab: u64) -> u64 {
    let p = app.session.prefs();
    let fields = json!({
        "tab": tab,
        "filter": "",
        "overrides": p.shortcuts,
        "alternates": p.alternate_shortcuts,
        "sets": p.shortcut_sets,
        "activeSet": p.shortcut_set,
        "hidden": p.menus.hidden,
        "colors": p.menus.colors,
        "toolbarHidden": p.toolbar.hidden,
        "selected": "",
        "capture": false,
        "message": "",
    });
    open_kind(app, "shortcuts", tl!("Keyboard Shortcuts and Menus"), fields)
}

/// Open the Embedded Profile Mismatch prompt for a file waiting to open (`open_prompt`): its
/// `message`, the answer selected first (`action`), and `marker`, the field that routes OK and
/// Cancel back to `open_prompt`.
/// `names` carries the embedded and working profile names and the Color Settings policy a
/// remembered choice is saved to (`policyKey`, null for a mode without one).
pub fn open_mismatch(app: &mut PhotocraftApp, message: &str, action: &str, missing: bool, marker: &str, names: Value) -> u64 {
    let mut fields = json!({"message": message, "action": action, "missing": missing, "remember": false});
    if let (Some(f), Value::Object(n)) = (fields.as_object_mut(), names) {
        f.extend(n);
    }
    fields[marker] = json!(true);
    open_kind(app, "mismatch", tl!("Embedded Profile Mismatch"), fields)
}

pub(super) fn toolbar_tab(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut hidden: Vec<String> = f.get("toolbarHidden").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    egui::ScrollArea::vertical().max_height(380.0).id_salt("toolbar-scroll").show(ui, |ui| {
        egui::Grid::new("toolbar-grid").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for tool in crate::state::Tool::ALL {
                let name = format!("{tool:?}");
                let mut on = !hidden.contains(&name);
                crate::widgets::checkbox(ui, &mut on, tl!(tool.label()));
                if on {
                    hidden.retain(|h| *h != name);
                } else if !hidden.contains(&name) {
                    hidden.push(name);
                }
                let k = tool.key();
                ui.label(if k == '\0' { String::new() } else { k.to_string() });
                ui.end_row();
            }
        });
    });
    if ui.button(tl!("Restore Defaults")).clicked() {
        hidden.clear();
    }
    f.insert("toolbarHidden".into(), json!(hidden));
}

pub(super) fn presets_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mut kind = f.get("kind").and_then(Value::as_str).unwrap_or("brushes").to_string();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Preset Type")).color(t.text_dim));
        let opts = [("brushes".to_string(), tl!("Brushes")), ("customShapes".to_string(), tl!("Custom Shapes")), ("patterns".to_string(), tl!("Patterns"))];
        crate::widgets::dropdown(ui, "preset-kind", tl!("Type"), &mut kind, &opts, 180.0);
    });
    let list = app
        .session
        .execute("edit.presets.presetManager", json!({"kind": kind}))
        .ok()
        .and_then(|v| v.get(&kind).cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    let mut selected = f.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
    egui::ScrollArea::vertical().max_height(260.0).id_salt("preset-list").show(ui, |ui| {
        for (i, name) in list.iter().enumerate() {
            if ui.selectable_label(i == selected, name).clicked() {
                selected = i;
                f.insert("newName".into(), json!(name));
            }
        }
        if list.is_empty() {
            ui.label(RichText::new(tl!("No presets of this type.")).color(t.text_faint));
        }
    });
    let mut new_name = f.get("newName").and_then(Value::as_str).unwrap_or("").to_string();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut new_name).desired_width(200.0)).accessible_name(tl!("Name"));
        if ui.add_enabled(selected < list.len() && !new_name.trim().is_empty(), egui::Button::new(tl!("Rename"))).clicked() {
            let _ = app.run("edit.presets.presetManager", json!({"action": "rename", "kind": kind, "index": selected, "newName": new_name}));
        }
        if ui.add_enabled(selected < list.len(), egui::Button::new(tl!("Delete"))).clicked() {
            let _ = app.run("edit.presets.presetManager", json!({"action": "delete", "kind": kind, "index": selected}));
        }
        // Load Photoshop brushes (.abr) into the library.
        if kind == "brushes" && ui.button(tl!("Load…")).on_hover_text(tl!("Import brushes (.abr)")).clicked() {
            let _ = app.open_dialog_file();
        }
    });
    f.insert("kind".into(), json!(kind));
    f.insert("selected".into(), json!(selected));
    f.insert("newName".into(), json!(new_name));
}

pub(super) fn presets_io_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mut action = f.get("action").and_then(Value::as_str).unwrap_or("export").to_string();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Action")).color(t.text_dim));
        crate::widgets::dropdown(
            ui,
            "presets-io",
            tl!("Action"),
            &mut action,
            &[("export".to_string(), tl!("Export Presets")), ("import".to_string(), tl!("Import Presets"))],
            180.0,
        );
    });
    for (k, label) in [("brushes", tl!("Brushes")), ("customShapes", tl!("Custom Shapes")), ("tools", tl!("Tool Presets"))] {
        let mut on = f.get(k).and_then(Value::as_bool).unwrap_or(true);
        crate::widgets::checkbox(ui, &mut on, label);
        f.insert(k.into(), json!(on));
    }
    f.insert("action".into(), json!(action));
}

pub(super) fn mismatch_body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(f.get("message").and_then(Value::as_str).unwrap_or("")).color(t.text));
    ui.add_space(6.0);
    let mut action = f.get("action").and_then(Value::as_str).unwrap_or("preserve").to_string();
    let missing = f.get("missing").and_then(Value::as_bool).unwrap_or(false);
    let name = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("?").to_string();
    let opts: Vec<(&str, String)> = if missing {
        vec![("preserve", tl!("Leave as is (don't color manage)").into()), ("assignWorking", tl!("Assign working space").into())]
    } else {
        vec![
            ("preserve", crate::i18n::fmt(tl!("Use the embedded profile ({profile})"), &[("profile", &name("embedded"))])),
            ("convert", crate::i18n::fmt(tl!("Convert to the working space ({profile})"), &[("profile", &name("working"))])),
            ("discard", tl!("Discard the embedded profile (don't color manage)").into()),
        ]
    };
    for (v, label) in opts {
        ui.radio_value(&mut action, v.to_string(), label);
    }
    f.insert("action".into(), json!(action));
    // Remembering sets the Color Settings policy and stops asking, so it needs a mode with a policy.
    if !missing && f.get("policyKey").is_some_and(Value::is_string) {
        ui.add_space(8.0);
        let mut remember = f.get("remember").and_then(Value::as_bool).unwrap_or(false);
        crate::widgets::checkbox(ui, &mut remember, tl!("Remember this choice for future images"))
            .on_hover_text(tl!("Turn the prompt back on with Ask On Mismatch in Edit › Color Settings."));
        f.insert("remember".into(), json!(remember));
    }
}

// ------------------------------------------------------------------ confirm

/// OK on one of our dialogs.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    match f.get("__prefsui").and_then(Value::as_str).unwrap_or("") {
        "prefs" => {
            let values = f.get("values").cloned().unwrap_or(Value::Null);
            app.run("prefs.set", json!({"path": "", "value": values}))
        }
        "shortcuts" => {
            let overrides = f.get("overrides").cloned().unwrap_or(json!({}));
            // Shortcuts moved to another command are taken from their old owner (shell
            // commands included, which the engine doesn't know).
            // The map replaces every stored override, so a malformed one is an error, not "none".
            let mut ov: BTreeMap<String, String> =
                serde_json::from_value(overrides).map_err(|e| format!("overrides must map command IDs to shortcut strings: {e}"))?;
            let alternates = f.get("alternates").cloned().unwrap_or(json!({}));
            let mut alt: BTreeMap<String, Vec<String>> =
                serde_json::from_value(alternates).map_err(|e| format!("alternates must map command IDs to lists of shortcuts: {e}"))?;
            let items = shortcut_items(app);
            // Newly assigned shortcuts are checked first and take theirs from any holder, custom
            // overrides included; unchanged overrides only take theirs from defaults. A
            // command's further keys (#3665) take and lose keys the same way.
            let before = &app.session.prefs().shortcuts;
            let before_alt = &app.session.prefs().alternate_shortcuts;
            let mut todo: Vec<(String, String, bool, bool)> =
                ov.iter().filter(|(_, s)| !s.is_empty()).map(|(k, s)| (k.clone(), s.clone(), before.get(k) != Some(s), false)).collect();
            for (k, keys) in &alt {
                let new = before_alt.get(k) != Some(keys);
                todo.extend(keys.iter().map(|s| (k.clone(), s.clone(), new, true)));
            }
            todo.sort_by_key(|(_, _, new, _)| !new);
            let main_of = |ov: &BTreeMap<String, String>, id: &str, def: Option<&str>| -> Option<String> {
                match ov.get(id) {
                    Some(s) if s.is_empty() => None,
                    Some(s) => Some(s.clone()),
                    None => def.map(str::to_string),
                }
            };
            for (id, sc, new, is_alt) in todo {
                let norm = prefs::normalize_shortcut(&sc);
                let Some(norm) = norm else { continue };
                // Taken by a newer assignment?
                if (!is_alt && ov.get(&id).is_some_and(String::is_empty)) || (is_alt && alt.get(&id).is_some_and(|k| !k.contains(&norm))) {
                    continue;
                }
                for (other, _, _, def) in &items {
                    if *other == id {
                        continue;
                    }
                    let held = match ov.get(other) {
                        Some(s) if new => Some(s.as_str()),
                        Some(_) => None,
                        None => def.as_deref(),
                    };
                    if held.and_then(prefs::normalize_shortcut).as_deref() == Some(norm.as_str()) {
                        ov.insert(other.clone(), String::new());
                    }
                    // Its further keys: the edited list when this assignment is new, else only
                    // the defaults.
                    if alt.contains_key(other) && !new {
                        continue;
                    }
                    let mut p = prefs::Preferences::default();
                    if let Some(list) = alt.get(other) {
                        p.alternate_shortcuts.insert(other.clone(), list.clone());
                    }
                    let keys = p.alternates(other, main_of(&ov, other, def.as_deref()).as_deref());
                    if keys.contains(&norm) {
                        alt.insert(other.clone(), keys.into_iter().filter(|k| *k != norm).collect());
                    }
                }
            }
            let hidden = f.get("hidden").cloned().unwrap_or(json!([]));
            let colors = f.get("colors").cloned().unwrap_or(json!({}));
            let toolbar = f.get("toolbarHidden").cloned().unwrap_or(json!([]));
            let sets = f.get("sets").cloned().unwrap_or(json!({}));
            let active = f.get("activeSet").cloned().unwrap_or(json!(""));
            app.run(
                "edit.keyboardShortcuts",
                json!({"reset": true, "set": ov, "alternates": alt, "allowUnknown": true, "removeConflicts": false, "sets": sets, "activeSet": active}),
            )?;
            app.run("prefs.set", json!({"values": {"menus": {"hidden": hidden, "colors": colors}, "toolbar": {"hidden": toolbar}}}))
        }
        "presets" => Ok(Value::Null),
        "presetsIO" => {
            let kinds: Vec<&str> = ["brushes", "customShapes", "tools"].into_iter().filter(|k| f.get(*k).and_then(Value::as_bool).unwrap_or(true)).collect();
            if f.get("action").and_then(Value::as_str) == Some("import") {
                app.pick_file_bytes(move |app, name, bytes| {
                    let text = String::from_utf8(bytes).map_err(|_| format!("{name} is not a preset file"))?;
                    app.run("edit.presets.exportImportPresets", json!({"action": "import", "kinds": kinds, "data": text}))
                })
            } else {
                let out = app.run("edit.presets.exportImportPresets", json!({"action": "export", "kinds": kinds}))?;
                let text = serde_json::to_string_pretty(&out["data"]).map_err(|e| e.to_string())?;
                app.pick_save("Presets.pcpresets", move |app, path| {
                    let write = app.services.write.as_mut().ok_or("no writer configured")?;
                    write(&path, text.as_bytes())?;
                    let template = "Exported presets to {path}";
                    app.ui.status = crate::i18n::fmt(tl!(template), &[("path", &path)]);
                    Ok(json!({"path": path}))
                })
            }
        }
        // The file it asks about opens with this answer (`open_prompt`).
        "mismatch" => {
            let action = f.get("action").and_then(Value::as_str).unwrap_or("preserve");
            if f.get("remember").and_then(Value::as_bool) == Some(true) {
                return crate::open_prompt::answer_remembering(app, action);
            }
            crate::open_prompt::answer(app, Some(action))
        }
        _ => Err("unknown dialog".into()),
    }
}
