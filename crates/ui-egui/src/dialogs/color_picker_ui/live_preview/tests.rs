use egui::vec2;
use egui_kittest::Harness;
use serde_json::{Map, Value, json};

use super::*;
use crate::color_picker_ui::{open_for_field, open_requested, request, request_field};
use crate::crop_shield::ShieldColor;
use crate::state::DialogKind;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 12})).unwrap();
    app
}

fn fields(app: &PhotocraftApp, id: u64) -> &Map<String, Value> {
    &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The Picker's colour changes, as a drag in it or `ui.dialog.set` changes it, and a frame passes.
fn pick(app: &mut PhotocraftApp, picker: u64, color: &str) {
    app.ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!(color));
    sync(app);
}

/// Edit › Fill on Color…, with its Color Picker open over it as its Color… choice opens it.
fn fill_with_picker(app: &mut PhotocraftApp) -> (u64, u64) {
    let fill = crate::fill_ui::open(app);
    let f = &mut app.ui.dialog_mut(fill).unwrap().fields;
    f.insert("contents".into(), json!("color"));
    f.insert("color".into(), json!("#20a040"));
    request(f, "Color Picker (Fill Color)");
    open_requested(app, fill);
    let picker = crate::color_picker_ui::top(app).expect("the picker opens over the Fill dialog");
    (fill, picker)
}

/// #9886: the Fill dialog's colour follows the picker before OK, Cancel puts back exactly what
/// the dialog held, and nothing reaches the document's History.
#[test]
fn a_dialog_colour_follows_the_picker_and_cancel_restores_it() {
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    let before = fields(&app, fill).clone();
    for color in ["#336699", "#ff0000", "#00ff00"] {
        pick(&mut app, picker, color);
        assert_eq!(fields(&app, fill)["color"], json!(color), "shown before OK");
    }
    assert_eq!(past(&app), 0, "previewing makes no history step");
    crate::dialogs::cancel(&mut app, picker).unwrap();
    assert_eq!(fields(&app, fill), &before, "Cancel puts the dialog back as it was");
    assert!(app.color_picker_live.is_empty());
    assert_eq!(past(&app), 0);
}

/// OK keeps the colour in the dialog, still without a history step; the dialog's own OK then
/// makes exactly one, with the picked colour.
#[test]
fn ok_keeps_the_colour_and_the_dialog_makes_one_history_step() {
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    pick(&mut app, picker, "#ff0000");
    pick(&mut app, picker, "#336699");
    crate::dialogs::confirm(&mut app, picker).unwrap();
    assert_eq!(fields(&app, fill)["color"], json!("#336699"));
    assert!(app.color_picker_live.is_empty());
    assert_eq!(past(&app), 0, "the picker's OK only sets the dialog's colour");
    crate::dialogs::confirm(&mut app, fill).unwrap();
    assert_eq!(past(&app), 1, "one step for the Fill");
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(3, 3);
    let want = [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0];
    assert!((0..3).all(|i| (c[i] - want[i]).abs() < 1.5 / 255.0), "{c:?}");
}

/// Back on the colour it opened on, the target shows what it held, even a value that is not a
/// hex colour; a picker that never changes leaves it alone.
#[test]
fn the_opening_colour_shows_the_original_value() {
    let mut app = app();
    let mut f = Map::new();
    f.insert("background".into(), json!("white"));
    f.insert("other".into(), json!(3));
    let parent = app.ui.open_dialog(DialogKind::Command, f);
    let before = fields(&app, parent).clone();
    let picker = open_for_field(&mut app, "Color Picker (Background Contents)", [1.0; 3], parent, "background");
    assert!(!sync(&mut app), "nothing to show yet");
    assert_eq!(fields(&app, parent), &before);
    pick(&mut app, picker, "#808080");
    assert_eq!(fields(&app, parent)["background"], json!("#808080"));
    pick(&mut app, picker, "#ffffff");
    assert_eq!(fields(&app, parent), &before, "the opening colour shows the original value");
    // Not a colour (a half-typed value from automation): the original value too.
    pick(&mut app, picker, "#12");
    assert_eq!(fields(&app, parent), &before);
}

