//! #10671: the selection's marching ants are painted over the guides, so a guide on a selection
//! edge doesn't hide it (it did at some zoom levels, where both land on the same device pixel).

use super::*;
use serde_json::json;

const GUIDE: Color32 = Color32::from_rgb(0x4a, 0xff, 0xff);

/// A document with guides on all four edges of a rectangular selection, as in the report.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    for (orientation, position) in [("vertical", 100), ("vertical", 300), ("horizontal", 100), ("horizontal", 260)] {
        app.run("view.newGuide", json!({"orientation": orientation, "position": position})).unwrap();
    }
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 160})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::RectMarquee;
    app.ui.extras.rulers = false;
    app.ui.extras.guides = true;
    app
}

/// The painted shapes' order in one frame: the indices of the guide lines and of the ants' meshes.
fn painted(app: &mut PhotocraftApp, ppp: f32, zoom: f32, primary: bool) -> (Vec<usize>, Vec<usize>) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    ctx.set_pixels_per_point(ppp);
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(700.0, 500.0));
    let view = View { zoom, center: [300.0, 200.0], doc_size: [600, 400], fit_pending: false, ..Default::default() };
    let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
        canvas_view(app, ui, 0, rect, view.clone(), primary);
    });
    output.textures_delta.clear();
    let (mut guides, mut ants) = (Vec::new(), Vec::new());
    for (i, s) in output.shapes.iter().enumerate() {
        match &s.shape {
            egui::Shape::LineSegment { stroke, .. } if stroke.color == GUIDE => guides.push(i),
            egui::Shape::Mesh(m) if m.vertices.iter().any(|v| v.color == Color32::BLACK) && m.vertices.iter().any(|v| v.color == Color32::WHITE) => {
                ants.push(i)
            }
            _ => {}
        }
    }
    (guides, ants)
}

#[test]
fn the_ants_are_painted_over_the_guides_at_every_zoom() {
    let mut app = app();
    for ppp in [1.0, 1.25, 2.0] {
        for zoom in [0.18, 0.25, 0.3333, 0.5, 0.6667, 1.0, 1.5, 2.0, 3.0] {
            let (guides, ants) = painted(&mut app, ppp, zoom, true);
            assert_eq!(guides.len(), 4, "@{ppp}x, {zoom}: the four guides are drawn");
            assert!(!ants.is_empty(), "@{ppp}x, {zoom}: the selection has its ants");
            let (last_guide, first_ants) = (guides.iter().max().unwrap(), ants.iter().min().unwrap());
            assert!(first_ants > last_guide, "@{ppp}x, {zoom}: ants at {ants:?} are under the guides at {guides:?}");
        }
    }
}

/// A secondary view (another tile of the document) draws no guides, but still its ants.
#[test]
fn a_secondary_view_still_draws_the_ants() {
    let mut app = app();
    let (guides, ants) = painted(&mut app, 1.0, 0.5, false);
    assert!(guides.is_empty());
    assert!(!ants.is_empty());
}
