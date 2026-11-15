//! The Count tool's "Count Group Name" dialog (#11766): the options bar's folder button opens it to
//! name a new count group, and the group menu's Rename… opens it to rename the current one, as in
//! Photoshop. It is one of the analysis tools' small dialogs (`analysis_ui::dialog`): OK runs
//! `count.newGroup` or `count.setGroup` with the typed name, Cancel and Escape run nothing.

use egui::{RichText, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The analysis dialog kind.
pub const KIND: &str = "countGroupName";

/// The dialog's title, before translation.
pub const TITLE: &str = "Count Group Name";

/// Opens the dialog for a new group, pre-filled with the name the next group gets
/// ("Count Group N", N = the groups so far + 1, as `count.newGroup` names it).
pub fn open_new(app: &mut PhotocraftApp) {
    let n = app.session.active().map_or(0, |d| d.doc.measurement.count_groups.len());
    let name = format!("{} {}", tl!("Count Group"), n + 1);
    open(app, json!({"mode": "new", "name": name}));
}

/// Opens the dialog to rename count group `group`, pre-filled with its name.
pub fn open_rename(app: &mut PhotocraftApp, group: usize) {
    let Some(name) = app.session.active().and_then(|d| d.doc.measurement.count_groups.get(group)).map(|g| g.name.clone()) else { return };
    open(app, json!({"mode": "rename", "group": group, "name": name}));
}

fn open(app: &mut PhotocraftApp, fields: Value) {
    let mut fields = fields.as_object().cloned().unwrap_or_default();
    // The document activation that opened it: the group index and the default name mean something
    // only there. A switch to another document (and back, or a reopened one with a reused id)
    // makes a new activation.
    fields.insert("gen".into(), json!(app.session.active_generation()));
    // The document's change marker: renaming targets a group by its index, which any later change
    // to the document (a group deleted, another added) can move to a different group, even with
    // the same names. It advances with every command, however many run before a frame.
    fields.insert("rev".into(), json!(app.session.active().map(|d| d.revision)));
    app.ui.analysis.dialog = Some((KIND.to_string(), fields));
}

/// Is the dialog out of date: has another document been activated since it opened, or (renaming)
/// has the document changed at all, which can move the group it targets? Then it is dropped, with
/// whatever was typed, so OK can't name a group of the wrong document or rename the wrong group.
pub fn stale(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    if f.get("gen").and_then(Value::as_u64) != Some(app.session.active_generation()) {
        return true;
    }
    f.get("mode").and_then(Value::as_str) == Some("rename") && f.get("rev").and_then(Value::as_u64) != app.session.active().map(|d| d.revision)
}

/// The dialog's body: the name field, focused with its text selected when the dialog opens.
/// Returns Escape as Cancel (`Some(false)`) and Enter in the field as OK (`Some(true)`).
pub fn body(ui: &mut Ui, f: &mut Map<String, Value>) -> Option<bool> {
    let mut result = None;
    let t = Tokens::get(ui.ctx());
    let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.horizontal(|ui| {
        let label = ui.add_sized(vec2(120.0, 18.0), egui::Label::new(RichText::new(tl!("Count Group Name:")).color(t.text_dim).size(11.0)));
        let mut out = egui::TextEdit::singleline(&mut name).desired_width(200.0).show(ui);
        let resp = (*out.response).clone().labelled_by(label.id);
        // The first frame: focus the field and select the whole name, as Photoshop opens it.
        if !f.contains_key("__selected") {
            resp.request_focus();
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()));
            out.state.cursor.set_char_range(Some(all));
            out.state.store(ui.ctx(), resp.id);
            f.insert("__selected".into(), json!(true));
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            result = Some(true);
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Some(false);
    }
    f.insert("name".into(), json!(name));
    result
}

/// The command OK runs: a new group with the typed name, or the current group renamed.
pub fn command(f: &Map<String, Value>) -> Option<(&'static str, Value)> {
    let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
    match f.get("mode").and_then(Value::as_str) {
        Some("rename") => Some(("count.setGroup", json!({"group": f.get("group"), "name": name}))),
        _ => Some(("count.newGroup", json!({"name": name}))),
    }
}
