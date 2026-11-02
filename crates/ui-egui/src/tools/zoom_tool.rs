//! Zoom tool drags (#171).
//!
//! With Scrubby Zoom on (the default, as in Photoshop) press-and-drag zooms continuously: right
//! zooms in, left zooms out, anchored at the point where the button went down. With it off, the
//! drag draws a rectangle and the release zooms to fit it. A click (no drag) still steps the zoom
//! in, or out with ⌥ (handled by the canvas).

use egui::{Pos2, Rect, vec2};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::paint_mouse::Buttons;
use crate::state::View;

/// Horizontal drag (screen points) that doubles, or halves, the zoom.
pub const DOUBLING: f32 = 100.0;

/// A Zoom-tool drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ZoomDrag {
    /// Screen point where the button went down.
    anchor: Pos2,
    /// The document point under it when the drag began (it stays under the anchor).
    doc: [f64; 2],
    zoom0: f32,
    /// Scrubby Zoom off: a zoom rectangle, its current corner.
    rect_to: Option<Pos2>,
}

fn id() -> egui::Id {
    egui::Id::unique("pc-zoom-tool-drag")
}

/// The scrubby zoom for a horizontal drag of `dx` points from a press at zoom `zoom0`, on a
/// document of `size` px (`zoom_levels` sets the range).
pub fn scrub_zoom(zoom0: f32, dx: f32, size: [u32; 2]) -> f32 {
    // ±40 doublings already span far more than the zoom range; it keeps the power finite.
    let e = if dx.is_finite() { (dx / DOUBLING).clamp(-40.0, 40.0) } else { 0.0 };
    crate::zoom_levels::clamp(zoom0 * e.exp2(), size)
}

/// Set `view` to `zoom` (device pixels per document pixel) with document point `doc` shown at
/// screen point `screen` (egui points) on a display with `ppp` physical pixels per point.
pub fn anchor_view(view: &mut View, xf: &ViewXform, screen: Pos2, doc: [f64; 2], zoom: f32, ppp: f32) {
    view.zoom_animating = false;
    view.zoom_anchor = None;
    view.zoom = zoom;
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let d = xf.unmap_vec((screen - xf.rect.center()) / (zoom / ppp));
    view.center = [doc[0] as f32 - d.x, doc[1] as f32 - d.y];
}

/// Route the canvas buttons for the Zoom tool. Returns true when it consumed the drag (the
/// canvas then handles only the click).
pub fn drag(app: &PhotocraftApp, ctx: &egui::Context, view: &mut View, xf: &ViewXform, b: &Buttons, pointer: Option<Pos2>) -> bool {
    let scrubby = app.ui.tool_options.zoom_scrubby;
    if b.started {
        let anchor = ctx.input(|i| i.pointer.press_origin()).filter(|p| xf.rect.contains(*p)).or(pointer);
        if let Some(anchor) = anchor {
            let z = ZoomDrag { anchor, doc: xf.to_doc(anchor), zoom0: view.zoom, rect_to: (!scrubby).then_some(anchor) };
            ctx.data_mut(|d| d.insert_temp(id(), z));
        }
    }
    let Some(mut z) = ctx.data(|d| d.get_temp::<ZoomDrag>(id())) else { return b.started || b.dragged || b.stopped };
    if let Some(p) = pointer.filter(|_| b.dragged || b.stopped) {
        match z.rect_to.as_mut() {
            Some(to) => *to = p,
            None => {
                // Only a new zoom re-anchors: a window that Zoom Resizes Windows grew around the
                // image has centred it since (zoom_window_fit.rs), and the release keeps that.
                let zoom = scrub_zoom(z.zoom0, p.x - z.anchor.x, view.doc_size);
                if zoom.to_bits() != view.zoom.to_bits() {
                    anchor_view(view, xf, z.anchor, z.doc, zoom, app.ppp);
                }
            }
        }
        ctx.data_mut(|d| d.insert_temp(id(), z));
    }
    if b.stopped {
        ctx.data_mut(|d| d.remove::<ZoomDrag>(id()));
        if let Some(to) = z.rect_to {
            let out = crate::workspace_ui::sticky_mods(app, ctx.input(|i| i.modifiers)).alt;
            zoom_to_rect(view, xf, Rect::from_two_pos(z.anchor, to), z.anchor, out, app.ppp);
        }
    }
    true
}

/// Scrubby Zoom off: zoom so the dragged rectangle fills the canvas (a tiny one steps the zoom
/// at the press point; ⌥ steps out).
fn zoom_to_rect(view: &mut View, xf: &ViewXform, r: Rect, anchor: Pos2, out: bool, ppp: f32) {
    if out || r.width() < 4.0 || r.height() < 4.0 {
        let doc = xf.to_doc(anchor);
        anchor_view(view, xf, anchor, doc, crate::zoom_levels::step(view.zoom, if out { -1 } else { 1 }, view.doc_size), ppp);
        return;
    }
    // `k` is the ratio of two screen-point lengths, so it scales the zoom in any unit.
    let k = (xf.rect.width() / r.width()).min(xf.rect.height() / r.height());
    let zoom = crate::zoom_levels::clamp(view.zoom * k, view.doc_size);
    let c = xf.to_doc(r.center());
    anchor_view(view, xf, xf.rect.center(), c, zoom, ppp);
}

/// Scrubby Zoom off: the zoom rectangle being dragged, as marching ants.
pub fn draw(ctx: &egui::Context, painter: &egui::Painter) {
    let Some(z) = ctx.data(|d| d.get_temp::<ZoomDrag>(id())) else { return };
    let Some(to) = z.rect_to else { return };
    let r = Rect::from_two_pos(z.anchor, to);
    let r = Rect::from_min_max(r.min.round() + vec2(0.5, 0.5), r.max.round() + vec2(0.5, 0.5));
    crate::tool_feedback::draw_ants(painter, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true);
}

#[cfg(test)]
mod tests;
