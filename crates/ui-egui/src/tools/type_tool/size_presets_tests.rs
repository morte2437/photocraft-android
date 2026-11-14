//! #12052: the Type options bar's font size field has a chevron with the reference application's
//! preset sizes. Picking one edits the same targets as typing a size, and the Character panel's
//! list keeps its own behaviour.

use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;

const MENU: &str = "Font size presets";
const SIZES: [&str; 17] =
    ["6 pt", "8 pt", "9 pt", "10 pt", "11 pt", "12 pt", "14 pt", "16 pt", "18 pt", "24 pt", "30 pt", "36 pt", "48 pt", "60 pt", "72 pt", "150 pt", "300 pt"];

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1600, "height": 900})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

fn add_layer(app: &mut PhotocraftApp, size: f32) -> LayerId {
    let made = app.run("type.create", json!({"text": "AéBCD", "font": "Inter", "size": size, "color": "#000000"})).unwrap();
    LayerId(made["layer"].as_u64().unwrap())
}

/// An app whose one selected type layer is `size` points.
fn app_with_layer(size: f32) -> (PhotocraftApp, LayerId) {
    let mut app = new_app();
    let id = add_layer(&mut app, size);
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    (app, id)
}

/// `draw` in a window tall enough for the list, once the app's fonts are set up. Up to 16 passes,
/// because the font list arrives in batches (see `canvas_tests::controls`).
fn harness(app: PhotocraftApp, draw: fn(&mut PhotocraftApp, &mut egui::Ui)) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 800.0)).with_max_steps(16).build_ui_state(
        move |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                draw(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| super::options_bar(app, ui));
}

fn character_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.set_width(340.0);
    super::super::character_panel(app, ui, false);
}

fn open(h: &mut Harness<'static, PhotocraftApp>) {
    // A button in the options bar, a combo box in the Character panel.
    h.get_by_label(MENU).click();
    h.run();
}

fn pick(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    open(h);
    // Presets near the end of the list can be below its scroll viewport.
    h.get_by_label(label).scroll_to_me();
    h.run();
    h.get_by_label(label).click();
    h.run();
}

/// The open list's labels, top to bottom, with "|" where a separator splits them (a gap clearly
/// larger than the row pitch). `any` is one item's label.
fn menu_items(h: &Harness<'static, PhotocraftApp>, any: &str) -> Vec<String> {
    let role = h.get_by_label(any).accesskit_node().role();
    let left = h.get_by_label(any).rect().left();
    let mut items: Vec<(f32, String)> = h
        .query_all_by_role(role)
        .filter(|n| (n.rect().left() - left).abs() < 1.0)
        .filter_map(|n| Some((n.rect().top(), n.accesskit_node().label()?)))
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pitch = items.windows(2).map(|w| w[1].0 - w[0].0).fold(f32::MAX, f32::min);
    let mut out = Vec::new();
    for (i, (top, label)) in items.iter().enumerate() {
        if i > 0 && top - items[i - 1].0 > pitch + 2.0 {
            out.push("|".to_owned());
        }
        out.push(label.clone());
    }
    out
}

/// Each run of the layer's text as (bytes, size in points).
fn runs(app: &PhotocraftApp, id: LayerId) -> Vec<(usize, f32)> {
    let t = super::super::text_layer(&app.session.active().unwrap().doc, id).unwrap();
    t.char_runs().iter().map(|r| (r.len, r.style.size_pt)).collect()
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

#[test]
fn a_current_value_that_is_no_preset_heads_the_list_above_a_separator() {
    let (app, _) = app_with_layer(17.0);
    let mut h = harness(app, options_bar);
    open(&mut h);
    let expected: Vec<&str> = ["17 pt", "|"].into_iter().chain(SIZES).collect();
    assert_eq!(menu_items(&h, "72 pt"), expected);
}

/// Not measured in the reference application: this keeps the Character panel's list, which has
/// no row above the presets.
#[test]
fn a_current_value_that_is_a_preset_is_only_marked_in_the_list() {
    let (app, _) = app_with_layer(18.0);
    let mut h = harness(app, options_bar);
    open(&mut h);
    assert_eq!(menu_items(&h, "72 pt"), SIZES);
    assert_eq!(h.get_by_label("18 pt").accesskit_node().toggled(), Some(Toggled::True));
    assert_eq!(h.get_by_label("72 pt").accesskit_node().toggled(), Some(Toggled::False));
}

#[test]
fn picking_a_preset_sets_the_selected_layers_and_the_tool_default_in_one_step() {
    let mut app = new_app();
    let ids = [add_layer(&mut app, 12.0), add_layer(&mut app, 16.0)];
    app.run("layer.select", json!({"layer": ids[0].0, "mode": "replace"})).unwrap();
    app.run("layer.select", json!({"layer": ids[1].0, "mode": "add"})).unwrap();
    let steps = history_len(&app);
    let mut h = harness(app, options_bar);
    pick(&mut h, "72 pt");
    assert_eq!(runs(h.state(), ids[0]), [(6, 72.0)]);
    assert_eq!(runs(h.state(), ids[1]), [(6, 72.0)]);
    assert_eq!(h.state().ui.tool_options.type_size, 72.0);
    assert_eq!(history_len(h.state()), steps + 1);
}

#[test]
fn picking_a_preset_while_editing_sets_only_the_selected_characters() {
    let (mut app, id) = app_with_layer(12.0);
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: None,
        caret: 3,
        anchor: 1,
        session: "size-presets".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    let mut h = harness(app, options_bar);
    pick(&mut h, "48 pt");
    // "A" is one byte, "éB" three, "CD" two.
    assert_eq!(runs(h.state(), id), [(1, 12.0), (3, 48.0), (2, 12.0)]);
}

#[test]
fn without_a_type_layer_a_preset_sets_the_default_and_edits_nothing() {
    let app = new_app();
    let steps = history_len(&app);
    let mut h = harness(app, options_bar);
    pick(&mut h, "72 pt");
    assert_eq!(h.state().ui.tool_options.type_size, 72.0);
    assert_eq!(history_len(h.state()), steps);
}

#[test]
fn choosing_the_current_value_row_changes_nothing() {
    let (app, id) = app_with_layer(17.0);
    let steps = history_len(&app);
    let mut h = harness(app, options_bar);
    open(&mut h);
    h.get_by_label("17 pt").click();
    h.run();
    assert_eq!(runs(h.state(), id), [(6, 17.0)]);
    assert_eq!(history_len(h.state()), steps);
    assert!(h.query_by_label("72 pt").is_none(), "the list closed");
}

/// 144 ppi, so a pixel is not a point; presets stay exact point values.
#[test]
fn the_list_follows_the_type_units_in_pixels() {
    let mut app = new_app();
    app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "pixels"})).unwrap();
    app.run("image.imageSize", json!({"resolution": 144, "resample": "none"})).unwrap();
    let id = add_layer(&mut app, 18.0);
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    let mut h = harness(app, options_bar);
    open(&mut h);
    let items = menu_items(&h, "144 px");
    assert_eq!(items.first().map(String::as_str), Some("12 px"));
    assert_eq!(items.last().map(String::as_str), Some("600 px"));
    assert_eq!(items.len(), SIZES.len());
    h.key_press(egui::Key::Escape);
    h.run();
    pick(&mut h, "144 px");
    assert_eq!(runs(h.state(), id), [(6, 72.0)]);
}

