//! #10931: the default Crop turn temporarily replaces the chosen overlay with
//! centred 48-point cells; Classic Mode and Straighten keep their existing look.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::crop_overlay::{self, CropOverlay, OverlayShow, Polyline};
use photocraft_ui_egui::crop_ui;
use photocraft_ui_egui::state::Tool;
use serde_json::{Value, json};

fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let (request, _rx) = ControlRequest::new(method, params.clone());
    assert!(matches!(handle(app, ctx, &request), Outcome::Done(_)), "{method} {params}");
}

fn app(classic: bool, show: OverlayShow) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":412,"height":301})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Crop;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let o = &mut app.ui.tool_options;
    o.crop_shield.classic_mode = classic;
    o.crop_shield.auto_center_preview = false;
    o.crop_overlay_show = show;
    o.crop_overlay = CropOverlay::Triangle;
    o.crop_overlay_orientation = 1;
    crop_ui::ensure_frame(&mut app);
    (app, ctx)
}

fn turning(app: &mut PhotocraftApp, ctx: &egui::Context) -> [f64; 2] {
    turning_by(app, ctx, 10.0)
}

fn turning_by(app: &mut PhotocraftApp, ctx: &egui::Context, degrees: f64) -> [f64; 2] {
    let c = crop_ui::center(app.ui.crop_rect.unwrap());
    let p = [c[0] + 500.0, c[1]];
    let a = degrees.to_radians();
    let q = [c[0] + 500.0 * a.cos(), c[1] + 500.0 * a.sin()];
    control(app, ctx, "ui.pointer", json!({"events":[{"kind":"down","x":p[0],"y":p[1]},{"kind":"move","x":q[0],"y":q[1]}]}));
    assert!(crop_ui::turning(app).is_some(), "the real pointer path must start a turn");
    q
}

fn release(app: &mut PhotocraftApp, ctx: &egui::Context, p: [f64; 2]) {
    control(app, ctx, "ui.pointer", json!({"events":[{"kind":"up","x":p[0],"y":p[1]}]}));
    assert!(crop_ui::turning(app).is_none());
}

fn positions(lines: &[Polyline], vertical: bool, len: f32) -> Vec<f32> {
    lines.iter().filter(|l| (l[0][0] == l[1][0]) == vertical).map(|l| l[0][if vertical { 0 } else { 1 }] * len).collect()
}

fn near(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 0.001, "{actual:?}, expected {expected:?}");
    }
}

fn full_grid(w: f32, h: f32) -> Vec<Polyline> {
    crop_overlay::turn_grid(w, h, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h)))
}

#[test]
fn measured_frame_sizes_have_centred_48_point_cells() {
    let grid = full_grid(329.0, 240.0);
    near(&positions(&grid, true, 329.0), &[20.5, 68.5, 116.5, 164.5, 212.5, 260.5, 308.5]);
    near(&positions(&grid, false, 240.0), &[24.0, 72.0, 120.0, 168.0, 216.0]);
    let grid = full_grid(711.0, 519.0);
    near(&positions(&grid, true, 711.0), &[19.5, 67.5, 115.5, 163.5, 211.5, 259.5, 307.5, 355.5, 403.5, 451.5, 499.5, 547.5, 595.5, 643.5, 691.5]);
    near(&positions(&grid, false, 519.0), &[19.5, 67.5, 115.5, 163.5, 211.5, 259.5, 307.5, 355.5, 403.5, 451.5, 499.5]);
}

#[test]
fn default_turn_replaces_the_overlay_and_release_restores_each_show_mode() {
    for show in [OverlayShow::Always, OverlayShow::Auto, OverlayShow::Never] {
        let (mut app, ctx) = app(false, show);
        let p = turning(&mut app, &ctx);
        let expected = if show == OverlayShow::Never { Vec::new() } else { full_grid(329.0, 240.0) };
        assert_eq!(crop_overlay::current_guides(&app, 329.0, 240.0), expected, "{show:?}");
        release(&mut app, &ctx, p);
        let expected = if show == OverlayShow::Always { crop_overlay::guides(CropOverlay::Triangle, 1, 329.0, 240.0) } else { Vec::new() };
        assert_eq!(crop_overlay::current_guides(&app, 329.0, 240.0), expected, "{show:?} after release");
    }
}

