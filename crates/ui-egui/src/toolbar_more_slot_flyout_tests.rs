//! Right-click on the toolbar's "…" button opens a one-row flyout, "Edit Toolbar…"; the row runs
//! `edit.toolbar`, Escape or a press outside closes it, and a left click still opens Edit Toolbar.

use egui::{Event, Pos2, Rect, vec2};

use crate::PhotocraftApp;
use crate::panels::toolbar::{TOOL_SECTIONS, toolbar};
use crate::theme::Tokens;

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) {
    let mut out = ctx.run_ui(
        egui::RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
        |ui| toolbar(app, ui),
    );
    out.textures_delta.clear();
}

fn setup() -> (PhotocraftApp, egui::Context, Pos2) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(bx, bx) && w.sense.senses_click()).map(|w| w.rect).collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    // The "…" button sits right below the last tool slot.
    let slots = TOOL_SECTIONS.iter().map(|s| s.len()).sum::<usize>();
    let at = buttons[slots].center();
    (app, ctx, at)
}

fn button(pos: Pos2, which: egui::PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button: which, pressed, modifiers: Default::default() }
}

fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, at: Pos2, t: f64) {
    frame(app, ctx, t, vec![Event::PointerMoved(at), button(at, egui::PointerButton::Secondary, true)]);
    frame(app, ctx, t + 0.05, vec![button(at, egui::PointerButton::Secondary, false)]);
    frame(app, ctx, t + 0.1, vec![]);
}

fn open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("toolbar-more-flyout-open"))).unwrap_or(false)
}

fn flyout(ctx: &egui::Context) -> Option<Rect> {
    ctx.memory(|m| m.area_rect(egui::Id::unique("toolbar-more-flyout")))
}

#[test]
fn right_click_on_more_opens_a_flyout_with_one_row() {
    let (mut app, ctx, at) = setup();
    assert!(!open(&ctx));
    right_click(&mut app, &ctx, at, 1.0);
    assert!(open(&ctx));
    let rect = flyout(&ctx).expect("the flyout is drawn");
    assert!(rect.height() < 50.0, "one 26 point row, not a list: {rect:?}");
    assert!(app.ui.dialogs.is_empty(), "opening the flyout runs nothing");
}

#[test]
fn choosing_edit_toolbar_runs_it_and_closes_the_flyout() {
    let (mut app, ctx, at) = setup();
    right_click(&mut app, &ctx, at, 1.0);
    let row = flyout(&ctx).unwrap().center();
    frame(&mut app, &ctx, 1.95, vec![Event::PointerMoved(row)]);
    frame(&mut app, &ctx, 2.0, vec![button(row, egui::PointerButton::Primary, true)]);
    frame(&mut app, &ctx, 2.05, vec![]);
    frame(&mut app, &ctx, 2.1, vec![button(row, egui::PointerButton::Primary, false)]);
    frame(&mut app, &ctx, 2.15, vec![]);
    assert_eq!(app.ui.dialogs.len(), 1, "edit.toolbar opened its dialog");
    assert!(!open(&ctx));
}

#[test]
fn escape_closes_the_flyout_without_running_anything() {
    let (mut app, ctx, at) = setup();
    right_click(&mut app, &ctx, at, 1.0);
    let key = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(&mut app, &ctx, 2.0, vec![key]);
    assert!(!open(&ctx));
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn a_press_outside_closes_the_flyout_without_running_anything() {
    let (mut app, ctx, at) = setup();
    right_click(&mut app, &ctx, at, 1.0);
    let away = Pos2::new(900.0, 900.0);
    frame(&mut app, &ctx, 2.0, vec![Event::PointerMoved(away), button(away, egui::PointerButton::Primary, true)]);
    frame(&mut app, &ctx, 2.05, vec![button(away, egui::PointerButton::Primary, false)]);
    assert!(!open(&ctx));
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn a_left_click_still_opens_edit_toolbar_directly() {
    let (mut app, ctx, at) = setup();
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), button(at, egui::PointerButton::Primary, true)]);
    frame(&mut app, &ctx, 1.05, vec![button(at, egui::PointerButton::Primary, false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(!open(&ctx), "no flyout for a left click");
}

#[test]
fn a_failed_edit_toolbar_from_the_flyout_reports_its_error_in_the_status_bar() {
    let (mut app, ctx, at) = setup();
    app.automation_input = true;
    app.services.automation_command = Some(Box::new(|id, _| if id == "edit.toolbar" { Err("toolbar edits denied".into()) } else { Ok(()) }));
    right_click(&mut app, &ctx, at, 1.0);
    let row = flyout(&ctx).unwrap().center();
    frame(&mut app, &ctx, 1.95, vec![Event::PointerMoved(row)]);
    frame(&mut app, &ctx, 2.0, vec![button(row, egui::PointerButton::Primary, true)]);
    frame(&mut app, &ctx, 2.05, vec![]);
    frame(&mut app, &ctx, 2.1, vec![button(row, egui::PointerButton::Primary, false)]);
    frame(&mut app, &ctx, 2.15, vec![]);
    assert!(app.ui.dialogs.is_empty(), "the denied command opened nothing");
    assert!(!open(&ctx));
    assert_eq!(app.ui.status, "toolbar edits denied");
    assert!(app.ui.status_error);
}
