use super::{Affine, Arc, Document, LayerContent, LayerId, PhotocraftApp, Point, PoisonError, TextLayer, TextLayout, text_layer};

/// Layout of a type layer (cached per document revision) and its text → document transform.
pub fn layout(app: &mut PhotocraftApp, id: LayerId) -> Option<(Arc<TextLayout>, Affine, String)> {
    let dragged = crate::type_transform::current_transform(app, id);
    // Borrows the document in place (no clone): the canvas hover hit-tests type every frame.
    let st = app.session.active()?;
    let t = text_layer(&st.doc, id)?;
    let transform = dragged.unwrap_or(t.transform);
    let key = (st.doc.id.0, st.revision, id.0);
    if let Some((k, l)) = &app.type_layout
        && *k == key
    {
        return Some((l.clone(), transform, t.text.clone()));
    }
    let l = Arc::new(photocraft_text::shared().lock().ok()?.layout(t, st.doc.resolution_dpi));
    app.type_layout = Some((key, l.clone()));
    Some((l, transform, t.text.clone()))
}

pub(super) fn to_text(aff: &Affine, x: f64, y: f64) -> (f32, f32) {
    let p = aff.inverse().unwrap_or(Affine::IDENTITY).apply(Point::new(x, y));
    (p.x as f32, p.y as f32)
}

/// Topmost shown type layer (it and every group around it visible, #4320) whose laid-out text
/// contains the document point. The press and the canvas hover (`hover_path_target`) share it, so
/// the pointer never promises a path click that the press would turn into editing this layer.
pub(crate) fn hit_layer(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<LayerId> {
    let slop = 6.0 / app.point_zoom().max(0.01);
    // A Type Mask session's text is a hidden layer; it is still the text being edited.
    let editing = app.ui.text_edit.as_ref().filter(|ed| ed.mask).map(|ed| LayerId(ed.layer));
    let mut ids: Vec<LayerId> = {
        let doc = &app.session.active()?.doc;
        doc.walk()
            .into_iter()
            .filter(|(path, _, l)| matches!(l.content, LayerContent::Text(_)) && (editing == Some(l.id) || doc.shown_at(path)))
            .map(|(_, _, l)| l.id)
            .collect()
    };
    ids.reverse(); // walk() is bottom-up; hit the topmost first
    ids.into_iter().find(|id| {
        // The pixels shown count too: a PSD's type keeps Photoshop's rendering until edited,
        // which can sit off our layout when fonts are substituted (#123).
        let shown = app.session.active().and_then(|st| text_layer(&st.doc, *id)).and_then(|t| t.cache.as_ref()).map(|c| c.content_bounds()).is_some_and(|r| {
            let s = f64::from(slop);
            !r.is_empty() && x >= f64::from(r.x0) - s && x <= f64::from(r.x1) + s && y >= f64::from(r.y0) - s && y <= f64::from(r.y1) + s
        });
        // The hover calls this every frame: skip the layout when the pixels already answer.
        if shown {
            return true;
        }
        let Some((l, aff, _)) = layout(app, *id) else { return false };
        let (tx, ty) = to_text(&aff, x, y);
        photocraft_text::text_point_inside(&l, tx, ty, slop)
    })
}

/// True when the layer's pixels are what our engine draws for it. A PSD's type layer keeps
/// Photoshop's pixels until it is edited, and with substituted fonts or a different line layout
/// they sit somewhere else than the glyphs the caret and selection are placed on.
pub(crate) fn shows_own_layout(doc: &Document, t: &TextLayer) -> bool {
    let Some(cache) = &t.cache else { return false };
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
    let ours = eng.render(t, doc.resolution_dpi, doc.pixel_format()).1.surface.content_bounds();
    let have = cache.content_bounds();
    [(ours.x0, have.x0), (ours.y0, have.y0), (ours.x1, have.x1), (ours.y1, have.y1)].iter().all(|(a, b)| (a - b).abs() <= 1)
}

pub(super) fn hit_offset(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> usize {
    offset_at(app, id, x, y)
}

/// The character offset of the caret position nearest document point (`x`, `y`) in layer `id`.
pub(crate) fn offset_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> usize {
    let Some((l, aff, text)) = layout(app, id) else { return 0 };
    let (tx, ty) = to_text(&aff, x, y);
    // Type on a path: the point's place along the path is the position in the (single) line.
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    let text_start = l.bounds().map_or(0.0, |bounds| f64::from(bounds[0]));
    let (tx, ty) = shape
        .as_ref()
        .and_then(|shape| photocraft_text::warp::Warp::on_path_with_text_start(shape, text_start))
        .and_then(|warp| warp.path_text_x_near_text(f64::from(tx), f64::from(ty), l.bounds().map_or(0.0, |bounds| f64::from(bounds[2]))))
        .map_or((tx, ty), |offset| (offset as f32, -1.0));
    photocraft_text::hit_char(&l, &text, tx, ty).0
}
