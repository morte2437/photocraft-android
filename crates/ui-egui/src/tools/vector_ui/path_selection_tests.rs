//! #306: ⌘↩ (Ctrl+Enter off the Mac) loads a path as a selection, as in Photoshop: the path
//! being drawn with the Pen, the path selected in the Paths panel, or the work path. The key is
//! rebindable in Edit › Keyboard Shortcuts, and with no path it says why instead of doing nothing.

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::vector_ui::PenPath;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(6);
    h
}

/// ⌘ on the Mac, Ctrl elsewhere, as egui-winit reports them.
fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers { command: true, mac_cmd: true, ..Default::default() }
    } else {
        Modifiers { command: true, ctrl: true, ..Default::default() }
    }
}

fn press(h: &mut Harness<'_, PhotocraftApp>, key: Key, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> serde_json::Value {
    let k = |x: f64, y: f64| json!({"anchor": [x, y]});
    json!({"subpaths": [{"closed": true, "knots": [k(x, y), k(x + w, y), k(x + w, y + h), k(x, y + h)]}]})
}

/// The selection's bounds (x, y, width, height), if any.
fn selection(h: &Harness<'_, PhotocraftApp>) -> Option<[i64; 4]> {
    let r = h.state().session.active()?.doc.selection.as_ref()?.content_bounds();
    Some([r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0].map(i64::from))
}

#[test]
fn cmd_enter_with_the_pen_loads_the_work_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "⌘↩ made a selection: {}", h.state().ui.status);
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
}

#[test]
fn cmd_enter_while_drawing_finishes_the_path_and_loads_it() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let p = |x: f64, y: f64| [[x, y], [x, y], [x, y]];
    let owner = crate::vector_ui::capture_pen_owner(h.state());
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], owner, ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.ui.pen.is_none(), "the pen path was finished");
    assert!(app.session.active().unwrap().doc.work_path.is_some(), "into the work path");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "and loaded as a selection: {}", app.ui.status);
    // Then deselected and no longer drawn, as in Photoshop; the work path stays (#6079).
    assert_eq!(app.ui.selected_path, None);
    assert!(!crate::vector_ui::work_path_shown(app));
    // Plain ↩ still only finishes a path.
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let owner = crate::vector_ui::capture_pen_owner(h.state());
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], owner, ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some());
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

#[test]
fn selecting_a_saved_path_keeps_the_hidden_work_path_hidden() {
    // #6079 review: only selecting the Work Path in the Paths panel shows it again.
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "Saved", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    let p = |x: f64, y: f64| [[x, y], [x, y], [x, y]];
    let owner = crate::vector_ui::capture_pen_owner(h.state());
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], owner, ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert!(!crate::vector_ui::work_path_shown(h.state()), "⌘↩ hid the work path");
    panel(&mut h, 2);
    let saved = h.get_by_label("Saved").rect();
    click(&mut h, egui::pos2(saved.left() + 70.0, saved.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"));
    assert!(!crate::vector_ui::work_path_shown(h.state()), "a saved path doesn't show the Work Path");
    // So Delete with the Pen doesn't delete the invisible work path either.
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the hidden work path survives Delete");
    // Selecting the Work Path shows it; now Delete deletes it.
    let work = h.get_by_label("Work Path").rect();
    click(&mut h, egui::pos2(work.left() + 70.0, work.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(crate::vector_ui::work_path_shown(h.state()), "selecting the Work Path shows it");
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "{}", h.state().ui.status);
}

#[test]
fn reselecting_the_hidden_work_path_ends_the_hide() {
    // #6079 review: once the Work Path is selected again in the Paths panel, deselecting it there
    // leaves an ordinary unselected Work Path (drawn and edited with a path tool, #6551), not the
    // hidden one from before.
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let p = |x: f64, y: f64| [[x, y], [x, y], [x, y]];
    let owner = crate::vector_ui::capture_pen_owner(h.state());
    h.state_mut().ui.pen = Some(PenPath { knots: vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 80.0)], owner, ..Default::default() });
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert!(!crate::vector_ui::work_path_shown(h.state()), "⌘↩ hid the work path");
    panel(&mut h, 2);
    let work = h.get_by_label("Work Path").rect();
    click(&mut h, egui::pos2(work.left() + 70.0, work.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    // Clicking the panel's empty area deselects it.
    click(&mut h, egui::pos2(work.left() + 70.0, work.bottom() + 40.0), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path, None);
    assert!(crate::vector_ui::work_path_shown(h.state()), "the unchanged Work Path didn't go back into hiding");
    assert_eq!(crate::vector_ui::active_path_name(h.state()).as_deref(), Some("work"));
    // So Delete with the Pen deletes it, like any unselected Work Path.
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "{}", h.state().ui.status);
}

#[test]
fn cmd_enter_loads_the_path_selected_in_the_paths_panel() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    // Any tool, with a path selected in the Paths panel (as in Photoshop).
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]), "{}", h.state().ui.status);
    // A selected path that went away falls back to the work path once that is selected again.
    h.state_mut().run("path.delete", json!({"name": "Path 1"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([0, 0, 30, 30]));
}

#[test]
fn cmd_enter_on_a_saved_path_leaves_no_target_path() {
    // #6079: after loading a saved path nothing is the target path; the shown Work Path must not
    // take over (a second ⌘↩ or the canvas menu's Delete Path would act on it).
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]), "{}", h.state().ui.status);
    assert_eq!(h.state().ui.selected_path, None);
    assert_eq!(crate::vector_ui::active_path_name(h.state()), None, "the Work Path didn't become the target path");
    assert!(!crate::vector_ui::work_path_shown(h.state()), "the Work Path isn't drawn");
    // A second ⌘↩ has no path to load.
    h.state_mut().run("select.deselect", json!({})).unwrap();
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), None, "the unselected Work Path wasn't loaded");
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the Work Path stays");
}

