//! #12073: committing a Type Mask always replaces the selection with the text outline, with ⇧,
//! ⌥ or ⇧⌥ held too: a click outside the text, the Commit button and ⌘↩ all go through
//! `commit_with`. `type.createSelection` keeps its `operation` param for scripts.

use egui::Modifiers;
use egui_kittest::Harness;
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const COMBOS: [Modifiers; 4] = [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT, Modifiers { shift: true, alt: true, ..Modifiers::NONE }];

/// A document with a selection away from the text, so Add, Subtract and Intersect would each
/// leave something other than the text outline.
pub(super) fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.run("select.rect", json!({"x": 600, "y": 250, "width": 100, "height": 100})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool_options.type_size = 48.0;
    app.ui.tool = Tool::HorizontalTypeMask;
    app
}

pub(super) fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

/// Start a mask with a click at `at` and type `s`.
fn mask(app: &mut PhotocraftApp, at: (f64, f64), s: &str) {
    mask_with(app, at, s, Modifiers::NONE);
}

/// [`mask`] started by a click holding `mods`.
fn mask_with(app: &mut PhotocraftApp, at: (f64, f64), s: &str, mods: Modifiers) {
    click(app, at.0, at.1, mods);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "typing a mask");
    super::insert(app, s);
}

pub(super) fn selection(app: &PhotocraftApp) -> Option<Surface> {
    app.session.active().unwrap().doc.selection.clone()
}

pub(super) fn bounds(s: &Option<Surface>) -> Option<photocraft_geom::Rect> {
    s.as_ref().map(Surface::content_bounds)
}

/// The selection a plain commit of `s` at `at` leaves: the text outline alone.
fn outline(at: (f64, f64), s: &str) -> Option<Surface> {
    let mut app = new_app();
    mask(&mut app, at, s);
    super::commit_with(&mut app, Modifiers::NONE);
    let outline = selection(&app);
    assert!(bounds(&outline).is_some_and(|r| r.x1 < 600), "the text alone: {:?}", bounds(&outline));
    outline
}

#[test]
fn every_commit_replaces_the_selection_whatever_modifiers_are_held() {
    let outline = outline((100.0, 200.0), "AB");
    for mods in COMBOS {
        let mut app = new_app();
        mask(&mut app, (100.0, 200.0), "AB");
        super::commit_with(&mut app, mods);
        assert!(app.ui.text_edit.is_none(), "{mods:?}");
        assert!(selection(&app) == outline, "{mods:?}: {:?}, not the outline {:?}", bounds(&selection(&app)), bounds(&outline));
    }
}

#[test]
fn a_modified_click_outside_the_text_replaces_the_selection() {
    let outline = outline((100.0, 200.0), "AB");
    for mods in COMBOS {
        let mut app = new_app();
        mask(&mut app, (100.0, 200.0), "AB");
        click(&mut app, 450.0, 60.0, mods);
        assert!(app.ui.text_edit.is_none(), "{mods:?}: the click commits");
        assert!(selection(&app) == outline, "{mods:?}: {:?}", bounds(&selection(&app)));
    }
}

#[test]
fn shift_command_return_replaces_the_selection() {
    let outline = outline((100.0, 200.0), "AB");
    let mut app = new_app();
    mask(&mut app, (100.0, 200.0), "AB");
    let mut h = Harness::builder().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // As `canvas_tests`' harness: the shortcuts wait for the app's fonts.
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::shortcuts::handle(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT });
    h.run_steps(1);
    let app = h.into_state();
    assert!(app.ui.text_edit.is_none());
    assert!(selection(&app) == outline, "{:?}", bounds(&selection(&app)));
}

/// The reference application's "AB", then ⇧-clicks to commit it and to start "C", then "C"
/// committed: only "C" is selected. (Here a click outside the text commits without starting new
/// text, so that is two ⇧-clicks.)
#[test]
fn the_next_mask_after_a_shift_click_commit_replaces_the_first() {
    let second = outline((300.0, 200.0), "C");
    let mut app = new_app();
    mask(&mut app, (100.0, 200.0), "AB");
    click(&mut app, 450.0, 60.0, Modifiers::SHIFT);
    assert!(app.ui.text_edit.is_none(), "the ⇧-click commits");
    mask_with(&mut app, (300.0, 200.0), "C", Modifiers::SHIFT);
    click(&mut app, 450.0, 60.0, Modifiers::SHIFT);
    assert!(selection(&app) == second, "{:?}, not only \"C\" {:?}", bounds(&selection(&app)), bounds(&second));
}

#[test]
fn type_create_selection_still_combines_for_scripts() {
    let mut app = new_app();
    let rect = bounds(&selection(&app)).unwrap();
    app.run("type.createSelection", json!({"text": "AB", "x": 100, "y": 200, "size": 48, "operation": "add"})).unwrap();
    let added = bounds(&selection(&app)).unwrap();
    assert_eq!(added.union(&rect), added, "the rectangle is kept");
    assert!(added.x0 < rect.x0, "and the text is added");
}
