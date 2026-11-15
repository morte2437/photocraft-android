use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::state::DialogKind;
use serde_json::{Map, json};

fn app_with_variables_dialog(cur: u64) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let state = json!({
        "defs": [{
            "name": "v1",
            "layer": 0,
            "type": "visibility",
            "method": "fit",
            "align": "center",
            "clip": false
        }],
        "dataSets": [{"name": "A", "values": []}]
    });
    let mut fields = Map::new();
    fields.insert("__variables".into(), state);
    fields.insert("__page".into(), json!("dataSets"));
    fields.insert("__cur".into(), json!(cur));
    app.ui.open_dialog(DialogKind::Command, fields);
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            photocraft_ui_egui::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, photocraft_ui_egui::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

#[test]
fn out_of_range_data_set_index_does_not_overflow_during_render() {
    let mut h = harness(app_with_variables_dialog(u64::MAX));
    h.run_steps(2);
}

#[test]
fn out_of_range_data_set_index_is_clamped_before_delete() {
    let mut h = harness(app_with_variables_dialog(999));
    h.get_by_label("Delete").click();
    h.run_steps(2);
}

/// A real image file and unsaved dialog edits exercise the same path as manual entry.
fn pixel_replacement_harness() -> (tempfile::TempDir, Harness<'static, PhotocraftApp>) {
    let dir = tempfile::tempdir().unwrap();
    for (name, color) in [("blue.png", "#0000ff"), ("red.png", "#ff0000")] {
        let mut source = photocraft_engine::Session::new();
        source.execute("file.new", json!({"width": 8, "height": 8, "background": color})).unwrap();
        let png = photocraft_io::export(&source.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        std::fs::write(dir.path().join(name), png).unwrap();
    }
    let path = dir.path().join("blue.png");

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    photocraft_ui_egui::menus::invoke(&mut app, &egui::Context::default(), "image.variables.dataSets", json!({})).unwrap();
    let dialog = app.ui.dialogs.last_mut().unwrap();
    dialog.fields.insert(
        "__variables".into(),
        json!({
            "defs": [{"name": "photo", "layer": layer.0, "type": "pixelReplacement", "method": "conform"}],
            "dataSets": [{"name": "Blue", "values": [{"variable": "photo", "kind": "pixels", "value": path}]}]
        }),
    );
    (dir, harness(app))
}

fn save_draft_and_open_apply_menu(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
    h.run_steps(3);
}

fn read_pixel(h: &mut Harness<'static, PhotocraftApp>) -> serde_json::Value {
    h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap()
}

/// CSV chosen through the import menu supplies the row that Apply renders and OK preserves.
#[test]
fn imported_csv_replaces_pixels_through_the_apply_dialog() {
    use photocraft_ui_egui::file_dialog::{FileDialogAnswer, FileDialogRequest};

    for contents in [false, true] {
        let (dir, mut h) = pixel_replacement_harness();
        h.get_by_label("OK").click();
        h.run_steps(3);
        h.state_mut().run("image.variables.dataSets", json!({"dataSets": []})).unwrap();
        let csv = format!("DataSet,photo\nImported,\"{}\"\n", dir.path().join("blue.png").display());
        let path = dir.path().join("sets.csv");
        std::fs::write(&path, &csv).unwrap();
        let answer = if contents {
            FileDialogAnswer::Contents("sets.csv".into(), csv.into_bytes())
        } else {
            FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()])
        };
        h.state_mut().services.file_dialog = Some(Box::new(move |request, _, reply| {
            assert!(matches!(request, FileDialogRequest::Open { multiple: false, .. }));
            reply.send(Some(answer.clone()));
        }));
        let ctx = h.ctx.clone();
        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "file.import.variableDataSets", json!({})).unwrap();
        h.state_mut().poll_file_dialog(&ctx, None);
        assert!(!h.state().file_dialog_open());
        let rows = &h.state().session.active().unwrap().doc.variables.data_sets;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Imported");
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "import alone does not apply a row");

        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
        h.run_steps(3);
        h.get_by_label("Apply").click();
        h.run_steps(3);
        assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(h.state().ui.dialogs.len(), 1);
        let revision = h.state().session.active().unwrap().revision;
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
    }
}

