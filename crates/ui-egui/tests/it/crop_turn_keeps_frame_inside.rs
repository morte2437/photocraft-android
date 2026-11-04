//! Crop tool, default mode: turning the image keeps the frame inside the turned image (#10869).
//!
//! Measured in the reference application on an 800 × 500 document, turning by a drag outside the
//! frame with Auto Center Preview off: the frame keeps its size where it fits the turned image,
//! else moves the shortest way into it, else shrinks about its centre keeping its proportions.
//! With Ctrl (⌘) held the frame isn't adjusted and the uncovered corners get the Fill.
//! PhotoCraft never adjusted the frame, so ↵ filled the corners with the background colour.
//!
//! Driven as an agent does, through `ui.pointer` (document px as the view showed them at the
//! press).

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::crop_mode::kept_inside;
use photocraft_ui_egui::crop_ui::{angle, center, corners};
use serde_json::{Value, json};

/// The document's size.
const W: f64 = 800.0;
const H: f64 = 500.0;
/// The whole canvas, the default frame.
const CANVAS: [f64; 4] = [0.0, 0.0, W, H];

/// An 800 × 500 white document with a #1050e0 background colour, the Crop tool in the default
/// mode with Auto Center Preview off, and `frame` as the pending crop frame: the default frame
/// when it is the whole canvas, else drawn from its bottom-right corner to its top-left one (a
/// press on the default frame's corner handle would resize it).
fn app(frame: [f64; 4]) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": W, "height": H, "background": "white"})).expect("new");
    app.run("tools.setColors", json!({"background": "#1050e0"})).expect("colors");
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool_options.crop_shield.auto_center_preview = false;
    control(&mut app, &ctx, "ui.set", json!({"tool": "crop", "zoom": 1.0, "center": [W / 2.0, H / 2.0]}));
    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    assert!(!app.ui.tool_options.crop_shield.classic_mode, "Photoshop's default mode");
    if frame != CANVAS {
        let events = json!([
            {"kind": "down", "x": frame[2], "y": frame[3]},
            {"kind": "move", "x": frame[0], "y": frame[1]},
            {"kind": "up", "x": frame[0], "y": frame[1]},
        ]);
        control(&mut app, &ctx, "ui.pointer", json!({"events": events}));
    }
    assert_eq!(app.ui.crop_rect, Some(frame));
    (app, ctx)
}

/// Runs one control-channel request, which must succeed.
fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params.clone());
    assert!(matches!(handle(app, ctx, &req), Outcome::Done(_)), "{method} {params}");
}

/// The point 500 px right of the frame's centre turned `deg` degrees clockwise about it: a press
/// there is outside every frame below, and moving to it from the unturned point turns the image
/// `deg` degrees clockwise.
fn around(frame: [f64; 4], deg: f64) -> [f64; 2] {
    let c = center(frame);
    let (s, k) = deg.to_radians().sin_cos();
    [c[0] + 500.0 * k, c[1] + 500.0 * s]
}

/// A turn of the image: a press outside `frame`, then for each `(deg, modifiers)` step a move to
/// `deg` with those modifiers held (one `ui.pointer` call each, so a key can be pressed during
/// the drag); `each` runs after every step. Releases at the last step's point with its modifiers.
fn turn(app: &mut PhotocraftApp, ctx: &egui::Context, frame: [f64; 4], steps: &[(f64, Value)], mut each: impl FnMut(&PhotocraftApp, f64)) {
    let p = around(frame, 0.0);
    control(app, ctx, "ui.pointer", json!({"events": [{"kind": "down", "x": p[0], "y": p[1]}]}));
    for (deg, mods) in steps {
        let p = around(frame, *deg);
        control(app, ctx, "ui.pointer", json!({"modifiers": mods, "events": [{"kind": "move", "x": p[0], "y": p[1]}]}));
        each(app, *deg);
    }
    let (deg, mods) = steps.last().expect("a step");
    let p = around(frame, *deg);
    control(app, ctx, "ui.pointer", json!({"modifiers": mods, "events": [{"kind": "up", "x": p[0], "y": p[1]}]}));
}