/// A dialog that derives other settings from the shown colour (Replace Color's Result sets the
/// sliders and drops the colour) builds each preview on the settings it had, and Cancel undoes
/// what it derived.
#[test]
fn derived_settings_are_rebuilt_from_the_opening_ones() {
    let mut app = app();
    let mut f = Map::new();
    f.insert("picked".into(), json!("#000000"));
    f.insert("hue".into(), json!(0));
    let parent = app.ui.open_dialog(DialogKind::Command, f);
    let before = fields(&app, parent).clone();
    let picker = open_for_field(&mut app, "Color Picker (Result)", [0.0; 3], parent, "picked");
    // The dialog's body consumes the colour each frame, as Replace Color's does.
    let consume = |app: &mut PhotocraftApp| {
        let f = &mut app.ui.dialog_mut(parent).unwrap().fields;
        let picked = f.remove("picked").unwrap();
        let hue = f["hue"].as_i64().unwrap();
        f.insert("hue".into(), json!(hue + i64::from(picked != json!("#000000"))));
    };
    pick(&mut app, picker, "#ff0000");
    consume(&mut app);
    pick(&mut app, picker, "#00ff00");
    assert_eq!(fields(&app, parent)["hue"], json!(0), "rebuilt from the opening settings, not the last preview");
    assert_eq!(fields(&app, parent)["picked"], json!("#00ff00"));
    consume(&mut app);
    crate::dialogs::cancel(&mut app, picker).unwrap();
    assert_eq!(fields(&app, parent), &before);
}

/// A picker closed without Cancel (a document closed, a parent cancelled) never leaves its
/// colour behind, and a closed parent ends the preview without an error or a panic.
#[test]
fn a_picker_or_parent_closed_any_other_way_ends_the_preview() {
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    let before = fields(&app, fill).clone();
    pick(&mut app, picker, "#336699");
    app.ui.close_dialog(picker);
    assert!(sync(&mut app));
    assert_eq!(fields(&app, fill), &before);
    assert!(app.color_picker_live.is_empty());

    // A cancelled dialog takes its field picker with it, leaving no picker whose OK has no target.
    let (fill, picker) = fill_with_picker(&mut app);
    let other = open_for_field(&mut app, "Color Picker (Other)", [0.0; 3], 999_999, "color");
    pick(&mut app, picker, "#336699");
    crate::dialogs::cancel(&mut app, fill).unwrap();
    assert!(app.ui.dialogs.iter().all(|d| d.id != picker), "the picker closes with its dialog");
    assert!(app.ui.dialogs.iter().any(|d| d.id == other), "another dialog's picker stays");
    assert!(app.color_picker_live.is_empty(), "the preview ends with its dialog");
    assert_eq!(past(&app), 0);
}

/// The dialog under a picker can't be confirmed (by automation) with a colour the picker still
/// only shows, even when the picker's `__dialog` was rewritten to point elsewhere.
#[test]
fn the_dialog_waits_for_its_picker() {
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    pick(&mut app, picker, "#336699");
    assert!(crate::dialogs::confirm(&mut app, fill).is_err());
    app.ui.dialog_mut(picker).unwrap().fields.insert("__dialog".into(), json!(999_999));
    assert!(crate::dialogs::confirm(&mut app, fill).is_err(), "the live preview's record of the dialog counts");
    assert_eq!(past(&app), 0);
    assert!(app.ui.dialogs.iter().any(|d| d.id == fill), "the dialog stays open");
}

/// Automation can't edit a dialog under a previewing picker: Cancel would put the edit back.
#[test]
fn the_dialog_under_a_picker_refuses_control_edits() {
    let ctx = egui::Context::default();
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    let (req, _rx) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": fill, "field": "opacity", "value": 50}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("an immediate reply") };
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(fields(&app, fill)["opacity"], json!(100.0));
    crate::dialogs::cancel(&mut app, picker).unwrap();
    let (req, _rx) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": fill, "field": "opacity", "value": 50}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("an immediate reply") };
    assert_eq!(r["ok"], true, "{r}");
}

