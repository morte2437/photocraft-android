//! #13806: Edit › Fade's Mode list holds the 27 layer blend modes in the reference order, in six
//! groups, and greys the modes a Grayscale 8-bit, Lab 8-bit or RGB 32-bit document can't fade with.

use photocraft_color::{BlendMode, ColorMode, SampleType};
use serde_json::{Value, json};

use super::{COMMAND, greyed, step_past};
use crate::PhotocraftApp;
use crate::dialogs::dialog_blend_ui::mode_key;
use crate::filter_dialog::{Kind, parse_spec};

/// The reference application's Fade Mode list, a group per row.
const GROUPS: [&[&str]; 6] = [
    &["Normal", "Dissolve"],
    &["Darken", "Multiply", "Color Burn", "Linear Burn", "Darker Color"],
    &["Lighten", "Screen", "Color Dodge", "Linear Dodge (Add)", "Lighter Color"],
    &["Overlay", "Soft Light", "Hard Light", "Vivid Light", "Linear Light", "Pin Light", "Hard Mix"],
    &["Difference", "Exclusion", "Subtract", "Divide"],
    &["Hue", "Saturation", "Color", "Luminosity"],
];

fn app(mode: &str, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 12, "mode": mode, "depth": depth})).unwrap();
    app
}

fn labels(modes: &[BlendMode]) -> Vec<&'static str> {
    modes.iter().map(|m| m.label()).collect()
}

fn mode_options() -> Vec<String> {
    let spec = photocraft_engine::commands::find(COMMAND).unwrap();
    let p = parse_spec(spec.params).into_iter().find(|p| p.key == "mode").unwrap();
    let Kind::Choice { options, .. } = p.kind else { panic!("mode is not a choice") };
    options
}

#[test]
fn the_list_is_the_27_modes_in_six_groups() {
    let shown = labels(&BlendMode::LAYER_MODES);
    assert_eq!(shown, GROUPS.concat());
    // A line above the first mode of every group but the first.
    let mut groups: Vec<Vec<&str>> = Vec::new();
    for m in BlendMode::LAYER_MODES {
        match groups.last_mut() {
            Some(g) if !crate::panels::starts_blend_group(m) => g.push(m.label()),
            _ => groups.push(vec![m.label()]),
        }
    }
    assert_eq!(groups, GROUPS.map(<[&str]>::to_vec));
}

#[test]
fn the_params_doc_lists_all_27_modes_and_the_old_names_still_work() {
    let options = mode_options();
    let modes: Vec<Option<BlendMode>> = options.iter().map(|o| photocraft_engine::commands::blend_from_str(o)).collect();
    assert_eq!(modes, BlendMode::LAYER_MODES.map(Some).to_vec());
    assert_eq!(options, BlendMode::LAYER_MODES.map(mode_key).to_vec(), "the keys the dialog writes");
    for old in ["normal", "multiply", "screen", "overlay", "softLight", "hardLight", "darken", "lighten", "difference", "color", "luminosity"] {
        assert!(options.iter().any(|o| o == old), "{old} is still listed");
    }
}

#[test]
fn every_listed_mode_fades() {
    for key in mode_options() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        let out = s.execute(COMMAND, json!({"opacity": 50, "mode": key})).unwrap();
        assert_eq!(out["mode"], json!(photocraft_engine::commands::blend_from_str(&key).unwrap().label()), "{key}");
    }
}

