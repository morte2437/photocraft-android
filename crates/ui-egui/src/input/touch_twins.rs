//! One tap, one press (#2095). In the browser eframe turns a finger or pen tap into two presses
//! and two releases: one pair from Pointer Events (`pointerdown` / `pointerup`) and one from Touch
//! Events (`touchstart` / `touchend`), which mobile browsers fire for the same contact. A menu
//! title or press menu toggles on the press, so the second press closed the menu the first had
//! just opened. [`drop_twins`] keeps the first of each and drops its twin.

use egui::{Event, PointerButton, RawInput};

/// Longest gap, in seconds, between a press and its twin from the other event family (both come
/// from one contact, normally within a frame or two).
const TWIN_WINDOW: f64 = 0.5;

/// Drops a primary press that arrives while the primary button is already held (pressed within
/// [`TWIN_WINDOW`]) and a primary release that arrives while it is already up. A press while held
/// for longer still passes, so a release the browser never sent can't lock the pointer down.
/// Call from `raw_input_hook`, before the frame runs.
pub fn drop_twins(ctx: &egui::Context, raw: &mut RawInput) {
    let (mut down, mut pressed_at, last) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.press_start_time(), i.time));
    let now = raw.time.unwrap_or(last + f64::from(raw.predicted_dt));
    raw.events.retain(|e| {
        let Event::PointerButton { button: PointerButton::Primary, pressed, .. } = e else { return true };
        if *pressed != down {
            down = *pressed;
            pressed_at = pressed.then_some(now);
            return true;
        }
        // A press while held is a twin only while fresh; a release while up always is.
        *pressed && !pressed_at.is_some_and(|t| now - t <= TWIN_WINDOW)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, Pos2, pos2};

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    fn touch(pos: Pos2, phase: egui::TouchPhase) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(7), phase, pos, force: None }
    }

    /// One browser tap as eframe delivers it: `pointerdown` runs a frame of its own, then
    /// `touchstart`, `pointerup` (again a frame of its own) and `touchend`.
    fn tap_frames(pos: Pos2) -> [Vec<Event>; 4] {
        [
            vec![button(pos, true)],
            vec![button(pos, true), touch(pos, egui::TouchPhase::Start)],
            vec![button(pos, false)],
            vec![button(pos, false), Event::PointerGone, touch(pos, egui::TouchPhase::End)],
        ]
    }

    #[test]
    fn a_tap_opens_a_menu_title_every_time() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app| {
                crate::menus::menu_bar(app, ui);
            },
            app,
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.run_steps(3);
        let file = harness.get_by_label("File").rect().center();
        let ctx = harness.ctx.clone();
        let tap = |harness: &mut Harness<'_, crate::PhotocraftApp>, dedupe: bool| {
            for events in tap_frames(file) {
                harness.input_mut().events = events;
                if dedupe {
                    drop_twins(&ctx, harness.input_mut());
                }
                harness.step();
            }
            harness.run_steps(2);
            harness.query_by_label_contains("Open…").is_some()
        };
        // Without the hook the second press closes the menu the first opened.
        assert!(!tap(&mut harness, false), "the cause: the twin press closes the menu");
        assert!(tap(&mut harness, true), "the first tap opens File");
        assert!(!tap(&mut harness, true), "a second tap closes it");
        assert!(tap(&mut harness, true), "a third tap opens it again");
    }

    /// A fast double tap delivered in one frame: each event changes the button's state, so none is
    /// a twin.
    #[test]
    fn a_press_release_press_in_one_frame_keeps_all_three() {
        let ctx = egui::Context::default();
        let p = pos2(10.0, 10.0);
        let mut raw = RawInput { time: Some(1.0), events: vec![button(p, true), button(p, false), button(p, true)], ..Default::default() };
        drop_twins(&ctx, &mut raw);
        assert_eq!(raw.events.len(), 3);
    }

    #[test]
    fn a_press_held_for_long_still_passes() {
        let ctx = egui::Context::default();
        let p = pos2(10.0, 10.0);
        let frame = |events: Vec<Event>, time: f64| {
            let mut raw = RawInput { time: Some(time), events, ..Default::default() };
            drop_twins(&ctx, &mut raw);
            let kept = raw.events.clone();
            ctx.run_ui(raw, |_| {}).textures_delta.clear();
            kept
        };
        assert_eq!(frame(vec![button(p, true)], 1.0).len(), 1);
        assert!(frame(vec![button(p, true)], 1.1).is_empty(), "the twin press");
        // The release never came: a later press is a new one, not a twin.
        assert_eq!(frame(vec![button(p, true)], 3.0).len(), 1);
        assert_eq!(frame(vec![button(p, false), button(p, false)], 3.1).len(), 1, "one release per press");
        assert!(frame(vec![button(p, false)], 3.2).is_empty(), "a release while up");
    }
}
