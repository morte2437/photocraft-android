//! #14034: File › New › Advanced Options › Pixel Aspect Ratio, right below Color Profile, with
//! Photoshop's entries and Square Pixels by default; Create gives the new document the picked
//! ratio, as View › Pixel Aspect Ratio then shows.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::new_doc_ui::pixel_aspect;
use crate::state::{DialogKind, UiState};
use crate::workspace_ui::CustomPar;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// Open New Document, set `fields`, press Create.
fn create(app: &mut PhotocraftApp, fields: Value) {
    let f = app.new_document_fields();
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    crate::dialogs::confirm(app, id).unwrap();
}

/// The View › Pixel Aspect Ratio entries shown ticked.
fn ticked(app: &PhotocraftApp) -> Vec<String> {
    crate::menus::menu_items(app).into_iter().filter(|i| i.id.starts_with("view.pixelAspectRatio.") && i.checked == Some(true)).map(|i| i.id).collect()
}

#[test]
fn the_menu_lists_the_measured_entries_with_square_pixels_by_default() {
    let ids: Vec<String> = pixel_aspect::options(&[]).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["square", "d1DvNtsc", "d1DvPal", "d1DvNtscWidescreen", "hdv1080", "d1DvPalWidescreen", "anamorphic2To1", "dvcproHd1080"]);
    let labels: Vec<String> = pixel_aspect::options(&[]).into_iter().map(|(_, l)| l).collect();
    assert_eq!(
        labels,
        [
            "Square Pixels",
            "D1/DV NTSC (0.91)",
            "D1/DV PAL (1.09)",
            "D1/DV NTSC Widescreen (1.21)",
            "HDV 1080/DVCPRO HD 720 (1.33)",
            "D1/DV PAL Widescreen (1.46)",
            "Anamorphic 2:1 (2)",
            "DVCPRO HD 1080 (1.5)",
        ]
    );
    // Every id is View › Pixel Aspect Ratio's, with the ratio its label shows.
    for (id, ratio) in ids.iter().zip([1.0, 0.91, 1.09, 1.21, 1.33, 1.46, 2.0, 1.5]) {
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(id), ratio, "{id}");
    }
    assert_eq!(pixel_aspect::picked(&UiState::new_document_fields(), &[]), "square");
}

#[test]
fn saved_custom_ratios_follow_the_presets() {
    let custom = [CustomPar { name: "Cinema".into(), ratio: 1.25 }, CustomPar { name: "Tall".into(), ratio: 0.5 }];
    let opts = pixel_aspect::options(&custom);
    assert_eq!(opts.len(), 10);
    assert_eq!(opts[7].0, "dvcproHd1080");
    assert_eq!(opts[8..], [("custom:1.25:Cinema".to_string(), "Cinema".to_string()), ("custom:0.5:Tall".to_string(), "Tall".to_string())]);
    // A picked custom ratio that was deleted since, or a stray value, shows Square Pixels.
    let mut f = Map::new();
    f.insert(pixel_aspect::FIELD.into(), json!("custom:1.25:Cinema"));
    assert_eq!(pixel_aspect::picked(&f, &custom), "custom:1.25:Cinema");
    assert_eq!(pixel_aspect::picked(&f, &custom[1..]), "square");
    f.insert(pixel_aspect::FIELD.into(), json!(7));
    assert_eq!(pixel_aspect::picked(&f, &custom), "square");
}

#[test]
fn create_gives_the_new_document_the_picked_ratio() {
    let mut app = app();
    create(&mut app, json!({"width": 720, "height": 480, pixel_aspect::FIELD: "d1DvNtsc"}));
    assert_eq!(app.session.active().map(|st| (st.doc.size.width, st.doc.size.height)), Some((720, 480)));
    assert_eq!(ticked(&app), ["view.pixelAspectRatio.d1DvNtsc"]);
    // A saved custom ratio ticks View › Pixel Aspect Ratio › Custom.
    app.ui.shell.custom_pars.push(CustomPar { name: "Cinema".into(), ratio: 1.25 });
    create(&mut app, json!({pixel_aspect::FIELD: "custom:1.25:Cinema"}));
    assert_eq!(ticked(&app), ["view.pixelAspectRatio.custom"]);
    assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.25);
    // The field is the shell's own: `file.new` doesn't get it.
    let mut f = UiState::new_document_fields();
    f.insert(pixel_aspect::FIELD.into(), json!("hdv1080"));
    assert!(crate::new_doc_ui::command_params(&f).get(pixel_aspect::FIELD).is_none());
}

