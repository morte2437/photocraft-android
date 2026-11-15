//! #11978: the Perspective Crop options bar shows its Cancel and Commit buttons only during a
//! session (a quad, or corners being clicked), and they still commit and cancel.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

const COMMIT: &str = "Commit current crop operation";
const CANCEL: &str = "Cancel current crop operation";

/// A 200 × 150 document with the Perspective Crop tool chosen and no session.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 80.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn shows(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.query_by_label_contains(label).is_some()
}

#[test]
fn without_a_session_neither_button_shows() {
    let h = harness(app());
    assert!(!shows(&h, COMMIT) && !shows(&h, CANCEL));
    assert!(h.query_by_label("Show Grid").is_some(), "the bar still ends with Show Grid");
}

#[test]
fn a_pending_quad_shows_both_and_commit_commits() {
    let mut app = app();
    app.ui.perspective_crop = Some([[20.0, 20.0], [180.0, 20.0], [180.0, 130.0], [20.0, 130.0]]);
    let mut h = harness(app);
    assert!(shows(&h, COMMIT) && shows(&h, CANCEL));
    h.get_by_label_contains(COMMIT).click();
    h.run_steps(3);
    let size = h.state().session.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (160, 110), "Commit cropped to the quad");
    assert!(!shows(&h, COMMIT) && !shows(&h, CANCEL), "gone after commit");
}

#[test]
fn clicked_corners_show_both_and_cancel_cancels() {
    let mut app = app();
    app.pcrop.clicks = vec![[20.0, 20.0], [180.0, 25.0]];
    let mut h = harness(app);
    assert!(shows(&h, COMMIT) && shows(&h, CANCEL), "with clicked corners");
    h.get_by_label_contains(CANCEL).click();
    h.run_steps(3);
    assert!(h.state().pcrop.clicks.is_empty(), "Cancel dropped the corners");
    assert_eq!(h.state().session.active().unwrap().doc.size.width, 200, "nothing cropped");
    assert!(!shows(&h, COMMIT) && !shows(&h, CANCEL), "gone after cancel");
}
