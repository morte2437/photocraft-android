//! The Crop tool's contextual task bar (#10890): what the Crop tool puts in the reusable bar of
//! `widgets::task_bar`, under the crop frame, as in the reference application. Left to right:
//!
//! 1. the grip (the widget's);
//! 2. *Generative Expand goes here* once generative fill comes through the shared AI layer (#41,
//!    #1199); until then it is left out, not shown disabled;
//! 3. the preset dropdown: the options bar's own (`crop_presets::dropdown`), so the two stay in
//!    sync;
//! 4. Straighten: the options bar's Straighten button;
//! 5. the ⋯ menu (the widget's);
//! 6. while a crop is pending ([`crop_ui::pending`]): Cancel and Done, which the options bar's
//!    cancel and commit buttons do.
//!
//! Shown while the Crop tool shows a frame on the active document, Window › Contextual Task Bar
//! is ticked and no dialog is open.

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::crop_ui;
use crate::state::Tool;
use crate::widgets::task_bar;

/// The Straighten icon. TODO(#10888): the spirit-level glyph once that options-bar item lands.
const STRAIGHTEN_ICON: &str = "ruler";

/// The crop frame on screen: the bounds of its (possibly turned) corners.
fn frame_on_screen(app: &PhotocraftApp, xf: &ViewXform) -> Option<egui::Rect> {
    let r = app.ui.crop_rect?;
    let pts = crop_ui::corners(r, app.ui.crop_angle).map(|[x, y]| xf.to_screen(x as f32, y as f32));
    Some(egui::Rect::from_points(&pts))
}

/// The bar shows: the Crop tool with a frame, the Window toggle on, no dialog over the canvas.
fn wanted(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.panels.context_task_bar && app.ui.dialogs.is_empty() && app.session.active().is_some()
}

/// Whether the bar can show now: [`wanted`] and a crop frame. `ui.inspect` checks it, so a bar
/// hidden by `ui.set` before the next draw no longer reports last frame's rect.
pub fn eligible(app: &PhotocraftApp) -> bool {
    wanted(app) && app.ui.crop_rect.is_some()
}

/// Draws the bar under the active document's crop frame (the canvas's overlay pass calls it on
/// that document's tile).
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    if !wanted(app) {
        return;
    }
    let Some(anchor) = frame_on_screen(app, xf) else { return };
    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = task_bar::show(&ctx, &mut place, anchor, xf.rect, |ui, more| contents(app, ui, more));
    app.ui.panels.task_bar = place;
    let Some(shown) = shown else { return };
    if shown.hide {
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = shown.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);
}

/// The Crop tool's controls in the bar.
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, more: &mut task_bar::MoreMenu) {
    // Generative Expand (#41) goes here, before the preset dropdown.
    let before_ratio = crop_ui::preset_ratio(app);
    // Its own id: the options bar shows the same dropdown.
    ui.push_id("crop-task-bar", |ui| crate::crop_presets::dropdown(app, ui));
    if crop_ui::preset_ratio(app) != before_ratio {
        crop_ui::apply_ratio(app);
    }
    let armed = app.crop.straighten.armed;
    if crate::icons::button_with_icon_size_tip(ui, STRAIGHTEN_ICON, task_bar::BUTTON, 14.0, armed, tl!("Straighten"), false).clicked() {
        crate::crop_straighten::toggle(app);
    }
    more.show(ui);
    if crop_ui::pending(app) {
        if ui.button(tl!("Cancel")).clicked() {
            crop_ui::dismiss(app);
        }
        if ui.button(tl!("Done")).clicked() {
            crate::canvas::commit_crop(app);
        }
    }
}