/// No modifier held.
fn plain() -> Value {
    json!({})
}

/// The pending frame.
fn frame(app: &PhotocraftApp) -> [f64; 4] {
    app.ui.crop_rect.expect("a frame")
}

/// The pending frame's corners, turned with it, all lie on the canvas.
fn inside(app: &PhotocraftApp) -> bool {
    corners(frame(app), angle(app)).iter().all(|c| (-1e-6..=W + 1e-6).contains(&c[0]) && (-1e-6..=H + 1e-6).contains(&c[1]))
}

/// Where the frame's centre shows on screen.
fn on_screen(app: &PhotocraftApp) -> egui::Pos2 {
    let c = center(frame(app));
    ViewXform::active(app).expect("a view").to_screen(c[0] as f32, c[1] as f32)
}

/// ↵: commits the crop and returns the document's new size.
fn committed(app: &mut PhotocraftApp) -> (u32, u32) {
    photocraft_ui_egui::canvas::commit_crop(app);
    let size = app.session.active().expect("document").doc.size;
    (size.width, size.height)
}

/// The flattened document's four corner pixels and its middle one.
fn corners_and_middle(app: &PhotocraftApp) -> [[f32; 4]; 5] {
    let doc = &app.session.active().expect("document").doc;
    let flat = photocraft_compose::flatten(doc);
    let (w, h) = (doc.size.width as usize, doc.size.height as usize);
    let px = |x: usize, y: usize| flat.px[y * w + x];
    [px(0, 0), px(w - 1, 0), px(w - 1, h - 1), px(0, h - 1), px(w / 2, h / 2)]
}

/// The background colour #1050e0 rather than the white image: its red is 16/255. A corner that
/// the turned image covers only in part still has far more red than that.
fn background(p: [f32; 4]) -> bool {
    p[0] < 0.5 && (p[2] - 224.0 / 255.0).abs() < 0.02
}

/// White: the image.
fn white(p: [f32; 4]) -> bool {
    p[..3].iter().all(|&v| v > 0.98)
}

/// The frame's angle is `want` degrees, up to the `f32` rounding of the view the pointer is read
/// through.
fn near_angle(deg: f64, want: f64) -> bool {
    (deg - want).abs() < 1e-4
}

/// The size of the largest frame of the canvas's proportions that fits inside the canvas turned
/// `deg` degrees.
fn fitted_size(deg: f64) -> (f64, f64) {
    let (s, c) = deg.to_radians().sin_cos();
    let (s, c) = (s.abs(), c.abs());
    let k = (W / (W * c + H * s)).min(H / (W * s + H * c));
    (W * k, H * k)
}

/// The issue's first two rows: the whole-canvas frame shrinks about its centre, keeping its
/// proportions, to the largest size that fits (687 × 429 measured at 6.3°, 580 × 362 at the 15°
/// that ⇧ snaps to), live while dragging, with the box put on screen. ↵ crops to it and no corner
/// shows the background colour.
#[test]
fn turning_the_whole_canvas_frame_shrinks_it_inside_the_turned_image() {
    for (deg, shift, measured) in [(6.3, false, (687.0, 429.0)), (13.0, true, (580.0, 362.0))] {
        let what = format!("turned {deg}°, shift {shift}");
        let (mut app, ctx) = app(CANVAS);
        let screen = on_screen(&app);
        // ⇧ pressed after the press, as measured: the turn snaps to 15°.
        let steps = [(deg / 2.0, plain()), (deg, json!({"shift": shift}))];
        turn(&mut app, &ctx, CANVAS, &steps, |app, step| {
            assert!(inside(app), "{what}: at {step}° the frame {:?} turned {} leaves the image", frame(app), angle(app));
            assert!(on_screen(app).distance(screen) < 0.01, "{what}: at {step}° the box moved on screen to {:?}", on_screen(app));
        });
        let turned = if shift { 15.0 } else { deg };
        assert!(near_angle(angle(&app), -turned), "{what}: the frame's angle is {}", angle(&app));
        let (w, h) = fitted_size(angle(&app));
        let f = frame(&app);
        let (fw, fh) = (f[2] - f[0], f[3] - f[1]);
        assert!((fw - w).abs() < 1e-6 && (fh - h).abs() < 1e-6, "{what}: {fw} × {fh}, want {w} × {h}");
        assert!((fw - measured.0).abs() <= 3.0 && (fh - measured.1).abs() <= 3.0, "{what}: {fw} × {fh} is far from the reference's {measured:?}");
        let c = center(f);
        assert!((c[0] - W / 2.0).abs() < 1e-6 && (c[1] - H / 2.0).abs() < 1e-6, "{what}: centred, not at {c:?}");
        let (cw, ch) = committed(&mut app);
        assert!((f64::from(cw) - w).abs() <= 1.0 && (f64::from(ch) - h).abs() <= 1.0, "{what}: committed {cw} × {ch}");
        for p in corners_and_middle(&app) {
            assert!(!background(p), "{what}: a corner shows the background colour: {p:?}");
        }
        assert!(white(corners_and_middle(&app)[4]), "{what}: the middle is the image");
    }
}

