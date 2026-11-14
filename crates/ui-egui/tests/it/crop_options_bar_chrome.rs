//! #10888: the Crop options bar's chrome. Straighten carries the reference label and the new
//! spirit-level glyph; the bar also shows the reference tooltips, Cancel/Commit only while a crop
//! is live, the Fill options help modal, and empty free-ratio sides.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::crop_straighten::{self, Straighten};
use photocraft_ui_egui::state::Tool;
use serde_json::json;

type App = Harness<'static, PhotocraftApp>;

fn app() -> App {
    Harness::builder().with_size(egui::vec2(1440.0, 320.0)).with_max_steps(64).with_accessibility_check(false).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        // The accessibility queries below use English labels.
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"name": "crop bar", "width": 800, "height": 600})).expect("new document");
        app.ui.tool = Tool::Crop;
        app
    })
}

/// Every text shape the harness drew this frame, tooltips and modals included.
fn drawn<State>(h: &Harness<'_, State>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

fn straighten_harness() -> Harness<'static, Straighten> {
    let h = Harness::builder()
        .with_size(egui::vec2(260.0, 60.0))
        .with_max_steps(64)
        .build_ui_state(|ui, s: &mut Straighten| crop_straighten::options_button(s, ui), Straighten::default());
    // The SVG glyph needs the app's image loaders; the label and the click do not.
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

#[test]
fn the_straighten_button_shows_its_label_and_arms() {
    let mut h = straighten_harness();
    h.run();
    assert!(h.query_by_label("Straighten").is_some(), "the button carries the reference label");
    h.get_by_label("Straighten").click();
    h.run();
    assert!(h.state().armed, "the button arms straighten");
    h.get_by_label("Straighten").click();
    h.run();
    assert!(!h.state().armed, "clicking again disarms it");
}

#[test]
fn the_straighten_glyph_is_embedded() {
    assert!(photocraft_ui_egui::icons::exists("spirit-level"));
}

#[test]
fn cancel_and_commit_show_only_for_a_live_crop() {
    let mut h = app();
    h.run_steps(6);
    assert!(h.query_by_label_contains("Cancel current crop operation").is_none(), "no cancel while idle");
    assert!(h.query_by_label_contains("Commit current crop operation").is_none(), "no commit while idle");
    assert!(h.query_by_label_contains("Reset the crop box").is_some(), "Reset stays, disabled");

    photocraft_ui_egui::crop_ui::ensure_frame(h.state_mut());
    photocraft_ui_egui::crop_ui::nudge(h.state_mut(), 1.0, 0.0);
    h.run_steps(2);
    let reset = h.get_by_label_contains("Reset the crop box").rect();
    let cancel = h.get_by_label_contains("Cancel current crop operation").rect();
    let commit = h.get_by_label_contains("Commit current crop operation").rect();
    assert!(reset.left() < cancel.left() && cancel.left() < commit.left(), "Reset, Cancel, Commit, in that order: {reset:?} {cancel:?} {commit:?}");
}

#[test]
fn the_fill_help_modal_opens_and_esc_leaves_the_crop() {
    let mut h = app();
    h.run_steps(4);
    photocraft_ui_egui::crop_ui::ensure_frame(h.state_mut());
    photocraft_ui_egui::crop_ui::nudge(h.state_mut(), 1.0, 0.0);
    h.run_steps(2);
    let before = h.state().ui.crop_rect;

    h.get_by_label("Displays pop-up help for the fill methods").click();
    h.run_steps(2);
    let text = drawn(&h);
    assert!(text.iter().any(|t| t == "Fill options"), "the modal opened: {text:?}");
    assert!(text.iter().any(|t| t.contains("Background (default):")));
    assert!(text.iter().any(|t| t.contains("Content-aware fill")));

    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(!drawn(&h).iter().any(|t| t == "Fill options"), "OK closes the modal");

    // Esc closes the modal only: the pending crop frame is still there afterwards.
    h.get_by_label("Displays pop-up help for the fill methods").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!drawn(&h).iter().any(|t| t == "Fill options"), "Esc closes the modal");
    assert_eq!(h.state().ui.crop_rect, before, "Esc leaves the crop alone");
}

