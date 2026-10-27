use super::*;
use crate::engine_data::parse;

fn frame(src: &str) -> E {
    parse(src.as_bytes()).unwrap()
}

/// The frame of `text/path-circle.psd` (Photoshop 2026).
const CIRCLE: &str = "<< /1 << /0 [ 32.0 128.0 32.0 74.98067 74.98067 32.0 128.0 32.0 128.0 32.0 181.01933 32.0 224.0 74.98067 224.0 128.0 224.0 128.0 224.0 181.01933 181.01933 224.0 128.0 224.0 128.0 224.0 74.98067 224.0 32.0 181.01933 32.0 128.0 ] >> /2 << /0 2 /2 [ 1.0 0.0 0.0 1.0 -32.0 -32.0 ] /6 [ -1.0 -1.0 ] >> >>";
/// The frame of `text/path-wave-open.psd` (Photoshop 2026).
const WAVE: &str = "<< /1 << /0 [ 16.0 100.0 46.0 40.0 120.0 40.0 160.0 80.0 160.0 80.0 200.0 120.0 274.0 100.0 304.0 60.0 ] >> /2 << /0 2 /2 [ 1.0 0.0 0.0 1.0 -16.0 -52.12246 ] /6 [ -1.0 -1.0 ] >> >>";

#[test]
fn photoshop_path_frames_read_into_text_space() {
    let wave = read(&frame(WAVE)).unwrap();
    assert!(!wave.closed);
    assert_eq!(wave.knots.len(), 3);
    let p = |x: f64, y: f64| Point::new(x - 16.0, y - 52.12246);
    let k = &wave.knots;
    assert!(same(k[0].anchor, p(16.0, 100.0)) && same(k[0].in_ctrl, p(16.0, 100.0)) && same(k[0].out_ctrl, p(46.0, 40.0)));
    assert!(same(k[1].in_ctrl, p(120.0, 40.0)) && same(k[1].anchor, p(160.0, 80.0)) && same(k[1].out_ctrl, p(200.0, 120.0)) && k[1].smooth);
    assert!(same(k[2].in_ctrl, p(274.0, 100.0)) && same(k[2].anchor, p(304.0, 60.0)));

    let circle = read(&frame(CIRCLE)).unwrap();
    assert!(circle.closed);
    assert_eq!(circle.knots.len(), 4);
    // The type starts at the bottom anchor (Photoshop's glyph 0) and runs to the left one.
    assert!(same(circle.knots[0].anchor, Point::new(96.0, 192.0)));
    assert!(same(circle.knots[1].anchor, Point::new(0.0, 96.0)));
    assert!(circle.knots.iter().all(|k| k.smooth));
}

#[test]
fn written_frames_read_back() {
    for src in [CIRCLE, WAVE] {
        let path = read(&frame(src)).unwrap();
        let mut entry = E::Dict(vec![("0".into(), frame(src))]);
        let before = entry.clone();
        update_frame(&mut entry, &TextShape::Path { path: path.clone(), start: 0.0, flip: false });
        assert_eq!(entry, before, "an unchanged path keeps Photoshop's frame");
        let mut moved = path.clone();
        for k in &mut moved.knots {
            *k = k.transform(&Affine::translate(5.0, -3.0));
        }
        update_frame(&mut entry, &TextShape::Path { path: moved.clone(), start: 0.0, flip: false });
        let back = read(entry.get("0").unwrap()).unwrap();
        assert!(same_path(&back, &moved), "{back:?}");
        assert_eq!(entry.path(&["0", "2", "6"]), before.path(&["0", "2", "6"]), "the frame's other keys stay");
    }
}

