use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::crop_ui::{CropDrag, corners};
use egui::{Modifiers, Pos2, pos2};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;

const NONE: Modifiers = Modifiers::NONE;

/// A 200 × 100 document in a 800 × 600 canvas at 100 %, centred, the Crop tool in default mode.
fn app() -> PhotocraftApp {
    let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.sync_views();
    app.ui.tool = Tool::Crop;
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    app.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let v = &mut app.ui.views[0];
    (v.zoom, v.center, v.fit_pending, v.rotation) = (1.0, [100.0, 50.0], false, 0.0);
    assert!(!classic(&app), "Photoshop's default mode is the default");
    app
}

fn xf(app: &PhotocraftApp) -> ViewXform {
    ViewXform::active(app).unwrap()
}

/// A drag through screen points, each mapped through the view as it is at that moment (as the
/// canvas does every frame).
fn screen_drag(app: &mut PhotocraftApp, pts: &[Pos2], mods: Modifiers) {
    let at = |app: &PhotocraftApp, p: Pos2| xf(app).to_doc(p);
    let d = at(app, pts[0]);
    tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: 1.0 }, mods);
    for p in &pts[1..] {
        let d = at(app, *p);
        tool_event(app, ToolEvent::Move { x: d[0], y: d[1], pressure: 1.0 }, mods);
    }
    let d = at(app, pts[pts.len() - 1]);
    tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
}

fn screen_box(app: &PhotocraftApp) -> [Pos2; 4] {
    let x = xf(app);
    corners(app.ui.crop_rect.unwrap(), crate::crop_ui::angle(app)).map(|p| x.to_screen(p[0] as f32, p[1] as f32))
}

fn assert_upright(q: [Pos2; 4]) {
    let ok = (q[0].y - q[1].y).abs() < 0.01 && (q[2].y - q[3].y).abs() < 0.01 && (q[0].x - q[3].x).abs() < 0.01 && (q[1].x - q[2].x).abs() < 0.01;
    assert!(ok && q[1].x > q[0].x && q[3].y > q[0].y, "not an upright box on screen: {q:?}");
}

fn close(a: [Pos2; 4], b: [Pos2; 4]) -> bool {
    a.iter().zip(b).all(|(p, q)| p.distance(q) < 0.02)
}

/// A real (not default) frame [40, 30, 100, 70], centred at (70, 50) = screen (370, 300).
fn with_frame(app: &mut PhotocraftApp) {
    crate::crop_ui::ensure_frame(app);
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    app.crop.default_frame = false;
}

#[test]
fn a_live_resize_snaps_the_handle_without_feedback_from_the_camera() {
    for (hx, hy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
        for alt in [false, true] {
            for (zoom, ppp, flip, ratio) in [(1.0, 1.0, false, false), (0.5, 1.0, false, true), (3.0, 2.0, false, false), (3.0, 2.0, true, true)] {
                let mut app = app();
                app.ppp = ppp;
                app.ui.views[0].zoom = zoom;
                app.ui.view.flip_horizontal = flip;
                if ratio {
                    app.ui.tool_options.crop_ratio = "original".into();
                }
                app.ui.extras.snap = true;
                crate::crop_ui::ensure_frame(&mut app);
                let box0 = screen_box(&app);
                let c = mid_of(box0);
                let from = xf(&app).to_screen(100.0 + hx as f32 * 100.0, 50.0 + hy as f32 * 50.0);
                let direction = (from - c).normalized();
                let mods = if alt { Modifiers::ALT } else { NONE };
                let d = xf(&app).to_doc(from);
                tool_event(&mut app, ToolEvent::Down { x: d[0], y: d[1], pressure: 1.0 }, mods);
                // Leave the snap radius, then return just inside it. A still pointer must
                // settle on the canvas edge instead of alternating on opposite sides of it.
                for offset in [20.0, -2.0, -2.0, -2.0, -2.0, -2.0] {
                    let p = from + direction * offset;
                    let d = xf(&app).to_doc(p);
                    tool_event(&mut app, ToolEvent::Move { x: d[0], y: d[1], pressure: 1.0 }, mods);
                    if offset == -2.0 {
                        assert!(close(screen_box(&app), box0), "handle ({hx}, {hy}), Alt {alt}: {:?}", app.ui.crop_rect);
                    }
                }
                let d = xf(&app).to_doc(from + direction * -2.0);
                tool_event(&mut app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
                assert!(close(screen_box(&app), box0), "release: {:?}", app.ui.crop_rect);
            }
        }
    }
}

#[test]
fn dragging_outside_turns_the_image_and_the_box_stays_upright() {
    let mut app = app();
    with_frame(&mut app);
    let before = screen_box(&app);
    // From straight right of the box centre to straight below it: a quarter turn clockwise
    // on screen, so the image turns clockwise and the frame turns -90° on it.
    screen_drag(&mut app, &[pos2(450.0, 300.0), pos2(440.0, 320.0), pos2(370.0, 380.0)], NONE);
    assert!((crate::crop_ui::angle(&app) - -90.0).abs() < 0.01, "{}", app.ui.crop_angle);
    assert!((view_rotation(&app) - 90.0).abs() < 0.01, "the view turned with it");
    assert_upright(screen_box(&app));
    assert!(close(screen_box(&app), before), "the box doesn't move on screen: {:?} vs {before:?}", screen_box(&app));
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "turning keeps the frame");
    // ⇧ steps of 15°, still upright.
    let a = 22.0f32.to_radians();
    screen_drag(&mut app, &[pos2(450.0, 300.0), pos2(370.0 + 80.0 * a.cos(), 300.0 + 80.0 * a.sin())], Modifiers::SHIFT);
    assert!((crate::crop_ui::angle(&app) - -105.0).abs() < 0.01, "{}", app.ui.crop_angle);
    assert_upright(screen_box(&app));
    assert!(close(screen_box(&app), before));
    assert!(crate::crop_ui::turning(&app).is_none());
}

