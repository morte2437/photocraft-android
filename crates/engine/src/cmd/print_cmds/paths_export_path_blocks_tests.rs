//! Paths to Illustrator: each exported path is its own marked block, with one group and one
//! combine note per subpath, and no path name anywhere in the file.

use super::*;
use photocraft_doc::{Knot, NamedPath, Path, PathOp, Subpath};
use photocraft_geom::Point;

const BEGIN: &str = "%Adobe_Photoshop_Path_Begin:<< /defaultFill false >>\n";
const END: &str = "%Adobe_Photoshop_Path_End\n";
const XOR: &str = "%AI3_Note:<< /operation /xor /defaultFill false >>\n";

fn with_op(mut sp: Subpath, op: PathOp) -> Subpath {
    sp.op = op;
    sp
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, op: PathOp) -> Subpath {
    with_op(Subpath::polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]), op)
}

fn circle(cx: f64, cy: f64, r: f64) -> Subpath {
    let k = r * 0.552_284_749_8;
    let knot = |ax: f64, ay: f64, ix: f64, iy: f64, ox: f64, oy: f64| Knot::smooth(Point::new(ax, ay), Point::new(ix, iy), Point::new(ox, oy));
    let mut sp = Subpath::polygon(&[(0.0, 0.0)]);
    sp.knots = vec![
        knot(cx + r, cy, cx + r, cy - k, cx + r, cy + k),
        knot(cx, cy + r, cx + k, cy + r, cx - k, cy + r),
        knot(cx - r, cy, cx - r, cy + k, cx - r, cy - k),
        knot(cx, cy - r, cx - k, cy - r, cx + k, cy - r),
    ];
    sp.op = PathOp::Exclude;
    sp
}

fn doc_with(paths: Vec<(&str, Vec<Subpath>)>) -> Document {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200, "background": "#ffffff", "name": "Blocks.psd"})).unwrap();
    s.edit("paths", |d, _| {
        d.resolution_dpi = 72.0;
        for (name, subs) in paths {
            d.paths.push(NamedPath { name: name.into(), path: Path::new(subs), psd_raw: None });
        }
        Ok(())
    })
    .unwrap();
    (*s.active().unwrap().doc).clone()
}

/// The text after the file's setup section: just the path blocks and the trailer.
fn body(ai: &str) -> &str {
    let from = ai.find("%%EndSetup\n").map(|i| i + "%%EndSetup\n".len()).unwrap();
    &ai[from..ai.find("%%PageTrailer").unwrap()]
}

fn rect_a() -> (&'static str, Vec<Subpath>) {
    ("Rect A", vec![rect(20.0, 20.0, 120.0, 100.0, PathOp::Exclude)])
}

fn curve_b() -> (&'static str, Vec<Subpath>) {
    ("Curve B", vec![circle(200.0, 100.0, 50.0), with_op(Subpath::polyline(&[(150.0, 170.0), (280.0, 170.0)]), PathOp::Exclude)])
}

#[test]
fn a_single_exclude_rectangle_is_one_exact_block_without_its_name() {
    let doc = doc_with(vec![rect_a()]);
    let (ai, n) = paths_to_ai(&doc, "Rect A").unwrap();
    assert_eq!(n, 1);
    let want =
        format!("{BEGIN}*u\n{XOR}1 XR\n20.0000 180.0000 m\n120.0000 180.0000 L\n120.0000 100.0000 L\n20.0000 100.0000 L\n20.0000 180.0000 L\nn\n*U\n{END}");
    assert_eq!(body(&ai), want);
    assert!(!ai.contains("Rect A"), "{ai}");
}

