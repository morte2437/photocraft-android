//! #11966: Pencil Auto Erase paints the background colour only when the start pixel is exactly
//! the foreground colour (its alpha aside); one level off paints the foreground. The live preview
//! agrees, and at 16 and 32 bits a foreground equal to the pixel still counts.

use serde_json::{Value, json};

use crate::Session;
use crate::brush_cmds::LiveStroke;

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A document of `depth` bits with a pixel layer filled with `fill`, the background colour white.
fn session(depth: u32, fill: [f32; 4]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": fill})).unwrap();
    s.tools.background = WHITE;
    s
}

fn pencil(fg: &str) -> Value {
    json!({"points": [[12, 12]], "size": 3, "color": fg, "autoErase": true})
}

fn active_pixel(s: &Session) -> [f32; 4] {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(12, 12)
}

fn preview_pixel(s: &Session, p: &Value) -> [f32; 4] {
    let preview = LiveStroke::begin_with(s, "paint.pencil", p).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    preview.doc.layer(id).unwrap().surface().unwrap().rgba(12, 12)
}

fn level(v: f32) -> u8 {
    (v * 255.0).round() as u8
}

const RED: [f32; 4] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];

#[test]
fn one_level_off_paints_the_foreground() {
    let mut s = session(8, RED);
    let p = pencil("#dd1e1e"); // 221, 30, 30
    let preview = preview_pixel(&s, &p);
    s.execute("paint.pencil", p).unwrap();
    let px = active_pixel(&s);
    assert_eq!([level(px[0]), level(px[1]), level(px[2]), level(px[3])], [221, 30, 30, 255]);
    assert_eq!(preview, px, "the live preview agrees");
}

#[test]
fn the_exact_foreground_paints_the_background_colour() {
    let mut s = session(8, RED);
    let p = pencil("#dc1e1e"); // 220, 30, 30
    assert_eq!(preview_pixel(&s, &p), WHITE);
    s.execute("paint.pencil", p).unwrap();
    assert_eq!(active_pixel(&s), WHITE);
}

#[test]
fn alpha_is_ignored_for_the_match() {
    let mut s = session(8, [RED[0], RED[1], RED[2], 0.5]);
    s.execute("paint.pencil", pencil("#dc1e1e")).unwrap();
    assert_eq!(active_pixel(&s), WHITE, "opaque background colour");
}

#[test]
fn deeper_documents_still_match_an_equal_foreground() {
    for depth in [16, 32] {
        let mut s = session(depth, RED);
        s.execute("paint.pencil", pencil("#dc1e1e")).unwrap();
        assert_eq!(active_pixel(&s), WHITE, "{depth} bits");
        let mut s = session(depth, RED);
        s.execute("paint.pencil", pencil("#dd1e1e")).unwrap();
        assert_ne!(active_pixel(&s), WHITE, "{depth} bits: one level off paints the foreground");
    }
}
