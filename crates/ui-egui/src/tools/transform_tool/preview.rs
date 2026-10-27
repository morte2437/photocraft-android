use super::*;

/// Texels of the moving pixels over `b`: full resolution unless the long side exceeds the GPU's
/// `max_side` (#91: this used to be capped at 2048 px, so big layers previewed blurry), plus the
/// uv extent covering `b`. `bare` leaves the layer's own style, opacity and fill out of a
/// flattened render: the styled preview applies them to the moved pixels itself.
pub(super) fn preview_image(
    doc: &Document,
    id: LayerId,
    lifted: Option<&photocraft_raster::Surface>,
    b: photocraft_geom::Rect,
    max_side: usize,
    bare: bool,
) -> (egui::ColorImage, [f32; 2]) {
    let layer = doc.layer(id);
    // Linked masks move with the pixels, so the moving texels carry them (#205).
    let mask = layer.and_then(|l| preview_mask(l, b));
    // Groups and type/fill layers preview from a flattened render of just that layer.
    let rendered;
    let surf: Option<&photocraft_raster::Surface> = match (lifted, layer.and_then(|l| l.surface())) {
        (Some(s), _) => Some(s),
        (None, Some(s)) if layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_))) => Some(s),
        _ => {
            let mut solo = doc.clone();
            if layer.is_some() {
                solo_layer(&mut solo.layers, id);
            } else {
                // Keep the previous fallback for an invalid id: render the document, with only
                // top-level clipping disabled. Production callers get ids from this document.
                for l in &mut solo.layers {
                    l.clipped = false;
                }
            }
            if let Some(l) = solo.layer_mut(id).filter(|_| bare) {
                l.effects.enabled = false;
                l.opacity = 1.0;
                l.fill_opacity = 1.0;
            }
            let buf = photocraft_compose::render(&solo, b);
            let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
            let flat: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
            s.write_region(b, &flat);
            rendered = s;
            Some(&rendered)
        }
    };
    let Some(surf) = surf else { return (egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]) };
    // A flattened render already has the masks applied.
    let raw = lifted.is_some() || layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_)));
    crate::transform_tex::read_surface(surf, mask.as_ref().filter(|_| raw), b, max_side)
}

/// The layer's masks as the moving texels carry them: the enabled masks when all of them are linked
/// (the vector mask and feathering folded in as the compositor applies them), else the linked
/// pixel mask alone. `None` when no enabled mask moves with the pixels.
fn preview_mask(l: &photocraft_doc::Layer, b: photocraft_geom::Rect) -> Option<crate::transform_tex::PreviewMask> {
    use crate::transform_tex::PreviewMask;
    let pixel = l.mask.as_ref().filter(|m| m.enabled);
    let vector = l.vector_mask.as_ref().filter(|v| v.enabled);
    let linked_pixel = pixel.filter(|m| m.linked);
    if pixel.is_none_or(|m| m.linked)
        && vector.is_none_or(|v| v.linked)
        && let Some(surface) = photocraft_compose::masks::combined_mask(l, b)
    {
        return Some(PreviewMask { surface, density: 1.0 });
    }
    linked_pixel.map(|m| PreviewMask { surface: m.surface.clone(), density: m.density })
}

/// The layers covering the moving one ([`covering`]), composited on their own and drawn over its
/// preview, so the layer keeps its place in the stack while the box is open (#2439): they cover
/// it, as in Photoshop, instead of the moving pixels covering the whole document. The canvas
/// under the box leaves them out ([`Above::hide_in`]). Built when the session starts and again
/// when an edit changes the document; the box itself moves without re-compositing.
pub(super) struct Above {
    pub(super) ctx: egui::Context,
    pub(super) name: String,
    pub(super) max_side: usize,
    /// The covering layers (whole layers or groups, at any depth).
    pub(super) ids: Vec<LayerId>,
    /// Their composite and the canvas area it covers; `None` when nothing visible covers the layer.
    pub(super) shown: Option<(crate::transform_tex::PreviewTextures, photocraft_geom::Rect)>,
}

