use super::{DRect, Document, PhotocraftApp, Pos2, Rect, Vec2, View, pos2, vec2};

/// Document ↔ screen mapping for a canvas rect and a view.
#[derive(Clone, Copy, Debug)]
pub struct ViewXform {
    pub rect: Rect,
    pub zoom: f32,
    pub center: [f32; 2],
    /// View › Flip Horizontal: the view is mirrored left-right (the document is not).
    pub flip: bool,
    /// Camera rotation in degrees around [`Self::center`]. Clockwise (Y-down) is positive; applied
    /// before flip.
    pub rotation: f32,
    /// View › Pixel Aspect Ratio Correction: a document pixel is drawn this many times wider than
    /// tall (1 = square). Applied in document space, before rotation, so a rotated view still
    /// shows the stretched image turned as a whole.
    pub aspect: f32,
}

impl ViewXform {
    /// The main canvas's mapping for the active document as last laid out (inside the rulers).
    pub fn active(app: &PhotocraftApp) -> Option<Self> {
        let v = app.ui.views.get(app.session.active_index()?)?;
        let (zoom, center) =
            if v.zoom_animating && app.session.prefs().general.animated_zoom { v.display_camera.unwrap_or((v.zoom, v.center)) } else { (v.zoom, v.center) };
        Some(
            Self {
                rect: crate::rulers::content_rect(app, app.last_canvas_rect),
                zoom: zoom / app.canvas_ppp(),
                center,
                flip: app.ui.view.flip_horizontal,
                rotation: v.rotation,
                aspect: app.ui.view.display_aspect(),
            }
            .pixel_aligned_for(app, app.canvas_ppp()),
        )
    }

    pub(super) fn pixel_aligned_for(self, app: &PhotocraftApp, ppp: f32) -> Self {
        // Default Crop counter-moves the frame and camera with fractional pointer deltas.
        // Preserve that continuous mapping so its fixed frame does not wobble during a drag.
        if crate::crop_mode::moves_image(app) { self } else { self.pixel_aligned(ppp) }
    }

    /// At integer physical magnifications, texel edges must land on display pixel edges.
    /// Keep this derived camera shared by images, overlays and input without changing the saved pan.
    pub(super) fn pixel_aligned(mut self, ppp: f32) -> Self {
        if !ppp.is_finite() || ppp <= 0.0 || !self.zoom.is_finite() || self.zoom <= 0.0 || !self.rotation.is_finite() || self.rotation.abs() >= 1e-8 {
            return self;
        }
        let scale = vec2(self.zoom_x(), self.zoom) * ppp;
        if !scale.is_finite()
            || scale.x.round() < 1.0
            || scale.y.round() < 1.0
            || (scale.x - scale.x.round()).abs() > 1e-5
            || (scale.y - scale.y.round()).abs() > 1e-5
        {
            return self;
        }
        let origin = self.to_screen(0.0, 0.0);
        let physical = origin * ppp;
        if !physical.is_finite() {
            return self;
        }
        let aligned = pos2(physical.x.round(), physical.y.round()) / ppp;
        let offset = self.unmap_vec((aligned - origin) / self.zoom);
        let center = [self.center[0] - offset.x, self.center[1] - offset.y];
        if center.iter().all(|v| v.is_finite()) {
            self.center = center;
        }
        self
    }

    fn sincos(self) -> (f32, f32) {
        if self.rotation.abs() < 1e-8 {
            (1.0, 0.0)
        } else {
            let r = self.rotation.to_radians();
            (r.cos(), r.sin())
        }
    }

