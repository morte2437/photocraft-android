use super::{LAYOUTS, PhotocraftApp, Value, json};

/// View › Fit Layer(s) on Screen: zoom and centre on the selected layers' bounds.
pub(super) fn fit_layers(app: &mut PhotocraftApp) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document")?;
    let st = app.session.active().ok_or("no document")?;
    let mut b = photocraft_geom::Rect::EMPTY;
    for id in st.selected_layers() {
        if let Some(r) = st.doc.layer(id).and_then(|l| l.surface()).map(|s| s.content_bounds()) {
            b = if b.is_empty() { r } else { b.union(&r) };
        }
    }
    if b.is_empty() {
        b = st.doc.bounds();
    }
    let size = [st.doc.size.width, st.doc.size.height];
    let area = app.last_canvas_rect.size();
    let area = if area.x > 50.0 { area } else { egui::vec2(1200.0, 800.0) };
    // The area is in egui points; the stored zoom is device pixels per document pixel.
    let ppp = app.canvas_ppp();
    let points = ((area.x - 40.0) / b.width().max(1) as f32).min((area.y - 40.0) / b.height().max(1) as f32);
    let v = &mut app.ui.views[i];
    v.zoom = crate::zoom_levels::clamp(points * ppp, size);
    v.center = [(b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0];
    v.fit_pending = false;
    v.request_zoom_animation(app.session.prefs().general.animated_zoom);
    v.zoom_anchor = None;
    Ok(json!({"zoom": v.zoom, "bounds": [b.x0, b.y0, b.x1, b.y1]}))
}

/// A finite `[x, y]` screen position within a sane range (window systems reject wild values).
fn screen_position(v: &Value) -> Option<[f32; 2]> {
    const LIMIT: f64 = 1.0e6;
    let a = v.as_array().filter(|a| a.len() == 2)?;
    let n = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite() && x.abs() <= LIMIT).map(|x| x as f32);
    Some([n(0)?, n(1)?])
}

pub(crate) fn float_window(app: &mut PhotocraftApp, doc: usize, offset: usize) -> u64 {
    let wid = app.ui.alloc_id();
    let mut view = app.ui.views.get(doc).cloned().unwrap_or_default();
    view.zoom_animating = false;
    view.zoom_anchor = None;
    view.zoom_transition = None;
    view.display_camera = None;
    view.fit_pending = offset > 0 || view.fit_pending;
    app.ui.windows.push(crate::state::DocWindow { id: wid, document: doc, view, open: true, position: None });
    wid
}