/// Flipped type and a moved start are written into the path, which lays the type out the same.
#[test]
fn flip_and_start_are_baked_into_the_frame() {
    use crate::warp::Warp;
    for src in [CIRCLE, WAVE] {
        let path = read(&frame(src)).unwrap();
        for (start, flip) in [(40.0, false), (0.0, true), (25.0, true)] {
            let shape = TextShape::Path { path: path.clone(), start, flip };
            let baked = TextShape::Path { path: frame_path(&path, start, flip), start: 0.0, flip: false };
            let (a, b) = (Warp::on_path(&shape).unwrap(), Warp::on_path(&baked).unwrap());
            for x in [0.0, 10.0, 50.0, 120.0] {
                for y in [-12.0, 0.0, 4.0] {
                    let (p, q) = (a.apply(x, y), b.apply(x, y));
                    assert!((p.0 - q.0).hypot(p.1 - q.1) < 0.5, "{src:.10} start {start} flip {flip} at ({x}, {y}): {p:?} vs {q:?}");
                }
            }
        }
    }
}

#[test]
fn hostile_frames_are_errors() {
    for src in [
        "<< /2 << /0 2 >> >>",
        "<< /1 << /0 [ ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 [ 1.0 2.0 3.0 ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 (x) 3 ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 1e999 3 ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 0 0 0 0 0 0 ] >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 1 0 0 ] >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 1e300 0 0 1e300 0 0 ] >> >>",
        "<< /1 << /0 [ 0 0 1 1 2 2 3 3 50 50 60 60 70 70 80 80 ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 [ 5 5 5 5 5 5 5 5 ] >> /2 << /0 2 >> >>",
        "<< /1 << /0 (path) >> /2 << /0 2 >> >>",
    ] {
        assert!(read(&frame(src)).is_err(), "{src}");
    }
    let many = format!("<< /1 << /0 [ {} ] >> /2 << /0 2 >> >>", "0 0 1 1 2 2 0 0 ".repeat(MAX_SEGMENTS + 1));
    assert!(read(&frame(&many)).is_err());
}

fn d(items: Vec<(&str, E)>) -> E {
    E::Dict(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A Photoshop-shaped `Txt2` block with one point-text object for `text` (`\r` breaks, trailing
/// `\r`) drawn in frame 0.
fn photoshop_txt2(text: &str) -> Vec<u8> {
    photoshop_txt2_in(text, point_frame())
}

/// [`photoshop_txt2`] with the frame list entry `frame`.
fn photoshop_txt2_in(text: &str, frame: E) -> Vec<u8> {
    photoshop_txt2_of(vec![photoshop_object(text, 0)], vec![frame])
}

/// A Photoshop-shaped `Txt2` block holding `objects` and the frame list `frames`.
fn photoshop_txt2_of(objects: Vec<E>, frames: Vec<E>) -> Vec<u8> {
    crate::engine_data::write_bare(&[
        ("98".into(), d(vec![("0", E::Int(14))])),
        ("0".into(), d(vec![("8", d(vec![("0", E::Array(frames))]))])),
        ("1".into(), d(vec![("1", E::Array(objects))])),
    ])
}

/// A Photoshop-shaped text object for `text` drawn in frame `at`.
fn photoshop_object(text: &str, at: i64) -> E {
    let len = text.encode_utf16().count() as i64;
    let run =
        || d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new())), ("6", d(vec![("0", E::Int(0)), ("1", E::Real(12.0))]))]))])), ("1", E::Int(len))]);
    let model = d(vec![("0", E::String(text.into())), ("5", d(vec![("0", E::Array(vec![run()]))])), ("6", d(vec![("0", E::Array(vec![run()]))]))]);
    let strike = d(vec![("99", E::Name("PC".into())), ("6", E::Array(vec![d(vec![("99", E::Name("F".into())), ("10", E::Int(at))])]))]);
    let view = d(vec![
        ("0", E::Array(vec![d(vec![("0", E::Int(at))])])),
        ("1", d(vec![("0", E::Array(vec![d(vec![("0", d(vec![("1", E::Int(1))])), ("1", E::Int(len))])]))])),
        ("2", E::Array(vec![strike])),
    ]);
    d(vec![("0", model), ("1", view)])
}