    /// Document pixel width over height on screen: [`Self::aspect`], or 1 when it is unusable.
    fn sx(self) -> f32 {
        if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 }
    }

    /// Screen points per document pixel along the document's x axis.
    pub fn zoom_x(&self) -> f32 {
        self.zoom * self.sx()
    }

    /// Unflip, unrotate and unstretch a screen-space vector (no zoom). Hand pan and zoom-about
    /// use this.
    pub fn unmap_vec(self, d: Vec2) -> Vec2 {
        let d = if self.flip { vec2(-d.x, d.y) } else { d };
        let (c, s) = self.sincos();
        vec2((d.x * c + d.y * s) / self.sx(), -d.x * s + d.y * c)
    }

    /// A document-space vector on screen: stretched, rotated, flipped and zoomed as the view
    /// draws it (the inverse of [`Self::unmap_vec`], with the zoom).
    pub fn map_vec(self, d: Vec2) -> Vec2 {
        let (c, s) = self.sincos();
        let dx = d.x * self.sx();
        let v = vec2(dx * c - d.y * s, dx * s + d.y * c) * self.zoom;
        if self.flip { vec2(-v.x, v.y) } else { v }
    }

    pub fn to_screen(&self, x: f32, y: f32) -> Pos2 {
        let (c, s) = self.sincos();
        let dx = (x - self.center[0]) * self.sx();
        let dy = y - self.center[1];
        let rx = dx * c - dy * s;
        let ry = dx * s + dy * c;
        let sx = if self.flip { -1.0 } else { 1.0 };
        self.rect.center() + vec2(rx * self.zoom * sx, ry * self.zoom)
    }
    pub fn to_doc(&self, p: Pos2) -> [f64; 2] {
        let d = (p - self.rect.center()) / self.zoom;
        let u = self.unmap_vec(d);
        [(u.x + self.center[0]) as f64, (u.y + self.center[1]) as f64]
    }
    /// On-screen bounds of the image, including camera rotation and pixel aspect correction.
    pub fn image_bounds(self, size: [u32; 2]) -> Rect {
        let (c, s) = self.sincos();
        let w = size[0] as f32 * self.sx() * self.zoom;
        let h = size[1] as f32 * self.zoom;
        let center = self.to_screen(size[0] as f32 / 2.0, size[1] as f32 / 2.0);
        Rect::from_center_size(center, vec2(c.abs() * w + s.abs() * h, s.abs() * w + c.abs() * h))
    }
    pub fn doc_rect(&self, r: DRect) -> Rect {
        let pts = [
            self.to_screen(r.x0 as f32, r.y0 as f32),
            self.to_screen(r.x1 as f32, r.y0 as f32),
            self.to_screen(r.x1 as f32, r.y1 as f32),
            self.to_screen(r.x0 as f32, r.y1 as f32),
        ];
        let mut min = pts[0];
        let mut max = pts[0];
        for p in pts {
            min = min.min(p);
            max = max.max(p);
        }
        Rect::from_min_max(min, max)
    }
}

/// Document point under a screen position on the main canvas, or `None` when the position is
/// outside it: the rulers are not part of the canvas, so drops on the ruler strip are not
/// canvas drops. Preset-panel drops and drag-and-drop placement share this so both land exactly
/// where the canvas draws.
pub fn doc_point_at(app: &PhotocraftApp, pos: Pos2) -> Option<[f64; 2]> {
    let xf = ViewXform::active(app)?;
    xf.rect.contains(pos).then(|| xf.to_doc(pos))
}

/// Fit the document into `area` (egui points) for a display with `ppp` physical pixels per
/// point; `View::zoom` is stored in physical pixels per document pixel. `aspect` is the
/// displayed pixel aspect ratio ([`ViewXform::aspect`]): the image is that much wider on screen.
pub fn fit_view(view: &mut View, doc: &Document, area: Vec2, ppp: f32, aspect: f32) {
    let (w, h) = (doc.size.width as f32, doc.size.height as f32);
    let shown_w = w * if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    // Fit never magnifies past 100%: one image pixel per *physical* pixel, so the cap applies
    // after the points → device pixels conversion (a 150% display would otherwise open at 150%).
    let fit = ((area.x - 40.0) / shown_w).min((area.y - 40.0) / h) * ppp;
    let zoom = crate::zoom_levels::clamp(fit.min(1.0), [doc.size.width, doc.size.height]);
    view.zoom = zoom;
    view.center = [w / 2.0, h / 2.0];
    view.fit_pending = false;
    view.fill_pending = false;
}

