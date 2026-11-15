//! #11914: holding Alt turns the Color Replacement tool into the Eyedropper. Alt+click and
//! Alt+drag set the foreground colour, add no history state and change no pixel; Ctrl+Alt keeps
//! its brush-resize meaning and a plain click still paints.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 400 × 300 RGB document: a green layer (30,180,60), blue foreground, the Color Replacement tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#1eb43c"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.ui.tool = Tool::ColorReplacement;
    app.sync_views();
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn alt_click_and_drag_pick_the_foreground() {
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, [300.0, 60.0], [300.0, 60.0], Modifiers::ALT);
    assert_eq!(foreground(&app), [30, 180, 60], "the green under the click");
    assert_eq!(steps(&app), before, "no history state");
    assert_eq!(pixel(&app, 300, 60), [30, 180, 60], "no pixel changed");
    // A drag samples as it goes, as the Eyedropper does.
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    drag(&mut app, [100.0, 100.0], [200.0, 150.0], Modifiers::ALT);
    assert_eq!(foreground(&app), [30, 180, 60]);
    assert_eq!(steps(&app), before);
}

#[test]
fn alt_shows_the_eyedropper_and_ctrl_alt_resizes() {
    let alt = Modifiers::ALT;
    let ctrl_alt = Modifiers { alt: true, ctrl: true, ..Modifiers::NONE };
    assert!(alt_samples(Tool::ColorReplacement, alt));
    assert!(alt_picks(Tool::ColorReplacement, alt), "the cursor shows the eyedropper while Alt is held");
    assert!(!alt_samples(Tool::ColorReplacement, ctrl_alt), "Ctrl+Alt is the brush-resize drag");
    assert!(!alt_samples(Tool::ColorReplacement, Modifiers::NONE));
}

#[test]
fn a_plain_click_still_paints() {
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, [300.0, 60.0], [310.0, 60.0], Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "a Color Replacement state");
    assert_ne!(pixel(&app, 300, 60), [30, 180, 60], "the green was replaced");
    assert_eq!(foreground(&app), [0, 0, 255]);
}
