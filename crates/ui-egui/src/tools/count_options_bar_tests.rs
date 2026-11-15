//! #11765: the Count tool's options bar shows every control from the start, in Photoshop's order,
//! with its tooltips, and greys what can't act: the group dropdown until there is a group, the
//! trash with fewer than two groups, Clear while the current group is empty or hidden.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const EYE: &str = "Toggle visibility of count group";
const FOLDER: &str = "Create a new count group";
const TRASH: &str = "Delete the currently selected count group";
const SWATCH: &str = "Count group color";

/// A 100 × 100 document with the Count tool and `groups` count groups (the first with a marker).
fn app(groups: usize) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    if groups > 0 {
        app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    }
    for _ in 1..groups {
        app.run("count.newGroup", json!({})).unwrap();
    }
    if groups > 0 {
        app.run("count.setGroup", json!({"group": 0, "active": true})).unwrap();
    }
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 120.0)).build_ui_state(
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

fn disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn combo_disabled(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the group dropdown").accesskit_node().is_disabled()
}

#[test]
fn the_controls_are_in_the_reference_order_in_every_state() {
    for groups in [0, 1, 2] {
        let h = harness(app(groups));
        let x = |label: &str| h.get_by_label(label).rect().left();
        let combo = h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the group dropdown").rect().left();
        let order = [combo, x(EYE), x(FOLDER), x(TRASH), x("Clear"), x(SWATCH), x("Marker Size:"), x("Label Size:")];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{groups} groups: left edges {order:?}");
        let count = h.get_all_by_label_contains("Count: ").next().expect("the Count: readout").rect().left();
        assert!(count < combo, "{groups} groups: the readout comes first");
    }
}

#[test]
fn with_no_group_the_dropdown_trash_and_clear_are_greyed_and_the_next_groups_values_show() {
    let h = harness(app(0));
    assert!(combo_disabled(&h), "the dropdown is greyed");
    let combo = h.get_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap();
    let node = combo.accesskit_node();
    assert_eq!(node.value().as_deref(), Some("Count Group"), "with the placeholder");
    assert!(disabled(&h, TRASH) && disabled(&h, "Clear"));
    assert!(!disabled(&h, EYE) && !disabled(&h, FOLDER));
    // The sizes the next new group gets today.
    let next = photocraft_doc::CountGroup::default();
    assert!(h.query_by_value(&next.marker_size.to_string()).is_some(), "Marker Size {}", next.marker_size);
    assert!(h.query_by_value(&next.label_size.to_string()).is_some(), "Label Size {}", next.label_size);
}

#[test]
fn using_the_eye_with_no_group_changes_nothing() {
    let mut h = harness(app(0));
    let steps = h.state().session.active().unwrap().history.past_len();
    h.get_by_label(EYE).click();
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert!(st.doc.measurement.count_groups.is_empty());
    assert_eq!(st.history.past_len(), steps);
}

#[test]
fn the_trash_needs_two_groups() {
    assert!(disabled(&harness(app(1)), TRASH), "one group");
    let mut h = harness(app(2));
    assert!(!disabled(&h, TRASH), "two groups");
    h.get_by_label(TRASH).click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.measurement.count_groups.len(), 1);
    assert!(disabled(&h, TRASH), "back to one group");
}

#[test]
fn clear_needs_a_visible_current_group_with_markers() {
    let h = harness(app(1));
    assert!(!disabled(&h, "Clear"), "one marker, visible");
    let mut hidden = app(1);
    hidden.run("count.setGroup", json!({"visible": false})).unwrap();
    assert!(disabled(&harness(hidden), "Clear"), "hidden");
    let mut empty = app(2);
    empty.run("count.setGroup", json!({"group": 1, "active": true})).unwrap();
    assert!(disabled(&harness(empty), "Clear"), "the current group has no markers");
}

#[test]
fn the_swatch_opens_the_count_color_picker() {
    let mut h = harness(app(1));
    h.get_by_label(SWATCH).click();
    h.run_steps(3);
    let want = crate::color_picker_ui::title_for("Count Color");
    assert_eq!(want, "Color Picker (Count Color)");
    assert!(h.state().ui.dialogs.iter().any(|d| crate::dialogs::title(d) == want), "a picker titled {want}");
}

