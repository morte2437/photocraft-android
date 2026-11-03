//! #6288: ⌘ (Ctrl off the Mac) over a guide is the Move tool's guide drag with every tool on
//! which ⌘ is the Move tool, and the pointer shows it (the guide's double arrow), as in
//! Photoshop. Driven through real egui input, so the held key goes through `hold_keys`.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 200})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    }
    .to_screen(x, y)
}

fn hover(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) -> egui::CursorIcon {
    h.event(egui::Event::ModifiersChanged(m));
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: f32, to: f32, m: Modifiers) {
    let p = screen(h, from, 150.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.run_steps(1);
    for k in 1..=4 {
        let q = screen(h, from + (to - from) * k as f32 / 4.0, 150.0);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    let q = screen(h, to, 150.0);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(2);
}

fn diagonal(h: &mut Harness<'static, PhotocraftApp>, a: [f32; 2], b: [f32; 2], m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    let p = screen(h, a[0], a[1]);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.run_steps(1);
    for k in 1..=4 {
        let t = k as f32 / 4.0;
        let q = screen(h, a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    let q = screen(h, b[0], b[1]);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(2);
}

/// The cursor over the guide with ⌘ held, and where a ⌘-drag from 200 to 240 leaves it.
fn command_drag(tool: Tool, locked: bool) -> (egui::CursorIcon, Vec<f64>) {
    let mut h = harness(tool);
    h.state_mut().ui.extras.lock_guides = locked;
    let c = hover(&mut h, 200.0, 150.0, Modifiers::COMMAND);
    drag(&mut h, 200.0, 240.0, Modifiers::COMMAND);
    (c, h.state().session.active().unwrap().doc.guides.vertical.clone())
}

const GUIDE: egui::CursorIcon = egui::CursorIcon::ResizeHorizontal;

#[test]
fn command_over_a_guide_shows_the_guide_cursor_and_drags_it_with_painting_tools() {
    for tool in [
        Tool::Brush,
        Tool::Eraser,
        Tool::Gradient,
        Tool::PaintBucket,
        Tool::CloneStamp,
        Tool::Eyedropper,
        Tool::Dodge,
        Tool::SpotHealing,
        Tool::Smudge,
        Tool::QuickSelection,
        Tool::RectMarquee,
        Tool::Lasso,
    ] {
        assert_eq!(command_drag(tool, false), (GUIDE, vec![240.0]), "{tool:?}");
    }
}

/// The Frame and Artboard tools took a ⌘ press as their own (a new frame or board) although ⌘
/// is the Move tool on them, so the guide cursor promised a drag that never moved the guide.
#[test]
fn command_drags_a_guide_with_the_frame_and_artboard_tools() {
    for tool in [Tool::Frame, Tool::Artboard] {
        assert_eq!(command_drag(tool, false), (GUIDE, vec![240.0]), "{tool:?}");
    }
    // Off a guide, ⌘ is the Move tool too: no frame is drawn.
    let mut h = harness(Tool::Frame);
    let layers = h.state().session.active().unwrap().doc.layers.len();
    diagonal(&mut h, [100.0, 100.0], [160.0, 140.0], Modifiers::COMMAND);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "⌘-drag with the Frame tool draws no frame");
    assert!(h.state().ui.frame_tool.drag.is_none());
    // Without ⌘ the Frame tool still draws its frame.
    diagonal(&mut h, [20.0, 20.0], [120.0, 90.0], Modifiers::NONE);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1, "a plain drag draws a frame");
}

#[test]
fn locked_guides_show_no_guide_cursor_and_do_not_move() {
    for tool in [Tool::Brush, Tool::Eraser, Tool::Frame] {
        let (cursor, guides) = command_drag(tool, true);
        assert_ne!(cursor, GUIDE, "{tool:?}");
        assert_eq!(guides, vec![200.0], "{tool:?}");
    }
}

/// Tools that keep ⌘ for themselves (Photoshop: the Pen's Direct Selection, the Hand, Zoom, Crop,
/// type, shapes, slices) neither show the guide cursor nor drag the guide.
#[test]
fn tools_with_their_own_command_keep_it() {
    for tool in [Tool::Pen, Tool::Hand, Tool::Zoom, Tool::Crop, Tool::Type, Tool::Rectangle, Tool::Slice] {
        let (cursor, guides) = command_drag(tool, false);
        assert_ne!(cursor, GUIDE, "{tool:?}");
        assert_eq!(guides, vec![200.0], "{tool:?}");
    }
}

/// Window › Modifier Keys: a latched ⌘ made the press take the guide but left the brush cursor.
#[test]
fn sticky_command_shows_the_guide_cursor() {
    for tool in [Tool::Brush, Tool::Eraser, Tool::RectMarquee] {
        let mut h = harness(tool);
        h.state_mut().ui.shell.sticky_command = true;
        assert_eq!(hover(&mut h, 200.0, 150.0, Modifiers::NONE), GUIDE, "{tool:?}");
        drag(&mut h, 200.0, 240.0, Modifiers::NONE);
        assert_eq!(h.state().session.active().unwrap().doc.guides.vertical, vec![240.0], "{tool:?}");
    }
}

/// The one decision for "a ⌘ press here takes the guide" outside the Move tool: not without ⌘,
/// not with a locked guide, not during a stroke, not for a tool that keeps ⌘.
#[test]
fn command_guide_at_decides_for_every_tool_that_moves_guides() {
    use crate::canvas::command_guide_at;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 200})).unwrap();
    let cmd = Modifiers::COMMAND;
    for tool in [Tool::Brush, Tool::Eraser, Tool::Gradient, Tool::PaintBucket, Tool::Eyedropper, Tool::Frame, Tool::Artboard, Tool::MagicWand] {
        assert_eq!(command_guide_at(&app, tool, [200.0, 100.0], cmd), Some((true, 0)), "{tool:?}");
        assert_eq!(command_guide_at(&app, tool, [200.0, 100.0], Modifiers::NONE), None, "{tool:?} without ⌘");
        assert_eq!(command_guide_at(&app, tool, [120.0, 100.0], cmd), None, "{tool:?} off the guide");
    }
    for tool in [Tool::Move, Tool::Pen, Tool::Hand, Tool::Zoom, Tool::Crop, Tool::Type, Tool::Rectangle, Tool::Slice, Tool::PathSelection] {
        assert_eq!(command_guide_at(&app, tool, [200.0, 100.0], cmd), None, "{tool:?} keeps ⌘");
    }
    app.ui.extras.lock_guides = true;
    assert_eq!(command_guide_at(&app, Tool::Brush, [200.0, 100.0], cmd), None, "locked guides");
    app.ui.extras.lock_guides = false;
    // A stroke under way keeps the pointer: ⌘ pressed mid-stroke does not take the guide.
    app.ui.tool = Tool::Brush;
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(app.drag.is_some());
    assert_eq!(command_guide_at(&app, Tool::Brush, [200.0, 100.0], cmd), None, "during a stroke");
}
