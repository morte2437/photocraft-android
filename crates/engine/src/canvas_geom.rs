//! Canvas-wide geometry: Image › Image Rotation (90°, 180°, flips, arbitrary), Canvas Size, Crop,
//! Trim and Image Size move everything in the document, not only pixels. Pixels are remapped by
//! the callers; this module moves the rest: type, shape and smart-object transforms, vector masks,
//! gradient fill angles, effect reference points, artboards, and document-level paths, guides,
//! slices, notes and measurement marks. Layer effect angles are left alone (Photoshop keeps the
//! Global Light when the canvas rotates).

use std::sync::Arc;

use photocraft_doc::{Document, Fill, Layer, LayerContent, TextLayer};
use photocraft_geom::{Affine, Point, Rect};
use photocraft_raster::Surface;

use crate::pixels::remap_surface;

const EPS: f64 = 1e-9;

fn map_pt(a: &Affine, p: [f64; 2]) -> [f64; 2] {
    let q = a.apply(Point::new(p[0], p[1]));
    [q.x, q.y]
}

/// Bounding box `[x0, y0, x1, y1]` of the four mapped corners of `[x0, y0, x1, y1]`.
fn map_box(a: &Affine, b: [f64; 4]) -> [f64; 4] {
    let c = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| map_pt(a, p));
    c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |r, p| [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[0]), r[3].max(p[1])])
}