#[test]
fn zoom_and_hidpi_do_not_change_screen_point_spacing() {
    let (mut app, ctx) = app(false, OverlayShow::Always);
    ctx.set_pixels_per_point(2.0);
    control(&mut app, &ctx, "ui.set", json!({"zoom":2.0}));
    let p = turning(&mut app, &ctx);
    let q = [egui::pos2(0.0, 0.0), egui::pos2(711.0, 0.0), egui::pos2(711.0, 519.0), egui::pos2(0.0, 519.0)];
    let lines = crop_overlay::current_guides_for_frame(&app, q, egui::Rect::from_min_max(q[0], q[2]));
    let xs = positions(&lines, true, 711.0);
    assert_eq!(xs.len(), 15);
    for pair in xs.windows(2) {
        assert!((pair[1] - pair[0] - 48.0).abs() < 0.001, "{xs:?}");
    }
    release(&mut app, &ctx, p);
}

#[test]
fn canvas_paints_48_point_cells_at_zoom_two_and_hidpi() {
    fn collect(shape: &egui::Shape, clip: egui::Rect, lines: &mut Vec<[egui::Pos2; 2]>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, clip, lines)),
            egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(egui::Color32::from_white_alpha(90)) => {
                assert_eq!(path.points.len(), 2);
                assert_eq!(path.stroke.width, 1.0);
                assert!(path.points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
                let points = [path.points[0], path.points[1]];
                assert!(egui::Rect::from_two_pos(points[0], points[1]).intersects(clip));
                lines.push(points);
            }
            _ => {}
        }
    }

    // Exercise both paint call sites: the initial upright turn and a moved
    // default-mode turn whose camera counter-rotates the frame.
    for degrees in [0.0, 10.0] {
        let (mut app, ctx) = app(false, OverlayShow::Always);
        ctx.set_pixels_per_point(2.0);
        control(&mut app, &ctx, "ui.set", json!({"zoom":2.0}));
        let p = turning_by(&mut app, &ctx, degrees);
        assert_eq!(crop_ui::angle(&app) == 0.0, degrees == 0.0);
        app.ui.extras.rulers = false;
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(600.0, 480.0));
        // Deliberately narrower than the canvas and frame, to detect a caller
        // passing the whole canvas instead of the painter's actual clip.
        let clip = egui::Rect::from_center_size(rect.center(), egui::vec2(180.0, 130.0));
        let mut view = app.ui.views[0].clone();
        view.fit_pending = false;
        view.fill_pending = false;
        view.doc_size = [412, 301];
        view.center = [206.0, 150.5];
        let xf = photocraft_ui_egui::canvas::ViewXform { rect, zoom: 1.0, center: view.center, flip: false, rotation: view.rotation, aspect: 1.0 };
        let q = crop_ui::corners(app.ui.crop_rect.unwrap(), crop_ui::angle(&app)).map(|p| xf.to_screen(p[0] as f32, p[1] as f32));
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_clip_rect(clip);
            let painted_view = photocraft_ui_egui::canvas::canvas_view(&mut app, ui, 0, rect, view.clone(), true);
            assert_eq!(painted_view.zoom, 2.0);
        });
        // Shape-only CPU inspection has no renderer to consume texture updates.
        output.textures_delta.clear();
        assert_eq!(output.pixels_per_point, 2.0);
        assert_eq!(app.canvas_ppp(), 2.0);
        assert!(crop_ui::turning(&app).is_some());
        let mut lines = Vec::new();
        for shape in &output.shapes {
            let before = lines.len();
            collect(&shape.shape, clip, &mut lines);
            if lines.len() != before {
                assert_eq!(shape.clip_rect, clip);
            }
        }
        assert_eq!(lines.len(), 6, "only the three visible lines per axis: {degrees} degrees, {lines:?}");
        for vertical in [true, false] {
            let mut positions = Vec::new();
            for points in &lines {
                let delta = points[1] - points[0];
                if (delta.x.abs() < 0.001) == vertical {
                    let (coordinate, lo, hi) = if vertical { (points[0].x, q[0].y, q[3].y) } else { (points[0].y, q[0].x, q[1].x) };
                    let endpoints = if vertical { [points[0].y, points[1].y] } else { [points[0].x, points[1].x] };
                    near(&endpoints, &[lo, hi]);
                    positions.push(coordinate);
                }
            }
            positions.sort_by(f32::total_cmp);
            let center = if vertical { rect.center().x } else { rect.center().y };
            near(&positions, &[center - 48.0, center, center + 48.0]);
        }
        release(&mut app, &ctx, p);
    }
}

