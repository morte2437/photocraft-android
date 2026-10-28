use super::tool_input::ToolEvent;
use super::view_xform::ViewXform;
use super::{Color32, PhotocraftApp, Pos2, Rect, Stroke, Tool, json, pos2, vec2};

/// Windows' crosshair cursor inverts the pixels under it, so over mid-grey (the pasteboard, many
/// photos) it vanishes (#737), and the Linux themes' and macOS's differ. Use the same black-and-white
/// bitmap on every desktop platform, carried by the OS so its movement does not wait for the canvas
/// to render (#1343).
pub(super) fn visible_crosshair(icon: egui::CursorIcon, painter: &egui::Painter, p: Pos2, draw: bool) -> egui::CursorIcon {
    if !draw || icon != egui::CursorIcon::Crosshair {
        return icon;
    }
    crate::tool_cursor::crosshair(painter, p, 8.0, 2.0)
}

/// The Pencil's cursor at `doc` (document pixels): the whole-pixel square its dab fills
/// (`paint::grid_square`), in screen points with its edges on physical pixels (`ppp` = pixels
/// per point), so it lines up with the pixel grid at any zoom.
pub(crate) fn pencil_cursor_rect(xf: &ViewXform, doc: [f64; 2], size: f32, ppp: f32) -> Rect {
    let [x0, y0, x1, y1] = photocraft_engine::paint::grid_square(doc[0], doc[1], size);
    let r = Rect::from_two_pos(xf.to_screen(x0 as f32, y0 as f32), xf.to_screen(x1 as f32, y1 as f32));
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(pos2(snap(r.min.x), snap(r.min.y)), pos2(snap(r.max.x), snap(r.max.y)))
}

/// Whether an aliased (Pencil) dab of `size` pixels is round rather than its whole
/// [`pencil_cursor_rect`] square. The dab fills the pixels whose centres lie within its radius of
/// the grid centre (`paint::grid_center`), so up to 3 px the corner pixels are in and the dab is
/// a square; from 4 px on they fall outside and it is a disc (#2662).
pub(crate) fn pencil_tip_is_round(size: f32) -> bool {
    let n = if size.is_finite() { size.round().clamp(1.0, 100_000.0) } else { 1.0 };
    let h = n / 2.0;
    (h - 0.5) * std::f32::consts::SQRT_2 > h
}

/// Whether a brush-tip circle draws its centre mark.
/// Clone Stamp and Healing Brush stay an empty circle until Option is held, the source-point
/// cursor. Other brushes follow Preferences › Cursors › Show Crosshair in Brush Tip, as in
/// Photoshop; the Background Eraser always shows its sampling hotspot.
pub(super) fn brush_tip_centre(tool: Tool, option: bool, show_crosshair: bool) -> bool {
    if tool == Tool::QuickSelection {
        return false;
    }
    if matches!(tool, Tool::CloneStamp | Tool::Healing) {
        return option || show_crosshair;
    }
    show_crosshair || tool == Tool::BackgroundEraser
}

/// The display colour of an RGB triple (0..1 per channel) as an egui colour.
fn display_color(c: [f32; 3]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

/// The Eyedropper ring's two colours at document point (x, y): the colour under the pointer
/// (what a click would pick) and the current foreground, or `None` where there is no colour
/// to sample (#213).
pub(crate) fn eyedropper_ring_colors(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<(Color32, Color32)> {
    let new = eyedropper_color(app, x, y)?;
    let fg = app.session.tools.foreground;
    Some((display_color(new), display_color([fg[0], fg[1], fg[2]])))
}

/// Draw the Eyedropper's comparison ring at `p`: an opaque neutral grey band around the
/// pointer with the sampled colour on its upper half and the current one on the lower,
/// framed by thin dark and light hairlines so every edge reads on any image, and the
/// pipette at its centre marking the sampled pixel (#213, #6973). Like Photoshop it shows
/// only while the mouse button is held (`held`): hovering shows the plain pipette and
/// samples nothing. `None` when not held, there is nothing to sample, or the Precise-cursor
/// preference wants the plain crosshair.
pub(super) fn eyedropper_ring(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, p: Pos2, held: bool) -> Option<egui::CursorIcon> {
    // The options bar's Show Sampling Ring turns it off (#1649).
    if !held || !app.ui.tool_options.eyedropper_ring || app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
        return None;
    }
    let [x, y] = xf.to_doc(p);
    let (new, current) = eyedropper_ring_colors(app, x, y)?;
    // Photoshop's proportions (#6973): a ring about 210 pt across whose opaque grey outer
    // band and coloured inner band are each about 17 pt wide, over a transparent centre.
    // In points, so the ring scales with Interface › UI Scale like the rest of the cursor.
    let (r_out, band) = (105.0_f32, 17.0_f32);
    let r_in = r_out - 2.0 * band;
    // Screen y grows down, so PI..TAU is the upper half (the sampled colour) and 0..PI the
    // lower (the current one); the halves meet at 3 and 9 o'clock, as in Photoshop.
    use std::f32::consts::{PI, TAU};
    let upper: Vec<Pos2> = (0..=48)
        .map(|i| {
            let a = PI + (TAU - PI) * i as f32 / 48.0;
            p + vec2(a.cos(), a.sin()) * (r_in + band / 2.0)
        })
        .collect();
    // `circle_stroke` paints its stroke outside the radius, `Shape::line` centres it.
    painter.circle_stroke(p, r_in, Stroke::new(band, current));
    painter.add(egui::Shape::line(upper, Stroke::new(band, new)));
    painter.circle_stroke(p, r_in + band, Stroke::new(band, RING_GREY));
    // Thin dark and light hairlines on the ring's two edges, one physical pixel wide, so it
    // reads on any colour including a flat mid-grey that matches the band itself (#6973).
    let px = 1.0 / painter.ctx().pixels_per_point().max(1.0);
    let (dark, light) = (Stroke::new(px, Color32::from_black_alpha(160)), Stroke::new(px, Color32::from_white_alpha(160)));
    painter.circle_stroke(p, r_out, dark);
    painter.circle_stroke(p, r_out + px, light);
    painter.circle_stroke(p, r_in - px, dark);
    painter.circle_stroke(p, r_in - 2.0 * px, light);
    // Photoshop keeps the pipette visible at the centre, its tip on the sampled pixel; a
    // ring this size says nothing about where it samples without it.
    Some(pipette_cursor(painter.ctx(), p))
}

/// The Eyedropper ring's opaque neutral grey, as Photoshop measures it (#6973).
const RING_GREY: Color32 = Color32::from_gray(127);

/// Tools on which holding ⌥ (Alt) switches to the Eyedropper: a click or drag sets the
/// foreground colour, as with the Eyedropper itself (#417).
pub(crate) fn alt_samples(tool: Tool, mods: egui::Modifiers) -> bool {
    // Control+Alt is the brush-resize drag (`brush_resize`), not sampling.
    mods.alt && !mods.ctrl && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Gradient | Tool::PaintBucket)
}