#[test]
fn square_pixels_resets_a_ratio_picked_in_the_view_menu() {
    let mut app = app();
    app.ui.view.pixel_aspect = "d1DvPal".into();
    create(&mut app, json!({}));
    assert_eq!(app.ui.view.pixel_aspect, "square");
    assert_eq!(ticked(&app), ["view.pixelAspectRatio.square"]);
    app.ui.view.pixel_aspect = "anamorphic2To1".into();
    create(&mut app, json!({pixel_aspect::FIELD: "square"}));
    assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.0);
}

#[test]
fn a_failed_create_leaves_the_ratio_alone() {
    let mut app = app();
    app.ui.view.pixel_aspect = "d1DvPal".into();
    let id = app.ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(0));
    d.fields.insert(pixel_aspect::FIELD.into(), json!("anamorphic2To1"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.ui.view.pixel_aspect, "d1DvPal");
}

mod dialog {
    use super::*;

    fn harness() -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(3);
        h
    }

    fn fields(h: &Harness<'static, PhotocraftApp>) -> Map<String, Value> {
        h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
    }

    /// The dialog's dropdowns top to bottom, by name.
    fn dropdowns(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
        let mut v: Vec<(f32, String)> = h.get_all_by_role(Role::ComboBox).map(|n| (n.rect().top(), n.accesskit_node().label().unwrap_or_default())).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, l)| l).collect()
    }

    /// Open the dropdown named `combo` and pick `entry`.
    fn pick(h: &mut Harness<'static, PhotocraftApp>, combo: &str, entry: &str) {
        h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some(combo)).expect(combo).click();
        h.run_steps(2);
        h.get_all_by_label(entry).last().expect(entry).click();
        h.run_steps(3);
    }

    /// Click into a text or number field and type `text` over its selection.
    fn type_into(h: &mut Harness<'static, PhotocraftApp>, role: Role, i: usize, text: &str) {
        h.query_all_by_role(role).nth(i).expect("the field").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        for c in text.chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run_steps(1);
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
    }

    #[test]
    fn the_dropdown_sits_right_below_color_profile() {
        let h = harness();
        let names = dropdowns(&h);
        let profile = names.iter().position(|n| n == "Color Profile").expect("the Color Profile dropdown");
        assert_eq!(names.get(profile + 1).map(String::as_str), Some("Pixel Aspect Ratio"), "{names:?}");
        let shown =
            h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Pixel Aspect Ratio")).unwrap().accesskit_node().value();
        assert_eq!(shown.as_deref(), Some("Square Pixels"));
    }

    #[test]
    fn the_pick_is_kept_while_the_other_fields_change_and_create_applies_it() {
        let mut h = harness();
        pick(&mut h, "Pixel Aspect Ratio", "D1/DV PAL Widescreen (1.46)");
        assert_eq!(fields(&h)[pixel_aspect::FIELD], "d1DvPalWidescreen");
        type_into(&mut h, Role::TextInput, 0, "Video");
        type_into(&mut h, Role::SpinButton, 0, "720");
        type_into(&mut h, Role::SpinButton, 1, "576");
        type_into(&mut h, Role::SpinButton, 2, "150");
        pick(&mut h, "Color Mode", "Grayscale");
        let f = fields(&h);
        assert_eq!((&f["name"], &f["width"], &f["height"], &f["mode"]), (&json!("Video"), &json!(720), &json!(576), &json!("gray")));
        assert_eq!(f["resolution"].as_f64(), Some(150.0));
        assert_eq!(f[pixel_aspect::FIELD], "d1DvPalWidescreen");
        h.get_by_label("Create").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
        assert_eq!(ticked(h.state()), ["view.pixelAspectRatio.d1DvPalWidescreen"]);
    }
}
