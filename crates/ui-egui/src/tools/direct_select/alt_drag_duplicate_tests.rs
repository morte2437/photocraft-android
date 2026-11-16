//! #13211: ⌥/Alt-drag on a segment with Direct Selection appends a moved copy of that subpath, with
//! its operation, to the same path, in one History step named "Drag Path", as in the reference.
//! ⌥-click still selects the whole subpath; the cases the reference was not measured in (⌥ on an
//! anchor, ⇧⌥, ⌘/Ctrl+⌥) still move the subpath.

use egui::Modifiers;
use photocraft_doc::vector::{Path, Subpath};
use photocraft_doc::{LayerContent, PathOp};
use serde_json::{Value, json};

use super::convert_point_history_tests::drag;
use super::{Drag, PathRef, path_of};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const A: [[f64; 2]; 4] = [[100.0, 300.0], [300.0, 300.0], [300.0, 500.0], [100.0, 500.0]];
/// The middle of A's top segment, and the measured drag from it.
const PRESS: [f64; 2] = [200.0, 300.0];
const MOVE: [f64; 2] = [50.0, 30.0];

/// The measured test path: A first of four closed squares, all Exclude Overlapping Shapes.
fn four() -> Value {
    let square = |[x, y]: [f64; 2], w: f64| json!({"closed": true, "op": "exclude", "knots": [[x, y], [x + w, y], [x + w, y + w], [x, y + w]]});
    json!({"subpaths": [{"closed": true, "op": "exclude", "knots": A}, square([400.0, 50.0], 150.0), square([450.0, 400.0], 120.0), square([620.0, 620.0], 100.0)]})
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 800, "background": "white"})).unwrap();
    app.ui.tool = Tool::DirectSelection;
    app
}

