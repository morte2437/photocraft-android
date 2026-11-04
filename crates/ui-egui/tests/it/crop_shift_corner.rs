//! Crop tool: ⇧-dragging a frame corner keeps its proportions by following the pointer along the
//! frame's diagonal, as Photoshop does (#10871).
//!
//! Measured in Photoshop: 800 × 500 document, frame = whole canvas, free ratio, Auto Center
//! Preview off, ⇧-drag of the bottom-right corner by (−100, −50) px commits 706 × 441 (the drag
//! projected onto (800, 500) shortens the frame by 11.8 %). PhotoCraft let the larger relative
//! extent win and committed 720 × 450.
//!
//! Driven through `ui.pointer`, as an agent or the canvas sends the drag; no GPU needed. The Auto
//! Center Preview test sends screen points through the view instead, as the mouse does, since the
//! view pans with the frame during that drag.

use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::{ToolEvent, ViewXform, tool_event};
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::crop_ui;
use serde_json::{Value, json};

/// The measured ⇧ drag.
const DRAG: [f64; 2] = [-100.0, -50.0];

#[derive(Default)]
struct Setup {
    auto_center: bool,
    classic: bool,
    /// Options-bar ratio preset, `""` for none.
    ratio: &'static str,
    snap: bool,
}

/// An 800 × 500 document at 100 % in a canvas it fits, the Crop tool's frame up.
fn crop_app(s: Setup) -> PhotocraftApp {
    let doc = Document::with_background("crop", Size::new(800, 500), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    let mut app = PhotocraftApp::new(session, Default::default());
    app.sync_views();
    app.ui.extras.snap = s.snap;
    app.ui.extras.guides = true;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let v = &mut app.ui.views[0];
    (v.zoom, v.center, v.fit_pending, v.rotation) = (1.0, [400.0, 250.0], false, 0.0);
    app.ui.tool_options.crop_shield.auto_center_preview = s.auto_center;
    app.ui.tool_options.crop_shield.classic_mode = s.classic;
    app.ui.tool_options.crop_ratio = s.ratio.into();
    pointer(&mut app, json!({"tool": "Crop", "events": []}));
    crop_ui::ensure_frame(&mut app);
    app
}

fn pointer(app: &mut PhotocraftApp, params: Value) {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("ui.pointer", params);
    let Outcome::Done(v) = handle(app, &ctx, &req) else { panic!("ui.pointer didn't finish") };
    assert_eq!(v["ok"], true, "{v}");
}

/// The frame's (turned) bottom-right corner, where the drags start.
fn corner(app: &PhotocraftApp) -> [f64; 2] {
    crop_ui::corners(app.ui.crop_rect.expect("a frame"), crop_ui::angle(app))[2]
}

/// Commits with ↵ and returns the document size.
fn commit(app: &mut PhotocraftApp) -> (u32, u32) {
    photocraft_ui_egui::canvas::commit_crop(app);
    let size = app.session.active().expect("a document").doc.size;
    (size.width, size.height)
}

/// Drags the bottom-right corner by document vector `d` in steps through `ui.pointer`, holding
/// `mods`, and commits.
fn drag_corner_and_commit(app: &mut PhotocraftApp, d: [f64; 2], mods: Value) -> (u32, u32) {
    drag_corner(app, d, mods);
    commit(app)
}

/// [`drag_corner_and_commit`] without the commit: the frame stays pending.
fn drag_corner(app: &mut PhotocraftApp, d: [f64; 2], mods: Value) {
    let [x, y] = corner(app);
    let mut events = vec![json!({"kind": "down", "x": x, "y": y})];
    events.extend((1..=4).map(|i| {
        let t = f64::from(i) / 4.0;
        json!({"kind": "move", "x": x + d[0] * t, "y": y + d[1] * t})
    }));
    events.push(json!({"kind": "up", "x": x + d[0], "y": y + d[1]}));
    let mut params = mods;
    params["events"] = Value::Array(events);
    pointer(app, params);
}

fn assert_near(got: (u32, u32), want: (u32, u32), what: &str) {
    let off = |a: u32, b: u32| a.abs_diff(b);
    assert!(off(got.0, want.0) <= 1 && off(got.1, want.1) <= 1, "{what}: committed {got:?}, want {want:?} ±1 px");
}

#[test]
fn shift_corner_drag_follows_the_frames_diagonal() {
    let mut app = crop_app(Setup::default());
    assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "the default frame is the whole canvas");
    let got = drag_corner_and_commit(&mut app, DRAG, json!({"shift": true}));
    assert_near(got, (706, 441), "⇧ drag by (−100, −50)");
}

