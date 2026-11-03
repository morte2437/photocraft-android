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
use photocraft_ui_egui::crop_ui::{angle, corners as turned_corners, pending};
use serde_json::{Value, json};

/// The background colour, #1050e0: what the turn uncovers is filled with it.
const BLUE: [f32; 3] = [16.0 / 255.0, 80.0 / 255.0, 224.0 / 255.0];

/// An 800 × 500 white document with a red 40 px square centred on (300, 200), the Crop tool
/// picked with options-bar ratio `ratio` ("" for none) in Classic Mode or the default mode, and
/// its default frame.
fn app(classic: bool, ratio: &str) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).expect("new");
    app.run("select.rect", json!({"x": 280, "y": 180, "width": 40, "height": 40})).expect("select");
    app.run("edit.fill", json!({"color": "#ff0000"})).expect("fill");
    app.run("select.deselect", json!({})).expect("deselect");
    app.run("tools.setColors", json!({"background": "#1050e0"})).expect("colors");
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool_options.crop_shield.classic_mode = classic;
    app.ui.tool_options.crop_ratio = ratio.into();
    control(&mut app, &ctx, "ui.set", json!({"tool": "crop"}));
    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    (app, ctx)
}

/// Runs one control-channel request, which must succeed.
fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (req, _rx) = ControlRequest::new(method, params.clone());
    assert!(matches!(handle(app, ctx, &req), Outcome::Done(_)), "{method} {params}");
}

/// One `ui.pointer` call: `events` with ⌘ held or not.
fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, command: bool, events: Value) {
    control(app, ctx, "ui.pointer", json!({"command": command, "events": events}));
}

/// A drag from `a` to `b` through their midpoint, `command` held or not.
fn drag(app: &mut PhotocraftApp, ctx: &egui::Context, command: bool, a: [f64; 2], b: [f64; 2]) {
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": mid[0], "y": mid[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    pointer(app, ctx, command, events);
}

/// The issue's line's ends: rising 10° to the right across the canvas.
fn ends() -> ([f64; 2], [f64; 2]) {
    ([100.0, 300.0], [700.0, 300.0 - 600.0 * 10.0f64.to_radians().tan()])
}

/// The issue's line, `command` held or not.
fn line(app: &mut PhotocraftApp, ctx: &egui::Context, command: bool) {
    let (a, b) = ends();
    drag(app, ctx, command, a, b);
}

/// ↵: commits the crop and returns the document's new size.
fn committed(app: &mut PhotocraftApp) -> (u32, u32) {
    photocraft_ui_egui::canvas::commit_crop(app);
    let size = app.session.active().expect("document").doc.size;
    (size.width, size.height)
}

/// The flattened document's pixel at (`x`, `y`).
fn pixel(app: &PhotocraftApp, x: usize, y: usize) -> [f32; 4] {
    let doc = &app.session.active().expect("document").doc;
    let flat = photocraft_compose::flatten(doc);
    flat.px[y * doc.size.width as usize + x]
}

fn is_blue(p: [f32; 4]) -> bool {
    BLUE.iter().zip(p).all(|(w, g)| (w - g).abs() < 0.02)
}

fn is_white(p: [f32; 4]) -> bool {
    p[..3].iter().all(|&v| v > 0.98)
}

fn is_red(p: [f32; 4]) -> bool {
    p[0] > 0.98 && p[1] < 0.02 && p[2] < 0.02
}

/// The four corner pixels of the document, in `crop_ui::corners`' order: top-left, top-right,
/// bottom-right, bottom-left.
fn corners(app: &PhotocraftApp) -> [[f32; 4]; 4] {
    let size = app.session.active().expect("document").doc.size;
    let (r, b) = (size.width as usize - 1, size.height as usize - 1);
    [pixel(app, 0, 0), pixel(app, r, 0), pixel(app, r, b), pixel(app, 0, b)]
}

