//! Photoshop's on-canvas brush resize (#231, #297): with a painting tool, Alt+right-drag
//! (Windows, Linux) or Control+Option+drag (macOS; Control+Alt+drag works everywhere) changes the
//! brush instead of painting. Left and right
//! set the diameter (the circle's edge follows the pointer), up and down the hardness (down makes
//! it harder). The brush circle stays where the drag began, tinted to show the hardness, with a
//! Diameter/Hardness readout beside the pointer.
//!
//! The change is `tools.setBrush` calls sharing one `coalesce` key, so the whole drag is one
//! journal entry (Rule 1) and, the brush being tool state, no history step. Krita's ⇧+drag (the
//! request in #231) is not used: ⇧ is Photoshop's straight-line modifier for painting tools.

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Pos2, Stroke, vec2};
use photocraft_engine::paint::MAX_BRUSH_SIZE;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// Screen points of vertical drag that take the hardness from 0 % to 100 %.
pub const HARDNESS_TRAVEL: f32 = 200.0;
/// The brush size range (`tools.setBrush` accepts the same).
const SIZE: std::ops::RangeInclusive<f32> = 1.0..=MAX_BRUSH_SIZE;

/// A resize drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resize {
    /// Where the drag began, in document pixels: the circle stays there.
    pub anchor: [f64; 2],
    /// The brush's diameter and vertical value when it began (its hardness, or its opacity with
    /// "Vary Round Brush Hardness on HUD" off).
    pub start: (f32, f32),
    /// The `coalesce` key of this drag's `tools.setBrush` calls.
    key: u64,
    /// Begun by an Alt+right-drag (#297): it ends when the right button is up.
    pub secondary: bool,
}

/// Tools the gesture resizes: every painting tool, and Quick Selection (it has a brush too).
pub fn applies(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::QuickSelection
}

/// Control+Alt (Control+Option on a Mac): `ctrl` is the Control key on every platform.
pub fn is_gesture(mods: egui::Modifiers) -> bool {
    mods.ctrl && mods.alt
}

/// Alt (Option) with the right button: Photoshop's Windows gesture (#297). The left button with
/// Alt alone stays the painting tools' eyedropper.
pub fn is_right_gesture(mods: egui::Modifiers) -> bool {
    mods.alt
}

/// The diameter and hardness a drag from `start` by (`dx`, `dy`) document pixels gives at `zoom`.
pub fn resized(start: (f32, f32), dx: f64, dy: f64, zoom: f32) -> (f32, f32) {
    let dx = if dx.is_finite() { dx as f32 } else { 0.0 };
    let dy = if dy.is_finite() { dy as f32 } else { 0.0 };
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let size = (start.0 + 2.0 * dx).round().clamp(*SIZE.start(), *SIZE.end());
    let hardness = (start.1 + dy * zoom / HARDNESS_TRAVEL).clamp(0.0, 1.0);
    // Whole percent, like the readout and the options bar.
    (size, (hardness * 100.0).round() / 100.0)
}

/// Route a tool event: true when it belongs to a resize drag (begun here with Control+Alt held,
/// or an Alt+right-drag `armed` for this press, or already in progress), so the tool never sees
/// it.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers, armed: bool) -> bool {
    static KEY: AtomicU64 = AtomicU64::new(0);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            // A new press ends any resize whose release was never seen.
            app.brush_resize = None;
            if app.drag.is_some() || !applies(app.ui.tool) || !(armed || is_gesture(mods)) {
                return false;
            }
            let vary = app.session.prefs().tools.vary_round_brush_hardness_on_hud;
            let (size, v) = {
                let b = &app.session.tools.brush;
                (b.size, if vary { b.hardness } else { b.opacity })
            };
            app.brush_resize = Some(Resize { anchor: [x, y], start: (size, v), key: KEY.fetch_add(1, Ordering::Relaxed), secondary: armed });
            true
        }
        ToolEvent::Move { x, y, .. } => match app.brush_resize {
            Some(r) => {
                update(app, r, x, y);
                true
            }
            None => false,
        },
        ToolEvent::Up { x, y } => match app.brush_resize.take() {
            Some(r) => {
                update(app, r, x, y);
                true
            }
            None => false,
        },
    }
}

/// End an Alt+right-drag resize whose button is no longer down (its release went elsewhere, e.g.
/// outside the window), so it can't swallow the next stroke.
pub fn release_stale(app: &mut PhotocraftApp, secondary_down: bool) {
    if !secondary_down && app.brush_resize.is_some_and(|r| r.secondary) {
        app.brush_resize = None;
    }
}