fn map_rect(a: &Affine, r: Rect) -> Rect {
    if r.is_empty() {
        return r;
    }
    let b = map_box(a, [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    let i = |v: f64| v.round().clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32;
    Rect::new(i(b[0]), i(b[1]), i(b[2]), i(b[3]))
}

/// A gradient angle (degrees, counter-clockwise on screen) carried through `a`'s linear part.
fn map_angle(a: &Affine, deg: f32) -> f32 {
    let (s, c) = f64::from(deg).to_radians().sin_cos();
    // Direction in y-down document space.
    let (dx, dy) = (c, -s);
    let (nx, ny) = (a.m[0] * dx + a.m[2] * dy, a.m[1] * dx + a.m[3] * dy);
    if nx.abs() < EPS && ny.abs() < EPS {
        return deg;
    }
    let out = (-ny).atan2(nx).to_degrees();
    // Keep whole-degree angles whole (90° turns and flips are exact up to rounding noise).
    let out = if (out - out.round()).abs() < 1e-6 { out.round() } else { out };
    // (−180°, 180°]: −0 from atan2 would otherwise give −180°.
    (if out <= -180.0 { out + 360.0 } else { out }) as f32
}

fn map_fill(a: &Affine, f: &mut Fill) {
    if let Fill::Gradient { angle, offset, .. } = f {
        *angle = map_angle(a, *angle);
        // The centre offset (a fraction of the frame) turns with the canvas: exact for 90° turns
        // and flips, where the frame's sides swap with the axes. Scale (Image Size) resizes the
        // frame too, so only each axis's direction applies.
        let unit = |x: f64, y: f64| {
            let n = x.hypot(y);
            if n < EPS { (x, y) } else { (x / n, y / n) }
        };
        let ((m0, m1), (m2, m3)) = (unit(a.m[0], a.m[1]), unit(a.m[2], a.m[3]));
        let (ox, oy) = (f64::from(offset.0), f64::from(offset.1));
        *offset = ((m0 * ox + m2 * oy) as f32, (m1 * ox + m3 * oy) as f32);
    }
}

/// Rebuilds a type layer's PSD `TySh` block for its current transform, keeping its pixels.
fn retag_text(dpi: f32, t: &mut TextLayer) {
    if t.psd_raw.is_none() {
        return;
    }
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let layout = eng.layout(t, dpi);
    t.psd_raw = Some(Arc::new(photocraft_text::psd::build_tysh(t, dpi, layout.bounds())));
}

/// Sets the document resolution keeping every type layer's size in pixels, as Photoshop does (its
/// point size changes instead). Text space is in pixels at `pt x dpi / 72`, so it grows by
/// new / old: box and path scale with it, and the layer transform undoes that.
pub(crate) fn set_resolution(doc: &mut Document, dpi: f32) {
    let old = std::mem::replace(&mut doc.resolution_dpi, dpi);
    if !(old.is_finite() && old > 0.0 && dpi.is_finite() && dpi > 0.0) || old == dpi {
        return;
    }
    let k = f64::from(dpi) / f64::from(old);
    let grow = Affine::scale(k);
    crate::image_cmds::for_each_layer(&mut doc.layers, &mut |l| {
        let LayerContent::Text(t) = &mut l.content else { return };
        t.transform = t.transform.mul(&Affine::scale(1.0 / k));
        match &mut t.shape {
            photocraft_doc::text::TextShape::Box { x, y, width, height } => {
                for v in [x, y, width, height] {
                    *v = grown(*v, k);
                }
            }
            photocraft_doc::text::TextShape::Path { path, start, .. } => {
                path.knots.iter_mut().for_each(|kn| *kn = kn.transform(&grow));
                *start = grown(*start, k);
            }
            photocraft_doc::text::TextShape::Area { path } => *path = path.transform(&grow),
            photocraft_doc::text::TextShape::Point => {}
        }
        retag_text(dpi, t);
    });
}

/// Moves the type of `doc` (scratch copies on their way to another document) to `dpi` keeping its
/// pixels, as the reference application does when a type layer is copied between documents of
/// different resolutions: every size in points is rewritten by old / new (24 pt at 72 ppi is
/// 5.76 pt at 300 ppi), so `pt x dpi / 72` pixels stay the same and the layer gets no
/// compensating transform. Box, path and transform are in pixels and stay; tracking and kerning
/// (1/1000 em) and auto leading (a multiple of the size) follow the size.
pub(crate) fn retarget_type_resolution(doc: &mut Document, dpi: f32) {
    // The text engine lays out a resolution that isn't a positive number at 72 ppi.
    let effective = |d: f32| if d.is_finite() && d > 0.0 { f64::from(d) } else { 72.0 };
    let old = std::mem::replace(&mut doc.resolution_dpi, dpi);
    let k = effective(old) / effective(dpi);
    if !(k.is_finite() && k > 0.0) || k == 1.0 {
        return;
    }
    crate::image_cmds::for_each_layer(&mut doc.layers, &mut |l| {
        let LayerContent::Text(t) = &mut l.content else { return };
        t.size_pt = point_size(t.size_pt, k);
        for r in &mut t.runs {
            let st = &mut r.style;
            st.size_pt = point_size(st.size_pt, k);
            st.leading_pt = st.leading_pt.map(|v| point_size(v, k));
            st.baseline_shift_pt = grown(st.baseline_shift_pt, k);
        }
        for p in &mut t.paragraphs {
            let st = &mut p.style;
            for v in [&mut st.first_line_indent_pt, &mut st.start_indent_pt, &mut st.end_indent_pt, &mut st.space_before_pt, &mut st.space_after_pt] {
                *v = grown(*v, k);
            }
        }
        retag_text(dpi, t);
    });
}

/// A positive size in points times `k`, kept finite and above zero (a tiny ratio must not make
/// the text vanish).
fn point_size(v: f32, k: f64) -> f32 {
    let out = grown(v, k);
    if v > 0.0 && out <= 0.0 { f32::MIN_POSITIVE } else { out }
}

/// `v x k` kept finite (a huge box from a file must not become infinite).
fn grown(v: f32, k: f64) -> f32 {
    (f64::from(v) * k).clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
}

/// Applies `a` to one layer's (and its children's) non-pixel geometry. With `content` false only
/// the parts Free Transform (`transform_layer`) doesn't move are touched: unlinked vector masks,
/// gradient fill angles, effect reference points and artboards.
pub(crate) fn transform_layer_geometry(l: &mut Layer, a: &Affine, content: bool, dpi: f32) {
    if let Some(vm) = l.vector_mask.as_mut()
        && (content || !vm.linked)
    {
        vm.path = vm.path.transform(a);
    }
    if let Some((x, y)) = l.effects.reference {
        let p = map_pt(a, [x, y]);
        l.effects.reference = Some((p[0], p[1]));
    }
    if !content {
        // Free Transform doesn't warp fill caches: drop it, the fill re-renders from its model.
        l.fill_cache = None;
    } else if let Some(fc) = &mut l.fill_cache {
        map_fill(a, &mut fc.fill);
    }
    match &mut l.content {
        LayerContent::Fill(f) => map_fill(a, f),
        LayerContent::Shape(sh) => {
            if let Some(f) = &mut sh.fill {
                map_fill(a, f);
            }
            if let Some(st) = &mut sh.stroke {
                map_fill(a, &mut st.paint);
            }
            if content {
                crate::vector_cmds::transform_shape(sh, a);
            }
        }
        LayerContent::Text(t) if content => {
            t.transform = a.mul(&t.transform);
            retag_text(dpi, t);
        }
        LayerContent::Smart(sm) if content => crate::smart_cmds::transform_placement(sm, a),
        LayerContent::Group(g) => {
            if let Some(ab) = &mut g.artboard {
                ab.rect = map_rect(a, ab.rect);
            }
            for c in &mut g.children {
                transform_layer_geometry(c, a, content, dpi);
            }
        }
        _ => {}
    }
}

/// Document-level geometry: paths, notes, Count/Ruler marks, slices and guides. Guides survive
/// axis-aligned maps (and swap orientation under 90° turns); any other map clears them.
pub(crate) fn transform_doc_marks(doc: &mut Document, a: &Affine) {
    for p in &mut doc.paths {
        p.path = p.path.transform(a);
    }
    if let Some(wp) = &mut doc.work_path {
        *wp = wp.transform(a);
    }
    for n in &mut doc.notes {
        n.position = map_pt(a, n.position);
        n.popup = map_box(a, n.popup);
    }
    for g in &mut doc.measurement.count_groups {
        for p in &mut g.points {
            *p = map_pt(a, *p);
        }
    }
    if let Some(r) = &mut doc.measurement.ruler {
        r.start = map_pt(a, r.start);
        r.end = map_pt(a, r.end);
        if let Some(p) = &mut r.protractor {
            *p = map_pt(a, *p);
        }
    }
    for s in &mut doc.slices.list {
        s.rect = map_rect(a, s.rect);
    }
    let [m0, m1, m2, m3, tx, ty] = a.m;
    let g = &mut doc.guides;
    if m1.abs() < EPS && m2.abs() < EPS {
        g.horizontal.iter_mut().for_each(|y| *y = m3 * *y + ty);
        g.vertical.iter_mut().for_each(|x| *x = m0 * *x + tx);
        // Colours are keyed by exact position, so they move by the same expressions.
        for c in &mut g.colors {
            c.position = if c.vertical { m0 * c.position + tx } else { m3 * c.position + ty };
        }
    } else if m0.abs() < EPS && m3.abs() < EPS {
        // A horizontal guide at y becomes a vertical one at x' = m2·y + tx, and vice versa.
        let v: Vec<f64> = g.horizontal.iter().map(|y| m2 * y + tx).collect();
        let h: Vec<f64> = g.vertical.iter().map(|x| m1 * x + ty).collect();
        g.horizontal = h;
        g.vertical = v;
        for c in &mut g.colors {
            c.position = if c.vertical { m1 * c.position + ty } else { m2 * c.position + tx };
            c.vertical = !c.vertical;
        }
    } else {
        g.horizontal.clear();
        g.vertical.clear();
        g.colors.clear();
    }
}

/// Applies `a` to all non-pixel geometry of the document (layers and document marks).
pub(crate) fn transform_geometry(doc: &mut Document, a: &Affine) {
    let dpi = doc.resolution_dpi;
    for l in &mut doc.layers {
        transform_layer_geometry(l, a, true, dpi);
    }
    transform_doc_marks(doc, a);
}

/// What to re-render after the geometry moved.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// Shapes only (their pixels are clipped to the canvas, which may have grown).
    Shapes,
    /// Type, shapes and smart objects (after a resampling map). Smart objects whose source is
    /// unavailable keep their (already mapped) cache.
    All,
}

pub(crate) fn refresh(doc: &mut Document, what: Refresh) {
    fn rec(snapshot: &Document, layers: &mut [Layer], what: Refresh) {
        for l in layers {
            match &mut l.content {
                LayerContent::Group(g) => rec(snapshot, &mut g.children, what),
                LayerContent::Shape(sh) => crate::vector_cmds::refresh_shape(snapshot, sh),
                LayerContent::Text(t) if what == Refresh::All => crate::type_cmds::refresh(snapshot, t),
                LayerContent::Smart(_) if what == Refresh::All => {
                    let _ = crate::smart_cmds::refresh_layer(snapshot, l);
                }
                _ => {}
            }
        }
    }
    let snapshot = doc.clone();
    rec(&snapshot, &mut doc.layers, what);
}

/// Image › Image Rotation by a right angle or a flip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    FlipHorizontal,
    FlipVertical,
    Rotate180,
    Cw90,
    Ccw90,
}

