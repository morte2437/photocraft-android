use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;
use crate::menus::invoke as menu;

/// A 40 × 30 layer: the top half at 0.4 gray, the bottom half at 0.5, with the left half selected
/// when `select`.
fn setup(select: bool) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 15})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 15, "width": 40, "height": 15})).unwrap();
    app.run("edit.fill", json!({"color": [0.5, 0.5, 0.5]})).unwrap();
    if select {
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    } else {
        app.run("select.deselect", json!({})).unwrap();
    }
    (app, egui::Context::default())
}

fn gray(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[0]
}

#[test]
fn command_dialog_with_a_selection_opens_equalize_choices() {
    let (mut app, _) = setup(true);
    let id = crate::dialogs::open_command_dialog(&mut app, COMMAND, "Equalize");
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert!(owns(fields));
    assert_eq!(fields[KEY], json!(true));

    let (mut app, _) = setup(false);
    let id = crate::dialogs::open_command_dialog(&mut app, COMMAND, "Equalize");
    assert!(!owns(&app.ui.dialog_mut(id).unwrap().fields));
}

#[test]
fn equalize_marker_is_a_route_marker() {
    assert!(crate::dialogs::is_route_marker(MARK));
}

#[test]
fn equalize_with_a_selection_asks_and_defaults_to_the_selected_area_only() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert!(owns(f));
    assert_eq!(f[KEY], json!(true));
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01, "nothing changes before OK");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(gray(&app, 1, 1) < 0.05 && gray(&app, 1, 20) > 0.95, "the selected area is equalized");
    assert!((gray(&app, 30, 1) - 0.4).abs() < 0.01 && (gray(&app, 30, 20) - 0.5).abs() < 0.01, "the rest is unchanged");
}

#[test]
fn entire_image_based_on_the_selected_area_maps_the_whole_layer() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(d).unwrap().fields.insert(KEY.into(), json!(false));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(gray(&app, 30, 1) < 0.05 && gray(&app, 30, 20) > 0.95, "outside the selection too");
}

/// OK on a dialog opened for one document, while another document is active, equalizes nothing and
/// leaves the dialog open.
#[test]
fn ok_refuses_while_another_document_is_active() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_err());
    assert!(app.ui.dialogs.iter().any(|x| x.id == d), "the dialog stays open");
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01, "the other document is unchanged");
    assert!(app.session.set_active(0));
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01, "nothing ran on the dialog's document either");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(gray(&app, 1, 1) < 0.05, "OK runs once its document is active again");
}

/// OK on a dialog whose selection was cleared while it was open equalizes nothing and leaves the
/// dialog open, so the choice is never applied to the whole layer.
#[test]
fn ok_refuses_after_the_selection_is_cleared() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.run("select.deselect", json!({})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_err());
    assert!(app.ui.dialogs.iter().any(|x| x.id == d), "the dialog stays open");
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01 && (gray(&app, 30, 20) - 0.5).abs() < 0.01, "the layer is unchanged");
}

/// OK after another layer became active equalizes nothing and leaves the dialog open; it runs once
/// the layer the dialog was opened for is active again.
#[test]
fn ok_refuses_after_the_active_layer_changes() {
    let (mut app, ctx) = setup(true);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_err());
    assert!(app.ui.dialogs.iter().any(|x| x.id == d), "the dialog stays open");
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01, "the new layer is unchanged");
    app.run("layer.select", json!({"layer": layer.0})).unwrap();
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01, "nothing ran on the dialog's layer either");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(gray(&app, 1, 1) < 0.05 && gray(&app, 1, 20) > 0.95, "OK runs once its layer is active again");
}

/// OK after the selection changed equalizes nothing and leaves the dialog open; it runs once the
/// selection is the one the dialog was opened with again.
#[test]
fn ok_refuses_after_the_selection_changes() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 30})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, d).is_err());
    assert!(app.ui.dialogs.iter().any(|x| x.id == d), "the dialog stays open");
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01 && (gray(&app, 30, 1) - 0.4).abs() < 0.01, "the layer is unchanged");
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(gray(&app, 1, 1) < 0.05 && (gray(&app, 30, 1) - 0.4).abs() < 0.01, "OK runs on the selection it was opened with");
}

#[test]
fn cancel_leaves_the_layer_unchanged() {
    let (mut app, ctx) = setup(true);
    let d = menu(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    crate::dialogs::cancel(&mut app, d).unwrap();
    assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01 && (gray(&app, 1, 20) - 0.5).abs() < 0.01);
}

#[test]
fn without_a_selection_or_with_params_equalize_runs_at_once() {
    let (mut app, ctx) = setup(false);
    let r = menu(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert!(r.get("dialog").is_none() && app.ui.dialogs.is_empty(), "{r}");
    assert!(gray(&app, 30, 1) < 0.05 && gray(&app, 30, 20) > 0.95);
    let (mut scripted, ctx) = setup(true);
    menu(&mut scripted, &ctx, COMMAND, json!({"selectedAreaOnly": false})).unwrap();
    assert!(scripted.ui.dialogs.is_empty());
    assert!(gray(&scripted, 30, 1) < 0.05, "a script's `selectedAreaOnly` runs without the dialog");
}

#[test]
fn the_dialog_shows_both_choices() {
    let (mut app, _) = setup(true);
    open(&mut app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::show(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    assert!(h.query_by_label("Equalize Selected Area Only").is_some());
    h.get_by_label("Equalize Entire Image Based On Selected Area").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs[0].fields[KEY], json!(false));
    assert!(h.query_by_label("OK").is_some() && h.query_by_label("Cancel").is_some());
}