#[test]
fn apply_menu_ok_requires_a_row_but_not_variable_definitions() {
    for (rows, explicit, expected) in [(json!([]), false, None), (json!([{"name": "Empty", "values": []}]), false, Some(0)), (json!([]), true, None)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        app.run("image.variables.dataSets", json!({"dataSets": rows})).unwrap();
        photocraft_ui_egui::menus::invoke(&mut app, &egui::Context::default(), "image.applyDataSet", json!({})).unwrap();
        if explicit {
            app.ui.dialogs.last_mut().unwrap().fields.insert("__apply".into(), json!(0));
        }
        let mut h = harness(app);
        h.state_mut().ui.status.clear();
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().doc.variables.active, expected);
        if explicit {
            assert!(h.state().ui.status.contains("no data sets in document"));
        } else {
            assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
        }
    }
}

/// Apply updates the canvas immediately; OK must not apply the same request again.
#[test]
fn apply_replaces_pixels_before_ok_and_keeps_the_dialog_open() {
    let (_dir, mut h) = pixel_replacement_harness();
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__apply".into(), json!("Blue"));
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "editing fields alone is not Apply");
    h.get_by_label("Apply").click();
    h.run_steps(3);

    assert_eq!(h.state().ui.dialogs.last().unwrap().id, id, "Apply leaves the editor open");
    assert!(!h.state().ui.dialogs.last().unwrap().fields.contains_key("__apply"));
    let pixel = read_pixel(&mut h);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let confirmed = read_pixel(&mut h);
    assert_eq!(confirmed, json!([0.0, 0.0, 1.0, 1.0]), "the same replacement works through OK");
    assert_eq!(pixel, confirmed, "Apply uses the edited definitions and image path immediately");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

/// Duplicate labels must not change which row Apply or OK sends to the engine.
#[test]
fn duplicate_names_apply_the_selected_row_for_apply_and_ok() {
    for button in ["Apply", "OK"] {
        let (_dir, mut h) = pixel_replacement_harness();
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        let selected = fields["__variables"]["dataSets"][0].clone();
        fields["__variables"]["dataSets"] = json!([{"name": "Blue", "values": []}, selected]);
        save_draft_and_open_apply_menu(&mut h);
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "editing data sets only saves them");
        h.get_by_label("▶").click();
        h.run_steps(2);
        let fields = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["__cur"], 1);
        assert!(!fields.contains_key("__apply"));
        h.get_by_label(button).click();
        h.run_steps(3);
        assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]), "{button}");
        assert_eq!(h.state().session.active().unwrap().doc.variables.active, Some(1), "{button}");
    }
}

