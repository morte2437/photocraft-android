use super::view_xform::ViewXform;
use super::{PhotocraftApp, Pos2, Rect, Stroke, Tool, vec2};

/// An in-progress pointer gesture on the canvas.
#[derive(Clone, Debug)]
pub struct Drag {
    pub tool: Tool,
    pub start: [f64; 2],
    pub points: Vec<[f64; 3]>,
    pub modifiers: egui::Modifiers,
    /// A Brush/Eraser stroke that erases: the Eraser, or a right-button Brush drag with
    /// Preferences › Tools › Right-click with painting tools set to Erase.
    pub erase: bool,
    /// A Brush or Pencil stroke in Photoshop's Clear mode (`paint_mouse::clears`, #3439): it
    /// erases, sent as `"mode": "clear"` so History names the tool rather than the Eraser.
    pub clear: bool,
    /// ⇧ constraint of a painting stroke (stroke_constraint.rs).
    pub constrain: Option<crate::stroke_constraint::Axis>,
    /// Modifiers held now (updated on every pointer event of the gesture).
    pub live: egui::Modifiers,
    /// Which of the modifiers held at the press have been released since.
    pub released: egui::Modifiers,
    /// The reposition key (Space) is held: pointer moves move the whole gesture instead of
    /// sizing it (marquees, lasso, shapes; `hold_keys`).
    pub reposition: bool,
    /// A marquee or lasso drag that started inside the selection moves it instead of drawing:
    /// `Some(false)` moves the outline, `Some(true)` moves the floating piece (`select.float`), as
    /// every Move-tool drag with a selection does (`move_ui::moves_selected_pixels_with`).
    pub sel_move: Option<bool>,
    /// The last ⇧-locked offset of a selection-tool drag of the selection (`move_mods`, #6231);
    /// keeps the locked direction stable between events. The Move tool's own drags use `move_mods`.
    pub sel_axis: Option<[f64; 2]>,
    pub lasso: Option<crate::lasso_ui::Lasso>,
    /// Freeform Pen and Path Selection gestures belong to their source document and target.
    pub(super) vector_owner: Option<VectorGestureOwner>,
    pub(super) pen_owner: Option<crate::vector_ui::PenOwner>,
}

impl Drag {
    pub fn new(tool: Tool, start: [f64; 2], points: Vec<[f64; 3]>, modifiers: egui::Modifiers, erase: bool) -> Self {
        Self {
            tool,
            start,
            points,
            modifiers,
            erase,
            clear: false,
            constrain: None,
            live: modifiers,
            released: egui::Modifiers::NONE,
            reposition: false,
            sel_move: None,
            sel_axis: None,
            lasso: None,
            vector_owner: None,
            pen_owner: None,
        }
    }

    /// Vector drags capture their owner at the press, the Freeform Pen also its destination, before
    /// later ⌥-clicks or a temporary Direct Selection can finish them.
    pub(crate) fn capture_vector_owners(&mut self, app: &PhotocraftApp) {
        if matches!(self.tool, Tool::FreeformPen | Tool::PathSelection) {
            self.vector_owner = VectorGestureOwner::capture(app);
        }
        if self.tool == Tool::FreeformPen {
            self.pen_owner = crate::vector_ui::capture_pen_owner(app);
        }
    }

    /// Reposition: move the start and every point so the last one lands on `to` (same size).
    pub fn shift_to(&mut self, to: [f64; 2]) {
        let last = self.points.last().map_or(self.start, |p| [p[0], p[1]]);
        let (dx, dy) = (to[0] - last[0], to[1] - last[1]);
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        self.start = [self.start[0] + dx, self.start[1] + dy];
        for p in &mut self.points {
            p[0] += dx;
            p[1] += dy;
        }
        if self.points.is_empty() {
            self.points.push([to[0], to[1], 1.0]);
        }
    }

    /// Record the modifiers of a pointer event.
    pub(crate) fn track(&mut self, mods: egui::Modifiers) {
        self.live = mods;
        self.released.shift |= !mods.shift;
        self.released.alt |= !mods.alt;
    }

