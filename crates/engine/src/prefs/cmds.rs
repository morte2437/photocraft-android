use super::customize::{keyboard_shortcuts, menus};
use super::validate::{SECTIONS, get_path, keyed};
use super::*;

// ------------------------------------------------------------------ commands

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(super) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn prefs_get(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).unwrap_or("");
    let v = s.prefs_view();
    get_path(&v, path).cloned().ok_or_else(|| bad("prefs.get", format!("unknown preference `{path}`")))
}

fn prefs_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "prefs.set";
    let mut changes: Vec<(String, Value)> = Vec::new();
    if let Some(path) = p.get("path").and_then(Value::as_str) {
        changes.push((path.to_string(), p.get("value").cloned().ok_or_else(|| bad(cmd, "missing `value`"))?));
    }
    if let Some(m) = p.get("values").and_then(Value::as_object) {
        changes.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if changes.is_empty() {
        return Err(bad(cmd, "give `path` and `value`, or `values: {path: value, …}`"));
    }
    // All or nothing: validate against a scratch copy first.
    let (saved_prefs, saved_color) = (s.prefs().clone(), s.color.settings.clone());
    let mut color_changed = false;
    for (path, v) in &changes {
        match s.set_pref(path, v.clone(), false) {
            Ok(touched) => color_changed |= touched,
            Err(e) => {
                s.prefs.edit(|p| *p = saved_prefs);
                s.color.settings = saved_color;
                return Err(bad(cmd, e));
            }
        }
    }
    if color_changed && let Err(e) = crate::color_cmds::validate_settings(&s.color.settings) {
        s.prefs.edit(|p| *p = saved_prefs);
        s.color.settings = saved_color;
        return Err(bad(cmd, e));
    }
    // `colorSettings.blendTextGamma` lives in the compositor too, as `edit.colorSettings` sets it.
    photocraft_compose::psblend::set_text_gamma(s.color.settings.blend_text_gamma);
    s.apply_prefs();
    let view = s.prefs_view();
    let out: Map<String, Value> = changes.iter().map(|(k, _)| (k.clone(), get_path(&view, k).cloned().unwrap_or(Value::Null))).collect();
    Ok(Value::Object(out))
}

fn prefs_reset(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").or_else(|| p.get("section")).and_then(Value::as_str);
    match path {
        Some("colorSettings") => s.color.settings = Default::default(),
        // One colour setting: `Preferences` has no `colorSettings`, so take the default
        // from `ColorSettings` and route it like `prefs.set` does.
        Some(path) if path.starts_with("colorSettings.") => {
            let defaults = serde_json::to_value(crate::color_cmds::ColorSettings::default()).map_err(|e| bad("prefs.reset", e.to_string()))?;
            let key = path.strip_prefix("colorSettings.").unwrap_or(path);
            let def = get_path(&defaults, key).cloned().ok_or_else(|| bad("prefs.reset", format!("unknown preference `{path}`")))?;
            s.set_pref(path, def, true).map_err(|e| bad("prefs.reset", e))?;
        }
        None => {
            s.color.settings = Default::default();
            s.edit_prefs(|p| p.reset(None)).map_err(|e| bad("prefs.reset", e))?;
        }
        Some(path) => {
            let mut next = s.prefs().clone();
            next.reset(Some(path)).map_err(|e| bad("prefs.reset", e))?;
            s.edit_prefs(|p| *p = next);
        }
    }
    s.prefs.edit(|_| ());
    photocraft_compose::psblend::set_text_gamma(s.color.settings.blend_text_gamma);
    s.apply_prefs();
    if path.is_some_and(|path| keyed(path).is_some()) {
        // The removed override is absent, so reading its old path would report an error.
        Ok(Value::Null)
    } else {
        prefs_get(s, &json!({"path": path.unwrap_or("")}))
    }
}

/// `edit.preferences.<section>`: the section's values (the GUI opens the dialog on it instead).
fn preferences_section(s: &mut Session, p: &Value) -> Result<Value> {
    let section = p.get("__section").and_then(Value::as_str).unwrap_or("general").to_string();
    let title = SECTIONS.iter().find(|(id, _)| *id == section).map_or("General", |(_, t)| *t);
    let values = s.prefs().get(&section).unwrap_or(Value::Null);
    Ok(json!({"section": section, "title": title, "values": values}))
}

