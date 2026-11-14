//! #11733: the Color Sampler's right-click menu on the canvas.

use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::canvas_tool_menu::{SAMPLER_DELETE, SAMPLER_MENU};
use crate::control::{ControlRequest, Outcome, handle};
use crate::{PhotocraftApp, Tool};

/// A 200 x 100 document with the Color Sampler and samplers at (20, 20) and (120, 60).
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 20, "y": 20})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 120, "y": 60})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    (app, egui::Context::default())
}

fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    let Outcome::Done(v) = handle(app, ctx, &req) else { panic!("{method} replies at once") };
    v
}

/// An agent's right-click at document point (x, y), as `ui.pointer` sends it.
fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64) {
    let r = control(app, ctx, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
    assert_eq!(r["ok"], json!(true), "{r}");
}

fn entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<Value> {
    let r = control(app, ctx, "ui.inspect", json!({}));
    r["result"]["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_else(|| panic!("no canvasToolMenu: {r}"))
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

/// The reference application's rows, in order, with `|` for a separator.
const EXPECTED: &[&str] = &[
    "Delete",
    "|",
    "Actual Color",
    "Proof Color",
    "|",
    "Grayscale",
    "RGB Color",
    "Web Color",
    "HSB Color",
    "HSL Color",
    "CMYK Color",
    "Lab Color",
    "|",
    "Grayscale 16 bit",
    "RGB 16 bit",
    "CMYK 16 bit",
    "Lab 16 bit",
    "|",
    "Total Ink",
];

fn labels(entries: &[Value]) -> Vec<String> {
    entries.iter().map(|e| if e["separator"] == json!(true) { "|".to_string() } else { e["label"].as_str().unwrap().to_string() }).collect()
}

#[test]
fn right_click_on_a_sampler_lists_the_menu_in_order_with_delete_enabled() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 20.5, 20.5);
    let menu = app.ui.canvas_tool_menu.clone().expect("the Color Sampler menu opens");
    assert_eq!(menu.tool, Tool::ColorSampler);
    assert_eq!(menu.sampler, Some(0));
    let entries = entries(&mut app, &ctx);
    assert_eq!(labels(&entries), EXPECTED);
    let enabled: Vec<&str> = entries.iter().filter(|e| e["enabled"] == json!(true)).filter_map(|e| e["id"].as_str()).collect();
    // The readout rows wait for per-sampler readout modes (#11735) and stay greyed until then.
    assert_eq!(enabled, [SAMPLER_DELETE]);
    for (_, id) in SAMPLER_MENU.iter().flatten() {
        assert!(!crate::canvas_tool_menu::mode_checked(&app, &menu, id), "{id} shows no checkmark");
    }
    assert_eq!(samplers(&app).len(), 2, "the right-click places no sampler");
}

#[test]
fn right_click_off_a_sampler_greys_every_row() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 70.0, 40.0);
    let menu = app.ui.canvas_tool_menu.clone().expect("the menu opens off a sampler too");
    assert_eq!(menu.sampler, None);
    let entries = entries(&mut app, &ctx);
    assert_eq!(labels(&entries), EXPECTED);
    assert!(entries.iter().all(|e| e["enabled"] != json!(true)), "every row greyed: {entries:?}");
    let r = control(&mut app, &ctx, "ui.context.choose", json!({"id": SAMPLER_DELETE}));
    assert_eq!(r["ok"], json!(false), "a greyed Delete does nothing: {r}");
    assert_eq!(samplers(&app).len(), 2, "nothing deleted, no sampler placed");
}