#[test]
fn without_a_path_it_says_why_and_the_key_can_be_rebound() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status_error && app.ui.status.contains("No path"), "status: {}", app.ui.status);
    // It's a listed, rebindable shortcut.
    let cmd_enter = crate::shortcuts::parse("Cmd+Enter").unwrap();
    assert!(crate::shortcut_dispatch::bindings(app).iter().any(|(id, sc)| id == "path.toSelection" && *sc == cmd_enter));
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"path.toSelection": "Cmd+Shift+Y"}})).unwrap();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 20.0, 20.0)})).unwrap();
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "⌘↩ was moved");
    let mut m = command();
    m.shift = true;
    press(&mut h, Key::Y, m);
    assert_eq!(selection(&h), Some([10, 10, 20, 20]), "{}", h.state().ui.status);
}

fn panel(h: &mut Harness<'_, PhotocraftApp>, tab: usize) {
    let ctx = h.ctx.clone();
    let (req, _) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":"promedium", "dockTabs":{"layers":tab}, "dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.event(Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn thumbnail_selection_ctrl_click_shape_keeps_active_layer_and_supports_undo() {
    let mut h = harness();
    let shape =
        h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap()["layer"].as_u64().unwrap();
    let active = h.state_mut().run("layer.new.layer", json!({"name":"Other"})).unwrap()["layer"].as_u64().unwrap();
    panel(&mut h, 0);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == shape).unwrap();
    click(&mut h, row.name.unwrap().center(), command());
    assert!(selection(&h).is_none(), "Ctrl-click on a shape's name only selects the layer");
    h.state_mut().run("layer.select", json!({"layer": active})).unwrap();
    h.run_steps(2);
    click(&mut h, egui::pos2(row.row.left() + 46.0, row.row.center().y), command());
    assert_eq!(selection(&h), Some([40, 20, 60, 40]), "{}", h.state().ui.status);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(active)));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_targets_thumbnail_and_clicked_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name":"Saved","path":rect_path(100.0,40.0,60.0,50.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    panel(&mut h, 2);
    let row = h.get_by_label("Work Path").rect();
    click(&mut h, egui::pos2(row.left() + 70.0, row.center().y), command());
    assert!(selection(&h).is_none(), "Ctrl-click on the name only selects the path row");
    h.state_mut().ui.selected_path = Some("Saved".into());
    h.run_steps(2);
    click(&mut h, egui::pos2(row.left() + 22.0, row.center().y), command());
    assert_eq!(selection(&h), Some([10, 10, 50, 40]), "{}", h.state().ui.status);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"), "loading a thumbnail does not change the selected path");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
    h.state_mut().run("select.deselect", json!({})).unwrap();
    let saved = h.get_by_label("Saved").rect();
    click(&mut h, egui::pos2(saved.left() + 22.0, saved.center().y), command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_combines_with_shift_and_alt() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    panel(&mut h, 2);
    let row = h.get_by_label("Work Path").rect();
    let at = egui::pos2(row.left() + 22.0, row.center().y);
    h.state_mut().run("select.rect", json!({"x":100,"y":40,"width":20,"height":20})).unwrap();
    click(&mut h, at, command() | Modifiers::SHIFT);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) > 0.99 && selected.sample_channel(110, 50, 0) > 0.99);
    h.state_mut().run("select.all", json!({})).unwrap();
    click(&mut h, at, command() | Modifiers::ALT);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) < 0.01 && selected.sample_channel(110, 50, 0) > 0.99);
    h.state_mut().run("select.rect", json!({"x":30,"y":0,"width":100,"height":100})).unwrap();
    click(&mut h, at, command() | Modifiers::SHIFT | Modifiers::ALT);
    assert_eq!(selection(&h), Some([30, 10, 30, 40]));
}