/// Preferences › Cursors › Other Cursors asks for the precise crosshair, or Caps Lock swaps the
/// standard cursor for it (and back), as in Photoshop (#4639).
pub(super) fn precise_other_cursor(app: &PhotocraftApp) -> bool {
    (app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise) != app.caps_lock
}

/// A pen tool's standard cursor: its toolbar icon with the nib (the Convert Point tool: the
/// anchor) on `p`. Draws it at `p` and returns the OS cursor to set (hidden).
pub(crate) fn pen_cursor(ctx: &egui::Context, tool: Tool, p: Pos2) -> egui::CursorIcon {
    // Where each icon's point is in its 24-unit box: the full-size pen's nib at (2.3, 2.3); the
    // Freeform and Curvature pens draw it at 0.68 × from (7, 7); Convert Point's apex anchor.
    let hot = match tool {
        Tool::FreeformPen | Tool::CurvaturePen => vec2(8.56, 8.56),
        Tool::ConvertPoint => vec2(12.0, 7.0),
        _ => vec2(2.3, 2.3),
    };
    crate::icons::cursor(ctx, crate::icons::tool_icon(tool), p, hot / 24.0, 22.0);
    egui::CursorIcon::None
}

/// The sampling cursor: a pipette whose tip is the sampled pixel. Draws it at `p` and returns the
/// OS cursor to set (hidden).
pub(crate) fn pipette_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    // The tip of the icon's pipette is at (2, 22) of its 24-unit box.
    crate::icons::cursor(ctx, "pipette", p, vec2(2.0, 22.0) / 24.0, 20.0);
    egui::CursorIcon::None
}

/// The Paint Bucket's cursor: a bucket whose pouring corner is the filled pixel. Draws it at `p`
/// and returns the OS cursor to set (hidden).
pub(crate) fn paint_bucket_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    // The left corner of the icon's bucket is at (2, 12) of its 24-unit box.
    crate::icons::cursor(ctx, "paint-bucket", p, vec2(2.0, 12.0) / 24.0, 22.0);
    egui::CursorIcon::None
}

/// Decided when the press starts, so ⌥ pressed or released mid-stroke never switches between
/// painting and sampling.
pub(super) fn alt_eyedropper(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if matches!(ev, ToolEvent::Down { .. }) {
        let physical_right = app.services.right_button_down.as_ref().is_some_and(|f| f());
        app.alt_sampling = !physical_right && alt_samples(app.active_tool(), mods);
    }
    if !app.alt_sampling {
        return false;
    }
    match ev {
        // Without ⌥: the sample sets the foreground colour, whatever the Eyedropper would do.
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } => sample_eyedropper(app, x, y, egui::Modifiers::NONE),
        ToolEvent::Up { .. } => app.alt_sampling = false,
    }
    true
}

pub(super) fn sample_eyedropper(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    if let Some([r, g, b]) = eyedropper_color(app, x, y) {
        let key = if mods.alt { "background" } else { "foreground" };
        let _ = app.run("tools.setColors", json!({ key: [r, g, b, 1.0] }));
    }
}

/// The colour at document point (x, y) with the Eyedropper's Sample Size (the composite pixel,
/// or the average of the square around it) from the layers `sample_layer` names (`document.sampleColor`).
/// `None` off the image or over transparency.
fn sample_color(app: &mut PhotocraftApp, x: f64, y: f64, sample_layer: &str) -> Option<[f32; 3]> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let size = app.ui.tool_options.eyedropper_size;
    let v = app.run("document.sampleColor", json!({"x": x, "y": y, "size": size, "sampleLayer": sample_layer})).ok()?;
    match serde_json::from_value::<Vec<f32>>(v).ok()?[..] {
        [r, g, b, a] if a > 0.0 => Some([r, g, b]),
        _ => None,
    }
}

/// What the Eyedropper tool (and a painting tool's ⌥-click) picks at document point (x, y): its
/// options bar's Sample Size and Sample (#1649).
pub(crate) fn eyedropper_color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    let layers = app.ui.tool_options.eyedropper_sample.clone();
    sample_color(app, x, y, &layers)
}

/// The active document's composite colour at document point (x, y), averaged over the
/// Eyedropper's Sample Size as Photoshop's dialog eyedroppers (Curves, Color Picker) do.
/// `None` off the image or over transparency.
pub(crate) fn composite_color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    sample_color(app, x, y, "all")
}