/// The issue's row: the frame turns and keeps its size and position, and ↵ fills what the turn
/// uncovers. Also with a 1:1 ratio preset's frame and with Delete Cropped Pixels off.
#[test]
fn a_command_held_straighten_line_keeps_the_frame_size() {
    for classic in [false, true] {
        for (ratio, frame) in [("", [0.0, 0.0, 800.0, 500.0]), ("1:1", [150.0, 0.0, 650.0, 500.0])] {
            for delete in [true, false] {
                let what = format!("classic {classic}, ratio {ratio:?}, delete {delete}");
                let (mut app, ctx) = app(classic, ratio);
                app.ui.tool_options.crop_delete = delete;
                assert_eq!(app.ui.crop_rect, Some(frame), "{what}: the default frame");
                line(&mut app, &ctx, true);
                // The frame turns (counter-clockwise: the line rises to the right) and stays put.
                assert!((angle(&app) + 10.0).abs() < 1e-6, "{what}: {}", app.ui.crop_angle);
                assert_eq!(app.ui.crop_rect, Some(frame), "{what}");
                assert!(pending(&app), "{what}: a pending edit");
                assert!(!photocraft_ui_egui::crop_straighten::mode(&app), "{what}: the line ended");
                // Where the turned frame's corners land on the canvas: the whole-canvas frame's
                // all leave it; the 500 × 500 one's top-right and bottom-left leave it at the top
                // and bottom, the other two stay inside the 800 px width.
                let turn = angle(&app);
                let outside = turned_corners(frame, turn).map(|c| !(0.0..=800.0).contains(&c[0]) || !(0.0..=500.0).contains(&c[1]));
                let size = (frame[2] - frame[0], frame[3] - frame[1]);
                assert_eq!(committed(&mut app), (size.0 as u32, size.1 as u32), "{what}");
                // The corners the turn uncovered take the background colour, the others and the
                // middle are the image.
                for (p, out) in corners(&app).into_iter().zip(outside) {
                    assert!(if out { is_blue(p) } else { is_white(p) }, "{what}: a corner {} the canvas is {p:?}", if out { "off" } else { "on" });
                }
                assert_eq!(outside, if ratio.is_empty() { [true; 4] } else { [false, true, false, true] }, "{what}");
                let mid = pixel(&app, (size.0 / 2.0) as usize, (size.1 / 2.0) as usize);
                assert!(is_white(mid), "{what}: the white image in the middle, not {mid:?}");
                // Delete Cropped Pixels off keeps the turned image outside the frame: Reveal All
                // brings it back. Both frames are centred on the canvas's centre, so the revealed
                // canvas is too. The point 300 px left of and 270 px above that centre (on the
                // turned frame's axes) lies outside the frame and on the white image.
                let q = [300.0, 270.0];
                let [on_image, ..] = turned_corners([400.0 - q[0], 250.0 - q[1], 400.0 + q[0], 250.0 + q[1]], turn);
                assert!(on_image[0] > 10.0 && on_image[1] > 10.0 && on_image[0] < 270.0, "{what}: the point is on the image, at {on_image:?}");
                let revealed = app.run("image.revealAll", json!({})).expect("reveal all");
                let doc_size = app.session.active().expect("document").doc.size;
                let (w, h) = (f64::from(doc_size.width), f64::from(doc_size.height));
                if delete {
                    assert_eq!((revealed["changed"].clone(), w, h), (json!(false), size.0, size.1), "{what}: nothing left to reveal");
                } else {
                    assert!(w > size.0 + 40.0 && h > size.1 + 40.0, "{what}: revealed {w} × {h}");
                    let p = pixel(&app, (w / 2.0 - q[0]) as usize, (h / 2.0 - q[1]) as usize);
                    assert!(is_white(p), "{what}: the image outside the frame is kept, not {p:?}");
                }
            }
        }
    }
}