    /// ⇧ (square / circle) and ⌥ (from the centre) for a marquee drag. A modifier held at the press
    /// picks the selection mode instead (add / subtract, #188) until it is released and pressed
    /// again, as in Photoshop.
    pub fn marquee_mods(&self) -> (bool, bool) {
        let fresh = |now: bool, at_press: bool, released: bool| now && (!at_press || released);
        (fresh(self.live.shift, self.modifiers.shift, self.released.shift), fresh(self.live.alt, self.modifiers.alt, self.released.alt))
    }
}

/// The marquee dragged from `d.start` to `end` as two document-space corners: the options-bar
/// style (fixed ratio / fixed size), ⇧ for a square or circle, ⌥ to draw from the centre.
pub fn marquee_corners(o: &crate::state::ToolOptions, d: &Drag, end: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let (shift, alt) = d.marquee_mods();
    if o.marquee_style == "fixedSize" {
        // A fixed box is positioned, not sized, by the drag. Using the press plus the drag's
        // sign pinned it there and flipped it by its full width/height when the pointer crossed
        // the press. The preview and release both follow the current pointer instead.
        let (w, h) = (f64::from(o.marquee_width).max(1.0), f64::from(o.marquee_height).max(1.0));
        let a = if alt { [end[0] - w / 2.0, end[1] - h / 2.0] } else { end };
        return (a, [a[0] + w, a[1] + h]);
    }
    let e = crate::chrome_ui::marquee_end(&o.marquee_style, o.marquee_width as f64, o.marquee_height as f64, shift, d.start, end);
    if !alt {
        return (d.start, e);
    }
    let (dx, dy) = (e[0] - d.start[0], e[1] - d.start[1]);
    // For a sized marquee the drag is the half extent when drawing from the centre.
    ([d.start[0] - dx, d.start[1] - dy], [d.start[0] + dx, d.start[1] + dy])
}