/// Even with Auto Center Preview, a drag inside an off-centre box (after a pan, say) leaves the
/// box where it is on screen: only the image moves (Photoshop 25.1).
#[test]
fn a_move_never_recentres_the_box() {
    let mut app = app();
    assert!(app.ui.tool_options.crop_shield.auto_center_preview);
    with_frame(&mut app);
    assert!(mid_of(screen_box(&app)).distance(pos2(370.0, 300.0)) < 0.01);
    screen_drag(&mut app, &[pos2(370.0, 300.0), pos2(380.0, 300.0), pos2(390.0, 300.0)], NONE);
    assert!(mid_of(screen_box(&app)).distance(pos2(370.0, 300.0)) < 0.01, "{:?}", mid_of(screen_box(&app)));
    let r = app.ui.crop_rect.unwrap();
    assert!(r.iter().zip([20.0, 30.0, 80.0, 70.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{r:?}");
}

/// Moves the image under the box from document point `from` by `d` (both in the view at the
/// press), checks the snap lines before release, and returns where the box's centre started
/// on screen.
fn image_drag(app: &mut PhotocraftApp, from: [f32; 2], d: [f32; 2], line: Option<(bool, f64)>) -> Pos2 {
    let (a, b) = (xf(app).to_screen(from[0], from[1]), xf(app).to_screen(from[0] + d[0], from[1] + d[1]));
    let mid = mid_of(screen_box(app));
    let p = xf(app).to_doc(a);
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, NONE);
    assert_eq!(app.prefs_rt.snap.as_ref().map(|s| &s.gesture), Some(&crate::snap_ui::Gesture::CropImage));
    for t in [0.5, 1.0] {
        let p = xf(app).to_doc(a + (b - a) * t);
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, NONE);
    }
    if let Some((vertical, pos)) = line {
        assert!(app.prefs_rt.snap_lines.iter().any(|l| l.vertical == vertical && l.pos == pos), "{:?}", app.prefs_rt.snap_lines);
    }
    let p = xf(app).to_doc(b);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, NONE);
    mid
}

fn assert_frame(app: &PhotocraftApp, want: [f64; 4], what: &str) {
    let r = app.ui.crop_rect.unwrap();
    assert!(r.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-3), "{what}: {r:?}, want {want:?}");
}

/// #8580: moving the image under the box snaps the frame's edges and centre with View › Snap
/// on, so the content can be centred in it without leaving the Crop tool. The box stays put.
#[test]
fn moving_the_image_snaps_the_frame_to_the_document_centre() {
    for (snap, zoom, flip) in [(false, 1.0, false), (true, 1.0, false), (true, 2.0, false), (true, 1.0, true)] {
        let mut app = app();
        app.ui.extras.snap = snap;
        app.ui.views[0].zoom = zoom;
        app.ui.view.flip_horizontal = flip;
        with_frame(&mut app);
        // The frame [40, 30, 100, 70] goes 27 px right over the image: its centre lands 3 px
        // short of the document's (100, 50), inside the snap radius at either zoom.
        let mid = image_drag(&mut app, [70.0, 50.0], [-27.0, 0.0], snap.then_some((true, 100.0)));
        let what = format!("snap {snap}, zoom {zoom}, flip {flip}");
        assert_frame(&app, if snap { [70.0, 30.0, 130.0, 70.0] } else { [67.0, 30.0, 127.0, 70.0] }, &what);
        assert!(mid_of(screen_box(&app)).distance(mid) < 0.01, "{what}: the box moved on screen");
        assert!(app.prefs_rt.snap.is_none() && app.prefs_rt.snap_lines.is_empty());
    }
}

