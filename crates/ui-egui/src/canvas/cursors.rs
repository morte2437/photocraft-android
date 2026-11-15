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
    crate::tool_cursor::standard_crosshair(painter, p)
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

/// Whether a brush-tip circle draws its centre mark, for Normal and Full Size Brush Tip cursors
/// alike (both draw through this). Quick Selection never draws one. Clone Stamp and Healing Brush
/// stay an empty circle until Option is held, the source-point cursor (the Healing Brush's ⌥ with
/// Normal Brush Tip shows the sample target instead, #12113). Other brushes follow
/// Preferences › Cursors › Show Crosshair in Brush Tip, as in Photoshop; the Background Eraser
/// and the Color Replacement tool (#11934) always show their sampling hotspot, and the Remove
/// tool its centre + (#12550).
pub(super) fn brush_tip_centre(tool: Tool, option: bool, show_crosshair: bool) -> bool {
    if tool == Tool::QuickSelection {
        return false;
    }
    if matches!(tool, Tool::CloneStamp | Tool::Healing) {
        return option || show_crosshair;
    }
    show_crosshair || matches!(tool, Tool::BackgroundEraser | Tool::ColorReplacement | Tool::Remove)
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
/// foreground colour, as with the Eyedropper itself (#417, the Background Eraser #11906, the Color
/// Replacement tool #11914).
pub(crate) fn alt_samples(tool: Tool, mods: egui::Modifiers) -> bool {
    // Control+Alt is the brush-resize drag (`brush_resize`), not sampling.
    mods.alt && !mods.ctrl && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Gradient | Tool::PaintBucket | Tool::BackgroundEraser | Tool::ColorReplacement)
}

/// Preferences › Cursors › Other Cursors asks for the precise crosshair, or Caps Lock swaps the
/// standard cursor for it (and back), as in Photoshop (#4639).
pub(crate) fn precise_other_cursor(app: &PhotocraftApp) -> bool {
    (app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise) != app.caps_lock
}

/// A pen tool's standard cursor: its toolbar icon with the nib (the Convert Point tool: the
/// anchor) on `p`. Uses an OS cursor bitmap where supported so its movement tracks hardware
/// pointer input without waiting for canvas frames (#9122); falls back to drawing the tooltip icon.
pub fn pen_cursor(ctx: &egui::Context, tool: Tool, p: Pos2) -> egui::CursorIcon {
    if let Some(icon) = crate::tool_cursor::pen(ctx, tool) {
        return icon;
    }
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

/// The Color Sampler's cursor over an existing sampler point: the arrow with the sampler target,
/// or with Alt the arrow with scissors (#11744).
pub(crate) fn sampler_hover_cursor(ctx: &egui::Context, p: Pos2, alt: bool) -> egui::CursorIcon {
    if alt {
        crate::icons::cursor_badge(ctx, "scissors", p);
    } else {
        crate::icons::cursor_badge(ctx, "sampler", p);
    }
    egui::CursorIcon::Default
}

/// The Color Sampler's cursor: an eyedropper with a small sampler target at its top-left,
/// hotspot at the pipette tip (#11744).
pub(crate) fn color_sampler_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    pipette_cursor(ctx, p);
    let size = 11.0;
    // The pipette tip is at (2, 22) in its 24-unit box of size 20.0. The top-left region is
    // around (5, 5), which is offset by (3, -17) * (20/24) = (2.5, -14.17) from the tip.
    let target_center = p + vec2(2.5, -14.0);
    let rect = Rect::from_center_size(target_center, egui::Vec2::splat(size));
    let area = egui::Area::new(egui::Id::unique("pc-sampler-badge")).order(egui::Order::Tooltip).fixed_pos(rect.min).constrain(false).interactable(false);
    area.show(ctx, |ui| {
        let outline = crate::icons::image("sampler", size, Color32::from_black_alpha(200));
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
            outline.paint_at(ui, rect.translate(vec2(dx, dy)));
        }
        crate::icons::image("sampler", size, Color32::WHITE).paint_at(ui, rect);
    });
    egui::CursorIcon::None
}

/// The sampling cursor: a pipette whose tip is the sampled pixel. Draws it at `p` and returns the
/// OS cursor to set. Native windows use a cached bitmap that moves independently of rendering.
pub(crate) fn pipette_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    crate::tool_cursor::pipette(ctx, p)
}

