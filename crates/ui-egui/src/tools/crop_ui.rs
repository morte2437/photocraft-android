//! Crop tool gestures. In Photoshop's "Classic Mode" (Use Classic Mode, P) the frame moves and
//! turns over a fixed image, as described below; in its default mode (`crop_mode`) the box stays
//! upright on screen and the same gestures inside and outside it move and turn the image behind
//! it instead (the view turns with the frame, so hit-testing in document space is unchanged):
//!
//! - with no frame, or inside the untouched default frame, a drag draws a new frame; ⇧ makes it
//!   square, ⌥ draws it from the centre, and holding Space while drawing repositions the frame
//!   being drawn;
//! - inside the frame, a drag moves it;
//! - on an edge or corner, a drag resizes it along the frame's own (possibly turned) axes; ⇧ keeps
//!   the frame's aspect ratio, ⌥ resizes about the centre, and an options-bar ratio preset always
//!   holds;
//! - outside the frame (off its handles) the pointer is the rotate cursor and a drag turns the
//!   frame about its centre, ⇧ in 15° steps, with the angle beside the pointer (#1792). As in
//!   Photoshop, a new frame is drawn after the pending one is committed or cancelled;
//! - while a frame is drawn or resized, its W × H (in px, as ↵ would crop it) shows beside the
//!   pointer, like the marquee's readout ([`sizing`]);
//! - with a pending frame and no gesture, the arrow keys nudge it 1 document px (⇧ 10 px) instead
//!   of the selection or layer, and X swaps it between portrait and landscape about its centre
//!   (keeping its angle, and swapping an options-bar ratio with it) instead of the colours
//!   ([`keys`], #1919). In the default mode the arrows move the image under the box instead.
//!
//! ↵ commits (`image.crop`, see `canvas::commit_crop`, with the turn as its `angle`: the document
//! is rotated so the frame is upright, then cropped to it) and Esc cancels. The pending frame lives
//! in `UiState::crop_rect` (the frame before its turn) and `UiState::crop_angle` (degrees,
//! clockwise on screen), so the control channel reads both. Both modes commit the same crop.
//!
//! As in Photoshop, from the first press until the crop is committed or cancelled the canvas also
//! shows the layers' pixels past its edges (kept by a crop with Delete Cropped Pixels off, or moved
//! out), with transparency out to the frame, under the shield ([`shows_beyond_canvas`]).

use egui::{CursorIcon, Modifiers};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// How far (screen points) from an edge the pointer still grabs it.
const HANDLE_PX: f64 = 8.0;
/// Smallest frame (document px) a gesture may leave behind.
const MIN_FRAME: f64 = 1.0;

/// Crop tool pointer state (not serialized: it only exists during a gesture).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CropState {
    pub drag: Option<CropDrag>,
    /// Space is held: drawing a new frame moves it instead of sizing it.
    pub space: bool,
    /// `UiState::crop_rect` is the untouched default frame ([`ensure_frame`]): a drag inside it
    /// draws a new frame (as in Photoshop) instead of moving it, and committing it does nothing.
    pub default_frame: bool,
    /// The document (index and size) the default frame was made for.
    frame_for: Option<(usize, u32, u32)>,
    /// The frame is being edited (pressed since it was made): the canvas shows what lies past it.
    pub editing: bool,
    /// The document the pending frame (or gesture) belongs to ([`cancel_stale`], #1918).
    doc: Option<photocraft_doc::DocId>,
    /// The options-bar Straighten button and ⌘-drag (`crop_straighten`).
    pub straighten: crate::crop_straighten::Straighten,
    /// The box's own Undo/Redo while a crop is pending (Photoshop keeps Edit › Undo live during a
    /// crop, so it must step the box, not the document — #3145): frame snapshots.
    undo: Vec<CropStep>,
    redo: Vec<CropStep>,
    /// The frame at the current gesture's press; pushed to `undo` on release when the box changed.
    press_step: Option<CropStep>,
}

/// One frame state of the crop box's own Undo/Redo.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CropStep {
    rect: Option<[f64; 4]>,
    angle: f64,
    default_frame: bool,
}

/// A crop gesture in progress. Rects are `[x0, y0, x1, y1]` in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropDrag {
    /// Drawing a new frame from `anchor`; `prev` is the frame it replaces, restored when the new
    /// one is too small (a click).
    Draw { anchor: [f64; 2], cur: [f64; 2], last: [f64; 2], prev: Option<[f64; 4]> },
    /// Moving the frame `rect` grabbed at `start`.
    Move { start: [f64; 2], rect: [f64; 4] },
    /// Dragging an edge or corner: `hx`/`hy` are -1 (left/top), 1 (right/bottom) or 0 (untouched).
    /// `view` is the view at the press when Auto Center Preview keeps the box's centre in place
    /// while it is resized (default mode, `crop_mode::live_resize`), else `None`.
    Resize { hx: i8, hy: i8, start: [f64; 2], rect: [f64; 4], view: Option<crate::crop_mode::PressView> },
    /// Turning the frame about `center`: the pointer's direction from it at the press (radians)
    /// and the frame's angle then (degrees).
    Rotate { center: [f64; 2], grab: f64, from: f64 },
    /// Default mode, inside the box: the image moves under it, so the document point grabbed at
    /// `start` stays under the pointer. `rect` is the frame and `view` the view at the press;
    /// pointer points are read in that view (`crop_mode::to_press`).
    MoveImage { start: [f64; 2], rect: [f64; 4], view: Option<crate::crop_mode::PressView> },
    /// Default mode, outside the box: the image turns about the frame's `center` (which stays put
    /// on screen). `grab` is the pointer's direction from it at the press (radians, in the press
    /// `view`), `from` the frame's angle then (degrees).
    TurnImage { center: [f64; 2], grab: f64, from: f64, view: Option<crate::crop_mode::PressView> },
}

/// What the pointer is over, relative to a crop frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Handle(i8, i8),
    Inside,
    Outside,
}