/// The issue's case: a box larger than the canvas (to extend it) centres on the content's
/// layer, both ways. A turned box doesn't snap (its edges don't line up with anything).
#[test]
fn moving_the_image_centres_a_larger_box_on_a_layer() {
    let mut app = app();
    app.ui.extras.snap = true;
    app.run("layer.new.layer", json!({"name": "logo"})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    crate::crop_ui::ensure_frame(&mut app);
    app.ui.crop_rect = Some([-40.0, -60.0, 140.0, 120.0]);
    app.crop.default_frame = false;
    // Its centre (50, 30) goes to (42, 38): 2 px from the logo's centre (40, 40).
    image_drag(&mut app, [50.0, 30.0], [8.0, -8.0], Some((false, 40.0)));
    assert_frame(&app, [-50.0, -50.0, 130.0, 130.0], "centred on the logo");

    app.ui.crop_angle = 30.0;
    sync(&mut app, None);
    let p = xf(&app).to_doc(mid_of(screen_box(&app)));
    tool_event(&mut app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, NONE);
    assert!(matches!(app.crop.drag, Some(CropDrag::MoveImage { .. })));
    assert!(app.prefs_rt.snap.is_none(), "a turned box doesn't snap");
    tool_event(&mut app, ToolEvent::Up { x: p[0], y: p[1] }, NONE);
}

/// With View › Snap To › Layers off, smart guides still centre the box on a layer, although
/// the document's bounds are snap targets that aren't within reach.
#[test]
fn moving_the_image_snaps_to_smart_guides_beside_out_of_reach_targets() {
    for smart in [false, true] {
        let mut app = app();
        app.ui.extras.snap = true;
        app.ui.view.snap_to.layers = false;
        app.ui.view.show.smart_guides = smart;
        app.run("layer.new.layer", json!({"name": "logo"})).unwrap();
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        crate::crop_ui::ensure_frame(&mut app);
        app.ui.crop_rect = Some([-40.0, -60.0, 140.0, 120.0]);
        app.crop.default_frame = false;
        image_drag(&mut app, [50.0, 30.0], [8.0, -8.0], smart.then_some((false, 40.0)));
        let want = if smart { [-50.0, -50.0, 130.0, 130.0] } else { [-48.0, -52.0, 132.0, 128.0] };
        assert_frame(&app, want, &format!("smart guides {smart}"));
    }
}

/// #7088: the box's own Undo/Redo of a move (or a turn) in default mode puts the frame back
/// and moves the image, not the box: it stays put on screen (Auto Center Preview off) or goes
/// to the middle of the canvas (on).
#[test]
fn box_undo_and_redo_keep_the_box_put() {
    for auto in [false, true] {
        let mut app = app();
        app.ui.tool_options.crop_shield.auto_center_preview = auto;
        with_frame(&mut app);
        // A quarter turn, then a move inside: two box steps.
        screen_drag(&mut app, &[pos2(450.0, 300.0), pos2(370.0, 380.0)], NONE);
        let turned = app.ui.crop_rect;
        screen_drag(&mut app, &[pos2(370.0, 300.0), pos2(380.0, 305.0), pos2(390.0, 310.0)], NONE);
        let moved = app.ui.crop_rect;
        assert_ne!(moved, turned);
        let want = if auto { pos2(400.0, 300.0) } else { pos2(370.0, 300.0) };
        let check = |app: &PhotocraftApp, what: &str| {
            let q = screen_box(app);
            assert_upright(q);
            assert!(mid_of(q).distance(want) < 0.05, "auto={auto} {what}: the box moved on screen to {:?}", mid_of(q));
        };
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.ui.crop_rect, turned, "auto={auto}: the move is undone");
        check(&app, "undo move");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]));
        assert!(crate::crop_ui::angle(&app).abs() < 1e-9, "the turn is undone");
        check(&app, "undo turn");
        app.run("edit.redo", json!({})).unwrap();
        check(&app, "redo turn");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.ui.crop_rect, moved, "auto={auto}: the move is redone");
        check(&app, "redo move");
    }
}

