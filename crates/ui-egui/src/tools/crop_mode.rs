//! The Crop tool's two modes (Photoshop's gear menu, "Set additional Crop options", #1919).
//!
//! - **Default mode** (Photoshop's default, Use Classic Mode off): the crop box stays upright on
//!   screen and the image turns behind it. A drag outside the box turns the image about the box's
//!   centre (⇧ in 15° steps; the readout beside the pointer is the image's turn, clockwise
//!   positive), a drag inside moves the image under the box (the box stays put; with View › Snap
//!   on, the upright frame's edges and centre snap as a Classic Mode frame's do, #8580), the arrow
//!   keys nudge the image, and the handles resize the box on screen as usual. With **Auto Center
//!   Preview** on (the default), a box being resized keeps its centre where it is on screen (also
//!   after a Space pan) while the handle follows the pointer, a drawn box moves to the middle of
//!   the canvas on release, and X re-centres it. A drag inside never re-centres the box
//!   (Photoshop 25.1).
//! - **Classic Mode** (P toggles it while the Crop tool shows a box): the box moves and turns over
//!   a fixed image (`crop_ui`). Show Cropped Area and Auto Center Preview are greyed out in it, and
//!   the area outside the box shows whatever Show Cropped Area says (Photoshop 25.1).
//!
//! The crop model is the same in both: `UiState::crop_rect` is the frame in document px and
//! `UiState::crop_angle` its turn, so ↵ commits the same `image.crop {…, angle}` (one undo step)
//! whichever mode drew it. Default mode only changes the view: while the Crop tool shows a frame,
//! the view's camera rotation is the frame's turn undone (`View::rotation = -crop_angle`), so the
//! box shows upright. In Classic Mode the camera is upright (rotation 0) and the frame shows with
//! its own turn on the image. As in Photoshop 25.1, picking the tool in either mode turns a Rotate
//! View (R) turn upright at once, about the view's centre (the frame stays on the same image
//! area), and the view stays upright after ↵ or Esc and when P switches modes (the R turn isn't
//! restored).
//!
//! Both options live in `ToolOptions::crop_shield` (`classic_mode`, `auto_center_preview`), which
//! `ui.inspect` reports and `ui.set {cropShield: {...}}` patches.

use egui::{Rect, vec2};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// Use Classic Mode is on: the box turns and moves over the image.
pub fn classic(app: &PhotocraftApp) -> bool {
    app.ui.tool_options.crop_shield.classic_mode
}

fn finite_frame(app: &PhotocraftApp) -> Option<[f64; 4]> {
    app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()))
}

/// The Crop tool drives the view: it shows a frame (either mode; [`sync`]).
pub fn turns_view(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && finite_frame(app).is_some() && app.session.active().is_some()
}

/// Gestures move and turn the image rather than the box (default mode with the Crop tool).
pub fn moves_image(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && !classic(app)
}

/// The view centre that keeps document point `c` where it is on screen when the camera rotation
/// goes from `from` to `to` degrees (zoom and flip unchanged). `center` itself when anything isn't
/// finite.
pub fn pivot_center(center: [f32; 2], c: [f64; 2], from: f32, to: f32, flip: bool) -> [f32; 2] {
    let a = ViewXform { rect: Rect::ZERO, zoom: 1.0, center, flip, rotation: from, aspect: 1.0 };
    // c's offset from the view centre on screen (at zoom 1), then the same offset in the new view.
    let off = a.to_screen(c[0] as f32, c[1] as f32).to_vec2();
    let u = ViewXform { rotation: to, ..a }.unmap_vec(off);
    let n = [c[0] as f32 - u.x, c[1] as f32 - u.y];
    if n.iter().all(|v| v.is_finite()) { n } else { center }
}

/// The active document's view rotation in degrees (0 without a view).
pub fn view_rotation(app: &PhotocraftApp) -> f32 {
    app.session.active_index().and_then(|i| app.ui.views.get(i)).map_or(0.0, |v| v.rotation)
}

/// Turns the active view so the frame shows upright while default mode drives it, pivoting about
/// document point `pivot` so that point stays put on screen (the view's own centre when `None`:
/// picking the tool, a commit or a cancel only straighten the view).
pub fn sync(app: &mut PhotocraftApp, pivot: Option<[f64; 2]>) {
    if !turns_view(app) {
        return;
    }
    let Some(i) = app.session.active_index() else { return };
    // Classic Mode: an upright camera, the frame turned on it. Default mode: the frame upright.
    let want = if classic(app) { 0.0 } else { crate::rotate_view::wrap_deg(-crate::crop_ui::angle(app) as f32) };
    let flip = app.ui.view.flip_horizontal;
    let Some(v) = app.ui.views.get_mut(i) else { return };
    if v.rotation != want {
        if let Some(p) = pivot.filter(|p| p.iter().all(|v| v.is_finite())) {
            v.center = pivot_center(v.center, p, v.rotation, want, flip);
        }
        v.rotation = want;
    }
}