#[test]
fn a_two_subpath_path_is_one_block_with_two_groups() {
    let doc = doc_with(vec![curve_b()]);
    let (ai, _) = paths_to_ai(&doc, "Curve B").unwrap();
    let b = body(&ai);
    assert!(b.starts_with(BEGIN) && b.ends_with(END), "{b}");
    assert_eq!(b.matches("Path_Begin").count(), 1);
    assert_eq!(b.matches("Path_End").count(), 1);
    let lines: Vec<&str> = b.lines().collect();
    let first_end = lines.iter().position(|l| *l == "*U").unwrap();
    // Group one: *u, note, XR, the circle (move plus four curves), n.
    assert_eq!(&lines[1..4], ["*u", XOR.trim_end(), "1 XR"]);
    assert!(lines[4].ends_with(" m"));
    assert!(lines[5..9].iter().all(|l| l.ends_with(" C")), "{lines:?}");
    assert_eq!(&lines[9..=first_end], ["n", "*U"]);
    // Group two: the open line, ending with N.
    assert_eq!(&lines[first_end + 1..], ["*u", XOR.trim_end(), "1 XR", "150.0000 30.0000 m", "280.0000 30.0000 L", "N", "*U", "%Adobe_Photoshop_Path_End"]);
    assert!(!ai.contains("Curve B"));
}

#[test]
fn each_combine_operation_writes_its_note() {
    let sq = |op| rect(0.0, 0.0, 10.0, 10.0, op);
    let doc = doc_with(vec![("Ops", vec![sq(PathOp::Combine), sq(PathOp::Subtract), sq(PathOp::Intersect), sq(PathOp::Exclude), sq(PathOp::Join)])]);
    let (ai, _) = paths_to_ai(&doc, "Ops").unwrap();
    let notes: Vec<&str> = ai.lines().filter(|l| l.starts_with("%AI3_Note:")).collect();
    assert_eq!(
        notes,
        [
            "%AI3_Note:<< /operation /union >>",
            "%AI3_Note:<< /operation /subtract >>",
            "%AI3_Note:<< /operation /intersect >>",
            "%AI3_Note:<< /operation /xor /defaultFill false >>",
            // A Join subpath takes the note of the component it belongs to.
            "%AI3_Note:<< /operation /xor /defaultFill false >>",
        ]
    );
}

#[test]
fn add_then_subtract_is_union_then_subtract() {
    let doc = doc_with(vec![("Frame", vec![rect(0.0, 0.0, 100.0, 100.0, PathOp::Combine), rect(25.0, 25.0, 75.0, 75.0, PathOp::Subtract)])]);
    let (ai, _) = paths_to_ai(&doc, "Frame").unwrap();
    let notes: Vec<&str> = ai.lines().filter(|l| l.starts_with("%AI3_Note:")).collect();
    assert_eq!(notes, ["%AI3_Note:<< /operation /union >>", "%AI3_Note:<< /operation /subtract >>"]);
    assert_eq!(ai.matches("*u\n").count(), 2);
    assert_eq!(ai.matches("Path_Begin").count(), 1);
}

#[test]
fn all_paths_are_consecutive_blocks_one_per_path() {
    let doc = doc_with(vec![rect_a(), curve_b()]);
    let (ai, n) = paths_to_ai(&doc, "all").unwrap();
    assert_eq!(n, 2);
    let b = body(&ai);
    assert_eq!(b.matches(BEGIN).count(), 2);
    assert!(b.contains(&format!("*U\n{END}{BEGIN}*u\n")), "{b}");
    assert!(!b.contains("%%Note:"));
}

#[test]
fn a_path_without_knots_writes_nothing_and_empty_subpaths_are_skipped() {
    let mut empty = Subpath::polygon(&[(0.0, 0.0)]);
    empty.knots.clear();
    let doc = doc_with(vec![("Empty", vec![empty.clone()]), ("Mixed", vec![empty, rect(0.0, 0.0, 10.0, 10.0, PathOp::Exclude)])]);
    let (ai, n) = paths_to_ai(&doc, "all").unwrap();
    assert_eq!(n, 1);
    assert_eq!(body(&ai).matches(BEGIN).count(), 1);
    assert_eq!(body(&ai).matches("*u\n").count(), 1);
    assert_eq!(paths_to_ai(&doc, "Empty").unwrap().1, 0);
    assert_eq!(paths_to_ai(&doc, "Empty").unwrap().0.matches("Path_Begin").count(), 0);
}
