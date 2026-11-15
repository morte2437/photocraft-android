//! #13287: with Path Selection and components of the saved path targeted in the Paths panel
//! selected, Esc deselects the components and keeps the path targeted, and Enter deselects them and
//! untargets the path. Neither adds a History step or moves a point.

use egui::{Key, Modifiers};
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::direct_select::{DirectSelection, PathRef, active_paths};
use crate::state::Tool;

/// 800 x 600 with the saved path "Path 1" (a square and a triangle) targeted, and the square
/// selected as a component.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": true, "knots": [[400, 300], [600, 300], [500, 500]]}
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PathSelection;
    app.ui.selected_path = Some("Path 1".into());
    crate::path_components::press(&mut app, [300.0, 300.0], false);
    let (target, ..) = crate::path_components::selected(&app).expect("the square is selected");
    assert_eq!(target, PathRef::Saved(0));
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    ctx.begin_pass(egui::RawInput { events: vec![ev], ..Default::default() });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn path_1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").unwrap().path.clone()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn nothing_selected(app: &PhotocraftApp) {
    assert!(crate::path_components::selected(app).is_none(), "no component is selected");
    assert_eq!(app.ui.direct_selection, DirectSelection::default());
}

/// Until Path Selection has its own arrow-key nudge (#10920), an arrow key must move nothing once
/// the components are deselected.
fn arrow_moves_nothing(app: &mut PhotocraftApp) {
    let (before, n) = (path_1(app), steps(app));
    press(app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(path_1(app), before, "the arrow key moves no point");
    assert_eq!(steps(app), n, "and adds no History step");
}

#[test]
fn enter_deselects_the_components_and_untargets_the_path() {
    let mut app = app();
    let (before, n) = (path_1(&app), steps(&app));
    press(&mut app, Key::Enter, Modifiers::NONE);
    nothing_selected(&app);
    assert_eq!(app.ui.selected_path, None, "Path 1 is no longer targeted");
    assert!(!active_paths(&app).iter().any(|(r, _)| *r == PathRef::Saved(0)), "its outline is gone");
    assert_eq!(path_1(&app), before);
    assert_eq!(steps(&app), n, "Enter adds no History step");
    arrow_moves_nothing(&mut app);
}

#[test]
fn escape_deselects_the_components_and_keeps_the_path_targeted() {
    let mut app = app();
    let (before, n) = (path_1(&app), steps(&app));
    press(&mut app, Key::Escape, Modifiers::NONE);
    nothing_selected(&app);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
    assert!(active_paths(&app).iter().any(|(r, _)| *r == PathRef::Saved(0)), "its outline stays");
    assert_eq!(path_1(&app), before);
    assert_eq!(steps(&app), n, "Esc adds no History step");
    arrow_moves_nothing(&mut app);
}

#[test]
fn enter_with_a_work_path_present_does_not_make_the_work_path_the_drawn_path() {
    let mut app = app();
    let line = json!({"subpaths": [{"knots": [[100, 100], [200, 100]]}]});
    app.run("path.set", json!({"name": "work", "path": line})).unwrap();
    // `path.set` of the Work Path changes the panel's target: target Path 1 and its component again.
    app.ui.selected_path = Some("Path 1".into());
    crate::path_components::press(&mut app, [300.0, 300.0], false);
    assert!(crate::path_components::selected(&app).is_some());
    press(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(app.ui.selected_path, None);
    assert!(!crate::vector_ui::work_path_shown(&app), "the Work Path stays hidden");
    assert!(active_paths(&app).is_empty(), "no path is drawn");
}

#[test]
fn escape_with_a_pixel_selection_deselects_the_pixels_and_keeps_the_components() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    press(&mut app, Key::Escape, Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "Esc deselects the pixels");
    assert!(crate::path_components::selected(&app).is_some(), "the components stay selected");
}

#[test]
fn shift_or_alt_with_enter_or_escape_keeps_the_components_and_the_path() {
    for key in [Key::Enter, Key::Escape] {
        for modifiers in [Modifiers::SHIFT, Modifiers::ALT] {
            let mut app = app();
            press(&mut app, key, modifiers);
            assert!(crate::path_components::selected(&app).is_some(), "{key:?} {modifiers:?}");
            assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "{key:?} {modifiers:?}");
        }
    }
}

#[test]
fn enter_and_escape_with_no_component_selected_keep_the_path_targeted() {
    for key in [Key::Enter, Key::Escape] {
        let mut app = app();
        crate::path_components::deselect_components(&mut app);
        press(&mut app, key, Modifiers::NONE);
        assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"), "{key:?}");
    }
}

#[test]
fn free_transform_path_still_owns_enter_and_escape() {
    let ctx = egui::Context::default();
    let mut app = app();
    let (before, n) = (path_1(&app), steps(&app));
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    assert!(app.ui.transform.is_some());
    press(&mut app, Key::Escape, Modifiers::NONE);
    assert!(app.ui.transform.is_none(), "Esc cancels the transform");
    assert_eq!(path_1(&app), before);
    assert_eq!(steps(&app), n, "cancelling adds no History step");

    let mut app = self::app();
    let n = steps(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    // 5 px down: an unchanged box commits nothing.
    let t = app.ui.transform.as_mut().unwrap();
    let [x0, y0, x1, y1] = t.rect;
    t.quad = [[x0, y0 + 5.0], [x1, y0 + 5.0], [x1, y1 + 5.0], [x0, y1 + 5.0]];
    press(&mut app, Key::Enter, Modifiers::NONE);
    assert!(app.ui.transform.is_none(), "Enter commits the transform");
    assert_eq!(steps(&app), n + 1, "one History step");
}
