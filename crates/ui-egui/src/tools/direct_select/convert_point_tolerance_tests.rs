//! #12748: the Convert Point tool hits an anchor within 5 points, as measured at display scale 1:
//! at 100 % a press 5 px from an anchor converts it and one 6 px away does nothing; at 200 % that
//! is 2.5 document px. Other display scales weren't measured and scale it as every canvas
//! tolerance. Direct Selection, the Pen's ⌥ and the tool's shown handles keep their 6 points.

use egui::{Modifiers, Pos2, Rect, vec2};
use photocraft_doc::Knot;
use photocraft_doc::vector::Path;
use serde_json::json;

use super::convert_point_history_tests::drag;
use crate::PhotocraftApp;
use crate::anchor_tools::delete_anchor_targeted_path_tests::{group_shape, setup, shape_path, steps};
use crate::canvas::ViewXform;
use crate::state::Tool;

/// The measured document: 800 × 600 px with a closed corner square as the Work Path, shown at
/// `zoom` (device pixels per document pixel) on a display of `ppp` device pixels per point.
fn app(tool: Tool, zoom: f32, ppp: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ppp = ppp;
    app.ui.tool = tool;
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

/// Does a drag that starts `dx` document px left of the corner (100, 300) pull out its handles,
/// in one History step?
fn converts_from(app: &mut PhotocraftApp, dx: f64) -> bool {
    let n = steps(app);
    drag(app, [100.0 - dx, 300.0], [100.0 - dx, 340.0], Modifiers::NONE);
    let corner = work(app).subpaths[0].knots[0];
    if corner == Knot::corner(100.0, 300.0) {
        assert_eq!(steps(app), n, "a press that converts nothing adds no step");
        return false;
    }
    assert_eq!(steps(app), n + 1);
    true
}

#[test]
fn at_100_percent_a_press_5_px_from_an_anchor_converts_it_and_6_px_does_nothing() {
    assert!(converts_from(&mut app(Tool::ConvertPoint, 1.0, 1.0), 5.0));
    assert!(!converts_from(&mut app(Tool::ConvertPoint, 1.0, 1.0), 6.0));
}

/// Not measured: the reach stays in points, so 5 points are 6.25 px at display scale 1.25 and
/// 10 px at 2 (6 points were 7.5 and 12 px).
#[test]
fn the_reach_is_5_points_on_any_display_scale() {
    for (ppp, inside, outside) in [(1.25, 6.25, 6.5), (2.0, 10.0, 11.0)] {
        assert!(converts_from(&mut app(Tool::ConvertPoint, 1.0, ppp), inside), "{inside} px at display scale {ppp}");
        assert!(!converts_from(&mut app(Tool::ConvertPoint, 1.0, ppp), outside), "{outside} px at display scale {ppp}");
    }
}

/// A press 5 or 6 points left of the corner, mapped to the document as the canvas maps it, on views
/// panned to fractional centres: the f32 mapping never moves the 5-point press out of reach.
#[test]
fn a_press_mapped_from_the_screen_5_points_away_converts_and_6_does_nothing() {
    for ppp in [1.0, 1.25] {
        for i in 0..40u8 {
            let center = [100.0 + f32::from(i) * 7.31, 300.0 - f32::from(i) * 3.17];
            for (pt, converts) in [(5.0, true), (6.0, false)] {
                let mut app = app(Tool::ConvertPoint, 1.0, ppp);
                app.last_canvas_rect = Rect::from_min_size(Pos2::new(37.3, 61.9), vec2(1013.7, 707.1));
                app.ui.views[0].center = center;
                let xf = ViewXform::active(&app).unwrap();
                let at = xf.to_doc(xf.to_screen(100.0, 300.0) - vec2(pt, 0.0));
                let n = steps(&app);
                drag(&mut app, at, [at[0], at[1] + 40.0], Modifiers::NONE);
                let changed = work(&app).subpaths[0].knots[0] != Knot::corner(100.0, 300.0);
                assert_eq!(changed, converts, "{pt} points at display scale {ppp}, centre {center:?}, press at {at:?}");
                assert_eq!(steps(&app), n + usize::from(converts));
            }
        }
    }
}

/// As today: the front path takes the press even when its anchor is just out of reach, so a path
/// behind with an anchor in reach (a shape inside a group) is not converted either.
#[test]
fn an_anchor_just_out_of_reach_swallows_the_press() {
    let (mut app, shape) = setup(Tool::ConvertPoint);
    group_shape(&mut app, shape);
    let front = json!({"subpaths": [{"closed": false, "knots": [[441.5, 100], [441.5, 40]]}]});
    app.run("path.set", json!({"name": "work", "path": front})).unwrap();
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    // 5.5 px from the Work Path's anchor (441.5, 100), 3 px from the shape's corner (450, 100).
    drag(&mut app, [447.0, 100.0], [447.0, 140.0], Modifiers::NONE);
    assert_eq!(shape_path(&mut app, shape), before);
    assert_eq!(work(&app).subpaths[0].knots[0], Knot::corner(441.5, 100.0));
    assert_eq!(steps(&app), n);
}

#[test]
fn at_200_percent_the_reach_is_2_5_document_pixels() {
    assert!(converts_from(&mut app(Tool::ConvertPoint, 2.0, 1.0), 2.5));
    assert!(!converts_from(&mut app(Tool::ConvertPoint, 2.0, 1.0), 3.0));
}

/// Not measured, so as today: the anchor's shown handles keep 6 points (12 px at display scale 2).
#[test]
fn a_shown_handle_keeps_its_6_points() {
    let mut app = app(Tool::ConvertPoint, 1.0, 2.0);
    drag(&mut app, [100.0, 300.0], [100.0, 340.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]], "the converted anchor's handles show");
    let n = steps(&app);
    drag(&mut app, [111.0, 340.0], [140.0, 340.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[0];
    assert_eq!((k.out_ctrl.x, k.out_ctrl.y), (129.0, 340.0), "the handle moved with the drag");
    assert_eq!((k.in_ctrl.x, k.in_ctrl.y), (100.0, 260.0), "alone");
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn direct_selection_keeps_its_6_points() {
    for (ppp, dx) in [(1.0, 6.0), (2.0, 12.0)] {
        let mut app = app(Tool::DirectSelection, 1.0, ppp);
        drag(&mut app, [100.0 - dx, 300.0], [100.0 - dx, 300.0], Modifiers::NONE);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]], "{dx} px at display scale {ppp}");
    }
}

