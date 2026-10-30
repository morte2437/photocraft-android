//! Which fields hold numbers. They're dragged, scrubbed or set with a slider, so a touch screen
//! doesn't open its keyboard for them: the Android shell asks [`number_focused`].

use egui::{Context, Id, Response, Ui};

fn key() -> Id {
    Id::unique("photocraft-number-fields")
}

/// `field` (a widget's id) is a number field this frame.
pub fn mark(ctx: &Context, field: Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let (at, ids) = d.get_temp_mut_or_default::<(u64, Vec<Id>)>(key());
        if *at != pass {
            *at = pass;
            ids.clear();
        }
        ids.push(field);
    });
}

/// Whether the focused field is a number field (as of the last frame that drew one).
pub fn number_focused(ctx: &Context) -> bool {
    let Some(focused) = ctx.memory(|m| m.focused()) else { return false };
    ctx.data(|d| d.get_temp::<(u64, Vec<Id>)>(key())).is_some_and(|(_, ids)| ids.contains(&focused))
}

/// Adds a [`egui::DragValue`], marked as a number field.
pub fn drag_value(ui: &mut Ui, value: egui::DragValue<'_>) -> Response {
    let r = ui.add(value);
    mark(ui.ctx(), r.id);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focused_number_field_is_told_from_a_text_field() {
        let ctx = Context::default();
        let (mut n, mut text) = (5.0f32, String::from("abc"));
        let mut ids = (Id::NULL, Id::NULL);
        for focus in [None, Some(0), Some(1)] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let number = drag_value(ui, egui::DragValue::new(&mut n));
                let field = ui.text_edit_singleline(&mut text);
                ids = (number.id, field.id);
                match focus {
                    Some(0) => number.request_focus(),
                    Some(1) => field.request_focus(),
                    _ => {}
                }
            });
            out.textures_delta.clear();
            let want = focus == Some(0);
            assert_eq!(number_focused(&ctx), want, "{focus:?}");
        }
        assert_ne!(ids.0, ids.1);
    }
}