/// A shield colour set elsewhere (`ui.set cropShield`) while its picker is open survives the
/// picker's Cancel.
#[test]
fn the_crop_shield_keeps_a_colour_set_while_its_picker_is_open() {
    let mut app = app();
    let picker = crate::crop_shield::pick_custom_color(&mut app);
    pick(&mut app, picker, "#336699");
    app.ui.tool_options.crop_shield.custom_color = [1, 2, 3];
    crate::dialogs::cancel(&mut app, picker).unwrap();
    let s = &app.ui.tool_options.crop_shield;
    assert_eq!((s.color, s.custom_color), (ShieldColor::Custom, [1, 2, 3]));

    let picker = crate::crop_shield::pick_custom_color(&mut app);
    app.ui.tool_options.crop_shield.color = ShieldColor::MatchCanvas;
    pick(&mut app, picker, "#ff0000");
    crate::dialogs::cancel(&mut app, picker).unwrap();
    let s = &app.ui.tool_options.crop_shield;
    assert_eq!((s.color, s.custom_color), (ShieldColor::MatchCanvas, [1, 2, 3]), "Cancel puts back the choice made while it was open");
}

/// The dialog's own ⌘Z history records no step for a previewed colour: none after Cancel, one
/// after OK.
#[test]
fn a_previewed_colour_is_no_dialog_undo_step() {
    let ctx = egui::Context::default();
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    crate::edit_history::track_dialogs(&mut app, &ctx);
    let undo = |app: &mut PhotocraftApp| {
        let mut now: Map<String, Value> = fields(app, fill).iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
        app.dialog_edits.get_mut(&fill).unwrap().step(&mut now, crate::edit_history::Step::Undo).then_some(now)
    };
    for color in ["#336699", "#ff0000"] {
        pick(&mut app, picker, color);
        crate::edit_history::track_dialogs(&mut app, &ctx);
    }
    crate::dialogs::cancel(&mut app, picker).unwrap();
    crate::edit_history::track_dialogs(&mut app, &ctx);
    assert_eq!(undo(&mut app), None, "Cancel leaves nothing to undo");

    request(&mut app.ui.dialog_mut(fill).unwrap().fields, "Color Picker (Fill Color)");
    open_requested(&mut app, fill);
    let picker = crate::color_picker_ui::top(&app).unwrap();
    for color in ["#336699", "#ff0000"] {
        pick(&mut app, picker, color);
        crate::edit_history::track_dialogs(&mut app, &ctx);
    }
    crate::dialogs::confirm(&mut app, picker).unwrap();
    crate::edit_history::track_dialogs(&mut app, &ctx);
    let back = undo(&mut app).expect("OK is one step");
    assert_eq!(back["color"], json!("#20a040"), "straight back to the colour before the picker");
}

/// The Crop shield's custom colour follows its picker; Cancel restores the colour and the
/// Match Canvas choice, OK keeps the custom one.
#[test]
fn the_crop_shield_colour_follows_its_picker() {
    let mut app = app();
    app.ui.tool_options.crop_shield.custom_color = [10, 20, 30];
    let before = app.ui.tool_options.crop_shield.clone();
    assert_eq!(before.color, ShieldColor::MatchCanvas);
    let picker = crate::crop_shield::pick_custom_color(&mut app);
    pick(&mut app, picker, "#336699");
    let s = &app.ui.tool_options.crop_shield;
    assert_eq!((s.color, s.custom_color), (ShieldColor::Custom, [0x33, 0x66, 0x99]), "shown before OK");
    crate::dialogs::cancel(&mut app, picker).unwrap();
    assert_eq!(app.ui.tool_options.crop_shield, before);

    let picker = crate::crop_shield::pick_custom_color(&mut app);
    pick(&mut app, picker, "#ff0000");
    pick(&mut app, picker, "#336699");
    crate::dialogs::confirm(&mut app, picker).unwrap();
    let s = &app.ui.tool_options.crop_shield;
    assert_eq!((s.color, s.custom_color), (ShieldColor::Custom, [0x33, 0x66, 0x99]));
    assert!(app.color_picker_live.is_empty());
    assert_eq!(past(&app), 0);
}