/// The test path as each kind of path Direct Selection edits, and how it addresses it.
fn with_target(kind: &str) -> (PhotocraftApp, PathRef) {
    let mut app = app();
    let target = match kind {
        "saved" => {
            app.run("path.set", json!({"name": "Path 1", "path": four()})).unwrap();
            app.ui.selected_path = Some("Path 1".into());
            PathRef::Saved(0)
        }
        "work" => {
            app.run("path.set", json!({"name": "work", "path": four()})).unwrap();
            PathRef::Work
        }
        "shape" => PathRef::Layer(app.run("shape.create", json!({"kind": "path", "path": four(), "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap()),
        "mask" => {
            app.run("layer.new.layer", json!({})).unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap().0;
            app.run("layer.vectorMask.add", json!({"layer": id, "path": four()})).unwrap();
            app.ui.vector_mask_target = true;
            PathRef::Layer(id)
        }
        _ => panic!("unknown target fixture: {kind}"),
    };
    app.sync_views();
    (app, target)
}

fn path(app: &PhotocraftApp, target: PathRef) -> Path {
    path_of(app, &target).unwrap().into_owned()
}

fn undo_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.undo_label().map(str::to_string)
}

fn plus(p: [f64; 2], d: [f64; 2]) -> [f64; 2] {
    [p[0] + d[0], p[1] + d[1]]
}

fn anchors(sp: &Subpath) -> Vec<[f64; 2]> {
    sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn moved(points: &[[f64; 2]], d: [f64; 2]) -> Vec<[f64; 2]> {
    points.iter().map(|p| [p[0] + d[0], p[1] + d[1]]).collect()
}

/// The points match to within float noise.
fn near(actual: &[[f64; 2]], expected: &[[f64; 2]]) -> bool {
    actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, e)| (a[0] - e[0]).abs() < 1e-3 && (a[1] - e[1]).abs() < 1e-3)
}

/// The measured example: the copy lands at 150,330–350,530 as sub 4 with A's operation, A stays,
/// one "Drag Path" step that one undo takes back.
#[test]
fn alt_drag_on_a_segment_appends_a_moved_copy_in_one_drag_path_step() {
    let (mut app, target) = with_target("saved");
    let (before, label) = (path(&app, target), undo_label(&app));
    let steps = app.session.active().unwrap().history.entries().len();
    drag(&mut app, PRESS, plus(PRESS, MOVE), Modifiers::ALT);
    let after = path(&app, target);
    assert_eq!(after.subpaths.len(), 5, "a copy was appended");
    assert!(near(&anchors(&after.subpaths[4]), &moved(&A, MOVE)), "the copy is at 150,330–350,530: {:?}", anchors(&after.subpaths[4]));
    assert_eq!(after.subpaths[4].op, PathOp::Exclude, "the copy keeps the source's operation");
    assert!(after.subpaths[4].closed);
    assert_eq!(after.subpaths[..4], before.subpaths[..], "A and the other subpaths stay where they were");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one History step");
    assert_eq!(undo_label(&app).as_deref(), Some("Drag Path"));
    assert_eq!(app.ui.direct_selection.anchors, vec![[4, 0], [4, 1], [4, 2], [4, 3]], "the dragged copy is selected");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(path(&app, target), before, "one undo removes the copy");
    assert_eq!(undo_label(&app), label);
}

#[test]
fn alt_drag_duplicates_on_the_work_path_shape_layers_and_vector_masks() {
    for kind in ["work", "shape", "mask"] {
        let (mut app, target) = with_target(kind);
        let before = path(&app, target);
        assert_eq!(before.subpaths.len(), 4, "{kind}");
        drag(&mut app, PRESS, plus(PRESS, MOVE), Modifiers::ALT);
        assert_eq!(app.ui.direct_selection.target, Some(target), "{kind}");
        let after = path(&app, target);
        assert_eq!(after.subpaths.len(), 5, "{kind}: a copy was appended");
        assert!(near(&anchors(&after.subpaths[4]), &moved(&A, MOVE)), "{kind}: {:?}", anchors(&after.subpaths[4]));
        assert_eq!(after.subpaths[4].op, before.subpaths[0].op, "{kind}");
        assert_eq!(after.subpaths[..4], before.subpaths[..], "{kind}: the source stays");
        assert_eq!(undo_label(&app).as_deref(), Some("Drag Path"), "{kind}");
        if let ("shape", PathRef::Layer(id)) = (kind, target) {
            let layer = app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
            assert!(matches!(layer.content, LayerContent::Shape(_)), "still a shape layer");
        }
    }
}

/// While dragging, the preview shows the source in place and the copy following the pointer.
#[test]
fn the_drag_previews_the_copy_beside_the_source() {
    let (mut app, target) = with_target("saved");
    let before = path(&app, target);
    tool_event(&mut app, ToolEvent::Down { x: PRESS[0], y: PRESS[1], pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: PRESS[0] + MOVE[0], y: PRESS[1] + MOVE[1], pressure: 1.0 }, Modifiers::ALT);
    let Some(Drag::Edit { preview: Some(preview), .. }) = &app.ui.direct_selection.drag else { panic!("an edit drag with a preview") };
    assert_eq!(preview.subpaths.len(), 5);
    assert_eq!(preview.subpaths[..4], before.subpaths[..]);
    assert!(near(&anchors(&preview.subpaths[4]), &moved(&A, MOVE)));
    assert_eq!(path(&app, target), before, "nothing is committed before the release");
}

#[test]
fn alt_click_without_a_drag_selects_the_whole_subpath_and_changes_nothing() {
    let (mut app, target) = with_target("saved");
    let (before, label) = (path(&app, target), undo_label(&app));
    drag(&mut app, PRESS, PRESS, Modifiers::ALT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0], [0, 1], [0, 2], [0, 3]]);
    assert_eq!(path(&app, target), before);
    assert_eq!(undo_label(&app), label);
}

