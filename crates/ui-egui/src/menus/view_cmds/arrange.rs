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

pub(crate) fn float_window(app: &mut PhotocraftApp, doc: usize, offset: usize, floating: bool) -> u64 {
    let wid = app.ui.alloc_id();
    let mut view = app.ui.views.get(doc).cloned().unwrap_or_default();
    view.zoom_animating = false;
    view.zoom_anchor = None;
    view.zoom_transition = None;
    view.display_camera = None;
    view.fit_pending = offset > 0 || view.fit_pending;
    app.ui.windows.push(crate::state::DocWindow { id: wid, document: doc, view, open: true, floating, focused: false, position: None });
    wid
}

/// The Window › Arrange items: a layout (2-up, 4-up, Tiled…), which starts from even cells and
/// refits every document, or one of the window commands beside them — Consolidate All to Tabs,
/// New Window, Float in Window, Float All in Windows, Match Zoom and the rest.
pub(super) fn arrange(app: &mut PhotocraftApp, k: &str, p: &Value) -> Result<Value, String> {
    if LAYOUTS.contains(&k) {
        app.ui.view.arrange = k.to_string();
        // A new arrangement starts from even cells.
        app.ui.view.arrange_splits.clear();
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
            Ok(json!({"window": float_window(app, doc, 0, false)}))
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
            // Tearing off the same tab again must not create another viewport for the
            // same document. Keep its existing view and position if it is already open.
            if let Some(w) = app.ui.windows.iter().find(|w| w.open && w.floating && w.document == doc) {
                let wid = w.id;
                app.session.set_active(doc);
                return Ok(json!({"window": wid}));
            }
            let wid = float_window(app, doc, 0, true);
            if let Some(w) = app.ui.windows.iter_mut().find(|w| w.id == wid) {
                w.position = position;
            }
            app.session.set_active(doc);
            Ok(json!({"window": wid}))
        }
        "floatAllInWindows" | "cascade" => {
            let n = app.session.documents().len();
            let ids: Vec<u64> = (0..n).map(|d| float_window(app, d, d, true)).collect();
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
            // A default-mode crop turns the active view's camera; that turn is the crop's, not the
            // view's own rotation (which picking the Crop tool made 0), so it isn't copied (#9308).
            let crop = crate::crop_mode::turns_view(app);
            let rotation = if crop { 0.0 } else { src.rotation };
            let apply = |v: &mut crate::state::View, doc: usize, cropped: bool| {
                if zoom {
                    v.zoom = crate::zoom_levels::clamp(src.zoom, sizes.get(doc).copied().unwrap_or(v.doc_size));
                    v.request_zoom_animation(animated_zoom);
                }
                if loc {
                    v.center = src.center;
                }
                if rot && !cropped {
                    v.rotation = rotation;
                }
                v.fit_pending = false;
            };
            app.ui.views.iter_mut().enumerate().for_each(|(d, v)| apply(v, d, crop && d == i));
            app.ui.windows.iter_mut().for_each(|w| apply(&mut w.view, w.document, false));
            // Match Zoom and Match Location copy no rotation: they report the camera's as it is.
            Ok(json!({"zoom": src.zoom, "center": src.center, "rotation": if rot { rotation } else { src.rotation }}))
        }
        _ => Err(format!("unknown arrangement {k}")),
    }
}

pub(crate) fn document_is_floating(app: &PhotocraftApp, doc: usize) -> bool {
    app.ui.windows.iter().any(|w| w.open && w.floating && w.document == doc)
}

/// Screen rectangles for `n` documents in an arrangement (`None` = tabs), with the borders
/// where they were left (`splits`, [`ViewOptions::arrange_splits`](super::ViewOptions)). The
/// active document always gets a cell; n-up layouts show that many documents starting from the
/// active one.
pub fn cells(layout: &str, rect: egui::Rect, n: usize, splits: &[f32]) -> Option<Vec<egui::Rect>> {
    let shape = Shape::of(layout, n)?;
    let (xs, ys) = shape.lines(rect, splits);
    Some(match shape {
        Shape::Grid { cols, rows, count } => (0..count.min(cols * rows))
            .filter_map(|i| {
                let (c, r) = (i % cols, i / cols);
                Some(egui::Rect::from_min_max(egui::pos2(*xs.get(c)?, *ys.get(r)?), egui::pos2(*xs.get(c + 1)?, *ys.get(r + 1)?)))
            })
            .collect(),
        // One tall cell on the left; the right column is cut in two.
        Shape::Stacked => {
            let (x, y) = (xs.get(1).copied().unwrap_or(rect.center().x), ys.get(1).copied().unwrap_or(rect.center().y));
            vec![
                egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.max.y)),
                egui::Rect::from_min_max(egui::pos2(x, rect.min.y), egui::pos2(rect.max.x, y)),
                egui::Rect::from_min_max(egui::pos2(x, y), rect.max),
            ]
        }
    })
}

/// A border between arranged documents that can be dragged, as in Photoshop (#10101).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// Which of the arrangement's splits it is: the index for [`drag_border`].
    pub split: usize,
    /// It runs top to bottom (and moves left and right).
    pub vertical: bool,
    /// Its line on screen: zero wide (or tall) across the cells it divides.
    pub line: egui::Rect,
}

