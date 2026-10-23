//! The History panel's own items at the top of its panel menu (☰, #2966), in Photoshop's order:
//! Step Forward, Step Backward, New Snapshot…, Delete, Clear History, New Document, History
//! Options…, Close. Every item runs an engine command (or the dock's `window.panel.close`); the
//! dock appends its group items (Close Tab Group …) after them.

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The menu's items: label and command (None = shown greyed; not built yet).
pub const ITEMS: &[(&str, Option<&str>)] = &[
    ("Step Forward", Some("history.stepForward")),
    ("Step Backward", Some("history.stepBackward")),
    ("---", None),
    // Takes a Full Document snapshot at once; switches to the New Snapshot dialog when that
    // lands (#3567).
    ("New Snapshot…", Some("history.newSnapshot")),
    ("Delete", Some("history.deleteState")),
    ("Clear History", Some("history.clear")),
    ("New Document", Some("history.newDocument")),
    ("---", None),
    // The History Options dialog is not built yet.
    ("History Options…", None),
    ("---", None),
    ("Close", Some("window.panel.close")),
];

fn params(cmd: &str) -> Value {
    if cmd == "window.panel.close" { json!({"panel": "history"}) } else { json!({}) }
}

/// The command and params an item runs: with a snapshot selected in the panel, Delete removes
/// that snapshot, as the panel's trash does (#5324); otherwise it removes the current state.
fn resolve(cmd: &str, snapshot: Option<u64>) -> (&str, Value) {
    match snapshot {
        Some(id) if cmd == "history.deleteState" => ("history.deleteSnapshot", json!({"snapshot": id})),
        _ => (cmd, params(cmd)),
    }
}

fn enabled(app: &PhotocraftApp, cmd: &str) -> bool {
    if cmd == "window.panel.close" {
        return !app.session.prefs().workspace_locked;
    }
    // An open Free Transform owns the history (see `panels::history`).
    app.ui.transform.is_none() && app.session.is_enabled(cmd)
}

/// Draw the History items of the panel menu and run the one clicked.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let snapshot = app.session.active().and_then(|st| crate::panels::selected_snapshot(ui.ctx(), st));
    let mut clicked: Option<&str> = None;
    for &(label, cmd) in ITEMS {
        if label == "---" {
            ui.separator();
            continue;
        }
        let on = cmd.is_some_and(|c| enabled(app, resolve(c, snapshot).0));
        if ui.add_enabled(on, egui::Button::new(crate::i18n::t(label))).clicked() {
            clicked = cmd;
        }
    }
    let Some(cmd) = clicked else { return };
    ui.close();
    let (cmd, params) = resolve(cmd, snapshot);
    if let Err(e) = app.run(cmd, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
        app
    }

    fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(egui::vec2(260.0, 420.0)).build_ui_state(|ui, app: &mut PhotocraftApp| panel_menu(app, ui), app);
        h.run_steps(2);
        h
    }

    fn states(h: &Harness<'static, PhotocraftApp>) -> (usize, usize) {
        let st = h.state().session.active().unwrap();
        (st.history.entries().len(), st.history.redo_labels().count())
    }

    fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
        h.get_by_label(label).click();
        h.run_steps(2);
    }

    fn is_disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
        h.get_by_label(label).accesskit_node().is_disabled()
    }

    #[test]
    fn menu_items_run_their_commands() {
        let mut h = harness(app());
        assert_eq!(states(&h), (4, 0));
        assert!(is_disabled(&h, "Step Forward") && !is_disabled(&h, "Step Backward"));
        assert!(is_disabled(&h, "History Options…"), "not built yet: greyed");
        assert!(!is_disabled(&h, "New Snapshot…"));
        click(&mut h, "Step Backward");
        assert_eq!(states(&h), (3, 1));
        assert!(!is_disabled(&h, "Step Forward"));
        click(&mut h, "Step Forward");
        assert_eq!(states(&h), (4, 0));
        click(&mut h, "Delete");
        assert_eq!(states(&h), (3, 0), "Delete removes the current state");
        let doc = h.state().session.active().unwrap().doc.clone();
        click(&mut h, "Clear History");
        assert_eq!(states(&h), (1, 0), "only the current state is left");
        assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &doc), "the image is unchanged");
        assert!(is_disabled(&h, "Clear History") && is_disabled(&h, "Delete") && is_disabled(&h, "Step Forward"));
        click(&mut h, "New Document");
        assert_eq!(h.state().session.documents().len(), 2);
        assert!(!h.state().ui.status_error);
    }

    #[test]
    fn close_hides_the_history_panel() {
        let mut app = app();
        app.run("window.panel.activate", json!({"panel": "history"})).unwrap();
        assert!(app.ui.docking.as_ref().unwrap().contains(&"history".to_string()));
        let mut h = harness(app);
        click(&mut h, "Close");
        assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
        let layout = h.state().ui.docking.clone().unwrap();
        assert!(!layout.contains(&"history".to_string()), "the History panel is closed");
    }

    #[test]
    fn items_wait_for_an_open_transform() {
        let mut app = app();
        app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
        let mut h = harness(app);
        let ctx = h.ctx.clone();
        crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
        h.run_steps(2);
        for label in ["Step Backward", "New Snapshot…", "Delete", "Clear History", "New Document"] {
            assert!(is_disabled(&h, label), "{label}");
        }
    }

    #[test]
    fn new_snapshot_adds_a_snapshot_and_leaves_the_states() {
        let mut h = harness(app());
        let snaps = |h: &Harness<'static, PhotocraftApp>| h.state().session.active().unwrap().snapshots.len();
        let (before, steps) = (snaps(&h), states(&h));
        click(&mut h, "New Snapshot…");
        assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
        assert_eq!(snaps(&h), before + 1);
        assert_eq!(states(&h), steps, "a snapshot is no history step");
    }

    /// With a snapshot selected in the panel, Delete removes the snapshot and leaves the states,
    /// as the panel's trash does (#5324); without one it removes the current state.
    #[test]
    fn delete_removes_the_selected_snapshot() {
        let mut app = app();
        app.run("history.newSnapshot", json!({})).unwrap();
        let mut h = harness(app);
        let snaps = |h: &Harness<'static, PhotocraftApp>| h.state().session.active().unwrap().snapshots.len();
        let before = snaps(&h);
        let id = h.state().session.active().unwrap().snapshots.last().unwrap().id;
        let ctx = h.ctx.clone();
        crate::panels::select_snapshot(&ctx, h.state().session.active().unwrap(), id);
        h.run_steps(2);
        let steps = states(&h);
        click(&mut h, "Delete");
        assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
        assert_eq!(snaps(&h), before - 1, "Delete removed the selected snapshot");
        assert_eq!(states(&h), steps, "and left the states alone");
        click(&mut h, "Delete");
        assert_eq!(snaps(&h), before - 1, "no snapshot selected now: the snapshots are kept");
        assert_eq!(states(&h).0, steps.0 - 1, "the current state is deleted");
    }

    #[test]
    fn without_a_document_only_close_is_enabled() {
        let h = harness(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()));
        for &(label, cmd) in ITEMS.iter().filter(|(l, _)| *l != "---") {
            assert_eq!(is_disabled(&h, label), cmd != Some("window.panel.close"), "{label}");
        }
    }
}
