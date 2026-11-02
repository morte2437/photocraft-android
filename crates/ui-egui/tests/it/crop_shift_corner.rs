//! Crop tool: ⇧-dragging a frame corner keeps its proportions by following the pointer along the
//! frame's diagonal, as Photoshop does (#10871).
//!
//! Measured in Photoshop: 800 × 500 document, frame = whole canvas, free ratio, Auto Center
//! Preview off, ⇧-drag of the bottom-right corner by (−100, −50) px commits 706 × 441 (the drag
//! projected onto (800, 500) shortens the frame by 11.8 %). PhotoCraft let the larger relative
//! extent win and committed 720 × 450.
//!
//! Driven through `ui.pointer`, as an agent or the canvas sends the drag; no GPU needed.

use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use serde_json::{Value, json};

/// An 800 × 500 document at 100 % in a canvas it fits, the Crop tool's default frame up.
fn crop_app(auto_center: bool) -> PhotocraftApp {
    let doc = Document::with_background("crop", Size::new(800, 500), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, Default::default());
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let v = &mut app.ui.views[0];
    (v.zoom, v.center, v.fit_pending, v.rotation) = (1.0, [400.0, 250.0], false, 0.0);
    app.ui.tool_options.crop_shield.auto_center_preview = auto_center;
    pointer(&mut app, json!({"tool": "Crop", "events": []}));
    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "the default frame is the whole canvas");
    app
}

fn pointer(app: &mut PhotocraftApp, params: Value) {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("ui.pointer", params);
    let Outcome::Done(v) = handle(app, &ctx, &req) else { panic!("ui.pointer didn't finish") };
    assert_eq!(v["ok"], true, "{v}");
}

/// Drags the bottom-right corner by `d` in steps, holding `mods`, and commits with ↵.
fn drag_corner_and_commit(app: &mut PhotocraftApp, d: [f64; 2], mods: Value) -> (u32, u32) {
    let (x, y) = (800.0, 500.0);
    let mut events = vec![json!({"kind": "down", "x": x, "y": y})];
    events.extend((1..=4).map(|i| {
        let t = f64::from(i) / 4.0;
        json!({"kind": "move", "x": x + d[0] * t, "y": y + d[1] * t})
    }));
    events.push(json!({"kind": "up", "x": x + d[0], "y": y + d[1]}));
    let mut params = mods;
    params["events"] = Value::Array(events);
    pointer(app, params);
    photocraft_ui_egui::canvas::commit_crop(app);
    let size = app.session.active().expect("a document").doc.size;
    (size.width, size.height)
}

fn assert_near(got: (u32, u32), want: (u32, u32), what: &str) {
    let off = |a: u32, b: u32| a.abs_diff(b);
    assert!(off(got.0, want.0) <= 1 && off(got.1, want.1) <= 1, "{what}: committed {got:?}, want {want:?} ±1 px");
}

#[test]
fn shift_corner_drag_follows_the_frames_diagonal() {
    let mut app = crop_app(false);
    let got = drag_corner_and_commit(&mut app, [-100.0, -50.0], json!({"shift": true}));
    assert_near(got, (706, 441), "⇧ drag by (−100, −50)");
}

#[test]
fn shift_corner_drag_outward_past_the_canvas_follows_the_diagonal() {
    let mut app = crop_app(false);
    // (100, 50) onto (800, 500) grows the frame by 11.8 %: 894.4 × 559.0, past the canvas.
    let got = drag_corner_and_commit(&mut app, [100.0, 50.0], json!({"shift": true}));
    assert_near(got, (894, 559), "⇧ drag by (100, 50)");
}

#[test]
fn shift_corner_drag_keeps_the_proportions_with_auto_center_preview() {
    let mut app = crop_app(true);
    let (w, h) = drag_corner_and_commit(&mut app, [-100.0, -50.0], json!({"shift": true}));
    assert!(w < 800 && h < 500, "the frame shrank: {w} × {h}");
    assert!((f64::from(w) / f64::from(h) - 1.6).abs() < 0.01, "8 : 5 kept: {w} × {h}");
}

#[test]
fn the_same_drag_without_shift_or_with_alt_is_unchanged() {
    // No ⇧: the corner follows the pointer freely, 700 × 450 as in Photoshop.
    let mut app = crop_app(false);
    assert_near(drag_corner_and_commit(&mut app, [-100.0, -50.0], json!({})), (700, 450), "no ⇧");
    // ⌥ ⇧ (not measured, today's rule): about the centre, 600 × 400 grows to 8 : 5 as 640 × 400.
    let mut app = crop_app(false);
    assert_near(drag_corner_and_commit(&mut app, [-100.0, -50.0], json!({"shift": true, "alt": true})), (640, 400), "⌥ ⇧");
}
