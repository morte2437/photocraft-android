//! History › New Snapshot (⌥-click the History panel's camera button, #2966): Photoshop's dialog
//! for `history.newSnapshot`, with the snapshot's Name and what it is taken From (the full
//! document, its layers merged, or only the current layer). The command stays directly callable
//! with the same params by agents and Actions.

use egui::{Align2, Sense, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "history.newSnapshot";
const MARK: &str = "__newSnapshot";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog on the active document, suggesting the name the camera button would give.
/// Not while a Free Transform is open: it owns the document until committed or cancelled, so the
/// camera button does nothing then either (#6087).
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    if app.ui.transform.is_some() {
        return None;
    }
    let name = photocraft_engine::history_cmds::next_snapshot_name(app.session.active()?);
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("New Snapshot"));
    f.insert("name".into(), json!(name));
    f.insert("from".into(), json!("fullDocument"));
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// Takes the snapshot. A cleared Name gets the next default name, as the camera button would give
/// (#6087); agents calling the command with an empty name still get an error.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let mut p = Map::new();
    if let Some(name) = f.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        p.insert("name".into(), json!(name));
    }
    if let Some(from) = f.get("from") {
        p.insert("from".into(), from.clone());
    }
    app.run(COMMAND, Value::Object(p))
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::hover());
    ui.painter().text(pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER, text, egui::FontId::proportional(12.0), t.text_dim);
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal(|ui| {
        label(ui, tl!("Name:"));
        let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
        // Opens on the name, selected, so typing replaces it (Photoshop).
        if !ui.is_sizing_pass() && !f.contains_key("__named") {
            r.request_focus();
            crate::field_tab::select_all(ui.ctx(), r.id, &name);
            f.insert("__named".into(), json!(true));
        }
        f.insert("name".into(), json!(name));
    });
    ui.horizontal(|ui| {
        label(ui, tl!("From:"));
        let mut from = f.get("from").and_then(Value::as_str).unwrap_or("fullDocument").to_string();
        let opts = [
            ("fullDocument".to_string(), tl!("Full Document")),
            ("mergedLayers".to_string(), tl!("Merged Layers")),
            ("currentLayer".to_string(), tl!("Current Layer")),
        ];
        if crate::widgets::dropdown(ui, "new-snapshot-from", tl!("From"), &mut from, &opts, 170.0) {
            f.insert("from".into(), json!(from));
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::PhotocraftApp;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app
    }

    /// An open Free Transform owns the document: New Snapshot does not open under it, whoever
    /// asks (#6087).
    #[test]
    fn does_not_open_during_free_transform() {
        let mut app = app();
        let ctx = egui::Context::default();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        assert!(app.ui.transform.is_some());
        assert_eq!(super::open(&mut app), None);
        assert!(app.ui.dialogs.is_empty());
        crate::transform_tool::cancel(&mut app);
        assert!(super::open(&mut app).is_some());
    }

    /// A cleared Name takes the default name on OK, as the camera button would (#6087); a typed
    /// one is trimmed. The command itself still refuses an empty name.
    #[test]
    fn a_cleared_name_gets_the_default_name() {
        let mut app = app();
        for (typed, want) in [("", "Snapshot 1"), ("   ", "Snapshot 2"), ("  Mine ", "Mine")] {
            let id = super::open(&mut app).unwrap();
            let d = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
            d.fields.insert("name".into(), json!(typed));
            crate::dialogs::confirm(&mut app, id).unwrap();
            assert_eq!(app.session.active().unwrap().snapshots.last().unwrap().name, want);
        }
        assert!(app.run(super::COMMAND, json!({"name": ""})).is_err(), "agents still get the error");
    }
}
