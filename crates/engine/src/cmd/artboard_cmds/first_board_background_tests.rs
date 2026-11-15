//! The first drawn artboard takes the Background in as "Layer 0" (#11387): `takeBackground` on
//! `layer.new.artboard`, at 8, 16 and 32 bit, with the cases that keep today's result.

use super::*;
use photocraft_geom::Rect;

/// 800 x 600 with the white Background and a pixel layer "Red" at 100,100 to 300,250.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "background": "white"})).unwrap();
    s.edit("Red", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect { x0: 100, y0: 100, x1: 300, y1: 250 }, &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    s
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn names(layers: &[Layer]) -> Vec<&str> {
    layers.iter().map(|l| l.name.as_str()).collect()
}

fn steps(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

fn draw(s: &mut Session, take: serde_json::Value) -> Result<Value> {
    s.execute("layer.new.artboard", json!({"rect": [400, 300, 200, 200], "takeBackground": take}))
}

#[test]
fn the_first_board_takes_the_background_in_as_layer_0() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before = steps(&s).len();
        let r = draw(&mut s, json!(true)).unwrap();
        let board = LayerId(r["layer"].as_u64().unwrap());
        let d = doc(&s);
        assert_eq!(names(&d.layers), ["Red", "Artboard 1"], "{depth} bit");
        assert!(d.background().is_none(), "{depth} bit");
        let b = d.layer(board).unwrap();
        assert_eq!(r["rect"], json!([400, 300, 200, 200]));
        let kids = b.children().unwrap();
        assert_eq!(names(kids), ["Layer 0"], "{depth} bit");
        let l0 = kids.first().unwrap();
        assert!(!l0.locks.transparency && !l0.locks.position, "{depth} bit");
        // The Background's own pixels travel with it: white at both samples, and Red still red.
        assert_eq!(l0.surface().unwrap().rgba(150, 150), WHITE, "{depth} bit");
        assert_eq!(l0.surface().unwrap().rgba(700, 500), WHITE, "{depth} bit");
        let red = d.layers.first().unwrap();
        assert_eq!(red.surface().unwrap().rgba(150, 150), [1.0, 0.0, 0.0, 1.0], "{depth} bit");
        assert_eq!((d.size.width, d.size.height), (800, 600), "{depth} bit");
        assert_eq!(s.active().unwrap().active_layer, Some(board));
        let after = steps(&s);
        assert_eq!(after.len(), before + 1, "one history step");
        assert_eq!(after.last().map(String::as_str), Some("New Artboard"));
        // One undo brings the Background back at the bottom with its name and locks.
        s.undo();
        let d = doc(&s);
        assert_eq!(names(&d.layers), ["Background", "Red"], "{depth} bit");
        let bg = d.background().unwrap();
        assert!(bg.locks.transparency && bg.locks.position, "{depth} bit");
        assert_eq!(bg.surface().unwrap().rgba(150, 150), WHITE, "{depth} bit");
        assert_eq!(bg.surface().unwrap().rgba(700, 500), WHITE, "{depth} bit");
        assert!(d.artboards().is_empty());
        assert_eq!((d.size.width, d.size.height), (800, 600));
    }
}

#[test]
fn without_the_param_or_with_false_the_background_stays_outside() {
    for p in [json!({"rect": [400, 300, 200, 200]}), json!({"rect": [400, 300, 200, 200], "takeBackground": false})] {
        let mut s = session(8);
        s.execute("layer.new.artboard", p).unwrap();
        assert_eq!(names(&doc(&s).layers), ["Background", "Red", "Artboard 1"]);
        assert!(doc(&s).background().is_some());
    }
}

#[test]
fn with_a_board_already_present_the_result_is_as_today() {
    let mut s = session(8);
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap();
    draw(&mut s, json!(true)).unwrap();
    let d = doc(&s);
    assert_eq!(names(&d.layers), ["Background", "Red", "Artboard 1", "Artboard 2"]);
    assert!(d.background().is_some());
    assert!(d.artboards().iter().all(|b| b.1 != "Layer 0"));
}

#[test]
fn without_a_background_the_result_is_as_today() {
    let mut s = session(8);
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert!(doc(&s).background().is_none());
    draw(&mut s, json!(true)).unwrap();
    assert_eq!(names(&doc(&s).layers), ["Layer 0", "Red", "Artboard 1"]);
    assert!(doc(&s).layers.last().unwrap().children().unwrap().is_empty());
}

#[test]
fn a_non_boolean_take_background_is_an_error_and_changes_nothing() {
    for bad in [json!("true"), json!(1), json!([true])] {
        let mut s = session(8);
        let (n, count) = (steps(&s).len(), doc(&s).layers.len());
        assert!(draw(&mut s, bad.clone()).is_err(), "{bad}");
        assert_eq!(steps(&s).len(), n, "{bad}");
        assert_eq!(doc(&s).layers.len(), count, "{bad}");
        assert!(doc(&s).background().is_some(), "{bad}");
    }
}

#[test]
fn the_param_is_listed_in_the_command_params_doc() {
    let spec = specs().into_iter().find(|c| c.id == "layer.new.artboard").unwrap();
    assert!(spec.params.contains("takeBackground"));
}