#[test]
fn crop_controls_carry_the_reference_tooltips() {
    use egui::accesskit::Role;
    let mut h = app();
    h.run_steps(4);
    for (role, label, tip) in [
        (Role::CheckBox, "Delete Cropped Pixels", "Determines if pixel data outside the crop box is retained or deleted"),
        (Role::Button, "Displays pop-up help for the fill methods", "Displays pop-up help for the fill methods"),
        (Role::ComboBox, "Fill:", "How expanded crop area gets filled"),
        (Role::ComboBox, "Aspect Ratio", "Select a preset aspect ratio or crop size"),
    ] {
        // Two nodes can share a name (the bar's twin layouts), so take the first.
        h.get_all_by_role_and_label(role, label).next().unwrap_or_else(|| panic!("no {label}")).hover();
        // egui holds a tooltip back for half a second before showing it.
        h.run_steps(40);
        assert!(drawn(&h).iter().any(|t| t == tip), "{label} shows its tooltip: {:?}", drawn(&h));
    }
}

#[test]
fn clearing_a_blank_zero_field_commits_zero() {
    let mut h = Harness::builder().with_size(egui::vec2(120.0, 60.0)).with_max_steps(4).with_accessibility_check(false).build_ui_state(
        |ui, v: &mut f32| {
            photocraft_ui_egui::widgets::value_field_prec_blank(ui, v, 0.0..=99_999.0, "", 54.0, 3);
        },
        1.5f32,
    );
    h.run();
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run_steps(2);
    // Select all and delete, as a user clearing a ratio side does.
    for (key, pressed, modifiers) in [
        (egui::Key::A, true, egui::Modifiers::COMMAND),
        (egui::Key::A, false, egui::Modifiers::COMMAND),
        (egui::Key::Backspace, true, egui::Modifiers::NONE),
        (egui::Key::Backspace, false, egui::Modifiers::NONE),
    ] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.run_steps(2);
    assert_eq!(*h.state(), 0.0, "a cleared side is zero, not the value it had");
}

#[test]
fn hiding_the_options_bar_releases_the_fill_help() {
    let mut h = app();
    h.run_steps(4);
    photocraft_ui_egui::crop_ui::ensure_frame(h.state_mut());
    photocraft_ui_egui::crop_ui::nudge(h.state_mut(), 1.0, 0.0);
    h.run_steps(2);
    h.get_by_label("Displays pop-up help for the fill methods").click();
    h.run_steps(2);
    assert!(drawn(&h).iter().any(|t| t == "Fill options"), "the help opened");

    // F hides the chrome and with it the bar; the help goes too and must not keep the crop's keys.
    h.state_mut().ui.panels.options_bar = false;
    h.run_steps(2);
    assert!(!drawn(&h).iter().any(|t| t == "Fill options"), "the modal goes with the bar");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.crop_rect.is_none(), "Enter commits the crop again");
}

#[test]
fn esc_in_the_frame_the_fill_help_opens_leaves_the_crop() {
    let mut h = app();
    h.run_steps(4);
    photocraft_ui_egui::crop_ui::ensure_frame(h.state_mut());
    photocraft_ui_egui::crop_ui::nudge(h.state_mut(), 1.0, 0.0);
    h.run_steps(2);
    let before = h.state().ui.crop_rect;

    // Esc and the click on the Fill ⓘ land in one frame: the shortcut pass runs first, so the click
    // must claim Esc before it cancels the crop underneath.
    let at = h.get_by_label("Displays pop-up help for the fill methods").rect().center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        button(true),
        button(false),
    ]);
    h.run_steps(2);
    assert_eq!(h.state().ui.crop_rect, before, "the crop frame and its history survive");
}

#[test]
fn the_free_ratio_sides_draw_nothing_at_zero() {
    fn field(value: f32) -> Vec<String> {
        let mut h = Harness::builder().with_size(egui::vec2(120.0, 60.0)).with_max_steps(4).with_accessibility_check(false).build_ui_state(
            |ui, v: &mut f32| {
                photocraft_ui_egui::widgets::value_field_prec_blank(ui, v, 0.0..=99_999.0, "", 54.0, 3);
            },
            value,
        );
        h.run();
        drawn(&h)
    }
    assert!(field(0.0).iter().all(|t| t.is_empty()), "an unset side is blank, not `0`: {:?}", field(0.0));
    assert!(field(1.5).iter().any(|t| t == "1.5"), "a set side still shows its number: {:?}", field(1.5));
}
