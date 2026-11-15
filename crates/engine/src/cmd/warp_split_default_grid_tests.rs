//! Which rule-of-thirds guides a split on the Default warp grid turns into real sections (#12495).

use super::*;

/// The 400 x 300 layer box of the issue, at (200, 150).
const BOX: [f64; 4] = [200.0, 150.0, 600.0, 450.0];

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// Runs a split command on the warp being edited, as the Free Transform warp session does.
fn split_cmd(s: &mut Session, cmd: &str, warp: Value, at: [f64; 2]) -> BezierMesh {
    let r = s.execute(cmd, json!({"warp": warp, "at": at})).unwrap();
    let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    out.mesh.unwrap()
}

fn default_grid() -> Value {
    serde_json::to_value(Warp::custom(BezierMesh::identity(BOX, 1, 1), BOX)).unwrap()
}

/// The anchor columns (x along the top row) and anchor rows (y down the left column).
fn anchors(m: &BezierMesh) -> (Vec<f64>, Vec<f64>) {
    let xs = (0..m.nx()).step_by(3).map(|i| m.point(i, 0)[0]).collect();
    let ys = (0..m.ny()).step_by(3).map(|j| m.point(0, j)[1]).collect();
    (xs, ys)
}

fn assert_near(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}, want {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 0.01, "{what}: {got:?}, want {want:?}");
    }
}

const THIRDS_X: [f64; 4] = [200.0, 200.0 + 400.0 / 3.0, 200.0 + 800.0 / 3.0, 600.0];
const THIRDS_Y: [f64; 4] = [150.0, 250.0, 350.0, 450.0];

#[test]
fn horizontal_split_of_the_default_grid_is_three_columns_by_two_rows() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &THIRDS_X, "the vertical thirds become anchor columns");
    assert_near(&ys, &[150.0, 320.0, 450.0], "one row line at the click, the horizontal thirds are gone");
    assert!(m.is_valid());
}

#[test]
fn vertical_split_of_the_default_grid_is_two_columns_by_three_rows() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpVertically", default_grid(), [300.0, 200.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[200.0, 300.0, 600.0], "one column line at the click, the vertical thirds are gone");
    assert_near(&ys, &THIRDS_Y, "the horizontal thirds become anchor rows");
    assert!(m.is_valid());
}

#[test]
fn crosswise_split_of_the_default_grid_keeps_both_thirds() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpCrosswise", default_grid(), [300.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[THIRDS_X[0], 300.0, THIRDS_X[1], THIRDS_X[2], THIRDS_X[3]], "columns");
    assert_near(&ys, &[150.0, 250.0, 320.0, 350.0, 450.0], "rows");
}

#[test]
fn a_split_of_an_already_split_grid_adds_only_its_line() {
    let mut s = session();
    let rows = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    let warp = serde_json::to_value(Warp::custom(rows, BOX)).unwrap();
    let m = split_cmd(&mut s, "edit.transform.splitWarpVertically", warp, [300.0, 200.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[THIRDS_X[0], 300.0, THIRDS_X[1], THIRDS_X[2], THIRDS_X[3]], "one more column");
    assert_near(&ys, &[150.0, 320.0, 450.0], "rows are not rebuilt");
    // A 3 x 3 grid that is not the Default one: a horizontal split keeps every section.
    let grid = serde_json::to_value(Warp::custom(BezierMesh::identity(BOX, 3, 3), BOX)).unwrap();
    let m = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", grid, [400.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &THIRDS_X, "columns");
    assert_near(&ys, &[150.0, 250.0, 320.0, 350.0, 450.0], "rows");
}