/// Edit › Toolbar: hide tools and reorder the toolbar.
fn toolbar(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = |k: &str| -> Option<Vec<String>> { p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()) };
    s.edit_prefs(|pr| {
        if p.get("reset").and_then(Value::as_bool) == Some(true) {
            pr.toolbar = ToolbarCustomization::default();
        }
        if let Some(h) = ids("hidden") {
            pr.toolbar.hidden = h;
        }
        if let Some(o) = ids("order") {
            pr.toolbar.order = o;
        }
    });
    Ok(serde_json::to_value(&s.prefs().toolbar).unwrap_or_default())
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: always, run: $run, journal: $journal }
    };
}

macro_rules! section {
    ($id:literal, $label:literal) => {
        section!($id, $label, None)
    };
    ($id:literal, $label:literal, $shortcut:expr) => {
        CommandSpec { id: $id, label: $label, menu: &["Edit", "Preferences"], shortcut: $shortcut, params: r##"{}"##, enabled: always, run: |s, _| preferences_section(s, &json!({"__section": section_of($id)})), journal: false }
    };
}

fn section_of(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or("general")
}

/// Preference command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "prefs.get",
            "Get Preferences",
            [],
            None,
            r##"{"path":"section.key"?=everything (e.g. "performance.historyStates", "colorSettings.workingRgb")}"##,
            prefs_get,
            false
        ),
        spec!(
            "prefs.set",
            "Set Preferences",
            [],
            None,
            r##"{"path":"section.key","value":json} or {"values":{"section.key":json,…}} (validated; all or nothing)"##,
            prefs_set,
            false
        ),
        spec!("prefs.reset", "Reset Preferences", [], None, r##"{"path":"section|section.key"?=everything}"##, prefs_reset, false),
        // Photoshop: ⌘K opens Preferences › General (⌘, is Layer › Hide Layers).
        section!("edit.preferences.general", "General…", Some("Cmd+K")),
        section!("edit.preferences.interface", "Interface…"),
        section!("edit.preferences.workspace", "Workspace…"),
        section!("edit.preferences.tools", "Tools…"),
        section!("edit.preferences.historyLog", "History Log…"),
        section!("edit.preferences.fileHandling", "File Handling…"),
        section!("edit.preferences.export", "Export…"),
        section!("edit.preferences.performance", "Performance…"),
        section!("edit.preferences.scratchDisks", "Scratch Disks…"),
        section!("edit.preferences.cursors", "Cursors…"),
        section!("edit.preferences.transparencyAndGamut", "Transparency & Gamut…"),
        section!("edit.preferences.unitsAndRulers", "Units & Rulers…"),
        section!("edit.preferences.guidesGridAndSlices", "Guides, Grid & Slices…"),
        section!("edit.preferences.plugIns", "Plug-ins…"),
        section!("edit.preferences.type", "Type…"),
        section!("edit.preferences.enhancedControls", "Enhanced Controls…"),
        section!("edit.preferences.rawDefaults", "Camera Raw…"),
        section!("edit.preferences.integrations", "Integrations…"),
        spec!(
            "edit.keyboardShortcuts",
            "Keyboard Shortcuts…",
            ["Edit"],
            Some("Cmd+Alt+Shift+K"),
            r##"{"set":{"<command id>|tools.temporary.hand|zoomIn|zoomOut":"Cmd+Shift+X"|""(remove)|null(default)}?,"alternates":{"<command id>":["F3",…]|[](none)|null(default)}?,"reset":true|["<id>",…]?,"removeConflicts":bool=true,"filter":str?,"list":bool=false,"importKys":"<.kys XML>"|{"path":".kys"}?,"useSet":"<name>"|""(defaults)?,"saveSet":"<name>"?,"deleteSet":"<name>"?,"sets":{"<name>":{"shortcuts":{…},"alternateShortcuts":{…}}}?,"activeSet":"<name>"|""?,"exportKys":true|{"path":".kys"}?,"summarize":true|{"mac":bool,"path":".html"}?} (with a path the file is written and the reply has kysPath/htmlPath instead of kys/html)"##,
            keyboard_shortcuts,
            true
        ),
        spec!(
            "edit.menus",
            "Menus…",
            ["Edit"],
            Some("Cmd+Alt+Shift+M"),
            r##"{"hide":["<id>",…]?,"show":["<id>",…]?,"color":{"<id>":"red|orange|yellow|green|blue|violet|gray|none"}?,"reset":bool=false}"##,
            menus,
            true
        ),
        spec!("edit.toolbar", "Toolbar…", ["Edit"], None, r##"{"hidden":["<tool>",…]?,"order":["<tool>",…]?,"reset":bool=false}"##, toolbar, true),
    ]
}