/// [`sync`] about the frame's centre: the box stays put on screen while the image turns.
pub fn sync_frame(app: &mut PhotocraftApp) {
    let c = finite_frame(app).map(crate::crop_ui::center);
    sync(app, c);
}

/// A commit or cancel in default mode: the view is left upright (Photoshop 25.1 doesn't bring
/// back a Rotate View turn).
pub fn upright(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Crop {
        return;
    }
    if let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
        v.rotation = 0.0;
    }
}

/// The image's turn the default mode shows beside the pointer: clockwise on screen is positive,
/// the frame's angle on the image undone.
pub fn image_turn(frame_angle: f64) -> f64 {
    let a = crate::crop_ui::normalized_angle(-frame_angle);
    // Never "-0.0°".
    if a == 0.0 { 0.0 } else { a }
}

/// A live-centred resize ([`live_resize`]): the frame `rect` and handle (`hx`, `hy`) at the press,
/// grabbed at document point `start`; the pointer now at `q` (in the press view); the frame's turn
/// `deg`, the aspect `ratio` to hold and ⌥ (resize about the centre).
#[derive(Clone, Copy, Debug)]
pub struct LiveResize {
    pub rect: [f64; 4],
    pub deg: f64,
    pub hx: i8,
    pub hy: i8,
    pub start: [f64; 2],
    pub q: [f64; 2],
    pub ratio: Option<f64>,
    pub alt: bool,
}

/// Auto Center Preview during a handle drag: the box stays centred on screen where it was at the
/// press and grows or shrinks symmetrically there, so the handle stays under the pointer, while on
/// the image the opposite edge stays put (the centre with ⌥), as in a plain resize. Everything is
/// computed from the press, so the view following the frame can't feed back into the drag.
/// Returns the proposed frame and the camera's document-space offset without changing the app.
pub(crate) fn resized_frame(g: LiveResize) -> Option<([f64; 4], [f64; 2])> {
    let LiveResize { rect, deg, hx, hy, start, q, ratio, alt } = g;
    let c0 = crate::crop_ui::center(rect);
    let (w0, h0) = (rect[2] - rect[0], rect[3] - rect[1]);
    // The pointer's offset from where it grabbed the handle, in the frame's own axes; the handle
    // moves by it on screen, so each half extent grows by it.
    let d = crate::crop_ui::turn([q[0] - start[0], q[1] - start[1]], [0.0, 0.0], -deg);
    let local = [-w0 / 2.0, -h0 / 2.0, w0 / 2.0, h0 / 2.0];
    let n = crate::crop_ui::resized(local, hx, hy, d, ratio, true);
    let (w1, h1) = (n[2] - n[0], n[3] - n[1]);
    // On the image the opposite edge stays put (with ⌥ the centre), so the centre moves.
    let shift = if alt { [0.0, 0.0] } else { [f64::from(hx) * (w1 - w0) / 2.0, f64::from(hy) * (h1 - h0) / 2.0] };
    let s = crate::crop_ui::turn(shift, [0.0, 0.0], deg);
    let c1 = [c0[0] + s[0], c0[1] + s[1]];
    let f = [c1[0] - w1 / 2.0, c1[1] - h1 / 2.0, c1[0] + w1 / 2.0, c1[1] + h1 / 2.0];
    f.iter().all(|v| v.is_finite()).then_some((f, s))
}

/// Move the camera with the resized frame to keep the crop box's centre stationary on screen.
pub fn live_resize(app: &mut PhotocraftApp, g: LiveResize, press: PressView) {
    let Some((f, s)) = resized_frame(g) else { return };
    app.ui.crop_rect = Some(f);
    // The view follows the centre, so the box stays where it was on screen.
    if let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
        let c = [press.center[0] + s[0] as f32, press.center[1] + s[1] as f32];
        if c.iter().all(|x| x.is_finite()) {
            v.center = c;
        }
    }
}

/// Auto Center Preview while a handle is dragged in default mode.
pub fn live_center(app: &PhotocraftApp) -> bool {
    moves_image(app) && app.ui.tool_options.crop_shield.auto_center_preview
}

/// Moves the image under the box by document vector `d` as the view shows it: the frame moves by
/// `-d` in the document and the view with it, so the box stays put on screen.
pub fn shift_image(app: &mut PhotocraftApp, d: [f64; 2]) {
    let Some(r) = finite_frame(app) else { return };
    if !(d[0].is_finite() && d[1].is_finite()) {
        return;
    }
    let n = [r[0] - d[0], r[1] - d[1], r[2] - d[0], r[3] - d[1]];
    if !n.iter().all(|v| v.is_finite()) {
        return;
    }
    app.ui.crop_rect = Some(n);
    if let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
        let c = [v.center[0] - d[0] as f32, v.center[1] - d[1] as f32];
        if c.iter().all(|x| x.is_finite()) {
            v.center = c;
        }
    }
}