fn layer(text: &str, shape: TextShape) -> photocraft_doc::TextLayer {
    use photocraft_doc::text::{CharStyle, TextRun};
    let mut t = photocraft_doc::TextLayer {
        text: text.into(),
        runs: vec![TextRun { len: text.len(), style: CharStyle { size_pt: 12.0, ..Default::default() } }],
        shape,
        transform: Affine::translate(32.0, 20.0),
        ..Default::default()
    };
    t.sync_summary();
    t
}

/// A layer read back, with what [`crate::psd::apply_txt2_path`] said.
type Opened = (photocraft_doc::TextLayer, Result<bool, String>);

/// Saves `layers` (object `i` each) into a file whose `Txt2` is `prev`, then reads each back:
/// its `TySh` and what [`crate::psd::apply_txt2_path`] makes of the saved `Txt2`.
fn save_and_open(layers: &[&photocraft_doc::TextLayer], prev: &[u8]) -> (Vec<u8>, Vec<Opened>) {
    let objects: Vec<(i32, &photocraft_doc::TextLayer)> = layers.iter().enumerate().map(|(i, l)| (i as i32, *l)).collect();
    let txt2 = crate::psd::build_txt2(&objects, Some(prev));
    let parsed = crate::psd::parse_txt2(&txt2).unwrap();
    let back = objects
        .iter()
        .map(|(i, l)| {
            let tysh = crate::psd::set_text_index(&crate::psd::build_tysh(l, 72.0, None), *i).unwrap();
            let mut back = crate::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
            crate::psd::apply_txt2(&mut back, &tysh, &parsed);
            let applied = crate::psd::apply_txt2_path(&mut back, &tysh, &parsed);
            crate::psd::apply_txt2_frame(&mut back, &tysh, &parsed);
            (back, applied)
        })
        .collect();
    (txt2, back)
}

/// #7962: type on a path survives a PSD save and open, for the file's own text object and for
/// a layer the file had no object for; a moved start and flipped type come back laid out the
/// same, in the path; a second save changes nothing.
#[test]
fn path_text_round_trips_through_tysh_and_txt2() {
    let circle = read(&frame(CIRCLE)).unwrap();
    let wave = read(&frame(WAVE)).unwrap();
    let a = layer("On a circle", TextShape::Path { path: circle.clone(), start: 0.0, flip: false });
    let b = layer("Along a wave", TextShape::Path { path: wave.clone(), start: 30.0, flip: true });
    let prev = photoshop_txt2("On a circle\r");
    let (txt2, back) = save_and_open(&[&a, &b], &prev);
    for ((l, applied), (want, start, flip)) in back.iter().zip([(&circle, 0.0, false), (&wave, 30.0, true)]) {
        assert_eq!(applied, &Ok(true), "{}", l.text);
        let TextShape::Path { path, start: s, flip: f } = &l.shape else { panic!("{}: {:?}", l.text, l.shape) };
        assert_eq!((*s, *f), (0.0, false));
        assert!(same_path(path, &frame_path(want, start, flip)), "{}: {path:?}", l.text);
        assert_eq!(l.transform, Affine::translate(32.0, 20.0), "the frame's matrix is not applied twice");
    }
    let (again, _) = save_and_open(&[&back[0].0, &back[1].0], &txt2);
    assert_eq!(again, txt2, "a re-save of the opened file changes nothing");
    // Off its path, the layer opens as point text again.
    let detached = layer("On a circle", TextShape::Point);
    let (_, back) = save_and_open(&[&detached], &txt2);
    assert_eq!(back[0].1, Ok(false));
    assert_eq!(back[0].0.shape, TextShape::Point);
}

/// A malformed path frame leaves the layer as EngineData describes it, with the reason.
#[test]
fn a_malformed_path_frame_keeps_the_engine_data_shape() {
    let a = layer("Hostile", TextShape::Box { x: 0.0, y: 0.0, width: 50.0, height: 20.0 });
    let tysh = crate::psd::set_text_index(&crate::psd::build_tysh(&a, 72.0, None), 0).unwrap();
    for bad in ["<< /1 << /0 [ 1 2 3 ] >> /2 << /0 2 >> >>", "<< /1 << /0 [ 0 0 1 1 2 2 3 3 ] >> /2 << /0 2 /2 [ 0 0 0 0 0 0 ] >> >>"] {
        let entry = E::Dict(vec![("0".into(), frame(bad))]);
        let txt2 = crate::psd::parse_txt2(&photoshop_txt2_in("Hostile\r", entry)).unwrap();
        let mut l = crate::psd::text_layer_from_tysh(&tysh, 72.0).unwrap();
        let before = l.shape.clone();
        assert!(crate::psd::apply_txt2_path(&mut l, &tysh, &txt2).is_err(), "{bad}");
        assert_eq!(l.shape, before);
    }
}

