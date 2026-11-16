//! #14326: in Edit › Keyboard Shortcuts, choosing another set while the current one has unsaved
//! edits asks "Save your changes before switching sets?" with Yes / No / Cancel, instead of
//! dropping the edits.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The dialog over saved set A (⌘C on F9), which is in use.
fn dialog() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.copy": "F9"}, "saveSet": "A"})).unwrap();
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, name: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.get(name).cloned().unwrap_or(Value::Null)
}

fn set_field(h: &mut Harness<'_, PhotocraftApp>, id: u64, name: &str, v: Value) {
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(name.into(), v);
    h.run_steps(2);
}

/// The first button with `label`: the set row's come before the dialog's own (its Cancel).
fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, label).next().expect("the button").click();
    h.run_steps(3);
}

/// The Set dropdown's shown label.
fn shown_set(h: &Harness<'_, PhotocraftApp>) -> String {
    let at = h.query_all_by_label("Set:").next().expect("the Set label").rect().center();
    let combo = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(at).total_cmp(&b.rect().center().distance(at)))
        .expect("the Set dropdown");
    let node = combo.accesskit_node();
    node.value().or(node.label()).map(|s| s.to_string()).unwrap_or_default()
}

/// Picks `label` in the Set dropdown, as the user does: open it, click the option.
fn pick_set(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.run_steps(6);
    let at = h.query_all_by_label("Set:").next().expect("the Set label").rect().center();
    let at = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(at).total_cmp(&b.rect().center().distance(at)))
        .expect("the Set dropdown")
        .rect()
        .center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
    let option = h
        .query_all_by_label(label)
        .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Button)
        .min_by(|a, b| a.rect().center().distance(at).total_cmp(&b.rect().center().distance(at)));
    option.expect("the set in the open list").click();
    h.run_steps(3);
}

fn asking(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.query_by_label(super::SWITCH_SET_QUESTION).is_some()
}

/// Set A in use, edited (⌘C on F8), then the defaults chosen in the dropdown.
fn edited_then_switching() -> (Harness<'static, PhotocraftApp>, u64) {
    let (mut h, id) = dialog();
    assert_eq!(shown_set(&h), "A");
    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8"}));
    assert_eq!(shown_set(&h), "A (modified)");
    pick_set(&mut h, "PhotoCraft Defaults");
    (h, id)
}

#[test]
fn switching_from_a_modified_set_asks_and_cancel_keeps_the_edits() {
    let (mut h, id) = edited_then_switching();
    assert!(asking(&h), "the question, with exactly the reference text");
    assert_eq!(
        super::SWITCH_SET_QUESTION,
        "Selecting a different set will result in a loss of all unsaved shortcuts.  Save your changes before switching sets?"
    );
    for label in ["Yes", "No", "Cancel"] {
        assert!(h.query_all_by_role_and_label(egui::accesskit::Role::Button, label).next().is_some(), "{label}");
    }
    assert_eq!(field(&h, id, "activeSet"), json!("A"), "nothing switched yet");
    assert_eq!(field(&h, id, "overrides"), json!({"edit.copy": "F8"}));
    click(&mut h, "Cancel");
    assert!(!asking(&h));
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "the dialog stays open");
    assert_eq!(field(&h, id, "activeSet"), json!("A"));
    assert_eq!(field(&h, id, "overrides"), json!({"edit.copy": "F8"}), "with its edits");
    assert_eq!(shown_set(&h), "A (modified)");
}

#[test]
fn no_discards_the_edits_and_switches() {
    let (mut h, id) = edited_then_switching();
    click(&mut h, "No");
    assert!(!asking(&h));
    assert_eq!(field(&h, id, "activeSet"), json!(""));
    assert_eq!(field(&h, id, "overrides"), json!({}), "the defaults' bindings");
    assert_eq!(shown_set(&h), "PhotoCraft Defaults");
    assert_eq!(field(&h, id, "sets")["A"]["shortcuts"], json!({"edit.copy": "F9"}), "A as it was saved");
    pick_set(&mut h, "A");
    assert!(!asking(&h), "no edits, no question");
    assert_eq!(shown_set(&h), "A", "the (modified) entry is gone");
}

