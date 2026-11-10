//! The Mixer Brush's Useful mixer brush combinations dropdown (#11946): between the Clean after
//! each stroke toggle and Wet, Custom then the reference's twelve entries in four groups; each
//! sets Wet, Load and Mix in one `tools.setBrush` change and leaves Flow. Reset Tool shows Custom.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::{MIXER_COMBINATION_GROUPS, MIXER_COMBINATIONS};
use crate::PhotocraftApp;
use crate::state::Tool;

const TIP: &str = "Useful mixer brush combinations";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"mixer": {"flow": 0.42}}})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1800.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

/// The combinations dropdown, by the entry it shows.
fn dropdown_rect(h: &Harness<'static, PhotocraftApp>, shows: &str) -> egui::Rect {
    h.get_all_by_role(egui::accesskit::Role::ComboBox)
        .find(|n| n.accesskit_node().value().as_deref() == Some(shows))
        .unwrap_or_else(|| panic!("no dropdown showing {shows}"))
        .rect()
}

fn open_and_pick(h: &mut Harness<'static, PhotocraftApp>, shows: &str, entry: &str) {
    h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some(shows)).unwrap().click();
    h.run_steps(3);
    entry_node(h, entry).click();
    h.run_steps(3);
}

/// An entry of the open list (not the Wet field, which shares the "Wet" label).
fn entry_node<'a>(h: &'a Harness<'static, PhotocraftApp>, entry: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(entry).rfind(|n| n.accesskit_node().role() != egui::accesskit::Role::SpinButton).unwrap_or_else(|| panic!("no entry {entry}"))
}

fn mixer(h: &Harness<'static, PhotocraftApp>) -> (f32, f32, f32, f32) {
    let m = &h.state().session.tools.brush.mixer;
    (m.wet, m.load, m.mix, m.flow)
}

fn set_brush_count(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.journal.iter().filter(|(id, _)| id == "tools.setBrush").count()
}

#[test]
fn the_dropdown_sits_before_wet_with_the_reference_entries_in_order() {
    let mut h = harness();
    if !has_fonts(&h) {
        return;
    }
    let combo = dropdown_rect(&h, "Custom");
    let clean = h.get_by_label("Clean the brush after each stroke").rect();
    let wet = h.get_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Wet")).unwrap().rect();
    assert!(clean.right() <= combo.left() && combo.right() <= wet.left(), "clean {clean:?}, dropdown {combo:?}, wet {wet:?}");
    assert_eq!(
        h.get_all_by_role(egui::accesskit::Role::ComboBox)
            .find(|n| n.accesskit_node().value().as_deref() == Some("Custom"))
            .unwrap()
            .accesskit_node()
            .label()
            .as_deref(),
        Some(TIP)
    );
    let labels: Vec<&str> = MIXER_COMBINATIONS.iter().map(|(l, _)| *l).collect();
    assert_eq!(
        labels,
        [
            "Custom",
            "Dry",
            "Dry, Light Load",
            "Dry, Heavy Load",
            "Moist",
            "Moist, Light Mix",
            "Moist, Heavy Mix",
            "Wet",
            "Wet, Light Mix",
            "Wet, Heavy Mix",
            "Very Wet",
            "Very Wet, Light Mix",
            "Very Wet, Heavy Mix"
        ]
    );
    // Separators above Dry, Moist, Wet and Very Wet.
    let groups: Vec<&str> = MIXER_COMBINATION_GROUPS.iter().map(|&i| MIXER_COMBINATIONS[i].0).collect();
    assert_eq!(groups, ["Dry", "Moist", "Wet", "Very Wet"]);
    // The open list shows the entries top to bottom in that order.
    h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Custom")).unwrap().click();
    h.run_steps(3);
    let tops: Vec<f32> = labels[1..].iter().map(|l| entry_node(&h, l).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "entries out of order: {tops:?}");
}

#[test]
fn each_entry_sets_wet_load_and_mix_in_one_change_and_keeps_flow() {
    let mut h = harness();
    if !has_fonts(&h) {
        return;
    }
    let expected = [
        ("Dry", (0.0, 0.5, 0.0)),
        ("Dry, Light Load", (0.0, 0.05, 0.0)),
        ("Dry, Heavy Load", (0.0, 1.0, 0.0)),
        ("Moist", (0.1, 0.05, 0.5)),
        ("Moist, Light Mix", (0.1, 0.05, 0.0)),
        ("Moist, Heavy Mix", (0.1, 0.05, 1.0)),
        ("Wet", (0.5, 0.5, 0.5)),
        ("Wet, Light Mix", (0.5, 0.5, 0.0)),
        ("Wet, Heavy Mix", (0.5, 0.5, 1.0)),
        ("Very Wet", (1.0, 0.5, 0.5)),
        ("Very Wet, Light Mix", (1.0, 0.5, 0.0)),
        ("Very Wet, Heavy Mix", (1.0, 0.5, 1.0)),
    ];
    let mut shows = "Custom";
    for (entry, (wet, load, mix)) in expected {
        let before = set_brush_count(&h);
        open_and_pick(&mut h, shows, entry);
        assert_eq!(mixer(&h), (wet, load, mix, 0.42), "{entry}");
        assert_eq!(set_brush_count(&h), before + 1, "{entry}: one tools.setBrush change");
        assert_eq!(h.state().ui.tool_options.mixer_combination, MIXER_COMBINATIONS.iter().position(|(l, _)| *l == entry).unwrap());
        dropdown_rect(&h, entry);
        shows = entry;
    }
}

#[test]
fn reset_tool_shows_custom_and_a_typed_value_keeps_the_pick() {
    let mut h = harness();
    if !has_fonts(&h) {
        return;
    }
    open_and_pick(&mut h, "Custom", "Wet, Heavy Mix");
    // A typed Wet keeps showing the picked entry (not measured; today's behaviour).
    h.get_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(50.0)).expect("Wet field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("30".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(mixer(&h).0, 0.3);
    dropdown_rect(&h, "Wet, Heavy Mix");
    // Reset Tool: Custom again.
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.mixer_combination, 0);
    dropdown_rect(&h, "Custom");
}