/// A frame drawn off the canvas's centre keeps its size and position through ⌘ lines, the
/// second one turning the already turned frame to its own angle.
#[test]
fn a_command_held_line_keeps_a_drawn_frame_where_it_is() {
    let frame = [100.0, 50.0, 500.0, 350.0];
    for classic in [false, true] {
        let (mut app, ctx) = app(classic, "");
        // Auto Center Preview stays on (the default): in the default mode it pans the view to the
        // drawn box, which must not move the frame on the image.
        assert!(app.ui.tool_options.crop_shield.auto_center_preview);
        drag(&mut app, &ctx, false, [frame[0], frame[1]], [frame[2], frame[3]]);
        assert_eq!(app.ui.crop_rect, Some(frame), "classic {classic}: the drawn frame");
        line(&mut app, &ctx, true);
        assert!((angle(&app) + 10.0).abs() < 1e-6, "classic {classic}: {}", app.ui.crop_angle);
        assert_eq!(app.ui.crop_rect, Some(frame), "classic {classic}: after the first line");
        // A line falling 4° to the right sets the frame's angle to 4°, not 10° - 4°.
        drag(&mut app, &ctx, true, [150.0, 100.0], [450.0, 100.0 + 300.0 * 4.0f64.to_radians().tan()]);
        assert!((angle(&app) - 4.0).abs() < 1e-6, "classic {classic}: {}", app.ui.crop_angle);
        assert_eq!(app.ui.crop_rect, Some(frame), "classic {classic}: after the second line");
        assert_eq!(committed(&mut app), (400, 300), "classic {classic}");
        // The crop is centred on the red square, where the frame was (its red pixels' centroid
        // is the middle, which one pixel in the 40 px square can't tell); turned 4° the frame
        // stays inside the image, so nothing is filled.
        let flat = photocraft_compose::flatten(&app.session.active().expect("document").doc);
        let red: Vec<(f64, f64)> = (0..300usize)
            .flat_map(|y| (0..400usize).map(move |x| (x, y)))
            .filter(|&(x, y)| is_red(flat.px[y * 400 + x]))
            .map(|(x, y)| (x as f64, y as f64))
            .collect();
        assert!(red.len() > 1000, "classic {classic}: the square is in the crop ({} red pixels)", red.len());
        let n = red.len() as f64;
        let (cx, cy) = (red.iter().map(|p| p.0).sum::<f64>() / n, red.iter().map(|p| p.1).sum::<f64>() / n);
        // Pixel centres sit at +0.5, so the middle of 400 × 300 is (199.5, 149.5).
        assert!((cx - 199.5).abs() <= 1.0 && (cy - 149.5).abs() <= 1.0, "classic {classic}: the red centroid is ({cx:.1}, {cy:.1})");
        for p in corners(&app) {
            assert!(is_white(p), "classic {classic}: a corner is {p:?}");
        }
    }
}

/// ⌘ let go before the mouse button: the line started as a ⌘ line, so it still keeps the frame.
#[test]
fn releasing_command_before_the_end_of_the_line_still_keeps_the_frame() {
    for classic in [false, true] {
        let (mut app, ctx) = app(classic, "");
        let (a, b) = ends();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        pointer(&mut app, &ctx, true, json!([{"kind": "down", "x": a[0], "y": a[1]}, {"kind": "move", "x": mid[0], "y": mid[1]}]));
        pointer(&mut app, &ctx, false, json!([{"kind": "move", "x": b[0], "y": b[1]}, {"kind": "up", "x": b[0], "y": b[1]}]));
        assert!((angle(&app) + 10.0).abs() < 1e-6, "classic {classic}: {}", app.ui.crop_angle);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "classic {classic}");
        assert_eq!(committed(&mut app), (800, 500), "classic {classic}");
    }
}

/// Either side of 45°: a line rising 44° to the right is levelled (the frame turns -44°), one
/// rising 46° is made plumb (it leans 44° the other way, so +44°). The frame keeps its size both
/// ways, however far it turns.
#[test]
fn a_steep_command_held_line_keeps_the_frame_level_or_plumb() {
    for classic in [false, true] {
        for (rise, want) in [(44.0f64, -44.0), (46.0, 44.0)] {
            let what = format!("classic {classic}, rising {rise}°");
            let (mut app, ctx) = app(classic, "");
            let (s, c) = rise.to_radians().sin_cos();
            drag(&mut app, &ctx, true, [400.0 - 200.0 * c, 250.0 + 200.0 * s], [400.0 + 200.0 * c, 250.0 - 200.0 * s]);
            assert!((angle(&app) - want).abs() < 1e-6, "{what}: {}", app.ui.crop_angle);
            assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "{what}");
            assert_eq!(committed(&mut app), (800, 500), "{what}");
            for p in corners(&app) {
                assert!(is_blue(p), "{what}: a corner is {p:?}");
            }
        }
    }
}