#[test]
fn shift_corner_drag_outward_past_the_canvas_follows_the_diagonal() {
    let mut app = crop_app(Setup::default());
    // (100, 50) onto (800, 500) grows the frame by 11.8 %: 894.4 × 559.0, past the canvas.
    let got = drag_corner_and_commit(&mut app, [100.0, 50.0], json!({"shift": true}));
    assert_near(got, (894, 559), "⇧ drag by (100, 50)");
}

/// A turned frame: the diagonal turns with it, so the measured drag taken in the frame's own axes
/// gives the measured size, in Classic Mode (the frame turns) and the default mode (the image
/// turns). Projected onto the upright diagonal instead it gives about 714 × 446.
#[test]
fn shift_corner_drag_on_a_turned_frame_follows_its_diagonal() {
    for classic in [true, false] {
        let mut app = crop_app(Setup { classic, ..Setup::default() });
        app.ui.crop_angle = 30.0;
        photocraft_ui_egui::crop_mode::sync_frame(&mut app);
        let d = crop_ui::turn(DRAG, [0.0, 0.0], 30.0);
        let got = drag_corner_and_commit(&mut app, d, json!({"shift": true}));
        assert_near(got, (706, 441), &format!("⇧ drag on a frame turned 30°, Classic Mode {classic}"));
    }
}

/// Auto Center Preview (the default): the box stays centred on screen and shrinks on both sides
/// as the handle follows the pointer, so each extent changes by twice the projected drag:
/// 800 − 2 × 94.4 = 611.2 and 500 − 2 × 59.0 = 382.0. With the larger relative extent winning it
/// was 640 × 400.
#[test]
fn shift_corner_drag_follows_the_diagonal_with_auto_center_preview() {
    let mut app = crop_app(Setup { auto_center: true, ..Setup::default() });
    let shift = egui::Modifiers::SHIFT;
    let [x, y] = corner(&app);
    let from = ViewXform::active(&app).expect("a view").to_screen(x as f32, y as f32);
    let to = from + egui::vec2(DRAG[0] as f32, DRAG[1] as f32);
    // Each screen point is read through the view as it is then, as the canvas does every frame.
    let at = |app: &PhotocraftApp, p: egui::Pos2| ViewXform::active(app).expect("a view").to_doc(p);
    let [x, y] = at(&app, from);
    tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, shift);
    for i in 1..=4 {
        let [x, y] = at(&app, from + (to - from) * (i as f32 / 4.0));
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, shift);
    }
    let [x, y] = at(&app, to);
    tool_event(&mut app, ToolEvent::Up { x, y }, shift);
    assert_near(commit(&mut app), (611, 382), "⇧ drag with Auto Center Preview");
}

/// A ratio preset keeps today's rule (not measured): the larger relative extent wins. On the
/// 500 × 500 frame 1 : 1 fits, (−100, −50) gives max(400, 450) = 450 square; along the diagonal
/// it would be 425 square.
#[test]
fn shift_corner_drag_with_a_ratio_preset_is_unchanged() {
    let mut app = crop_app(Setup { ratio: "1:1", ..Setup::default() });
    assert_eq!(app.ui.crop_rect, Some([150.0, 0.0, 650.0, 500.0]), "the preset fits a 500 × 500 frame");
    assert_near(drag_corner_and_commit(&mut app, DRAG, json!({"shift": true})), (450, 450), "⇧ with 1 : 1");
}

#[test]
fn the_same_drag_without_shift_or_with_alt_is_unchanged() {
    // No ⇧: the corner follows the pointer freely, 700 × 450 as in Photoshop.
    let mut app = crop_app(Setup::default());
    assert_near(drag_corner_and_commit(&mut app, DRAG, json!({})), (700, 450), "no ⇧");
    // ⌥ ⇧ (not measured, today's rule): about the centre, 600 × 400 grows to 8 : 5 as 640 × 400.
    let mut app = crop_app(Setup::default());
    assert_near(drag_corner_and_commit(&mut app, DRAG, json!({"shift": true, "alt": true})), (640, 400), "⌥ ⇧");
}