/// Hit-test a frame with `tol` document px of reach (shrunk for tiny frames so they stay movable).
pub fn hit(r: [f64; 4], p: [f64; 2], tol: f64) -> Hit {
    let axis = |lo: f64, hi: f64, v: f64, other_in: bool| -> i8 {
        let t = tol.min((hi - lo) / 4.0).max(0.0);
        // Outside the frame the full reach applies; inside it, the shrunk one.
        let near = |e: f64, outward: f64| (v - e) * outward <= tol && (e - v) * outward <= t;
        match (other_in, near(lo, -1.0), near(hi, 1.0)) {
            (false, ..) => 0,
            (_, true, true) => {
                if (v - lo).abs() <= (v - hi).abs() {
                    -1
                } else {
                    1
                }
            }
            (_, true, false) => -1,
            (_, false, true) => 1,
            _ => 0,
        }
    };
    let in_x = p[0] >= r[0] - tol && p[0] <= r[2] + tol;
    let in_y = p[1] >= r[1] - tol && p[1] <= r[3] + tol;
    let (hx, hy) = (axis(r[0], r[2], p[0], in_y), axis(r[1], r[3], p[1], in_x));
    if hx != 0 || hy != 0 {
        Hit::Handle(hx, hy)
    } else if p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3] {
        Hit::Inside
    } else {
        Hit::Outside
    }
}