#[test]
fn the_size_captions_scrub_their_fields() {
    let mut h = harness(app(1));
    // The visible caption, not the field it names (`get_by_label` returns the labelled field).
    let from = h
        .query_all(egui_kittest::kittest::by().label("Marker Size:").include_labels())
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .expect("the Marker Size: caption")
        .rect()
        .center();
    // As the Brush options' scrub test drives a caption (options_hit_area_tests): hover, press,
    // move in steps, release, letting each step settle.
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for i in 1..=4 {
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.hover_at(from + vec2(12.0 * i as f32, 0.0));
        h.run();
    }
    h.drop_at(from + vec2(48.0, 0.0));
    h.run_steps(2);
    let g = &h.state().session.active().unwrap().doc.measurement.count_groups[0];
    assert!(g.marker_size > 1, "dragging the caption right raised Marker Size: {}", g.marker_size);
    assert!(g.marker_size <= 10, "within its range");
}

/// Drags the visible "Marker Size:" caption `dx` points right in four moves.
fn scrub_marker_size(h: &mut Harness<'static, PhotocraftApp>, dx: f32) {
    let from = h
        .query_all(egui_kittest::kittest::by().label("Marker Size:").include_labels())
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .expect("the Marker Size: caption")
        .rect()
        .center();
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for i in 1..=4 {
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.hover_at(from + vec2(dx / 4.0 * i as f32, 0.0));
        h.run();
    }
    h.drop_at(from + vec2(dx, 0.0));
    h.run_steps(2);
}

#[test]
fn one_size_scrub_is_one_history_step() {
    let mut h = harness(app(1));
    let steps = |h: &Harness<'static, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let before = steps(&h);
    scrub_marker_size(&mut h, 24.0);
    let size = h.state().session.active().unwrap().doc.measurement.count_groups[0].marker_size;
    assert!(size > 2, "the scrub moved Marker Size several steps: {size}");
    assert_eq!(steps(&h), before + 1, "one scrub, one Undo step");
    // Back left, as the first may have reached the top of the range.
    scrub_marker_size(&mut h, -24.0);
    let back = h.state().session.active().unwrap().doc.measurement.count_groups[0].marker_size;
    assert!(back < size, "the second scrub moved it back: {size} -> {back}");
    assert_eq!(steps(&h), before + 2, "a second scrub is a step of its own");
}

#[test]
fn automation_delete_and_clear_work_as_today() {
    // The greying is the options bar's: the commands still delete the last group and clear a
    // hidden one.
    let mut app = app(1);
    app.run("count.setGroup", json!({"visible": false})).unwrap();
    app.run("count.clear", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.measurement.count_total(), 0);
    app.run("count.deleteGroup", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.measurement.count_groups.is_empty());
}

/// The visible caption node (a `Role::Label`) reading `text`, if any.
fn caption_node(h: &Harness<'static, PhotocraftApp>, text: &str) -> bool {
    h.query_all(egui_kittest::kittest::by().label(text).include_labels()).any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
}

#[test]
fn the_size_captions_are_translated() {
    // The catalogs translate "Marker Size" and "Label Size" (main's captions); the colon follows.
    // A missing catalog row shows the English, so the test doesn't require the translation.
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let _de = crate::i18n::language_scope(crate::i18n::Lang::from_code("de").expect("German"));
    let h = harness(app(1));
    for key in ["Marker Size", "Label Size"] {
        let want = format!("{}:", crate::i18n::t(key));
        assert!(caption_node(&h, &want), "the caption reads {want}");
    }
}

#[test]
fn the_size_fields_are_named_by_their_visible_captions() {
    let h = harness(app(1));
    for caption in ["Marker Size:", "Label Size:"] {
        let fields = h.query_all(egui_kittest::kittest::by().label(caption).predicate(|n| n.role() != egui::accesskit::Role::Label)).count();
        assert_eq!(fields, 1, "one field named {caption}, as its caption reads");
    }
}
