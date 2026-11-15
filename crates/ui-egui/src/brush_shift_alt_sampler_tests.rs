//! #11890: Shift+Alt+click with a painting tool that has the Alt eyedropper adds a colour sampler
//! at the clicked pixel, as the Color Sampler tool places one: the foreground colour stays and
//! nothing is painted. Alt+click without Shift still sets the foreground; plain clicks paint.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const SHIFT_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };

/// A 400 × 300 document, a red layer to sample, blue foreground.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.ui.tool = tool;
    app.sync_views();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    // A click's own jitter: a move or two on the spot.
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn samplers(app: &mut PhotocraftApp) -> Vec<Value> {
    let r = app.run("view.colorSamplers.list", json!({})).unwrap();
    r.get("samplers").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn foreground(app: &PhotocraftApp) -> [f32; 4] {
    app.session.tools.foreground
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn shift_alt_click_adds_a_sampler_with_every_alt_eyedropper_tool() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::Gradient, Tool::PaintBucket, Tool::BackgroundEraser] {
        let mut app = app(tool);
        let fg = foreground(&app);
        let before = steps(&app);
        click(&mut app, 199.7, 120.2, SHIFT_ALT);
        let got = samplers(&mut app);
        // The point the Color Sampler tool's command places for the same click.
        let mut twin = self::app(tool);
        twin.run("view.colorSamplers.add", json!({"x": 199.7, "y": 120.2})).unwrap();
        assert_eq!(got, samplers(&mut twin), "{tool:?}: one sampler at the clicked pixel");
        assert_eq!(got.len(), 1, "{tool:?}");
        assert_eq!(foreground(&app), fg, "{tool:?}: the foreground stays");
        assert_eq!(steps(&app), before, "{tool:?}: nothing painted");
        assert!(!app.alt_sampling, "{tool:?}: the gesture ended");
    }
}

#[test]
fn at_the_limit_the_click_adds_nothing_and_says_so() {
    let mut app = app(Tool::Brush);
    for i in 0..10 {
        app.run("view.colorSamplers.add", json!({"x": 10 + i * 20, "y": 10})).unwrap();
    }
    let (fg, before) = (foreground(&app), steps(&app));
    click(&mut app, 200.0, 150.0, SHIFT_ALT);
    assert_eq!(samplers(&mut app).len(), 10);
    assert_eq!(steps(&app), before, "nothing painted");
    assert!(app.ui.status.contains("limit"), "the Color Sampler tool's limit message: {:?}", app.ui.status);
    assert!(!app.ui.status_error, "reported as the Color Sampler tool does");
    assert_eq!(foreground(&app), fg);
}

#[test]
fn alt_click_still_samples_and_a_plain_click_still_paints() {
    let mut app = app(Tool::Brush);
    click(&mut app, 50.0, 50.0, Modifiers::ALT);
    assert_eq!(foreground(&app)[..3], [1.0, 0.0, 0.0], "Alt+click picks the red under the pointer");
    assert!(samplers(&mut app).is_empty());
    let before = steps(&app);
    click(&mut app, 80.0, 80.0, Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "a plain click paints a dab");
    assert!(samplers(&mut app).is_empty());
}

#[test]
fn a_shift_alt_click_off_the_canvas_adds_nothing_and_never_panics() {
    let mut app = app(Tool::Brush);
    let fg = foreground(&app);
    for (x, y) in [(-20.0, 50.0), (450.0, 50.0), (f64::NAN, 10.0), (10.0, f64::INFINITY)] {
        click(&mut app, x, y, SHIFT_ALT);
    }
    assert!(samplers(&mut app).is_empty());
    assert_eq!(foreground(&app), fg);
}

#[test]
fn shift_alt_click_on_a_sampler_samples_the_foreground_as_before() {
    // A press on an existing point wasn't measured, so it keeps the Alt eyedropper's sample.
    let mut app = app(Tool::Brush);
    app.run("view.colorSamplers.add", json!({"x": 120, "y": 80})).unwrap();
    let before = samplers(&mut app);
    click(&mut app, 121.0, 80.5, SHIFT_ALT);
    assert_eq!(samplers(&mut app), before, "no second point on the first");
    assert_eq!(foreground(&app)[..3], [1.0, 0.0, 0.0], "the red under the pointer, as an Alt+click");
}

#[test]
fn a_shift_alt_click_off_the_canvas_reports_nothing() {
    // The Color Sampler tool places no point off the canvas and says nothing; so does this press.
    let mut app = app(Tool::Brush);
    app.ui.status = "Ready".into();
    for (x, y) in [(-20.0, 50.0), (450.0, 50.0), (f64::NAN, 10.0)] {
        click(&mut app, x, y, SHIFT_ALT);
        assert_eq!(app.ui.status, "Ready", "({x}, {y}): the status bar is left as it was");
    }
}

#[test]
fn a_shift_alt_click_refused_for_another_reason_keeps_the_error_styling() {
    // Only the limit is the Color Sampler tool's plain report. A Pen draft whose commit failed,
    // left in another document, refuses every command until it is dealt with: that stays an error.
    let mut app = app(Tool::Pen);
    for x in [30.0, 1e40] {
        tool_event(&mut app, ToolEvent::Down { x, y: 60.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x, y: 60.0 }, Modifiers::NONE);
    }
    crate::vector_ui::pen_commit(&mut app, false);
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.commit_failed), "a failed Pen draft");
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = Tool::Brush;
    // The Pen failure styled the status bar as an error already: only the click may set it again.
    app.ui.status_error = false;
    click(&mut app, 200.0, 150.0, SHIFT_ALT);
    assert!(samplers_of(&app).is_empty(), "nothing added");
    assert!(!app.ui.status.is_empty(), "the refusal is reported");
    assert!(app.ui.status_error, "and keeps its error styling: {:?}", app.ui.status);
}

#[test]
fn a_full_document_refused_for_another_reason_keeps_the_error_styling() {
    // Ten points and a failed Pen draft in another document: the refusal is the Pen's, not the
    // limit's, so it stays an error.
    let mut app = app(Tool::Pen);
    for x in [30.0, 1e40] {
        tool_event(&mut app, ToolEvent::Down { x, y: 60.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x, y: 60.0 }, Modifiers::NONE);
    }
    crate::vector_ui::pen_commit(&mut app, false);
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.commit_failed), "a failed Pen draft");
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    for i in 0..10 {
        app.session.execute("view.colorSamplers.add", json!({"x": 10 + i * 20, "y": 10})).unwrap();
    }
    app.ui.tool = Tool::Brush;
    app.ui.status_error = false;
    click(&mut app, 200.0, 150.0, SHIFT_ALT);
    assert_eq!(samplers_of(&app).len(), 10, "nothing added");
    assert!(!app.ui.status.contains("limit"), "the Pen's refusal, not the limit: {:?}", app.ui.status);
    assert!(app.ui.status_error, "and keeps its error styling: {:?}", app.ui.status);
}

fn samplers_of(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().map(|d| d.color_samplers.clone()).unwrap_or_default()
}