#[test]
fn the_list_follows_the_type_units_in_millimetres() {
    let (mut app, id) = app_with_layer(18.0);
    app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "mm"})).unwrap();
    let mut h = harness(app, options_bar);
    pick(&mut h, "25.4 mm");
    assert_eq!(runs(h.state(), id), [(6, 72.0)]);
}

#[test]
fn typing_a_size_still_works() {
    let (app, id) = app_with_layer(12.0);
    let mut h = harness(app, options_bar);
    h.get_by_role_and_label(Role::SpinButton, "Font size").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("33".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(runs(h.state(), id), [(6, 33.0)]);
    assert_eq!(h.state().ui.tool_options.type_size, 33.0);
}

#[test]
fn the_chevron_has_the_size_fields_tooltip() {
    let (app, _) = app_with_layer(12.0);
    let h = harness(app, options_bar);
    let tips: Vec<String> = h.query_all_by_label(MENU).filter_map(|n| n.accesskit_node().description().map(|d| d.to_string())).collect();
    assert_eq!(tips, ["Set font size"]);
}

/// The arrow sits inside the field's right edge, as on the font menu and the Layers panel's
/// Opacity, not in a box of its own beside the field.
#[test]
fn the_arrow_is_embedded_in_the_field() {
    let (app, _) = app_with_layer(12.0);
    let h = harness(app, options_bar);
    // The number's own rectangle: the unit and the arrow follow it inside the field's frame.
    let digits = h.get_by_role_and_label(Role::SpinButton, "Font size").rect();
    let arrow = h.get_by_label(MENU).rect();
    assert_eq!(arrow.width(), crate::widgets::POPUP_ARROW_W, "{arrow:?}");
    assert!(arrow.left() >= digits.right(), "{arrow:?} overlaps the number {digits:?}");
    assert!(digits.top() <= arrow.top() && arrow.bottom() <= digits.bottom(), "{arrow:?} leaves the field's height {digits:?}");
}

/// The bar draws one "Loading fonts…" placeholder until the fonts are ready, then the Font and
/// Style menus, so the controls before the size field change while the app runs. A widget more
/// in front of it stands in for that switch here.
#[test]
fn the_list_stays_open_when_the_controls_before_it_change() {
    fn shifted() -> egui::Id {
        egui::Id::unique("size-presets-shift")
    }
    fn shifting_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.data(|d| d.get_temp::<bool>(shifted())).unwrap_or(false) {
                ui.label("Loading fonts…");
            }
            super::options_bar(app, ui);
        });
    }
    let (app, _) = app_with_layer(17.0);
    let mut h = harness(app, shifting_bar);
    open(&mut h);
    h.ctx.data_mut(|d| d.insert_temp(shifted(), true));
    h.run();
    assert!(h.query_by_label("72 pt").is_some(), "the list closed");
    h.get_by_label("72 pt").scroll_to_me();
    h.run();
    h.get_by_label("72 pt").click();
    h.run();
    assert_eq!(h.state().ui.tool_options.type_size, 72.0);
}

/// A guard, so it passes without the change too: the current-value row is the options bar's.
#[test]
fn the_character_panel_list_keeps_no_current_value_row() {
    let (app, _) = app_with_layer(17.0);
    let mut h = harness(app, character_panel);
    open(&mut h);
    assert_eq!(menu_items(&h, "72 pt"), SIZES);
}
