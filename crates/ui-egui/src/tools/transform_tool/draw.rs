use super::begin::warps_type;
use super::drag_modes::{HandleIndicator, handle_indicator};
use super::options_bar::{quad_scale, readout};
use super::pointer::{Hit, handle_at, rotate_dir, rotation_degrees};
use super::warp::{on_section_edge, quick_split_kind, single_patch};
use super::{
    BezierMesh, Color32, DocId, HANDLE_PX, Homography, LayerId, PhotocraftApp, Pos2, SplitTool, Stroke, TransformPreview, TransformSession, ViewXform, Warp,
    WarpDrag, WarpStyle, corners, pos2, vec2,
};

/// The distort or skew pointer at screen point `at`, where [`super::cursor`] hid the system
/// pointer over a handle. The canvas calls it from its cursor branch, so nothing is drawn where a
/// guide, a pan or a zoom set the cursor instead.
pub(crate) fn draw_handle_cursor(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, at: Pos2, mods: egui::Modifiers) {
    let Some(t) = &app.ui.transform else { return };
    let h = handle_at(app, t, xf.to_doc(at), super::handle_tolerance(app));
    let scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    match (h, handle_indicator(app, t, h, mods)) {
        (Hit::Corner(i), Some(HandleIndicator::Distort)) => {
            if let Some(q) = t.quad.get(i) {
                draw_distort_cursor(ctx, at, scr(*q) - scr(t.pivot));
            }
        }
        (Hit::Edge(i), Some(HandleIndicator::Skew)) => {
            if let (Some(a), Some(b)) = (t.quad.get(i), t.quad.get((i + 1) % 4)) {
                draw_skew_cursor(ctx, at, scr(*b) - scr(*a));
            }
        }
        _ => {}
    }
}

/// Draws the Free Distort cursor at `at`: a high-contrast vector pointer pointing toward
/// or outward through the corner, with shadow outline for contrast on any canvas background.
fn draw_distort_cursor(ctx: &egui::Context, at: Pos2, toward: egui::Vec2) {
    let len = toward.length();
    let u = if len.is_finite() && len > 1e-6 { toward / len } else { vec2(-1.0, -1.0).normalized() };
    let perp = vec2(-u.y, u.x);
    let tokens = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("transform-distort-cursor")));

    // An arrowhead dart pointing along `u` with tip at `at`.
    let tip = at;
    const LENGTH: f32 = 16.0;
    const HALF_WIDTH: f32 = 7.0;
    let base = tip - u * LENGTH;
    let left = base + perp * HALF_WIDTH;
    let right = base - perp * HALF_WIDTH;
    let notch = tip - u * (LENGTH * 0.65);

    // Convex decomposition: two triangles meeting along [tip, notch].
    painter.add(egui::Shape::convex_polygon(vec![tip, left, notch], tokens.accent_text, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(vec![tip, notch, right], tokens.accent_text, Stroke::NONE));
    painter.add(egui::Shape::closed_line(vec![tip, left, notch, right], Stroke::new(1.5, tokens.shadow)));
}

/// Draws the Skew cursor at `at`: opposing parallel arrows aligned with the edge tangent `tangent`.
fn draw_skew_cursor(ctx: &egui::Context, at: Pos2, tangent: egui::Vec2) {
    let len = tangent.length();
    let u = if len.is_finite() && len > 1e-6 { tangent / len } else { vec2(1.0, 0.0) };
    let perp = vec2(-u.y, u.x);
    let tokens = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("transform-skew-cursor")));

    for (offset, dir) in [(4.0f32, 1.0f32), (-4.0f32, -1.0f32)] {
        let center = at + perp * offset;
        let p_start = center - u * (dir * 9.0);
        let p_end = center + u * (dir * 9.0);
        painter.line_segment([p_start, p_end], Stroke::new(3.5, tokens.shadow));
        painter.line_segment([p_start, p_end], Stroke::new(1.5, tokens.accent_text));

        let tip = p_end + u * (dir * 3.5);
        let wing1 = p_end - u * (dir * 2.5) + perp * 3.5;
        let wing2 = p_end - u * (dir * 2.5) - perp * 3.5;
        painter.add(egui::Shape::convex_polygon(vec![tip, wing1, wing2], tokens.accent_text, Stroke::new(1.0, tokens.shadow)));
    }
}