#[test]
fn yes_asks_for_a_name_saves_the_edits_and_then_switches() {
    let (mut h, id) = edited_then_switching();
    click(&mut h, "Yes");
    assert!(!asking(&h));
    assert_eq!(field(&h, id, "newSetName"), json!("A Copy"), "the name prompt, as New Set… asks");
    assert_eq!(field(&h, id, "activeSet"), json!("A"), "not switched before the name");
    click(&mut h, "Create");
    assert_eq!(field(&h, id, "sets")["A Copy"]["shortcuts"], json!({"edit.copy": "F8"}), "the edits are saved");
    assert_eq!(field(&h, id, "sets")["A"]["shortcuts"], json!({"edit.copy": "F9"}));
    assert_eq!(field(&h, id, "activeSet"), json!(""), "and the chosen set is in use");
    assert_eq!(field(&h, id, "overrides"), json!({}));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(h.state().session.prefs().shortcut_sets["A Copy"].shortcuts["edit.copy"], "F8", "OK keeps the saved set");
}

#[test]
fn cancelling_the_name_after_yes_keeps_the_set_and_its_edits() {
    let (mut h, id) = edited_then_switching();
    click(&mut h, "Yes");
    click(&mut h, "Cancel");
    assert!(field(&h, id, "newSetName").is_null());
    assert_eq!(field(&h, id, "activeSet"), json!("A"));
    assert_eq!(field(&h, id, "overrides"), json!({"edit.copy": "F8"}));
    // A later New Set… doesn't switch away either.
    click(&mut h, "New Set…");
    set_field(&mut h, id, "newSetName", json!("B"));
    click(&mut h, "Create");
    assert_eq!(field(&h, id, "activeSet"), json!("B"));
    assert_eq!(field(&h, id, "overrides"), json!({"edit.copy": "F8"}));
}

#[test]
fn an_unmodified_set_switches_at_once() {
    let (mut h, id) = dialog();
    pick_set(&mut h, "PhotoCraft Defaults");
    assert!(!asking(&h));
    assert_eq!(field(&h, id, "activeSet"), json!(""));
    assert_eq!(field(&h, id, "overrides"), json!({}));
}

#[test]
fn an_action_function_key_is_no_edit() {
    let (mut h, first) = dialog();
    h.state_mut().ui.dialogs.retain(|d| d.id != first);
    h.state_mut().session.actions.list.push(photocraft_engine::actions_cmds::Action::new("Sepia", Vec::new()));
    crate::actions::assign_shortcut(h.state_mut(), 0, "F2").unwrap();
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    assert_eq!(shown_set(&h), "A", "an action key doesn't modify A");
    pick_set(&mut h, "PhotoCraft Defaults");
    assert!(!asking(&h));
    assert_eq!(field(&h, id, "activeSet"), json!(""));
}

#[test]
fn the_modified_defaults_offer_their_own_copy_name() {
    let (mut h, id) = dialog();
    pick_set(&mut h, "PhotoCraft Defaults");
    set_field(&mut h, id, "overrides", json!({"edit.cut": "F7"}));
    assert_eq!(shown_set(&h), "PhotoCraft Defaults (modified)");
    pick_set(&mut h, "A");
    assert!(asking(&h));
    click(&mut h, "Yes");
    assert_eq!(field(&h, id, "newSetName"), json!("PhotoCraft Defaults Copy"));
    click(&mut h, "Create");
    assert_eq!(field(&h, id, "sets")["PhotoCraft Defaults Copy"]["shortcuts"], json!({"edit.cut": "F7"}));
    assert_eq!(field(&h, id, "activeSet"), json!("A"));
    assert_eq!(field(&h, id, "overrides"), json!({"edit.copy": "F9"}));
}