/// Snapping reads the handle where the ⇧ drag puts it, on the diagonal (x ≈ 705.6 for the
/// measured drag), not where the larger-extent rule would (x = 720). The tolerance is 8 px at
/// 100 %: a guide 5.6 px away catches the handle, one 8.4 px away doesn't.
#[test]
fn a_shift_corner_drag_snaps_where_its_handle_is() {
    for (guide, want) in [(700.0, (700, 438)), (714.0, (706, 441))] {
        let mut app = crop_app(Setup { snap: true, ..Setup::default() });
        app.run("view.newGuide", json!({"orientation": "vertical", "position": guide})).expect("guide");
        let got = drag_corner_and_commit(&mut app, DRAG, json!({"shift": true}));
        assert_near(got, want, &format!("⇧ drag with a guide at x = {guide}"));
    }
}

/// On a frame turned 30° the corner slides along the turned diagonal, direction (0.89, 1.67) for
/// 8 : 5, so a vertical guide is met by sliding along it, not by moving the handle sideways.
///
/// - ⇧, free ratio: the measured drag leaves the handle at x ≈ 569.2, 4.2 px from a guide at
///   x = 565. Sliding on to it shortens the frame by 12.7 % to 698.1 × 436.3. Moving the handle
///   sideways and projecting that back onto the diagonal stopped it at x ≈ 568.3, off the guide,
///   at 704 × 440.
/// - An 8 : 5 preset (the larger extent wins, 720 × 450) leaves it at x ≈ 577.1, 4.1 px from a
///   guide at x = 573: 712.5 × 445.3. Sideways, the larger extent grew the frame to 723 × 452,
///   away from the guide.
#[test]
fn a_shift_corner_drag_on_a_turned_frame_snaps_along_its_diagonal() {
    for (ratio, guide, want) in [("", 565.0, (698, 436)), ("8:5", 573.0, (712, 445))] {
        for classic in [true, false] {
            let what = format!("ratio {ratio:?}, guide at x = {guide}, Classic Mode {classic}");
            let mut app = crop_app(Setup { classic, ratio, snap: true, ..Setup::default() });
            app.ui.crop_angle = 30.0;
            photocraft_ui_egui::crop_mode::sync_frame(&mut app);
            app.run("view.newGuide", json!({"orientation": "vertical", "position": guide})).expect("guide");
            drag_corner(&mut app, crop_ui::turn(DRAG, [0.0, 0.0], 30.0), json!({"shift": true}));
            let at = corner(&app);
            assert!((at[0] - guide).abs() < 1e-3, "{what}: the handle is at {at:?}, not on the guide");
            let r = app.ui.crop_rect.expect("a frame");
            assert!(((r[2] - r[0]) / (r[3] - r[1]) - 1.6).abs() < 1e-9, "{what}: 8 : 5 kept, {r:?}");
            assert_near(commit(&mut app), want, &what);
        }
    }
}

/// Turned 31° the other way, the 8 : 5 frame's bottom-right diagonal runs 1° off horizontal, so it
/// crosses a horizontal guide 3 px from the handle about 170 px further along. That crossing is
/// well outside the 8 px snap tolerance: the guide doesn't catch the handle and the frame follows
/// the drag, as with no guide. Sliding to the crossing grew the frame by about 40 %.
#[test]
fn a_turned_shift_corner_ignores_a_guide_its_diagonal_meets_far_away() {
    let drag = crop_ui::turn([-50.0, -25.0], [0.0, 0.0], -31.0);
    for classic in [true, false] {
        let what = format!("Classic Mode {classic}");
        // Off the document's centre lines, which snap too.
        let setup = || {
            let mut app = crop_app(Setup { classic, snap: true, ..Setup::default() });
            app.ui.crop_rect = Some([100.0, 60.0, 500.0, 310.0]);
            app.ui.crop_angle = -31.0;
            photocraft_ui_egui::crop_mode::sync_frame(&mut app);
            app
        };
        let mut free = setup();
        drag_corner(&mut free, drag, json!({"shift": true}));
        let (want, at) = (free.ui.crop_rect.expect("a frame"), corner(&free));
        let mut app = setup();
        app.run("view.newGuide", json!({"orientation": "horizontal", "position": at[1] + 3.0})).expect("guide");
        drag_corner(&mut app, drag, json!({"shift": true}));
        let r = app.ui.crop_rect.expect("a frame");
        assert!(r.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-6), "{what}: {r:?}, want {want:?} as with no guide");
    }
}