pub(super) fn arrange(app: &mut PhotocraftApp, k: &str, p: &Value) -> Result<Value, String> {
    if LAYOUTS.contains(&k) {
        app.ui.view.arrange = k.to_string();
        // Each document fits its new cell.
        app.ui.views.iter_mut().for_each(|v| v.fit_pending = true);
        return Ok(json!({"arrange": k}));
    }
    match k {
        "consolidateAllToTabs" => {
            app.ui.view.arrange = "tabs".into();
            app.ui.windows.clear();
            Ok(json!({"arrange": "tabs"}))
        }
        "newWindowForDocument" => {
            let doc = app.session.active_index().ok_or("no document")?;
            Ok(json!({"window": float_window(app, doc, 0)}))
        }
        // Optional `document` (index; default the active one) and `position` (`[x, y]`, the
        // window's screen position in points): dragging a tab off the tab strip uses both (#3733).
        "floatInWindow" => {
            let doc = match p.get("document") {
                None | Some(Value::Null) => app.session.active_index().ok_or("no document")?,
                Some(v) => {
                    let i = v.as_u64().ok_or("document must be a document index")?;
                    usize::try_from(i).ok().filter(|&i| i < app.session.documents().len()).ok_or(format!("no document {i}"))?
                }
            };
            let position = match p.get("position") {
                None | Some(Value::Null) => None,
                Some(v) => Some(screen_position(v).ok_or("position must be [x, y] in points")?),
            };
            let wid = float_window(app, doc, 0);
            if let Some(w) = app.ui.windows.iter_mut().find(|w| w.id == wid) {
                w.position = position;
            }
            app.session.set_active(doc);
            Ok(json!({"window": wid}))
        }
        "floatAllInWindows" | "cascade" => {
            let n = app.session.documents().len();
            let ids: Vec<u64> = (0..n).map(|d| float_window(app, d, d)).collect();
            Ok(json!({"windows": ids}))
        }
        "matchZoom" | "matchLocation" | "matchRotation" | "matchAll" => {
            let i = app.session.active_index().ok_or("no document")?;
            let src = app.ui.views.get(i).cloned().ok_or("no document")?;
            let (zoom, loc, rot) =
                (matches!(k, "matchZoom" | "matchAll"), matches!(k, "matchLocation" | "matchAll"), matches!(k, "matchRotation" | "matchAll"));
            let animated_zoom = app.session.prefs().general.animated_zoom;
            // Each view stays within its own document's zoom range (a view not drawn yet has no
            // `doc_size`, so the size comes from the document).
            let sizes: Vec<[u32; 2]> = app.session.documents().iter().map(|d| [d.doc.size.width, d.doc.size.height]).collect();
            let apply = |v: &mut crate::state::View, doc: usize| {
                if zoom {
                    v.zoom = crate::zoom_levels::clamp(src.zoom, sizes.get(doc).copied().unwrap_or(v.doc_size));
                    v.request_zoom_animation(animated_zoom);
                }
                if loc {
                    v.center = src.center;
                }
                if rot {
                    v.rotation = src.rotation;
                }
                v.fit_pending = false;
            };
            app.ui.views.iter_mut().enumerate().for_each(|(d, v)| apply(v, d));
            app.ui.windows.iter_mut().for_each(|w| apply(&mut w.view, w.document));
            Ok(json!({"zoom": src.zoom, "center": src.center, "rotation": src.rotation}))
        }
        _ => Err(format!("unknown arrangement {k}")),
    }
}

/// Screen rectangles for `n` documents in an arrangement (`None` = tabs). The active document
/// always gets a cell; n-up layouts show that many documents starting from the active one.
pub fn cells(layout: &str, rect: egui::Rect, n: usize) -> Option<Vec<egui::Rect>> {
    use craft_ui::layout::{SplitAxis, SplitSize, split_rect};

    if n < 2 || !LAYOUTS.contains(&layout) {
        return None;
    }
    let grid = |cols: usize, rows: usize, count: usize| -> Vec<egui::Rect> {
        let (w, h) = (rect.width() / cols as f32, rect.height() / rows as f32);
        (0..count.min(cols * rows))
            .map(|i| egui::Rect::from_min_size(rect.min + egui::vec2((i % cols) as f32 * w, (i / cols) as f32 * h), egui::vec2(w, h)))
            .collect()
    };
    Some(match layout {
        "tileAllVertically" => grid(n, 1, n),
        "tileAllHorizontally" => grid(1, n, n),
        "twoUpVertical" => {
            let panes = split_rect(rect, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 0.0, 0.0)?;
            vec![panes.first, panes.second]
        }
        "twoUpHorizontal" => {
            let panes = split_rect(rect, SplitAxis::Vertical, SplitSize::Ratio(0.5), 0.0, 0.0)?;
            vec![panes.first, panes.second]
        }
        "threeUpVertical" => grid(3, 1, 3),
        "threeUpHorizontal" => grid(1, 3, 3),
        "threeUpStacked" => {
            let columns = split_rect(rect, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 0.0, 0.0)?;
            let rows = split_rect(columns.second, SplitAxis::Vertical, SplitSize::Ratio(0.5), 0.0, 0.0)?;
            vec![columns.first, rows.first, rows.second]
        }
        "fourUp" => grid(2, 2, 4),
        "sixUp" => grid(3, 2, 6),
        _ => {
            let cols = (n as f32).sqrt().ceil() as usize;
            grid(cols, n.div_ceil(cols), n)
        }
    })
}
