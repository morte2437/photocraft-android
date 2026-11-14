//! On a Grayscale document's Background, the Eraser and Edit › Fill with the background colour
//! paint that colour's grey as the reference converts it: RGB 0, 200, 0 gives 173, not its 117
//! luminance (#11940). The swatch goes through the sRGB tone curve (sGray), not the document's Dot
//! Gain curve, which would give 153.

use crate::Session;
use serde_json::json;

const GREEN_200: [f32; 4] = [0.0, 200.0 / 255.0, 0.0, 1.0];

fn session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "mode": mode, "depth": 8, "background": "white"})).unwrap();
    s.tools.background = GREEN_200;
    s
}

fn pixel(s: &mut Session, x: i32, y: i32) -> [f32; 4] {
    let v = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    let a: Vec<f32> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as f32).collect();
    [a[0], a[1], a[2], a[3]]
}

fn level(c: f32) -> i32 {
    (c * 255.0).round() as i32
}

fn erase_at(s: &mut Session, x: i32, y: i32) {
    s.execute("paint.stroke", json!({"points": [[x, y]], "size": 9, "hardness": 1.0, "erase": true})).unwrap();
}

#[test]
fn the_eraser_on_a_gray_background_paints_the_background_colours_gray() {
    let mut s = session("grayscale");
    erase_at(&mut s, 20, 16);
    let p = pixel(&mut s, 20, 16);
    assert!((level(p[0]) - 173).abs() <= 2, "erased pixel {p:?}");
    assert_eq!(level(p[0]), level(p[1]));
    assert_eq!(level(p[0]), level(p[2]));
    assert_eq!(level(pixel(&mut s, 50, 16)[0]), 255, "pixels away from the click stay white");
}

#[test]
fn edit_fill_with_the_background_colour_gives_the_same_gray() {
    let mut s = session("grayscale");
    s.execute("edit.fill", json!({"contents": "background"})).unwrap();
    let filled = pixel(&mut s, 5, 5);
    assert!((level(filled[0]) - 173).abs() <= 2, "filled pixel {filled:?}");

    let mut e = session("grayscale");
    erase_at(&mut e, 20, 16);
    assert_eq!(level(pixel(&mut e, 20, 16)[0]), level(filled[0]), "the Eraser and Edit › Fill agree");
}

#[test]
fn rgb_documents_keep_the_background_colour_as_it_is() {
    let mut s = session("rgb");
    erase_at(&mut s, 20, 16);
    let p = pixel(&mut s, 20, 16);
    assert_eq!([level(p[0]), level(p[1]), level(p[2])], [0, 200, 0], "erased pixel {p:?}");

    let mut f = session("rgb");
    f.execute("edit.fill", json!({"contents": "background"})).unwrap();
    let q = pixel(&mut f, 5, 5);
    assert_eq!([level(q[0]), level(q[1]), level(q[2])], [0, 200, 0], "filled pixel {q:?}");
}

fn pencil_auto_erase_params() -> serde_json::Value {
    json!({"points": [[20, 16]], "size": 5, "autoErase": true, "seed": 7})
}

#[test]
fn pencil_auto_erase_keeps_the_raw_background_live_and_committed() {
    // #11940 lists the Eraser's and Edit › Fill's background lines; Pencil Auto Erase paints the
    // foreground-colour pixel with the plain background colour as before (the luma grey, about
    // 117 for RGB 0, 200, 0), in the committed stroke and in the live one the canvas shows.
    let mut reference = session("grayscale");
    reference.tools.foreground = [1.0, 1.0, 1.0, 1.0];
    reference.execute("paint.pencil", pencil_auto_erase_params()).unwrap();
    let expected = pixel(&mut reference, 20, 16);
    assert!((level(expected[0]) - 117).abs() <= 2, "committed pixel {expected:?}");

    let mut s = session("grayscale");
    s.tools.foreground = [1.0, 1.0, 1.0, 1.0];
    let mut p = pencil_auto_erase_params();
    let live = crate::brush_cmds::LiveStroke::begin_with(&s, "paint.pencil", &p).unwrap();
    p["seed"] = json!(live.seed);
    s.offer_live_stroke(live).unwrap();
    let result = s.execute("paint.pencil", p).unwrap();
    assert_eq!(result["fromLive"], true, "the commit took the live pixels");
    let got = pixel(&mut s, 20, 16);
    assert_eq!(level(got[0]), level(expected[0]), "live pixel {got:?}, committed {expected:?}");
}

fn assert_erase_and_fill_give_luma(mut s: Session, what: &str) {
    erase_at(&mut s, 20, 16);
    let erased = pixel(&mut s, 20, 16);
    assert!((level(erased[0]) - 117).abs() <= 1, "{what}: erased pixel {erased:?}");
    s.execute("edit.fill", json!({"contents": "background"})).unwrap();
    let filled = pixel(&mut s, 5, 5);
    assert!((level(filled[0]) - 117).abs() <= 1, "{what}: filled pixel {filled:?}");
}

#[test]
fn other_gray_working_spaces_keep_the_plain_luma() {
    // #11940 measured the default Working Gray only; the others keep today's behaviour.
    for space in ["sgray", "gray-gamma-2.2"] {
        let mut s = session("grayscale");
        s.execute("edit.colorSettings", json!({"workingGray": space})).unwrap();
        assert_erase_and_fill_give_luma(s, &format!("workingGray {space}"));
    }
}

#[test]
fn a_gray_document_tagged_with_another_profile_keeps_the_plain_luma() {
    for profile in ["sgray", "gray-gamma-2.2"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 32, "mode": "grayscale", "depth": 8, "background": "white", "profile": profile})).unwrap();
        s.tools.background = GREEN_200;
        assert_erase_and_fill_give_luma(s, &format!("document profile {profile}"));
    }
}

#[test]
fn a_gray_document_tagged_with_the_default_profile_still_gets_the_measured_gray() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "mode": "grayscale", "depth": 8, "background": "white", "profile": "dot-gain-20-compat"})).unwrap();
    s.tools.background = GREEN_200;
    erase_at(&mut s, 20, 16);
    let p = pixel(&mut s, 20, 16);
    assert!((level(p[0]) - 173).abs() <= 2, "erased pixel {p:?}");
}

#[test]
fn the_live_eraser_on_a_gray_background_matches_the_committed_stroke() {
    let mut direct = session("grayscale");
    erase_at(&mut direct, 20, 16);
    let expected = pixel(&mut direct, 20, 16);
    assert!((level(expected[0]) - 173).abs() <= 2, "committed pixel {expected:?}");

    let mut s = session("grayscale");
    let mut p = json!({"points": [[20, 16]], "size": 9, "hardness": 1.0, "erase": true, "seed": 7});
    let live = crate::brush_cmds::LiveStroke::begin_with(&s, "paint.stroke", &p).unwrap();
    p["seed"] = json!(live.seed);
    s.offer_live_stroke(live).unwrap();
    let result = s.execute("paint.stroke", p).unwrap();
    assert_eq!(result["fromLive"], true, "the commit took the live pixels");
    let got = pixel(&mut s, 20, 16);
    assert!((level(got[0]) - 173).abs() <= 2, "live pixel {got:?}");
    assert_eq!(level(got[0]), level(expected[0]), "live pixel {got:?}, committed {expected:?}");
}
