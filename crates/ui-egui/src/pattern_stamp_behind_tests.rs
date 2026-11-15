//! #12879: the Pattern Stamp's Mode list has Behind right after Dissolve (and no Clear), in the
//! options bar's dropdown and the Shift+right-click menu; greyed on the Background. Picking it
//! makes the stamp paint only where the layer is transparent; picking another mode leaves it.
//! The Clone Stamp's and the History Brush's lists are not changed here.

use egui::{Context, Event, Key, Modifiers, RawInput};
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use super::{Item, current, enabled, engine_call, items, labels, pattern_stamp_choices, pattern_stamp_pick};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::Tool;

/// The reference application's Pattern Stamp Mode list, top to bottom.
const MODES: [&str; 28] = [
    "Normal",
    "Dissolve",
    "Behind",
    "Darken",
    "Multiply",
    "Color Burn",
    "Linear Burn",
    "Darker Color",
    "Lighten",
    "Screen",
    "Color Dodge",
    "Linear Dodge (Add)",
    "Lighter Color",
    "Overlay",
    "Soft Light",
    "Hard Light",
    "Vivid Light",
    "Linear Light",
    "Pin Light",
    "Hard Mix",
    "Difference",
    "Exclusion",
    "Subtract",
    "Divide",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
];

/// A 64 × 64 document with the Pattern Stamp; with `layer`, a layer above the Background whose
/// left half is red and whose right half is transparent.
fn app(layer: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    if layer {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
        app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    app.ui.tool = Tool::PatternStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.sync_views();
    app
}

/// One control-channel call, answered at once.
fn call(app: &mut PhotocraftApp, ctx: &Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// Shift+right-click on the canvas: the tool's Mode menu.
fn open_menu(app: &mut PhotocraftApp, ctx: &Context) {
    call(
        app,
        ctx,
        "ui.pointer",
        json!({"button": "right", "modifiers": {"shift": true}, "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "up", "x": 20, "y": 20}]}),
    );
    assert!(app.ui.brush_mode_menu.is_some(), "the Mode menu opened");
}

/// Opens the menu and chooses the row `label`; the call's reply.
fn choose(app: &mut PhotocraftApp, ctx: &Context, label: &str) -> Value {
    open_menu(app, ctx);
    call(app, ctx, "ui.context.choose", json!({"id": label}))
}

/// Shift+Alt+`key` through the shortcut handler.
fn key(app: &mut PhotocraftApp, ctx: &Context, key: Key) {
    let modifiers = Modifiers::SHIFT | Modifiers::ALT;
    let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }];
    ctx.begin_pass(RawInput { events, ..Default::default() });
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// The last journaled command.
fn last_call(app: &PhotocraftApp) -> (String, Value) {
    app.session.journal.last().cloned().unwrap()
}

/// The active layer's pixel as straight RGBA.
fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// One Pattern Stamp stroke across both halves of the layer, as a drag commits it.
fn stamp(app: &mut PhotocraftApp) {
    assert!(crate::retouch_ui::finish_stroke(app, Tool::PatternStamp, &[[12.0, 32.0, 1.0], [52.0, 32.0, 1.0]], Modifiers::default(), false));
}

/// Both lists are the reference application's, in order.
#[test]
fn the_mode_lists_have_behind_after_dissolve_and_no_clear() {
    // The options bar's dropdown.
    let dropdown: Vec<&str> = pattern_stamp_choices().iter().map(|c| c.1).collect();
    assert_eq!(dropdown, MODES);
    assert_eq!(pattern_stamp_choices()[2].0, (None, true), "Behind");
    // The Shift+right-click menu: Edit Brush…, then the same list.
    let menu = labels(Tool::PatternStamp);
    assert_eq!(menu.first(), Some(&"Edit Brush…"));
    assert_eq!(menu[1..], MODES);
    // Behind sits before the first separator of the blend groups.
    let rows = items(Tool::PatternStamp);
    let at = rows.iter().position(|i| *i == Item::Behind).unwrap();
    assert_eq!(rows[at - 1], Item::Blend(BlendMode::Dissolve));
    assert_eq!((rows[at + 1], rows[at + 2]), (Item::Separator, Item::Blend(BlendMode::Darken)));
    assert!(!rows.contains(&Item::Clear));
    // What `ui.inspect` reports for the open menu.
    let ctx = Context::default();
    let mut app = app(true);
    open_menu(&mut app, &ctx);
    let inspect = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(inspect["result"]["brushModeMenu"]["choices"], json!(menu), "{inspect}");
}

/// The Clone Stamp (#12602) and the History Brush (#12849) are separate items.
#[test]
fn only_the_pattern_stamp_gains_behind() {
    for tool in [Tool::CloneStamp, Tool::HistoryBrush, Tool::Healing, Tool::Gradient, Tool::PaintBucket] {
        assert!(!items(tool).contains(&Item::Behind), "{tool:?}");
        assert!(engine_call(&app(true), tool, Item::Behind).is_none(), "{tool:?}");
    }
}

/// Behind lives in the Pattern Stamp's own brush, through `tools.setBrush`.
#[test]
fn choosing_behind_sets_it_on_the_tools_brush_and_another_mode_leaves_it() {
    let ctx = Context::default();
    let mut app = app(true);
    assert!(enabled(&app, Item::Behind));
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Blend(BlendMode::Normal)));
    assert_eq!(choose(&mut app, &ctx, "Behind")["ok"], true);
    assert!(app.session.tools.brush.behind);
    assert_eq!(last_call(&app), ("tools.setBrush".into(), json!({"brush": {"mode": "Normal", "behind": true}})));
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Behind), "the checked row");
    assert_eq!(crate::retouch_ui::pattern_stamp_params(&app, &[[1.0, 1.0, 1.0]])["mode"], "Behind");
    // Another mode leaves Behind, in the same one command.
    let n = app.session.journal.len();
    assert_eq!(choose(&mut app, &ctx, "Multiply")["ok"], true);
    assert_eq!(app.session.journal.len(), n + 1);
    assert_eq!(last_call(&app), ("tools.setBrush".into(), json!({"brush": {"mode": "Multiply", "behind": false}})));
    assert!(!app.session.tools.brush.behind);
    assert_eq!(current(&app, Tool::PatternStamp), Some(Item::Blend(BlendMode::Multiply)));
    assert_eq!(crate::retouch_ui::pattern_stamp_params(&app, &[[1.0, 1.0, 1.0]])["mode"], "Multiply");
    // Behind is the Pattern Stamp's own: the Brush keeps its mode.
    choose(&mut app, &ctx, "Behind");
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert!(!app.session.tools.brush.behind, "the Brush's own brush");
    app.ui.tool = Tool::PatternStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert!(app.session.tools.brush.behind, "kept with the Pattern Stamp");
}