#[test]
fn thumbnail_selection_ctrl_click_layer_path_preserves_panel_target() {
    let mut h = harness();
    h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(0.0,0.0,10.0,10.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    panel(&mut h, 2);
    let row = h.get_by_label("Badge Shape Path").rect();
    click(&mut h, egui::pos2(row.left() + 22.0, row.center().y), command());
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target);
    if let Ok(path) = std::env::var("PHOTOCRAFT_THUMBNAIL_SELECTION_SCREENSHOT") {
        h.render().unwrap().save(path).unwrap();
    }
}

/// #3915: Clicking a shape with the Path Selection tool selects its layer.
#[test]
fn clicking_shape_with_path_selection_selects_its_layer() {
    let mut h = harness();
    let rect_layer =
        h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [20, 20, 80, 60], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    // Add a new empty layer above so rect is not the active layer.
    let empty_layer = h.state_mut().run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, empty_layer);

    // Clicking inside the rectangle with Path Selection selects the rect layer.
    h.state_mut().ui.tool = Tool::PathSelection;
    crate::vector_ui::path_selection_finish(h.state_mut(), [50.0, 50.0], [50.0, 50.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);

    // Clicking outside does not change active layer.
    crate::vector_ui::path_selection_finish(h.state_mut(), [150.0, 90.0], [150.0, 90.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);
}

/// A work path selected in the Paths panel remains visible (outline) regardless of the active tool
/// (one that is not selected is hidden, #6551). Before the fix,
/// `draw_overlay` was gated on `vector_tool` so paths disappeared when switching to Brush, Move,
/// etc. This test renders the canvas with a Brush tool active and a work path present, then
/// confirms the path's outline pixels are painted in the accent colour, proving the path is drawn.
#[test]
fn path_overlay_drawn_for_non_vector_tools() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    // Place a work path whose top edge runs exactly along y=10 in document space.
    app.run("path.set", json!({"name": "work", "path": rect_path(50.0, 10.0, 100.0, 80.0)})).unwrap();
    app.ui.selected_path = Some("work".into());
    // Switch to a non-vector tool — before the fix this hid the path outline.
    app.ui.tool = Tool::Brush;

    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(6);
    // Fit the view so document coordinates map predictably to screen pixels.
    {
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = 2.0; // 2 px/doc-px → the 200×200 doc fills 400×400 screen pixels
        v.center = [100.0, 100.0];
        v.fit_pending = false;
    }
    h.run_steps(3);

    // Get the accent colour so we know what to look for.
    let accent = crate::theme::Tokens::get(&h.ctx).accent;

    let xf = crate::canvas::ViewXform::active(h.state()).expect("active document view");
    let ppp = h.ctx.pixels_per_point();
    let img = h.render().unwrap();

    // Sample the rendered outline at points derived from the same transform the canvas used. The
    // top edge is doc-y=10; x=70..130 stays away from the rectangle's anchor squares. A 1 px
    // egui stroke is anti-aliased over the white canvas, so accept visible blends of the accent.
    let accent_r = accent.r();
    let accent_g = accent.g();
    let accent_b = accent.b();
    let is_accent_stroke = |pixel: [u8; 4]| {
        let denom = f32::from(255u8.saturating_sub(accent_r)).max(1.0);
        let coverage = f32::from(255u8.saturating_sub(pixel[0])) / denom;
        if !(0.2..=1.05).contains(&coverage) {
            return false;
        }
        let blended = |channel: u8| 255.0 + coverage * (f32::from(channel) - 255.0);
        (f32::from(pixel[1]) - blended(accent_g)).abs() <= 12.0 && (f32::from(pixel[2]) - blended(accent_b)).abs() <= 12.0
    };

    let has_accent_near = |at: egui::Pos2| {
        let cx = (at.x * ppp).round() as i32;
        let cy = (at.y * ppp).round() as i32;
        let radius = (2.0 * ppp).ceil() as i32;
        for y in (cy - radius)..=(cy + radius) {
            for x in (cx - radius)..=(cx + radius) {
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    let pixel = img.get_pixel(x as u32, y as u32);
                    if is_accent_stroke([pixel[0], pixel[1], pixel[2], pixel[3]]) {
                        return true;
                    }
                }
            }
        }
        false
    };
    let hits = (70..=130).step_by(5).filter(|x| has_accent_near(xf.to_screen(*x as f32, 10.0))).count();
    assert!(
        hits >= 10,
        "expected the work path outline to be visible with Brush tool active, \
         but found accent-coloured pixels at only {hits} sample point(s) along the transformed top edge \
         (accent = #{accent_r:02x}{accent_g:02x}{accent_b:02x}); \
         the path must be drawn regardless of the active tool",
    );
}

