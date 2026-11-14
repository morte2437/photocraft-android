//! #11741: View › Extras (⌘H) hides the Color Sampler markers on the canvas, as in Photoshop. The
//! points stay in the document and the Info panel keeps reading them; Extras on again draws the
//! markers back at the same places with the same numbers.

use super::*;

/// A 200 × 100 document with two samplers and the Color Sampler tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::ColorSampler;
    app.run("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    app.run("view.colorSamplers.add", json!({"x": 120, "y": 30})).unwrap();
    app
}

fn xf() -> ViewXform {
    ViewXform {
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0)),
        zoom: 1.0,
        center: [100.0, 50.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    }
}

/// Every shape the overlay paints, and the marker numbers with where they are.
fn painted(app: &PhotocraftApp, ctx: &egui::Context) -> (usize, Vec<(String, egui::Pos2)>) {
    let xf = xf();
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    let numbers = o
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos)),
            _ => None,
        })
        .collect();
    (o.shapes.len(), numbers)
}

fn toggle_extras(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::menus::invoke(app, ctx, "view.extras", json!({})).unwrap();
}

/// The Info panel's sampler readouts, read afresh rather than from the per-revision cache.
fn readouts(app: &mut PhotocraftApp) -> Vec<Value> {
    app.info_samplers = None;
    crate::panels::info::sampler_readouts(app).to_vec()
}

#[test]
fn extras_off_hides_the_sampler_markers_and_keeps_the_points_and_readouts() {
    let mut app = app();
    let ctx = egui::Context::default();
    let (shown, numbers) = painted(&app, &ctx);
    assert!(shown > 0, "the markers are drawn with Extras on");
    let points = app.session.active().unwrap().color_samplers.clone();
    let info = readouts(&mut app);
    assert_eq!(info.len(), 2, "the Info panel lists both samplers: {info:?}");

    toggle_extras(&mut app, &ctx);
    assert!(!app.ui.view.extras);
    assert_eq!(painted(&app, &ctx).0, 0, "no marker is drawn with Extras off");
    assert_eq!(app.session.active().unwrap().color_samplers, points, "the samplers stay in the document");
    assert_eq!(readouts(&mut app), info, "the Info panel readouts are unchanged");

    toggle_extras(&mut app, &ctx);
    assert!(app.ui.view.extras);
    assert_eq!(painted(&app, &ctx), (shown, numbers), "Extras on again draws the same markers at the same places");
}

#[test]
fn extras_off_hides_a_marker_being_dragged() {
    let mut app = app();
    let ctx = egui::Context::default();
    toggle_extras(&mut app, &ctx);
    app.ui.analysis.drag = Some(Drag::Sampler(0, [80.0, 60.0]));
    assert_eq!(painted(&app, &ctx).0, 0, "a dragged marker stays hidden");
    app.ui.analysis.drag = Some(Drag::SamplerNew([30.0, 70.0]));
    assert_eq!(painted(&app, &ctx).0, 0, "a new marker stays hidden");
}
