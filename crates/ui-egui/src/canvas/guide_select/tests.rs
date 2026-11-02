use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};
use crate::canvas_tool_menu::{GUIDE_CLEAR, GUIDE_EDIT};
use crate::state::Tool;

/// A 200 × 100 document with the Move tool, a vertical guide at 50 and a magenta horizontal one
/// at 30.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30, "color": "magenta"})).unwrap();
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.vertical.clone(), g.horizontal.clone())
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_click_selects_a_guide_without_moving_it_and_a_press_elsewhere_deselects() {
    let mut app = app();
    let steps = past(&app);
    click(&mut app, 50.0, 70.0);
    assert_eq!(selected(&app), Some((true, 0)));
    assert!(is_selected(&app, true, 50.0) && !is_selected(&app, false, 30.0));
    assert_eq!((guides(&app), past(&app)), ((vec![50.0], vec![30.0]), steps), "a click is not a move");
    // One guide at a time: clicking another selects it instead.
    click(&mut app, 120.0, 30.0);
    assert_eq!(selected(&app), Some((false, 0)));
    click(&mut app, 120.0, 70.0);
    assert_eq!(selected(&app), None);
    // A press with another tool deselects too.
    click(&mut app, 50.0, 70.0);
    app.ui.tool = Tool::RectMarquee;
    tool_event(&mut app, ToolEvent::Down { x: 150.0, y: 80.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(selected(&app), None);
}

#[test]
fn a_moved_guide_stays_selected_and_dragging_one_off_the_canvas_deletes_it() {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 70.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 70.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 70.0 }, Modifiers::NONE);
    assert_eq!(guides(&app).0, vec![80.0]);
    assert_eq!(selected(&app), Some((true, 0)));
    tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 70.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: -20.0, y: 70.0 }, Modifiers::NONE);
    assert!(guides(&app).0.is_empty());
    assert_eq!(selected(&app), None);
}

#[test]
fn the_selection_lapses_when_its_guide_goes_or_guides_are_hidden_or_locked() {
    let mut app = app();
    click(&mut app, 50.0, 70.0);
    app.ui.extras.lock_guides = true;
    assert_eq!(selected(&app), None, "locked guides can't be selected");
    app.ui.extras.lock_guides = false;
    app.ui.extras.guides = false;
    assert_eq!(selected(&app), None, "nor hidden ones");
    app.ui.extras.guides = true;
    assert_eq!(selected(&app), Some((true, 0)));
    app.run("view.clearGuides", json!({})).unwrap();
    assert_eq!(selected(&app), None);
    app.session.undo();
    assert_eq!(selected(&app), Some((true, 0)), "undo brings the guide back, still selected");
    // Another document's guide at the same place isn't the selected one.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
    assert_eq!(selected(&app), None);
}

#[test]
fn delete_removes_the_selected_guide_and_leaves_the_pixels() {
    let mut app = app();
    app.run("select.all", json!({})).unwrap();
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, key| {
        let modifiers = Modifiers::NONE;
        let raw = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    click(&mut app, 120.0, 30.0);
    press(&mut app, Key::Delete);
    assert_eq!(guides(&app), (vec![50.0], vec![]));
    assert_eq!(selected(&app), None);
    click(&mut app, 50.0, 70.0);
    press(&mut app, Key::Backspace);
    assert_eq!(guides(&app), (vec![], vec![]));
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo_label(), Some("Delete Guide"), "Edit › Clear didn't run");
    assert!(st.doc.selection.is_some());
    // One undoable step per guide.
    app.session.undo();
    assert_eq!(guides(&app), (vec![50.0], vec![]));
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::canvas_tool_menu::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// The vertical guide through the middle of the document, on screen: the view centres the image.
fn guide_on_screen(h: &Harness<'static, PhotocraftApp>) -> Pos2 {
    h.state().last_canvas_rect.center() + vec2(0.0, 20.0)
}

fn centred_guide_app() -> PhotocraftApp {
    let mut app = app();
    app.run("view.clearGuides", json!({})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 100})).unwrap();
    app
}

#[test]
fn right_click_on_a_guide_selects_it_and_opens_photoshops_guide_menu() {
    let mut h = harness(centred_guide_app());
    let at = guide_on_screen(&h);
    right_click(&mut h, at);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("the guide menu");
    assert!(menu.guide);
    assert_eq!(selected(h.state()), Some((true, 0)));
    let labels: Vec<Option<&str>> = crate::canvas_tool_menu::rows(&menu).iter().map(|r| r.map(|(label, _)| label)).collect();
    assert_eq!(
        labels,
        [
            Some("Edit Selected Guides"),
            None,
            Some("Lock Guides"),
            Some("Clear Guides"),
            Some("Clear Canvas Guides"),
            Some("Clear Selected Guides"),
            Some("Clear Selected Artboard Guides"),
        ]
    );
    let app = h.state();
    for id in [GUIDE_EDIT, GUIDE_CLEAR, "view.lockGuides", "view.clearGuides", "view.clearCanvasGuides"] {
        assert!(crate::canvas_tool_menu::available(app, &menu, id), "{id}");
    }
    assert!(!crate::canvas_tool_menu::available(app, &menu, "view.clearSelectedArtboardGuides"), "no artboard");
    assert!(!crate::canvas_tool_menu::available(app, &menu, "edit.freeTransform"), "not a guide row");
    // Off a guide, the Move tool's right-click is still the layer picker's.
    h.key_press(Key::Escape);
    h.run_steps(2);
    let off = h.state().last_canvas_rect.center() + vec2(-60.0, 20.0);
    right_click(&mut h, off);
    assert!(h.state().ui.canvas_tool_menu.is_none());
}