#[test]
fn greyed_modes_follow_the_measured_colour_modes_and_depths() {
    use SampleType::*;
    assert_eq!(labels(greyed(ColorMode::Grayscale, U8)), ["Darker Color", "Lighter Color", "Hue", "Saturation", "Color", "Luminosity"]);
    assert_eq!(labels(greyed(ColorMode::Lab, U8)), ["Darken", "Color Burn", "Lighten", "Color Dodge", "Difference", "Exclusion", "Subtract", "Divide"]);
    assert_eq!(
        labels(greyed(ColorMode::Rgb, F32)),
        [
            "Color Burn",
            "Linear Burn",
            "Screen",
            "Color Dodge",
            "Overlay",
            "Soft Light",
            "Hard Light",
            "Vivid Light",
            "Linear Light",
            "Pin Light",
            "Hard Mix",
            "Exclusion"
        ]
    );
    // Measured with none greyed, and the cases not measured keep the old list (none greyed).
    for (mode, depth) in
        [(ColorMode::Rgb, U16), (ColorMode::Cmyk, U8), (ColorMode::Rgb, U8), (ColorMode::Grayscale, U16), (ColorMode::Lab, U16), (ColorMode::Cmyk, U16)]
    {
        assert!(greyed(mode, depth).is_empty(), "{mode:?} {depth:?}");
    }
}

#[test]
fn opening_the_dialog_notes_the_documents_greyed_modes() {
    for (mode, depth, want) in [("lab", 8, greyed(ColorMode::Lab, SampleType::U8)), ("rgb", 32, greyed(ColorMode::Rgb, SampleType::F32)), ("rgb", 16, &[][..])]
    {
        let mut app = app(mode, depth);
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        let fields = &app.ui.dialog_mut(id).unwrap().fields;
        let noted: Vec<&str> = fields["__fadeGreyed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(noted, want.iter().map(|m| mode_key(*m)).collect::<Vec<_>>(), "{mode} {depth}");
        assert_eq!(fields["mode"], json!("normal"));
    }
}

#[test]
fn a_remembered_mode_the_document_greys_opens_as_normal() {
    let mut app = app("lab", 8);
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"mode": "darken", "opacity": 40})));
    let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((fields["mode"].clone(), fields["opacity"].clone()), (json!("normal"), json!(40)));

    // In a document that can fade with it, the remembered mode stays.
    let mut app = self::app("rgb", 8);
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"mode": "darken"})));
    let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["mode"], json!("darken"));
}

#[test]
fn the_keys_step_past_greyed_modes() {
    use BlendMode::*;
    let lab = greyed(ColorMode::Lab, SampleType::U8);
    assert_eq!(step_past(Normal, Multiply, lab), Multiply, "not greyed");
    // Down from Dissolve lands on Darken, greyed: Multiply is the next one.
    assert_eq!(step_past(Dissolve, Darken, lab), Multiply);
    // Up from Multiply: Dissolve.
    assert_eq!(step_past(Multiply, Darken, lab), Dissolve);
    // Down from Hard Mix past Difference, Exclusion, Subtract and Divide.
    assert_eq!(step_past(HardMix, Difference, lab), Hue);
    // Nothing open past it: stay.
    let all = BlendMode::LAYER_MODES;
    assert_eq!(step_past(Normal, Dissolve, &all[1..]), Normal);
}

#[test]
fn the_row_keeps_an_open_mode_and_writes_its_key() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = app("rgb", 32);
    let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("mode".into(), json!("Linear Dodge (Add)"));
    ctx.run_ui(egui::RawInput::default(), |ui| crate::filter_dialog::body(ui, &mut fields)).drop_without_applying_deltas();
    assert_eq!(fields["mode"], json!("linearDodge"));
    // A greyed mode an agent wrote isn't chosen from the list: the row leaves it to the command.
    fields.insert("mode".into(), json!("screen"));
    ctx.run_ui(egui::RawInput::default(), |ui| crate::filter_dialog::body(ui, &mut fields)).drop_without_applying_deltas();
    assert_eq!(fields["mode"], json!("screen"));
    // Nor is a mode no list names: OK reports it instead of fading in Normal.
    fields.insert("mode".into(), json!("nonsense"));
    ctx.run_ui(egui::RawInput::default(), |ui| crate::filter_dialog::body(ui, &mut fields)).drop_without_applying_deltas();
    assert_eq!(fields["mode"], json!("nonsense"));
}
