//! #12912: removing the only split of a one-way split of the Default warp grid gives back the
//! Default grid (one patch, its thirds drawn as guides), for a horizontal and a vertical split.
//! Other grids, and thirds that were edited after the split, keep today's behaviour.

use super::*;

/// The 400 x 300 layer box of the issue, at (200, 150).
const BOX: [f64; 4] = [200.0, 150.0, 600.0, 450.0];
const THIRDS: [f64; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// Runs a split command on `mesh`, as the Free Transform warp session does.
fn split_cmd(s: &mut Session, cmd: &str, mesh: BezierMesh, at: [f64; 2]) -> BezierMesh {
    let warp = serde_json::to_value(Warp::custom(mesh, BOX)).unwrap();
    let r = s.execute(cmd, json!({"warp": warp, "at": at})).unwrap();
    let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    out.mesh.unwrap()
}

fn default_grid() -> BezierMesh {
    BezierMesh::identity(BOX, 1, 1)
}

fn assert_mesh_near(got: &BezierMesh, want: &BezierMesh) {
    assert_eq!((got.us.len(), got.vs.len()), (want.us.len(), want.vs.len()), "{got:?}");
    assert_eq!(got.points.len(), want.points.len());
    for (g, w) in got.points.iter().zip(&want.points) {
        assert!((g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6, "{g:?} != {w:?}");
    }
}

#[test]
fn removing_the_only_horizontal_split_gives_back_the_default_grid() {
    let mut s = session();
    let split = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    assert_eq!((split.us.len(), split.vs.len()), (4, 3), "3 columns x 2 rows");
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", split, [400.0, 320.0]);
    assert_eq!((m.us.as_slice(), m.vs.as_slice()), ([0.0, 1.0].as_slice(), [0.0, 1.0].as_slice()));
    assert_mesh_near(&m, &default_grid());
}

#[test]
fn removing_the_only_vertical_split_gives_back_the_default_grid() {
    let mut s = session();
    let split = split_cmd(&mut s, "edit.transform.splitWarpVertically", default_grid(), [300.0, 200.0]);
    assert_eq!((split.us.len(), split.vs.len()), (3, 4), "2 columns x 3 rows");
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", split, [300.0, 200.0]);
    assert_eq!((m.us.as_slice(), m.vs.as_slice()), ([0.0, 1.0].as_slice(), [0.0, 1.0].as_slice()));
    assert_mesh_near(&m, &default_grid());
}

#[test]
fn a_bent_default_grid_comes_back_bent() {
    // The whole Default patch warped before the split: the merge is exact, so it comes back as is.
    let mut bent = default_grid();
    let nx = bent.nx();
    bent.points[1] = [bent.points[1][0], bent.points[1][1] - 40.0];
    bent.points[nx + 2] = [bent.points[nx + 2][0] + 25.0, bent.points[nx + 2][1]];
    let mut s = session();
    let split = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", bent.clone(), [400.0, 320.0]);
    // On the row line, halfway across: well clear of the column thirds.
    let at = split.eval(0.5, split.vs[1]);
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", split, at);
    assert_mesh_near(&m, &bent);
}

#[test]
fn edited_thirds_are_kept() {
    // An anchor on a third moved after the split: merging would lose it, so the columns stay.
    let mut s = session();
    let mut split = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    let nx = split.nx();
    split.points[3][0] += 20.0;
    split.points[nx * 3 + 3][0] += 20.0;
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", split, [400.0, 320.0]);
    assert_eq!(m.us, THIRDS.to_vec(), "the three columns stay");
    assert_eq!(m.vs, vec![0.0, 1.0], "the row line is removed");
}

#[test]
fn other_grids_keep_todays_behaviour() {
    let mut s = session();
    // A 3 x 3 grid: removing one row line leaves 3 x 2.
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", BezierMesh::identity(BOX, 3, 3), [400.0, 250.0]);
    assert_eq!((m.us.len(), m.vs.len()), (4, 3));
    // A 2 x 2 grid: removing its row line leaves 2 columns, not the Default grid.
    let m = split_cmd(&mut s, "edit.transform.removeWarpSplit", BezierMesh::identity(BOX, 2, 2), [250.0, 300.0]);
    assert_eq!((m.us.as_slice(), m.vs.as_slice()), ([0.0, 0.5, 1.0].as_slice(), [0.0, 1.0].as_slice()));
}
