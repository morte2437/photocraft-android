//! Crop tool: a Straighten line drawn with ⌘ (Ctrl) held turns the image but keeps the frame's
//! size (#10870), while the options-bar Straighten button keeps fitting the frame inside the
//! turned image.
//!
//! Measured in the reference application: an 800 × 500 document, the frame around the whole
//! canvas, a line rising about 10°. The button commits 633 × 396; ⌘ held commits 800 × 500 with
//! the uncovered corners in the background colour (Fill: Background). PhotoCraft fitted the frame
//! both ways.
//!
//! Driven as an agent does, through `ui.pointer` (document px), in both crop modes.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use serde_json::{Value, json};

const BLUE: [f32; 3] = [16.0 / 255.0, 80.0 / 255.0, 224.0 / 255.0];

fn app(classic: bool) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).expect("new");
    app.run("tools.setColors", json!({"background": "#1050e0"})).expect("colors");
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool_options.crop_shield.classic_mode = classic;
    control(&mut app, &ctx, "ui.set", json!({"tool": "crop"}));
    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "the default frame");
    (app, ctx)
}

fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params.clone());
    assert!(matches!(handle(app, ctx, &req), Outcome::Done(_)), "{method} {params}");
}

/// A line rising 10° to the right across the canvas, `command` held or not.
fn line(app: &mut PhotocraftApp, ctx: &egui::Context, command: bool) {
    let (a, b) = ([100.0, 300.0], [700.0, 300.0 - 600.0 * 10.0f64.to_radians().tan()]);
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": mid[0], "y": mid[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    control(app, ctx, "ui.pointer", json!({"command": command, "events": events}));
}

fn committed(app: &mut PhotocraftApp) -> (u32, u32) {
    photocraft_ui_egui::canvas::commit_crop(app);
    let size = app.session.active().expect("document").doc.size;
    (size.width, size.height)
}

#[test]
fn a_command_held_straighten_line_keeps_the_frame_size() {
    for classic in [false, true] {
        let (mut app, ctx) = app(classic);
        line(&mut app, &ctx, true);
        // The frame turns (counter-clockwise: the line rises to the right) and stays put.
        assert!((photocraft_ui_egui::crop_ui::angle(&app) + 10.0).abs() < 1e-6, "classic {classic}: {}", app.ui.crop_angle);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "classic {classic}");
        assert!(photocraft_ui_egui::crop_ui::pending(&app), "classic {classic}: a pending edit");
        assert_eq!(committed(&mut app), (800, 500), "classic {classic}");
        // The corners the turn uncovered take the background colour; the middle is the image.
        let doc = &app.session.active().expect("document").doc;
        let flat = photocraft_compose::flatten(doc);
        let at = |x: usize, y: usize| flat.px[y * 800 + x];
        for (x, y) in [(0, 0), (799, 0), (0, 499), (799, 499)] {
            let p = at(x, y);
            assert!(BLUE.iter().zip(p).all(|(w, g)| (w - g).abs() < 0.02), "classic {classic}: corner ({x}, {y}) {p:?}");
        }
        assert!(at(400, 250)[..3].iter().all(|&v| v > 0.98), "classic {classic}: the white image in the middle");
    }
}

#[test]
fn the_straighten_button_still_fits_the_frame() {
    for classic in [false, true] {
        let (mut app, ctx) = app(classic);
        photocraft_ui_egui::crop_straighten::toggle(&mut app);
        line(&mut app, &ctx, false);
        assert!((photocraft_ui_egui::crop_ui::angle(&app) + 10.0).abs() < 1e-6, "classic {classic}");
        assert!(!photocraft_ui_egui::crop_straighten::mode(&app), "classic {classic}: one line per click");
        let (w, h) = committed(&mut app);
        assert!(w.abs_diff(633) <= 1 && h.abs_diff(396) <= 1, "classic {classic}: {w} × {h}");
    }
}