/// The centre of `[x0, y0, x1, y1]`.
pub fn center(r: [f64; 4]) -> [f64; 2] {
    [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
}

/// `p` turned `deg` degrees clockwise on screen (y down) about `c`.
pub fn turn(p: [f64; 2], c: [f64; 2], deg: f64) -> [f64; 2] {
    let (s, k) = deg.to_radians().sin_cos();
    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
    [c[0] + k * dx - s * dy, c[1] + s * dx + k * dy]
}

/// The corners of frame `r` turned `deg` degrees about its centre: top-left, top-right,
/// bottom-right, bottom-left (of the upright frame).
pub fn corners(r: [f64; 4], deg: f64) -> [[f64; 2]; 4] {
    let c = center(r);
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map(|p| turn(p, c, deg))
}

/// The axis-aligned bounds `[x0, y0, x1, y1]` of frame `r` turned `deg` degrees.
pub fn turned_bounds(r: [f64; 4], deg: f64) -> [f64; 4] {
    if deg == 0.0 {
        return r;
    }
    corners(r, deg)
        .iter()
        .fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// [`hit`] for frame `r` turned `deg` degrees about its centre: handles and inside are found in
/// the frame's own axes.
pub fn hit_turned(r: [f64; 4], deg: f64, p: [f64; 2], tol: f64) -> Hit {
    if deg == 0.0 { hit(r, p, tol) } else { hit(r, turn(p, center(r), -deg), tol) }
}

/// An angle in degrees brought into (-180, 180]; 0 for anything not finite.
pub fn normalized_angle(deg: f64) -> f64 {
    if !deg.is_finite() {
        return 0.0;
    }
    let a = deg.rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

/// The frame's angle after turning it from `from` degrees by the pointer's sweep about `c` from
/// direction `grab` (radians) to `p`; ⇧ snaps it to 15° steps, as in Photoshop.
pub fn turned_angle(c: [f64; 2], grab: f64, from: f64, p: [f64; 2], snap: bool) -> f64 {
    let now = (p[1] - c[1]).atan2(p[0] - c[0]);
    let a = from + (now - grab).to_degrees();
    normalized_angle(if snap { (a / 15.0).round() * 15.0 } else { a })
}

/// [`resized`] for frame `r` turned `deg` degrees: the handle moves along the frame's own axes by
/// the document-space drag `d`, and the edges opposite stay where they are on the page.
pub fn resized_turned(r: [f64; 4], deg: f64, hx: i8, hy: i8, d: [f64; 2], ratio: Option<f64>, alt: bool) -> [f64; 4] {
    if deg == 0.0 {
        return resized(r, hx, hy, d, ratio, alt);
    }
    let c = center(r);
    // In the frame's axes (about its old centre), then the new centre back on the page.
    let n = resized(r, hx, hy, turn(d, [0.0, 0.0], -deg), ratio, alt);
    let nc = turn(center(n), c, deg);
    let (hw, hh) = ((n[2] - n[0]) / 2.0, (n[3] - n[1]) / 2.0);
    [nc[0] - hw, nc[1] - hh, nc[0] + hw, nc[1] + hh]
}

/// `[x0, y0, x1, y1]` with x0 ≤ x1, y0 ≤ y1.
fn normalized(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

fn size_ok(r: [f64; 4]) -> bool {
    r[2] - r[0] >= MIN_FRAME && r[3] - r[1] >= MIN_FRAME
}

/// The options-bar ratio preset as width / height (W : H in W x H x Resolution mode).
pub(crate) fn preset_ratio(app: &PhotocraftApp) -> Option<f64> {
    if app.ui.tool_options.crop_ratio == crate::crop_size::WHR {
        let dpi = app.session.active().map_or(72.0, |s| f64::from(s.doc.resolution_dpi));
        let pt_per_in = app.session.prefs().units_and_rulers.point_size.per_inch();
        return crate::crop_size::ratio(&app.ui.tool_options, dpi, pt_per_in);
    }
    let size = app.session.active().map_or((1.0, 1.0), |s| (s.doc.size.width as f64, s.doc.size.height as f64));
    crate::chrome_ui::crop_ratio(&app.ui.tool_options.crop_ratio, size.0, size.1).map(|(w, h)| w / h).filter(|k| k.is_finite() && *k > 0.0)
}

/// Largest centred rectangle of `ratio` contained in the current frame; never adds area.
fn fitted_ratio(r: [f64; 4], ratio: f64) -> Option<[f64; 4]> {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if !r.iter().all(|v| v.is_finite()) || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 || !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    if ((w / h) / ratio - 1.0).abs() < 1e-12 {
        return Some(r);
    }
    let c = [r[0] + w / 2.0, r[1] + h / 2.0];
    let (w, h) = if w / h > ratio { (h * ratio, h) } else { (w, w / ratio) };
    let fitted = [c[0] - w / 2.0, c[1] - h / 2.0, c[0] + w / 2.0, c[1] + h / 2.0];
    (fitted.iter().all(|v| v.is_finite()) && fitted[2] > fitted[0] && fitted[3] > fitted[1]).then_some(fitted)
}

/// Apply committed options to the preview now, before a resize or confirmation is needed.
pub(crate) fn apply_ratio(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Crop || app.crop.drag.is_some() {
        return;
    }
    frame(app);
    fit_frame(app);
    crate::crop_mode::sync_frame(app);
}

/// [`apply_ratio`] for a ratio being typed: fits from `base`, the frame the edit started with,
/// not from the frame the last keystroke's ratio left (#7752).
pub(crate) fn apply_ratio_from(app: &mut PhotocraftApp, base: [f64; 4]) {
    if app.ui.tool != Tool::Crop || app.crop.drag.is_some() {
        return;
    }
    if let Some(ratio) = preset_ratio(app)
        && let Some(fitted) = fitted_ratio(base, ratio)
        && app.ui.crop_rect != Some(fitted)
    {
        keyed(app, fitted);
    }
    crate::crop_mode::sync_frame(app);
}

fn fit_frame(app: &mut PhotocraftApp) {
    let dpi = app.session.active().map_or(72.0, |s| f64::from(s.doc.resolution_dpi));
    let pt_per_in = app.session.prefs().units_and_rulers.point_size.per_inch();
    let sized = crate::crop_size::target(&app.ui.tool_options, dpi, pt_per_in).is_some();
    if let Some(r) = app.ui.crop_rect
        && let Some(ratio) = preset_ratio(app)
        && let Some(fitted) = fitted_ratio(r, ratio)
        // W x H x Resolution can change output size even when the frame already fits.
        && (fitted != r || sized)
    {
        // #6750: fitting the untouched default frame to a ratio is not an edit: the user only
        // picked the tool, so leaving it drops the frame instead of cropping to it, as with the
        // plain default frame. A W x H x Resolution output size is an explicit instruction and
        // stays a pending crop even on the default frame.
        if fitted != r && !sized && app.crop.default_frame {
            app.ui.crop_rect = Some(fitted);
        } else {
            keyed(app, fitted);
        }
    }
}

/// A new frame drawn from `anchor` to `cur`: ⇧ square, ⌥ from the centre, `ratio` held.
pub fn drawn(anchor: [f64; 2], cur: [f64; 2], ratio: Option<f64>, mods: Modifiers) -> [f64; 4] {
    let e = match ratio {
        Some(k) => crate::chrome_ui::marquee_end("fixedRatio", k, 1.0, false, anchor, cur),
        None => crate::chrome_ui::marquee_end("", 0.0, 0.0, mods.shift, anchor, cur),
    };
    if mods.alt { normalized([2.0 * anchor[0] - e[0], 2.0 * anchor[1] - e[1]], e) } else { normalized(anchor, e) }
}

/// Frame `r` with handle (`hx`, `hy`) dragged by `d`. `alt` resizes about the centre; `ratio`
/// (width / height) keeps the proportions.
pub fn resized(r: [f64; 4], hx: i8, hy: i8, d: [f64; 2], ratio: Option<f64>, alt: bool) -> [f64; 4] {
    // Per axis: the fixed point, the signed extent from it, and whether it is centred (the
    // extent is then a half-extent).
    let axis = |lo: f64, hi: f64, h: i8, d: f64| -> (f64, f64, bool) {
        let c = (lo + hi) / 2.0;
        match h {
            1 if alt => (c, hi + d - c, true),
            1 => (lo, hi + d - lo, false),
            -1 if alt => (c, lo + d - c, true),
            -1 => (hi, lo + d - hi, false),
            _ => (c, (hi - lo) / 2.0, true),
        }
    };
    let (fx, mut ex, cx) = axis(r[0], r[2], hx, d[0]);
    let (fy, mut ey, cy) = axis(r[1], r[3], hy, d[1]);
    if let Some(k) = ratio.filter(|k| k.is_finite() && *k > 0.0) {
        let full = |e: f64, c: bool| e.abs() * if c { 2.0 } else { 1.0 };
        let (w, h) = (full(ex, cx), full(ey, cy));
        let (w, h) = match (hx != 0, hy != 0) {
            // A corner: the larger relative extent wins.
            (true, true) => {
                let s = (w / k).max(h);
                (s * k, s)
            }
            (true, false) => (w, w / k),
            (false, true) => (h * k, h),
            (false, false) => (w, h),
        };
        let back = |e: f64, len: f64, c: bool| if e < 0.0 { -1.0 } else { 1.0 } * if c { len / 2.0 } else { len };
        ex = back(ex, w, cx);
        ey = back(ey, h, cy);
    }
    let span = |f: f64, e: f64, c: bool| if c { (f - e, f + e) } else { (f, f + e) };
    let ((ax, bx), (ay, by)) = (span(fx, ex, cx), span(fy, ey, cy));
    normalized([ax, ay], [bx, by])
}

/// Space is held (set by the canvas each frame, or by `ui.pointer`'s `space` flag).
pub fn set_space(app: &mut PhotocraftApp, down: bool) {
    app.crop.space = down;
}

/// As in Photoshop, the Crop tool always shows a frame: on picking the tool, and after a crop is
/// cancelled or committed, it frames the selection's bounds, or the whole canvas. In default mode
/// the view then follows the frame being edited (`crop_mode::sync`).
pub fn ensure_frame(app: &mut PhotocraftApp) {
    frame(app);
    crate::crop_mode::sync(app, None);
}

fn frame(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Crop {
        app.crop.editing = false;
        crate::crop_straighten::reset(app);
        // Leaving the tool drops an untouched default frame and, as in Photoshop, commits a
        // drawn one (#3368).
        if app.crop.default_frame {
            app.ui.crop_rect = None;
            app.crop.default_frame = false;
        }
        if app.ui.crop_rect.is_some() && !keeps_pending(app) {
            commit_on_leave(app);
        }
        if app.ui.crop_rect.is_none() {
            app.ui.crop_angle = 0.0;
        }
        return;
    }
    // No frame (cancelled or committed): the next one starts upright.
    if app.ui.crop_rect.is_none() && app.crop.drag.is_none() {
        app.ui.crop_angle = 0.0;
    }
    let key = app.session.active_index().zip(app.session.active()).map(|(i, st)| (i, st.doc.size.width, st.doc.size.height));
    // Another document (or a resized one) gets its own default frame.
    if app.crop.default_frame && app.crop.frame_for != key && app.crop.drag.is_none() {
        app.ui.crop_rect = None;
    }
    if app.ui.crop_rect.is_some() || app.crop.drag.is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let canvas = st.doc.bounds();
    // Computed once per new frame, not per frame drawn.
    let r = st.doc.selection.as_ref().map(|s| s.content_bounds().intersect(&canvas)).filter(|b| !b.is_empty()).unwrap_or(canvas);
    if r.is_empty() {
        return;
    }
    app.ui.crop_rect = Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    app.crop.default_frame = true;
    app.crop.frame_for = key;
    app.crop.editing = false;
    app.crop.undo.clear();
    app.crop.redo.clear();
    app.crop.press_step = None;
    claim(app);
    fit_frame(app);
}

/// Another tool is current but the drawn frame stays pending: a gesture on it is still in
/// progress, or the pen's eraser end stands in for the Crop tool until the tip comes back
/// (`stylus`). The held tools (Space, ⌘…, `hold_keys`) never change the current tool.
fn keeps_pending(app: &PhotocraftApp) -> bool {
    app.crop.drag.is_some() || app.stylus.tool_before_eraser == Some(Tool::Crop)
}

/// Commits the drawn frame on leaving the Crop tool as ↵ would with the tool still picked (so the
/// view goes upright too, `crop_mode::upright`), then gives the new tool back. A refused crop
/// keeps its frame: that is cancelled, so the new tool doesn't retry it every frame.
fn commit_on_leave(app: &mut PhotocraftApp) {
    let tool = std::mem::replace(&mut app.ui.tool, Tool::Crop);
    crate::canvas::commit_crop(app);
    if app.ui.crop_rect.is_some() {
        cancel(app);
    }
    app.ui.tool = tool;
}

/// A crop gesture is in progress: Space repositions the frame rather than panning.
pub fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.crop.drag.is_some()
}

/// Cancel the pending crop: the frame, any gesture on it and the default-frame state. With the
/// Crop tool still picked, [`ensure_frame`] then makes the active document a new default frame.
pub fn cancel(app: &mut PhotocraftApp) {
    crate::crop_mode::upright(app);
    app.ui.crop_rect = None;
    app.ui.crop_angle = 0.0;
    app.crop.drag = None;
    app.crop.default_frame = false;
    app.crop.frame_for = None;
    app.crop.editing = false;
    clear_history(app);
    crate::crop_straighten::reset(app);
}

/// Drop the box's own Undo/Redo (leaving the tool, cancelling or committing a crop).
pub fn clear_history(app: &mut PhotocraftApp) {
    app.crop.undo.clear();
    app.crop.redo.clear();
    app.crop.press_step = None;
}

/// The frame's current state, as an undo step.
fn step_of(app: &PhotocraftApp) -> CropStep {
    CropStep { rect: app.ui.crop_rect, angle: app.ui.crop_angle, default_frame: app.crop.default_frame }
}

/// Put a frame state back.
fn apply_step(app: &mut PhotocraftApp, s: CropStep) {
    app.ui.crop_rect = s.rect;
    app.ui.crop_angle = s.angle;
    app.crop.default_frame = s.default_frame;
}

/// The pending frame, its box Undo/Redo and the view's turn, kept while `image.crop` runs so a
/// refused commit gives them back ([`restore`]).
pub(crate) struct Kept {
    step: CropStep,
    undo: Vec<CropStep>,
    redo: Vec<CropStep>,
    view: Option<(f32, [f32; 2])>,
}

pub(crate) fn keep(app: &PhotocraftApp) -> Kept {
    let view = app.session.active_index().and_then(|i| app.ui.views.get(i)).map(|v| (v.rotation, v.center));
    Kept { step: step_of(app), undo: app.crop.undo.clone(), redo: app.crop.redo.clone(), view }
}

pub(crate) fn restore(app: &mut PhotocraftApp, k: Kept) {
    apply_step(app, k.step);
    (app.crop.undo, app.crop.redo) = (k.undo, k.redo);
    if let Some((rotation, center)) = k.view
        && let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i))
    {
        (v.rotation, v.center) = (rotation, center);
    }
}

/// While the Crop tool has box history, Undo/Redo step the *box*, not the document (Photoshop keeps
/// Edit › Undo live during a crop, #1918/#3145). `Some(enabled)` for those commands, else `None` so
/// the caller falls back to document history. While either box stack has a step, `intercept` takes
/// both commands, so one with nothing left to step greys out rather than offering the document's.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if app.ui.tool != Tool::Crop || (app.crop.undo.is_empty() && app.crop.redo.is_empty()) {
        return None;
    }
    match id {
        "edit.undo" => Some(!app.crop.undo.is_empty()),
        "edit.redo" => Some(!app.crop.redo.is_empty()),
        "edit.toggleLastState" => Some(false),
        _ => None,
    }
}