/// The pixel rectangle a Rectangular/Elliptical Marquee drag selects so far: shown live (ants
/// and readout) exactly as the release commits it, on whole pixel edges as in Photoshop. `None`
/// while the gesture is still a click (it never left the press point by more than a few screen
/// pixels at `zoom`, points per document pixel) or the rectangle has no area: a release then
/// deselects. A one-pixel row or column is a selection (#2391). With the Fixed Size style a
/// click places the selection, as in Photoshop.
pub(crate) fn marquee_preview_px(o: &crate::state::ToolOptions, d: &Drag, zoom: f32) -> Option<[f64; 4]> {
    let fixed = o.marquee_style == "fixedSize";
    if !matches!(d.tool, Tool::RectMarquee | Tool::EllipseMarquee) || (!fixed && crate::shape_dialog::is_click(d.start, &d.points, zoom)) {
        return None;
    }
    let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let (a, b) = marquee_corners(o, d, last);
    let mut r = if fixed { fixed_size_px(a, b, o.marquee_width, o.marquee_height) } else { marquee_px(a, b) };
    if marquee_is_square(o, d) {
        r = square_px(r, d.start, last);
    }
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// The pixel rectangle `[x0, y0, x1, y1]` a marquee between two corners selects.
pub fn marquee_px(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [a[0].min(b[0]).floor(), a[1].min(b[1]).floor(), a[0].max(b[0]).ceil(), a[1].max(b[1]).ceil()]
}

/// The pixel rectangle of a Fixed Size marquee between the corners `a` and `b`: exactly `w` × `h`
/// whole pixels. Snapping both edges outwards (`marquee_px`) added a pixel to each side whenever
/// the press point sat between pixels, so 10 × 10 selected 11 × 11 (#6015). The near edge goes
/// to the closest pixel edge and the far one is the size away.
pub fn fixed_size_px(a: [f64; 2], b: [f64; 2], w: f32, h: f32) -> [f64; 4] {
    let (w, h) = (f64::from(w).round().max(1.0), f64::from(h).round().max(1.0));
    let (x0, y0) = (a[0].min(b[0]).round(), a[1].min(b[1]).round());
    [x0, y0, x0 + w, y0 + h]
}

/// Whether the marquee drag `d` is constrained to a square (or circle): ⇧ during the drag with
/// the Normal style, or a Fixed Ratio of 1:1. Mirrors the branches of `chrome_ui::marquee_end`.
fn marquee_is_square(o: &crate::state::ToolOptions, d: &Drag) -> bool {
    let (w, h) = (f64::from(o.marquee_width), f64::from(o.marquee_height));
    match o.marquee_style.as_str() {
        "fixedSize" => false,
        "fixedRatio" if w > 0.0 && h > 0.0 => w == h,
        _ => d.marquee_mods().0,
    }
}

/// A constrained square `r` (`marquee_px` of its corners) with equal sides on whole pixels.
///
/// The square is built in continuous coordinates, and snapping each axis to the pixel grid on
/// its own (floor the near edge, ceil the far one) made one side a pixel longer than the other
/// whenever the press point sat between pixels: a ⇧ drag read 101 × 102 (#5472). The shorter
/// side grows to the longer one, away from the press point (`start`, towards `end`), so the
/// square still covers every pixel the drag touched.
pub fn square_px(r: [f64; 4], start: [f64; 2], end: [f64; 2]) -> [f64; 4] {
    let [x0, y0, x1, y1] = r;
    let (w, h) = (x1 - x0, y1 - y0);
    let n = w.max(h);
    let grow = |lo: f64, hi: f64, extent: f64, from: f64, to: f64| {
        let d = (n - extent).max(0.0);
        if to < from { (lo - d, hi) } else { (lo, hi + d) }
    };
    let (x0, x1) = grow(x0, x1, w, start[0], end[0]);
    let (y0, y1) = grow(y0, y1, h, start[1], end[1]);
    [x0, y0, x1, y1]
}

/// The one-pixel line `[x0, y0, x1, y1]` a Single Row (Single Column) Marquee at `p` selects: the
/// pixel row (column) under `p`, kept on the canvas, across the whole `size`. `None` for other
/// tools, an empty canvas or a point that is not a number.
pub(crate) fn single_line_px(tool: Tool, size: [u32; 2], p: [f64; 2]) -> Option<[f64; 4]> {
    let [w, h] = size.map(f64::from);
    if w < 1.0 || h < 1.0 || !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    match tool {
        Tool::SingleRowMarquee => {
            let y = p[1].floor().clamp(0.0, h - 1.0);
            Some([0.0, y, w, y + 1.0])
        }
        Tool::SingleColumnMarquee => {
            let x = p[0].floor().clamp(0.0, w - 1.0);
            Some([x, 0.0, x + 1.0, h])
        }
        _ => None,
    }
}

/// The line a Single Row / Single Column Marquee gesture `d` selects so far: it follows the
/// pointer until the release.
pub(super) fn single_line_drag_px(app: &PhotocraftApp, d: &Drag) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    single_line_px(d.tool, [st.doc.size.width, st.doc.size.height], last)
}

