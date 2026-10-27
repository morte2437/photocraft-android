use super::display::{canvas_display, checker, display_doc, display_image};
use super::drag::{
    angle_readout, draw_marquee_readout, draw_value_readout, marquee_ants, marquee_drag_ants, marquee_preview_px, single_line_drag_px, size_readout,
};
use super::gesture::constrained_polygon_point;
use super::strokes::{EngineStroke, SPOT_HEALING_TRAIL, draw_smoothing_string, strokes_live};
use super::textures::texture_side;
use super::view_xform::ViewXform;
use super::*;

/// A texture mapped over the `size` document area starting at `origin` (follows a rotated view).
pub(super) fn paint_mapped_image_at(painter: &egui::Painter, xf: &ViewXform, tex: egui::TextureId, uv: Rect, origin: [f32; 2], size: [f32; 2], tint: Color32) {
    let ([x, y], [w, h]) = (origin, size);
    let pts = [xf.to_screen(x, y), xf.to_screen(x + w, y), xf.to_screen(x + w, y + h), xf.to_screen(x, y + h)];
    let uvs = [uv.min, pos2(uv.max.x, uv.min.y), uv.max, pos2(uv.min.x, uv.max.y)];
    let mut mesh = Mesh::with_texture(tex);
    for i in 0..4 {
        mesh.vertices.push(egui::epaint::Vertex { pos: pts[i], uv: uvs[i], color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The GPU canvas view, then its View › Pattern Preview copies (#1067): the same texture drawn
/// again whole document sizes away, each with its own view uniform and no shadow.
pub(super) fn paint_gpu_view(painter: &egui::Painter, rect: Rect, params: crate::gpu_canvas::ViewParams, copies: &[[i32; 2]]) {
    crate::gpu_canvas::GpuCanvas::paint(painter, rect, params);
    for (k, &c) in copies.iter().enumerate() {
        let copy = crate::gpu_canvas::ViewParams {
            offset: crate::pattern_preview::offset(c, params.doc_size),
            shadow: false,
            // Keyed by slot, not by tile index, so panning reuses the same few uniform buffers.
            view_key: egui::Id::unique((params.view_key, "pattern", k)).value(),
            ..params
        };
        crate::gpu_canvas::GpuCanvas::paint(painter, rect, copy);
    }
}

/// The mapping that draws Pattern Preview copy `c` of a `size` document where it sits next to
/// the document: the same view, looking the matching number of document sizes back.
pub(super) fn pattern_xf(xf: &ViewXform, c: [i32; 2], size: [u32; 2]) -> ViewXform {
    let [ox, oy] = crate::pattern_preview::offset(c, size);
    ViewXform { center: [xf.center[0] - ox, xf.center[1] - oy], ..*xf }
}

/// CPU canvas: the transparency checkerboard (or white) and the display texture, for the
/// document or one of its Pattern Preview copies (`xf` from [`pattern_xf`]).
pub(super) fn paint_cpu_document(
    painter: &egui::Painter,
    xf: &ViewXform,
    bounds: DRect,
    rotated: bool,
    checker: Option<(egui::TextureId, f32)>,
    tex: Option<egui::TextureId>,
    opaque: bool,
) {
    let size = [bounds.width() as f32, bounds.height() as f32];
    let origin = [bounds.x0 as f32, bounds.y0 as f32];
    let img_rect = xf.doc_rect(bounds);
    match checker {
        Some((checker_id, square)) if !opaque || tex.is_none() => {
            if rotated {
                let tiles = vec2(size[0], size[1]) / (2.0 * square);
                paint_mapped_image_at(painter, xf, checker_id, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), origin, size, Color32::WHITE);
            } else {
                let tiles = img_rect.size() / (2.0 * square);
                painter.image(checker_id, img_rect, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), Color32::WHITE);
            }
        }
        // On software window adapters this hidden full-canvas pass is CPU rasterization too.
        // Keep the white-background branch unchanged (its antialiased outer edge is visible).
        Some(_) => {}
        None => {
            if rotated {
                paint_mapped_quad(painter, xf, origin, size, Color32::WHITE);
            } else {
                painter.rect_filled(img_rect, 0.0, Color32::WHITE);
            }
        }
    }
    if let Some(tex) = tex {
        if rotated {
            paint_mapped_image_at(painter, xf, tex, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), origin, size, Color32::WHITE);
        } else {
            let uv = if xf.flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
            painter.image(tex, img_rect, uv, Color32::WHITE);
        }
    }
}

/// [`ensure_texture_in_view`]'s detail texture over document area `area` (x0, y0, x1, y1), on its
/// own checkerboard (or white), so the reduced texture under it doesn't show through where the
/// image is transparent. The checkerboard keeps [`paint_cpu_document`]'s phase.
pub(super) fn paint_cpu_detail(
    painter: &egui::Painter,
    xf: &ViewXform,
    bounds: DRect,
    rotated: bool,
    checker: Option<(egui::TextureId, f32)>,
    tex: egui::TextureId,
    area: [f32; 4],
) {
    let [x0, y0, x1, y1] = area;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let pts = corners.map(|[x, y]| xf.to_screen(x, y));
    let quad = |tex: egui::TextureId, uvs: [Pos2; 4]| {
        let mut mesh = Mesh::with_texture(tex);
        for (pos, uv) in pts.into_iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    };
    match checker {
        Some((checker_id, square)) => {
            let tile = 2.0 * square;
            let uvs = if rotated {
                corners.map(|[x, y]| pos2(x / tile, y / tile))
            } else {
                let min = xf.doc_rect(bounds).min;
                pts.map(|p| ((p - min) / tile).to_pos2())
            };
            quad(checker_id, uvs);
        }
        None => {
            painter.add(egui::Shape::convex_polygon(pts.to_vec(), Color32::WHITE, Stroke::NONE));
        }
    }
    quad(tex, [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]);
}

fn paint_mapped_quad(painter: &egui::Painter, xf: &ViewXform, origin: [f32; 2], size: [f32; 2], color: Color32) {
    let ([x, y], [w, h]) = (origin, size);
    let pts = vec![xf.to_screen(x, y), xf.to_screen(x + w, y), xf.to_screen(x + w, y + h), xf.to_screen(x, y + h)];
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Draw boundary segments as marching ants: white base, black dashes phased along x + y.
pub(super) fn marching_ants_segments(painter: &egui::Painter, xf: &ViewXform, segs: &[crate::outline::Segment], time: f64) {
    marching_ants_mapped(painter, segs, time, |p| xf.to_screen(p[0] as f32, p[1] as f32));
}

/// [`marching_ants_segments`] with each outline point placed on screen by `to_screen` (a
/// transformed outline: Content-Aware Move's Transform On Drop).
pub(crate) fn marching_ants_mapped(painter: &egui::Painter, segs: &[crate::outline::Segment], time: f64, to_screen: impl Fn([i32; 2]) -> Pos2) {
    marching_ants_lines(painter, segs.iter().map(|(a, b)| (to_screen(*a), to_screen(*b))), time);
}

/// The part of the line `a`–`b` inside `r` (Liang–Barsky), if any.
pub(crate) fn clip_line(r: Rect, a: Pos2, b: Pos2) -> Option<(Pos2, Pos2)> {
    // Perspective can put endpoints billions of pixels away. f32 subtraction/interpolation
    // loses viewport-sized differences there, clipping visible ants to the wrong endpoints.
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, ax - f64::from(r.min.x)), (dx, f64::from(r.max.x) - ax), (-dy, ay - f64::from(r.min.y)), (dy, f64::from(r.max.y) - ay)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    let at = |t: f64| pos2((ax + dx * t) as f32, (ay + dy * t) as f32);
    (t0 <= t1).then(|| (at(t0), at(t1)))
}

/// Marching ants along lines already placed on screen (a warped outline is a run of short ones),
/// in device pixels like Photoshop's (`crate::ants`, #7489).
pub(crate) fn marching_ants_lines(painter: &egui::Painter, lines: impl Iterator<Item = (Pos2, Pos2)>, time: f64) {
    crate::ants::draw(painter, lines, time, true);
}

/// Photoshop crop overlay: the `shield` outside (`crop_shield::fill`; none when disabled), bright
/// frame, the composition `guides` (unit-square polylines, `crop_overlay::guides`), corner and
/// edge handles.
fn crop_overlay(painter: &egui::Painter, r: Rect, shield: Option<Color32>, guides: &[crate::crop_overlay::Polyline]) {
    let clip = painter.clip_rect();
    if let Some(dim) = shield {
        for band in [
            Rect::from_min_max(clip.min, egui::pos2(clip.max.x, r.min.y)),
            Rect::from_min_max(egui::pos2(clip.min.x, r.max.y), clip.max),
            Rect::from_min_max(egui::pos2(clip.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
            Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(clip.max.x, r.max.y)),
        ] {
            painter.rect_filled(band, 0.0, dim);
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    draw_crop_guides(painter, guides, |a, b| r.min + r.size() * vec2(a, b));
    let h = Stroke::new(3.0, Color32::WHITE);
    let l = 14.0f32.min(r.width() / 3.0).min(r.height() / 3.0);
    for (c, dx, dy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.left_bottom(), 1.0, -1.0), (r.right_bottom(), -1.0, -1.0)] {
        painter.line_segment([c, c + vec2(l * dx, 0.0)], h);
        painter.line_segment([c, c + vec2(0.0, l * dy)], h);
    }
    // Edge handles: short bars at the middle of each side.
    let (lx, ly) = (l.min(r.width() / 4.0) / 2.0, l.min(r.height() / 4.0) / 2.0);
    for (c, horizontal) in [(r.center_top(), true), (r.center_bottom(), true), (r.left_center(), false), (r.right_center(), false)] {
        let e = if horizontal { vec2(lx, 0.0) } else { vec2(0.0, ly) };
        painter.line_segment([c - e, c + e], h);
    }
}

/// [`crop_overlay`] for a turned frame (or a rotated view): `q` is the frame's top-left, top-right,
/// bottom-right and bottom-left corner on screen, a parallelogram.
fn crop_overlay_turned(painter: &egui::Painter, q: [Pos2; 4], shield: Option<Color32>, guides: &[crate::crop_overlay::Polyline]) {
    let (u, v) = (q[1] - q[0], q[3] - q[0]);
    let (lu, lv) = (u.length(), v.length());
    if !(lu.is_finite() && lv.is_finite()) || lu < 1e-3 || lv < 1e-3 {
        return;
    }
    let at = |a: f32, b: f32| q[0] + u * a + v * b;
    // The shield: four trapezoids from the frame out to the frame grown past the whole view.
    let clip = painter.clip_rect();
    let reach = clip.size().length() + (clip.center() - at(0.5, 0.5)).length();
    let (eu, ev) = (reach / lu, reach / lv);
    let outer = [at(-eu, -ev), at(1.0 + eu, -ev), at(1.0 + eu, 1.0 + ev), at(-eu, 1.0 + ev)];
    if let Some(dim) = shield {
        let mut mesh = egui::Mesh::default();
        for p in q.iter().chain(outer.iter()) {
            mesh.colored_vertex(*p, dim);
        }
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            mesh.add_triangle(i, j, 4 + j);
            mesh.add_triangle(i, 4 + j, 4 + i);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    painter.add(egui::Shape::closed_line(q.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    draw_crop_guides(painter, guides, at);
    let h = Stroke::new(3.0, Color32::WHITE);
    let l = 14.0f32.min(lu / 3.0).min(lv / 3.0);
    let (du, dv) = (u / lu, v / lv);
    for (c, su, sv) in [(q[0], 1.0, 1.0), (q[1], -1.0, 1.0), (q[3], 1.0, -1.0), (q[2], -1.0, -1.0)] {
        painter.line_segment([c, c + du * (l * su)], h);
        painter.line_segment([c, c + dv * (l * sv)], h);
    }
    // Edge handles: short bars at the middle of each side, along it.
    let (lx, ly) = (l.min(lu / 4.0) / 2.0, l.min(lv / 4.0) / 2.0);
    for (c, e) in [(at(0.5, 0.0), du * lx), (at(0.5, 1.0), du * lx), (at(0.0, 0.5), dv * ly), (at(1.0, 0.5), dv * ly)] {
        painter.line_segment([c - e, c + e], h);
    }
}

/// The crop overlay's guides, each unit-square point placed on screen by `at`.
fn draw_crop_guides(painter: &egui::Painter, guides: &[crate::crop_overlay::Polyline], at: impl Fn(f32, f32) -> Pos2) {
    let thin = Stroke::new(1.0, Color32::from_white_alpha(90));
    for l in guides {
        painter.add(egui::Shape::line(l.iter().map(|p| at(p[0], p[1])).collect(), thin));
    }
}

/// What the Crop tool shows past the canvas, cached per document state ([`draw_beyond_canvas`]).
#[derive(Clone)]
struct BeyondCanvas {
    snapshot: std::sync::Weak<Document>,
    key: u64,
    /// Every layer's pixels and the canvas (`extra_cmds::reveal_all_bounds`).
    extent: DRect,
    /// The composite around the canvas: each band, the area its texture covers, the texture.
    bands: Vec<(DRect, DRect, egui::TextureHandle)>,
}

/// Band `r` around the canvas `c` (on screen) pushed a point under the canvas's edge, so that the
/// edge's anti-aliased pixels blend with the band rather than with the pasteboard.
fn under_canvas(mut r: Rect, c: Rect) -> Rect {
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    if near(r.max.y, c.min.y) {
        r.max.y += 1.0;
    }
    if near(r.min.y, c.max.y) {
        r.min.y -= 1.0;
    }
    if near(r.max.x, c.min.x) {
        r.max.x += 1.0;
    }
    if near(r.min.x, c.max.x) {
        r.min.x -= 1.0;
    }
    r
}

/// `outer` minus `inner`: the bands above, below, left and right of it (empty ones dropped).
fn ring(outer: DRect, inner: DRect) -> Vec<DRect> {
    let i = inner.intersect(&outer);
    if i.is_empty() {
        return if outer.is_empty() { Vec::new() } else { vec![outer] };
    }
    [
        DRect::new(outer.x0, outer.y0, outer.x1, i.y0),
        DRect::new(outer.x0, i.y1, outer.x1, outer.y1),
        DRect::new(outer.x0, i.y0, i.x0, i.y1),
        DRect::new(i.x1, i.y0, outer.x1, i.y1),
    ]
    .into_iter()
    .filter(|r| !r.is_empty())
    .collect()
}

/// Crop tool, while a frame is edited (`crop_ui::shows_beyond_canvas`): like Photoshop's crop
/// preview, the canvas grows to every layer's pixels (a crop with Delete Cropped Pixels off keeps
/// them; a moved layer reaches past the edges) and to the frame, transparency drawn as the
/// checkerboard, all under the shield `crop_overlay` adds. Only the ring around the canvas is
/// composited, on the CPU, once per document state: at most `MAX_TEXTURE` texels per side, from a
/// downsampled proxy past that (so a far-off layer costs no more than a 16 MP composite).
/// Drawn before the canvas, each band reaching a point under its edge. Returns whether anything
/// past the canvas was drawn.
pub(super) fn draw_beyond_canvas(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, output: Option<u32>) -> bool {
    let ctx = painter.ctx().clone();
    let Some((snapshot, id)) = app.session.documents().get(idx).map(|st| (std::sync::Arc::downgrade(&st.doc), st.doc.id)) else { return false };
    let key = egui::Id::unique(("crop-beyond-canvas", id.0, output));
    let frame =
        app.ui.crop_rect.filter(|f| f.iter().all(|v| v.is_finite()) && app.session.active_index() == Some(idx) && crate::crop_ui::shows_beyond_canvas(app));
    let Some(f) = frame else {
        ctx.data_mut(|d| d.remove::<BeyondCanvas>(key));
        return false;
    };
    let (doc, preview_key) = display_doc(app, idx);
    if doc.has_artboards() {
        return false;
    }
    let (display, display_key) = canvas_display(app, &doc, output);
    let doc_key = preview_key ^ display_key;
    let cached: Option<BeyondCanvas> = ctx.data(|d| d.get_temp(key));
    let c = match cached.filter(|c| c.snapshot.ptr_eq(&snapshot) && c.key == doc_key) {
        Some(c) => c,
        None => {
            let t0 = crate::gpu_canvas::now_ms();
            let extent = photocraft_engine::extra_cmds::reveal_all_bounds(&doc);
            // One texel per `k` document pixels keeps every texture within the device's side.
            let k = extent.width().max(extent.height()).div_ceil(texture_side(&ctx)).max(1);
            let proxy = (k > 1).then(|| photocraft_compose::proxy::proxy_document(&doc, k));
            let src = proxy.as_ref().unwrap_or(&*doc);
            let k = k as i32;
            let opts = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
            let bands = ring(extent, doc.bounds())
                .into_iter()
                .map(|b| {
                    // The proxy pixels over `b`: document pixels / k, rounded outwards.
                    let up = |v: i32| v.div_euclid(k) + i32::from(v.rem_euclid(k) != 0);
                    let p = DRect::new(b.x0.div_euclid(k), b.y0.div_euclid(k), up(b.x1), up(b.y1));
                    let buf = photocraft_compose::render_reduced_rect(src, p, p.width(), p.height());
                    let tex = ctx.load_texture("crop-beyond-canvas", display_image(display.as_deref(), &buf), opts);
                    (b, DRect::new(p.x0.saturating_mul(k), p.y0.saturating_mul(k), p.x1.saturating_mul(k), p.y1.saturating_mul(k)), tex)
                })
                .collect();
            app.perf.span("crop-beyond-canvas", crate::gpu_canvas::now_ms() - t0);
            let c = BeyondCanvas { snapshot, key: doc_key, extent, bands };
            ctx.data_mut(|d| d.insert_temp(key, c.clone()));
            c
        }
    };
    // Transparency out to the frame, in phase with the canvas's own checkerboard.
    let canvas = xf.doc_rect(doc.bounds());
    // A turned frame reaches as far as its corners.
    let f = crate::crop_ui::turned_bounds(f, crate::crop_ui::angle(app));
    let fr = DRect::new(f[0].floor() as i32, f[1].floor() as i32, f[2].ceil() as i32, f[3].ceil() as i32);
    let square = app.session.prefs().transparency_and_gamut.square();
    let checker_id = square.map(|_| checker(app, &ctx));
    let around = ring(c.extent.union(&fr), doc.bounds());
    // A turned view (Rotate View, or the Crop tool's default mode turning the image behind its
    // box) draws each band as its turned quad: its screen bounding box would stretch the band's
    // texture and spread the checkerboard past the band.
    let turned = xf.rotation != 0.0;
    let checker_uv = |p: Pos2, sq: f32| ((p - canvas.min) / (2.0 * sq)).to_pos2();
    for &b in &around {
        if turned {
            let q = doc_quad(xf, b);
            match square.zip(checker_id) {
                Some((sq, id)) => textured_quad(painter, id, q, q.map(|p| checker_uv(p, sq))),
                None => {
                    painter.add(egui::Shape::convex_polygon(q.to_vec(), Color32::WHITE, Stroke::NONE));
                }
            }
            continue;
        }
        let r = under_canvas(xf.doc_rect(b), canvas);
        match square.zip(checker_id) {
            Some((sq, id)) => {
                let uv = Rect::from_min_max(checker_uv(r.min, sq), checker_uv(r.max, sq));
                painter.image(id, r, uv, Color32::WHITE);
            }
            None => {
                painter.rect_filled(r, 0.0, Color32::WHITE);
            }
        }
    }
    for (b, t, tex) in &c.bands {
        let part = |a: i32, o: i32, len: u32| (i64::from(a) - i64::from(o)) as f32 / len.max(1) as f32;
        let (mut u0, mut u1) = (part(b.x0, t.x0, t.width()), part(b.x1, t.x0, t.width()));
        if turned {
            // The quad's corners are the band's own (flip included), so no swap.
            let (v0, v1) = (part(b.y0, t.y0, t.height()), part(b.y1, t.y0, t.height()));
            textured_quad(painter, tex.id(), doc_quad(xf, *b), [pos2(u0, v0), pos2(u1, v0), pos2(u1, v1), pos2(u0, v1)]);
            continue;
        }
        if xf.flip {
            (u0, u1) = (u1, u0);
        }
        let uv = Rect::from_min_max(pos2(u0, part(b.y0, t.y0, t.height())), pos2(u1, part(b.y1, t.y0, t.height())));
        // The texture's edge texels stretch over the point under the canvas.
        let (r, e) = (xf.doc_rect(*b), under_canvas(xf.doc_rect(*b), canvas));
        let at = |p: Pos2| uv.min + (p - r.min) / r.size() * uv.size();
        painter.image(tex.id(), e, Rect::from_min_max(at(e.min), at(e.max)), Color32::WHITE);
    }
    !around.is_empty()
}

/// Document rect `b`'s corners on screen, clockwise from its top-left (a parallelogram in a turned
/// view).
fn doc_quad(xf: &ViewXform, b: DRect) -> [Pos2; 4] {
    [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)].map(|(x, y)| xf.to_screen(x as f32, y as f32))
}

/// Texture `tex` drawn over the quad `q`, `uv` at its corners.
fn textured_quad(painter: &egui::Painter, tex: egui::TextureId, q: [Pos2; 4], uv: [Pos2; 4]) {
    if !q.iter().chain(uv.iter()).all(|p| p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let mut mesh = egui::Mesh::with_texture(tex);
    for (pos, uv) in q.into_iter().zip(uv) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Overlays that persist between gestures: polygonal or magnetic lasso in progress, pending crop box.
pub(super) fn draw_tool_state(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, hover: Option<Pos2>) {
    crate::magnetic_lasso_ui::draw(app, painter, xf, hover);
    if !app.ui.polygon.is_empty() {
        let mut pts: Vec<Pos2> = app.ui.polygon.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        if let Some(h) = hover {
            // The rubber-band preview must use the same constrained point as the click,
            // including when the canvas is zoomed, rotated or flipped.
            let mods = painter.ctx().input(|i| i.modifiers);
            if app.ui.tool == Tool::PolygonLasso && mods.shift && !mods.alt {
                let raw = xf.to_doc(h);
                let last = app.ui.polygon.last().copied().unwrap_or(raw);
                let snapped = constrained_polygon_point(last, raw);
                pts.push(xf.to_screen(snapped[0] as f32, snapped[1] as f32));
            } else {
                pts.push(h);
            }
        }
        // Just the outline and its rubber band (the vertices aren't handles to grab), solid until
        // the selection is made (#7495).
        crate::tool_feedback::draw_solid_path(painter, &pts, false);
    }
    crate::crop_straighten::draw(app, painter, |p| xf.to_screen(p[0] as f32, p[1] as f32), hover);
    crate::blur_pins::draw(app, painter, xf);
    if let Some(c) = app.ui.crop_rect {
        let deg = crate::crop_ui::angle(app);
        let shield = crate::crop_shield::current(app, painter.ctx());
        if deg == 0.0 && xf.rotation == 0.0 {
            let r = Rect::from_two_pos(xf.to_screen(c[0] as f32, c[1] as f32), xf.to_screen(c[2] as f32, c[3] as f32));
            crop_overlay(painter, r, shield, &crate::crop_overlay::current_guides(app, r.width(), r.height()));
        } else {
            let q = crate::crop_ui::corners(c, deg).map(|p| xf.to_screen(p[0] as f32, p[1] as f32));
            crop_overlay_turned(painter, q, shield, &crate::crop_overlay::current_guides(app, q[0].distance(q[1]), q[0].distance(q[3])));
        }
        // The frame's angle beside the pointer while it turns (#1792).
        if let (Some(a), Some(h)) = (crate::crop_ui::turning(app), hover) {
            draw_value_readout(app, painter.ctx(), "crop-angle-readout", h, ["Angle:"], [angle_readout(a)]);
        } else if let (Some([w, h]), Some(at)) = (crate::crop_ui::sizing(app), hover) {
            // Its W × H while it is drawn or resized (#1919), like the marquee's.
            draw_marquee_readout(app, painter.ctx(), at, size_readout(app, [0.0, 0.0, w, h]));
        }
    }
}

pub(super) fn draw_drag_preview(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    draw_tool_state(app, painter, xf, painter.ctx().input(|i| i.pointer.hover_pos()));
    // Remove strokes waiting for ↩ stay tinted between drags.
    crate::retouch_ui::draw_remove_pending(app, painter, xf);
    let Some(d) = &app.drag else {
        app.trail = None;
        return;
    };
    // The Move tool's ΔX/ΔY beside the pointer, for layers and selected pixels alike (#5727).
    if let Some(values) = crate::move_ui::offset_readout(app) {
        let at = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
        draw_value_readout(app, painter.ctx(), "move-readout", xf.to_screen(at[0] as f32, at[1] as f32), ["ΔX:", "ΔY:"], values);
    }
    // Dragging the selection: its own ants follow the pointer (`selection_shown_offset`).
    if d.sel_move.is_some() {
        return;
    }
    let last = d.points.last().map(|p| [p[0], p[1]]).unwrap_or(d.start);
    let marquee = marquee_preview_px(&app.ui.tool_options, d, app.point_zoom());
    if let Some(r) = marquee {
        draw_marquee_readout(app, painter.ctx(), xf.to_screen(last[0] as f32, last[1] as f32), size_readout(app, r));
    }
    match d.tool {
        // The canvas shows the live stroke itself (`LiveStroke`); a stroke that couldn't start one
        // (Sample All Layers, say) keeps the trail.
        t if strokes_live(t) && app.live_stroke.is_some() => {
            // Smoothing holds the brush back from the pointer: draw the string between them.
            if let Some(EngineStroke::Brush(l)) = app.live_stroke.as_ref().map(|l| &l.stroke)
                && let Some(s) = l.smoothing_string()
            {
                draw_smoothing_string(painter, xf, s);
            }
        }
        t if t.is_brushlike() || t == Tool::QuickSelection => {
            // Retouching strokes preview as a translucent trail of the brush footprint: a mask,
            // not a brush-wide egui polyline (which zoomed in tessellates into wedges, #189).
            let col = match t {
                Tool::QuickSelection => Color32::from_white_alpha(40),
                // The Remove Tool tints what it will remove, like Photoshop's.
                Tool::Remove => crate::theme::Tokens::get(painter.ctx()).danger.gamma_multiply(0.45),
                // Photoshop paints the area to heal in half-opaque dark grey while dragging; a faint
                // white trail barely showed on light pixels (#7342). The heal itself is computed on
                // release.
                Tool::SpotHealing => SPOT_HEALING_TRAIL,
                _ => Color32::from_white_alpha(60),
            };
            let Some(st) = app.session.active() else { return };
            let size = [st.doc.size.width, st.doc.size.height];
            let doc_rect = xf.doc_rect(st.doc.bounds());
            let trail = app.trail.get_or_insert_with(|| crate::stroke_trail::Trail::new(size));
            if t == Tool::SpotHealing {
                // What the heal covers, dab by dab: soft edges, pressure and shape dynamics (#2364).
                trail.feed_dabs(&d.points, || photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({})));
            } else {
                trail.feed(&d.points, app.session.tools.brush.size);
            }
            trail.draw(painter, doc_rect, xf.flip, col);
        }
        t if crate::vector_ui::is_shape_tool(t) => crate::vector_ui::draw_shape_preview(app, painter, xf, t, d.start, last, d.live),
        Tool::SingleRowMarquee | Tool::SingleColumnMarquee => {
            // Mapped corner by corner like the other marquees, so a rotated view shows the line
            // where the release commits it (#2512).
            if let Some([x0, y0, x1, y1]) = single_line_drag_px(app, d) {
                crate::tool_feedback::draw_ants(painter, &marquee_ants(xf, d.tool, [x0, y0], [x1, y1]), true);
            }
        }
        // Nothing selected yet: no outline, as the release would deselect.
        Tool::RectMarquee | Tool::EllipseMarquee if marquee.is_none() => {}
        Tool::RectMarquee | Tool::EllipseMarquee | Tool::ObjectSelection => {
            // Marching ants, visible on any pixels (#172): built in document space and mapped
            // corner by corner, so a rotated view shows them where the release commits (#2512).
            let pts = marquee_drag_ants(d, xf, marquee);
            crate::tool_feedback::draw_ants(painter, &pts, true);
        }
        // Patch / Content-Aware Move dragging the selection: its outline follows the pointer.
        Tool::Patch | Tool::ContentAwareMove if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => {
            let start = d.start;
            let [dx, dy] = crate::retouch_ui::patch_offset(app, start, last);
            if let Some((_, _, segs)) = &app.outline_cache {
                let moved: Vec<crate::outline::Segment> = segs.iter().map(|(a, b)| ([a[0] + dx, a[1] + dy], [b[0] + dx, b[1] + dy])).collect();
                marching_ants_segments(painter, xf, &moved, painter.ctx().input(|i| i.time));
            }
        }
        Tool::FreeformPen => {
            let mut pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            // Between ⌥-clicks: the straight segment the next click makes.
            if let Some(lasso) = &d.lasso {
                pts.push(xf.to_screen(lasso.cursor[0] as f32, lasso.cursor[1] as f32));
            }
            painter.add(egui::Shape::line(pts, Stroke::new(1.5, crate::theme::Tokens::get(painter.ctx()).accent)));
        }
        Tool::Lasso | Tool::Patch | Tool::ContentAwareMove => {
            let mut pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            if let Some(lasso) = &d.lasso {
                pts.push(xf.to_screen(lasso.cursor[0] as f32, lasso.cursor[1] as f32));
            }
            // The Lasso's outline is solid until the release makes the selection (#7495); a Patch
            // or Content-Aware Move area keeps its ants.
            if d.tool == Tool::Lasso {
                // On pixel centres, so a 1 pt line stays crisp wherever the canvas sits (a pixel
                // edge would blur it to grey).
                let pts: Vec<Pos2> = pts.into_iter().map(|p| p.round() + vec2(0.5, 0.5)).collect();
                crate::tool_feedback::draw_solid_path(painter, &pts, false);
            } else {
                crate::tool_feedback::draw_ants(painter, &pts, false);
            }
        }
        Tool::Gradient => {
            let a = xf.to_screen(d.start[0] as f32, d.start[1] as f32);
            let b = xf.to_screen(last[0] as f32, last[1] as f32);
            painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
            painter.circle_filled(a, 3.0, Color32::WHITE);
            painter.circle_filled(b, 3.0, Color32::WHITE);
        }
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => {
            let r = Rect::from_two_pos(xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32));
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::BLACK), 3.0, 3.0));
        }
        // The layers themselves follow the pointer (`move_ui`); the arrow only when they can't.
        Tool::Move if !crate::move_ui::showing(app) => {
            let from = xf.to_screen(d.start[0] as f32, d.start[1] as f32);
            let off = xf.to_screen(last[0] as f32, last[1] as f32) - from;
            painter.arrow(from, off, Stroke::new(2.0, crate::theme::Tokens::get(painter.ctx()).accent));
        }
        _ => {}
    }
}
