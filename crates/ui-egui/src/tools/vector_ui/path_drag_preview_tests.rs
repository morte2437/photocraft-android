//! #9306: a Path Selection drag shows its path following the pointer, where the release will put
//! it, instead of leaving it in place until the release. The document only changes on release.

use egui::{Modifiers, Pos2};
use photocraft_doc::vector::Path;
use photocraft_geom::{Affine, Point};
use serde_json::json;

use super::path_ops::{MovedPath, path_selection_preview};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::state::Tool;

const NAME: &str = "Path 1";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app.ui.extras.snap = false;
    app
}

fn square(x: f64, y: f64, w: f64) -> serde_json::Value {
    json!({"closed": true, "knots": [[x, y], [x + w, y], [x + w, y + w], [x, y + w]]})
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn revision(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().revision
}

fn preview(app: &PhotocraftApp) -> Option<(MovedPath, Path)> {
    path_selection_preview(app, &app.session.active().unwrap().doc)
}

fn press_and_move(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    assert!(app.drag.is_some());
}

fn release(app: &mut PhotocraftApp, at: [f64; 2]) {
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, Modifiers::NONE);
}

fn anchors(p: &Path) -> Vec<Point> {
    p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor)).collect()
}

/// The centres of the anchor squares the vector overlay draws, document = screen pixels.
fn drawn_anchors(app: &PhotocraftApp) -> Vec<Pos2> {
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200., 200.)),
        zoom: 1.0,
        center: [100., 100.],
        flip: false,
        rotation: 0.,
        aspect: 1.,
    };
    let doc = app.session.active().unwrap().doc.clone();
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| super::draw_overlay(app, ui.painter(), &xf, &doc));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if (r.rect.width() - 6.0).abs() < 0.01 => Some(r.rect.center()),
            _ => None,
        })
        .collect()
}

fn near(drawn: &[Pos2], p: Point) -> bool {
    drawn.iter().any(|q| (f64::from(q.x) - p.x).abs() < 0.01 && (f64::from(q.y) - p.y).abs() < 0.01)
}

#[test]
fn a_work_path_drag_shows_the_path_where_the_release_puts_it() {
    let mut app = app();
    app.run("path.set", json!({"path": {"subpaths": [square(20.0, 20.0, 40.0)]}})).unwrap();
    let before = work(&app);
    let rev = revision(&app);
    press_and_move(&mut app, [20.0, 40.0], [30.4, 45.6]);
    assert_eq!(work(&app), before, "the drag itself leaves the document alone");
    assert_eq!(revision(&app), rev);
    let (which, shown) = preview(&app).expect("the drag previews the work path");
    assert_eq!(which, MovedPath::Work);
    // The same whole-pixel offset the release moves by.
    assert_eq!(shown, before.transform(&Affine::translate(10.0, 6.0)));
    let drawn = drawn_anchors(&app);
    assert!(anchors(&shown).iter().all(|p| near(&drawn, *p)), "anchors drawn at the moved path: {drawn:?}");
    assert!(!near(&drawn, Point::new(60.0, 20.0)), "not at the path's old corner: {drawn:?}");
    release(&mut app, [30.4, 45.6]);
    assert_eq!(work(&app), shown, "the release lands where the preview showed");
    assert!(preview(&app).is_none());
}

#[test]
fn a_click_shows_no_move() {
    let mut app = app();
    app.run("path.set", json!({"path": {"subpaths": [square(20.0, 20.0, 40.0)]}})).unwrap();
    press_and_move(&mut app, [20.0, 40.0], [20.2, 40.1]);
    assert!(preview(&app).is_none());
}

#[test]
fn only_the_selected_component_moves_in_the_preview() {
    let mut app = app();
    app.run("path.set", json!({"path": {"subpaths": [square(20.0, 20.0, 40.0), square(120.0, 120.0, 40.0)]}})).unwrap();
    let before = work(&app);
    // Pressing the first square's edge selects that component only.
    press_and_move(&mut app, [20.0, 40.0], [25.0, 47.0]);
    let (which, shown) = preview(&app).expect("the drag previews the component move");
    assert_eq!(which, MovedPath::Work);
    let moved = anchors(&shown);
    let old = anchors(&before);
    assert_eq!(&moved[..4], &old[..4].iter().map(|p| Point::new(p.x + 5.0, p.y + 7.0)).collect::<Vec<_>>()[..]);
    assert_eq!(&moved[4..], &old[4..], "the other component stays");
    let drawn = drawn_anchors(&app);
    assert!(near(&drawn, Point::new(25.0, 27.0)) && near(&drawn, Point::new(120.0, 120.0)), "{drawn:?}");
    release(&mut app, [25.0, 47.0]);
    assert_eq!(work(&app), shown);
}

#[test]
fn the_selected_saved_path_previews_too() {
    let mut app = app();
    app.run("path.set", json!({"name": NAME, "path": {"subpaths": [square(40.0, 40.0, 80.0)]}})).unwrap();
    app.ui.selected_path = Some(NAME.into());
    let saved = |app: &PhotocraftApp| app.session.active().unwrap().doc.paths.iter().find(|p| p.name == NAME).unwrap().path.clone();
    let before = saved(&app);
    press_and_move(&mut app, [40.0, 80.0], [52.0, 71.0]);
    let (which, shown) = preview(&app).expect("the drag previews the saved path");
    assert_eq!(which, MovedPath::Saved);
    assert_eq!(saved(&app), before);
    release(&mut app, [52.0, 71.0]);
    assert_eq!(saved(&app), shown);
}

#[test]
fn a_shape_layer_drag_previews_no_outline() {
    // Its pixels move only on release, so its outline does not run ahead of them.
    let mut app = app();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#ff0000"})).unwrap();
    press_and_move(&mut app, [40.0, 40.0], [50.0, 50.0]);
    assert!(preview(&app).is_none());
    release(&mut app, [50.0, 50.0]);
    let st = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    assert_eq!(sh.path.control_bounds().map(|b| [b.0, b.1]), Some([30.0, 30.0]), "the release still moves it");
}

#[test]
fn a_stale_drag_previews_nothing() {
    let mut app = app();
    app.run("path.set", json!({"path": {"subpaths": [square(20.0, 20.0, 40.0)]}})).unwrap();
    press_and_move(&mut app, [20.0, 40.0], [30.0, 50.0]);
    assert!(preview(&app).is_some());
    // The document changed under the drag, so its release moves nothing either.
    app.session.execute("path.set", json!({"name": NAME, "path": {"subpaths": [square(0.0, 0.0, 10.0)]}})).unwrap();
    assert!(preview(&app).is_none());
}