/// The filter dialogs' preview locator: a small square centred on `p`, the point a click centres
/// the dialog's preview window on (#9648), white between dark edges so it reads on any image.
/// Draws it at `p` and returns the OS cursor to set (hidden).
pub(crate) fn preview_locator_cursor(ctx: &egui::Context, p: Pos2) -> egui::CursorIcon {
    const SIDE: f32 = 9.0;
    let rect = Rect::from_center_size(p, vec2(SIDE, SIDE));
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pc-preview-locator")));
    let dark = Stroke::new(1.0, Color32::from_black_alpha(200));
    painter.rect_stroke(rect.expand(1.0), 0.0, dark, egui::StrokeKind::Inside);
    painter.rect_stroke(rect.shrink(1.0), 0.0, dark, egui::StrokeKind::Inside);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
    egui::CursorIcon::None
}

/// The cursor while ⌥ turns a painting tool into the Eyedropper (`alt_eyedropper`): the pipette,
/// or the comparison ring while it samples with the button `held` (#213), or the precise crosshair
/// with Preferences › Cursors › Other Cursors = Precise. The Mixer Brush's ⌥ loads paint instead:
/// its sample target, with no ring (#11957).
pub(super) fn alt_pick_cursor(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, p: Pos2, held: bool, tool: Tool) -> egui::CursorIcon {
    let precise = app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise;
    if tool == Tool::MixerBrush && !app.alt_sampling {
        return if precise { egui::CursorIcon::Crosshair } else { sample_target_cursor(painter, p) };
    }
    if let Some(icon) = eyedropper_ring(app, painter, xf, p, held) {
        icon
    } else if precise {
        egui::CursorIcon::Crosshair
    } else {
        pipette_cursor(painter.ctx(), p)
    }
}

/// The Mixer Brush's and the Healing Brush's ⌥ cursor, the reference's sample target (#11957,
/// #12113): a small circle with a crosshair through it, centred on the hotspot. Draws it at `p`
/// and returns the OS cursor to set.
pub(crate) fn sample_target_cursor(painter: &egui::Painter, p: Pos2) -> egui::CursorIcon {
    let across = vec![vec2(-8.0, 0.0), vec2(8.0, 0.0)];
    let down = vec![vec2(0.0, -8.0), vec2(0.0, 8.0)];
    super::tool_cursor::outline(painter, p, vec![ring(5.0), across, down], false)
}

/// A no-entry sign, a circle with a diagonal bar centred on the hotspot: the Mixer Brush over a
/// layer it can't paint (Lock All, #11957). Draws it at `p` and returns the OS cursor to set.
pub(crate) fn no_entry_cursor(painter: &egui::Painter, p: Pos2) -> egui::CursorIcon {
    let bar = vec![vec2(-4.5, -4.5), vec2(4.5, 4.5)];
    super::tool_cursor::outline(painter, p, vec![ring(6.5), bar], false)
}

/// A closed circle of `radius` points around the hotspot.
fn ring(radius: f32) -> Vec<egui::Vec2> {
    (0..32).map(|i| i as f32 / 32.0 * std::f32::consts::TAU).map(|a| vec2(a.cos(), a.sin()) * radius).collect()
}

/// Whether the Mixer Brush's target layer has Lock All on, so it shows [`no_entry_cursor`].
pub(crate) fn mixer_target_locked(app: &PhotocraftApp) -> bool {
    app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).is_some_and(|layer| layer.locks.all)
}

#[cfg(test)]
#[path = "../mixer_cursor_tests.rs"]
mod mixer_cursor_tests;

#[cfg(test)]
#[path = "../healing_alt_cursor_tests.rs"]
mod healing_alt_cursor_tests;

/// The Paint Bucket artwork and target share one cached cursor image. Report rasterization
/// failures once per distinct message, with the same concise error in the dialog and status bar.
pub(crate) fn paint_bucket_cursor(app: &mut PhotocraftApp, painter: &egui::Painter, p: Pos2) -> egui::CursorIcon {
    let ctx = painter.ctx();
    let id = egui::Id::unique("photocraft-bucket-cursor-error");
    match crate::tool_cursor::bucket(painter, p) {
        Ok(icon) => {
            // The failure belonged to another scale: its status goes, and a new one reports again.
            if let Some(error) = ctx.data(|data| data.get_temp::<String>(id)) {
                ctx.data_mut(|data| data.remove::<String>(id));
                if app.ui.status == error {
                    app.ui.status.clear();
                    app.ui.status_error = false;
                }
            }
            icon
        }
        Err(error) => {
            if ctx.data(|data| data.get_temp::<String>(id)).as_ref() != Some(&error) {
                if !app.ui.dialogs.iter().any(|dialog| {
                    dialog.kind == crate::state::DialogKind::Error && dialog.fields.get("message").and_then(serde_json::Value::as_str) == Some(&error)
                }) {
                    let mut fields = serde_json::Map::new();
                    fields.insert("message".into(), json!(error));
                    app.ui.open_dialog(crate::state::DialogKind::Error, fields);
                }
                ctx.data_mut(|data| data.insert_temp(id, error.clone()));
            }
            app.ui.status = error;
            app.ui.status_error = true;
            egui::CursorIcon::Default
        }
    }
}