impl Above {
    pub(super) fn new(ctx: &egui::Context, name: String, doc: &Document, id: LayerId, max_side: usize) -> Self {
        let mut a = Self { ctx: ctx.clone(), name, max_side, ids: Vec::new(), shown: None };
        a.rebuild(doc, id);
        a
    }

    /// Composite what covers layer `id` of `doc` (the live document) again.
    pub(super) fn rebuild(&mut self, doc: &Document, id: LayerId) {
        self.ids.clear();
        covering(&doc.layers, id, &mut self.ids);
        let mut up = doc.clone();
        show_only(&mut up.layers, &self.ids);
        let canvas = up.bounds();
        let rect = up
            .layers
            .iter()
            .filter(|l| l.visible)
            .fold(photocraft_geom::Rect::EMPTY, |r, l| r.union(&photocraft_compose::composite_bounds(l, canvas).unwrap_or(canvas)))
            .intersect(&canvas);
        self.shown = (!rect.is_empty()).then(|| {
            let (image, uv) = composite_image(&up, rect, self.max_side);
            (crate::transform_tex::PreviewTextures::new(&self.ctx, self.name.clone(), image, uv), rect)
        });
    }

    /// Leave the covering layers out of `layers` (the document under the box).
    pub(super) fn hide_in(&self, layers: &mut [photocraft_doc::Layer]) {
        for l in layers.iter_mut() {
            if self.ids.contains(&l.id) {
                l.visible = false;
            } else if let LayerContent::Group(g) = &mut l.content {
                self.hide_in(&mut g.children);
            }
        }
    }

    /// Draw the composite over the moving pixels.
    pub(super) fn draw(&self, painter: &egui::Painter, xf: &ViewXform, display: Option<&photocraft_engine::display_color::CanvasDisplay>) {
        let Some((texture, r)) = &self.shown else { return };
        let (tex, uv) = texture.pick_display(painter.ctx(), xf.zoom * painter.ctx().pixels_per_point(), 1.0, false, display);
        let mut mesh = egui::Mesh::with_texture(tex);
        let corners = [(r.x0, r.y0, 0.0, 0.0), (r.x1, r.y0, uv[0], 0.0), (r.x1, r.y1, uv[0], uv[1]), (r.x0, r.y1, 0.0, uv[1])];
        for (x, y, u, v) in corners {
            mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(x as f32, y as f32), uv: pos2(u, v), color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }
}

/// `rect` of `doc`'s composite as premultiplied texels: full resolution when it fits in
/// `max_side`, composited in bands (no full-size float copy), else area-averaged down to fit.
fn composite_image(doc: &Document, rect: photocraft_geom::Rect, max_side: usize) -> (egui::ColorImage, [f32; 2]) {
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let to_color = |p: &[u8; 4]| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
    if w.max(h) <= max_side.max(1) {
        let mut px = Vec::with_capacity(w * h);
        let _ = photocraft_compose::render_bands(doc, rect, 0, |band| {
            px.extend(band.to_rgba8().pixels.as_chunks::<4>().0.iter().map(to_color));
            Ok::<(), ()>(())
        });
        px.resize(w * h, Color32::TRANSPARENT);
        return (egui::ColorImage::new([w, h], px), [1.0, 1.0]);
    }
    let k = w.max(h).div_ceil(max_side.max(1));
    let (tw, th) = (w.div_ceil(k).max(1), h.div_ceil(k).max(1));
    let buf = photocraft_compose::render_reduced_rect(doc, rect, tw as u32, th as u32);
    let px: Vec<Color32> = buf.to_rgba8().pixels.as_chunks::<4>().0.iter().map(to_color).collect();
    let [bw, bh] = [buf.rect.width() as usize, buf.rect.height() as usize];
    if px.len() != bw * bh || bw == 0 || bh == 0 {
        return (egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]);
    }
    (egui::ColorImage::new([bw, bh], px), [1.0, 1.0])
}