/// A layer taken off its path doesn't keep its path frame (it would reopen on the path).
#[test]
fn a_layer_off_its_path_loses_the_path_frame() {
    let mut entry = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    update_frame(&mut entry, &TextShape::Point);
    assert_eq!(entry, point_frame());
    let mut entry = E::Dict(vec![("0".into(), frame(WAVE))]);
    update_frame(&mut entry, &TextShape::Box { x: 1.0, y: 2.0, width: 30.0, height: 40.0 });
    assert_eq!(entry.get("0").map(frame_kind), Some(BOX_FRAME));
    assert_eq!(
        entry.path(&["0", "1", "0"]).and_then(numbers).unwrap()[..8],
        [1.0, 2.0, 1.0, 2.0, 31.0, 2.0, 31.0, 2.0],
        "Photoshop's box outline: corners as straight segments from the top left"
    );
    // A point or box frame is left alone.
    let mut point = point_frame();
    update_frame(&mut point, &TextShape::Point);
    assert_eq!(point, point_frame());
}

/// A path frame the import could not read (the layer opened as EngineData's box or point
/// text) is saved back as it was, not replaced by a box or point frame.
#[test]
fn an_unreadable_path_frame_is_kept() {
    let two_pieces = "<< /1 << /0 [ 0 0 1 1 2 2 3 3 50 50 60 60 70 70 80 80 ] >> /2 << /0 2 >> >>";
    for shape in [TextShape::Point, TextShape::Box { x: 0.0, y: 0.0, width: 80.0, height: 80.0 }] {
        let mut entry = E::Dict(vec![("0".into(), frame(two_pieces))]);
        let before = entry.clone();
        update_frame(&mut entry, &shape);
        assert_eq!(entry, before, "{shape:?}");
    }
}

/// The frame number the object at `index` of the saved `txt2` names, and that frame.
fn saved_frame(txt2: &[u8], index: usize) -> (i64, E) {
    let t = crate::psd::parse_txt2(txt2).unwrap();
    let object = &t.path(&["1", "1"]).and_then(E::as_array).unwrap()[index];
    let at = object.path(&["1", "0"]).and_then(E::as_array).unwrap()[0].get("0").and_then(E::as_i64).unwrap();
    (at, t.path(&["0", "8", "0"]).and_then(E::as_array).unwrap()[at as usize].clone())
}

/// An edited layer whose object the save can't refit keeps drawing in that object's own frame,
/// not in the frame its slot number names, which is the path layer's here.
#[test]
fn an_object_rebuilt_from_a_template_keeps_its_own_frame() {
    let circle = read(&frame(CIRCLE)).unwrap();
    let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
    // Object 1's style run has a length that is no count, so it can't be refitted.
    let mut broken = photoshop_object("Old\r", 0);
    let run = d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new()))]))])), ("1", E::String("x".into()))]);
    broken.get_mut("0").unwrap().set("6", d(vec![("0", E::Array(vec![run]))]));
    let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1), broken], vec![point_frame(), path_frame.clone()]);
    let (txt2, back) = save_and_open(&[&on_path, &layer("New", TextShape::Point)], &prev);
    assert_eq!(saved_frame(&txt2, 1).0, 0, "the rebuilt object draws in its own frame");
    assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
    assert_eq!(back[0].1, Ok(true));
    assert!(matches!(back[0].0.shape, TextShape::Path { .. }), "{:?}", back[0].0.shape);
}

