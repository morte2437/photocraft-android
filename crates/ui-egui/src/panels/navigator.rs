use super::*;

pub(super) fn navigator(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(idx) = app.session.active_index() else {
        empty(ui, tl!("No document"));
        return;
    };
    let ctx = ui.ctx().clone();
    let size = app.session.documents()[idx].doc.size;
    let avail = ui.available_width();
    // The preview fills the group's height above the zoom controls (their height as measured
    // last frame), so it grows when the group is made taller, as in Photoshop (#2726).
    let controls_h = ui.data(|d| d.get_temp::<f32>(navigator_controls_id())).unwrap_or(48.0);
    let left = ui.available_height() - controls_h;
    let box_h = if left.is_finite() { left.max(40.0) } else { 150.0 };
    let (frame, resp) = ui.allocate_exact_size(vec2(avail, box_h), Sense::click_and_drag());
    ui.painter().rect_filled(frame, t.radius_sm, t.canvas);
    let rect = navigator_fit(frame, size.width, size.height);
    ui.data_mut(|d| d.insert_temp(navigator_preview_id(), rect));
    // On the GPU canvas, its own texture at this size (#4518); otherwise the CPU thumbnail.
    if let Some(params) = crate::canvas::navigator_gpu_view(app, idx, rect, &ctx) {
        crate::gpu_canvas::GpuCanvas::paint(ui.painter(), rect, params);
    } else if crate::canvas::navigator_waits_for_gpu(app, idx, &ctx) {
        widgets::checker(ui.painter(), rect, 6.0);
    } else {
        let Some(tex) = crate::canvas::navigator_texture(app, &ctx, idx) else { return };
        widgets::checker(ui.painter(), rect, 6.0);
        ui.painter().image(tex, rect, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    let s = rect.width() / size.width.max(1) as f32;
    // ⌘/Ctrl-drag over the preview draws a rectangle the view then zooms to, as in Photoshop
    // (#6872); a plain click or drag moves the view.
    let command = crate::workspace_ui::sticky_mods(app, ctx.input(|i| i.modifiers)).command;
    if resp.drag_started() && command {
        let start = ctx.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
        ui.data_mut(|d| match start {
            Some(p) => {
                d.insert_temp(navigator_zoom_drag_id(), p);
            }
            None => d.remove::<Pos2>(navigator_zoom_drag_id()),
        });
    }
    let zoom_from = ui.data(|d| d.get_temp::<Pos2>(navigator_zoom_drag_id()));
    if let Some(from) = zoom_from {
        if let Some(to) = resp.interact_pointer_pos() {
            let r = Rect::from_two_pos(from, to).intersect(rect);
            crate::tool_feedback::draw_ants(ui.painter(), &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()], true);
            if resp.drag_stopped() {
                let doc_rect = Rect::from_min_max(((r.min - rect.min) / s).to_pos2(), ((r.max - rect.min) / s).to_pos2());
                let (canvas, ppp, aspect) = (app.last_canvas_rect, app.canvas_ppp(), app.ui.view.display_aspect());
                navigator_zoom_to(&mut app.ui.views[idx], doc_rect, [size.width, size.height], canvas.size(), ppp, aspect);
            }
        }
        if resp.drag_stopped() || !resp.dragged() {
            ui.data_mut(|d| d.remove::<Pos2>(navigator_zoom_drag_id()));
        }
    } else if (resp.clicked() || resp.dragged())
        && let Some(pp) = resp.interact_pointer_pos()
    {
        let d = (pp - rect.min) / s;
        app.ui.views[idx].center = [d.x.clamp(0.0, size.width as f32), d.y.clamp(0.0, size.height as f32)];
        app.ui.views[idx].zoom_animating = false;
        app.ui.views[idx].zoom_anchor = None;
        app.ui.views[idx].fit_pending = false;
    }
    if command && resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::ZoomIn);
    }
    // The Navigator has its own zoom target: an unmodified wheel over the preview
    // should zoom even when the document canvas is configured to scroll on wheel.
    // Use the canvas wheel classifier to honour one ×1.1 step per notch,
    // eased over its frames or instant as General › Animated Zoom says.
    let pinch_zooms = app.session.prefs().enhanced_controls.zoom_with_trackpad_pinch;
    let animated_zoom = app.session.prefs().general.animated_zoom;
    let touch_gestures = app.session.prefs().enhanced_controls.touch_gestures;
    let navigator_wheel = egui::Id::unique("pc-navigator-wheel");
    if resp.hovered()
        && let Some(crate::wheel_nav::Wheel::Zoom(factor)) =
            crate::wheel_nav::read_with(&ctx, true, pinch_zooms, animated_zoom, navigator_wheel, true, true, touch_gestures)
    {
        let view = &mut app.ui.views[idx];
        let next = crate::zoom_levels::clamp(view.zoom * factor, view.doc_size);
        if next != view.zoom * factor {
            crate::wheel_nav::settle(&ctx, navigator_wheel);
        }
        if next != view.zoom {
            view.zoom = next;
            view.zoom_animating = false;
            view.zoom_anchor = None;
            view.fit_pending = false;
        }
    }
    // Visible-area rectangle.
    let v = app.ui.views[idx].clone();
    let canvas = app.last_canvas_rect;
    let (zoom, center) =
        if v.zoom_animating && app.session.prefs().general.animated_zoom { v.display_camera.unwrap_or((v.zoom, v.center)) } else { (v.zoom, v.center) };
    let point_zoom = (zoom / app.canvas_ppp()).max(1e-6);
    // Pixel aspect correction shows fewer document columns across the same canvas width.
    let vw = canvas.width() / point_zoom / app.ui.view.display_aspect() * s;
    let vh = canvas.height() / point_zoom * s;
    let c = pos2(rect.min.x + center[0] * s, rect.min.y + center[1] * s);
    let vr = Rect::from_center_size(c, vec2(vw, vh)).intersect(frame.shrink(1.0));
    ui.painter().rect_stroke(vr, 2.0, Stroke::new(1.5, Color32::from_rgb(255, 84, 84)), StrokeKind::Middle);
    // Photoshop's one row under the preview (#4840): the zoom, typed in percent, then Zoom Out,
    // the slider and Zoom In.
    let controls = ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let set = |app: &mut PhotocraftApp, zoom: f32, animate: bool| {
                let target = crate::zoom_levels::clamp(zoom, v.doc_size);
                if target != app.ui.views[idx].zoom {
                    app.ui.views[idx].zoom = target;
                    app.ui.views[idx].request_zoom_animation(animate && app.session.prefs().general.animated_zoom);
                }
                app.ui.views[idx].fit_pending = false;
            };
            let mut pct = v.zoom * 100.0;
            let field = widgets::compact_wrapping_value_field(ui, &mut pct, crate::zoom_levels::percent_range(&app.ui.views[idx]), "%", vec2(64.0, NAV_ROW_H))
                .accessible_name(tl!("Zoom"));
            if field.changed() {
                set(app, pct / 100.0, false);
            }
            let out = crate::brush_picker::named(icons::button(ui, "minus", NAV_ROW_H, false, tl!("Zoom Out")), "Zoom Out");
            if out.clicked() {
                set(app, crate::zoom_levels::step(v.zoom, -1, v.doc_size), true);
            }
            // The whole zoom range, and always the current zoom: a narrower slider would pull it back.
            let (lo, hi) = (crate::zoom_levels::min(v.doc_size).min(v.zoom).log2(), crate::zoom_levels::MAX.max(v.zoom).log2());
            let mut lz = v.zoom.log2();
            let slider_w = (ui.available_width() - NAV_ROW_H - 4.0).max(60.0);
            let slid = ui.allocate_ui(vec2(slider_w, NAV_ROW_H), |ui| lz.is_finite() && widgets::slider(ui, &mut lz, lo..=hi, None).changed()).inner;
            if slid {
                set(app, 2f32.powf(lz), false);
            }
            let zin = crate::brush_picker::named(icons::button(ui, "plus", NAV_ROW_H, false, tl!("Zoom In")), "Zoom In");
            if zin.clicked() {
                set(app, crate::zoom_levels::step(v.zoom, 1, v.doc_size), true);
            }
        });
    });
    // From the preview's bottom, so the spacing above the controls counts too.
    let controls = Rect::from_min_max(pos2(frame.left(), frame.bottom()), pos2(frame.right(), controls.response.rect.bottom()));
    ui.data_mut(|d| {
        d.insert_temp(navigator_controls_id(), controls.height());
        d.insert_temp(navigator_controls_rect_id(), controls);
    });
}