/// Decided when the press starts, so ⌥ pressed or released mid-stroke never switches between
/// painting and sampling.
pub(super) fn alt_eyedropper(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if let ToolEvent::Down { x, y, .. } = ev {
        let physical_right = app.services.right_button_down.as_ref().is_some_and(|f| f());
        app.alt_sampling = !physical_right && !app.secondary_erase && (alt_samples(app.active_tool(), mods) || shape_alt_samples(app, mods, x, y));
    }
    if !app.alt_sampling {
        return false;
    }
    // The Triangle tool's ⇧⌥ at the press samples as ⌥ does, and the release point's colour is the
    // one it keeps (#13364).
    let triangle = app.active_tool() == Tool::Triangle;
    match ev {
        // ⇧⌥-click adds a colour sampler at the clicked pixel instead, as the Color Sampler tool
        // places one: no foreground change, nothing painted (#11890). A refused point (the
        // document's limit) reports in the status bar as that tool's does; off the canvas, as that
        // tool, it places none and says nothing. A press on an existing point wasn't measured, so
        // it samples the foreground as before.
        ToolEvent::Down { x, y, .. } if mods.shift && !triangle && !on_sampler(app, x, y) => {
            let size = app.session.active().map(|d| d.doc.size);
            let inside = size.is_some_and(|s| (0.0..f64::from(s.width)).contains(&x) && (0.0..f64::from(s.height)).contains(&y));
            if inside && let Err(e) = app.run("view.colorSamplers.add", json!({"x": x, "y": y})) {
                // Only the limit is a plain report; any other refusal (a failed Pen draft left in
                // another document) stays an error, also when the document is full.
                let limit = e == photocraft_engine::sampler_cmds::limit_message();
                app.ui.status = e;
                app.ui.status_error = !limit;
            }
        }
        // While ⇧⌥ stay held after that press, moves sample nothing.
        ToolEvent::Move { .. } if mods.shift && !triangle => {}
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } if triangle => sample_shape_fill(app, x, y),
        // Without ⌥: the sample sets the foreground colour, whatever the Eyedropper would do.
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } => sample_eyedropper(app, x, y, egui::Modifiers::NONE),
        ToolEvent::Up { x, y } => {
            if triangle {
                sample_shape_fill(app, x, y);
            }
            app.alt_sampling = false;
        }
    }
    true
}

/// The Triangle tool samples with ⌥ held at the press at (x, y) only in Shape mode and with no
/// shape layer active (#13364): with one, ⌥ subtracts from it. Path or Pixels mode and a press
/// with nothing to sample (off the image, over transparency) weren't measured and keep drawing
/// from the centre. Control+⌥ stays the brush-resize drag, as in [`alt_samples`].
fn shape_alt_samples(app: &mut PhotocraftApp, mods: egui::Modifiers, x: f64, y: f64) -> bool {
    mods.alt
        && !mods.ctrl
        && app.active_tool() == Tool::Triangle
        && app.ui.tool_options.shape_mode == "shape"
        && crate::vector_ui::active_shape_path(app).is_none()
        && eyedropper_color(app, x, y).is_some()
}

/// The Triangle tool's ⌥ sample: the colour at (x, y) becomes the foreground and the tool's Fill, a
/// solid fill, which follows the foreground (#13364). Where nothing is sampled, both stay.
fn sample_shape_fill(app: &mut PhotocraftApp, x: f64, y: f64) {
    if sampled_eyedropper(app, x, y, egui::Modifiers::NONE) {
        app.ui.tool_options.shape_fill = true;
        app.ui.tool_options.shape_fill_gradient = None;
    }
}

/// Is document point (x, y) on a colour sampler, within the Color Sampler tool's reach of one (7
/// screen pixels)?
fn on_sampler(app: &PhotocraftApp, x: f64, y: f64) -> bool {
    let tol = 7.0 / f64::from(app.point_zoom().max(0.01));
    app.session.active().is_some_and(|d| d.color_samplers.iter().any(|q| (q[0] - x).hypot(q[1] - y) <= tol))
}

#[cfg(test)]
#[path = "triangle_alt_eyedropper_tests.rs"]
mod triangle_alt_eyedropper_tests;

#[cfg(test)]
#[path = "../brush_shift_alt_sampler_tests.rs"]
mod brush_shift_alt_sampler_tests;