/// Ease a transient presentation camera over 220 ms without changing the requested camera.
pub(super) fn animated_view(view: &mut View, xf: ViewXform, ppp: f32, animated_zoom: bool, time: f64) -> View {
    if !animated_zoom {
        view.request_zoom_animation(false);
    }
    let target = (view.zoom, view.center);
    // Clamping, a canvas resize, or another camera command can make the original anchor
    // incompatible with the requested camera. The target must always win.
    if let Some((doc, screen)) = view.zoom_anchor {
        let offset = xf.unmap_vec((pos2(screen[0], screen[1]) - xf.rect.center()) / (view.zoom / ppp));
        if (doc[0] as f32 - offset.x - view.center[0]).abs() > 0.01 || (doc[1] as f32 - offset.y - view.center[1]).abs() > 0.01 {
            view.zoom_anchor = None;
        }
    }
    let mut shown = view.clone();
    if view.zoom_animating {
        if view.zoom_transition.is_none_or(|a| a.target != target) {
            view.zoom_transition = Some(crate::state::ZoomTransition { from: view.display_camera.unwrap_or(target), target, started: time });
        }
        if let Some(a) = view.zoom_transition {
            let elapsed = (time - a.started) / 0.22;
            let t = if elapsed.is_finite() { elapsed.clamp(0.0, 1.0) as f32 } else { 1.0 };
            let ease = t * t * (3.0 - 2.0 * t);
            let lerp = |from: f32, to: f32| from + (to - from) * ease;
            shown.zoom = lerp(a.from.0, target.0);
            shown.center = [lerp(a.from.1[0], target.1[0]), lerp(a.from.1[1], target.1[1])];
            if let Some((doc, screen)) = view.zoom_anchor {
                let zoom = shown.zoom / ppp;
                if zoom.is_finite() && zoom > 0.0 {
                    let offset = xf.unmap_vec((pos2(screen[0], screen[1]) - xf.rect.center()) / zoom);
                    shown.center = [doc[0] as f32 - offset.x, doc[1] as f32 - offset.y];
                }
            }
            if t >= 1.0 {
                view.zoom_animating = false;
            }
        }
    }
    if !view.zoom_animating {
        shown.zoom = target.0;
        shown.center = target.1;
        view.zoom_anchor = None;
        view.zoom_transition = None;
    }
    view.display_camera = Some((shown.zoom, shown.center));
    shown
}

/// Centre the document and zoom until it fills the canvas. One axis can extend beyond the
/// viewport, matching the Hand tool's Fill Screen action.
pub fn fill_view(view: &mut View, doc: &Document, area: Vec2, ppp: f32, aspect: f32) {
    let (w, h) = (doc.size.width as f32, doc.size.height as f32);
    let shown_w = w * if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let points = (area.x / shown_w).max(area.y / h);
    let zoom = crate::zoom_levels::clamp(points * ppp, [doc.size.width, doc.size.height]);
    view.zoom = zoom;
    view.center = [w / 2.0, h / 2.0];
    view.fit_pending = false;
    view.fill_pending = false;
}

/// Where a wheel or pinch zoom anchors for a pointer at `p`: the pointer itself over the image,
/// otherwise the nearest point of the image (#3720). Anchoring on a pointer out on the gray
/// pasteboard slid the image away from it, out of the window after a few notches; Photoshop keeps
/// the image in view. Clamped in document space, so a rotated or flipped view behaves the same.
pub(super) fn wheel_anchor(xf: &ViewXform, p: Pos2, doc_size: [u32; 2]) -> Pos2 {
    let [w, h] = doc_size;
    let d = xf.to_doc(p);
    if w == 0 || h == 0 || !(d[0].is_finite() && d[1].is_finite()) {
        return p;
    }
    let c = [d[0].clamp(0.0, f64::from(w)), d[1].clamp(0.0, f64::from(h))];
    if c == d { p } else { xf.to_screen(c[0] as f32, c[1] as f32) }
}

/// Zoom to `new_zoom` about `p`: keep its document point under the pointer, or move it to the
/// view centre when Zoom Clicked Point to Center (`center_on_point`) is enabled.
pub(super) fn zoom_about(view: &mut View, xf: &ViewXform, p: Pos2, new_zoom: f32, center_on_point: bool, ppp: f32) {
    let before = xf.to_doc(p);
    view.zoom = new_zoom;
    if center_on_point {
        view.center = [before[0] as f32, before[1] as f32];
        return;
    }
    // `new_zoom` is device px per document pixel; the pointer vector is in egui points.
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let u = xf.unmap_vec((p - xf.rect.center()) / (new_zoom / ppp));
    view.center = [before[0] as f32 - u.x, before[1] as f32 - u.y];
}
