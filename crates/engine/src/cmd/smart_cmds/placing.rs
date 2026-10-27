use super::source::{SourceImage, buffer_to_surface, cache_key, cache_put, source_bytes};
use super::*;

/// Rounds float noise off a composed transform (scale 0.1 then 10 should be exactly 1), so
/// repeated transforms land back on exact, lossless placements.
pub(crate) fn snap_affine(a: Affine) -> Affine {
    Affine { m: a.m.map(|v| if (v - v.round()).abs() < 1e-9 { v.round() } else { v }) }
}

/// Where the smart object's source pixels land in the document: its projective map (Distort,
/// Perspective) or its affine transform.
pub fn placement(sm: &SmartObject) -> Homography {
    sm.perspective.map(Homography).unwrap_or_else(|| affine_homography(&sm.transform))
}

pub(crate) fn affine_homography(a: &Affine) -> Homography {
    let [a, b, c, d, e, f] = a.m;
    Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])
}

/// Sets where the source lands. An affine map is stored exactly in `transform` (no
/// `perspective`); a projective one in `perspective`, with `transform` its affine approximation at
/// the source origin (for code that only needs scale and position).
pub(crate) fn set_placement(sm: &mut SmartObject, h: Homography) {
    let m = h.0;
    let n = if m[8].abs() > 1e-12 { m.map(|v| v / m[8]) } else { m };
    if n[6].abs() < 1e-12 && n[7].abs() < 1e-12 {
        sm.transform = snap_affine(Affine { m: [n[0], n[3], n[1], n[4], n[2], n[5]] });
        sm.perspective = None;
        return;
    }
    let h = Homography(n);
    let (o, x, y) = (h.apply(0.0, 0.0), h.apply(1.0, 0.0), h.apply(0.0, 1.0));
    sm.transform = Affine { m: [x.0 - o.0, x.1 - o.1, y.0 - o.0, y.1 - o.1, o.0, o.1] };
    sm.perspective = Some(n);
}

/// Composes `a` (document → document) onto the smart object's placement.
pub(crate) fn transform_placement(sm: &mut SmartObject, a: &Affine) {
    match sm.perspective {
        None => sm.transform = snap_affine(a.mul(&sm.transform)),
        Some(_) => set_placement(sm, affine_homography(a).mul(&placement(sm))),
    }
}

/// Moves a smart object by whole pixels without re-rendering.
pub(crate) fn shift_smart(sm: &mut SmartObject, dx: i32, dy: i32) {
    transform_placement(sm, &Affine::translate(dx as f64, dy as f64));
    if let Some(c) = &mut sm.cache {
        *c = crate::layer_multi_cmds::shift_surface(c, dx, dy);
    }
    if let Some(m) = &mut sm.filter_mask {
        m.surface = crate::layer_multi_cmds::shift_surface(&m.surface, dx, dy);
    }
}

/// Forget PSD placed-layer data that no longer describes the smart object (its source changed):
/// PSD export then writes fresh placed-layer blocks and embeds the new source.
pub(super) fn detach_psd(l: &mut Layer) {
    if let LayerContent::Smart(sm) = &mut l.content {
        sm.psd_raw = None;
    }
    l.psd_blocks.retain(|(k, _)| !PLACED_KEYS.contains(&k));
}

// ---------- conversion ----------

/// Moves a layer (any kind) by whole pixels without re-rendering anything.
pub(super) fn shift_layer(l: &mut Layer, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    let a = Affine::translate(dx as f64, dy as f64);
    let mv = |s: &Surface| translate_surface(s, dx, dy);
    if let Some(m) = &mut l.mask {
        m.surface = mv(&m.surface);
    }
    if let Some(vm) = &mut l.vector_mask {
        vm.path = vm.path.transform(&a);
    }
    if let Some(fc) = &mut l.fill_cache {
        fc.surface = mv(&fc.surface);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = mv(s),
        LayerContent::Text(t) => {
            t.transform = a.mul(&t.transform);
            if let Some(c) = &mut t.cache {
                *c = mv(c);
            }
        }
        LayerContent::Shape(sh) => {
            crate::vector_cmds::transform_shape(sh, &a);
            if let Some(c) = &mut sh.cache {
                *c = mv(c);
            }
        }
        LayerContent::Smart(sm) => shift_smart(sm, dx, dy),
        LayerContent::Group(g) => g.children.iter_mut().for_each(|c| shift_layer(c, dx, dy)),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
    }
}