/// Sample the image or targeted raster mask at (x, y) using the configured Sample Size.
/// Write a valid sample to the foreground swatch, or to the background swatch with Alt.
pub(super) fn sample_eyedropper(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    sampled_eyedropper(app, x, y, mods);
}

/// [`sample_eyedropper`], true when it set a swatch.
fn sampled_eyedropper(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) -> bool {
    let rgb = if let Some((surface, canvas)) = eyedropper_mask_source(app) {
        sample_layer_mask(surface, canvas, x, y, app.ui.tool_options.eyedropper_size)
    } else {
        eyedropper_color(app, x, y).map(|rgb| if super::paint_target(app) == json!("pixels") { rgb } else { [photocraft_color::convert::rgb_to_gray(rgb); 3] })
    };
    if let Some(rgb) = rgb {
        let key = if mods.alt { "background" } else { "foreground" };
        return app.run("tools.setColors", json!({ key: [rgb[0],rgb[1],rgb[2],1.0] })).is_ok();
    }
    false
}

/// Only the Eyedropper's Current Layer and Current & Below modes pick the targeted raster mask.
fn eyedropper_mask_source(app: &PhotocraftApp) -> Option<(&photocraft_raster::Surface, photocraft_geom::Rect)> {
    use photocraft_engine::{channel_cmds::ChannelTarget, mask_view_cmds};

    if app.active_tool() != Tool::Eyedropper
        || app.ui.vector_mask_target
        || !matches!(app.ui.tool_options.eyedropper_sample.as_str(), "current" | "currentAndBelow")
    {
        return None;
    }
    let st = app.session.active()?;
    // Painting can implicitly edit an adjustment's mask even with its content thumbnail selected.
    let targeted = app.ui.mask_target || st.channel_view.target == ChannelTarget::LayerMask || mask_view_cmds::current(st).is_some_and(|v| !v.filter);
    if !targeted || st.doc.quick_mask.is_some() || !app.layer_mask_targeted() {
        return None;
    }
    let mask = st.doc.layer(st.active_layer?)?.mask.as_ref()?;
    Some((&mask.surface, st.doc.bounds()))
}

/// Read the stored mask channel, including its sparse default, rather than its composited coverage.
fn sample_layer_mask(surface: &photocraft_raster::Surface, canvas: photocraft_geom::Rect, x: f64, y: f64, size: u32) -> Option<[f32; 3]> {
    let rect = photocraft_engine::cmd::sample_cmds::sample_rect(x, y, size, canvas)?;
    let mut sum = 0.0;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            sum += f64::from(surface.sample_channel(x, y, 0));
        }
    }
    let gray = (sum / (f64::from(rect.width()) * f64::from(rect.height()))) as f32;
    gray.is_finite().then_some([gray; 3])
}

/// The colour at document point (x, y) with the Eyedropper's Sample Size (the composite pixel,
/// or the average of the square around it) from the layers `sample_layer` names (`document.sampleColor`).
/// `None` off the image or over transparency.
fn sample_color(app: &mut PhotocraftApp, x: f64, y: f64, sample_layer: &str) -> Option<[f32; 3]> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let size = app.ui.tool_options.eyedropper_size;
    // The box depends on the view zoom at 100 % and below, as in Photoshop (#11751).
    let zoom = f64::from(app.current_zoom());
    let v = app.run("document.sampleColor", json!({"x": x, "y": y, "size": size, "sampleLayer": sample_layer, "zoom": zoom})).ok()?;
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

pub(crate) fn alt_loads_mixer(tool: Tool, mods: egui::Modifiers) -> bool {
    mods.alt && !mods.ctrl && tool == Tool::MixerBrush
}
pub(crate) fn alt_picks(tool: Tool, mods: egui::Modifiers) -> bool {
    alt_samples(tool, mods) || alt_loads_mixer(tool, mods)
}

#[cfg(test)]
#[path = "../eyedropper_mask_sample_tests.rs"]
mod eyedropper_mask_sample_tests;

#[cfg(test)]
#[path = "../color_replacement_cursor_tests.rs"]
mod color_replacement_cursor_tests;

#[cfg(test)]
#[path = "../color_replacement_alt_click_tests.rs"]
mod color_replacement_alt_click_tests;

#[cfg(test)]
#[path = "../background_eraser_alt_click_tests.rs"]
mod background_eraser_alt_click_tests;

#[cfg(test)]
#[path = "remove_cursor_tests.rs"]
mod remove_cursor_tests;

#[cfg(test)]
#[path = "paint_bucket_cursor_tests.rs"]
mod paint_bucket_cursor_tests;