fn mid_of(q: [Pos2; 4]) -> Pos2 {
    pos2((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0)
}

#[test]
fn dragging_inside_moves_the_image_under_the_box() {
    let mut app = app();
    with_frame(&mut app);
    let before = screen_box(&app);
    screen_drag(&mut app, &[pos2(370.0, 300.0), pos2(380.0, 305.0), pos2(390.0, 310.0)], NONE);
    // The image followed the pointer 20, 10 to the right and down: the frame lies that much
    // further up-left on it.
    let r = app.ui.crop_rect.unwrap();
    assert!(r.iter().zip([20.0, 20.0, 80.0, 60.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{r:?}");
    assert!(close(screen_box(&app), before), "the box stays put on screen");
    // Turned 30° first: the move follows the pointer on screen, along the turned image.
    screen_drag(&mut app, &[pos2(450.0, 300.0), pos2(370.0 + 80.0 * 30f32.to_radians().cos(), 300.0 + 80.0 * 30f32.to_radians().sin())], NONE);
    assert!((crate::crop_ui::angle(&app) - -30.0).abs() < 0.01);
    let (before, c0) = (screen_box(&app), crate::crop_ui::center(app.ui.crop_rect.unwrap()));
    let img = |app: &PhotocraftApp| xf(app).to_screen(0.0, 0.0);
    let origin = img(&app);
    screen_drag(&mut app, &[pos2(370.0, 300.0), pos2(400.0, 300.0)], NONE);
    assert!(close(screen_box(&app), before), "box put");
    assert!((img(&app) - origin - vec2(30.0, 0.0)).length() < 0.05, "the image moved 30 right on screen");
    let c1 = crate::crop_ui::center(app.ui.crop_rect.unwrap());
    assert!(((c1[0] - c0[0]).hypot(c1[1] - c0[1]) - 30.0).abs() < 1e-3);
}

/// A `ui.pointer` click (down and up at the same point, no move) changes nothing, upright or
/// turned: reading the point in the press view used to round it through `f32` and the view,
/// nudging the frame by a hair and pushing a box Undo step that undid nothing visible.
#[test]
fn a_ui_pointer_click_changes_nothing() {
    let ctx = egui::Context::default();
    for deg in [0.0, -30.0, 41.5] {
        let mut app = app();
        with_frame(&mut app);
        app.ui.crop_angle = deg;
        sync_frame(&mut app);
        let (rect, angle) = (app.ui.crop_rect, app.ui.crop_angle);
        // Inside the frame (moves the image) and outside it (turns the image).
        for p in [[70.3, 50.7], [55.17, 41.9], [83.01, 59.33], [150.3, 50.7], [10.9, 90.1]] {
            let ev = json!([{"kind": "down", "x": p[0], "y": p[1]}, {"kind": "up", "x": p[0], "y": p[1]}]);
            let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": ev}));
            let crate::control::Outcome::Done(v) = crate::control::handle(&mut app, &ctx, &req) else { panic!("ui.pointer didn't finish") };
            assert_eq!(v["ok"], true, "{v}");
            assert_eq!((app.ui.crop_rect, app.ui.crop_angle), (rect, angle), "click at {p:?}, frame at {deg}°");
            assert_eq!(crate::crop_ui::is_enabled(&app, "edit.undo"), None, "click at {p:?}, frame at {deg}°: a box Undo step");
        }
    }
}

#[test]
fn the_readout_is_the_images_turn_clockwise_positive() {
    let mut app = app();
    with_frame(&mut app);
    // A 30° clockwise sweep on screen, held mid-drag.
    let d = |app: &PhotocraftApp, p: Pos2| xf(app).to_doc(p);
    let a = 30f32.to_radians();
    let (p0, p1) = (pos2(450.0, 300.0), pos2(370.0 + 80.0 * a.cos(), 300.0 + 80.0 * a.sin()));
    let s = d(&app, p0);
    tool_event(&mut app, ToolEvent::Down { x: s[0], y: s[1], pressure: 1.0 }, NONE);
    let m = d(&app, p1);
    tool_event(&mut app, ToolEvent::Move { x: m[0], y: m[1], pressure: 1.0 }, NONE);
    let shown = crate::crop_ui::turning(&app).unwrap();
    assert!((shown - 30.0).abs() < 0.01, "clockwise is positive: {shown}");
    assert!((crate::crop_ui::angle(&app) - -30.0).abs() < 0.01, "the frame turns the other way on the image");
    let m = d(&app, p1);
    tool_event(&mut app, ToolEvent::Up { x: m[0], y: m[1] }, NONE);
    assert_eq!(image_turn(0.0).to_string(), "0", "never -0");
    // Classic Mode shows the frame's own turn, clockwise positive as before.
    let mut app = self::app();
    app.ui.tool_options.crop_shield.classic_mode = true;
    with_frame(&mut app);
    let s = d(&app, p0);
    tool_event(&mut app, ToolEvent::Down { x: s[0], y: s[1], pressure: 1.0 }, NONE);
    let m = d(&app, p1);
    tool_event(&mut app, ToolEvent::Move { x: m[0], y: m[1], pressure: 1.0 }, NONE);
    assert!((crate::crop_ui::turning(&app).unwrap() - 30.0).abs() < 0.01);
}

#[test]
fn auto_center_keeps_the_box_centred_while_a_handle_is_dragged() {
    for auto in [true, false] {
        let mut app = app();
        app.ui.tool_options.crop_shield.auto_center_preview = auto;
        with_frame(&mut app);
        // Turn 90° clockwise on screen first: the frame is at -90° in the image.
        screen_drag(&mut app, &[pos2(450.0, 300.0), pos2(370.0, 380.0)], NONE);
        let q = screen_box(&app);
        assert_upright(q);
        let mid = |q: [Pos2; 4]| pos2((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
        // The box's right edge on screen, dragged 10 then 20 further right.
        let right = pos2(q[1].x, (q[1].y + q[2].y) / 2.0);
        assert_eq!(crate::crop_ui::cursor(&app, xf(&app).to_doc(right)), Some(egui::CursorIcon::ResizeHorizontal), "screen-space arrow");
        let at = |app: &PhotocraftApp, p: Pos2| xf(app).to_doc(p);
        let s = at(&app, right);
        tool_event(&mut app, ToolEvent::Down { x: s[0], y: s[1], pressure: 1.0 }, NONE);
        for dx in [10.0, 20.0] {
            let m = at(&app, right + vec2(dx, 0.0));
            tool_event(&mut app, ToolEvent::Move { x: m[0], y: m[1], pressure: 1.0 }, NONE);
            let n = screen_box(&app);
            assert_upright(n);
            assert!((n[1].x - (right.x + dx)).abs() < 0.02, "auto={auto} dx={dx}: the handle follows the pointer: {n:?}");
            if auto {
                assert!(mid(n).distance(mid(q)) < 0.02, "dx={dx}: live, the box stays centred: {:?} vs {:?}", mid(n), mid(q));
                assert!(((n[1].x - n[0].x) - (q[1].x - q[0].x) - 2.0 * dx).abs() < 0.02, "it grows on both sides");
            } else {
                assert!((n[0].x - q[0].x).abs() < 0.02, "the left edge stays put");
            }
            assert!(((n[3].y - n[0].y) - (q[3].y - q[0].y)).abs() < 0.02);
        }
        let m = at(&app, right + vec2(20.0, 0.0));
        tool_event(&mut app, ToolEvent::Up { x: m[0], y: m[1] }, NONE);
        let n = screen_box(&app);
        if auto {
            assert!(mid(n).distance(mid(q)) < 0.05, "on release it stays where its centre was: {:?} vs {:?}", mid(n), mid(q));
            // On the image, the opposite edge stayed put: the frame grew 40 px from it.
            let r = app.ui.crop_rect.unwrap();
            assert!(((r[2] - r[0]) - 100.0).abs() < 1e-3 && ((r[3] - r[1]) - 40.0).abs() < 1e-3, "{r:?}");
            let left = corners(r, crate::crop_ui::angle(&app))[0];
            let was = corners([40.0, 30.0, 100.0, 70.0], -90.0)[0];
            assert!((left[0] - was[0]).abs() < 1e-3 && (left[1] - was[1]).abs() < 1e-3, "{left:?} vs {was:?}");
        } else {
            assert!(mid(n).distance(pos2(380.0, 300.0)) < 0.05, "{:?}", mid(n));
        }
    }
    // ⌥: about the centre, and a collapsed drag puts frame and view back.
    let mut app = app();
    with_frame(&mut app);
    let q = screen_box(&app);
    let center = app.ui.views[0].center;
    let right = pos2(q[1].x, (q[1].y + q[2].y) / 2.0);
    screen_drag(&mut app, &[right, right + vec2(10.0, 0.0)], Modifiers::ALT);
    let r = app.ui.crop_rect.unwrap();
    assert!(((r[0] + r[2]) / 2.0 - 70.0).abs() < 1e-3 && ((r[2] - r[0]) - 80.0).abs() < 1e-3, "{r:?}");
    let mut app = self::app();
    with_frame(&mut app);
    screen_drag(&mut app, &[right, pos2(370.0, 300.0)], NONE);
    assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "too small: the frame is back");
    assert_eq!(app.ui.views[0].center, center, "and the view");
}

/// The same frame and angle commit the same crop in both modes, and in both the view stays
/// upright after a commit or a cancel (Photoshop 25.1 doesn't restore a Rotate View turn).
#[test]
fn commit_is_the_same_in_both_modes_and_the_view_stays_upright() {
    let mut out = Vec::new();
    for classic in [true, false] {
        let mut app = app();
        app.run("select.rect", json!({"x": 70, "y": 50, "width": 2, "height": 2})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool_options.crop_shield.classic_mode = classic;
        app.ui.views[0].rotation = 20.0;
        with_frame(&mut app);
        app.ui.crop_angle = 30.0;
        app.crop.editing = true;
        crate::crop_ui::ensure_frame(&mut app);
        let want = if classic { 0.0 } else { -30.0 };
        assert!((view_rotation(&app) - want).abs() < 1e-4, "classic={classic}: {}", view_rotation(&app));
        if !classic {
            assert_upright(screen_box(&app));
        }
        let past = app.session.active().unwrap().history.past_len();
        crate::canvas::commit_crop(&mut app);
        assert_eq!(view_rotation(&app), 0.0, "classic={classic}: after ↵");
        crate::crop_ui::ensure_frame(&mut app);
        assert_eq!(view_rotation(&app), 0.0, "classic={classic}: with the next frame");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), past + 1, "one undo step");
        let surf = st.doc.layers[0].surface().unwrap();
        let px: Vec<[f32; 4]> =
            (0..st.doc.size.width as i32).flat_map(|x| (0..st.doc.size.height as i32).map(move |y| (x, y))).map(|(x, y)| surf.rgba(x, y)).collect();
        out.push((st.doc.size, px));
        // Esc.
        let mut app = self::app();
        app.ui.tool_options.crop_shield.classic_mode = classic;
        app.ui.views[0].rotation = -45.0;
        with_frame(&mut app);
        screen_drag(&mut app, &[pos2(500.0, 300.0), pos2(370.0, 420.0)], NONE);
        crate::crop_ui::cancel(&mut app);
        crate::crop_ui::ensure_frame(&mut app);
        assert_eq!(view_rotation(&app), 0.0, "classic={classic}: Esc");
    }
    assert_eq!(out[0].0, out[1].0, "same size");
    assert!(out[0].1 == out[1].1, "same pixels");
}

#[test]
fn picking_the_crop_tool_turns_a_rotated_view_upright() {
    let mut app = app();
    app.ui.tool = Tool::Brush;
    app.ui.views[0].rotation = 30.0;
    let center = app.ui.views[0].center;
    app.ui.tool = Tool::Crop;
    crate::crop_ui::ensure_frame(&mut app);
    assert!(app.crop.default_frame && !app.crop.editing, "no click needed");
    assert_eq!(view_rotation(&app), 0.0);
    assert_eq!(app.ui.views[0].center, center, "about the view's centre");
    // Leaving the tool doesn't bring the turn back.
    app.ui.tool = Tool::Brush;
    crate::crop_ui::ensure_frame(&mut app);
    assert_eq!(view_rotation(&app), 0.0);
    // Classic Mode too (Photoshop 25.1): the camera goes upright about the view's centre and
    // the frame stays on the same image area, keeping its own turn.
    let mut app = self::app();
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.tool = Tool::Brush;
    app.ui.views[0].rotation = 30.0;
    app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
    app.ui.crop_angle = 12.0;
    app.ui.tool = Tool::Crop;
    crate::crop_ui::ensure_frame(&mut app);
    assert_eq!(view_rotation(&app), 0.0);
    assert_eq!(app.ui.views[0].center, center, "about the view's centre");
    assert_eq!((app.ui.crop_rect, app.ui.crop_angle), (Some([40.0, 30.0, 100.0, 70.0]), 12.0), "the frame is unchanged");
    // P switches modes without bringing the R turn back: default mode turns the view with the
    // frame, Classic Mode puts the camera upright again.
    let press_p = |app: &mut PhotocraftApp| {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.events.push(egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: NONE }));
        assert!(crate::crop_shield::keys(app, &ctx));
    };
    press_p(&mut app);
    assert!(!classic(&app));
    assert!((view_rotation(&app) - -12.0).abs() < 1e-4, "{}", view_rotation(&app));
    press_p(&mut app);
    assert!(classic(&app));
    assert_eq!(view_rotation(&app), 0.0);
    crate::crop_ui::cancel(&mut app);
    crate::crop_ui::ensure_frame(&mut app);
    assert_eq!(view_rotation(&app), 0.0, "Esc in Classic Mode: still upright");
}

fn key(app: &mut PhotocraftApp, key: egui::Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }));
    crate::shortcuts::handle(app, &ctx);
}