fn subtree_bounds(l: &Layer, canvas: Rect) -> Rect {
    match &l.content {
        LayerContent::Group(g) => g.children.iter().map(|c| subtree_bounds(c, canvas)).fold(Rect::EMPTY, |a, b| a.union(&b)),
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => canvas,
        _ => l.surface().map_or(Rect::EMPTY, Surface::content_bounds),
    }
}

fn any_layer(l: &Layer, f: &dyn Fn(&Layer) -> bool) -> bool {
    f(l) || l.children().is_some_and(|c| c.iter().any(|c| any_layer(c, f)))
}

/// Builds the smart-object layer replacing `l` in `doc`: the layer (with its mask, effects and
/// fill opacity) becomes a nested document cropped to its rendered bounds; visibility, opacity,
/// blend mode, clipping and label stay on the smart layer. The cache is the exact composite, so
/// the document looks the same before and after.
pub fn layer_to_smart(doc: &Document, l: &Layer) -> Result<Layer> {
    let background = doc.is_background(l.id);
    let mut inner = l.clone();
    inner.visible = true;
    inner.opacity = 1.0;
    inner.clipped = false;
    inner.locks = Default::default();
    if !(inner.is_group() && inner.blend == BlendMode::PassThrough) {
        inner.blend = BlendMode::Normal;
    }
    if background {
        inner.name = "Layer 0".into();
    }
    let mut out = layers_to_smart(doc, &l.name, vec![inner])?;
    if background {
        out.name = "Layer 0".into();
    }
    out.visible = l.visible;
    out.opacity = l.opacity;
    out.blend = if l.blend == BlendMode::PassThrough { BlendMode::Normal } else { l.blend };
    out.clipped = l.clipped;
    out.label = l.label;
    if !background {
        out.locks = l.locks;
    }
    Ok(out)
}

/// Builds a smart-object layer named `name` whose nested document holds `layers` (bottom to top)
/// at its top level, as Photoshop does when several layers are converted: no wrapper group. The
/// nested document is cropped to the rendered bounds and the cache is the exact composite (the
/// same as the layers inside a pass-through group). The smart layer is visible, opaque and Normal.
pub fn layers_to_smart(doc: &Document, name: &str, layers: Vec<Layer>) -> Result<Layer> {
    let canvas = doc.bounds();
    let mut sub = Document::new(format!("{name}.pcraft"), doc.size, doc.mode, doc.depth);
    sub.resolution_dpi = doc.resolution_dpi;
    sub.icc_profile = doc.icc_profile.clone();
    sub.global_light = doc.global_light;
    // Pattern fills and pattern layer effects reference document-level patterns by ID.
    // Preserve those resources in the embedded document before rendering or saving it.
    sub.patterns = doc.patterns.clone();
    if layers.iter().any(|l| any_layer(l, &|x| matches!(x.content, LayerContent::Smart(_)))) {
        // Nested PSD placed layers find their embedded files here.
        sub.metadata.psd_global_blocks = doc.metadata.psd_global_blocks.clone();
    }
    let has_fx = layers.iter().any(|l| any_layer(l, &|x| x.effects.enabled && !x.effects.items.is_empty()));
    let region = layers.iter().map(|l| subtree_bounds(l, canvas)).fold(canvas, |a, b| a.union(&b)).inflate(if has_fx { 256 } else { 0 });
    sub.layers = layers;

    let buf = photocraft_compose::render(&sub, region);
    let w = region.width() as usize;
    let mut b = Rect::EMPTY;
    for (i, p) in buf.px.iter().enumerate() {
        if p[3] > 0.0 {
            let (x, y) = (region.x0 + (i % w) as i32, region.y0 + (i / w) as i32);
            b = b.union(&Rect::new(x, y, x + 1, y + 1));
        }
    }
    if b.is_empty() {
        return Err(other("the layer is empty, so there is nothing to convert"));
    }
    let fmt = doc.pixel_format();
    let mut cache = buffer_to_surface(&buf, fmt);
    cache = photocraft_algo::resample::crop_surface(&cache, b);
    cache.prune();

    sub.layers.iter_mut().for_each(|l| shift_layer(l, -b.x0, -b.y0));
    sub.size = Size::new(b.width(), b.height());
    let bytes = encode_source(&sub)?;
    // Seed the decode cache with the composite we already have.
    cache_put(cache_key(&bytes, fmt), SourceImage { surface: Arc::new(translate_surface(&cache, -b.x0, -b.y0)), bounds: sub.bounds() });

    let source = SmartSource::Embedded { file_name: sub.name.clone(), bytes: Arc::new(bytes) };
    Ok(Layer::new(name, LayerContent::Smart(SmartObject::new(source, Affine::translate(b.x0 as f64, b.y0 as f64), Some(cache)))))
}