/// Intercept Undo/Redo/Toggle Last State while the Crop tool owns box history, for every caller
/// (menus, shortcuts, the control channel, MCP) — as `transform_tool::intercept` does.
pub fn intercept(app: &mut PhotocraftApp, id: &str) -> Option<Result<serde_json::Value, String>> {
    if app.ui.tool != Tool::Crop || (app.crop.undo.is_empty() && app.crop.redo.is_empty()) {
        return None;
    }
    match id {
        "edit.undo" => Some(Ok(step(app, false))),
        "edit.redo" => Some(Ok(step(app, true))),
        "edit.toggleLastState" => Some(Err("Commit or cancel the crop first".into())),
        _ => None,
    }
}

/// Undo (or redo) one box step.
fn step(app: &mut PhotocraftApp, redo: bool) -> serde_json::Value {
    let s = if redo { app.crop.redo.pop() } else { app.crop.undo.pop() };
    let Some(s) = s else { return serde_json::Value::Null };
    let now = step_of(app);
    apply_step(app, s);
    if redo {
        app.crop.undo.push(now);
    } else {
        app.crop.redo.push(now);
    }
    // Default mode: the box stays put on screen, so the view follows the frame's centre (#7088).
    if let (true, Some(a), Some(b)) = (crate::crop_mode::moves_image(app), now.rect, s.rect) {
        let (a, b) = (center(a), center(b));
        let d = [(b[0] - a[0]) as f32, (b[1] - a[1]) as f32];
        if let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
            let c = [v.center[0] + d[0], v.center[1] + d[1]];
            if c.iter().all(|x| x.is_finite()) {
                v.center = c;
            }
        }
    }
    crate::crop_mode::sync_frame(app);
    crate::crop_mode::auto_center(app);
    serde_json::Value::Null
}

