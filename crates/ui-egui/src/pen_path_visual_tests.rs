//! Synthetic offscreen evidence for editing a selected saved path beside the Work Path.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::kittest::Queryable;
use photocraft_doc::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

fn path(app: &PhotocraftApp, name: &str) -> Path {
    let doc = &app.session.active().unwrap().doc;
    if name == "work" { doc.work_path.clone().unwrap() } else { doc.paths.iter().find(|p| p.name == name).unwrap().path.clone() }
}

fn translated(mut path: Path, dx: f64, dy: f64) -> Path {
    for knot in path.subpaths.iter_mut().flat_map(|s| &mut s.knots) {
        for point in [&mut knot.anchor, &mut knot.in_ctrl, &mut knot.out_ctrl] {
            point.x += dx;
            point.y += dy;
        }
    }
    path
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Run explicitly with a wgpu adapter. PHOTOCRAFT_PEN_VISUAL_DIR chooses the output directory;
/// PHOTOCRAFT_PEN_VISUAL_BASELINE=1 characterizes the old Work Path targeting for a before capture.
#[test]
#[ignore = "manual offscreen visual fixture; writes synthetic screenshots"]
fn saved_path_drag_visual_fixture() {
    let baseline = std::env::var("PHOTOCRAFT_PEN_VISUAL_BASELINE").is_ok_and(|v| v == "1");
    let output = std::env::var_os("PHOTOCRAFT_PEN_VISUAL_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| "log/pen-path-visual".into());
    std::fs::create_dir_all(&output).expect("create synthetic screenshot directory");
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.edit_prefs(|p| p.interface.language = "en".into());
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            app.set_wgpu(rs.clone());
        }
        app.run("file.new", json!({"name": "Saved path targeting", "width": 400, "height": 260, "background": "white"})).unwrap();
        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[40, 80], [140, 80], [90, 180]]}]}})).unwrap();
        app.run(
            "path.set",
            json!({"name": "Saved drawing", "path": {"subpaths": [{"closed": false, "knots": [
                {"anchor": [220, 90], "in": [220, 90], "out": [260, 40]},
                {"anchor": [320, 180], "in": [280, 230], "out": [320, 180]}
            ]}]}}),
        )
        .unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        app.ui.extras.snap = false;
        app.ui.dock_tabs.layers = 2;
        app.ui.views[0].fit_pending = false;
        app.ui.views[0].zoom = 1.5;
        app.ui.views[0].center = [200.0, 130.0];
        app
    });
    h.run_steps(8);
    h.get_by_label("Saved drawing").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved drawing"), "the Paths panel row selects the saved path");
    let (work, saved) = (path(h.state(), "work"), path(h.state(), "Saved drawing"));
    let history = h.state().session.active().unwrap().history.past_len();
    let mode = if baseline { "baseline" } else { "fixed" };
    let before = output.join(format!("{mode}-before.png"));
    h.render().expect("render initial scene").save(&before).expect("save initial scene");

    // Screen coordinates come from the real full-app layout, including its dock and rulers.
    let xf = ViewXform::active(h.state()).expect("active document canvas");
    let start = xf.to_screen(220.0, 90.0);
    let end = xf.to_screen(245.0, 70.0);
    assert!(xf.rect.contains(start) && xf.rect.contains(end));
    h.event(Event::PointerMoved(start));
    h.event(button(start, true));
    h.step();
    h.event(Event::PointerMoved(end));
    h.step();
    assert!(h.state().drag.is_some(), "native movement past the drag threshold starts a path gesture");
    h.event(button(end, false));
    h.step();
    h.run_steps(12);
    assert!(h.state().drag.is_none());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1, "one native drag is one Undo step");
    if baseline {
        assert_eq!(path(h.state(), "Saved drawing"), saved, "baseline incorrectly leaves the selected saved path untouched");
        assert_eq!(path(h.state(), "work"), translated(work, 25.0, -20.0), "baseline incorrectly moves the Work Path");
    } else {
        assert_eq!(path(h.state(), "work"), work, "editing a saved path preserves the Work Path");
        assert_eq!(path(h.state(), "Saved drawing"), translated(saved, 25.0, -20.0), "the selected saved path and its curve handles move together");
    }
    let after = output.join(format!("{mode}-after.png"));
    h.render().expect("render completed drag").save(&after).expect("save completed drag");
    println!("{mode}: selected Saved drawing; native drag (+25, -20); screenshots: {}, {}", before.display(), after.display());
}