/// A new layer whose slot number is the frame another object is drawn in gets a frame of its
/// own: neither layer takes the other's shape, on this save or the next.
#[test]
fn a_new_object_gets_a_frame_no_other_object_is_drawn_in() {
    let circle = read(&frame(CIRCLE)).unwrap();
    let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
    let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1)], vec![point_frame(), path_frame.clone()]);
    let new = layer("New", TextShape::Point);
    let (txt2, back) = save_and_open(&[&on_path, &new], &prev);
    assert_eq!(saved_frame(&txt2, 1), (2, point_frame()), "a frame past the file's own");
    assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
    assert_eq!(back[0].1, Ok(true));
    assert_eq!((&back[1].1, &back[1].0.shape), (&Ok(false), &TextShape::Point));
    // The path layer taken off its path on the next save loses its path frame.
    let (again, _) = save_and_open(&[&layer("On a circle", TextShape::Point), &new], &txt2);
    assert_eq!(saved_frame(&again, 0), (1, point_frame()));
}

/// A rebuilt object whose old frame number is past the frame list doesn't name a frame the
/// block lacks (Photoshop refuses such files, #7698).
#[test]
fn a_rebuilt_object_never_names_a_missing_frame() {
    let mut broken = photoshop_object("Old\r", 70_000);
    let run = d(vec![("0", d(vec![("0", d(vec![("0", E::String(String::new()))]))])), ("1", E::String("x".into()))]);
    broken.get_mut("0").unwrap().set("6", d(vec![("0", E::Array(vec![run]))]));
    let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    let prev = photoshop_txt2_of(vec![photoshop_object("On a circle\r", 1), broken], vec![point_frame(), path_frame.clone()]);
    let on_path = layer("On a circle", TextShape::Path { path: read(&frame(CIRCLE)).unwrap(), start: 0.0, flip: false });
    let (txt2, _) = save_and_open(&[&on_path, &layer("New", TextShape::Point)], &prev);
    assert_eq!(saved_frame(&txt2, 1), (2, point_frame()));
    assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
}

/// An object the file keeps that names a frame past the frame list doesn't grow the list (to
/// 65536 point frames here).
#[test]
fn a_kept_object_naming_a_far_frame_adds_no_frames() {
    let prev = photoshop_txt2_of(vec![photoshop_object("Far\r", crate::psd::MAX_TEXT_INDEX.into())], vec![point_frame()]);
    let out = crate::psd::parse_txt2(&crate::psd::build_txt2(&[(0, &layer("Far", TextShape::Point))], Some(&prev))).unwrap();
    assert_eq!(out.path(&["0", "8", "0"]).and_then(E::as_array).map(<[E]>::len), Some(1));
}

/// A frame the file already has two objects drawn in is left as the file has it, not given
/// the shape of whichever layer comes last.
#[test]
fn a_frame_two_objects_name_is_kept() {
    let circle = read(&frame(CIRCLE)).unwrap();
    let on_path = layer("On a circle", TextShape::Path { path: circle, start: 0.0, flip: false });
    let path_frame = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    let objects = vec![photoshop_object("On a circle\r", 1), photoshop_object("Point\r", 1)];
    let prev = photoshop_txt2_of(objects, vec![point_frame(), path_frame.clone()]);
    let (txt2, _) = save_and_open(&[&on_path, &layer("Point", TextShape::Point)], &prev);
    assert_eq!(saved_frame(&txt2, 0), (1, path_frame));
    assert_eq!(saved_frame(&txt2, 1).0, 1);
}

/// Area type's frame becomes a text box over its path's bounds, as its EngineData describes it.
#[test]
fn an_area_layer_gets_a_box_frame() {
    let square = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[(10.0, 20.0), (50.0, 20.0), (50.0, 80.0), (10.0, 80.0)])]);
    let mut entry = E::Dict(vec![("0".into(), frame(CIRCLE))]);
    update_frame(&mut entry, &TextShape::Area { path: square });
    assert_eq!(entry, box_frame(10.0, 20.0, 40.0, 60.0));
}