#[test]
fn guide_menu_rows_clear_the_selected_guide_and_lock_guides() {
    let mut h = harness(centred_guide_app());
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    let at = guide_on_screen(&h);
    right_click(&mut h, at);
    let ctx = h.ctx.clone();
    crate::canvas_tool_menu::choose(h.state_mut(), &ctx, GUIDE_CLEAR);
    assert_eq!(guides(h.state()).0, vec![20.0], "only the right-clicked guide goes");
    h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 100})).unwrap();
    h.run_steps(2);
    let at = guide_on_screen(&h);
    right_click(&mut h, at);
    crate::canvas_tool_menu::choose(h.state_mut(), &ctx, "view.lockGuides");
    assert!(h.state().ui.extras.lock_guides);
    assert_eq!(selected(h.state()), None, "locked guides aren't selected");
}

#[test]
fn edit_selected_guides_opens_new_guide_on_the_guide_and_applies_the_edit() {
    let mut app = app();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
    click(&mut app, 120.0, 30.0);
    let id = crate::new_guide_ui::open_edit(&mut app).unwrap();
    let f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    assert_eq!((f["orientation"].as_str(), f["position"].as_str(), f["color"].as_str()), (Some("horizontal"), Some("30 %"), Some("magenta")));
    assert_eq!(f["__label"], "Edit Selected Guides");
    // Turn it vertical at 30 % of the width; the colour isn't touched, so it keeps its own.
    let mut edited = f.clone();
    edited.insert("orientation".into(), json!("vertical"));
    edited.insert("position".into(), json!("30"));
    app.ui.dialogs.retain(|d| d.id != id);
    crate::new_guide_ui::confirm(&mut app, &edited).unwrap();
    assert_eq!(guides(&app), (vec![50.0, 60.0], vec![]));
    let g = &app.session.active().unwrap().doc.guides;
    assert_eq!((g.color(true, 60.0), g.color(false, 30.0)), (Some([0xff, 0x00, 0xff]), None));
    assert_eq!(selected(&app), Some((true, 1)), "the edited guide stays selected");
}

#[test]
fn editing_an_uncoloured_guide_keeps_the_preference_colour_unless_one_is_chosen() {
    let mut app = app();
    click(&mut app, 50.0, 70.0);
    let id = crate::new_guide_ui::open_edit(&mut app).unwrap();
    let f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    // The default preference colour is Cyan's, so a guide dragged from a ruler shows as Cyan.
    assert_eq!(f["color"].as_str(), Some("cyan"));
    let params = crate::new_guide_ui::params(&app, &f);
    assert!(params.get("color").is_none(), "{params}");
    let mut yellow = f.clone();
    yellow.insert("color".into(), json!("yellow"));
    assert_eq!(crate::new_guide_ui::params(&app, &yellow)["color"], "yellow");
    // A refused position keeps the dialog open with the message and the guide untouched.
    let mut bad = f.clone();
    bad.insert("position".into(), json!("far"));
    app.ui.dialogs.retain(|d| d.id != id);
    assert!(crate::new_guide_ui::confirm(&mut app, &bad).is_err());
    assert_eq!(guides(&app).0, vec![50.0]);
    assert!(app.ui.dialogs.last().is_some_and(|d| d.fields.contains_key("__error")));
    // A preference colour that isn't a named one shows as Custom….
    app.ui.dialogs.clear();
    app.run("prefs.set", json!({"path": "guidesGridAndSlices.guideColor", "value": "#123456"})).unwrap();
    let id = crate::new_guide_ui::open_edit(&mut app).unwrap();
    let f = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("custom"), Some("#123456")));
}

#[test]
fn edit_selected_guides_needs_a_selected_guide() {
    let mut app = app();
    assert!(crate::new_guide_ui::open_edit(&mut app).is_err());
    assert!(app.ui.dialogs.is_empty());
}

/// ⌥-click turns a guide through the point clicked, keeping its colour, as one undoable step, and
/// leaves it selected; a locked guide can't be turned.
#[test]
fn alt_click_turns_a_guide_through_the_point_clicked() {
    let mut app = app();
    let alt = Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 30.0 }, alt);
    assert_eq!(guides(&app), (vec![50.0, 120.0], vec![]), "the magenta horizontal guide is now vertical at x = 120");
    let g = &app.session.active().unwrap().doc.guides;
    assert_eq!(g.color(true, 120.0), Some([0xff, 0x00, 0xff]));
    assert_eq!(selected(&app), Some((true, 1)));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Edit Guide"));
    app.session.undo();
    assert_eq!(guides(&app), (vec![50.0], vec![30.0]));
    app.ui.extras.lock_guides = true;
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 70.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 70.0 }, alt);
    assert_eq!(guides(&app), (vec![50.0], vec![30.0]), "locked guides stay put");
}

/// Double-clicking a guide opens Preferences › Guides, Grid & Slices, as in Photoshop.
#[test]
fn double_click_on_a_guide_opens_the_guide_preferences() {
    let mut h = harness(centred_guide_app());
    let at = guide_on_screen(&h);
    h.hover_at(at);
    for _ in 0..40 {
        h.step();
    }
    for _ in 0..2 {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    let dialog = h.state().ui.dialogs.last().map(|d| serde_json::to_string(&d.fields).unwrap_or_default()).unwrap_or_default();
    assert!(dialog.contains("guidesGridAndSlices"), "{dialog}");
    assert_eq!(guides(h.state()).0, vec![100.0], "the guide didn't move");
}
