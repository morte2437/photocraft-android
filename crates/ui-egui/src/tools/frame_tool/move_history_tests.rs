//! Moving an empty frame with the Frame tool records "Move" (#13880).
use super::*;
use crate::canvas::tool_event;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn transform(app: &PhotocraftApp, id: LayerId) -> [f64; 6] {
    app.session.active().unwrap().doc.layer(id).unwrap().frame.as_ref().unwrap().transform.m
}

/// An empty 100 x 80 frame at 20,30, drawn with the Frame tool.
fn empty_frame(app: &mut PhotocraftApp) -> LayerId {
    drag(app, [20., 30.], [120., 110.]);
    selected(app).unwrap().id
}

#[test]
fn dragging_an_empty_frame_records_one_move_step_that_undoes_in_one() {
    let mut app = app();
    let id = empty_frame(&mut app);
    let before = entries(&app);
    drag(&mut app, [70., 70.], [90., 80.]);
    assert_eq!(transform(&app, id), [100., 0., 0., 80., 40., 40.], "moved +20,+10");
    let after = entries(&app);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().map(String::as_str), Some("Move"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(transform(&app, id), [100., 0., 0., 80., 20., 30.]);
    assert_eq!(entries(&app), before);
}

#[test]
fn property_changes_and_handle_drags_keep_frame_properties() {
    let mut app = app();
    let id = empty_frame(&mut app);
    app.run("layer.frame.set", json!({"layer":id.0,"stroke":{"color":"#ff0000","width":2}})).unwrap();
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
    app.run("layer.frame.set", json!({"layer":id.0,"transform":[120,0,0,80,20,30]})).unwrap();
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
    let steps = entries(&app).len();
    // The bottom-right resize handle.
    drag(&mut app, [140., 110.], [160., 130.]);
    assert_eq!(transform(&app, id).map(f64::round), [140., 0., 0., 100., 20., 30.], "resized, not moved");
    assert_eq!(entries(&app).len(), steps + 1);
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
}

#[test]
fn a_bad_history_name_is_refused_and_changes_nothing() {
    let mut app = app();
    let id = empty_frame(&mut app);
    let before = entries(&app);
    for bad in [json!(5), json!("Rotate"), json!(""), json!(["Move"]), json!({"name":"Move"}), json!("move")] {
        assert!(app.run("layer.frame.set", json!({"layer":id.0,"transform":[100,0,0,80,40,40],"historyName":bad})).is_err(), "{bad}");
        assert_eq!(entries(&app), before);
        assert_eq!(transform(&app, id), [100., 0., 0., 80., 20., 30.]);
    }
    app.run("layer.frame.set", json!({"layer":id.0,"transform":[100,0,0,80,40,40],"historyName":null})).unwrap();
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
    app.run("layer.frame.set", json!({"layer":id.0,"transform":[100,0,0,80,20,30],"historyName":"Move"})).unwrap();
    assert_eq!(entries(&app).last().map(String::as_str), Some("Move"));
}

/// A frame with an image is not what #13880 measured: its border drag keeps today's step name.
#[test]
fn dragging_the_border_of_a_frame_with_an_image_keeps_frame_properties() {
    let mut app = app();
    let id = empty_frame(&mut app);
    let content = app
        .session
        .edit("Image", |doc, _| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(20, 30, 120, 110), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            Ok(id)
        })
        .unwrap();
    app.run("layer.frame.setContent", json!({"layer":id.0,"content":content.0})).unwrap();
    select(&mut app, id, Target::Frame);
    drag(&mut app, [45., 30.], [65., 40.]);
    assert_eq!(transform(&app, id), [100., 0., 0., 80., 40., 40.]);
    assert_eq!(entries(&app).last().map(String::as_str), Some("Frame Properties"));
}