/// For each vertex of the preview's `n` × `n` grid over the unit square (row by row), whether it
/// maps to the front side of `h`'s horizon. A Distort dragged past the horizon sends the rest of
/// the image to the far side, where its cells would wrap round infinity; the preview leaves them
/// out, as the committed warp does (#4863).
pub(super) fn front_vertices(h: &Homography, n: usize) -> Vec<bool> {
    let front = h.front_side([0.0, 0.0, 1.0, 1.0]);
    (0..=n).flat_map(|j| (0..=n).map(move |i| (i as f64 / n as f64, j as f64 / n as f64))).map(|(u, v)| front * h.w(u, v) > 1e-9).collect()
}

/// The moving pixels as the box shows them (warped, skewed or styled), plus the layers above
/// them. This is image content: the canvas draws it right after the document, under the grid
/// and guides, so a guide stays visible across a layer being transformed (#5086), as in
/// Photoshop. The box, handles and reference point are [`draw_overlay`], drawn over the guides.
pub fn draw_content(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview) else { return };
    let output = crate::monitor_status::view_display(app, painter.ctx());
    let (display, _) = crate::canvas::canvas_display(app, &pv.doc, output);
    // Selection tint and lone mask/channel values are UI diagnostics, not document colors.
    let display = display.as_deref().filter(|_| !t.selection && t.target.is_none() && t.path.is_none());
    if let Some(w) = &t.warp {
        draw_warp_content(painter, xf, t, w, pv, display);
        return;
    }
    let ctx = painter.ctx();
    let styled =
        styled_frame(app, xf.zoom * ctx.pixels_per_point(), ctx.input(|i| i.pointer.any_down())).and_then(|(s, frame)| s.texture(ctx, &frame, display));
    if let Some((tex, r)) = styled {
        let mut mesh = egui::Mesh::with_texture(tex);
        for (x, y, u, v) in [(r[0], r[1], 0.0, 0.0), (r[2], r[1], 1.0, 0.0), (r[2], r[3], 1.0, 1.0), (r[0], r[3], 0.0, 1.0)] {
            mesh.vertices.push(egui::epaint::Vertex { pos: pos2(x, y), uv: pos2(u, v), color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(clip_to_canvas(mesh, canvas_size(pv), |p| xf.to_screen(p.x, p.y)));
    } else if let Some(h) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], t.quad) {
        // A 24×24 grid keeps perspective previews straight.
        let n = 24;
        let zoom = xf.zoom * painter.ctx().pixels_per_point();
        // Content-Aware Scale shows its latest live result once there is one.
        let (tex, uv) = match pv.content_aware.as_ref().and_then(|l| l.shown(t)) {
            Some((texture, texel)) => texture.pick_display(painter.ctx(), zoom, texel, false, display),
            None => pv.texture.pick_display(painter.ctx(), zoom, quad_scale(t), t.interpolation == "nearest", display),
        };
        let mut mesh = egui::Mesh::with_texture(tex);
        let tint = Color32::from_white_alpha((pv.opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let ahead = front_vertices(&h, n);
        for j in 0..=n {
            for i in 0..=n {
                let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                let (x, y) = h.apply(u, v);
                mesh.vertices.push(egui::epaint::Vertex { pos: pos2(x as f32, y as f32), uv: pos2(u as f32 * uv[0], v as f32 * uv[1]), color: tint });
            }
        }
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                for tri in [[a, a + 1, a + n as u32 + 2], [a, a + n as u32 + 2, a + n as u32 + 1]] {
                    // A cell with a corner past the horizon would stretch across the canvas (#4863).
                    if tri.iter().all(|&k| ahead[k as usize]) {
                        mesh.add_triangle(tri[0], tri[1], tri[2]);
                    }
                }
            }
        }
        painter.add(clip_to_canvas(mesh, canvas_size(pv), |p| xf.to_screen(p.x, p.y)));
    }
    if let Some(a) = &pv.above {
        a.draw(painter, xf, display);
    }
}

