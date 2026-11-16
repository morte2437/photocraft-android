//! Object Selection › Hard Edge (#12855): `select.object`'s `"hardEdge"`.

use super::*;

const W: i32 = 200;
const H: i32 = 160;

/// The object: a red rectangle on blue.
fn inside(x: i32, y: i32) -> bool {
    (60..140).contains(&x) && (50..110).contains(&y)
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        let mut data = Vec::with_capacity((W * H) as usize * n);
        for y in 0..H {
            for x in 0..W {
                let c: [f32; 3] = if inside(x, y) { [0.86, 0.12, 0.12] } else { [0.12, 0.12, 0.86] };
                data.extend_from_slice(&c[..n.min(3)]);
                if n == 4 {
                    data.push(1.0);
                }
            }
        }
        bg.write_region(Rect::new(0, 0, W, H), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

/// The selection as 8-bit coverage, row by row.
fn sel(s: &Session) -> Vec<u8> {
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| (cov(s, x, y) * 255.0).round() as u8).collect()
}

fn bounds(s: &Session) -> Rect {
    s.active().unwrap().doc.selection.as_ref().map_or(Rect::new(0, 0, 0, 0), |m| m.content_bounds())
}

fn partial(m: &[u8]) -> usize {
    m.iter().filter(|v| (1..255).contains(*v)).count()
}

/// The selection `params` gives on a fresh document.
fn select(depth: u32, params: Value) -> (Session, Value) {
    let mut s = session(depth);
    let r = s.execute("select.object", params).unwrap();
    (s, r)
}

#[test]
fn hard_edge_is_on_by_default_and_leaves_the_result_alone() {
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    let (on, r) = select(8, json!({"point": [100, 80], "hardEdge": true}));
    assert_eq!(r["changed"], true);
    assert_eq!(sel(&plain), sel(&on));
    assert_eq!(bounds(&plain), bounds(&on));
    let (plain, _) = select(8, json!({"rect": [50, 40, 100, 80]}));
    let (on, _) = select(8, json!({"rect": [50, 40, 100, 80], "hardEdge": true}));
    assert_eq!(sel(&plain), sel(&on));
}

/// The rim of the object found in [`session`], across the middle row and down the middle
/// column: `[outside, the outer pixel, inside]` on the left (top), then `[the last pixel inside,
/// the two pixels after it, outside]` on the right (bottom).
fn rim(m: &[u8]) -> ([[u8; 3]; 2], [[u8; 4]; 2]) {
    let at = |x: i32, y: i32| m[(y * W + x) as usize];
    // The object is x 60..140, y 50..110.
    (
        [[at(58, 80), at(59, 80), at(60, 80)], [at(100, 48), at(100, 49), at(100, 50)]],
        [[at(139, 80), at(140, 80), at(141, 80), at(142, 80)], [at(100, 109), at(100, 110), at(100, 111), at(100, 112)]],
    )
}

#[test]
fn hard_edge_off_softens_both_sides_by_a_pixel_at_each_depth() {
    for depth in [8, 16, 32] {
        let (hard, _) = select(depth, json!({"point": [100, 80]}));
        let (soft, r) = select(depth, json!({"point": [100, 80], "hardEdge": false}));
        assert_eq!(r["changed"], true, "depth {depth}");
        let (hb, sb) = (bounds(&hard), bounds(&soft));
        assert_eq!(hb, Rect::new(59, 49, 141, 111), "depth {depth}: Hard Edge on, as before");
        // The left and top stay; the line of 1s takes the right and bottom one pixel further out.
        assert_eq!(sb, Rect::new(hb.x0, hb.y0, hb.x1 + 1, hb.y1 + 1), "depth {depth}");
        assert_eq!(r["bounds"], json!([sb.x0, sb.y0, sb.width(), sb.height()]), "depth {depth}");
        assert_eq!(rim(&sel(&hard)), ([[0, 43, 213], [0, 43, 213]], [[213, 43, 0, 0], [213, 43, 0, 0]]), "depth {depth}: Hard Edge on");
        // Off: the outer pixel on the left (top) softens to about 176 (197), the old rim pixel on
        // the right (bottom) fills, the one after it gets about 65 and the next one 1, as the
        // reference measures.
        assert_eq!(rim(&sel(&soft)), ([[0, 176, 255], [0, 197, 255]], [[255, 65, 1, 0], [255, 65, 1, 0]]), "depth {depth}: Hard Edge off");
    }
}

#[test]
fn hard_edge_off_softens_a_dragged_rectangle_too() {
    let (hard, _) = select(8, json!({"rect": [50, 40, 100, 80]}));
    let (soft, r) = select(8, json!({"rect": [50, 40, 100, 80], "hardEdge": false}));
    assert_eq!(r["changed"], true);
    assert_ne!(sel(&soft), sel(&hard));
    assert!(cov(&soft, 100, 80) >= 0.99);
}

#[test]
fn hard_edge_on_passes_the_found_object_through_without_a_copy() {
    // The Object Finder's cached object can be the size of the canvas: the default click must not
    // copy its mask.
    let r = Region { bbox: Rect::new(2, 2, 6, 6), mask: vec![255; 16] };
    let canvas = Rect::new(0, 0, 10, 10);
    for p in [json!({}), json!({"hardEdge": true}), json!({"hardEdge": "false"})] {
        assert!(matches!(object_edge(canvas, &p, Some(&r)), Some(Cow::Borrowed(found)) if std::ptr::eq(found, &r)), "{p}");
    }
    assert!(matches!(object_edge(canvas, &json!({"hardEdge": false}), Some(&r)), Some(Cow::Owned(_))));
    assert!(object_edge(canvas, &json!({}), None).is_none());
}

#[test]
fn a_wrong_type_hard_edge_is_the_default() {
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    for v in [json!("false"), json!(0), json!(null), json!([false]), json!({"on": false})] {
        let (s, r) = select(8, json!({"point": [100, 80], "hardEdge": v}));
        assert_eq!(r["changed"], true, "hardEdge {v}");
        assert_eq!(sel(&s), sel(&plain), "hardEdge {v}");
    }
}

#[test]
fn the_object_finder_cache_keeps_the_object_as_found() {
    // A soft click must not leave a softened object behind for the next hard click.
    let (plain, _) = select(8, json!({"point": [100, 80]}));
    let (mut s, _) = select(8, json!({"point": [100, 80], "hardEdge": false}));
    let soft = sel(&s);
    s.execute("select.object", json!({"point": [90, 70]})).unwrap();
    assert_eq!(sel(&s), sel(&plain));
    s.execute("select.object", json!({"point": [90, 70], "hardEdge": false})).unwrap();
    assert_eq!(sel(&s), soft);
    // One history step each.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(sel(&s), sel(&plain));
}

#[test]
fn hard_edge_off_finding_nothing_changes_nothing() {
    let (s, r) = select(8, json!({"point": [3, 3], "hardEdge": false}));
    assert_eq!(r["changed"], false);
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn soften_edge_stays_full_at_the_canvas_border() {
    let canvas = Rect::new(0, 0, 10, 10);
    // A hard 4x4 block in the top-left corner: its left and top sides are the document's, so the
    // pixels past the canvas repeat it and those sides stay full.
    let r = soften_edge(&Region { bbox: Rect::new(0, 0, 4, 4), mask: vec![255; 16] }, canvas);
    assert_eq!(r.bbox, Rect::new(0, 0, 6, 6));
    assert_eq!(r.mask.len(), 36);
    let at = |x: i32, y: i32| (r.at(x, y) * 255.0).round() as u8;
    assert_eq!([at(0, 0), at(3, 0), at(4, 0), at(5, 0)], [255, 255, 23, 10], "the top row, along the border");
    assert_eq!([at(0, 0), at(0, 3), at(0, 4), at(0, 5)], [255, 251, 32, 8], "the left column, along the border");
    // Away from the border the box grows a pixel on the left and top; on the right and bottom the
    // filter reaches two pixels, of which a 2x2 block's ramp keeps one.
    let r = soften_edge(&Region { bbox: Rect::new(4, 4, 6, 6), mask: vec![255; 4] }, canvas);
    assert_eq!(r.bbox, Rect::new(3, 3, 7, 7));
    // A block at the canvas' right and bottom edges doesn't grow past them and stays full there.
    let r = soften_edge(&Region { bbox: Rect::new(7, 7, 10, 10), mask: vec![255; 9] }, canvas);
    assert_eq!(r.bbox, Rect::new(6, 6, 10, 10));
    assert_eq!(r.at(9, 9), 1.0);
}

#[test]
fn soften_edge_survives_odd_regions() {
    let canvas = Rect::new(0, 0, 10, 10);
    // Empty, outside the canvas, and a mask shorter than its box: no panic, nothing invented.
    let empty = Region { bbox: Rect::new(3, 3, 3, 3), mask: Vec::new() };
    assert_eq!(soften_edge(&empty, canvas), empty);
    let outside = Region { bbox: Rect::new(20, 20, 24, 24), mask: vec![255; 16] };
    assert_eq!(soften_edge(&outside, canvas), outside);
    let short = soften_edge(&Region { bbox: Rect::new(2, 2, 6, 6), mask: vec![255; 3] }, canvas);
    assert_eq!(short.mask.len(), (short.bbox.width() * short.bbox.height()) as usize);
    assert_eq!(short.bbox, Rect::new(1, 1, 5, 3), "trimmed to the pixels the three samples reach");
    assert_eq!(short.at(8, 8), 0.0);
    let far = Region { bbox: Rect::new(i32::MAX - 2, i32::MAX - 2, i32::MAX, i32::MAX), mask: vec![255; 4] };
    assert_eq!(soften_edge(&far, canvas), far);
}

#[test]
fn the_params_string_documents_hard_edge() {
    let specs = specs();
    let spec = specs.iter().find(|c| c.id == "select.object").unwrap();
    assert!(spec.params.contains(r#""hardEdge":bool=true"#), "{}", spec.params);
}

/// The reference application's test document (#12855): 800x600 RGB/8, white Background, and a
/// transparent layer with a red (220,30,30) rectangle 100,100-300,250 and a blue (30,30,220)
/// anti-aliased disc, centre 500,175, radius 75 (8x8 samples a pixel).
fn reference_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let mut data = Vec::with_capacity(800 * 600 * 4);
        for y in 0..600 {
            for x in 0..800 {
                if (100..300).contains(&x) && (100..250).contains(&y) {
                    data.extend_from_slice(&[220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]);
                    continue;
                }
                let inside = (0..64)
                    .filter(|i| {
                        let (dx, dy) = (x as f32 + (i % 8) as f32 / 8.0 + 1.0 / 16.0 - 500.0, y as f32 + (i / 8) as f32 / 8.0 + 1.0 / 16.0 - 175.0);
                        dx * dx + dy * dy <= 75.0 * 75.0
                    })
                    .count();
                let a = inside as f32 / 64.0;
                data.extend_from_slice(&if a > 0.0 { [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, a] } else { [0.0; 4] });
            }
        }
        surf.write_region(Rect::new(0, 0, 800, 600), &data);
        Ok(())
    })
    .unwrap();
    s
}

/// The 8-bit selection after a click at `point` on the reference document: its
/// `[left, top, right, bottom]` and its values, row by row.
fn measure(point: [i32; 2], hard_edge: bool) -> ([i32; 4], Vec<u8>) {
    let mut s = reference_document();
    s.execute("select.object", json!({"point": point, "hardEdge": hard_edge})).unwrap();
    let m = s.active().unwrap().doc.selection.as_ref().unwrap();
    let b = m.content_bounds();
    let v = (0..600).flat_map(|y| (0..800).map(move |x| (x, y))).map(|(x, y)| (m.sample_channel(x, y, 0) * 255.0).round() as u8).collect();
    ([b.x0, b.y0, b.x1, b.y1], v)
}

/// Fully selected, partly selected, and partly selected below 128, as the issue counts them.
fn counts(v: &[u8]) -> (usize, usize, usize) {
    (v.iter().filter(|v| **v == 255).count(), partial(v), v.iter().filter(|v| (1..128).contains(*v)).count())
}

/// The commonest value on column `x` of the reference document (`row`: on row `x`) and how many
/// of its pixels hold it, empty pixels left out; `None` when the line is empty.
fn commonest(v: &[u8], x: i32, row: bool) -> Option<(u8, usize)> {
    let mut seen = [0usize; 256];
    for i in 0..if row { 800 } else { 600 } {
        let (px, py) = if row { (i, x) } else { (x, i) };
        seen[usize::from(v[(py * 800 + px) as usize])] += 1;
    }
    (1..=255u8).map(|value| (value, seen[usize::from(value)])).filter(|(_, n)| *n > 0).max_by_key(|(_, n)| *n)
}

/// The reference application's red rectangle with Hard Edge off, measured column by column and
/// row by row (#12855; the rectangle is drawn at x 100-299, y 100-249). It softens both sides by
/// a pixel: x = 99 at 175-177 (134 of its 151 pixels) and y = 99 at 196-200 (177), x = 299 and
/// y = 249 fully selected (145, 195), then x = 300 and y = 250 at about 65 (135, 195), and a line
/// of value 1 at x = 301 and y = 251 (150, 200), which sets the bounds 99,99,302,252. Its counts
/// are 29988 fully and 1066 partly selected, 710 of them below 128. Each line holds its value
/// along its whole straight edge; the few other pixels are at the corners.
#[test]
fn hard_edge_off_matches_the_reference_rectangle() {
    let (b, v) = measure([200, 175], false);
    assert_eq!(b, [99, 99, 302, 252], "the reference's bounds");
    let at = |x: i32, y: i32| v[(y * 800 + x) as usize];
    assert_eq!((97..=101).map(|x| at(x, 175)).collect::<Vec<_>>(), [0, 0, 176, 255, 255]);
    assert_eq!((297..=303).map(|x| at(x, 175)).collect::<Vec<_>>(), [255, 255, 255, 65, 1, 0, 0]);
    assert_eq!((97..=101).map(|y| at(200, y)).collect::<Vec<_>>(), [0, 0, 197, 255, 255]);
    assert_eq!((247..=253).map(|y| at(200, y)).collect::<Vec<_>>(), [255, 255, 255, 65, 1, 0, 0]);
    assert_eq!(commonest(&v, 99, false), Some((176, 145)), "x = 99");
    assert_eq!(commonest(&v, 299, false), Some((255, 146)), "x = 299");
    assert_eq!(commonest(&v, 300, false), Some((65, 145)), "x = 300");
    assert_eq!(commonest(&v, 301, false), Some((1, 145)), "x = 301: the line of 1s");
    assert_eq!(commonest(&v, 99, true), Some((197, 195)), "y = 99");
    assert_eq!(commonest(&v, 249, true), Some((255, 195)), "y = 249");
    assert_eq!(commonest(&v, 250, true), Some((65, 195)), "y = 250");
    assert_eq!(commonest(&v, 251, true), Some((1, 195)), "y = 251: the line of 1s");
    assert_eq!((commonest(&v, 98, false), commonest(&v, 98, true)), (None, None), "nothing further out on the left and top");
    assert_eq!((commonest(&v, 302, false), commonest(&v, 252, true)), (None, None), "nor past the line of 1s");
    assert_eq!(counts(&v), (29992, 1049, 694), "the reference: 29988, 1066, 710");
}

/// The disc: the reference gives 423,99,578,252 with Hard Edge on and off, and mostly changes
/// the low values (1057 partly selected below 128 on, 978 off). Ours with Hard Edge on is
/// 424,99,576,251, narrower than the reference's; off, the line of 1s takes the right and bottom
/// a pixel further, to within a pixel of the reference's bounds on every side. Off has fewer low
/// values, as in the reference, but our partly selected count drops (1200 to 724) where the
/// reference's stays at about 1570.
#[test]
fn hard_edge_off_on_the_reference_disc() {
    let (on_bounds, on) = measure([500, 175], true);
    let (off_bounds, off) = measure([500, 175], false);
    assert_eq!(on_bounds, [424, 99, 576, 251]);
    assert_eq!((off_bounds, counts(&off)), ([424, 99, 577, 252], (17419, 724, 432)));
    assert!(counts(&off).2 < counts(&on).2, "fewer low values with Hard Edge off");
}

/// Hard Edge on is left as it is (#12855): the tool is driven by a segmentation model, so the
/// reference's own figures (rectangle 99,99,300,250 · 29651 · 700; disc 423,99,578,252 · 17305 ·
/// 1569) are not a target, only the difference Hard Edge makes.
#[test]
fn hard_edge_on_is_unchanged_on_the_reference_document() {
    let (b, v) = measure([200, 175], true);
    assert_eq!((b, counts(&v)), ([99, 99, 301, 251], (29304, 1400, 704)));
    let (b, v) = measure([500, 175], true);
    assert_eq!((b, counts(&v)), ([424, 99, 576, 251], (16888, 1200, 604)));
}