/// The pending frame belongs to the active document.
fn claim(app: &mut PhotocraftApp) {
    app.crop.doc = app.session.active().map(|st| st.doc.id);
}

/// A crop is pending: the Crop tool's frame on the active document has been set (drawn, moved,
/// resized or turned), not just the untouched default frame the tool shows.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop
        && app.ui.crop_rect.is_some()
        && !app.crop.default_frame
        && (app.crop.doc.is_none() || app.crop.doc == app.session.active().map(|st| st.doc.id))
}

/// Menu commands Photoshop greys out while a crop is pending (#1918), until it is committed or
/// cancelled. Checked item by item in Photoshop 27.11: everything except Close, Save, Save As,
/// Save a Copy; Undo, Redo, Toggle Last State, Search; Image › Crop (which commits the frame); the
/// View menu but Proof Setup, Pixel Aspect Ratio (Correction), 32-bit Preview Options, Flip
/// Horizontal and Screen Mode; the Window menu but Workspace and Adjustments; Help but System Info.
pub fn blocks(app: &PhotocraftApp, id: &str) -> bool {
    pending(app) && !available_while_pending(id)
}

fn available_while_pending(id: &str) -> bool {
    if let Some(v) = id.strip_prefix("view.") {
        let greyed = ["proofSetup.", "pixelAspectRatio", "thirtyTwoBitPreviewOptions", "flipHorizontal", "screenMode."];
        return !greyed.iter().any(|g| v.starts_with(g));
    }
    if let Some(w) = id.strip_prefix("window.") {
        return !(w.starts_with("workspace.") || w == "panel.adjustments");
    }
    if id.starts_with("help.") {
        return id != "help.systemInfo";
    }
    matches!(
        id,
        "file.close"
            | "file.save"
            | "file.saveAs"
            | "file.saveACopy"
            | "file.exit"
            | "edit.undo"
            | "edit.redo"
            | "edit.toggleLastState"
            | "edit.search"
            | "image.crop"
    )
}

/// A pending crop belongs to the document it was drawn on (#1918): when another document becomes
/// active (File › New, Open, a tab switch, closing the document) it is cancelled, so it never
/// shows on or crops another document. (As in Photoshop: switching to another document discards
/// the pending crop, and that document gets the tool's own frame.) Run on every frame and command
/// (`sync_views`) and before a commit.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let active = app.session.active().map(|st| st.doc.id);
    let pending = app.ui.crop_rect.is_some() || app.crop.drag.is_some();
    // A frame made before any document was claimed (control channel, tests) is the active one's.
    if pending && app.crop.doc.is_some() && app.crop.doc != active {
        cancel(app);
    }
    app.crop.doc = active;
}

/// The frame is being edited, so the canvas shows the pixels past its edges (Photoshop's crop
/// preview): from the first press with the tool until the crop is committed or cancelled.
pub fn shows_beyond_canvas(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && (app.crop.editing || app.crop.drag.is_some())
}

/// The button went down on the canvas with the Crop tool: the frame is being edited from now on.
/// Returns true when that is new.
pub fn press(app: &mut PhotocraftApp) -> bool {
    let new = app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && !app.crop.editing;
    if new {
        app.crop.editing = true;
    }
    new
}

pub(crate) fn tolerance(app: &PhotocraftApp) -> f64 {
    HANDLE_PX / (app.point_zoom() as f64).max(0.01)
}

/// The pending frame's angle (degrees), 0 when it isn't a finite number.
pub fn angle(app: &PhotocraftApp) -> f64 {
    normalized_angle(app.ui.crop_angle)
}

/// A press at document point `p` would turn the frame (Crop tool, outside the frame and its
/// handles) rather than draw, move or resize one.
pub fn turns_at(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.tool == Tool::Crop
        && app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite())).is_some_and(|r| hit_turned(r, angle(app), p, tolerance(app)) == Hit::Outside)
}

/// A double-click at document point `p` commits the crop (Photoshop: double-click inside the box,
/// like ↵): Crop tool, a pending frame, and `p` inside it (not on a handle, not outside).
pub fn commits_at(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.tool == Tool::Crop
        && app.crop.drag.is_none()
        && app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite())).is_some_and(|r| hit_turned(r, angle(app), p, tolerance(app)) == Hit::Inside)
}

/// The frame is being turned: the angle for the readout beside the pointer. Classic Mode shows
/// the frame's turn; the default mode the image's (clockwise positive, as Photoshop 25.1 does).
pub fn turning(app: &PhotocraftApp) -> Option<f64> {
    match app.crop.drag {
        Some(CropDrag::Rotate { .. }) => Some(angle(app)),
        Some(CropDrag::TurnImage { .. }) => Some(crate::crop_mode::image_turn(angle(app))),
        _ => None,
    }
}

/// The size ↵ would crop frame `r` to: its own width and height (before any turn) in whole
/// document px, at least 1; `None` when they aren't finite.
pub fn frame_size(r: [f64; 4]) -> Option<[f64; 2]> {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    (w.is_finite() && h.is_finite()).then(|| [w.round().max(1.0), h.round().max(1.0)])
}