/// CodeRabbit on #14864: after Yes the name row waits for Create. Choosing another set meanwhile
/// (and No) replaces that operation, so the name step and its pending switch go: Create would
/// otherwise save the other set's bindings as "A Copy" and switch to the first target.
#[test]
fn another_set_chosen_during_the_name_step_ends_it_and_its_pending_switch() {
    let (mut h, id) = dialog();
    // A second set B, saved beside A.
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.cut": "F7"}, "saveSet": "B"})).unwrap();
    h.state_mut().run("edit.keyboardShortcuts", json!({"useSet": "A"})).unwrap();
    let sets = h.state().session.prefs().shortcut_sets.clone();
    set_field(&mut h, id, "sets", json!(sets));
    set_field(&mut h, id, "activeSet", json!("A"));
    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8"}));
    pick_set(&mut h, "PhotoCraft Defaults");
    click(&mut h, "Yes");
    assert_eq!(field(&h, id, "newSetName"), json!("A Copy"), "the name step is open");
    assert_eq!(field(&h, id, "switchAfterSave"), json!(""), "with the defaults as the pending target");
    pick_set(&mut h, "B");
    assert!(asking(&h), "another switch question");
    click(&mut h, "No");
    assert_eq!(field(&h, id, "activeSet"), json!("B"), "B is in use");
    assert_eq!(field(&h, id, "newSetName"), Value::Null, "the name step is gone");
    assert_eq!(field(&h, id, "switchAfterSave"), Value::Null, "and so is its pending switch");
}

/// CodeRabbit on #14864: choosing another set while the name step is open (the set is still
/// modified, so the question opens again) ends the name step at once, so Create can't run the
/// earlier switch before the new question is answered.
#[test]
fn choosing_a_set_during_the_name_step_ends_it_before_the_new_question_is_answered() {
    let (mut h, id) = dialog();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.cut": "F7"}, "saveSet": "B"})).unwrap();
    h.state_mut().run("edit.keyboardShortcuts", json!({"useSet": "A"})).unwrap();
    let sets = h.state().session.prefs().shortcut_sets.clone();
    set_field(&mut h, id, "sets", json!(sets));
    set_field(&mut h, id, "activeSet", json!("A"));
    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8"}));
    pick_set(&mut h, "PhotoCraft Defaults");
    click(&mut h, "Yes");
    assert_eq!(field(&h, id, "newSetName"), json!("A Copy"));
    pick_set(&mut h, "B");
    assert!(asking(&h), "the new question is open");
    assert_eq!(field(&h, id, "newSetName"), Value::Null, "the name step is gone already");
    assert_eq!(field(&h, id, "switchAfterSave"), Value::Null, "with its switch");
}

/// CodeRabbit on #14864: a question left open for set B is dropped when another choice switches at
/// once (A saved meanwhile, so C applies without asking): No must not switch to B afterwards.
#[test]
fn a_question_for_one_set_is_dropped_when_another_set_switches_at_once() {
    let (mut h, id) = dialog();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.cut": "F7"}, "saveSet": "B"})).unwrap();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"edit.paste": "F6"}, "saveSet": "C"})).unwrap();
    h.state_mut().run("edit.keyboardShortcuts", json!({"useSet": "A"})).unwrap();
    let sets = h.state().session.prefs().shortcut_sets.clone();
    set_field(&mut h, id, "sets", json!(sets));
    set_field(&mut h, id, "activeSet", json!("A"));
    set_field(&mut h, id, "overrides", json!({"edit.copy": "F8"}));
    pick_set(&mut h, "B");
    assert!(asking(&h), "the question for B is open");
    click(&mut h, "Save");
    pick_set(&mut h, "C");
    assert_eq!(field(&h, id, "activeSet"), json!("C"), "A is saved, so C applies at once");
    assert!(!asking(&h), "and the question for B is gone");
    assert_eq!(field(&h, id, "switchTo"), Value::Null);
}