fn update(app: &mut PhotocraftApp, r: Resize, x: f64, y: f64) {
    // Screen-relative, as in Photoshop (#4228): right is bigger and down harder on screen, however
    // the view is rotated or flipped. The screen delta over the zoom is in document pixels at 0°.
    let (dx, dy, zoom) = match ViewXform::active(app) {
        Some(xf) if xf.zoom.is_finite() && xf.zoom > 0.0 => {
            let d = xf.to_screen(x as f32, y as f32) - xf.to_screen(r.anchor[0] as f32, r.anchor[1] as f32);
            (f64::from(d.x / xf.zoom), f64::from(d.y / xf.zoom), xf.zoom)
        }
        // No view: document axes, mirrored by View › Flip Horizontal.
        _ => ((x - r.anchor[0]) * if app.ui.view.flip_horizontal { -1.0 } else { 1.0 }, y - r.anchor[1], app.point_zoom()),
    };
    let (size, v) = resized(r.start, dx, dy, zoom);
    // "Vary Round Brush Hardness on HUD" (Preferences ▸ Tools, on by default): off, the vertical
    // movement takes the opacity instead of the hardness, as in Photoshop.
    let vary = app.session.prefs().tools.vary_round_brush_hardness_on_hud;
    let b = &app.session.tools.brush;
    if b.size == size && if vary { b.hardness == v } else { b.opacity == v } {
        return;
    }
    let brush = if vary { json!({ "size": size, "hardness": v }) } else { json!({ "size": size, "opacity": v }) };
    let p = json!({ "brush": brush, "coalesce": format!("brush-resize:{}", r.key) });
    if let Err(e) = app.run("tools.setBrush", p) {
        app.ui.status = e;
    }
}

/// Draw the resize feedback: the brush circle at the anchor, filled with the brush preview
/// colour whose solid core shows the hardness (Photoshop's), and the readout beside the pointer.
/// Returns true while a resize is in progress (the canvas then hides its cursor).
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) -> bool {
    let Some(r) = app.brush_resize else { return false };
    let b = &app.session.tools.brush;
    let c = xf.to_screen(r.anchor[0] as f32, r.anchor[1] as f32);
    let radius = (b.size / 2.0 * xf.zoom).max(1.0);
    tip_preview(painter, c, radius, b.hardness, preview_tint(app));
    painter.circle_stroke(c, radius + 0.5, Stroke::new(1.0, Color32::from_black_alpha(140)));
    painter.circle_stroke(c, radius, Stroke::new(1.0, Color32::from_white_alpha(220)));
    // Photoshop hides the pointer while the circle stays put.
    painter.ctx().set_cursor_icon(egui::CursorIcon::None);
    let at = painter.ctx().pointer_latest_pos().unwrap_or(c);
    let (label, value) = if app.session.prefs().tools.vary_round_brush_hardness_on_hud {
        (tl!("Hardness:"), format!("{}%", (b.hardness * 100.0).round() as i64))
    } else {
        (tl!("Opacity:"), format!("{}%", (b.opacity * 100.0).round() as i64))
    };
    crate::canvas::draw_readout(painter.ctx(), "brush-resize-readout", at, [tl!("Diameter:"), label], [format!("{} px", b.size.round() as i64), value]);
    true
}

/// The tip preview's tint: Preferences ▸ Cursors ▸ Brush Preview colour (red by default, as in
/// Photoshop), at the overlay's alpha. Unreadable values keep the red.
fn preview_tint(app: &PhotocraftApp) -> Color32 {
    let alpha = 110;
    crate::color_picker_ui::parse_hex(&app.session.prefs().cursors.brush_preview_color).map_or(Color32::from_rgba_unmultiplied(255, 0, 0, alpha), |c| {
        Color32::from_rgba_unmultiplied((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, alpha)
    })
}

/// A round tip of `radius` screen points: solid out to `hardness` of the radius, fading to clear
/// at the edge. One mesh (two rings and a centre), so its cost doesn't grow with the brush.
fn tip_preview(painter: &egui::Painter, c: Pos2, radius: f32, hardness: f32, color: Color32) {
    let clip = painter.clip_rect();
    if !clip.expand(radius).contains(c) {
        return;
    }
    // Enough segments for a smooth edge at any size, capped so huge brushes stay cheap.
    let n = ((radius * 0.5) as usize).clamp(24, 160);
    let inner = radius * hardness.clamp(0.0, 1.0);
    let clear = Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, color);
    for k in 0..n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        let d = vec2(a.cos(), a.sin());
        mesh.colored_vertex(c + d * inner, color);
        mesh.colored_vertex(c + d * radius, clear);
    }
    let n = n as u32;
    for k in 0..n {
        let (i0, o0) = (1 + 2 * k, 2 + 2 * k);
        let (i1, o1) = (1 + 2 * ((k + 1) % n), 2 + 2 * ((k + 1) % n));
        mesh.add_triangle(0, i0, i1);
        mesh.add_triangle(i0, o0, o1);
        mesh.add_triangle(i0, o1, i1);
    }
    painter.add(mesh);
}

#[cfg(test)]
mod tests;