/// The active view's camera (rotation and centre) when a default-mode drag began: the drag reads
/// its points in it, so it doesn't matter that the view turns and pans with the image meanwhile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressView {
    rotation: f32,
    center: [f32; 2],
}

/// The active view's camera now.
pub fn press_view(app: &PhotocraftApp) -> Option<PressView> {
    app.session.active_index().and_then(|i| app.ui.views.get(i)).map(|v| PressView { rotation: v.rotation, center: v.center })
}

/// Document point `p` as seen through camera `from` re-read through camera `to`: the point that
/// `to` shows at the screen position where `from` shows `p` (zoom and flip are the same). `p`
/// itself when anything isn't finite.
fn remap(p: [f64; 2], from: PressView, to: PressView, flip: bool) -> [f64; 2] {
    let at = |v: PressView| ViewXform { rect: Rect::ZERO, zoom: 1.0, center: v.center, flip, rotation: v.rotation, aspect: 1.0 };
    let q = at(to).to_doc(at(from).to_screen(p[0] as f32, p[1] as f32));
    if q.iter().all(|v| v.is_finite()) { q } else { p }
}

/// A pointer point `p` the canvas mapped through the view as it is now, read in the press-time
/// view `press` (unchanged without one, or while the view hasn't moved: the `f32` round trip
/// would nudge a click).
pub fn to_press(app: &PhotocraftApp, press: Option<PressView>, p: [f64; 2]) -> [f64; 2] {
    match (press, press_view(app)) {
        (Some(press), Some(now)) if press != now => remap(p, now, press, app.ui.view.flip_horizontal),
        _ => p,
    }
}

/// A press-view point mapped back into the current camera after snapping the crop handle.
pub(crate) fn from_press(app: &PhotocraftApp, press: PressView, p: [f64; 2]) -> [f64; 2] {
    match press_view(app) {
        Some(now) => remap(p, press, now, app.ui.view.flip_horizontal),
        None => p,
    }
}

/// The control channel's `ui.pointer` gives document points as the view showed them at the press
/// (it can't know how the view turns and pans with the image): during a default-mode drag, the
/// point the canvas would report for the same screen position now.
pub fn control_point(app: &PhotocraftApp, p: [f64; 2]) -> [f64; 2] {
    let press = match app.crop.drag {
        Some(
            crate::crop_ui::CropDrag::MoveImage { view, .. } | crate::crop_ui::CropDrag::TurnImage { view, .. } | crate::crop_ui::CropDrag::Resize { view, .. },
        ) => view,
        _ => None,
    };
    match (press, press_view(app)) {
        (Some(press), Some(now)) if app.ui.tool == Tool::Crop && press != now => remap(p, press, now, app.ui.view.flip_horizontal),
        _ => p,
    }
}

/// A default-mode drag inside the box: the image has moved by document vector `d` (in the press
/// view) since the press, when the frame was `rect` and the view `press`. The frame moves by `-d`
/// on the image and the view's centre with it, so the box stays put on screen.
pub fn offset_image(app: &mut PhotocraftApp, rect: [f64; 4], press: Option<PressView>, d: [f64; 2]) {
    let n = [rect[0] - d[0], rect[1] - d[1], rect[2] - d[0], rect[3] - d[1]];
    if !n.iter().all(|v| v.is_finite()) {
        return;
    }
    app.ui.crop_rect = Some(n);
    let (Some(press), Some(v)) = (press, app.session.active_index().and_then(|i| app.ui.views.get_mut(i))) else { return };
    let c = [press.center[0] - d[0] as f32, press.center[1] - d[1] as f32];
    if c.iter().all(|x| x.is_finite()) {
        v.center = c;
    }
}

/// A screen vector `s` (points, at the current zoom ignored) as a document vector of the same
/// length: undoes the view's flip and rotation.
pub fn screen_to_doc_vec(app: &PhotocraftApp, s: [f64; 2]) -> [f64; 2] {
    let xf = ViewXform { rect: Rect::ZERO, zoom: 1.0, center: [0.0, 0.0], flip: app.ui.view.flip_horizontal, rotation: view_rotation(app), aspect: 1.0 };
    let u = xf.unmap_vec(vec2(s[0] as f32, s[1] as f32));
    [f64::from(u.x), f64::from(u.y)]
}

/// Auto Center Preview: in default mode, pans the view so the box sits in the middle of the canvas
/// (the view turns about its centre, so the box stays upright).
pub fn auto_center(app: &mut PhotocraftApp) {
    if !turns_view(app) || classic(app) || !app.ui.tool_options.crop_shield.auto_center_preview {
        return;
    }
    let Some(c) = finite_frame(app).map(crate::crop_ui::center) else { return };
    let c = [c[0] as f32, c[1] as f32];
    if let (Some(v), true) = (app.session.active_index().and_then(|i| app.ui.views.get_mut(i)), c.iter().all(|x| x.is_finite())) {
        v.center = c;
    }
}

#[cfg(test)]
mod tests;