impl Turn {
    /// Pixel map `(x, y) → (x', y')` (pixel indices) for a `w × h` canvas.
    pub(crate) fn pixel_map(self, w: i32, h: i32) -> impl Fn(i32, i32) -> (i32, i32) {
        move |x, y| match self {
            Turn::FlipHorizontal => (w - 1 - x, y),
            Turn::FlipVertical => (x, h - 1 - y),
            Turn::Rotate180 => (w - 1 - x, h - 1 - y),
            Turn::Cw90 => (h - 1 - y, x),
            Turn::Ccw90 => (y, w - 1 - x),
        }
    }
    /// The same map in continuous (pixel-edge) coordinates.
    pub(crate) fn affine(self, w: f64, h: f64) -> Affine {
        Affine {
            m: match self {
                Turn::FlipHorizontal => [-1.0, 0.0, 0.0, 1.0, w, 0.0],
                Turn::FlipVertical => [1.0, 0.0, 0.0, -1.0, 0.0, h],
                Turn::Rotate180 => [-1.0, 0.0, 0.0, -1.0, w, h],
                Turn::Cw90 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
                Turn::Ccw90 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
            },
        }
    }
    fn swaps(self) -> bool {
        matches!(self, Turn::Cw90 | Turn::Ccw90)
    }
}