/// The frame is being drawn or resized: its W × H, for the readout beside the pointer (as the
/// marquee's, in px).
pub fn sizing(app: &PhotocraftApp) -> Option<[f64; 2]> {
    match app.crop.drag {
        // Until the pointer moves, the frame shown is still the previous one.
        Some(CropDrag::Draw { anchor, cur, .. }) if anchor != cur => {}
        Some(CropDrag::Resize { .. }) => {}
        _ => return None,
    }
    frame_size(app.ui.crop_rect?)
}

/// The Crop tool owns the arrow keys and X: a pending frame and no gesture in progress.
pub fn takes_keys(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.crop.drag.is_none() && app.ui.transform.is_none() && app.ui.crop_rect.is_some_and(|r| r.iter().all(|v| v.is_finite()))
}

/// The frame now set by the keys: a real frame (no longer the default), being edited.
fn keyed(app: &mut PhotocraftApp, r: [f64; 4]) {
    if r.iter().all(|v| v.is_finite()) {
        app.ui.crop_rect = Some(r);
        app.crop.default_frame = false;
        app.crop.editing = true;
    }
}

/// Moves the pending frame by (`dx`, `dy`) document px (a turned frame moves on the page, keeping
/// its angle); in default mode the image moves that way on screen under the box instead. Returns
/// false when the Crop tool doesn't take the keys ([`takes_keys`]).
pub fn nudge(app: &mut PhotocraftApp, dx: f64, dy: f64) -> bool {
    let Some(r) = app.ui.crop_rect.filter(|_| takes_keys(app)) else { return false };
    if crate::crop_mode::moves_image(app) {
        if dx.is_finite() && dy.is_finite() {
            app.crop.default_frame = false;
            app.crop.editing = true;
            crate::crop_mode::sync_frame(app);
            let d = crate::crop_mode::screen_to_doc_vec(app, [dx, dy]);
            crate::crop_mode::shift_image(app, d);
        }
        return true;
    }
    if dx.is_finite() && dy.is_finite() {
        keyed(app, [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy]);
    }
    true
}

/// Frame `r` with its width and height swapped about its centre.
pub fn swapped(r: [f64; 4]) -> [f64; 4] {
    let c = center(r);
    let (hw, hh) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
    [c[0] - hh, c[1] - hw, c[0] + hh, c[1] + hw]
}

/// The options-bar ratio `key` with its sides swapped (what the bar's ⇄ button does), `None` when
/// no ratio is set; "original" becomes the document's size turned on its side.
pub fn swapped_ratio(key: &str, doc_w: f64, doc_h: f64) -> Option<String> {
    let (w, h) = crate::chrome_ui::crop_ratio(key, doc_w, doc_h).filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)?;
    Some(format!("{}:{}", crate::crop_size::fmt_ratio(h), crate::crop_size::fmt_ratio(w)))
}

/// X: swaps the pending frame between portrait and landscape about its centre, keeping its angle,
/// and swaps the options-bar ratio with it, as in Photoshop. Returns false when the Crop tool
/// doesn't take the keys ([`takes_keys`]), so X swaps the colours as usual.
pub fn swap_orientation(app: &mut PhotocraftApp) -> bool {
    let Some(r) = app.ui.crop_rect.filter(|_| takes_keys(app)) else { return false };
    let size = app.session.active().map_or((0.0, 0.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
    // W x H x Resolution swaps W and H instead of a ratio.
    if !crate::crop_size::swap(&mut app.ui.tool_options)
        && let Some(k) = swapped_ratio(&app.ui.tool_options.crop_ratio, size.0, size.1)
    {
        app.ui.tool_options.crop_ratio = k;
    }
    if preset_ratio(app).is_some() {
        apply_ratio(app);
    } else {
        keyed(app, swapped(r));
        crate::crop_mode::sync_frame(app);
        crate::crop_mode::auto_center(app);
    }
    true
}

/// Crop tool keys with a pending frame: arrows nudge it 1 px (⇧ 10 px) instead of the selection
/// or layer, and X swaps its orientation instead of the colours. Returns true when a key was used.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if !takes_keys(app) {
        return false;
    }
    use egui::Key;
    let arrows = [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)];
    for (key, ux, uy) in arrows {
        // ⇧ first: `consume_key` ignores an extra ⇧.
        for mods in [Modifiers::SHIFT, Modifiers::NONE] {
            if ctx.input_mut(|i| i.consume_key(mods, key)) {
                let k = if mods.shift { 10.0 } else { 1.0 };
                return nudge(app, ux * k, uy * k);
            }
        }
    }
    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::X)) && swap_orientation(app)
}