/// Shift+Alt+Q picks Behind and another mode's key leaves it, as for the Brush.
#[test]
fn the_blend_keys_pick_and_leave_behind() {
    let ctx = Context::default();
    let mut app = app(true);
    key(&mut app, &ctx, Key::Q);
    assert!(app.session.tools.brush.behind, "Shift+Alt+Q");
    assert_eq!(last_call(&app), ("tools.setBrush".into(), json!({"brush": {"mode": "Normal", "behind": true}})));
    key(&mut app, &ctx, Key::M);
    assert!(!app.session.tools.brush.behind, "Shift+Alt+M leaves Behind");
    assert_eq!(app.session.tools.brush.mode, BlendMode::Multiply);
    // Clear is not one of the Pattern Stamp's modes.
    let n = app.session.journal.len();
    key(&mut app, &ctx, Key::R);
    assert_eq!(app.session.journal.len(), n);
}

/// The Background has no transparency: the row is greyed and the key does nothing.
#[test]
fn behind_is_greyed_on_the_background() {
    let ctx = Context::default();
    let mut app = app(false);
    assert!(!enabled(&app, Item::Behind));
    assert!(enabled(&app, Item::Blend(BlendMode::Multiply)));
    let n = app.session.journal.len();
    assert_ne!(choose(&mut app, &ctx, "Behind")["ok"], true, "a greyed row can't be chosen");
    key(&mut app, &ctx, Key::Q);
    assert_eq!(app.session.journal.len(), n);
    assert!(!app.session.tools.brush.behind);
    let inspect = call(&mut app, &ctx, "ui.inspect", json!({}));
    let rows = inspect["result"]["brushModeMenu"]["items"].as_array().cloned().unwrap_or_default();
    let behind = rows.iter().find(|r| r["label"] == "Behind").unwrap();
    assert_eq!(behind["enabled"], false, "{behind}");
}

/// A stroke committed from the tool, Aligned on and off.
#[test]
fn a_behind_stroke_paints_only_the_transparent_half() {
    for aligned in [true, false] {
        let ctx = Context::default();
        let mut app = app(true);
        app.ui.tool_options.pattern_stamp_aligned = aligned;
        app.run("tools.setBrush", json!({"brush": {"size": 16.0, "hardness": 1.0}})).unwrap();
        let red = pixel(&app, 16, 32);
        assert_eq!(pixel(&app, 48, 32)[3], 0.0);
        choose(&mut app, &ctx, "Behind");
        let steps = app.session.active().unwrap().history.past_len();
        stamp(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "aligned {aligned}: one History step");
        assert!((0..32).all(|x| pixel(&app, x, 32) == red), "aligned {aligned}: the existing pixels are unchanged");
        assert!(pixel(&app, 48, 32)[3] > 0.99, "aligned {aligned}: the pattern fills the transparency");
        // In Normal the same stroke paints over the red.
        choose(&mut app, &ctx, "Normal");
        stamp(&mut app);
        assert_ne!(pixel(&app, 16, 32), red, "aligned {aligned}: Normal paints over");
    }
}

