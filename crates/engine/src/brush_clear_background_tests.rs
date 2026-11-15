//! #11897: the Brush's chosen Clear on the Background paints as Normal. The options bar turns
//! Clear off at the press there (`clear_off_on_background` in ui-egui), so the engine never sees
//! it; a `"mode": "clear"` stroke that does reach the Background is a held `~`, which keeps
//! erasing to the background colour. On a pixel layer Clear still erases, and the Eraser on the
//! Background still paints the background colour.

use crate::Session;
use serde_json::{Value, json};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    s.tools.foreground = BLACK;
    s.tools.background = GREEN;
    s
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 0.02)
}

/// A hard stroke across (10..30, 10), as the canvas sends it in Clear mode.
fn clear_stroke() -> Value {
    json!({"points": [[10, 10], [30, 10]], "size": 8, "hardness": 1.0, "erase": true, "mode": "clear"})
}

/// A held `~` on the Background erases to the background colour, as before #11897.
#[test]
fn a_clear_stroke_on_the_background_still_paints_the_background_colour() {
    let mut s = session();
    s.execute("paint.stroke", clear_stroke()).unwrap();
    let p = pixel(&mut s, 20, 10);
    assert!(close(p, GREEN), "painted {p:?}, not the foreground");
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Brush Tool"));
}

#[test]
fn a_normal_stroke_on_the_background_paints_the_foreground() {
    let mut s = session();
    s.execute("paint.stroke", json!({"points": [[10, 10], [30, 10]], "size": 8, "hardness": 1.0})).unwrap();
    let p = pixel(&mut s, 20, 10);
    assert!(close(p, BLACK), "{p:?}");
}

#[test]
fn clear_on_a_pixel_layer_still_erases() {
    let mut s = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    s.execute("paint.stroke", clear_stroke()).unwrap();
    let p = pixel(&mut s, 20, 10);
    assert!(close(p, WHITE), "erased down to the white Background: {p:?}");
}

#[test]
fn the_eraser_on_the_background_still_paints_the_background_colour() {
    let mut s = session();
    s.execute("paint.stroke", json!({"points": [[10, 10], [30, 10]], "size": 8, "hardness": 1.0, "erase": true})).unwrap();
    let p = pixel(&mut s, 20, 10);
    assert!(close(p, GREEN), "{p:?}");
}