/// Pointer input for the Crop tool. Returns true when the event was its.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if app.ui.tool != Tool::Crop {
        app.crop.drag = None;
        return false;
    }
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if app.session.active().is_none() || !p[0].is_finite() || !p[1].is_finite() {
        return true;
    }
    // The Straighten button, or ⌘ held: the drag draws a line to level the frame on.
    if crate::crop_straighten::pointer(app, ev, mods) {
        claim(app);
        // Default mode: the image turns so the line ends up level (the press keeps its point).
        // The turn is one box Undo step, like any other frame gesture.
        match ev {
            ToolEvent::Down { .. } => {
                app.crop.press_step = Some(step_of(app));
                crate::crop_mode::sync(app, Some(p));
            }
            ToolEvent::Move { .. } => crate::crop_mode::sync_frame(app),
            ToolEvent::Up { .. } => {
                crate::crop_mode::sync_frame(app);
                push_press_step(app);
            }
        }
        return true;
    }
    match ev {
        ToolEvent::Down { .. } => {
            claim(app);
            app.crop.editing = true;
            app.crop.press_step = Some(step_of(app));
            // Default mode: the view is upright for the frame (picking the tool already made it so;
            // this catches a frame set over the control channel), about the pressed point.
            crate::crop_mode::sync(app, Some(p));
            let frame = app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()));
            let deg = angle(app);
            let image = crate::crop_mode::moves_image(app);
            app.crop.drag = Some(match frame.map(|r| (r, hit_turned(r, deg, p, tolerance(app)))) {
                Some((rect, Hit::Handle(hx, hy))) => {
                    let view = if crate::crop_mode::live_center(app) { crate::crop_mode::press_view(app) } else { None };
                    CropDrag::Resize { hx, hy, start: p, rect, view }
                }
                // Default mode: inside the box the image moves, the box staying where it is on
                // screen (Photoshop doesn't re-centre it). Inside the untouched frame a drag still
                // draws a new box.
                Some((rect, Hit::Inside)) if !app.crop.default_frame && image => {
                    CropDrag::MoveImage { start: p, rect, view: crate::crop_mode::press_view(app) }
                }
                Some((rect, Hit::Inside)) if !app.crop.default_frame => CropDrag::Move { start: p, rect },
                Some((rect, Hit::Outside)) if image => {
                    let c = center(rect);
                    CropDrag::TurnImage { center: c, grab: (p[1] - c[1]).atan2(p[0] - c[0]), from: deg, view: crate::crop_mode::press_view(app) }
                }
                // Outside the frame a drag turns it, as in Photoshop (#1792).
                Some((rect, Hit::Outside)) => {
                    let c = center(rect);
                    CropDrag::Rotate { center: c, grab: (p[1] - c[1]).atan2(p[0] - c[0]), from: deg }
                }
                _ => {
                    app.ui.crop_angle = 0.0;
                    CropDrag::Draw { anchor: p, cur: p, last: p, prev: app.ui.crop_rect }
                }
            });
        }
        ToolEvent::Move { .. } => update(app, p, mods),
        ToolEvent::Up { .. } => {
            update(app, p, mods);
            let Some(drag) = app.crop.drag.take() else { return true };
            let ok = app.ui.crop_rect.is_some_and(size_ok);
            // A click outside the untouched default frame (no turn) leaves it the default.
            let changed = !matches!(drag, CropDrag::Rotate { from, .. } | CropDrag::TurnImage { from, .. } if from == angle(app))
                && !matches!(drag, CropDrag::MoveImage { rect, .. } if app.ui.crop_rect == Some(rect));
            if ok && changed {
                app.crop.default_frame = false;
            }
            match drag {
                CropDrag::Draw { prev, .. } if !ok => app.ui.crop_rect = prev,
                CropDrag::Move { rect, .. } | CropDrag::Resize { rect, view: None, .. } if !ok => app.ui.crop_rect = Some(rect),
                // A live-centred resize collapsed: the frame and the view it moved go back.
                CropDrag::Resize { rect, view: Some(view), .. } if !ok => crate::crop_mode::offset_image(app, rect, Some(view), [0.0, 0.0]),
                // Auto Center Preview: a box drawn in default mode moves to the middle. (A resized
                // one stays where its centre was: `crop_mode::live_resize`.)
                CropDrag::Draw { .. } if ok => {
                    crate::crop_mode::sync_frame(app);
                    crate::crop_mode::auto_center(app);
                }
                _ => {}
            }
            push_press_step(app);
        }
    }
    crate::crop_mode::sync_frame(app);
    true
}

/// On release: the frame state at the press becomes a box Undo step when the gesture changed it.
fn push_press_step(app: &mut PhotocraftApp) {
    if let Some(before) = app.crop.press_step.take()
        && step_of(app) != before
    {
        app.crop.undo.push(before);
        app.crop.redo.clear();
    }
}

/// The frame follows the pointer at `p`.
fn update(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) {
    let ratio = preset_ratio(app);
    let space = app.crop.space;
    let deg = angle(app);
    match app.crop.drag {
        Some(CropDrag::Rotate { center, grab, from }) => {
            app.ui.crop_angle = turned_angle(center, grab, from, p, mods.shift);
            return;
        }
        Some(CropDrag::TurnImage { center, grab, from, view }) => {
            // The image turns with the pointer's sweep on screen, so the frame turns against it.
            let q = crate::crop_mode::to_press(app, view, p);
            let a = from - ((q[1] - center[1]).atan2(q[0] - center[0]) - grab).to_degrees();
            if a.is_finite() {
                app.ui.crop_angle = normalized_angle(if mods.shift { (a / 15.0).round() * 15.0 } else { a });
                crate::crop_mode::sync_frame(app);
            }
            return;
        }
        Some(CropDrag::MoveImage { start, rect, view }) => {
            // The image follows the pointer: the frame moves the other way on it, and the view
            // with the frame, so the box stays put.
            let q = crate::crop_mode::to_press(app, view, p);
            crate::crop_mode::offset_image(app, rect, view, [q[0] - start[0], q[1] - start[1]]);
            return;
        }
        Some(CropDrag::Resize { hx, hy, start, rect, view: Some(view) }) => {
            // Auto Center Preview: the box stays centred where it was while the handle follows
            // the pointer.
            let q = crate::crop_mode::to_press(app, Some(view), p);
            let ratio = resize_ratio(app, rect, mods);
            crate::crop_mode::live_resize(app, crate::crop_mode::LiveResize { rect, deg, hx, hy, start, q, ratio, alt: mods.alt }, view);
            return;
        }
        _ => {}
    }
    let Some(drag) = app.crop.drag.as_mut() else { return };
    let rect = match drag {
        CropDrag::Draw { anchor, cur, last, .. } => {
            if space {
                // Space: the frame being drawn follows the pointer, keeping its size.
                let d = [p[0] - last[0], p[1] - last[1]];
                *anchor = [anchor[0] + d[0], anchor[1] + d[1]];
                *cur = [cur[0] + d[0], cur[1] + d[1]];
            } else {
                *cur = p;
            }
            *last = p;
            if *anchor == *cur {
                return;
            }
            drawn(*anchor, *cur, ratio, mods)
        }
        CropDrag::Move { start, rect } => {
            let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
            [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
        }
        CropDrag::Resize { hx, hy, start, rect, .. } => {
            let keep = || {
                let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
                (w > 0.0 && h > 0.0).then(|| w / h)
            };
            let ratio = ratio.or_else(|| if mods.shift { keep() } else { None });
            resized_turned(*rect, deg, *hx, *hy, [p[0] - start[0], p[1] - start[1]], ratio, mods.alt)
        }
        CropDrag::Rotate { .. } | CropDrag::TurnImage { .. } | CropDrag::MoveImage { .. } => return,
    };
    app.ui.crop_rect = Some(rect);
}

/// Geometry and snapping must use the same ratio so constrained handles stay aligned.
pub(crate) fn resize_ratio(app: &PhotocraftApp, rect: [f64; 4], mods: Modifiers) -> Option<f64> {
    let keep = (rect[2] - rect[0] > 0.0 && rect[3] - rect[1] > 0.0).then(|| (rect[2] - rect[0]) / (rect[3] - rect[1]));
    preset_ratio(app).or(if mods.shift { keep } else { None })
}

/// Cursor over the canvas at document point `p`: resize arrows on the edges, move inside (the
/// crosshair inside the untouched default frame, where a drag draws a new one).
pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<CursorIcon> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    if crate::crop_straighten::mode(app) {
        return Some(CursorIcon::Crosshair);
    }
    let deg = angle(app);
    let h = match app.crop.drag {
        Some(CropDrag::Move { .. } | CropDrag::MoveImage { .. }) => Hit::Inside,
        Some(CropDrag::Resize { hx, hy, .. }) => Hit::Handle(hx, hy),
        Some(CropDrag::Draw { .. }) => return Some(CursorIcon::Crosshair),
        Some(CropDrag::Rotate { .. } | CropDrag::TurnImage { .. }) => Hit::Outside,
        None => hit_turned(app.ui.crop_rect?, deg, p, tolerance(app)),
    };
    Some(match h {
        // The arrow as the handle faces on screen: the frame's turn plus the view's, mirrored when
        // the view is flipped.
        Hit::Handle(hx, hy) => resize_icon(hx, hy, deg + f64::from(crate::crop_mode::view_rotation(app)), app.ui.view.flip_horizontal),
        // Inside the untouched default frame a drag draws a new one (`pointer`), so nothing moves (#4461).
        Hit::Inside if app.crop.default_frame => CursorIcon::Crosshair,
        Hit::Inside => CursorIcon::Move,
        // Photoshop's curved two-headed arrow (`draw_turn_cursor`, from `turn_cursor_dir`).
        Hit::Outside => CursorIcon::None,
    })
}