/// The marching ants of a marquee drag: its rectangle, or the ellipse inscribed in it, built in
/// document space and mapped point by point through the view, so the hinting lands where the
/// release commits the selection.
///
/// A rotated view projects the rectangle onto a rotated one, and an axis-aligned screen box
/// through two of its corners is a different shape in a different place: the commit was always
/// right, only the hinting was drawn elsewhere (#2512). The points are not snapped: the ants snap
/// straight edges to device pixels as they draw them (`crate::ants`), like every other outline, so
/// the lines stay crisp at any zoom (#172) and display scale (#7489).
pub(crate) fn marquee_ants(xf: &ViewXform, tool: Tool, a: [f64; 2], b: [f64; 2]) -> Vec<Pos2> {
    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
    let at = |x: f64, y: f64| xf.to_screen(x as f32, y as f32);
    if tool != Tool::EllipseMarquee {
        return vec![at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)];
    }
    // An axis-aligned view (rotation a multiple of 90 degrees) maps the ellipse onto the one
    // inscribed in the screen box of two opposite corners. The ants draw its vertices where they
    // are: snapping each on its own turns the nearly flat arcs at the edge midpoints into 1 px
    // stairs, whose antialiased steps wash the ants out to grey on white and black.
    let quarter = xf.rotation.rem_euclid(90.0);
    if quarter < 1e-4 || 90.0 - quarter < 1e-4 {
        let r = Rect::from_two_pos(at(x0, y0), at(x1, y1));
        return crate::tool_feedback::ellipse_points(r);
    }
    // As many vertices as the same ellipse would get drawn on screen (`ellipse_points`).
    let scr = Rect::from_points(&[xf.to_screen(x0 as f32, y0 as f32), xf.to_screen(x1 as f32, y1 as f32)]);
    let n = (((scr.width() + scr.height()) * 0.25) as usize).clamp(16, 256);
    let (cx, cy, rx, ry) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5);
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            at(cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// The ants of the marquee being drawn: `marquee` when the drag has one (the options-bar style
/// and the modifiers applied), the raw press and pointer corners otherwise. This is the whole of
/// what the hinting draws for the selection tools, so the test in `marquee_tests` checks it
/// against the selection the release commits (#2512).
pub(crate) fn marquee_drag_ants(d: &Drag, xf: &ViewXform, marquee: Option<[f64; 4]>) -> Vec<Pos2> {
    let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let (a, b) = marquee.map_or((d.start, last), |[x0, y0, x1, y1]| ([x0, y0], [x1, y1]));
    marquee_ants(xf, d.tool, a, b)
}

/// The size readout shown beside the cursor while dragging a marquee: the width and height values.
pub fn marquee_readout(r: [f64; 4]) -> [String; 2] {
    [format!("{} px", r[2] - r[0]), format!("{} px", r[3] - r[1])]
}

/// [`marquee_readout`] in the Units & Rulers unit, as Photoshop shows it (cm when the rulers are in
/// centimeters), from the active document's resolution; pixels without a document.
pub fn size_readout(app: &PhotocraftApp, r: [f64; 4]) -> [String; 2] {
    let Some(d) = app.session.active() else { return marquee_readout(r) };
    let (dpi, size) = (f64::from(d.doc.resolution_dpi), d.doc.size);
    readout_in(&app.session.prefs().units_and_rulers, dpi, [size.width as f64, size.height as f64], r)
}

/// `r`'s width and height formatted in `ur`'s ruler unit at `dpi` (percent of `extent`).
pub fn readout_in(ur: &photocraft_engine::prefs::UnitsAndRulers, dpi: f64, extent: [f64; 2], r: [f64; 4]) -> [String; 2] {
    let s = ur.rulers.suffix();
    [format!("{} {s}", ur.format(r[2] - r[0], dpi, extent[0])), format!("{} {s}", ur.format(r[3] - r[1], dpi, extent[1]))]
}

/// Draw the marquee size readout below-right of the cursor (kept on screen), like Photoshop's:
/// two rows, `W:` / `H:` labels on the left and the values right-aligned.
pub(super) fn draw_marquee_readout(app: &PhotocraftApp, ctx: &egui::Context, cursor: Pos2, values: [String; 2]) {
    draw_value_readout(app, ctx, "marquee-readout", cursor, ["W:", "H:"], values);
}

/// A turn of `deg` degrees as the readout beside the pointer shows it, to 0.1° in (-180, 180].
/// Rounded first so a hair below zero never reads "-0.0°" and a hair above -180 reads "180.0°".
pub(crate) fn angle_readout(deg: f64) -> String {
    let d = (deg * 10.0).round() / 10.0;
    let d = if d == 0.0 {
        0.0
    } else if d == -180.0 {
        180.0
    } else {
        d
    };
    format!("{d:.1}°")
}

/// A transformation-values readout beside the pointer (a marquee's or the crop frame's size, an
/// angle), unless Preferences › Tools › Show Transformation Values is off, as in Photoshop (#204).
/// The brush HUD isn't one: Show Transformation Values doesn't hide it, so it calls [`draw_readout`].
pub(crate) fn draw_value_readout<const N: usize>(app: &PhotocraftApp, ctx: &egui::Context, id: &str, cursor: Pos2, labels: [&str; N], values: [String; N]) {
    if app.session.prefs().tools.show_transformation_values {
        draw_readout(ctx, id, cursor, labels, values);
    }
}

/// A readout of `N` rows beside the pointer (labels left, values right-aligned), above everything.
pub(crate) fn draw_readout<const N: usize>(ctx: &egui::Context, id: &str, cursor: Pos2, labels: [&str; N], values: [String; N]) {
    let t = crate::theme::Tokens::get(ctx);
    let font = egui::FontId::proportional(11.5);
    let width = |text: &str| ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), t.text).size().x);
    let labels = labels.map(|label| tl!(label));
    let (lw, vw) = (labels.map(width), values.each_ref().map(|v| width(v)));
    let (label_col, value_col) = (lw.iter().fold(0.0f32, |a, &b| a.max(b)), vw.iter().fold(0.0f32, |a, &b| a.max(b)));
    egui::Area::new(egui::Id::unique(id)).order(egui::Order::Tooltip).fixed_pos(cursor + vec2(16.0, 18.0)).interactable(false).constrain(true).show(
        ctx,
        |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_sm)
                .inner_margin(egui::Margin::symmetric(7, 4))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    for (i, value) in values.into_iter().enumerate() {
                        // Labels left, values right-aligned on one edge, sharing a baseline.
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(labels[i]).font(font.clone()).color(t.text_dim)).extend());
                            ui.add_space(label_col - lw[i] + 12.0 + value_col - vw[i]);
                            ui.add(egui::Label::new(egui::RichText::new(value).font(font.clone()).color(t.text)).extend());
                        });
                    }
                });
        },
    );
}

