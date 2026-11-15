//! Add Anchor Point on a straight segment (#12706): the segment is split like any cubic (controls
//! start, start, end, end), so the new anchor is smooth with handles and the neighbours keep
//! their retracted handles.

use photocraft_doc::{Path, Subpath};

use super::*;

fn square() -> Path {
    Path::new(vec![Subpath::polygon(&[(100.0, 300.0), (300.0, 300.0), (300.0, 500.0), (100.0, 500.0)])])
}

fn knot(p: &Path, k: usize) -> Knot {
    p.subpaths[0].knots[k]
}

fn near(a: Point, x: f64, y: f64, tol: f64) -> bool {
    (a.x - x).abs() <= tol && (a.y - y).abs() <= tol
}

#[test]
fn a_closed_square_edge_gets_a_smooth_anchor_with_quarter_length_handles() {
    let mut p = square();
    let before = p.subpaths[0].segments()[0];
    let added = add_anchor(&mut p, [0, 0], 0.5).unwrap();
    assert_eq!(added, [0, 1]);
    let k = knot(&p, 1);
    assert!(near(k.anchor, 200.0, 300.0, 0.5), "anchor {:?}", k.anchor);
    assert!(near(k.in_ctrl, 150.0, 300.0, 0.5), "in handle {:?}", k.in_ctrl);
    assert!(near(k.out_ctrl, 250.0, 300.0, 0.5), "out handle {:?}", k.out_ctrl);
    assert!(k.smooth, "the new anchor is smooth");
    let (first, last) = (knot(&p, 0), knot(&p, 2));
    assert_eq!(first.out_ctrl, first.anchor, "(100,300) keeps its handle retracted");
    assert_eq!(last.in_ctrl, last.anchor, "(300,300) keeps its handle retracted");
    assert_eq!(p.subpaths[0].knots.len(), 5);
    // The outline is unchanged: both new cubics lie on the old line, and together cover it.
    let segs = p.subpaths[0].segments();
    for i in 0..=100 {
        let u = f64::from(i) / 100.0;
        for q in [eval(&segs[0], u), eval(&segs[1], u)] {
            assert!((q.y - 300.0).abs() < 1e-9 && (100.0..=300.0).contains(&q.x), "off the old line: {q:?}");
        }
    }
    let (a, b) = (eval(&segs[0], 1.0), eval(&segs[1], 0.0));
    assert!(dist(a, b) < 1e-9 && dist(eval(&segs[0], 0.0), eval(&before, 0.0)) < 1e-9 && dist(eval(&segs[1], 1.0), eval(&before, 1.0)) < 1e-9);
}

#[test]
fn an_open_line_hit_at_650_gets_the_measured_handles() {
    let mut p = Path::new(vec![Subpath::polyline(&[(400.0, 300.0), (700.0, 300.0)])]);
    // `t` the way the Add tool finds it: `hit` on the clicked point, which returns the cubic
    // parameter (not linear along a retracted segment), then `add_anchor` at that `t`.
    let Some(Hit::Segment(seg, t)) = hit(&p, Point::new(650.0, 300.0), 1.0, &[]) else { panic!("no segment hit") };
    let added = add_anchor(&mut p, seg, t).unwrap();
    let k = knot(&p, added[1]);
    assert!(near(k.anchor, 650.0, 300.0, 1.0), "anchor {:?}", k.anchor);
    assert!(near(k.in_ctrl, 564.0, 300.0, 1.0), "in handle {:?}", k.in_ctrl);
    assert!(near(k.out_ctrl, 679.6, 300.0, 1.0), "out handle {:?}", k.out_ctrl);
    assert!(k.smooth);
}