/// Agents: `ui.dialog.set` on the picker shows the colour in the dialog at once, and
/// `ui.dialog.cancel` puts it back.
#[test]
fn control_edits_preview_at_once() {
    let ctx = egui::Context::default();
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let r = call(&mut app, "ui.dialog.set", json!({"dialog": picker, "field": "color", "value": "#336699"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(fields(&app, fill)["color"], json!("#336699"));
    assert_eq!(call(&mut app, "ui.dialog.confirm", json!({"dialog": fill}))["ok"], false, "the picker answers first");
    assert_eq!(call(&mut app, "ui.dialog.cancel", json!({"dialog": picker}))["ok"], true);
    assert_eq!(fields(&app, fill)["color"], json!("#20a040"));
}

/// In the running dialogs: a colour typed into the picker shows in the Fill dialog's swatch field
/// before OK, and Esc (the picker's Cancel) puts it back.
#[test]
fn the_dialog_shows_the_colour_while_the_picker_is_open() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let (fill, picker) = fill_with_picker(h.state_mut());
    h.run_steps(3);
    h.state_mut().ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#336699"));
    h.run_steps(3);
    assert_eq!(fields(h.state(), fill)["color"], json!("#336699"));
    assert_eq!(past(h.state()), 0);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(crate::color_picker_ui::top(h.state()), None);
    assert_eq!(fields(h.state(), fill)["color"], json!("#20a040"));
}

/// A picker's target is fixed when it opens: automation can't point its OK at another dialog, and
/// cancelling a dialog its `__dialog` was rewritten to never closes it.
#[test]
fn a_pickers_target_stays_the_dialog_it_opened_on() {
    let ctx = egui::Context::default();
    let mut app = app();
    let (fill, picker) = fill_with_picker(&mut app);
    let other = app.ui.open_dialog(DialogKind::Command, Map::new());
    let set = |app: &mut PhotocraftApp, field: &str, value: Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": picker, "field": field, "value": value}));
        let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("an immediate reply") };
        r["ok"].as_bool()
    };
    assert_eq!(set(&mut app, "__dialog", json!(other)), Some(false));
    assert_eq!(set(&mut app, "__field", json!("opacity")), Some(false));
    assert_eq!(set(&mut app, "__colorPicker", json!("cropShield")), Some(false), "nor can its OK go to the Crop shield");
    assert_eq!(set(&mut app, "color", json!("#336699")), Some(true), "the colour itself stays drivable");
    assert_eq!(fields(&app, fill)["color"], json!("#336699"));

    // Rewritten another way, `__dialog` still doesn't make it another dialog's picker.
    app.ui.dialog_mut(picker).unwrap().fields.insert("__dialog".into(), json!(other));
    crate::dialogs::cancel(&mut app, other).unwrap();
    assert!(app.ui.dialogs.iter().any(|d| d.id == picker), "the picker stays with its own dialog");
    assert_eq!(fields(&app, fill)["color"], json!("#336699"), "and keeps previewing there");
}

/// Preferences › Apply, like OK, waits for an open colour picker: it would otherwise save the
/// colour the picker previews and may still cancel.
#[test]
fn preferences_apply_waits_for_an_open_picker() {
    let mut app = app();
    let prefs = crate::prefs_ui::open_preferences(&mut app, "guidesGridAndSlices");
    let before = app.session.prefs().guides_grid_and_slices.guide_color.clone();
    let f = &mut app.ui.dialog_mut(prefs).unwrap().fields;
    request_field(f, "/values/guidesGridAndSlices/guideColor", "Color Picker (Guide Color)");
    open_requested(&mut app, prefs);
    let picker = crate::color_picker_ui::top(&app).expect("the picker opens over Preferences");
    pick(&mut app, picker, "#ff8000");
    assert!(crate::prefs_ui::apply(&mut app, prefs).is_err(), "Apply refuses while the picker is open");
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, before, "nothing is saved");
}