/// Height of the Navigator's zoom row: its field, buttons and slider.
pub(super) const NAV_ROW_H: f32 = 22.0;

/// The document's `w` × `h` fitted inside the Navigator's `frame` with an 8 pt inset, centred and
/// keeping its aspect.
fn navigator_fit(frame: Rect, w: u32, h: u32) -> Rect {
    let inner = (frame.size() - vec2(16.0, 16.0)).max(vec2(1.0, 1.0));
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let s = (inner.x / w).min(inner.y / h);
    Rect::from_center_size(frame.center(), vec2(w * s, h * s))
}

/// Where a ⌘/Ctrl-drag over the Navigator's preview started, while it lasts.
pub(super) fn navigator_zoom_drag_id() -> egui::Id {
    egui::Id::unique("navigator-zoom-drag")
}

/// Zoom `view` so the document rectangle `doc_rect` (pixels) fills a canvas of `canvas` points,
/// centred on it, as a Navigator ⌘/Ctrl-drag does (#6872). `doc` is the document's size in
/// pixels (the view's own copy is only set once the canvas has been drawn), `ppp` the canvas's
/// device pixels per point and `aspect` the pixel aspect ratio shown; the zoom stays within the
/// zoom range. A rectangle under 2 document pixels across zooms one step in on its centre.
pub(super) fn navigator_zoom_to(view: &mut crate::state::View, doc_rect: Rect, doc: [u32; 2], canvas: egui::Vec2, ppp: f32, aspect: f32) {
    // Checked before clipping: `min`/`max` would turn a NaN edge into a document edge.
    if ![doc_rect.min.x, doc_rect.min.y, doc_rect.max.x, doc_rect.max.y].iter().all(|v| v.is_finite()) {
        return;
    }
    let (w, h) = (doc[0] as f32, doc[1] as f32);
    let r = doc_rect.intersect(Rect::from_min_size(Pos2::ZERO, vec2(w, h)));
    if r.width() < 0.0 || r.height() < 0.0 {
        return;
    }
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let zoom = if r.width() < 2.0 || r.height() < 2.0 || canvas.x <= 0.0 || canvas.y <= 0.0 {
        crate::zoom_levels::step(view.zoom, 1, view.doc_size)
    } else {
        // Document columns shown across the canvas are `canvas.x / (point_zoom * aspect)`.
        let point_zoom = (canvas.x / (r.width() * aspect)).min(canvas.y / r.height());
        point_zoom * ppp
    };
    if !zoom.is_finite() {
        return;
    }
    view.zoom = crate::zoom_levels::clamp(zoom, view.doc_size);
    view.center = [r.center().x, r.center().y];
    view.fit_pending = false;
    view.request_zoom_animation(false);
}

fn navigator_preview_id() -> egui::Id {
    egui::Id::unique("navigator-preview-rect")
}

fn navigator_controls_id() -> egui::Id {
    egui::Id::unique("navigator-controls-height")
}

fn navigator_controls_rect_id() -> egui::Id {
    egui::Id::unique("navigator-controls-rect")
}

/// Where the Navigator drew its document preview last frame.
#[cfg(test)]
pub(crate) fn last_navigator_preview(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(navigator_preview_id()))
}

/// The Navigator's zoom controls (below the preview) last frame.
#[cfg(test)]
pub(crate) fn last_navigator_controls(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(navigator_controls_rect_id()))
}