/// The box, handles, reference point and path outline over the preview that [`draw_content`]
/// drew; the warp grid and its split guides for a warp.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview) else { return };
    let scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    if let Some(w) = &t.warp {
        let over_canvas = painter.ctx().input(|i| i.pointer.hover_pos()).filter(|p| xf.rect.contains(*p)).map(|p| xf.to_doc(p));
        let at = pv.split_pointer.or(over_canvas);
        // The same keys as `cursor`, sticky ones included, so the guide matches the pointer.
        let mods = crate::workspace_ui::sticky_mods(app, painter.ctx().input(|i| i.modifiers));
        let alt = mods.alt || mods.command;
        let guide = at.filter(|_| !warps_type(app)).and_then(|p| {
            let tool = pv.split_tool.or_else(|| {
                (alt || pv.split_quick).then(|| {
                    let m = w.to_mesh(1, 1);
                    quick_split_kind(&m, w.bounds, m.param_at(p))
                })
            });
            tool.map(|tool| (tool, p))
        });
        draw_warp_overlay(painter, xf, w, pv, guide);
        return;
    }
    if pv.committing.is_some() {
        return;
    }
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    if let (Some(path), Some(h)) = (&pv.path, Homography::rect_to_quad(t.rect, t.quad)) {
        crate::vector_ui::draw_outline(
            painter,
            path,
            &|q| {
                let (x, y) = h.apply(q[0], q[1]);
                xf.to_screen(x as f32, y as f32)
            },
            accent,
        );
    }
    let pts: Vec<Pos2> = t.quad.iter().map(|q| scr(*q)).collect();
    painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.0, accent)));
    let mids: Vec<Pos2> = (0..4).map(|i| pts[i].lerp(pts[(i + 1) % 4], 0.5)).collect();
    for p in pts.iter().chain(mids.iter()) {
        let r = egui::Rect::from_center_size(*p, vec2(7.0, 7.0));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
    // Reference point (Photoshop's crosshair circle).
    let c = scr(t.pivot);
    painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::WHITE));
    painter.circle_stroke(c, 5.0, Stroke::new(0.5, Color32::BLACK));
    painter.line_segment([c - vec2(8.0, 0.0), c + vec2(8.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
    painter.line_segment([c - vec2(0.0, 8.0), c + vec2(0.0, 8.0)], Stroke::new(1.0, Color32::WHITE));
    if let Some(at) = painter.ctx().pointer_hover_pos().filter(|p| xf.rect.contains(*p)) {
        let p = xf.to_doc(at);
        let tol = HANDLE_PX / f64::from(xf.zoom.max(0.01));
        if let Some(k) = rotate_dir(t, Some(pv), p, tol) {
            // Through the view, so the arc bends round the corner as it shows under a turned or
            // flipped view.
            let toward = xf.to_screen((p[0] + k[0] * 16.0) as f32, (p[1] + k[1] * 16.0) as f32) - at;
            crate::crop_ui::draw_turn_cursor(painter.ctx(), at, toward);
        }
        // Preferences › Tools › Show Transformation Values off hides it, as in Photoshop (#204).
        if let Some(r) = live_readout(app).filter(|_| app.session.prefs().tools.show_transformation_values) {
            draw_live_readout(app, painter.ctx(), at, r);
        }
    }
}

/// Applying the open box moves the selection outline with it: `edit.transform` and
/// `edit.transform.warp` map it like the pixels. Not Transform Selection (its ants are hidden), a
/// path, Content-Aware Scale or a type layer's Warp Text, which leave the selection alone.
fn carries_selection(app: &PhotocraftApp, t: &TransformSession) -> bool {
    !t.selection && t.path.is_none() && t.content_aware.is_none() && !(t.warp.is_some() && warps_type(app))
}

/// A transform of `document` is open that carries its selection along (#5297): the canvas traces
/// the whole outline, not only the part in view where it started, for [`draw_selection`].
pub(crate) fn carries_selection_of(app: &PhotocraftApp, document: DocId) -> bool {
    matches!((&app.ui.transform, &app.transform_preview), (Some(t), Some(pv)) if pv.doc.id == document && carries_selection(app, t))
}

/// The marching ants where the open box puts them (#5297): the selection follows a move, scale,
/// rotation, skew, distort, perspective or warp while it is made, to where applying it leaves it.
/// `segs` is the outline of `document`'s selection at rest. False when no open transform carries
/// that selection: the caller draws it where it is.
pub(crate) fn draw_selection(
    app: &PhotocraftApp,
    painter: &egui::Painter,
    xf: &ViewXform,
    document: DocId,
    segs: &[crate::outline::Segment],
    time: f64,
) -> bool {
    let Some(t) = app.ui.transform.as_ref().filter(|_| carries_selection_of(app, document)) else { return false };
    let scr = |(x, y): (f64, f64)| xf.to_screen(x as f32, y as f32);
    let doc = |p: [i32; 2]| [f64::from(p[0]), f64::from(p[1])];
    let along = |a: [f64; 2], b: [f64; 2], k: f64| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
    if let Some(w) = &t.warp {
        // A warp bends the outline's straight runs: each is drawn in pieces about 8 points long.
        let piece = (8.0 / f64::from(xf.zoom.max(1e-3))).max(1.0);
        let lines = segs.iter().flat_map(|(a, b)| {
            let (a, b) = (doc(*a), doc(*b));
            let n = (((b[0] - a[0]).abs() + (b[1] - a[1]).abs()) / piece).ceil().clamp(1.0, 256.0) as usize;
            let at = move |i: usize| {
                let p = along(a, b, i as f64 / n as f64);
                scr(w.map(p[0], p[1]))
            };
            (0..n).map(move |i| (at(i), at(i + 1)))
        });
        crate::canvas::marching_ants_lines(painter, lines, time);
    } else if let Some(h) = Homography::rect_to_quad(t.rect, t.quad) {
        // Past a Distort's horizon the outline has no place, like the pixels (#4863): a run that
        // crosses it stops short of it, where its points are still a finite way off.
        let front = h.front_side(t.rect);
        let depth = |p: [f64; 2]| front * h.w(p[0], p[1]);
        let near = corners(t.rect).iter().map(|c| depth(*c)).fold(0.0, f64::max) * 1e-3;
        let lines = segs.iter().filter_map(|(a, b)| {
            let (mut a, mut b) = (doc(*a), doc(*b));
            let (da, db) = (depth(a), depth(b));
            if da < near && db < near {
                return None;
            }
            if da < near {
                a = along(a, b, (near - da) / (db - da));
            } else if db < near {
                b = along(b, a, (near - db) / (da - db));
            }
            Some((scr(h.apply(a[0], a[1])), scr(h.apply(b[0], b[1]))))
        });
        crate::canvas::marching_ants_lines(painter, lines, time);
    }
    true
}

/// What the styled preview of the moving layer is drawn from (transform_fx.rs), at `zoom` screen
/// pixels per document pixel. `None` for a layer without a style (also once it is switched off
/// while the box is open) or a hidden one: the bare pixels are drawn instead. The style, opacity
/// and fill are the layer's as they are now, so edits made while the box is open show.
pub(crate) fn styled_frame(app: &PhotocraftApp, zoom: f32, dragging: bool) -> Option<(&crate::transform_fx::StyledPreview, crate::transform_fx::Frame<'_>)> {
    let (t, pv) = (app.ui.transform.as_ref()?, app.transform_preview.as_ref()?);
    let styled = pv.styled.as_ref().filter(|_| t.warp.is_none())?;
    let st = app.session.active().filter(|st| st.doc.id == pv.doc.id)?;
    let layer = st.doc.layer(LayerId(t.layer)).filter(|l| l.visible && photocraft_compose::effects::has_effects(l))?;
    let frame = crate::transform_fx::Frame {
        doc: &st.doc,
        layer,
        revision: st.revision,
        texels: &pv.texture,
        quad: t.quad,
        quad_scale: quad_scale(t),
        nearest: t.interpolation == "nearest",
        zoom,
        dragging,
    };
    Some((styled, frame))
}

/// The value shown beside the pointer while the box is dragged, like Photoshop's Show
/// Transformation Values: the box's angle while rotating, its scale while resizing, the offset
/// while moving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Readout {
    /// Clockwise degrees in document space (`rotation_degrees`), whatever the view's turn or flip.
    Angle(f64),
    /// Width and height in percent of the original box, and the box's size in document pixels.
    /// The size is the scaled width and height (what W and H 100 % mean in the options bar),
    /// not the edge lengths: a skew leans the sides, which lengthens them but not the height
    /// between the top and bottom edges.
    Scale { percent: [f64; 2], px: [f64; 2] },
    /// Document pixels moved since the press.
    Offset([f64; 2]),
}

/// The readout for the drag in progress, or `None` when nothing is dragged (or the drag is the
/// reference point, a warp, or a distortion that has no single scale).
pub(crate) fn live_readout(app: &PhotocraftApp) -> Option<Readout> {
    let t = app.ui.transform.as_ref().filter(|t| t.warp.is_none())?;
    let g = app.transform_preview.as_ref()?.gesture?;
    match g.hit {
        Hit::Outside => Some(Readout::Angle(rotation_degrees(t.quad))),
        Hit::Inside => Some(Readout::Offset([t.quad[0][0] - g.quad0[0][0], t.quad[0][1] - g.quad0[0][1]])),
        Hit::Corner(_) | Hit::Edge(_) if parallelogram(&t.quad) => {
            let (w, h, _, _) = readout(t);
            let px = [w / 100.0 * (t.rect[2] - t.rect[0]), h / 100.0 * (t.rect[3] - t.rect[1])];
            (w.is_finite() && h.is_finite()).then_some(Readout::Scale { percent: [w, h], px })
        }
        Hit::Corner(_) | Hit::Edge(_) | Hit::Pivot => None,
    }
}

/// True when `q` is still an affine image of the box (opposite sides parallel), so a W/H scale
/// describes it; a distorted or perspective box has none.
fn parallelogram(q: &[[f64; 2]; 4]) -> bool {
    let scale = q.iter().flatten().fold(1.0f64, |m, v| m.max(v.abs()));
    (0..2).all(|k| ((q[0][k] + q[2][k]) - (q[1][k] + q[3][k])).abs() <= scale * 1e-9)
}

/// [`live_readout`] as the rounded label beside the pointer (the marquee's and Crop's readout).
fn draw_live_readout(app: &PhotocraftApp, ctx: &egui::Context, at: Pos2, r: Readout) {
    let id = "transform-readout";
    match r {
        Readout::Angle(a) => crate::canvas::draw_readout(ctx, id, at, ["Angle:"], [format!("{:.1}°", fixed(a, 1))]),
        Readout::Scale { percent, px } => crate::canvas::draw_readout(ctx, id, at, ["W:", "H:"], scale_values(app, percent, px)),
        Readout::Offset([dx, dy]) => crate::canvas::draw_readout(ctx, id, at, ["ΔX:", "ΔY:"], [dx, dy].map(|v| format!("{:.0} px", fixed(v, 0)))),
    }
}

/// `v` rounded to `places` decimals, so a hair below zero never reads "-0".
fn fixed(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    let v = (v * k).round() / k;
    if v == 0.0 { 0.0 } else { v }
}

/// The W and H rows of the scale readout: the box's size in the Units & Rulers unit at the
/// document's resolution, as Photoshop shows it (cm when the rulers are in centimeters, #9400);
/// the scale of the original box when the rulers are in percent.
pub(crate) fn scale_values(app: &PhotocraftApp, percent: [f64; 2], px: [f64; 2]) -> [String; 2] {
    if app.session.prefs().units_and_rulers.rulers == photocraft_engine::prefs::Unit::Percent {
        return percent.map(|v| format!("{:.1}%", fixed(v, 1)));
    }
    crate::canvas::size_readout(app, [0.0, 0.0, px[0], px[1]])
}

/// Width and height of the document being transformed, in document pixels.
fn canvas_size(pv: &TransformPreview) -> [f32; 2] {
    [pv.doc.size.width as f32, pv.doc.size.height as f32]
}

/// Clips `mesh` (positions in document pixels) to the canvas `[0, 0, size]` and maps it to the
/// screen. Pixels past the canvas edge are never shown, also not while transforming (Photoshop;
/// #2027): only the box and its handles reach over the pasteboard. Clipping in document space
/// stays exact under a rotated or flipped view, where a screen clip rectangle would not.
fn clip_to_canvas(mut mesh: egui::Mesh, size: [f32; 2], to_screen: impl Fn(Pos2) -> Pos2) -> egui::Mesh {
    let inside = |p: Pos2| p.x >= 0.0 && p.y >= 0.0 && p.x <= size[0] && p.y <= size[1];
    if mesh.vertices.iter().all(|v| inside(v.pos)) {
        for v in &mut mesh.vertices {
            v.pos = to_screen(v.pos);
        }
        return mesh;
    }
    let mut out = egui::Mesh::with_texture(mesh.texture_id);
    for tri in mesh.indices.as_chunks::<3>().0 {
        let mut poly: Vec<egui::epaint::Vertex> = tri.iter().filter_map(|i| mesh.vertices.get(*i as usize).copied()).collect();
        // Sutherland–Hodgman against the four canvas edges (`dist` >= 0 is the canvas side).
        for edge in 0..4 {
            let dist = |v: &egui::epaint::Vertex| match edge {
                0 => v.pos.x,
                1 => size[0] - v.pos.x,
                2 => v.pos.y,
                _ => size[1] - v.pos.y,
            };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for (k, a) in poly.iter().enumerate() {
                let Some(b) = poly.get((k + 1) % poly.len()) else { continue };
                let (da, db) = (dist(a), dist(b));
                if da >= 0.0 {
                    next.push(*a);
                }
                if (da >= 0.0) != (db >= 0.0) && da != db {
                    let t = da / (da - db);
                    next.push(egui::epaint::Vertex { pos: a.pos.lerp(b.pos, t), uv: a.uv.lerp(b.uv, t), color: a.color });
                }
            }
            poly = next;
        }
        if poly.len() < 3 {
            continue;
        }
        let base = out.vertices.len() as u32;
        out.vertices.extend(poly.iter().map(|v| egui::epaint::Vertex { pos: to_screen(v.pos), ..*v }));
        for k in 1..poly.len() as u32 - 1 {
            out.add_triangle(base, base + k, base + k + 1);
        }
    }
    out
}

/// Warp preview (the moving pixels on a fine textured mesh) plus the control mesh. Patch
/// boundaries are solid. A single patch (Grid › Default) also draws its rule-of-thirds guides;
/// 3×3, 4×4 and 5×5 are just those even cells, with an anchor at every intersection. `guide` is
/// the split line following the pointer.
/// How near the pointer (screen px) a handle's anchor or the handle itself must be for the
/// handle to show. Everything else stays out of the way: only anchors are always drawn.
const HANDLE_REVEAL_PX: f32 = 56.0;

/// The anchor a handle belongs to, as `(i, j)` in the control grid (an anchor is its own).
pub(super) fn handle_anchor(i: usize, j: usize) -> (usize, usize) {
    if i.is_multiple_of(3) {
        let aj = match j % 3 {
            0 => j,
            1 => j - 1,
            _ => j + 1,
        };
        (i, aj)
    } else {
        (if i % 3 == 1 { i - 1 } else { i + 1 }, j)
    }
}

/// Anchors always, small and quiet. A handle and its arm show only while the pointer is close to
/// them or they are being dragged, so a dense grid does not bury the picture in dots.
fn draw_warp_points(painter: &egui::Painter, m: &BezierMesh, scr: impl Fn([f64; 2]) -> Pos2, pv: &TransformPreview, line: Stroke) {
    let (nx, ny) = (m.nx(), m.ny());
    let hover = painter.ctx().input(|i| i.pointer.hover_pos());
    let dragged = match &pv.warp_drag {
        Some(WarpDrag::Point { index, .. }) => Some(handle_anchor(index % nx, index / nx)),
        _ => None,
    };
    let near = |p: Pos2| hover.is_some_and(|h| h.distance(p) <= HANDLE_REVEAL_PX);
    let faint = Stroke::new(0.75, line.color.gamma_multiply(0.8));
    for j in 0..ny {
        for i in 0..nx {
            if !on_section_edge(i, j) {
                continue;
            }
            let p = scr(m.point(i, j));
            if i.is_multiple_of(3) && j.is_multiple_of(3) {
                let r = egui::Rect::from_center_size(p, vec2(5.0, 5.0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                painter.rect_stroke(r, 0.0, faint, egui::StrokeKind::Inside);
                continue;
            }
            let (ai, aj) = handle_anchor(i, j);
            let a = scr(m.point(ai, aj));
            if !(near(p) || near(a) || dragged == Some((ai, aj))) {
                continue;
            }
            painter.line_segment([a, p], faint);
            painter.circle_filled(p, 2.75, Color32::WHITE);
            painter.circle_stroke(p, 2.75, faint);
        }
    }
}

/// The warped pixels and the layers above them ([`draw_content`] for a warp).
fn draw_warp_content(
    painter: &egui::Painter,
    xf: &ViewXform,
    t: &TransformSession,
    w: &Warp,
    pv: &TransformPreview,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
) {
    let r = t.rect;
    let m = w.to_mesh(1, 1);
    let (us, vs) = crate::warp_preview::samples(&m, xf.zoom as f64);
    let (tex, uv) = pv.texture.pick_display(painter.ctx(), xf.zoom * painter.ctx().pixels_per_point(), 1.0, t.interpolation == "nearest", display);
    let mut mesh = egui::Mesh::with_texture(tex);
    let tint = Color32::from_white_alpha((pv.opacity.clamp(0.0, 1.0) * 255.0) as u8);
    for v in &vs {
        for u in &us {
            let (x, y) = w.map(r[0] + u * (r[2] - r[0]), r[1] + v * (r[3] - r[1]));
            mesh.vertices.push(egui::epaint::Vertex { pos: pos2(x as f32, y as f32), uv: pos2(*u as f32 * uv[0], *v as f32 * uv[1]), color: tint });
        }
    }
    let pos: Vec<[f32; 2]> = mesh.vertices.iter().map(|v| [v.pos.x, v.pos.y]).collect();
    let rest: Vec<[f32; 2]> = vs.iter().flat_map(|v| us.iter().map(move |u| [(r[0] + u * (r[2] - r[0])) as f32, (r[1] + v * (r[3] - r[1])) as f32])).collect();
    mesh.indices = crate::warp_preview::triangle_order(&pos, &rest, us.len());
    painter.add(clip_to_canvas(mesh, canvas_size(pv), |p| xf.to_screen(p.x, p.y)));
    if let Some(a) = &pv.above {
        a.draw(painter, xf, display);
    }
}

/// The warp grid, its control points and split guides ([`draw_overlay`] for a warp).
fn draw_warp_overlay(painter: &egui::Painter, xf: &ViewXform, w: &Warp, pv: &TransformPreview, guide: Option<(SplitTool, [f64; 2])>) {
    let scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    let m = w.to_mesh(1, 1);
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    // Transform Path › Warp: the path drawn through the warp.
    if let Some(path) = &pv.path {
        crate::vector_ui::draw_outline(
            painter,
            path,
            &|q| {
                let (x, y) = w.map(q[0], q[1]);
                xf.to_screen(x as f32, y as f32)
            },
            accent,
        );
    }
    let line = Stroke::new(1.0, accent);
    let thin = Stroke::new(0.75, accent.gamma_multiply(0.7));
    let curve = |fixed_u: Option<f64>, fixed_v: Option<f64>| -> Vec<Pos2> {
        (0..=40)
            .map(|k| {
                let s = k as f64 / 40.0;
                let p = m.eval(fixed_u.unwrap_or(s), fixed_v.unwrap_or(s));
                scr(p)
            })
            .collect()
    };
    // Patch boundaries are the grid. Only Default (one patch) draws the rule-of-thirds guides;
    // a 3×3 preset is three even cells, not those guides repeated inside every cell.
    let thirds = single_patch(&m);
    for (k, win) in m.us.windows(2).enumerate() {
        if thirds {
            for third in [1.0, 2.0] {
                painter.add(egui::Shape::line(curve(Some(win[0] + (win[1] - win[0]) * third / 3.0), None), thin));
            }
        }
        if k == 0 {
            painter.add(egui::Shape::line(curve(Some(win[0]), None), line));
        }
        painter.add(egui::Shape::line(curve(Some(win[1]), None), line));
    }
    for (k, win) in m.vs.windows(2).enumerate() {
        if thirds {
            for third in [1.0, 2.0] {
                painter.add(egui::Shape::line(curve(None, Some(win[0] + (win[1] - win[0]) * third / 3.0)), thin));
            }
        }
        if k == 0 {
            painter.add(egui::Shape::line(curve(None, Some(win[0])), line));
        }
        painter.add(egui::Shape::line(curve(None, Some(win[1])), line));
    }
    if w.style == WarpStyle::Custom {
        draw_warp_points(painter, &m, scr, pv, line);
    }
    if let Some((tool, p)) = guide {
        let (s, t) = m.param_at(p);
        let under = Stroke::new(3.0, Color32::BLACK);
        let over = Stroke::new(1.25, Color32::WHITE);
        let mut strokes = Vec::new();
        if matches!(tool, SplitTool::Vertical | SplitTool::Cross) {
            strokes.push(curve(Some(s), None));
        }
        if matches!(tool, SplitTool::Horizontal | SplitTool::Cross) {
            strokes.push(curve(None, Some(t)));
        }
        for pts in strokes {
            painter.add(egui::Shape::line(pts.clone(), under));
            painter.add(egui::Shape::line(pts, over));
        }
    }
}