/// The issue's Ctrl row: with Ctrl (⌘) held the frame keeps the whole canvas and ↵ fills the
/// corners the turn uncovered. Pressed part way through the turn, Ctrl gives the press frame back
/// at once.
#[test]
fn with_command_held_the_turn_keeps_the_frame() {
    for late in [false, true] {
        let (mut app, ctx) = app(CANVAS);
        let first = if late { plain() } else { json!({"command": true}) };
        turn(&mut app, &ctx, CANVAS, &[(3.0, first), (6.0, json!({"command": true}))], |app, step| {
            if late && step == 3.0 {
                assert!(frame(app) != CANVAS && inside(app), "Ctrl not held yet: the frame fits, {:?}", frame(app));
            } else {
                assert_eq!(frame(app), CANVAS, "late {late}: at {step}°");
            }
        });
        assert!(near_angle(angle(&app), -6.0), "late {late}: {}", angle(&app));
        assert_eq!(committed(&mut app), (800, 500), "late {late}");
        let [corner @ .., middle] = corners_and_middle(&app);
        for p in corner {
            assert!(background(p), "late {late}: an uncovered corner is {p:?}");
        }
        assert!(white(middle), "late {late}: the middle is the image");
    }
}

/// The issue's fourth row: a 300 × 201 frame at (100, 99) still lies inside the image turned
/// 11.3° about its centre, so it stays exactly as it was.
#[test]
fn a_frame_that_fits_the_turned_image_keeps_its_size_and_place() {
    let start = [100.0, 99.0, 400.0, 300.0];
    let (mut app, ctx) = app(start);
    turn(&mut app, &ctx, start, &[(5.0, plain()), (11.3, plain())], |app, step| {
        assert_eq!(frame(app), start, "at {step}°");
    });
    assert!(near_angle(angle(&app), -11.3), "{}", angle(&app));
    assert_eq!(committed(&mut app), (300, 201));
    for p in corners_and_middle(&app) {
        assert!(white(p), "{p:?}");
    }
}

/// The issue's last row: a 300 × 200 frame in the top-left corner, turned 11.1°, keeps its size
/// and moves the shortest way into the turned image: its turned corners end up touching the
/// canvas's left and top edges, not further in. The box stays put on screen while the image
/// slides under it.
#[test]
fn a_frame_in_the_corner_moves_in_without_shrinking() {
    let start = [0.0, 0.0, 300.0, 200.0];
    let (mut app, ctx) = app(start);
    let screen = on_screen(&app);
    turn(&mut app, &ctx, start, &[(5.0, plain()), (11.1, plain())], |app, step| {
        assert!(inside(app), "at {step}° the frame {:?} leaves the image", frame(app));
        assert!(on_screen(app).distance(screen) < 0.01, "at {step}° the box moved on screen to {:?}", on_screen(app));
    });
    let f = frame(&app);
    assert!(((f[2] - f[0]) - 300.0).abs() < 1e-9 && ((f[3] - f[1]) - 200.0).abs() < 1e-9, "the size is kept: {f:?}");
    let turned = corners(f, angle(&app));
    let left = turned.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min);
    let top = turned.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min);
    assert!(left.abs() < 1e-9 && top.abs() < 1e-9, "moved the shortest way: the turned frame's left {left}, top {top}");
    assert_eq!(committed(&mut app), (300, 200));
    for p in corners_and_middle(&app) {
        assert!(!background(p), "a corner shows the background colour: {p:?}");
    }
}