/// The borders of an arrangement, as laid out by [`cells`].
pub fn borders(layout: &str, rect: egui::Rect, n: usize, splits: &[f32]) -> Vec<Border> {
    let Some(shape) = Shape::of(layout, n) else { return Vec::new() };
    let (xs, ys) = shape.lines(rect, splits);
    let inner = |v: &[f32]| v.get(1..v.len().saturating_sub(1)).unwrap_or_default().to_vec();
    let (inner_x, inner_y) = (inner(&xs), inner(&ys));
    let vertical = inner_x.iter().enumerate().map(|(i, &x)| Border {
        split: i,
        vertical: true,
        line: egui::Rect::from_min_max(egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)),
    });
    // 3-up Stacked's row border spans only the right column.
    let left = match shape {
        Shape::Stacked => inner_x.first().copied().unwrap_or(rect.min.x),
        Shape::Grid { .. } => rect.min.x,
    };
    let horizontal = inner_y.iter().enumerate().map(|(i, &y)| Border {
        split: inner_x.len() + i,
        vertical: false,
        line: egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(rect.max.x, y)),
    });
    vertical.chain(horizontal).collect()
}

/// Move border `split` of the arrangement to `pointer` (screen points), keeping every cell at
/// least [`MIN_CELL`] points across where the area allows. `splits` is rewritten in full, so a
/// stale or damaged list starts again from even cells.
pub fn drag_border(layout: &str, rect: egui::Rect, n: usize, splits: &mut Vec<f32>, split: usize, pointer: egui::Pos2) {
    let Some(shape) = Shape::of(layout, n) else { return };
    let (cols, rows) = shape.counts();
    let mut fractions = shape.fractions(splits);
    // The borders on the dragged one's axis are `fractions[first..first + count]`.
    let (axis_start, axis_len, first, count, pos) = if split < cols - 1 {
        (rect.min.x, rect.width(), 0, cols - 1, pointer.x)
    } else if split < (cols - 1) + (rows - 1) {
        (rect.min.y, rect.height(), cols - 1, rows - 1, pointer.y)
    } else {
        return;
    };
    if !(axis_len > 0.0 && pos.is_finite()) {
        return;
    }
    let own = fractions.get(first..first + count).map(<[f32]>::to_vec).unwrap_or_default();
    let k = split - first;
    // [`MIN_CELL`] in fractions of this axis, whatever the number of cells: capping it by the
    // even spacing would let a cell fall under the minimum on a long axis (#10101 review).
    let gap = MIN_CELL / axis_len;
    let lo = if k == 0 { 0.0 } else { own.get(k - 1).copied().unwrap_or(0.0) } + gap;
    let hi = own.get(k + 1).copied().unwrap_or(1.0) - gap;
    // The neighbours leave no room for a cell of the minimum width: the border stays where it is,
    // rather than taking a value out of order that would invalidate the whole list.
    if lo > hi {
        return;
    }
    if let Some(f) = fractions.get_mut(split) {
        *f = ((pos - axis_start) / axis_len).clamp(lo, hi);
    }
    *splits = fractions;
}

/// The narrowest a cell is dragged to, in points.
pub const MIN_CELL: f32 = 80.0;

/// An arrangement as columns and rows of cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `cols` × `rows` even cells, the first `count` of them used.
    Grid { cols: usize, rows: usize, count: usize },
    /// 3-up Stacked: two columns, the right one in two rows.
    Stacked,
}

impl Shape {
    fn of(layout: &str, n: usize) -> Option<Shape> {
        if n < 2 || !LAYOUTS.contains(&layout) {
            return None;
        }
        let grid = |cols: usize, rows: usize, count: usize| Shape::Grid { cols, rows, count };
        Some(match layout {
            "tileAllVertically" => grid(n, 1, n),
            "tileAllHorizontally" => grid(1, n, n),
            "twoUpVertical" => grid(2, 1, 2),
            "twoUpHorizontal" => grid(1, 2, 2),
            "threeUpVertical" => grid(3, 1, 3),
            "threeUpHorizontal" => grid(1, 3, 3),
            "threeUpStacked" => Shape::Stacked,
            "fourUp" => grid(2, 2, 4),
            "sixUp" => grid(3, 2, 6),
            _ => {
                let cols = (n as f32).sqrt().ceil() as usize;
                grid(cols, n.div_ceil(cols), n)
            }
        })
    }

    fn counts(self) -> (usize, usize) {
        match self {
            Shape::Grid { cols, rows, .. } => (cols, rows),
            Shape::Stacked => (2, 2),
        }
    }

    /// The split positions as fractions: the column borders, then the row borders. `splits`
    /// is used only when it fits this shape (as many values, each axis increasing within 0–1);
    /// otherwise the cells are even.
    fn fractions(self, splits: &[f32]) -> Vec<f32> {
        let (cols, rows) = self.counts();
        let even = |k: usize| (1..k).map(move |i| i as f32 / k as f32);
        let ordered = |v: &[f32]| v.iter().all(|f| f.is_finite() && *f > 0.0 && *f < 1.0) && v.windows(2).all(|w| matches!(w, [a, b] if a < b));
        let fit = splits.len() == (cols - 1) + (rows - 1) && splits.get(..cols - 1).is_some_and(ordered) && splits.get(cols - 1..).is_some_and(ordered);
        if fit { splits.to_vec() } else { even(cols).chain(even(rows)).collect() }
    }

    /// Every column line's x and every row line's y on screen, the area's edges included.
    fn lines(self, rect: egui::Rect, splits: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (cols, _) = self.counts();
        let fractions = self.fractions(splits);
        let (fx, fy) = fractions.split_at((cols - 1).min(fractions.len()));
        let at = |start: f32, len: f32, inner: &[f32]| {
            std::iter::once(start).chain(inner.iter().map(|f| start + len * f)).chain(std::iter::once(start + len)).collect::<Vec<f32>>()
        };
        (at(rect.min.x, rect.width(), fx), at(rect.min.y, rect.height(), fy))
    }
}