/// A layer that composites over anything the same as over transparency, so it can be drawn over
/// the moving pixels on its own: a Normal layer that isn't an adjustment, or a Normal or Pass
/// Through group of such layers (hidden layers count, they draw nothing). Adjustment layers and
/// the other blend modes depend on what is under them.
fn plain(l: &photocraft_doc::Layer) -> bool {
    use photocraft_color::BlendMode;
    !l.visible
        || match &l.content {
            LayerContent::Adjustment(_) => false,
            LayerContent::Group(g) => matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough) && g.children.iter().all(plain),
            _ => l.blend == BlendMode::Normal,
        }
}

/// The layers over `id` in `layers` (bottom to top, groups holding `id` searched too) that cover
/// it in the preview: from the top down, while they are [`plain`]. The first layer that isn't
/// stops the search, so it and everything under it stay in the document under the box as before
/// (an adjustment there keeps applying to the layers under it). A clipping group goes as a whole,
/// and the layers clipped to `id` (or to the group holding it) stay with it.
pub(super) fn covering(layers: &[photocraft_doc::Layer], id: LayerId, out: &mut Vec<LayerId>) {
    let mut end = layers.len();
    while end > 0 {
        // The clipping group ending at `end`: its base and the layers clipped to it.
        let mut start = end - 1;
        while start > 0 && layers.get(start).is_some_and(|l| l.clipped) {
            start -= 1;
        }
        let unit = layers.get(start..end).unwrap_or_default();
        if unit.iter().any(|l| contains(l, id)) {
            // A group holding `id` with nothing clipped to it: go on inside it.
            if let [g] = unit
                && g.id != id
                && let LayerContent::Group(group) = &g.content
                && matches!(g.blend, photocraft_color::BlendMode::Normal | photocraft_color::BlendMode::PassThrough)
            {
                covering(&group.children, id, out);
            }
            return;
        }
        if !unit.iter().all(plain) {
            return;
        }
        out.extend(unit.iter().map(|l| l.id));
        end = start;
    }
}

/// Hide everything in `layers` but the `keep` layers and the groups holding them.
pub(super) fn show_only(layers: &mut [photocraft_doc::Layer], keep: &[LayerId]) {
    for l in layers.iter_mut() {
        if keep.contains(&l.id) {
            continue;
        }
        match &mut l.content {
            LayerContent::Group(g) if g.children.iter().any(|c| keep.iter().any(|k| contains(c, *k))) => {
                // This ancestor only clips the covering children. Its background is already
                // on the base canvas; drawing it again would cover the moving layer in white.
                if let Some(board) = &mut g.artboard {
                    board.background = photocraft_doc::ArtboardBackground::Transparent;
                }
                show_only(&mut g.children, keep);
            }
            _ => l.visible = false,
        }
    }
}

/// Leave only `id`, its descendants and the groups holding it visible. Every layer on that path
/// is detached from its clipping base because the base is hidden in this isolated render (#5239).
/// Hide every layer but `id` and the groups holding it, and unclip those, so a render shows that
/// layer alone (the Free Transform preview, Liquify's one-layer backdrop).
pub(crate) fn solo_layer(layers: &mut [photocraft_doc::Layer], id: LayerId) {
    for l in layers {
        if l.id == id {
            l.clipped = false;
            continue;
        }
        match &mut l.content {
            LayerContent::Group(g) if g.children.iter().any(|c| contains(c, id)) => {
                l.clipped = false;
                // Isolate the child, not the artboard's paper. Keep the board for clipping.
                if let Some(board) = &mut g.artboard {
                    board.background = photocraft_doc::ArtboardBackground::Transparent;
                }
                solo_layer(&mut g.children, id);
            }
            _ => l.visible = false,
        }
    }
}

fn contains(l: &photocraft_doc::Layer, id: LayerId) -> bool {
    l.id == id || matches!(&l.content, LayerContent::Group(g) if g.children.iter().any(|c| contains(c, id)))
}
