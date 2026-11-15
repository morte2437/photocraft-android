//! #11766: the Count tool's folder button opens the "Count Group Name" dialog, pre-filled with the
//! next group's name; OK makes the group with the typed name and makes it current, Cancel and
//! Escape make nothing. The group menu lists the groups, a separator and Rename…, which opens the
//! same dialog for the current group. The first click on the canvas still makes "Count Group 1".

use egui::{Key, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// A 400 × 300 document with one count group (made by a canvas click) and the Count tool; the
/// harness draws the options bar and the analysis dialogs.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = Tool::Count;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, egui::Modifiers::NONE);
    let mut h = Harness::builder().with_size(vec2(1600.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // The app's font families arrive with `setup_context`, after the first frame.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
            super::windows(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

fn groups(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().session.active().unwrap().doc.measurement.count_groups.iter().map(|g| g.name.clone()).collect()
}

fn history(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn dialog_name(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    let (kind, f) = h.state().ui.analysis.dialog.clone()?;
    assert_eq!(kind, crate::count_group_name_ui::KIND);
    f.get("name").and_then(|v| v.as_str()).map(String::from)
}

#[test]
fn the_first_canvas_click_still_makes_count_group_1_without_a_dialog() {
    let h = harness();
    assert_eq!(groups(&h), vec!["Count Group 1".to_string()]);
    assert!(h.state().ui.analysis.dialog.is_none());
}

#[test]
fn the_folder_button_opens_the_dialog_and_ok_makes_the_named_group() {
    let mut h = harness();
    let before = history(&h);
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert_eq!(dialog_name(&h).as_deref(), Some("Count Group 2"), "pre-filled with the next group's name");
    assert_eq!(groups(&h).len(), 1, "nothing made yet");
    // The name field has the keyboard with the whole name selected: typing replaces it.
    h.event(egui::Event::Text("Cells".into()));
    h.run_steps(2);
    assert_eq!(dialog_name(&h).as_deref(), Some("Cells"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none());
    assert_eq!(groups(&h), vec!["Count Group 1".to_string(), "Cells".to_string()]);
    let m = &h.state().session.active().unwrap().doc.measurement;
    assert_eq!(m.active_count_group, 1, "the new group is the current one");
    assert_eq!(history(&h), before + 1, "one history state");
}

#[test]
fn cancel_and_escape_make_nothing() {
    let mut h = harness();
    let before = history(&h);
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none());
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "Escape closes it");
    assert_eq!(groups(&h).len(), 1);
    assert_eq!(history(&h), before, "no history state");
}

#[test]
fn rename_opens_the_dialog_with_the_current_name_and_ok_renames() {
    let mut h = harness();
    crate::count_group_name_ui::open_rename(h.state_mut(), 0);
    h.run_steps(3);
    assert_eq!(dialog_name(&h).as_deref(), Some("Count Group 1"));
    h.event(egui::Event::Text("Nuclei".into()));
    h.run_steps(2);
    let before = history(&h);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(groups(&h), vec!["Nuclei".to_string()]);
    assert_eq!(history(&h), before + 1);
}

#[test]
fn the_group_menu_lists_rename_after_the_groups() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "Second"})).unwrap();
    h.run_steps(2);
    h.get_by_label("Group").click();
    h.run_steps(2);
    // The open list: the groups, then Rename… (after a separator).
    let rename = h.get_by_label("Rename…");
    let second = h.get_by_label("Second");
    assert!(rename.rect().top() > second.rect().top());
    rename.click();
    h.run_steps(3);
    assert_eq!(dialog_name(&h).as_deref(), Some("Second"), "Rename… renames the current group");
    assert_eq!(groups(&h).len(), 2, "picking Rename… doesn't switch groups");
}

/// CodeRabbit on #14707: the dialog belongs to the document that opened it. Switching documents
/// drops it, so OK can't rename a group of another document or name its new group, and nothing is
/// made.
#[test]
fn switching_documents_drops_the_dialog() {
    for rename in [false, true] {
        let mut h = harness();
        if rename {
            crate::count_group_name_ui::open_rename(h.state_mut(), 0);
        } else {
            h.get_by_label("Create a new count group").click();
        }
        h.run_steps(3);
        assert!(h.state().ui.analysis.dialog.is_some(), "the dialog is open (rename {rename})");
        h.event(egui::Event::Text("Wrong place".into()));
        h.state_mut().run("file.new", json!({"width": 100, "height": 100})).unwrap();
        // A count group in the new document, which the old index would name.
        h.state_mut().run("count.newGroup", json!({"name": "Mine"})).unwrap();
        h.run_steps(3);
        assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped (rename {rename})");
        assert_eq!(groups(&h), vec!["Mine".to_string()], "the other document's groups are untouched (rename {rename})");
    }
}