#[test]
fn apply_menu_apply_then_ok_keeps_one_history_entry() {
    let (_dir, mut h) = pixel_replacement_harness();
    save_draft_and_open_apply_menu(&mut h);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
    let revision = h.state().session.active().unwrap().revision;
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn apply_menu_ok_honors_changes_after_apply() {
    for change in ["row", "value", "document"] {
        let (dir, mut h) = pixel_replacement_harness();
        let red = json!(dir.path().join("red.png"));
        if change == "row" {
            let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
            let mut row = fields["__variables"]["dataSets"][0].clone();
            row["values"][0]["value"] = red.clone();
            fields["__variables"]["dataSets"].as_array_mut().unwrap().push(row);
        }
        save_draft_and_open_apply_menu(&mut h);
        h.get_by_label("Apply").click();
        h.run_steps(3);
        if change == "row" {
            h.get_by_label("▶").click();
        } else if change == "value" {
            h.state_mut().ui.dialogs.last_mut().unwrap().fields["__variables"]["dataSets"][0]["values"][0]["value"] = red;
        } else {
            h.state_mut().run("image.adjustments.invert", json!({})).unwrap();
        }
        h.run_steps(3);
        let before = if change == "document" { json!([1.0, 1.0, 0.0, 1.0]) } else { json!([0.0, 0.0, 1.0, 1.0]) };
        assert_eq!(read_pixel(&mut h), before, "{change}: only document edits change pixels before OK");
        h.get_by_label("OK").click();
        h.run_steps(3);
        let after = if change == "document" { json!([0.0, 0.0, 1.0, 1.0]) } else { json!([1.0, 0.0, 0.0, 1.0]) };
        assert_eq!(read_pixel(&mut h), after, "{change}");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(read_pixel(&mut h), before);
    }
}

#[test]
fn apply_menu_cancel_does_not_apply_the_selected_row() {
    let (_dir, mut h) = pixel_replacement_harness();
    save_draft_and_open_apply_menu(&mut h);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn apply_menu_ok_after_deleting_the_last_row_only_saves() {
    let (_dir, mut h) = pixel_replacement_harness();
    save_draft_and_open_apply_menu(&mut h);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    h.state_mut().ui.status.clear();
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
    assert!(h.state().session.active().unwrap().doc.variables.data_sets.is_empty());
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn legacy_named_request_stays_draft_until_ok() {
    let (_dir, mut h) = pixel_replacement_harness();
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__apply".into(), json!("Blue"));
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
}

#[test]
fn variables_editor_ok_only_saves_configuration() {
    for menu in ["image.variables.define", "image.variables.dataSets"] {
        let (_dir, mut h) = pixel_replacement_harness();
        let state = h.state().ui.dialogs.last().unwrap().fields["__variables"].clone();
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        let ctx = h.ctx.clone();
        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, menu, json!({})).unwrap();
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__variables".into(), state);
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(h.state().session.active().unwrap().doc.variables.data_sets.len(), 1, "{menu}");
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "{menu}");
    }
}

/// A failed request reports once; correcting it can be applied and kept when Cancel closes.
#[test]
fn failed_apply_can_be_corrected_without_retries_or_cancel_rollback() {
    for apply_mode in [false, true] {
        let (dir, mut h) = pixel_replacement_harness();
        if apply_mode {
            save_draft_and_open_apply_menu(&mut h);
        }
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        fields["__variables"]["dataSets"][0]["values"][0]["value"] = json!(dir.path().join("missing.png"));
        h.get_by_label("Apply").click();
        h.run_steps(3);
        assert!(h.state().ui.status.contains("missing.png"), "{}", h.state().ui.status);
        assert!(!h.state().ui.dialogs.last().unwrap().fields.contains_key("__apply"));
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
        h.state_mut().ui.status.clear();
        h.run_steps(3);
        assert!(h.state().ui.status.is_empty(), "a failed Apply must not retry each frame");

        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        fields["__variables"]["dataSets"][0]["values"][0]["value"] = json!(dir.path().join("blue.png"));
        h.get_by_label("Apply").click();
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
    }
}

/// `ui.dialog.set` can store any JSON in `__variables`; the editors write into its entries.
fn app_with_malformed_variables(page: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let mut fields = Map::new();
    fields.insert(
        "__variables".into(),
        json!({"defs": [42, {"name": "v1", "layer": 0, "type": "textReplacement"}], "dataSets": ["bad", {"name": "A", "values": [7, "x"]}, {"name": "B", "values": "nope"}]}),
    );
    fields.insert("__page".into(), json!(page));
    app.ui.open_dialog(DialogKind::Command, fields);
    app
}

fn type_into_first_text_field(h: &mut Harness<'static, PhotocraftApp>) {
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().expect("a text field").click();
    h.run_steps(2);
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().expect("a text field").type_text("z");
    h.run_steps(2);
}

#[test]
fn malformed_variable_entries_are_dropped_before_an_edit() {
    for page in ["define", "dataSets"] {
        let mut h = harness(app_with_malformed_variables(page));
        type_into_first_text_field(&mut h);
        let state = &h.state().ui.dialogs.last().unwrap().fields["__variables"];
        assert_eq!(state["defs"].as_array().unwrap().len(), 1, "{page}: {state}");
        assert_eq!(state["dataSets"].as_array().unwrap().len(), 2, "{page}: {state}");
        assert!(state["dataSets"].as_array().unwrap().iter().all(|s| s["values"].as_array().unwrap().iter().all(|v| v.is_object())), "{page}: {state}");
    }
}