/// The document, revision and target a vector drag started on.
#[derive(Clone, Debug)]
pub(super) struct VectorGestureOwner {
    doc: photocraft_doc::DocId,
    document: std::sync::Weak<Document>,
    revision: u64,
    layer: Option<photocraft_doc::LayerId>,
    selected_path: Option<String>,
    vector_mask: bool,
    mode: String,
    tool: Tool,
}

/// Cancel a vector drag before it can preview or finish on another document or target; the rest of
/// its pointer events go nowhere. True when one was cancelled.
pub(crate) fn cancel_stale_vector_drag(app: &mut PhotocraftApp) -> bool {
    let stale = app.drag.as_ref().and_then(|d| d.vector_owner.as_ref()).is_some_and(|owner| !owner.valid(app));
    if stale {
        app.drag = None;
        app.cancelled_vector_drag = true;
    }
    stale
}

impl VectorGestureOwner {
    fn capture(app: &PhotocraftApp) -> Option<Self> {
        let st = app.session.active()?;
        Some(Self {
            doc: st.doc.id,
            document: std::sync::Arc::downgrade(&st.doc),
            revision: st.revision,
            layer: st.active_layer,
            selected_path: app.ui.selected_path.clone(),
            vector_mask: app.ui.vector_mask_target,
            mode: app.ui.tool_options.vector_mode.clone(),
            tool: app.active_tool(),
        })
    }

    pub(super) fn valid(&self, app: &PhotocraftApp) -> bool {
        Self::capture(app).is_some_and(|now| {
            self.doc == now.doc
                && self.document.ptr_eq(&now.document)
                && self.revision == now.revision
                && self.layer == now.layer
                && self.selected_path == now.selected_path
                && self.vector_mask == now.vector_mask
                && self.mode == now.mode
                && self.tool == now.tool
        })
    }
}

use photocraft_doc::Document;