#[test]
fn classic_turn_keeps_the_selected_overlay() {
    let (mut app, ctx) = app(true, OverlayShow::Always);
    let p = turning(&mut app, &ctx);
    assert_eq!(crop_overlay::current_guides(&app, 329.0, 240.0), crop_overlay::guides(CropOverlay::Triangle, 1, 329.0, 240.0));
    release(&mut app, &ctx, p);
}

#[test]
fn straighten_line_keeps_the_selected_overlay() {
    let (mut app, ctx) = app(false, OverlayShow::Always);
    control(&mut app, &ctx, "ui.pointer", json!({"command":true,"events":[{"kind":"down","x":40,"y":80},{"kind":"move","x":300,"y":110}]}));
    assert!(crop_ui::turning(&app).is_none());
    assert_eq!(crop_overlay::current_guides(&app, 329.0, 240.0), crop_overlay::guides(CropOverlay::Triangle, 1, 329.0, 240.0));
    control(&mut app, &ctx, "ui.pointer", json!({"command":true,"events":[{"kind":"up","x":300,"y":110}]}));
}

#[test]
fn invalid_sizes_are_empty_and_huge_frames_keep_bounded_guides() {
    for (w, h) in [(0.0, 240.0), (329.0, 0.0), (f32::NAN, 240.0), (329.0, f32::INFINITY)] {
        assert!(full_grid(w, h).is_empty());
    }
    // A panned 1000-point viewport far outside the old centre-based cap.
    let visible = egui::Rect::from_min_max(egui::pos2(900_000.0, 800_000.0), egui::pos2(901_000.0, 801_000.0));
    let grid = crop_overlay::turn_grid(1e6, 1e6, visible);
    assert_eq!(grid.len(), 42, "the visible extent must have actual guides, not just satisfy an upper bound");
    let xs = positions(&grid, true, 1e6);
    assert!((xs[0] - 900_032.0).abs() < 0.1 && (xs[xs.len() - 1] - 900_992.0).abs() < 0.1, "{xs:?}");
    assert!(grid.iter().flatten().flatten().all(|f| f.is_finite() && (0.0..=1.0).contains(f)));
    let hostile = full_grid(1e6, 1e6);
    assert!(!hostile.is_empty() && hostile.len() <= 8192);
}

#[test]
fn a_zoomed_panned_view_is_covered_without_visiting_the_whole_frame() {
    let (mut app, ctx) = app(false, OverlayShow::Always);
    let p = turning(&mut app, &ctx);
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
    let c = crop_ui::corners(app.ui.crop_rect.unwrap(), crop_ui::angle(&app));
    let at = [c[0][0] + 0.8 * (c[1][0] - c[0][0]) + 0.7 * (c[3][0] - c[0][0]), c[0][1] + 0.8 * (c[1][1] - c[0][1]) + 0.7 * (c[3][1] - c[0][1])];
    control(&mut app, &ctx, "ui.set", json!({"zoom":100.0,"center":at}));
    assert!(crop_ui::turning(&app).is_some());
    let xf = photocraft_ui_egui::canvas::ViewXform::active(&app).unwrap();
    let q = crop_ui::corners(app.ui.crop_rect.unwrap(), crop_ui::angle(&app)).map(|p| xf.to_screen(p[0] as f32, p[1] as f32));
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    assert!(u.y.abs() < 0.1 && v.x.abs() < 0.1, "the default-mode frame stays upright");
    assert!(u.length() > 4608.0 && v.length() > 4608.0, "exercise a frame beyond the old centre cap");
    // Model navigation's local camera being newer than the stored one: guides
    // must use the same quad/clip as paint, not reconstruct the stored camera.
    app.ui.views[0].center = [206.0, 150.5];
    let lines = crop_overlay::current_guides_for_frame(&app, q, xf.rect);
    assert!(!lines.is_empty() && lines.len() < 50, "work is bounded by the visible viewport: {}", lines.len());
    for (vertical, lo, hi) in [(true, xf.rect.left(), xf.rect.right()), (false, xf.rect.top(), xf.rect.bottom())] {
        let mut positions: Vec<f32> = lines
            .iter()
            .filter(|l| (l[0][0] == l[1][0]) == vertical)
            .map(|l| {
                let screen = q[0] + u * l[0][0] + v * l[0][1];
                if vertical { screen.x } else { screen.y }
            })
            .collect();
        positions.sort_by(f32::total_cmp);
        let first = *positions.first().unwrap();
        let last = *positions.last().unwrap();
        assert!(first >= lo - 0.1 && first - lo < 48.1 && last <= hi + 0.1 && hi - last < 48.1, "{positions:?}, viewport {lo}..{hi}");
        assert!(positions.windows(2).all(|p| (p[1] - p[0] - 48.0).abs() < 0.1), "{positions:?}");
    }
    release(&mut app, &ctx, p);
}

