//! #13862: a frame made around a layer is named "<layer name> Frame"; an empty frame keeps
//! "Frame N", and an explicit `name` wins over both.
use super::*;
use photocraft_doc::Layer;
use photocraft_geom::Rect;

/// The issue's document: 600 x 400 px with a white Background, pixel layers Red (100 x 80 at
/// 50,50) and Blue, and an empty pixel layer on top.
fn session() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
    let (red, blue) = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut red = Layer::raster("Red", fmt);
            red.surface_mut().unwrap().fill_rect(Rect::new(50, 50, 150, 130), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let mut blue = Layer::raster("Blue", fmt);
            blue.surface_mut().unwrap().fill_rect(Rect::new(300, 200, 400, 280), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
            let (r, b) = (red.id, blue.id);
            doc.layers.push(red);
            doc.layers.push(blue);
            doc.layers.push(Layer::raster("Layer 1", fmt));
            Ok((r, b))
        })
        .unwrap();
    (s, red, blue)
}

fn frame(s: &mut Session, params: Value) -> (String, Vec<String>) {
    let r = s.execute("layer.new.frame", params).unwrap();
    let g = s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    (g.name.clone(), g.children().unwrap().iter().map(|l| l.name.clone()).collect())
}

#[test]
fn a_frame_around_a_layer_is_named_after_it() {
    let (mut s, red, blue) = session();
    let (name, content) = frame(&mut s, json!({"x": 40, "y": 40, "width": 120, "height": 100, "layer": red.0}));
    assert_eq!((name.as_str(), content), ("Red Frame", vec!["Red".to_string()]), "the content layer keeps its own name");
    let (name, _) = frame(&mut s, json!({"x": 290, "y": 190, "width": 120, "height": 100, "shape": "ellipse", "layer": blue.0}));
    assert_eq!(name, "Blue Frame");
}

#[test]
fn empty_frames_keep_the_frame_counter() {
    let (mut s, _, _) = session();
    let (first, content) = frame(&mut s, json!({"x": 10, "y": 10, "width": 30, "height": 30}));
    assert_eq!((first.as_str(), content.len()), ("Frame 1", 0));
    let (second, _) = frame(&mut s, json!({"x": 500, "y": 300, "width": 30, "height": 30}));
    assert_eq!(second, "Frame 2");
}

#[test]
fn an_explicit_name_wins_over_the_content_name() {
    let (mut s, red, _) = session();
    let (name, content) = frame(&mut s, json!({"x": 40, "y": 40, "width": 120, "height": 100, "layer": red.0, "name": "Hero"}));
    assert_eq!((name.as_str(), content), ("Hero", vec!["Red".to_string()]));
    let (name, _) = frame(&mut s, json!({"x": 10, "y": 10, "width": 30, "height": 30, "name": "Empty"}));
    assert_eq!(name, "Empty");
}

#[test]
fn undo_and_redo_keep_the_content_name() {
    let (mut s, red, _) = session();
    let (name, _) = frame(&mut s, json!({"x": 40, "y": 40, "width": 120, "height": 100, "layer": red.0}));
    assert_eq!(name, "Red Frame");
    assert!(s.undo());
    let names = |s: &Session| s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect::<Vec<_>>();
    assert!(!names(&s).contains(&"Red Frame".to_string()) && names(&s).contains(&"Red".to_string()), "undo takes the frame away and gives Red back");
    assert!(s.redo());
    assert!(names(&s).contains(&"Red Frame".to_string()));
}
