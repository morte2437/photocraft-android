//! The healing and brush slots list their tools in the reference order, the healing slot starts on
//! Spot Healing Brush, and J, B, Alt-click and Shift+letter all follow that order.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::toolbar::{TOOL_SECTIONS, next_slot_tool, toolbar};
use crate::state::Tool;

const HEALING: [Tool; 6] = [Tool::SpotHealing, Tool::Remove, Tool::Healing, Tool::Patch, Tool::ContentAwareMove, Tool::RedEye];
const BRUSH: [Tool; 4] = [Tool::Brush, Tool::Pencil, Tool::ColorReplacement, Tool::MixerBrush];

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn slot_with(tool: Tool) -> &'static [Tool] {
    TOOL_SECTIONS.iter().flat_map(|s| s.iter()).find(|slot| slot.contains(&tool)).copied().unwrap()
}

fn shift_sequence(key: Key, steps: usize) -> Vec<Tool> {
    let mut app = app();
    app.ui.tool = Tool::Hand;
    (0..steps)
        .map(|_| {
            press(&mut app, key, Modifiers::SHIFT);
            app.ui.tool
        })
        .collect()
}

#[test]
fn the_healing_slot_lists_spot_healing_first() {
    assert_eq!(slot_with(Tool::Healing), &HEALING);
}

#[test]
fn the_brush_slot_lists_color_replacement_before_mixer_brush() {
    assert_eq!(slot_with(Tool::Brush), &BRUSH);
}

#[test]
fn a_fresh_healing_slot_shows_spot_healing() {
    assert_eq!(slot_with(Tool::Remove)[0], Tool::SpotHealing);
}

#[test]
fn j_on_fresh_settings_selects_spot_healing() {
    let mut app = app();
    app.ui.tool = Tool::Hand;
    press(&mut app, Key::J, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::SpotHealing);
}

#[test]
fn tool_all_orders_the_two_pairs_like_the_slots() {
    let pos = |t: Tool| Tool::ALL.iter().position(|x| *x == t).unwrap();
    assert!(pos(Tool::SpotHealing) < pos(Tool::Remove));
    assert!(pos(Tool::ColorReplacement) < pos(Tool::MixerBrush));
}

#[test]
fn alt_click_steps_both_slots_in_the_new_order_and_wraps() {
    for order in [&HEALING[..], &BRUSH[..]] {
        for (i, from) in order.iter().enumerate() {
            assert_eq!(next_slot_tool(slot_with(*from), *from), order[(i + 1) % order.len()], "from {from:?}");
        }
    }
}

#[test]
fn shift_j_follows_the_reference_sequence() {
    let want: Vec<Tool> = HEALING.iter().chain(&HEALING[..1]).copied().collect();
    assert_eq!(shift_sequence(Key::J, 7), want);
}

#[test]
fn shift_b_follows_the_reference_sequence() {
    let want: Vec<Tool> = BRUSH.iter().chain(&BRUSH[..1]).copied().collect();
    assert_eq!(shift_sequence(Key::B, 5), want);
}

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    let mut out = ctx.run_ui(
        egui::RawInput {
            time: Some(time),
            events,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 1800.0))),
            ..Default::default()
        },
        |ui| toolbar(app, ui),
    );
    out.textures_delta.clear();
}

/// Alt-click on the slot button, through the pointer, steps the reordered healing and brush slots.
#[test]
fn pointer_alt_click_steps_the_healing_and_brush_slots_in_the_new_order() {
    let mut app = app();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if crate::theme::Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<egui::Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let alt = Modifiers { alt: true, ..Default::default() };
    let pointer = |pos, pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let mut t = 1.0;
    for order in [&HEALING[..], &BRUSH[..]] {
        let index = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).position(|slot| slot.contains(&order[0])).unwrap();
        let at = buttons[index].center();
        app.ui.tool = order[0];
        for want in order.iter().cycle().skip(1).take(order.len()) {
            frame(&mut app, &ctx, t, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
            frame(&mut app, &ctx, t + 0.05, vec![egui::Event::ModifiersChanged(Default::default()), pointer(at, false, alt)]);
            frame(&mut app, &ctx, t + 0.1, vec![]);
            assert_eq!(app.ui.tool, *want, "alt-click at {t}");
            t += 0.5;
        }
    }
}
