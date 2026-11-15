//! #5532: the Layers panel's Blend Mode list draws a line between Photoshop's six groups of modes:
//! Normal, Dissolve | Darken … Darker Color | Lighten … Lighter Color | Overlay … Hard Mix |
//! Difference … Divide | Hue … Luminosity.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;

/// The app at 1440×900 with a new document and a pixel layer above the Background, active.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app
    });
    h.run_steps(8);
    h
}

/// The Layers panel's dropdown: the lowest one (the options bar and the title bar have some too).
fn dropdown(h: &Harness<'_, PhotocraftApp>) -> Rect {
    h.query_all_by_role(Role::ComboBox).map(|n| n.rect()).max_by(|a, b| a.top().total_cmp(&b.top())).unwrap()
}

/// The open list's rect (the one foreground area: no other popup is open).
fn open_list(h: &Harness<'_, PhotocraftApp>) -> Option<Rect> {
    h.ctx.memory(|m| m.layer_ids().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)))
}

fn blend(h: &Harness<'_, PhotocraftApp>) -> BlendMode {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

fn steps(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// Presses at `at`, holds for `held` frames and releases there.
fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, held: usize) {
    h.hover_at(at);
    h.run_steps(1);
    for (pressed, frames) in [(true, held), (false, 4)] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(frames);
    }
}

#[test]
fn the_layers_blend_mode_list_has_a_line_between_each_group() {
    let mut h = harness();
    let at = dropdown(&h).center();
    click(&mut h, at, 1);
    assert!(open_list(&h).is_some(), "the list is open");
    // Each mode's row in the open list: its label's last node (the box shows the chosen one's
    // first). Rows scrolled out of the list keep their place in it, so the gaps hold for all.
    let rows: Vec<Rect> = BlendMode::LAYER_MODES
        .iter()
        .map(|m| h.get_all_by_label(m.label()).map(|n| n.rect()).next_back().unwrap_or_else(|| panic!("{m:?} is in the list")))
        .collect();
    let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].top() - w[0].bottom()).collect();
    let row_gap = gaps.iter().copied().fold(f32::INFINITY, f32::min);
    let lines: Vec<(BlendMode, BlendMode)> =
        gaps.iter().enumerate().filter(|(_, g)| **g > row_gap + 2.0).map(|(i, _)| (BlendMode::LAYER_MODES[i], BlendMode::LAYER_MODES[i + 1])).collect();
    assert_eq!(
        lines,
        [
            (BlendMode::Dissolve, BlendMode::Darken),
            (BlendMode::DarkerColor, BlendMode::Lighten),
            (BlendMode::LighterColor, BlendMode::Overlay),
            (BlendMode::HardMix, BlendMode::Difference),
            (BlendMode::Divide, BlendMode::Hue),
        ],
        "row gaps {gaps:?}"
    );
}

/// A plain click on the Layers panel's Blend Mode box at 1440×900, released where it was pressed
/// (after one frame, or after a few as a person's click takes), opens the list beside the box and
/// chooses nothing: the list doesn't open over the box, where the release would pick a row
/// (#5495 review of a taller list).
#[test]
fn a_click_on_the_blend_mode_box_opens_the_list_and_changes_nothing() {
    for held in [6, 1] {
        let mut h = harness();
        let undo = steps(&h);
        let button = dropdown(&h);
        click(&mut h, button.center(), held);
        assert_eq!((blend(&h), steps(&h)), (BlendMode::Normal, undo), "held {held}: the click chose nothing");
        let Some(list) = open_list(&h) else { panic!("held {held}: the list is open") };
        assert!(!list.intersects(button.shrink(1.0)), "held {held}: the list {list:?} leaves the box {button:?} free");
        assert!(list.top() >= 0.0 && list.bottom() <= 900.0, "held {held}: the list {list:?} is in the window");
    }
}