/// ⇧ pressed during the drag locks the copy's move to 45°, as it does for a moved subpath.
#[test]
fn shift_during_the_drag_locks_the_copy_to_45_degrees() {
    let (mut app, target) = with_target("saved");
    let before = path(&app, target);
    let alt_shift = Modifiers { alt: true, shift: true, ..Modifiers::NONE };
    tool_event(&mut app, ToolEvent::Down { x: PRESS[0], y: PRESS[1], pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Move { x: PRESS[0] + 30.0, y: PRESS[1] + 26.0, pressure: 1.0 }, Modifiers::ALT);
    // Well inside the diagonal's range, so the exact snapping rule does not matter.
    let end = plus(PRESS, [60.0, 52.0]);
    tool_event(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, alt_shift);
    tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, alt_shift);
    let after = path(&app, target);
    assert_eq!(after.subpaths.len(), 5, "a copy was appended");
    let (copy, a) = (after.subpaths[4].knots[0].anchor, A[0]);
    let d = [copy.x - a[0], copy.y - a[1]];
    assert!(d[0] > 1.0 && (d[0] - d[1]).abs() < 1e-3, "the copy moved along the diagonal: {d:?}");
    assert!(near(&anchors(&after.subpaths[4]), &moved(&A, d)));
    assert_eq!(after.subpaths[..4], before.subpaths[..], "the source stays");
}

/// With only some subpaths' anchors drawn (after Delete Anchor Point, #12769), the copy's are too.
#[test]
fn the_copy_shows_its_anchors_when_only_some_subpaths_show() {
    let (mut app, target) = with_target("saved");
    let owner = super::Owner::current(&app);
    app.ui.direct_selection = super::DirectSelection { target: Some(target), shown: Some(vec![0]), owner, ..Default::default() };
    drag(&mut app, PRESS, plus(PRESS, MOVE), Modifiers::ALT);
    assert_eq!(path(&app, target).subpaths.len(), 5, "a copy was appended");
    assert_eq!(app.ui.direct_selection.shown, Some(vec![0, 4]));
}

/// Not measured in the reference, so as before: these drags move the subpath itself, copy nothing
/// and keep today's step name.
#[test]
fn unmeasured_alt_drags_still_move_the_subpath() {
    let alt_shift = Modifiers { alt: true, shift: true, ..Modifiers::NONE };
    let command_alt = Modifiers { alt: true, ..Modifiers::COMMAND };
    // The Mac's Control key: `ctrl` without `command`.
    let control_alt = Modifiers { alt: true, ctrl: true, ..Modifiers::NONE };
    // Horizontal, so ⇧'s angle lock leaves the move as it is.
    let d = [50.0, 0.0];
    let cases = [
        ("⌥ on an anchor", A[0], Modifiers::ALT),
        ("⇧⌥ on a segment", PRESS, alt_shift),
        ("⌘⌥ on a segment", PRESS, command_alt),
        ("⌃⌥ on a segment", PRESS, control_alt),
    ];
    for (case, from, mods) in cases {
        let (mut app, target) = with_target("saved");
        let before = path(&app, target);
        drag(&mut app, from, plus(from, d), mods);
        let after = path(&app, target);
        assert_eq!(after.subpaths.len(), 4, "{case}: no copy");
        assert!(near(&anchors(&after.subpaths[0]), &moved(&A, d)), "{case}: A moved: {:?}", anchors(&after.subpaths[0]));
        assert_eq!(after.subpaths[1..], before.subpaths[1..], "{case}");
        assert_eq!(undo_label(&app).as_deref(), Some("Drag Anchor Points"), "{case}");
    }
    // A Type Path: ⌥-drag on its segment past the text's range moves it.
    let mut app = app();
    let target = PathRef::TypePath(super::tests::type_on_a_path(&mut app));
    let before = anchors(&path(&app, target).subpaths[0]);
    drag(&mut app, [100.2, 160.0], [150.2, 160.0], Modifiers::ALT);
    assert_eq!(app.ui.direct_selection.target, Some(target));
    let after = path(&app, target);
    assert_eq!(after.subpaths.len(), 1, "Type Path: no copy");
    assert!(near(&anchors(&after.subpaths[0]), &moved(&before, d)), "Type Path moved: {:?}", anchors(&after.subpaths[0]));
    assert_eq!(undo_label(&app).as_deref(), Some("Drag Anchor Points"), "Type Path");
}
