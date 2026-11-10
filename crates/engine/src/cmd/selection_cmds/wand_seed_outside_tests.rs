//! #11779: with Contiguous on, the Magic Wand's Subtract and Intersect grow the region from the
//! seed through the current selection only. A seed outside the selection subtracts nothing and
//! intersects to nothing.

use serde_json::json;

use crate::Session;

/// 800 × 600, white, with a grey ramp of ten 40 px blocks from x 100 (100, 105, …, 145).
fn ramp() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    for k in 0..10 {
        let v = 100 + 5 * k;
        s.execute("select.rect", json!({"x": 100 + 40 * k, "y": 350, "width": 40, "height": 100})).unwrap();
        s.execute("edit.fill", json!({"color": format!("#{v:02x}{v:02x}{v:02x}")})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn wand(s: &mut Session, x: i32, tolerance: f64, mode: &str) {
    s.execute("select.magicWand", json!({"x": x, "y": 400, "tolerance": tolerance, "mode": mode})).unwrap();
}

fn wand_global(s: &mut Session, x: i32, tolerance: f64, mode: &str) {
    s.execute("select.magicWand", json!({"x": x, "y": 400, "tolerance": tolerance, "mode": mode, "contiguous": false})).unwrap();
}

/// The columns of row 400 that are at least half selected: (first, end), or `None`. Every
/// column in between must be selected too, so a hole in the run fails the test.
fn selected(s: &Session) -> Option<(i32, i32)> {
    let doc = &s.active().unwrap().doc;
    let row = photocraft_geom::Rect::new(0, 400, 800, 401);
    let m = photocraft_algo::selection::mask_from_surface(doc.selection.as_ref(), row);
    let first = m.iter().position(|v| *v >= 0.5)?;
    let last = m.iter().rposition(|v| *v >= 0.5)?;
    assert!(m[first..=last].iter().all(|v| *v >= 0.5), "a hole in the selected run {first}..={last}");
    Some((first as i32, last as i32 + 1))
}

/// Greys 100 to 130 (x 100–380) selected, as tolerance 32 on the first block selects them, plus
/// the Anti-alias fringe: column 380 of grey 135 at about 251/255, as measured in #11776.
fn ramp_selected() -> Session {
    let mut s = ramp();
    wand(&mut s, 110, 32.0, "replace");
    assert_eq!(selected(&s), Some((100, 381)));
    s
}

#[test]
fn a_seed_outside_the_selection_subtracts_nothing() {
    let mut s = ramp_selected();
    // Grey 145 (x 460–500) is outside the selection, though its own region reaches into it.
    wand(&mut s, 480, 32.0, "subtract");
    assert_eq!(selected(&s), Some((100, 381)), "unchanged");
}

#[test]
fn a_seed_outside_the_selection_intersects_to_nothing() {
    let mut s = ramp_selected();
    wand(&mut s, 480, 32.0, "intersect");
    assert_eq!(selected(&s), None);
}

#[test]
fn a_seed_inside_the_selection_works_as_before() {
    // Grey 125 with tolerance 10 reaches greys 115–135; inside the selection that is x 220–380.
    // With the default Anti-alias fringe, as measured in #11779: Subtract leaves 100–219 (x 219
    // keeps about 6/255, under the 0.5 cut) and Intersect 219–381.
    let mut s = ramp_selected();
    wand(&mut s, 320, 10.0, "subtract");
    assert_eq!(selected(&s), Some((100, 219)));
    let mut s = ramp_selected();
    wand(&mut s, 320, 10.0, "intersect");
    assert_eq!(selected(&s), Some((219, 381)));
}

#[test]
fn contiguous_off_keeps_the_global_match() {
    // Without Contiguous the seed's colour matches everywhere, as before: greys 115–145 are
    // x 220–500, wherever the seed lies.
    let mut s = ramp_selected();
    wand_global(&mut s, 480, 32.0, "subtract");
    assert_eq!(selected(&s), Some((100, 219)));
    let mut s = ramp_selected();
    wand_global(&mut s, 480, 32.0, "intersect");
    assert_eq!(selected(&s), Some((219, 381)));
}

#[test]
fn without_a_selection_subtract_and_intersect_select_nothing() {
    for mode in ["subtract", "intersect"] {
        let mut s = ramp();
        wand(&mut s, 320, 10.0, mode);
        assert_eq!(selected(&s), None, "{mode}");
    }
}

#[test]
fn the_fill_passes_only_pixels_at_least_half_selected() {
    use photocraft_algo::selection::wand_region_within;
    use photocraft_geom::Rect;
    // One flat grey row of four pixels; only the coverage limit decides the region.
    let img = [[128u8, 128, 128, 255]; 4];
    let area = Rect::new(0, 0, 4, 1);
    let fill = |within: [f32; 4], seed: i32| wand_region_within(&img, area, (seed, 0), 0.0, true, false, Some(&within));
    assert!(fill([0.49, 1.0, 1.0, 1.0], 0).is_none(), "a seed just under half selected finds nothing");
    let r = fill([0.5, 1.0, 1.0, 1.0], 0).unwrap();
    assert_eq!((0..4).map(|x| r.at(x, 0) >= 0.5).collect::<Vec<_>>(), [true; 4], "half selected counts");
    let r = fill([1.0, 1.0, 0.49, 1.0], 0).unwrap();
    assert_eq!((0..4).map(|x| r.at(x, 0) >= 0.5).collect::<Vec<_>>(), [true, true, false, false], "the fill stops at 0.49");
}