#[test]
fn arrows_nudge_the_image_shift_by_ten_and_x_recentres() {
    let mut app = app();
    with_frame(&mut app);
    app.crop.editing = true;
    app.ui.crop_angle = -90.0;
    crate::crop_ui::ensure_frame(&mut app);
    let before = screen_box(&app);
    let img = |app: &PhotocraftApp| xf(app).to_screen(0.0, 0.0);
    let origin = img(&app);
    key(&mut app, egui::Key::ArrowRight, NONE);
    assert!(close(screen_box(&app), before), "the box stays put");
    assert!((img(&app) - origin - vec2(1.0, 0.0)).length() < 0.02, "the image moved 1 px right on screen");
    key(&mut app, egui::Key::ArrowDown, Modifiers::SHIFT);
    assert!((img(&app) - origin - vec2(1.0, 10.0)).length() < 0.02, "⇧: 10 px");
    assert!(close(screen_box(&app), before));
    assert!(crate::crop_ui::swap_orientation(&mut app));
    assert_upright(screen_box(&app));
    let q = screen_box(&app);
    assert!(pos2((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0).distance(pos2(400.0, 300.0)) < 0.05, "Auto Center Preview");
}

/// Photoshop 25.1 greys out Show Cropped Area and Auto Center Preview in Classic Mode. Show
/// Cropped Area then acts as on (an assumption) and H does nothing; the stored choice stays.
#[test]
fn classic_mode_shows_the_cropped_area_and_h_does_nothing() {
    let mut app = app();
    crate::crop_ui::ensure_frame(&mut app);
    app.ui.tool_options.crop_shield.show_cropped_area = false;
    assert!(crate::crop_shield::hides_outside(&app));
    app.ui.tool_options.crop_shield.classic_mode = true;
    assert!(!crate::crop_shield::hides_outside(&app));
    let canvas = egui::Color32::from_rgb(40, 40, 40);
    assert_ne!(crate::crop_shield::fill(&app.ui.tool_options.crop_shield, canvas, false), Some(canvas), "the shield, not the opaque canvas");
    key(&mut app, egui::Key::H, NONE);
    assert_eq!(app.ui.tool, Tool::Crop, "H is taken, not the Hand tool");
    assert!(!app.ui.tool_options.crop_shield.show_cropped_area, "and changes nothing");
    app.ui.tool_options.crop_shield.classic_mode = false;
    assert!(crate::crop_shield::hides_outside(&app), "the stored choice is back");
    key(&mut app, egui::Key::H, NONE);
    assert!(app.ui.tool_options.crop_shield.show_cropped_area);
}

#[test]
fn p_toggles_classic_mode_while_the_crop_tool_shows_a_box() {
    let mut app = app();
    crate::crop_ui::ensure_frame(&mut app);
    let press = |app: &mut PhotocraftApp| {
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.events.push(egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: NONE }));
        crate::shortcuts::handle(app, &ctx);
    };
    press(&mut app);
    assert!(classic(&app) && app.ui.tool == Tool::Crop, "P doesn't pick the Pen tool");
    press(&mut app);
    assert!(!classic(&app));
    // Mid-drag P is left alone; without a box P is the Pen key.
    app.crop.drag = Some(CropDrag::Move { start: [0.0, 0.0], rect: [0.0, 0.0, 10.0, 10.0] });
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.events.push(egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: NONE }));
    assert!(!crate::crop_shield::keys(&mut app, &ctx));
    assert!(!classic(&app));
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::Crop;
    press(&mut app);
    assert_ne!(app.ui.tool, Tool::Crop);
    assert!(!classic(&app));
}

