//! The Brush Preset picker's gear menu: Photoshop's entries in its order with its separators, the
//! picker's defaults (Brush Tip off, Show Search Bar on) and which entries are greyed (#11819).

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;
use crate::state::Tool;

/// Photoshop's gear menu, measured top to bottom; "―" is a separator.
const MEASURED: &[&str] = &[
    "New Brush Preset…",
    "New Brush Group…",
    "―",
    "Rename Brush…",
    "Delete Brush…",
    "―",
    "Brush Name",
    "Brush Stroke",
    "Brush Tip",
    "―",
    "Show Additional Preset Info",
    "―",
    "Show Search Bar",
    "Show Recent Brushes",
    "―",
    "Append Default Brushes…",
    "Import Brushes…",
    "Export Selected Brushes…",
    "―",
    "Get More Brushes…",
    "―",
    "Converted Legacy Tool Presets",
    "Legacy Brushes",
];

/// The entries a click works on with a preset selected; every other one is greyed.
const LIVE: &[&str] = &["New Brush Preset…", "Rename Brush…", "Delete Brush…", "Brush Name", "Brush Stroke", "Brush Tip", "Show Search Bar", "Import Brushes…"];

/// The entries that act on the selected preset.
const NEEDS_PRESET: &[&str] = &["Rename Brush…", "Delete Brush…", "Export Selected Brushes…"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

/// [`harness`] with the first preset picked, as after a click on it.
fn harness_with_preset() -> Harness<'static, PhotocraftApp> {
    let mut h = harness();
    let first = h.state().session.tools.presets.first().map(|p| p.name.clone()).expect("the default presets");
    h.state_mut().run("tools.setBrush", json!({ "preset": first })).unwrap();
    h.run_steps(2);
    h
}

/// Open the picker near the canvas's top left (room below for the whole menu), then its gear.
fn open_gear(h: &mut Harness<'static, PhotocraftApp>) {
    let r = h.state().last_canvas_rect;
    h.state_mut().ui.brush_picker = Some([r.left() + 20.0, r.top() + 10.0]);
    h.run_steps(2);
    h.get_by_label("Brush Preset Options").click();
    h.run_steps(2);
}

fn entries() -> Vec<&'static str> {
    GEAR_MENU.iter().filter_map(|e| e.map(Gear::label)).collect()
}

#[test]
fn the_gear_menu_lists_the_reference_entries_in_order_with_its_separators() {
    let menu: Vec<&str> = GEAR_MENU.iter().map(|e| e.map_or("―", Gear::label)).collect();
    assert_eq!(menu, MEASURED);
    // The menu drawn: every entry once, top to bottom in that order.
    let mut h = harness();
    open_gear(&mut h);
    let tops: Vec<f32> = entries().iter().map(|l| h.get_by_label(l).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "drawn top to bottom in the measured order: {tops:?}");
    assert!(h.query_all_by_label("Delete Brush").next().is_none(), "Delete Brush… has its ellipsis");
}

#[test]
fn the_picker_starts_without_the_brush_tip_and_with_the_search_bar() {
    let h = harness();
    let st = &h.state().ui.brush_picker_list;
    assert!(st.show_name && st.show_stroke && !st.show_tip, "Brush Name and Brush Stroke on, Brush Tip off");
    assert!(st.show_search, "Show Search Bar is on");
    // The Brushes panel keeps its own default.
    assert!(h.state().ui.brushes_panel.show_tip);
    // The menu shows the same, and the placeholders keep Photoshop's checks.
    let mut h = harness();
    open_gear(&mut h);
    for (label, on) in [
        ("Brush Name", true),
        ("Brush Stroke", true),
        ("Brush Tip", false),
        ("Show Additional Preset Info", true),
        ("Show Search Bar", true),
        ("Show Recent Brushes", true),
    ] {
        let checked = h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
        assert_eq!(checked, on, "{label}");
    }
}

