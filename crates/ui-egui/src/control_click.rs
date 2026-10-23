//! Normalize macOS Control-click before egui routes clicks to panels and canvas tools.
#[derive(Default)]
pub(crate) struct ControlClick {
    secondary_down: bool,
}

impl ControlClick {
    pub fn normalize(&mut self, events: &mut [egui::Event], macos: bool) {
        if !macos {
            return;
        }
        for event in events {
            match event {
                egui::Event::PointerButton { button, pressed, modifiers, .. } if *button == egui::PointerButton::Primary => {
                    if *pressed {
                        // Control-Option is the existing brush resize gesture.
                        self.secondary_down = modifiers.ctrl && !modifiers.alt && !modifiers.command;
                    }
                    if self.secondary_down {
                        *button = egui::PointerButton::Secondary;
                    }
                    if !*pressed {
                        self.secondary_down = false;
                    }
                }
                // A drag keeps the mapping when the pointer leaves the window or focus is lost: the
                // release still arrives and must match the secondary press egui already saw. The
                // next press recomputes it, so a missed release can't remap a plain click.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton};

    fn button(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: PointerButton::Primary, pressed, modifiers }
    }

    #[test]
    fn control_click_opens_context_menu_even_when_control_is_released_first() {
        let ctx = egui::Context::default();
        let mut state = ControlClick::default();
        let mut secondary = false;
        let mut primary = false;
        for mut events in [vec![], vec![Event::PointerMoved(egui::pos2(50.0, 50.0)), button(true, Modifiers::CTRL)], vec![button(false, Modifiers::NONE)]] {
            state.normalize(&mut events, true);
            let raw = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(raw, |ui| {
                let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click());
                secondary |= r.secondary_clicked();
                primary |= r.clicked();
                r.context_menu(|ui| {
                    ui.label("Layer context menu");
                });
            })
            .textures_delta
            .clear();
        }
        assert!(secondary);
        assert!(!primary);
    }

    #[test]
    fn other_platforms_and_existing_modifier_gestures_stay_primary() {
        for (macos, mods) in
            [(false, Modifiers::CTRL), (true, Modifiers::NONE), (true, Modifiers::CTRL | Modifiers::ALT), (true, Modifiers::CTRL | Modifiers::COMMAND)]
        {
            let mut events = vec![button(true, mods), button(false, mods)];
            ControlClick::default().normalize(&mut events, macos);
            assert!(events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
        }
    }

    #[test]
    fn a_control_drag_out_of_the_window_and_back_releases_the_secondary_button() {
        let ctx = egui::Context::default();
        let mut state = ControlClick::default();
        let frames = [
            vec![Event::PointerMoved(egui::pos2(50.0, 50.0)), button(true, Modifiers::CTRL)],
            vec![Event::PointerGone],
            vec![Event::WindowFocused(false)],
            vec![Event::PointerMoved(egui::pos2(60.0, 60.0))],
            vec![button(false, Modifiers::NONE)],
        ];
        let mut down = Vec::new();
        for mut events in frames {
            state.normalize(&mut events, true);
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                down.push(ui.input(|i| (i.pointer.secondary_down(), i.pointer.primary_down())));
            })
            .textures_delta
            .clear();
        }
        assert_eq!(down.first(), Some(&(true, false)), "the Control press is a secondary press");
        assert_eq!(down.last(), Some(&(false, false)), "the release after re-entry ends it: no stuck button");
    }

    #[test]
    fn focus_loss_does_not_remap_the_next_plain_click() {
        let mut state = ControlClick::default();
        state.normalize(&mut [button(true, Modifiers::CTRL)], true);
        state.normalize(&mut [Event::WindowFocused(false)], true);
        let mut events = [button(true, Modifiers::NONE), button(false, Modifiers::NONE)];
        state.normalize(&mut events, true);
        assert!(events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
    }
}
