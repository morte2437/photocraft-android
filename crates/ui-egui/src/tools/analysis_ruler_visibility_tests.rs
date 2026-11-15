//! #11987: the ruler stays drawn on the canvas whichever tool is active, and View › Extras (⌘H)
//! hides and shows it, also with the Ruler tool active. The ruler itself never changes.

use super::{PhotocraftApp, Ruler, Tool, ViewXform, draw_overlay, json};

/// A 200 × 100 document with a ruler from 20,20 to 180,80 and its protractor arm to 20,90.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [20, 20], "end": [180, 80], "protractor": [20, 90]})).unwrap();
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

fn ruler(app: &PhotocraftApp) -> Option<Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

/// The line segments the overlay paints from the ruler's start (its line and its protractor arm).
fn ruler_lines(app: &PhotocraftApp, ctx: &egui::Context) -> usize {
    let xf = xf();
    let start = xf.to_screen(20.0, 20.0);
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(app, ui.painter(), &xf));
    o.textures_delta.clear();
    o.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::LineSegment { points, .. } if points[0].distance(start) < 0.5)).count()
}

fn extras(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::menus::invoke(app, ctx, "view.extras", json!({})).unwrap();
}

#[test]
fn the_ruler_stays_drawn_under_other_tools() {
    let ctx = egui::Context::default();
    let mut app = app();
    let before = ruler(&app);
    for tool in [Tool::Ruler, Tool::Brush, Tool::Move, Tool::RectMarquee] {
        app.ui.tool = tool;
        assert!(ruler_lines(&app, &ctx) >= 2, "{tool:?}: the line and the protractor arm are drawn");
        assert_eq!(ruler(&app), before, "{tool:?}: the ruler is unchanged");
    }
}

#[test]
fn view_extras_hides_and_shows_it_with_any_tool() {
    let ctx = egui::Context::default();
    for tool in [Tool::Ruler, Tool::Brush] {
        let mut app = app();
        app.ui.tool = tool;
        let before = ruler(&app);
        assert!(app.ui.view.extras);
        extras(&mut app, &ctx);
        assert!(!app.ui.view.extras);
        assert_eq!(ruler_lines(&app, &ctx), 0, "{tool:?}: Extras off hides it");
        assert_eq!(ruler(&app), before, "{tool:?}: the measurement stays");
        extras(&mut app, &ctx);
        assert!(ruler_lines(&app, &ctx) >= 2, "{tool:?}: Extras on shows it again");
        assert_eq!(ruler(&app), before);
    }
}

#[test]
fn a_new_document_shows_no_ruler_and_the_first_one_shows_its_own_again() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.ui.tool = Tool::Brush;
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    assert_eq!(ruler_lines(&app, &ctx), 0, "the new document has none");
    app.run("document.activate", json!({"document": 0})).unwrap();
    assert!(ruler_lines(&app, &ctx) >= 2, "the first document's ruler is back");
}