#[test]
fn a_saved_brush_tip_still_wins_and_an_empty_card_gets_the_defaults_back() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.brush_picker = json!({"showTip": true, "showSearch": false}));
    restore(&mut app);
    assert!(app.ui.brush_picker_list.show_tip && !app.ui.brush_picker_list.show_search);
    app.session.prefs.edit(|p| p.brush_picker = json!({"showName": false, "showStroke": false, "showTip": false}));
    app.ui.brush_picker_list = list_state();
    restore(&mut app);
    let st = &app.ui.brush_picker_list;
    assert_eq!((st.show_name, st.show_stroke, st.show_tip), (true, true, false), "the at-least-one-part rule restores the defaults");
}

#[test]
fn show_search_bar_hides_the_search_field_and_is_remembered() {
    let mut h = harness();
    open_gear(&mut h);
    let fields = |h: &Harness<'static, PhotocraftApp>| h.query_all_by_role(Role::TextInput).count();
    let shown = fields(&h);
    h.state_mut().ui.brush_picker_list.filter = "zzz".into();
    h.get_by_label("Show Search Bar").click();
    h.run_steps(3);
    assert!(!h.state().ui.brush_picker_list.show_search);
    assert_eq!(fields(&h), shown - 1, "the search field goes");
    assert!(h.state().ui.brush_picker_list.filter.is_empty(), "a hidden search filters nothing");
    let ctx = h.ctx.clone();
    persist(h.state_mut(), &ctx);
    assert_eq!(h.state().session.prefs().brush_picker["showSearch"], json!(false));
    h.get_by_label("Show Search Bar").click();
    h.run_steps(3);
    assert!(h.state().ui.brush_picker_list.show_search);
    assert_eq!(fields(&h), shown, "and comes back");
}

#[test]
fn entries_without_a_function_and_preset_entries_without_a_preset_are_greyed() {
    let mut h = harness_with_preset();
    open_gear(&mut h);
    for label in entries() {
        assert_eq!(!h.get_by_label(label).accesskit_node().is_disabled(), LIVE.contains(&label), "{label} with a preset selected");
    }
    // A placeholder does nothing when clicked.
    let before = (h.state().session.tools.presets.len(), h.state().ui.dialogs.len());
    h.get_by_label("New Brush Group…").click();
    h.run_steps(2);
    assert_eq!((h.state().session.tools.presets.len(), h.state().ui.dialogs.len()), before);
    // With no preset selected the preset's entries grey too.
    let mut h = harness();
    assert!(h.state().session.tools.current_preset.is_none(), "a fresh session has no preset picked");
    open_gear(&mut h);
    for label in entries() {
        let live = LIVE.contains(&label) && !NEEDS_PRESET.contains(&label);
        assert_eq!(!h.get_by_label(label).accesskit_node().is_disabled(), live, "{label} with no preset selected");
    }
}

#[test]
fn delete_brush_still_deletes_the_selected_preset() {
    let mut h = harness_with_preset();
    let name = h.state().session.tools.current_preset.clone().expect("a preset is selected");
    open_gear(&mut h);
    h.get_by_label("Delete Brush…").click();
    h.run_steps(3);
    assert!(!h.state().session.tools.presets.iter().any(|p| p.name == name), "{name} is deleted");
}

#[test]
fn two_clicks_in_one_frame_still_leave_one_card_part_on() {
    let mut h = harness();
    open_gear(&mut h);
    // Brush Tip is off; both remaining parts get an assistive-technology click in the same frame
    // (kittest's own clicks each run a frame of their own, so the events go in by hand).
    let clicks: Vec<_> = ["Brush Name", "Brush Stroke"]
        .into_iter()
        .map(|label| {
            let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
            egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest { target_node, target_tree, action: egui::accesskit::Action::Click, data: None })
        })
        .collect();
    h.input_mut().events.extend(clicks);
    h.run_steps(3);
    let st = &h.state().ui.brush_picker_list;
    assert_eq!((st.show_name, st.show_stroke, st.show_tip), (false, true, false), "the last part on stays on");
}