pub(super) fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => vec![LayerId(id)],
        None => crate::layer_multi_cmds::selected(s),
    };
    let new_id = s.edit("Convert to Smart Object", |doc, active| {
        let ids = crate::layer_multi_cmds::top_level(doc, &ids);
        let top = *ids.last().ok_or_else(|| other("no layer selected"))?;
        let l = doc.layer(top).ok_or(EngineError::NoLayer(top))?;
        let so = if ids.len() == 1 {
            layer_to_smart(doc, l)?
        } else {
            let children = ids.iter().map(|id| doc.layer(*id).cloned().ok_or(EngineError::NoLayer(*id))).collect::<Result<Vec<_>>>()?;
            layers_to_smart(doc, &l.name, children)?
        };
        let new_id = so.id;
        // As with Group Layers, place the result in the top selected root's parent. Insert
        // before removal so paths shifting in other parents cannot discard unselected content.
        doc.insert_above(Some(top), so);
        for id in &ids {
            doc.remove(*id).ok_or(EngineError::NoLayer(*id))?;
        }
        if let [only] = ids[..] {
            retarget_layer_refs(doc, only, new_id);
        }
        *active = Some(new_id);
        Ok(new_id)
    })?;
    crate::layer_multi_cmds::reselect(s, vec![new_id], Some(new_id));
    Ok(json!({"layer": new_id.0}))
}

fn retarget_layer_refs(doc: &mut Document, from: LayerId, to: LayerId) {
    let Some((fill_opacity, effects)) = doc.layer(to).map(|l| (l.fill_opacity, l.effects.clone())) else { return };
    for comp in doc.layer_comps.iter_mut().chain(doc.last_document_state.as_mut()) {
        for state in comp.states.iter_mut().filter(|state| state.layer == from) {
            state.layer = to;
            if let Some(appearance) = &mut state.appearance {
                appearance.fill_opacity = fill_opacity;
                appearance.effects = effects.clone();
            }
        }
    }
    for def in doc.variables.defs.iter_mut().filter(|def| def.layer == from) {
        def.layer = to;
    }
    for slice in doc.slices.list.iter_mut().filter(|slice| slice.layer == Some(from)) {
        slice.layer = Some(to);
    }
}

// ---------- contents ----------

pub(super) fn via_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("New Smart Object via Copy", |doc, active| {
        let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let mut copy = src.duplicate();
        copy.name = doc.copy_name(&src.name);
        if let LayerContent::Smart(sm) = &mut copy.content {
            // Independent contents: resolve to embedded bytes and drop the shared PSD uuid.
            sm.contents_id = SmartContentsId::fresh();
            if let Some((file_name, bytes)) = source_bytes(&doc.metadata, &sm.source) {
                sm.source = SmartSource::Embedded { file_name, bytes };
            }
        } else {
            return Err(other("the layer is not a smart object"));
        }
        detach_psd(&mut copy);
        let new_id = doc.insert_above(Some(id), copy);
        *active = Some(new_id);
        Ok(json!({"layer": new_id.0}))
    })
}
