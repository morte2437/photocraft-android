//! Pattern Stamp and Background Eraser strokes draw live (TOOL-213-7, #3754): the canvas shows
//! the engine's real result while dragging, the release commits exactly those pixels as one
//! History step, and the commit is what the one-shot command paints for the same stroke. The Spot
//! Healing Brush shows the area it will heal and heals on release, as Photoshop does.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, display_doc, strokes_live, tool_event};
use crate::state::Tool;

const STROKE: [(f64, f64); 4] = [(12.0, 20.0), (30.0, 24.0), (48.0, 18.0), (66.0, 22.0)];

/// An 80×40 document at `depth` bits: a white Background with a black block on its left half,
/// with `tool` picked (14 px, soft, no smoothing).
fn app(depth: u32, tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 40, "depth": depth})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 14, "hardness": 0.5, "pressureSize": false, "smoothing": {"amount": 0.0}}})).unwrap();
    app
}

/// The active layer's pixels (or its mask) in `doc`.
fn surface(doc: &photocraft_doc::Document, app: &PhotocraftApp, mask: bool) -> Vec<f32> {
    let l = doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
    let s = if mask { &l.mask.as_ref().unwrap().surface } else { l.surface().unwrap() };
    s.read_region(photocraft_geom::Rect::new(0, 0, 80, 40))
}

/// Drag `STROKE` with the picked tool. Returns what the canvas showed at the last move (the
/// targeted surface) and the params the release committed.
fn live_drag(app: &mut PhotocraftApp, cmd: &str, mask: bool) -> (Vec<f32>, Value) {
    let (before, steps) = (app.session.active().unwrap().doc.clone(), app.session.active().unwrap().history.past_len());
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: STROKE[0].0, y: STROKE[0].1, pressure: 1.0 }, m);
    for &(x, y) in &STROKE[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    assert!(app.live_stroke.is_some(), "{cmd}: the canvas shows the stroke while dragging");
    let shown = display_doc(app, 0).0;
    let shown = surface(&shown, app, mask);
    assert!(shown != surface(&before, app, mask), "{cmd}: the preview paints as it goes");
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &st.doc) && st.history.past_len() == steps, "{cmd}: nothing committed while dragging");
    let (x, y) = STROKE[3];
    tool_event(app, ToolEvent::Up { x, y }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.live_stroke.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "{cmd}: one History step");
    let params = app.session.journal.iter().rev().find(|(name, _)| name == cmd).unwrap().1.clone();
    (shown, params)
}

/// The release committed what the drag showed, and the one-shot command gives the same pixels
/// again; without the live stroke's seed too where nothing is jittered (`keep_seed` false: a
/// brush without jitter paints the same whatever the seed; Impressionist jitters every dab).
fn check_commit(app: &mut PhotocraftApp, cmd: &str, shown: &[f32], mut params: Value, mask: bool, keep_seed: bool) {
    let committed = app.session.active().unwrap().doc.clone();
    assert!(surface(&committed, app, mask) == shown, "{cmd}: the release commits the pixels the drag showed");
    app.run("edit.undo", json!({})).unwrap();
    if !keep_seed {
        params.as_object_mut().unwrap().remove("seed");
    }
    app.run(cmd, params).unwrap();
    assert!(surface(&app.session.active().unwrap().doc, app, mask) == shown, "{cmd}: the one-shot command paints the same");
}

#[test]
fn pattern_stamp_draws_live_and_commits_what_it_showed() {
    for depth in [8, 16] {
        for impressionist in [false, true] {
            let mut app = app(depth, Tool::PatternStamp);
            assert!(strokes_live(Tool::PatternStamp));
            app.ui.tool_options.pattern_stamp_impressionist = impressionist;
            app.ui.tool_options.pattern_stamp_scale = 150.0;
            let (shown, params) = live_drag(&mut app, "paint.patternStamp", false);
            assert_eq!(params["impressionist"], json!(impressionist));
            check_commit(&mut app, "paint.patternStamp", &shown, params, false, impressionist);
        }
    }
}

#[test]
fn a_live_pattern_stamp_on_the_layer_mask_paints_the_mask() {
    let mut app = app(8, Tool::PatternStamp);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    let pixels = surface(&app.session.active().unwrap().doc, &app, false);
    let (shown, params) = live_drag(&mut app, "paint.patternStamp", true);
    assert_eq!(params["target"], json!("mask"));
    assert!(surface(&app.session.active().unwrap().doc, &app, false) == pixels, "the layer's pixels are untouched");
    check_commit(&mut app, "paint.patternStamp", &shown, params, true, false);
}

#[test]
fn background_eraser_draws_live_and_commits_what_it_showed() {
    for depth in [8, 16] {
        for sampling in ["continuous", "once"] {
            let mut app = app(depth, Tool::BackgroundEraser);
            assert!(strokes_live(Tool::BackgroundEraser));
            app.ui.tool_options.bg_sampling = sampling.into();
            let (shown, params) = live_drag(&mut app, "paint.backgroundEraser", false);
            // The release turned the Background into a normal layer, as the command does.
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Layer 0");
            check_commit(&mut app, "paint.backgroundEraser", &shown, params, false, false);
        }
    }
}

#[test]
fn a_refused_background_eraser_stroke_shows_nothing_and_reports_on_release() {
    let mut app = app(8, Tool::BackgroundEraser);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 40.0, y: 20.0, pressure: 1.0 }, m);
    assert!(app.live_stroke.is_none(), "the engine refuses the stroke, so nothing is drawn live");
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 20.0 }, m);
    assert!(app.ui.status_error && app.ui.status.contains("transparency"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

#[test]
fn spot_healing_shows_the_area_it_heals_and_heals_on_release() {
    let mut app = app(8, Tool::SpotHealing);
    assert!(!strokes_live(Tool::SpotHealing), "the heal needs the whole stroke");
    assert_eq!(crate::canvas::SPOT_HEALING_TRAIL.r(), 0, "a dark overlay, as Photoshop's");
    let steps = app.session.active().unwrap().history.past_len();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 20.0, pressure: 1.0 }, m);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 20.0 }, m);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}
