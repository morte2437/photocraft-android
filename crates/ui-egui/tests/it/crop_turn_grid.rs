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
    let c = crop_ui::center(app.ui.crop_rect.unwrap());
    let p = [c[0] + 500.0, c[1]];
    let a = 10.0_f64.to_radians();
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
    let lines = crop_overlay::current_guides(&app, 711.0, 519.0);
    let xs = positions(&lines, true, 711.0);
    assert_eq!(xs.len(), 15);
    for pair in xs.windows(2) {
        assert!((pair[1] - pair[0] - 48.0).abs() < 0.001, "{xs:?}");
    }
    release(&mut app, &ctx, p);
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
    let lines = crop_overlay::current_guides(&app, u.length(), v.length());
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