#[test]
fn rotated_and_flipped_viewports_keep_frame_axis_spacing() {
    let (mut app, ctx) = app(false, OverlayShow::Always);
    let p = turning(&mut app, &ctx);
    let clip = egui::Rect::from_min_max(egui::pos2(100.0, 150.0), egui::pos2(260.0, 350.0));
    for flipped in [false, true] {
        // The measured329x240 frame turned90deg; reflecting its horizontal
        // direction exercises the inverse basis's negative determinant too.
        let (x, side) = if flipped { (60.0, 300.0) } else { (300.0, 60.0) };
        let q = [egui::pos2(x, 100.0), egui::pos2(x, 429.0), egui::pos2(side, 429.0), egui::pos2(side, 100.0)];
        let lines = crop_overlay::current_guides_for_frame(&app, q, clip);
        near(&positions(&lines, true, 329.0), &[68.5, 116.5, 164.5, 212.5]);
        near(&positions(&lines, false, 240.0), &[72.0, 120.0, 168.0]);
        for line in lines {
            let at = |t: [f32; 2]| q[0] + (q[1] - q[0]) * t[0] + (q[3] - q[0]) * t[1];
            let delta = at(line[1]) - at(line[0]);
            if line[0][0] == line[1][0] {
                assert!(delta.y.abs() < 0.001 && (delta.x.abs() - 240.0).abs() < 0.001);
            } else {
                assert!(delta.x.abs() < 0.001 && (delta.y.abs() - 329.0).abs() < 0.001);
            }
        }
    }
    // A non-cardinal view turn: the square viewport projects to centre +/-
    // 96*sqrt(2), so five48-point lines per frame axis intersect it.
    let reach = 240.0 * std::f32::consts::SQRT_2;
    let q = [egui::pos2(500.0, 500.0 - reach), egui::pos2(500.0 + reach, 500.0), egui::pos2(500.0, 500.0 + reach), egui::pos2(500.0 - reach, 500.0)];
    let clip = egui::Rect::from_min_max(egui::pos2(404.0, 404.0), egui::pos2(596.0, 596.0));
    let lines = crop_overlay::current_guides_for_frame(&app, q, clip);
    near(&positions(&lines, true, 480.0), &[144.0, 192.0, 240.0, 288.0, 336.0]);
    near(&positions(&lines, false, 480.0), &[144.0, 192.0, 240.0, 288.0, 336.0]);
    release(&mut app, &ctx, p);
    assert_eq!(crop_overlay::current_guides_for_frame(&app, q, clip), crop_overlay::guides(CropOverlay::Triangle, 1, 480.0, 480.0));
    app.ui.tool = Tool::Brush;
    assert!(crop_overlay::current_guides_for_frame(&app, q, clip).is_empty());
}

#[test]
fn empty_clips_and_degenerate_frame_bases_have_no_guides() {
    let (mut app, ctx) = app(false, OverlayShow::Always);
    let p = turning(&mut app, &ctx);
    let frame = [egui::pos2(0.0, 0.0), egui::pos2(329.0, 0.0), egui::pos2(329.0, 240.0), egui::pos2(0.0, 240.0)];
    assert!(crop_overlay::current_guides_for_frame(&app, frame, egui::Rect::NOTHING).is_empty());
    let clip = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(329.0, 240.0));
    assert!(crop_overlay::current_guides_for_frame(&app, [egui::Pos2::ZERO; 4], clip).is_empty());
    let collinear = [egui::Pos2::ZERO, egui::pos2(329.0, 0.0), egui::pos2(569.0, 0.0), egui::pos2(240.0, 0.0)];
    assert!(crop_overlay::current_guides_for_frame(&app, collinear, clip).is_empty());
    let mut nonfinite = frame;
    nonfinite[1].x = f32::NAN;
    assert!(crop_overlay::current_guides_for_frame(&app, nonfinite, clip).is_empty());
    let offscreen = egui::Rect::from_min_max(egui::pos2(400.0, 400.0), egui::pos2(600.0, 600.0));
    assert!(crop_overlay::current_guides_for_frame(&app, frame, offscreen).is_empty());
    release(&mut app, &ctx, p);
}