#[test]
fn thumbnail_selection_after_hiding_extras_restores_outline() {
    let mut h = harness();
    let shape =
        h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap()["layer"].as_u64().unwrap();
    let active = h.state_mut().run("layer.new.layer", json!({"name":"Other"})).unwrap()["layer"].as_u64().unwrap();
    panel(&mut h, 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
    assert!(!h.state().ui.view.extras);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == shape).unwrap();
    click(&mut h, row.name.unwrap().center(), command());
    assert!(!h.state().ui.view.extras, "selecting layers must not reveal Extras");
    h.state_mut().run("layer.select", json!({"layer": active})).unwrap();
    h.run_steps(2);
    click(&mut h, egui::pos2(row.row.left() + 46.0, row.row.center().y), command());
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
    assert!(h.state().ui.view.shows(h.state().ui.view.show.selection_edges), "Ctrl-clicking the thumbnail reveals the new selection");
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(active)));
}

#[test]
fn loading_a_path_after_hiding_extras_restores_outline() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
    assert!(!h.state().ui.view.extras);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
    assert!(h.state().ui.view.shows(h.state().ui.view.show.selection_edges));
}

/// #6079 review: in an arranged view, the frame a click into another tile activates its document
/// still draws the old active document's overlays. The Work Path Make Selection hid there must not
/// flash up in that frame.
#[test]
fn a_hidden_work_path_stays_hidden_when_another_tile_is_clicked() {
    // Path outlines (accent lines) in the right tile in the frame a click into the left tile makes.
    let right_outlines_on_switch = |hide: bool| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..2 {
            app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        }
        app.sync_views();
        app.ui.view.arrange = "tileAllVertically".into();
        app.ui.tool = Tool::Pen;
        app.run("path.set", json!({"name": "work", "path": rect_path(40.0, 30.0, 100.0, 80.0)})).unwrap();
        crate::vector_ui::path_to_selection(&mut app, json!({})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        assert!(!crate::vector_ui::work_path_shown(&app), "Make Selection hid the second document's Work Path");
        if !hide {
            app.ui.hidden_work_path = None;
        }
        let mut h = Harness::builder().with_size(vec2(1000.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let area = h.state().last_canvas_rect;
        let at = egui::pos2(area.left() + area.width() / 4.0, area.center().y);
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        assert_eq!(h.state().session.active_index(), Some(0), "the click activated the left document");
        let accent = crate::theme::Tokens::get(&h.ctx).accent;
        let outlines = h.output().shapes.iter().filter(|s| {
            matches!(&s.shape, egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(accent) && p.visual_bounding_rect().min.x > area.center().x)
        });
        outlines.count()
    };
    assert!(right_outlines_on_switch(false) > 0, "a shown Work Path is drawn in that frame");
    assert_eq!(right_outlines_on_switch(true), 0, "the hidden one isn't");
}