/// Every step of the turn starts from the frame at the press, so turning back within the same
/// drag gives the whole frame back instead of keeping the smallest size it passed through.
#[test]
fn turning_back_gives_the_frame_back() {
    let (mut app, ctx) = app(CANVAS);
    turn(&mut app, &ctx, CANVAS, &[(10.0, plain()), (0.0, plain())], |_, _| {});
    assert_eq!((frame(&app), angle(&app)), (CANVAS, 0.0));
}

/// A second turn starts from the frame the first one fitted, whose turned corners touch the
/// canvas's edges: turned further it shrinks again, turned back upright it stays as it is.
#[test]
fn a_second_turn_starts_from_the_fitted_frame() {
    for second in [10.0, -6.3] {
        let (mut app, ctx) = app(CANVAS);
        turn(&mut app, &ctx, CANVAS, &[(6.3, plain())], |_, _| {});
        let first = frame(&app);
        turn(&mut app, &ctx, first, &[(second, plain())], |_, _| {});
        let f = frame(&app);
        let what = format!("then {second}°: {f:?} turned {}", angle(&app));
        assert!(near_angle(angle(&app), -(6.3 + second)), "{what}");
        assert!(inside(&app), "{what}");
        if second > 0.0 {
            let want = fitted_size(angle(&app));
            assert!(((f[2] - f[0]) - want.0).abs() < 1e-6 && ((f[3] - f[1]) - want.1).abs() < 1e-6, "{what}: want {want:?}");
        } else {
            assert_eq!(f, first, "{what}");
        }
    }
}

/// A frame `kept_inside` fitted is left exactly as it is at the same angle, at every tenth of a
/// degree up to 90°. The fitted frame's turned corners touch the canvas's edges, and at many
/// angles rounding puts them a hair past: without the fit's tolerance such a frame would count as
/// reaching past the canvas, and a second turn would no longer keep it inside.
#[test]
fn a_fitted_frame_fits_at_its_own_angle() {
    let off_centre = [0.0, 0.0, 300.0, 200.0];
    for i in 1..900 {
        let deg = f64::from(i) / 10.0;
        for start in [CANVAS, off_centre] {
            let f = kept_inside(start, deg, W, H).expect("a frame");
            assert_eq!(kept_inside(f, deg, W, H), Some(f), "{start:?} at {deg}°");
        }
    }
}

/// Not measured, so they keep today's behaviour: with Fill set to Content-Aware Fill, with a
/// frame already reaching past the canvas at the press, and in Classic Mode (where the frame
/// turns over the image) the frame keeps its size and place.
#[test]
fn content_aware_fill_a_frame_past_the_canvas_and_classic_mode_keep_the_frame() {
    let past = [-50.0, 100.0, 300.0, 300.0];
    for (what, start) in [("content-aware", CANVAS), ("past the canvas", past), ("classic", CANVAS)] {
        let (mut app, ctx) = app(start);
        app.ui.tool_options.crop_content_aware = what == "content-aware";
        app.ui.tool_options.crop_shield.classic_mode = what == "classic";
        turn(&mut app, &ctx, start, &[(6.3, plain())], |_, _| {});
        assert!(angle(&app).abs() > 6.0, "{what}: turned, {}", angle(&app));
        assert_eq!(frame(&app), start, "{what}");
    }
}