/// A ⌘-click, or a ⌘ line too short to measure, changes nothing: the default frame stays the
/// default, so ↵ crops nothing.
#[test]
fn a_command_held_click_or_tiny_line_changes_nothing() {
    for classic in [false, true] {
        let (mut app, ctx) = app(classic, "");
        drag(&mut app, &ctx, true, [400.0, 250.0], [400.0, 250.0]);
        drag(&mut app, &ctx, true, [400.0, 250.0], [400.001, 249.999]);
        assert_eq!((app.ui.crop_rect, angle(&app)), (Some([0.0, 0.0, 800.0, 500.0]), 0.0), "classic {classic}");
        assert!(!pending(&app), "classic {classic}: still the default frame");
        let steps = app.session.active().expect("document").history.past_len();
        assert_eq!(committed(&mut app), (800, 500), "classic {classic}");
        assert_eq!(app.session.active().expect("document").history.past_len(), steps, "classic {classic}: no crop");
    }
}

/// Undo steps the box back before ↵ (the frame upright, the default again) and Redo turns it
/// again; after ↵ the crop is one document step that Undo reverts.
#[test]
fn a_command_held_line_undoes_as_one_box_step_and_commits_as_one_document_step() {
    for classic in [false, true] {
        let (mut app, ctx) = app(classic, "");
        line(&mut app, &ctx, true);
        app.run("edit.undo", json!({})).expect("undo");
        assert_eq!((app.ui.crop_rect, angle(&app)), (Some([0.0, 0.0, 800.0, 500.0]), 0.0), "classic {classic}: undone");
        assert!(!pending(&app), "classic {classic}: the default frame again");
        app.run("edit.redo", json!({})).expect("redo");
        assert!((angle(&app) + 10.0).abs() < 1e-6 && pending(&app), "classic {classic}: redone");
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 800.0, 500.0]), "classic {classic}: redone");
        let steps = app.session.active().expect("document").history.past_len();
        assert_eq!(committed(&mut app), (800, 500), "classic {classic}");
        assert_eq!(app.session.active().expect("document").history.past_len(), steps + 1, "classic {classic}");
        assert!(is_blue(pixel(&app, 0, 0)), "classic {classic}: filled");
        app.run("edit.undo", json!({})).expect("undo the crop");
        assert!(is_white(pixel(&app, 0, 0)) && is_red(pixel(&app, 300, 200)), "classic {classic}: the image as it was");
    }
}

/// The button's line fits the frame inside the turned image, keeping the frame's proportions
/// (the canvas's, or a 1:1 preset's), also with ⌘ held while the button is armed.
#[test]
fn the_straighten_button_still_fits_the_frame() {
    let (s, c) = 10.0f64.to_radians().sin_cos();
    // The 1:1 frame's side: its turned bounds fit the canvas's height.
    let square = 500.0 / (s + c);
    for classic in [false, true] {
        for (ratio, command, want) in [("", false, (633.0, 396.0)), ("", true, (633.0, 396.0)), ("1:1", false, (square, square))] {
            let what = format!("classic {classic}, ratio {ratio:?}, command {command}");
            let (mut app, ctx) = app(classic, ratio);
            photocraft_ui_egui::crop_straighten::toggle(&mut app);
            line(&mut app, &ctx, command);
            assert!((angle(&app) + 10.0).abs() < 1e-6, "{what}");
            assert!(!photocraft_ui_egui::crop_straighten::mode(&app), "{what}: one line per click");
            let (w, h) = committed(&mut app);
            assert!((f64::from(w) - want.0).abs() <= 1.0 && (f64::from(h) - want.1).abs() <= 1.0, "{what}: {w} × {h}");
            if ratio == "1:1" {
                assert_eq!(w, h, "{what}: keeps the frame's 1:1");
            }
            // Fitted inside the turned image: nothing to fill.
            for p in corners(&app) {
                assert!(!is_blue(p), "{what}: a corner is {p:?}");
            }
        }
    }
}
