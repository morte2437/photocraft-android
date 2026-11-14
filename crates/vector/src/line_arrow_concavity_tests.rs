//! Arrowhead Concavity is -50..50 % of the head's length, as in Photoshop (#13258): the middle of
//! the head's rear edge bends toward the tip for a positive value and away from it for a negative
//! one, and the shaft ends where that bent edge crosses the shaft's edge. Joining the shaft and
//! the heads into one outline is #13260; these tests check the anchors the reference outline has
//! and that the subpaths (each filled and combined, as the rasterizer does) cover what it covers.

use super::*;

/// The measured setup: weight 4, head width 5 and length 10, drag 400,300 to 600,300.
fn arrow(start: bool, end: bool, concavity: f64) -> Path {
    arrow_line((400.0, 300.0), (600.0, 300.0), 4.0, start, end, 5.0, 10.0, concavity)
}

fn anchors(sp: &Subpath) -> Vec<[f64; 2]> {
    sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

fn same(sp: &Subpath, want: &[[f64; 2]], what: &str) {
    let got = anchors(sp);
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(g, w)| close(*g, *w)), "{what}: {got:?} != {want:?}");
}

/// Whether `pt` is inside any subpath of `p` (each a polygon, combined as a union).
fn covers(p: &Path, pt: [f64; 2]) -> bool {
    p.subpaths.iter().any(|sp| {
        let a = anchors(sp);
        let mut inside = false;
        for i in 0..a.len() {
            let (p0, p1) = (a[i], a[(i + 1) % a.len()]);
            if (p0[1] > pt[1]) != (p1[1] > pt[1]) && pt[0] < p0[0] + (pt[1] - p0[1]) / (p1[1] - p0[1]) * (p1[0] - p0[0]) {
                inside = !inside;
            }
        }
        inside
    })
}

/// Every anchor of the reference's one-outline result is an anchor of `p`.
fn has_reference_anchors(p: &Path, reference: &[[f64; 2]]) {
    let all: Vec<[f64; 2]> = p.subpaths.iter().flat_map(anchors).collect();
    for r in reference {
        assert!(all.iter().any(|a| close(*a, *r)), "reference anchor {r:?} missing from {all:?}");
    }
}

#[test]
fn minus_fifty_bends_the_base_away_from_the_tip_and_the_shaft_ends_where_it_crosses() {
    let p = arrow(false, true, -50.0);
    assert_eq!(p.subpaths.len(), 2);
    same(&p.subpaths[0], &[[400.0, 302.0], [589.0, 302.0], [589.0, 298.0], [400.0, 298.0]], "shaft to 589");
    // Tip, outer corners, and the base's middle 5 px (50 % of 10) behind the base at 590.
    same(&p.subpaths[1], &[[600.0, 300.0], [590.0, 302.5], [585.0, 300.0], [590.0, 297.5]], "end head");
    has_reference_anchors(&p, &[[400.0, 302.0], [400.0, 298.0], [589.0, 298.0], [590.0, 297.5], [600.0, 300.0], [590.0, 302.5], [589.0, 302.0]]);
}

#[test]
fn fifty_bends_both_bases_toward_their_tips_by_five_px() {
    let p = arrow(true, true, 50.0);
    assert_eq!(p.subpaths.len(), 3);
    // The shaft's ends follow the bent bases to their middles, filling the wedge in front of them.
    same(&p.subpaths[0], &[[409.0, 302.0], [591.0, 302.0], [595.0, 300.0], [591.0, 298.0], [409.0, 298.0], [405.0, 300.0]], "shaft 409..591");
    same(&p.subpaths[1], &[[400.0, 300.0], [410.0, 302.5], [405.0, 300.0], [410.0, 297.5]], "start head");
    same(&p.subpaths[2], &[[600.0, 300.0], [590.0, 302.5], [595.0, 300.0], [590.0, 297.5]], "end head");
    has_reference_anchors(
        &p,
        &[
            [409.0, 302.0],
            [410.0, 302.5],
            [400.0, 300.0],
            [410.0, 297.5],
            [409.0, 298.0],
            [591.0, 298.0],
            [590.0, 297.5],
            [600.0, 300.0],
            [590.0, 302.5],
            [591.0, 302.0],
        ],
    );
}

#[test]
fn zero_keeps_a_flat_base_and_the_shaft_ends_at_it() {
    let p = arrow(false, true, 0.0);
    assert!(close(anchors(&p.subpaths[0])[1], [590.0, 302.0]));
    assert!(close(anchors(&p.subpaths[1])[2], [590.0, 300.0]));
}

#[test]
fn values_past_fifty_draw_as_fifty_so_older_documents_still_open() {
    assert_eq!(arrow(true, true, 80.0), arrow(true, true, 50.0));
    assert_eq!(arrow(true, true, 100.0), arrow(true, true, 50.0));
    assert_eq!(arrow(true, true, -90.0), arrow(true, true, -50.0));
    assert_eq!(arrow(true, true, f64::NAN), arrow(true, true, 0.0));
}

#[test]
fn a_head_no_wider_than_the_shaft_meets_it_at_the_base_whatever_the_concavity() {
    for c in [-50.0, 0.0, 50.0] {
        let p = arrow_line((400.0, 300.0), (600.0, 300.0), 6.0, false, true, 5.0, 10.0, c);
        assert!(close(anchors(&p.subpaths[0])[1], [590.0, 303.0]), "concavity {c}");
    }
}

#[test]
fn long_heads_bent_away_never_turn_the_shaft_inside_out() {
    for (start, end) in [(true, true), (false, true), (true, false)] {
        let p = arrow_line((0.0, 0.0), (20.0, 0.0), 4.0, start, end, 30.0, 50.0, -50.0);
        let shaft = anchors(&p.subpaths[0]);
        assert!(shaft[0][0] <= shaft[1][0], "{start} {end}: {shaft:?}");
        assert!(p.subpaths.iter().flat_map(anchors).all(|a| a[0].is_finite() && a[1].is_finite()));
    }
}

#[test]
fn a_base_bent_toward_the_tip_leaves_no_hole_and_nothing_past_the_flanks() {
    let p = arrow(true, true, 50.0);
    // The reference outline has no edge between the shaft's end and the bent base's middle.
    for pt in [[593.0, 300.0], [594.5, 300.0], [592.0, 299.0], [407.0, 300.0], [405.5, 300.0], [408.0, 301.0]] {
        assert!(covers(&p, pt), "{pt:?} is inside the reference outline");
    }
    // Between x 591 and 600 the flanks are inside the shaft's width (298.25 at x 593).
    for pt in [[593.0, 298.1], [593.0, 301.9], [407.0, 298.1], [407.0, 301.9]] {
        assert!(!covers(&p, pt), "{pt:?} is outside the reference outline");
    }
    // Outside the shaft's width, the notch stays open between the base and the flank's corner.
    assert!(!covers(&p, [590.6, 297.9]));
}

#[test]
fn a_head_no_wider_than_the_shaft_still_fills_the_wedge_its_bent_base_leaves() {
    let p = arrow_line((400.0, 300.0), (600.0, 300.0), 6.0, false, true, 5.0, 10.0, 50.0);
    same(&p.subpaths[0], &[[400.0, 303.0], [590.0, 303.0], [590.0, 302.5], [595.0, 300.0], [590.0, 297.5], [590.0, 297.0], [400.0, 297.0]], "shaft");
    assert!(covers(&p, [592.0, 300.0]));
}
