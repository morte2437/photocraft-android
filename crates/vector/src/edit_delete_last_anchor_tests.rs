//! Deleting the last anchor of a path (#12764): a saved path may end up empty, everything else
//! keeps refusing it.

use photocraft_doc::{Path, Subpath};
use photocraft_geom::Rect;

use super::*;

fn lone() -> Path {
    Path::new(vec![Subpath::polyline(&[(100.0, 300.0)])])
}

#[test]
fn allowing_empty_removes_the_paths_only_anchor_and_its_subpath() {
    let mut p = lone();
    assert_eq!(delete_anchor_allow_empty(&mut p, [0, 0]), Ok(true));
    assert!(p.subpaths.is_empty());
}

#[test]
fn without_it_the_only_anchor_is_still_refused_and_kept() {
    let mut p = lone();
    assert!(delete_anchor(&mut p, [0, 0]).is_err());
    assert_eq!(p, lone());
}

#[test]
fn allowing_empty_changes_nothing_else() {
    let mut a = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]), Subpath::polyline(&[(5.0, 5.0)])]);
    let mut b = a.clone();
    assert_eq!(delete_anchor_allow_empty(&mut a, [1, 0]), delete_anchor(&mut b, [1, 0]));
    assert_eq!(delete_anchor_allow_empty(&mut a, [0, 1]), delete_anchor(&mut b, [0, 1]));
    assert_eq!(a, b);
}

#[test]
fn allowing_empty_still_rejects_missing_anchors() {
    let mut p = lone();
    for k in [[0, 1], [1, 0], [usize::MAX, 0], [0, usize::MAX]] {
        assert!(delete_anchor_allow_empty(&mut p, k).is_err(), "{k:?}");
    }
    assert_eq!(p, lone());
    let mut empty = Path::new(Vec::new());
    assert!(delete_anchor_allow_empty(&mut empty, [0, 0]).is_err());
}

#[test]
fn an_empty_path_draws_nothing() {
    let mut p = lone();
    delete_anchor_allow_empty(&mut p, [0, 0]).unwrap();
    let cov = crate::fill_rasterizer(&p, crate::DEFAULT_TOLERANCE).render(Rect::from_xywh(0, 0, 200, 400));
    assert!(cov.iter().all(|c| *c == 0.0));
}