#[test]
fn straighten_turns_the_image_so_the_line_is_level_on_screen() {
    let mut app = app();
    crate::crop_ui::ensure_frame(&mut app);
    crate::crop_straighten::toggle(&mut app);
    // A horizon falling to the right, drawn on screen.
    let (a, b) = (pos2(320.0, 290.0), pos2(480.0, 310.0));
    let (da, db) = (xf(&app).to_doc(a), xf(&app).to_doc(b));
    screen_drag(&mut app, &[a, b], NONE);
    assert!(crate::crop_ui::angle(&app) > 5.0, "{}", app.ui.crop_angle);
    let (sa, sb) = (xf(&app).to_screen(da[0] as f32, da[1] as f32), xf(&app).to_screen(db[0] as f32, db[1] as f32));
    assert!((sa.y - sb.y).abs() < 0.01, "the line is level: {sa:?} {sb:?}");
    assert_upright(screen_box(&app));
}

/// `ui.pointer` points are read in the view at the press, so an `up` where the last `move` was
/// doesn't move or turn the image a second time.
#[test]
fn ui_pointer_drives_the_default_mode_in_press_coordinates() {
    let mut app = app();
    app.ui.tool_options.crop_shield.auto_center_preview = false;
    with_frame(&mut app);
    let ctx = egui::Context::default();
    let pointer = |app: &mut PhotocraftApp, pts: [[f64; 2]; 2]| {
        let ev = json!([
            {"kind": "down", "x": pts[0][0], "y": pts[0][1]},
            {"kind": "move", "x": pts[1][0], "y": pts[1][1]},
            {"kind": "up", "x": pts[1][0], "y": pts[1][1]},
        ]);
        let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": ev}));
        let crate::control::Outcome::Done(v) = crate::control::handle(app, &ctx, &req) else { panic!("ui.pointer didn't finish") };
        assert_eq!(v["ok"], true, "{v}");
    };
    pointer(&mut app, [[70.0, 50.0], [80.0, 55.0]]);
    let r = app.ui.crop_rect.unwrap();
    assert!(r.iter().zip([30.0, 25.0, 90.0, 65.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{r:?}");
    assert!((app.ui.views[0].center[0] - 90.0).abs() < 1e-3 && (app.ui.views[0].center[1] - 45.0).abs() < 1e-3);
    // Outside: from right of the centre (60, 45) to below it, a quarter turn of the image.
    pointer(&mut app, [[140.0, 45.0], [60.0, 125.0]]);
    assert!((crate::crop_ui::angle(&app) - -90.0).abs() < 0.01, "{}", app.ui.crop_angle);
    assert_upright(screen_box(&app));
}

#[test]
fn degenerate_input_never_panics() {
    let mut app = app();
    for frame in [[0.0, 0.0, 0.0, 0.0], [5.0, 5.0, 5.0, 5.0], [f64::NAN, 0.0, 10.0, 10.0], [f64::INFINITY, 0.0, 1e300, 1e300], [-1e300, -1e300, 1e300, 1e300]] {
        for angle in [0.0, f64::NAN, f64::INFINITY, 1e300, -720.5] {
            app.ui.crop_rect = Some(frame);
            app.ui.crop_angle = angle;
            app.crop.default_frame = false;
            app.crop.editing = true;
            crate::crop_ui::ensure_frame(&mut app);
            for p in [pos2(400.0, 300.0), pos2(10.0, 10.0), pos2(f32::NAN, 3.0), pos2(1e30, -1e30)] {
                screen_drag(&mut app, &[p, p + vec2(15.0, 7.0)], NONE);
                screen_drag(&mut app, &[p, p + vec2(15.0, 7.0)], Modifiers::SHIFT | Modifiers::ALT);
            }
            crate::crop_ui::nudge(&mut app, f64::NAN, 1.0);
            crate::crop_ui::nudge(&mut app, 1.0, 1.0);
            crate::crop_ui::swap_orientation(&mut app);
            auto_center(&mut app);
            shift_image(&mut app, [f64::NAN, f64::INFINITY]);
            let press = PressView { rotation: f32::NAN, center: [f32::INFINITY, 0.0] };
            let g = LiveResize { rect: frame, deg: angle, hx: 1, hy: -1, start: [0.0, f64::NAN], q: [1e300, -1e300], ratio: Some(f64::NAN), alt: false };
            live_resize(&mut app, g, press);
            assert!(app.ui.crop_rect.is_none_or(|r| r.iter().all(|v| v.is_finite()) || r == frame), "{frame:?} {angle}");
            let v = &app.ui.views[0];
            assert!(v.rotation.is_finite() && v.center.iter().all(|c| c.is_finite()), "{frame:?} {angle}: {v:?}");
            crate::crop_ui::cancel(&mut app);
        }
    }
    assert_eq!(pivot_center([1.0, 2.0], [f64::NAN, 0.0], 0.0, 90.0, false), [1.0, 2.0]);
    assert_eq!(pivot_center([1.0, 2.0], [0.0, 0.0], f32::NAN, 90.0, true), [1.0, 2.0]);
    // No view at all (no document).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::Crop;
    app.ui.crop_rect = Some([0.0, 0.0, 10.0, 10.0]);
    app.crop.editing = true;
    sync(&mut app, None);
    auto_center(&mut app);
    shift_image(&mut app, [1.0, 1.0]);
    upright(&mut app);
}

#[test]
fn the_options_default_to_photoshops_and_older_settings_load() {
    let s = crate::crop_shield::CropShield::default();
    assert!(!s.classic_mode && s.auto_center_preview);
    let s: crate::crop_shield::CropShield = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
    assert!(!s.classic_mode && s.auto_center_preview);
    let s: crate::crop_shield::CropShield = serde_json::from_str(r#"{"classic_mode": true, "auto_center_preview": false}"#).unwrap();
    assert!(s.classic_mode && !s.auto_center_preview);
}

#[test]
fn pivoting_keeps_the_point_on_screen() {
    for flip in [false, true] {
        for (from, to) in [(0.0, 30.0), (45.0, -90.0), (170.0, -170.0)] {
            let center = [100.0f32, 50.0];
            let c = [70.0, 20.0];
            let a = ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 2.0, center, flip, rotation: from, aspect: 1.0 };
            let n = pivot_center(center, c, from, to, flip);
            let b = ViewXform { center: n, rotation: to, ..a };
            let (p, q) = (a.to_screen(70.0, 20.0), b.to_screen(70.0, 20.0));
            assert!(p.distance(q) < 0.01, "{flip} {from} {to}: {p:?} vs {q:?}");
        }
    }
}

/// #1919: with Delete Cropped Pixels off, a second crop's handle dragged out toward the hidden
/// pixels snaps exactly onto their edge (a layer edge), however far from the handle's centre
/// the press grabbed it, in both modes, with and without Auto Center Preview: the committed
/// crop has no transparent strip.
#[test]
fn a_recrop_handle_snaps_exactly_to_the_hidden_pixels_edge() {
    // The image 200 × 100 cropped to [40, 20, 140, 70]: its pixels stay at [-40, -20, 160, 80].
    let hidden = [-40.0, -20.0, 160.0, 80.0];
    for classic in [false, true] {
        for auto in [true, false] {
            for (hx, hy) in [(1i8, 0i8), (0, -1), (-1, -1), (1, 1)] {
                // A corner also with the image's 2:1 ratio held, which both hidden corners keep.
                for (grab, ratio) in [(-4.0f32, false), (0.0, false), (4.0, false), (-4.0, true), (4.0, true)] {
                    if ratio && (hx == 0 || hy == 0) {
                        continue;
                    }
                    let what = format!("Classic Mode {classic}, Auto Center {auto}, handle ({hx}, {hy}), grabbed {grab} px off, ratio {ratio}");
                    let mut app = app();
                    app.ui.tool_options.crop_delete = false;
                    app.ui.tool_options.crop_shield.classic_mode = classic;
                    app.ui.tool_options.crop_shield.auto_center_preview = auto;
                    crate::crop_ui::ensure_frame(&mut app);
                    app.ui.crop_rect = Some([40.0, 20.0, 140.0, 70.0]);
                    app.crop.default_frame = false;
                    crate::canvas::commit_crop(&mut app);
                    let v = &mut app.ui.views[0];
                    (v.zoom, v.center, v.fit_pending, v.rotation) = (1.0, [50.0, 25.0], false, 0.0);
                    app.ui.extras.snap = true;
                    crate::crop_ui::ensure_frame(&mut app);
                    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 100.0, 50.0]));
                    if ratio {
                        app.ui.tool_options.crop_ratio = "original".into();
                    }
                    let grab = vec2(f32::from(hx), f32::from(hy)) * grab;
                    let from = xf(&app).to_screen(50.0 + f32::from(hx) * 50.0, 25.0 + f32::from(hy) * 25.0) + grab;
                    // Toward the hidden corner (or edge), a screen pixel at a time on the longer axis.
                    let gap = vec2(f32::from(hx) * if hx < 0 { 40.0 } else { 60.0 }, f32::from(hy) * if hy < 0 { 20.0 } else { 30.0 });
                    let dir = gap / gap.x.abs().max(gap.y.abs());
                    let d = xf(&app).to_doc(from);
                    tool_event(&mut app, ToolEvent::Down { x: d[0], y: d[1], pressure: 1.0 }, NONE);
                    // Drag outward a screen pixel at a time; let go once the moving edges show
                    // the snap to the hidden pixels' edges (as the user would).
                    let snapped = |app: &PhotocraftApp| {
                        let on = |vertical: bool, pos: f64| {
                            app.prefs_rt
                                .snap_lines
                                .iter()
                                .any(|l| l.vertical == vertical && l.pos == pos && l.kind == photocraft_engine::snap::SnapKind::LayerEdge)
                        };
                        (hx == 0 || on(true, if hx < 0 { hidden[0] } else { hidden[2] })) && (hy == 0 || on(false, if hy < 0 { hidden[1] } else { hidden[3] }))
                    };
                    let mut end = None;
                    for i in 1..=80 {
                        let d = xf(&app).to_doc(from + dir * i as f32);
                        tool_event(&mut app, ToolEvent::Move { x: d[0], y: d[1], pressure: 1.0 }, NONE);
                        if snapped(&app) {
                            end = Some(from + dir * i as f32);
                            break;
                        }
                    }
                    let end = end.unwrap_or_else(|| panic!("{what}: never snapped to the hidden edge"));
                    // Released where it stopped, read through the view as it is now.
                    let end = xf(&app).to_doc(end);
                    tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, NONE);
                    let pick = |h: i8, lo: f64, hi: f64, a: f64, b: f64| match h {
                        -1 => [a, hi],
                        1 => [lo, b],
                        _ => [lo, hi],
                    };
                    let [x0, x1] = pick(hx, 0.0, 100.0, hidden[0], hidden[2]);
                    let [y0, y1] = pick(hy, 0.0, 50.0, hidden[1], hidden[3]);
                    assert_eq!(app.ui.crop_rect, Some([x0, y0, x1, y1]), "{what}: the frame lands exactly on the hidden edge");
                    crate::canvas::commit_crop(&mut app);
                    let st = app.session.active().unwrap();
                    assert_eq!((st.doc.size.width, st.doc.size.height), ((x1 - x0) as u32, (y1 - y0) as u32), "{what}");
                    let layer = st.doc.walk().into_iter().map(|(_, _, l)| l.id).next().unwrap();
                    let r = photocraft_engine::snap::layer_rect(&st.doc, layer).unwrap();
                    assert_eq!(r, [hidden[0] - x0, hidden[1] - y0, hidden[2] - x0, hidden[3] - y0], "{what}: no transparent strip");
                }
            }
        }
    }
}
