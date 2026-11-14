//! #12054: the Type options bar ends with the reference application's button that shows and
//! hides the Character and Paragraph panels. It runs Window › Character, so the cases the issue
//! leaves unmeasured keep that command's behaviour.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;

const TOGGLE: &str = "Toggle Character and Paragraph panels";

fn bare_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = crate::state::Tool::Type;
    app
}

/// An app with one selected type layer, and the layer's id.
fn app_with_layer() -> (PhotocraftApp, LayerId) {
    let mut app = bare_app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let made = app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    (app, id)
}

fn editing(mut app: PhotocraftApp, id: LayerId) -> PhotocraftApp {
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: None,
        caret: 2,
        anchor: 2,
        session: "panels-toggle".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    app
}

/// The options bar in a row, once the app's fonts are set up (the first pass has none).
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 200.0)).with_max_steps(16).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar::options_bar(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_role_and_label(Role::Button, TOGGLE).click();
    h.run();
}

/// Whether the Character | Paragraph group is shown, and its front tab (0 Character, 1 Paragraph).
fn group(h: &Harness<'static, PhotocraftApp>) -> (bool, usize) {
    (h.state().ui.panels.character, h.state().ui.dock_tabs.character)
}

/// Clicks the button twice: the group opens with Character in front, then closes.
fn opens_then_closes(app: PhotocraftApp) {
    let mut h = harness(app);
    assert!(!group(&h).0, "the group starts hidden");
    click(&mut h);
    assert_eq!(group(&h), (true, 0));
    click(&mut h);
    assert!(!group(&h).0, "the second click hides the group");
}

#[test]
fn the_button_follows_create_warped_text_with_the_reference_tooltip() {
    let (app, _) = app_with_layer();
    let h = harness(app);
    let warp = h.get_by_role_and_label(Role::Button, "Create warped text").rect();
    let toggle = h.get_by_role_and_label(Role::Button, TOGGLE);
    let rect = toggle.rect();
    assert!(rect.left() >= warp.right(), "{rect:?} is not after {warp:?}");
    assert!((rect.center().y - warp.center().y).abs() < 1.0, "{rect:?} is not in the row of {warp:?}");
    // Nothing sits between the two.
    let between = h.query_all_by_role(Role::Button).filter(|b| b.rect().left() >= warp.right() && b.rect().right() <= rect.left()).count();
    assert_eq!(between, 0);
    assert_eq!(toggle.accesskit_node().description().map(|d| d.to_string()), Some(TOGGLE.to_owned()));
}

#[test]
fn with_a_type_layer_it_opens_the_group_with_character_in_front_then_closes_it() {
    opens_then_closes(app_with_layer().0);
}

#[test]
fn without_a_document_it_opens_and_closes_the_group() {
    opens_then_closes(bare_app());
}

#[test]
fn while_editing_text_it_opens_and_closes_the_group() {
    let (app, id) = app_with_layer();
    opens_then_closes(editing(app, id));
}

/// While editing, Cancel and Commit stay at the bar's right end, after the toggle.
#[test]
fn while_editing_cancel_and_commit_follow_the_toggle() {
    let (app, id) = app_with_layer();
    let h = harness(editing(app, id));
    let toggle = h.get_by_role_and_label(Role::Button, TOGGLE).rect();
    for label in ["Cancel any current edits (Esc)", "Commit any current edits (⌘↩)"] {
        let r = h.query_all_by_role(Role::Button).find(|b| b.accesskit_node().label().is_some_and(|l| l.starts_with(&label[..12])));
        let r = r.unwrap_or_else(|| panic!("no {label}")).rect();
        assert!(r.left() >= toggle.right(), "{label} {r:?} is before the toggle {toggle:?}");
    }
}

/// Not measured: with Paragraph in front, the button does what Window › Character does and
/// brings Character to the front of the open group.
#[test]
fn with_paragraph_in_front_it_brings_character_forward_like_window_character() {
    let (mut app, _) = app_with_layer();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.panel.paragraph", json!({})).unwrap();
    let mut h = harness(app);
    assert_eq!(group(&h), (true, 1));
    click(&mut h);
    assert_eq!(group(&h), (true, 0));
}

#[test]
fn the_toggle_leaves_the_document_alone() {
    let (app, _) = app_with_layer();
    let steps = app.session.active().unwrap().history.entries().len();
    let mut h = harness(app);
    click(&mut h);
    click(&mut h);
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps);
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}
