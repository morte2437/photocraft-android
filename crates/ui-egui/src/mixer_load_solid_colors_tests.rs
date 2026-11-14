//! The Mixer Brush load menu's Load Solid Colors Only (#11952): Load Brush, Clean Brush, a
//! separator, then the checkable entry, off by default and kept with the Mixer Brush's own brush.
//! On, ⌥-click loads the one colour under the hotspot (red/green edge at 200,300 → 30,180,60), the
//! swatch shows it and the next stroke paints it; off, ⌥-click loads the tip image as before.

use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const RED: [f32; 4] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
const GREEN: [f32; 4] = [30.0 / 255.0, 180.0 / 255.0, 60.0 / 255.0, 1.0];

/// An 800 × 600 document whose pixel layer is red left of x 200 and green from there on, with the
/// Mixer Brush active.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Split"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 600})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": RED})).unwrap();
    app.run("select.rect", json!({"x": 200, "y": 0, "width": 600, "height": 600})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": GREEN})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app
}

fn alt_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::ALT);
}

fn level(c: [f32; 4]) -> [i32; 4] {
    c.map(|v| (v * 255.0).round() as i32)
}

#[test]
fn the_load_menu_lists_load_solid_colors_only_after_a_separator() {
    let mut h = Harness::builder().with_size(vec2(1600.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    if !h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return;
    }
    assert!(!h.state().session.tools.brush.mixer.solid_colors_only, "off by default");
    let open = |h: &mut Harness<'static, PhotocraftApp>| {
        h.get_by_label("Current Brush Load").click();
        h.run_steps(3);
    };
    open(&mut h);
    let tops: Vec<f32> = ["Load Brush", "Clean Brush", "Load Solid Colors Only"].iter().map(|l| h.get_by_label(l).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "menu order {tops:?}");
    // The separator: a larger gap above the entry than between Load Brush and Clean Brush.
    assert!(tops[2] - tops[1] > tops[1] - tops[0], "no separator before Load Solid Colors Only: {tops:?}");
    let entry = h.get_by_label("Load Solid Colors Only");
    assert_eq!(entry.accesskit_node().role(), egui::accesskit::Role::CheckBox);
    assert_eq!(entry.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    entry.click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.mixer.solid_colors_only, "the entry turns it on");
    assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("tools.setBrush"));
    // It is the Mixer Brush's own setting: the Brush tool's brush doesn't have it on.
    let app = h.state_mut();
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(app);
    assert!(!app.session.tools.brush.mixer.solid_colors_only, "the Brush tool's brush is its own");
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(app);
    assert!(app.session.tools.brush.mixer.solid_colors_only, "the Mixer Brush keeps it");
}

#[test]
fn with_it_on_alt_click_loads_the_colour_under_the_hotspot_and_the_next_stroke_paints_it() {
    let mut app = app();
    app.run("tools.setBrush", json!({"brush": {"mixer": {"solidColorsOnly": true, "wet": 0.0}, "size": 20, "hardness": 1.0}})).unwrap();
    alt_click(&mut app, 200.0, 300.0);
    let mixer = &app.session.tools.mixer;
    assert_eq!(mixer.reservoir.map(level), Some(level(GREEN)), "{mixer:?}");
    assert!(mixer.spatial.is_none(), "a plain colour, not the tip image");
    // The Current Brush Load swatch shows it.
    assert_eq!(mixer.next_load(app.session.tools.foreground).map(level), Some(level(GREEN)));
    // The next stroke, on an empty layer above, paints it.
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 500.0, y: 100.0 }, Modifiers::NONE);
    let st = app.session.active().unwrap();
    let px = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(450, 100);
    let [r, g, b, a] = level(px);
    assert!((r - 30).abs() <= 2 && (g - 180).abs() <= 2 && (b - 60).abs() <= 2 && a >= 250, "stroke painted {:?}", level(px));
}

#[test]
fn with_it_off_alt_click_loads_the_tip_image_as_before() {
    let mut app = app();
    alt_click(&mut app, 200.0, 300.0);
    assert!(app.session.tools.mixer.spatial.is_some(), "the sampled tip image");
}