/// CodeRabbit on #14707: coming back to a document (or opening one that reuses a closed
/// document's id) is a new activation, so the dialog is dropped then too.
#[test]
fn leaving_a_document_and_coming_back_drops_the_dialog() {
    let mut h = harness();
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    h.state_mut().run("file.new", json!({"width": 100, "height": 100})).unwrap();
    h.state_mut().session.set_active(0);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "dropped after the round trip");
    assert_eq!(groups(&h), vec!["Count Group 1".to_string()], "nothing made");
}

/// CodeRabbit on #14707: the rename dialog targets one group; deleting an earlier one shifts the
/// indexes, so it is dropped rather than renaming the group that moved into the index.
#[test]
fn deleting_a_group_drops_the_rename_dialog() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "C"})).unwrap();
    assert_eq!(groups(&h), vec!["Count Group 1".to_string(), "B".to_string(), "C".to_string()]);
    crate::count_group_name_ui::open_rename(h.state_mut(), 1);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    h.state_mut().run("count.deleteGroup", json!({"group": 0})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped");
    assert_eq!(groups(&h), vec!["B".to_string(), "C".to_string()], "no group renamed");
}

/// CodeRabbit on #14707: groups may share a name, so the rename dialog can't tell them apart by
/// name. With `[A, B, B]` and the dialog on the first B, deleting A shifts the second B into that
/// index: any change to the group list drops the dialog.
#[test]
fn deleting_a_group_drops_the_rename_dialog_with_duplicate_names() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    assert_eq!(groups(&h), vec!["Count Group 1".to_string(), "B".to_string(), "B".to_string()]);
    crate::count_group_name_ui::open_rename(h.state_mut(), 1);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    h.state_mut().run("count.deleteGroup", json!({"group": 0})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped");
    assert_eq!(groups(&h), vec!["B".to_string(), "B".to_string()], "no group renamed");
}

/// CodeRabbit on #14707: closing the document the dialog was opened on and opening another drops
/// it (a new activation, whatever id the new document gets).
#[test]
fn closing_the_document_and_opening_another_drops_the_dialog() {
    let mut h = harness();
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    h.state_mut().session.close(0);
    h.state_mut().run("file.new", json!({"width": 100, "height": 100})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "Mine"})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped");
    assert_eq!(groups(&h), vec!["Mine".to_string()], "the new document's groups are untouched");
}

/// CodeRabbit on #14707: two control requests that run before the next frame (delete the first of
/// `[B, B]`, add another `B`) leave the same names with a different group at index 0, which a list
/// of names can't tell. Any change to the document drops the rename dialog.
#[test]
fn two_queued_edits_that_restore_the_names_still_drop_the_rename_dialog() {
    let mut h = harness();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.state_mut().run("count.deleteGroup", json!({"group": 0})).unwrap();
    h.run_steps(2);
    crate::count_group_name_ui::open_rename(h.state_mut(), 0);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some(), "the dialog is open on [B, B]");
    h.state_mut().run("count.deleteGroup", json!({"group": 0})).unwrap();
    h.state_mut().run("count.newGroup", json!({"name": "B"})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped though the names are [B, B] again");
}

/// CodeRabbit on #14707: the same document closed and reopened under the same id (an activation
/// the id can't tell) drops the dialog.
#[test]
fn reopening_the_document_under_the_same_id_drops_the_dialog() {
    let mut h = harness();
    h.get_by_label("Create a new count group").click();
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_some());
    let closed = h.state_mut().session.close(0).unwrap();
    let id = closed.doc.id;
    h.state_mut().session.add_document((*closed.doc).clone(), None);
    assert_eq!(h.state().session.active().unwrap().doc.id, id, "the reopened document has the closed one's id");
    let before = groups(&h);
    h.run_steps(3);
    assert!(h.state().ui.analysis.dialog.is_none(), "the dialog is dropped");
    assert_eq!(groups(&h), before);
}