/// Where the turn cursor's arc bends at document point `p` (pointer outside the frame, or turning
/// it): towards the frame corner of `p`'s quadrant, along the diagonal of the frame's own axes.
/// As in Photoshop that gives four cursors, one per corner; they turn with the frame.
pub fn turn_cursor_dir(app: &PhotocraftApp, p: [f64; 2]) -> Option<[f64; 2]> {
    if app.ui.tool != Tool::Crop || crate::crop_straighten::mode(app) {
        return None;
    }
    let r = app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()))?;
    let deg = angle(app);
    let turning = match app.crop.drag {
        Some(CropDrag::Rotate { .. } | CropDrag::TurnImage { .. }) => true,
        Some(_) => false,
        None => hit_turned(r, deg, p, tolerance(app)) == Hit::Outside,
    };
    if !(turning && p[0].is_finite() && p[1].is_finite()) {
        return None;
    }
    let c = center(r);
    let local = turn(p, c, -deg);
    let sx = if local[0] < c[0] { 1.0 } else { -1.0 };
    let sy = if local[1] < c[1] { 1.0 } else { -1.0 };
    Some(turn([sx * std::f64::consts::FRAC_1_SQRT_2, sy * std::f64::consts::FRAC_1_SQRT_2], [0.0, 0.0], deg))
}

/// Photoshop's turn cursor (Crop, and Free Transform's rotate zone) at screen point `p`: a short
/// arc with an arrowhead at each end, curving round the frame corner that lies in screen direction
/// `toward` (an original drawing, no proprietary cursor asset).
pub fn draw_turn_cursor(ctx: &egui::Context, p: egui::Pos2, toward: egui::Vec2) {
    let len = toward.length();
    if !len.is_finite() || len < 1e-6 {
        return;
    }
    let u = toward / len;
    let tokens = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("crop-turn-cursor")));
    const R: f32 = 12.0;
    const HALF: f32 = 0.75;
    let c = p + u * R;
    let mid = (-u.y).atan2(-u.x);
    let at = |a: f32| c + egui::vec2(a.cos(), a.sin()) * R;
    let points: Vec<egui::Pos2> = (0..=16).map(|i| at(mid - HALF + 2.0 * HALF * i as f32 / 16.0)).collect();
    painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(3.5, tokens.shadow)));
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, tokens.accent_text)));
    for (a, sign) in [(mid - HALF, -1.0f32), (mid + HALF, 1.0)] {
        let tip = at(a) + egui::vec2(-a.sin(), a.cos()) * (sign * 2.0);
        let tangent = egui::vec2(-a.sin(), a.cos()) * sign;
        let normal = egui::vec2(a.cos(), a.sin());
        painter.add(egui::Shape::convex_polygon(
            vec![tip + tangent * 3.5, tip - tangent * 3.0 + normal * 3.5, tip - tangent * 3.0 - normal * 3.5],
            tokens.accent_text,
            egui::Stroke::new(1.0, tokens.shadow),
        ));
    }
}

/// The resize arrow for handle (`hx`, `hy`) of a frame turned `deg` degrees: the handle's outward
/// direction, turned with the frame and mirrored by a flipped view (`flip`), to the nearest of the
/// four arrows.
fn resize_icon(hx: i8, hy: i8, deg: f64, flip: bool) -> CursorIcon {
    let d = turn([f64::from(hx), f64::from(hy)], [0.0, 0.0], deg);
    // View › Flip Horizontal mirrors the screen after the turn.
    let d = if flip { [-d[0], d[1]] } else { d };
    let a = d[1].atan2(d[0]).rem_euclid(std::f64::consts::PI);
    match ((a / std::f64::consts::FRAC_PI_4).round() as i64).rem_euclid(4) {
        0 => CursorIcon::ResizeHorizontal,
        1 => CursorIcon::ResizeNwSe,
        2 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
    }
}

#[cfg(test)]
mod tests;