/// The right-click menu's Delete removes the sampler under the pointer, closes the menu, and
/// the samplers after it move up a number.
#[test]
fn delete_removes_the_sampler_under_the_pointer_and_renumbers_the_rest() {
    let (mut app, ctx) = app();
    app.run("view.colorSamplers.add", json!({"x": 180, "y": 90})).unwrap();
    right_click(&mut app, &ctx, 120.0, 60.0);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.sampler), Some(1));
    let r = control(&mut app, &ctx, "ui.context.choose", json!({"id": SAMPLER_DELETE}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(samplers(&app), [[20.5, 20.5], [180.5, 90.5]], "#3 became #2");
    assert!(app.ui.canvas_tool_menu.is_none(), "choosing closes the menu");
    let listed = app.run("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(listed["samplers"][1]["number"], json!(2));
}

/// Delete never removes a different sampler when the list changes while the menu is open: one
/// deleted or moved meanwhile, or another document with a sampler at the same index and spot.
#[test]
fn delete_never_removes_another_sampler_after_the_list_changes_under_the_menu() {
    let (mut app, ctx) = app();
    app.run("view.colorSamplers.add", json!({"x": 180, "y": 90})).unwrap();
    right_click(&mut app, &ctx, 120.0, 60.0);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.sampler), Some(1));
    // An agent deletes #1 while the menu is open: index 1 now names the old #3.
    app.run("view.colorSamplers.delete", json!({"index": 0})).unwrap();
    let r = control(&mut app, &ctx, "ui.context.choose", json!({"id": SAMPLER_DELETE}));
    assert_eq!(r["ok"], json!(false), "Delete greys out: {r}");
    assert_eq!(samplers(&app), [[120.5, 60.5], [180.5, 90.5]], "no other sampler is deleted");
    // The menu itself closes on the next frame.
    right_click(&mut app, &ctx, 120.0, 60.0);
    app.run("view.colorSamplers.move", json!({"index": 0, "x": 30, "y": 30})).unwrap();
    ctx.run_ui(Default::default(), |ui| crate::canvas_tool_menu::show(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.ui.canvas_tool_menu.is_none(), "a moved sampler closes the menu");
    // Another document with a sampler at the same index and spot: still not this menu's sampler.
    right_click(&mut app, &ctx, 180.0, 90.0);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.sampler), Some(1));
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (x, y) in [(20, 20), (180, 90)] {
        app.run("view.colorSamplers.add", json!({"x": x, "y": y})).unwrap();
    }
    let menu = app.ui.canvas_tool_menu.clone().expect("the menu is still open");
    assert!(!crate::canvas_tool_menu::entry_enabled(&app, &menu, SAMPLER_DELETE), "another document's sampler");
}

#[test]
fn readout_rows_do_nothing_until_samplers_have_readout_modes() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 20.0, 20.0);
    let before = app.session.active().unwrap().revision;
    let r = control(&mut app, &ctx, "ui.context.choose", json!({"id": "colorSampler.readout.hsl"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert_eq!(app.session.active().unwrap().revision, before);
    assert_eq!(samplers(&app).len(), 2);
}

/// A sampler gone while the menu is open, a non-finite point, or no document at all: Delete
/// greys out and nothing panics.
#[test]
fn a_stale_sampler_or_odd_point_fails_gracefully() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, 120.0, 60.0);
    // The sampler went away while the menu was open: Delete greys out instead of removing another.
    app.run("view.colorSamplers.clear", json!({})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!crate::canvas_tool_menu::entry_enabled(&app, &menu, SAMPLER_DELETE));
    crate::canvas_tool_menu::choose(&mut app, &ctx, SAMPLER_DELETE);
    assert_eq!(samplers(&app), [[5.5, 5.5]]);
    for p in [[f64::NAN, 5.0], [f64::INFINITY, 5.0], [5.0, f64::NEG_INFINITY]] {
        assert_eq!(super::sampler_at(&app, p), None);
    }
    // No document: the menu opens greyed, nothing panics.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ColorSampler;
    assert_eq!(super::sampler_at(&empty, [1.0, 1.0]), None);
    if crate::canvas_tool_menu::open_at(&mut empty, &ctx, Tool::ColorSampler, [10.0, 10.0], Some([1.0, 1.0])) {
        let menu = empty.ui.canvas_tool_menu.clone().unwrap();
        assert!(SAMPLER_MENU.iter().flatten().all(|(_, id)| !crate::canvas_tool_menu::entry_enabled(&empty, &menu, id)));
        crate::canvas_tool_menu::choose(&mut empty, &ctx, SAMPLER_DELETE);
    }
}

#[test]
fn other_tools_keep_their_menus() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::RectMarquee;
    right_click(&mut app, &ctx, 20.0, 20.0);
    let menu = app.ui.canvas_tool_menu.clone().expect("the selection menu");
    assert_eq!(crate::canvas_tool_menu::rows(&menu), crate::canvas_tool_menu::NO_SELECTION_MENU);
    app.ui.canvas_tool_menu = None;
    app.ui.tool = Tool::Pen;
    right_click(&mut app, &ctx, 20.0, 20.0);
    let menu = app.ui.canvas_tool_menu.clone().expect("the path menu");
    assert_eq!(crate::canvas_tool_menu::rows(&menu), crate::canvas_tool_menu::PEN_MENU);
    app.ui.canvas_tool_menu = None;
    // Outside the image the Color Sampler keeps the pasteboard menu.
    app.ui.tool = Tool::ColorSampler;
    right_click(&mut app, &ctx, 250.0, 40.0);
    assert!(app.ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard));
}

/// A real right-click on the canvas over sampler #2 opens the menu on it.
#[test]
fn canvas_right_click_opens_the_menu_on_the_sampler() {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app().0
    });
    h.run_steps(5);
    let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
    let at = xf.to_screen(120.5, 60.5);
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("the Color Sampler menu opens");
    assert_eq!((menu.tool, menu.sampler), (Tool::ColorSampler, Some(1)));
    assert_eq!(h.state().session.active().unwrap().color_samplers.len(), 2, "no sampler was placed or moved");
}