#[test]
fn the_pens_alt_keeps_its_6_points() {
    for (ppp, dx) in [(1.0, 6.0), (2.0, 12.0)] {
        assert!(converts_from_with_alt(&mut app(Tool::Pen, 1.0, ppp), dx), "{dx} px at display scale {ppp}");
    }
}

fn converts_from_with_alt(app: &mut PhotocraftApp, dx: f64) -> bool {
    drag(app, [100.0 - dx, 300.0], [100.0 - dx, 340.0], Modifiers::ALT);
    app.ui.pen.is_none() && work(app).subpaths[0].knots[0] != Knot::corner(100.0, 300.0)
}

/// While drawing, ⌥ on the last anchor breaks its outgoing handle within 6 points (the Pen's reach
/// is strict, so 11.5 px at display scale 2).
#[test]
fn the_pens_alt_on_the_last_anchor_while_drawing_keeps_its_6_points() {
    let mut app = app(Tool::Pen, 1.0, 2.0);
    drag(&mut app, [500.0, 100.0], [500.0, 100.0], Modifiers::NONE);
    drag(&mut app, [700.0, 100.0], [740.0, 100.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[1][2], [740.0, 100.0]);
    drag(&mut app, [688.5, 100.0], [688.5, 100.0], Modifiers::ALT);
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    assert_eq!(knots.len(), 2, "the press was on the last anchor: no anchor was added");
    assert_eq!(knots[1][0], [700.0, 100.0]);
    assert_ne!(knots[1][2], [740.0, 100.0], "its outgoing handle was taken over");
}
