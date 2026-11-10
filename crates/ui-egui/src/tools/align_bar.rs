//! The Move tool's Align and Distribute controls (Photoshop shows them whenever the Move tool is
//! active; they enable once enough layers are selected). Every button dispatches an existing
//! `layer.align.*` / `layer.distribute.*` command.
//!
//! The bar reads, after Show Transform Controls: Align left, horizontal centres, right and
//! Distribute vertically; Align top, vertical centres, bottom and Distribute horizontally; the
//! "..." button (a compact icon panel); the gear (the Move tool's three hover and group toggles,
//! #11381).

use serde_json::json;

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// (command id, icon, label). The labels are the Layer menu's, already translated.
pub(crate) const ALIGN: [(&str, &str, &str); 6] = [
    ("layer.align.leftEdges", "align-edge-left", "Left Edges"),
    ("layer.align.horizontalCenters", "align-edge-hcenter", "Horizontal Centers"),
    ("layer.align.rightEdges", "align-edge-right", "Right Edges"),
    ("layer.align.topEdges", "align-edge-top", "Top Edges"),
    ("layer.align.verticalCenters", "align-edge-vcenter", "Vertical Centers"),
    ("layer.align.bottomEdges", "align-edge-bottom", "Bottom Edges"),
];

/// The panel's Distribute group: top, vertical centres, bottom, left, horizontal centres, right.
const DISTRIBUTE: [(&str, &str, &str); 6] = [
    ("layer.distribute.topEdges", "distribute-top", "Top Edges"),
    ("layer.distribute.verticalCenters", "distribute-vcenter", "Vertical Centers"),
    ("layer.distribute.bottomEdges", "distribute-bottom", "Bottom Edges"),
    ("layer.distribute.leftEdges", "distribute-left", "Left Edges"),
    ("layer.distribute.horizontalCenters", "distribute-hcenter", "Horizontal Centers"),
    ("layer.distribute.rightEdges", "distribute-right", "Right Edges"),
];

/// One bar button: (command id, icon, tooltip).
type BarButton = (&'static str, &'static str, &'static str);

/// The two button groups of the bar, in order. The tooltips are the reference application's.
fn bar_groups() -> [[BarButton; 4]; 2] {
    [
        [
            ("layer.align.leftEdges", "align-edge-left", tl!("Align left edges")),
            ("layer.align.horizontalCenters", "align-edge-hcenter", tl!("Align horizontal centers")),
            ("layer.align.rightEdges", "align-edge-right", tl!("Align right edges")),
            ("layer.distribute.vertically", "distribute-vertical", tl!("Distribute vertically")),
        ],
        [
            ("layer.align.topEdges", "align-edge-top", tl!("Align top edges")),
            ("layer.align.verticalCenters", "align-edge-vcenter", tl!("Align vertical centers")),
            ("layer.align.bottomEdges", "align-edge-bottom", tl!("Align bottom edges")),
            ("layer.distribute.horizontally", "distribute-horizontal", tl!("Distribute horizontally")),
        ],
    ]
}

/// Whether the "..." panel is open: a click on the button toggles it and nothing else closes it
/// (Escape included), as in the reference application.
pub(crate) fn panel_open_id() -> egui::Id {
    egui::Id::unique("move-align-panel-open")
}

pub(crate) fn panel_is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(panel_open_id())).unwrap_or(false)
}

/// The two groups of four icons, then the "..." button and its panel. Each icon uses 20 pt
/// artwork inside a 28 pt hit target for high-DPI legibility without changing enabled-state or
/// command dispatch (#8849).
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 0.0;
    for group in bar_groups() {
        for (cmd, icon, tip) in group {
            let on = app.session.is_enabled(cmd);
            if ui.add_enabled_ui(on, |ui| icons::button_with_icon_size(ui, icon, 28.0, 20.0, false, tip)).inner.clicked() {
                let _ = app.run(cmd, json!({}));
            }
        }
        ui.add_space(6.0);
    }
    let more = icons::button_with_icon_size(ui, "ellipsis", 28.0, 20.0, false, tl!("Align and distribute"));
    let ctx = ui.ctx().clone();
    let open = panel_is_open(&ctx);
    if more.clicked() {
        ctx.data_mut(|d| d.insert_temp(panel_open_id(), !open));
    }
    if panel_is_open(&ctx) {
        panel(app, &ctx, more.rect);
    }
    ui.add_space(6.0);
}

/// The compact icon panel under the "..." button: Align (6) and Distribute (6). The Distribute
/// Spacing group (#10715) and the Align To dropdown (#10358) go below these as further `group`
/// rows.
fn panel(app: &mut PhotocraftApp, ctx: &egui::Context, anchor: egui::Rect) {
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("move-align-panel")).order(egui::Order::Foreground).fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 2.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                let mut group = |ui: &mut egui::Ui, title: &str, kind: &str, items: &[(&str, &str, &str)]| {
                    ui.label(egui::RichText::new(title).small().color(t.text_dim));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (cmd, icon, label) in items {
                            let tip = format!("{kind}: {}", tl!(label));
                            let on = app.session.is_enabled(cmd);
                            if ui.add_enabled_ui(on, |ui| icons::button_with_icon_size(ui, icon, 28.0, 20.0, false, &tip)).inner.clicked() {
                                let _ = app.run(cmd, json!({}));
                            }
                        }
                    });
                };
                group(ui, tl!("Align"), tl!("Align"), &ALIGN);
                ui.add_space(4.0);
                group(ui, tl!("Distribute"), tl!("Distribute"), &DISTRIBUTE);
            });
        },
    );
}

/// The gear after the "..." button: a menu of the Move tool's three check items, all on by
/// default and saved with the tool options.
pub fn gear(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let gear = icons::button_with_icon_size(ui, "settings", 28.0, 20.0, false, tl!("Options"));
    egui::Popup::menu(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(200.0);
        let o = &mut app.ui.tool_options;
        ui.checkbox(&mut o.move_show_hover_bounds, tl!("Show layer bounds on hover"));
        ui.checkbox(&mut o.move_expand_group_on_click, tl!("Expand layer group on click"));
        ui.checkbox(&mut o.move_show_layers_hover_bounds, tl!("Show hover bounds from Layers"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_is_a_live_command_with_an_icon() {
        let specs = photocraft_engine::command_specs();
        let bar: Vec<BarButton> = bar_groups().into_iter().flatten().collect();
        let panel = ALIGN.iter().chain(DISTRIBUTE.iter());
        for (cmd, icon) in bar.iter().map(|b| (b.0, b.1)).chain(panel.map(|b| (b.0, b.1))) {
            assert!(specs.iter().any(|c| c.id == cmd), "{cmd} is not a command");
            assert!(icons::exists(icon), "{icon} is not embedded");
        }
        assert!(icons::exists("settings"));
    }
}
