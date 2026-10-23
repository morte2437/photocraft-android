//! The Move tool's Align and Distribute controls (Photoshop shows them whenever the Move tool is
//! active; they enable once enough layers are selected). Every button dispatches an existing
//! `layer.align.*` / `layer.distribute.*` command.

use serde_json::json;

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// (command id, icon, label). The labels are the Layer menu's, already translated.
const ALIGN: [(&str, &str, &str); 6] = [
    ("layer.align.leftEdges", "align-edge-left", "Left Edges"),
    ("layer.align.horizontalCenters", "align-edge-hcenter", "Horizontal Centers"),
    ("layer.align.rightEdges", "align-edge-right", "Right Edges"),
    ("layer.align.topEdges", "align-edge-top", "Top Edges"),
    ("layer.align.verticalCenters", "align-edge-vcenter", "Vertical Centers"),
    ("layer.align.bottomEdges", "align-edge-bottom", "Bottom Edges"),
];

const DISTRIBUTE: [(&str, &str, &str); 8] = [
    ("layer.distribute.topEdges", "distribute-top", "Top Edges"),
    ("layer.distribute.verticalCenters", "distribute-vcenter", "Vertical Centers"),
    ("layer.distribute.bottomEdges", "distribute-bottom", "Bottom Edges"),
    ("layer.distribute.leftEdges", "distribute-left", "Left Edges"),
    ("layer.distribute.horizontalCenters", "distribute-hcenter", "Horizontal Centers"),
    ("layer.distribute.rightEdges", "distribute-right", "Right Edges"),
    ("layer.distribute.horizontally", "distribute-horizontal", "Horizontally"),
    ("layer.distribute.vertically", "distribute-vertical", "Vertically"),
];

/// The six align icons followed by a menu holding both groups with icons and names.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.x = 0.0;
    for (cmd, icon, label) in ALIGN {
        let tip = format!("{}: {}", tl!("Align"), tl!(label));
        let on = app.session.is_enabled(cmd);
        if ui.add_enabled_ui(on, |ui| icons::button(ui, icon, 24.0, false, &tip)).inner.clicked() {
            let _ = app.run(cmd, json!({}));
        }
    }
    ui.add_space(6.0);
    let more = icons::button(ui, "ellipsis", 24.0, false, tl!("Align and distribute"));
    egui::Popup::menu(&more).show(|ui| {
        ui.set_min_width(200.0);
        let mut section = |ui: &mut egui::Ui, title: &str, items: &[(&str, &str, &str)]| {
            ui.label(egui::RichText::new(title).small().color(t.text_dim));
            for (cmd, icon, label) in items {
                let on = app.session.is_enabled(cmd);
                let tint = if on { t.icon } else { t.text_dim };
                let button = egui::Button::image_and_text(icons::image(icon, 16.0, tint), tl!(label));
                if ui.add_enabled(on, button).clicked() {
                    let _ = app.run(cmd, json!({}));
                    ui.close();
                }
            }
        };
        section(ui, tl!("Align"), &ALIGN);
        ui.separator();
        section(ui, tl!("Distribute"), &DISTRIBUTE);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_is_a_live_command_with_an_icon() {
        let specs = photocraft_engine::command_specs();
        for (cmd, icon, _) in ALIGN.iter().chain(DISTRIBUTE.iter()) {
            assert!(specs.iter().any(|c| c.id == *cmd), "{cmd} is not a command");
            assert!(icons::exists(icon), "{icon} is not embedded");
        }
    }
}
