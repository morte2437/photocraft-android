//! #12008: while a Gradient fill layer is the active layer, the Gradient tool's Gradient / Classic
//! gradient dropdown is greyed and reads "Gradient", and a drag edits that fill layer even when
//! Classic gradient was chosen. Selecting a pixel layer brings the stored choice back.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A document with a pixel layer below a Gradient fill layer; returns (app, pixel layer, fill layer).
fn doc() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let pixels = LayerId(app.run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let fill = LayerId(app.run("layer.newFillLayer.gradient", json!({"from": "#235b49", "to": "#a7d4ab", "angle": 90})).unwrap()["layer"].as_u64().unwrap());
    app.sync_views();
    app.ui.tool = Tool::Gradient;
    (app, pixels, fill)
}

fn select(app: &mut PhotocraftApp, id: LayerId) {
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(id));
}

fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(4);
    h
}

/// The mode dropdown, found by its role and options rather than its label (the blend Mode label
/// reads "Mode:" or "Mode" by theme): (greyed, the option it shows).
fn dropdown(h: &Harness<'static, PhotocraftApp>) -> (bool, String) {
    let modes = ["Gradient", "Classic gradient"];
    let mut combos = h.query_all_by_role(Role::ComboBox).filter(|n| n.accesskit_node().value().is_some_and(|v| modes.contains(&v.as_str())));
    let node = combos.next().expect("the mode dropdown");
    assert!(combos.next().is_none(), "one mode dropdown");
    let node = node.accesskit_node();
    (node.is_disabled(), node.value().unwrap_or_default())
}

/// Pick `option` from the enabled mode dropdown, as a click on it and then on the option does.
fn pick(h: &mut Harness<'static, PhotocraftApp>, option: &str) {
    let modes = ["Gradient", "Classic gradient"];
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().value().is_some_and(|v| modes.contains(&v.as_str()))).expect("the mode dropdown").click();
    h.run_steps(2);
    h.query_all_by_label(option).find(|n| n.accesskit_node().role() != Role::ComboBox).expect("the option").click();
    h.run_steps(2);
}

fn gradient_of(app: &PhotocraftApp, id: LayerId) -> Fill {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Fill(f @ Fill::Gradient { .. }) => f.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

#[test]
fn a_gradient_fill_layer_greys_the_choice_and_reads_gradient() {
    for classic in [false, true] {
        let (mut app, _, _) = doc();
        app.ui.tool_options.gradient_classic = classic;
        let h = bar(app);
        assert_eq!(dropdown(&h), (true, "Gradient".into()), "Classic chosen before: {classic}");
        assert_eq!(h.state().ui.tool_options.gradient_classic, classic, "the stored choice is kept");
    }
}

#[test]
fn a_pixel_layer_enables_it_with_the_stored_choice() {
    for (classic, shown) in [(false, "Gradient"), (true, "Classic gradient")] {
        let (mut app, pixels, fill) = doc();
        app.ui.tool_options.gradient_classic = classic;
        let mut h = bar(app);
        assert!(dropdown(&h).0, "greyed on the fill layer");
        select(h.state_mut(), pixels);
        h.run_steps(2);
        assert_eq!(dropdown(&h), (false, shown.into()));
        // The re-enabled dropdown still records a new choice.
        let other = if classic { "Gradient" } else { "Classic gradient" };
        pick(&mut h, other);
        assert_eq!(dropdown(&h), (false, other.into()));
        assert_eq!(h.state().ui.tool_options.gradient_classic, !classic, "the new choice is stored");
        select(h.state_mut(), fill);
        h.run_steps(2);
        assert_eq!(dropdown(&h), (true, "Gradient".into()), "greyed again");
        assert_eq!(h.state().ui.tool_options.gradient_classic, !classic, "and kept");
    }
}

/// With Classic gradient chosen, a drag on the canvas edits the active fill layer's gradient instead
/// of painting pixels.
#[test]
fn a_drag_edits_the_fill_layer_with_classic_chosen() {
    let (mut app, pixels, fill) = doc();
    app.ui.tool_options.gradient_classic = true;
    assert!(crate::gradient_ui::live_mode(&app), "live on the fill layer");
    let before = gradient_of(&app, fill);
    let steps = app.session.active().unwrap().history.entries().len();
    // Through the canvas's own input path, as a mouse drag arrives.
    let none = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 100.0, pressure: 1.0 }, none);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 120.0, pressure: 1.0 }, none);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 280.0, y: 140.0 }, none);
    assert!(app.ui.dialogs.is_empty(), "no rasterize prompt");
    assert_ne!(gradient_of(&app, fill), before, "the fill layer's gradient changed");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one history step");
    // A pixel layer goes back to the stored Classic choice.
    select(&mut app, pixels);
    assert!(!crate::gradient_ui::live_mode(&app));
}