/// The whole app drawn, with the Pattern Stamp on a new document; with `layer`, on a transparent
/// layer above the Background.
fn harness(layer: bool) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        if layer {
            app.run("layer.new.layer", json!({})).unwrap();
        }
        app.ui.tool = Tool::PatternStamp;
        app
    });
    h.run_steps(8);
    h
}

/// The options bar's Mode dropdown: the highest one showing `Normal` (the Layers panel's is lower).
fn mode_dropdown(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> egui::Rect {
    use egui_kittest::kittest::Queryable;
    h.query_all_by_role(egui::accesskit::Role::ComboBox)
        .filter(|n| n.value().as_deref() == Some("Normal"))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap()
}

/// Opens the options bar's Mode dropdown with a click.
fn open_dropdown(h: &mut egui_kittest::Harness<'_, PhotocraftApp>) {
    use egui_kittest::kittest::Queryable;
    let top = mode_dropdown(h).top();
    h.query_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.value().as_deref() == Some("Normal") && n.rect().top() == top).unwrap().click();
    h.run_steps(4);
}

/// One wheel notch down (the next mode) over the point `at`.
fn wheel_down(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    wheel_notches_down(h, at, 1.0);
}

/// `notches` wheel notches down in one event over the point `at`.
fn wheel_notches_down(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2, notches: f32) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -notches),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(3);
}

/// On the Background the options bar's dropdown greys Behind: the row is disabled, a click on it
/// picks nothing, and the wheel steps from Dissolve straight to Darken.
#[test]
fn the_options_bar_dropdown_greys_behind_on_the_background() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = harness(false);
    let at = mode_dropdown(&h).center();
    open_dropdown(&mut h);
    assert!(h.get_by_label("Behind").accesskit_node().is_disabled(), "Behind is greyed");
    assert!(!h.get_by_label("Dissolve").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Darken").accesskit_node().is_disabled());
    let n = h.state().session.journal.len();
    h.get_by_label("Behind").click();
    h.run_steps(4);
    assert_eq!(h.state().session.journal.len(), n, "a greyed entry picks nothing");
    assert!(!h.state().session.tools.brush.behind);
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Normal);
    // The wheel passes it.
    let mut h = harness(false);
    wheel_down(&mut h, at);
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Dissolve);
    wheel_down(&mut h, at);
    assert!(!h.state().session.tools.brush.behind, "the wheel steps past the greyed Behind");
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Darken);
    // Several notches at once: the greyed entry costs none of them (Dissolve, Darken, Multiply).
    let mut h = harness(false);
    wheel_notches_down(&mut h, at, 3.0);
    assert!(!h.state().session.tools.brush.behind);
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Multiply);
}

/// On a pixel layer the dropdown's Behind is enabled and a click picks it.
#[test]
fn the_options_bar_dropdown_offers_behind_on_a_pixel_layer() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = harness(true);
    open_dropdown(&mut h);
    assert!(!h.get_by_label("Behind").accesskit_node().is_disabled(), "Behind is enabled");
    h.get_by_label("Behind").click();
    h.run_steps(4);
    assert!(h.state().session.tools.brush.behind, "picked");
    assert_eq!(current(h.state(), Tool::PatternStamp), Some(Item::Behind));
}

/// What a pick stores: Behind when it is enabled; with it greyed, the mode one step further the
/// same way, whichever way the wheel or a key came.
#[test]
fn a_pick_that_lands_on_a_greyed_behind_steps_on() {
    let (normal, dissolve, behind, darken) =
        ((Some(BlendMode::Normal), false), (Some(BlendMode::Dissolve), false), (None, true), (Some(BlendMode::Darken), false));
    assert_eq!(pattern_stamp_pick(dissolve, behind, true), behind);
    assert_eq!(pattern_stamp_pick(dissolve, behind, false), darken, "down");
    assert_eq!(pattern_stamp_pick(normal, behind, false), darken, "two notches down");
    assert_eq!(pattern_stamp_pick(darken, behind, false), dissolve, "up");
    assert_eq!(pattern_stamp_pick(normal, darken, false), darken, "other modes are picked as they are");
    // Behind chosen earlier on a pixel layer stays until another mode is picked.
    assert_eq!(pattern_stamp_pick(behind, dissolve, false), dissolve);
}