/// Every pixel surface a canvas turn moves: content, rendered caches (type, shape, smart object,
/// fill), stored video frames (current and original), masks (layer, smart filter), alpha
/// channels, Quick Mask and the selection.
fn remap_pixels(doc: &mut Document, map: &dyn Fn(i32, i32) -> (i32, i32)) {
    fn rec(layers: &mut [Layer], map: &dyn Fn(i32, i32) -> (i32, i32)) {
        let re = |s: &mut Surface| *s = remap_surface(s, map);
        for l in layers {
            if let Some(m) = &mut l.mask {
                re(&mut m.surface);
            }
            if let Some(fc) = &mut l.fill_cache {
                re(&mut fc.surface);
            }
            // Frames away from the playhead come back on scrubbing; they turn with the canvas (#4956).
            if let Some(v) = &mut l.video {
                v.frames.iter_mut().chain(v.original_frames.iter_mut().flatten()).for_each(re);
            }
            match &mut l.content {
                LayerContent::Raster(s) => re(s),
                LayerContent::Text(t) => t.cache.iter_mut().for_each(re),
                LayerContent::Shape(sh) => sh.cache.iter_mut().for_each(re),
                LayerContent::Smart(sm) => {
                    sm.cache.iter_mut().for_each(re);
                    if let Some(m) = &mut sm.filter_mask {
                        re(&mut m.surface);
                    }
                }
                LayerContent::Group(g) => rec(&mut g.children, map),
                _ => {}
            }
        }
    }
    rec(&mut doc.layers, map);
    for ch in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        ch.surface = remap_surface(&ch.surface, map);
    }
    if let Some(sel) = &doc.selection {
        doc.selection = Some(remap_surface(sel, map));
    }
}

/// Turns or flips the whole canvas. Pixels (and rendered caches) are remapped exactly, so nothing
/// is re-rendered; vector geometry follows through the matching affine map.
pub(crate) fn turn_canvas(doc: &mut Document, turn: Turn) {
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    let map = turn.pixel_map(w, h);
    remap_pixels(doc, &map);
    let a = turn.affine(f64::from(doc.size.width), f64::from(doc.size.height));
    if turn.swaps() {
        doc.size = photocraft_doc::Size::new(doc.size.height, doc.size.width);
    }
    transform_geometry(doc, &a);
}

#[cfg(test)]
mod tests;
