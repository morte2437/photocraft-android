//! #12913: the Pen's canvas menu names its transform row after what it would act on: Free
//! Transform (greyed) with no path, Free Transform Points while drawing, Free Transform Path on a
//! finished path. A path that exists but isn't active, and the Path Selection and Direct
//! Selection menus, keep Free Transform Path.

use egui::Modifiers;
use egui_kittest::kittest::{NodeT, Queryable};

use super::tests::{app, choose_open, harness, right_click};
use super::{PEN_MENU, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

const LABELS: [&str; 3] = ["Free Transform", "Free Transform Points", "Free Transform Path"];

/// Pen clicks at document points `at`, as on the canvas.
fn pen_clicks(app: &mut PhotocraftApp, at: &[(f64, f64)]) {
    app.ui.tool = Tool::Pen;
    for &(x, y) in at {
        crate::vector_ui::pen_down(app, x, y, false);
        crate::vector_ui::pen_up(app);
    }
}

/// Right-click the middle of the canvas with `tool`: the one transform row label the path menu
/// shows, and whether that row is enabled. `ui.inspect` lists the row the same way.
fn transform_row(mut app: PhotocraftApp, tool: Tool) -> (&'static str, bool) {
    app.ui.tool = tool;
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == tool && rows(m) == PEN_MENU), "{tool:?}: the path menu is open");
    let shown: Vec<_> = LABELS.into_iter().filter(|label| h.query_by_label(label).is_some()).collect();
    assert_eq!(shown.len(), 1, "{tool:?}: one transform row, got {shown:?}");
    let enabled = !h.get_by_label(shown[0]).accesskit_node().is_disabled();
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    let entries = inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default();
    let entry = entries.iter().find(|e| e["id"] == "path.transform");
    assert_eq!(entry.map(|e| (&e["label"], &e["enabled"])), Some((&serde_json::json!(shown[0]), &serde_json::json!(enabled))), "{tool:?}: ui.inspect");
    (shown[0], enabled)
}

#[test]
fn with_no_path_the_pen_row_reads_free_transform_greyed() {
    assert_eq!(transform_row(app(), Tool::Pen), ("Free Transform", false));
}

#[test]
fn while_drawing_the_pen_row_reads_free_transform_points() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    assert!(app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2), "the path is still being drawn");
    assert_eq!(transform_row(app, Tool::Pen), ("Free Transform Points", true));
}

#[test]
fn on_a_closed_path_the_pen_row_reads_free_transform_path() {
    let mut app = app();
    // The last click on the first anchor closes the path.
    pen_clicks(&mut app, &[(2.0, 2.0), (28.0, 2.0), (28.0, 28.0), (2.0, 2.0)]);
    assert!(app.ui.pen.is_none(), "the path is finished");
    assert!(app.session.active().unwrap().doc.work_path.as_ref().is_some_and(|p| p.subpaths.iter().all(|s| s.closed)));
    assert_eq!(transform_row(app, Tool::Pen), ("Free Transform Path", true));
}

/// Make Selection hides the Work Path (#6079): it stays in the document but isn't active, so the
/// row keeps today's label, greyed.
#[test]
fn an_inactive_path_keeps_free_transform_path() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (28.0, 2.0), (28.0, 28.0), (2.0, 2.0)]);
    choose_open(&mut app, "path.toSelection");
    let dialog = app.ui.dialogs.last().map(|d| d.id).unwrap();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert!(!crate::vector_ui::work_path_shown(&app), "the Work Path is not active");
    assert_eq!(transform_row(app, Tool::Pen), ("Free Transform Path", false));
}

#[test]
fn path_selection_and_direct_selection_keep_free_transform_path() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        assert_eq!(transform_row(app(), tool), ("Free Transform Path", false), "{tool:?} with no path");
        let mut app = app();
        pen_clicks(&mut app, &[(2.0, 2.0), (28.0, 2.0), (28.0, 28.0), (2.0, 2.0)]);
        assert_eq!(transform_row(app, tool), ("Free Transform Path", true), "{tool:?} on a path");
    }
}