/// With Classic chosen the bar reads "Gradient" on the fill layer, so it is the live bar: Reverse
/// still edits the fill layer, and there is no blend Mode or Opacity, which only the Classic
/// gradient has (#12007). The pixel layer gets them back with the stored Classic choice.
#[test]
fn the_fill_layer_gets_the_live_bar_with_classic_chosen() {
    let (mut app, pixels, fill) = doc();
    app.ui.tool_options.gradient_classic = true;
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.gradient_reverse = !before.gradient_reverse;
    crate::gradient_ui::options_changed(&mut app, &before);
    assert!(matches!(gradient_of(&app, fill), Fill::Gradient { reverse, .. } if reverse != before.gradient_reverse), "Reverse");
    let mode_and_opacity = |h: &Harness<'static, PhotocraftApp>| {
        let mode = h.query_all_by_role(Role::ComboBox).any(|n| n.accesskit_node().value().as_deref() == Some("Normal"));
        (mode, h.query_by_label_contains("Opacity").is_some())
    };
    let mut h = bar(app);
    assert_eq!(mode_and_opacity(&h), (false, false), "the fill layer");
    select(h.state_mut(), pixels);
    h.run_steps(2);
    assert_eq!(mode_and_opacity(&h), (true, true), "the pixel layer");
}

/// Painting into the Quick Mask with a fill layer active was not measured: the stored choice
/// stands, enabled, as the tool then paints the Quick Mask, not the layer.
#[test]
fn the_quick_mask_as_target_keeps_the_choice() {
    let (mut app, _, _) = doc();
    app.ui.tool_options.gradient_classic = true;
    app.run("select.editInQuickMaskMode", json!({})).unwrap();
    assert!(!crate::gradient_ui::live_mode(&app));
    assert_eq!(dropdown(&bar(app)), (false, "Classic gradient".into()));
}

/// Hidden and locked fill layers were not measured: they keep today's behaviour (the stored
/// choice, enabled).
#[test]
fn hidden_and_locked_fill_layers_keep_the_choice() {
    for setup in ["hidden", "locked"] {
        let (mut app, _, fill) = doc();
        app.ui.tool_options.gradient_classic = true;
        let p = if setup == "hidden" { json!({"layer": fill.0, "visible": false}) } else { json!({"layer": fill.0, "locks": {"all": true}}) };
        app.run("layer.setProps", p).unwrap();
        assert!(!crate::gradient_ui::live_mode(&app), "{setup}");
        assert_eq!(dropdown(&bar(app)), (false, "Classic gradient".into()), "{setup}");
    }
}

/// A fill layer inside a hidden group is not shown, so like a hidden fill layer it keeps the
/// stored choice; inside a visible group it is greyed as usual.
#[test]
fn a_fill_layer_in_a_hidden_group_keeps_the_choice() {
    for hidden in [true, false] {
        let (mut app, _, fill) = doc();
        app.ui.tool_options.gradient_classic = true;
        app.run("layer.groupLayers", json!({})).unwrap();
        let group = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.setProps", json!({"layer": group.0, "visible": !hidden})).unwrap();
        select(&mut app, fill);
        assert_eq!(crate::gradient_ui::live_mode(&app), !hidden, "hidden group: {hidden}");
        let h = bar(app);
        let want = if hidden { (false, "Classic gradient") } else { (true, "Gradient") };
        assert_eq!(dropdown(&h), (want.0, want.1.into()), "hidden group: {hidden}");
        assert!(h.state().ui.tool_options.gradient_classic, "the stored Classic choice is kept");
    }
}

/// A fill layer made active while the list is open, as a script or a layer shortcut can do: a
/// pick from the still-open list doesn't overwrite the stored choice.
#[test]
fn an_open_list_does_not_overwrite_the_choice_once_a_fill_layer_is_active() {
    let (mut app, pixels, fill) = doc();
    app.ui.tool_options.gradient_classic = true;
    select(&mut app, pixels);
    let mut h = bar(app);
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Classic gradient")).expect("the mode dropdown").click();
    h.run_steps(2);
    select(h.state_mut(), fill);
    h.run_steps(2);
    assert_eq!(dropdown(&h), (true, "Gradient".into()));
    let option = h.query_all_by_label("Gradient").find(|n| n.accesskit_node().role() != Role::ComboBox).expect("the list stays open");
    option.click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.gradient_classic, "the stored Classic choice is kept");
}
