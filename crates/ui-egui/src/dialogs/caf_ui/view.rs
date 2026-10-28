use super::controls::{pointer, tool_icon, tool_tip};
use super::lifecycle::{apply, bounds, cancel, commit, fill_edit, report, reset, set_sampling};
use super::preview::{drive_preview, rebase, refresh};
use super::{
    Align2, BAR_H, CafTool, CafWorkspace, Color32, ERect, FontId, LEFT_W, PANEL_W, PREVIEW_W, PhotocraftApp, Pos2, Rect, Sense, Stroke, Tokens, ToolEvent,
    Value, json, pos2, vec2, widgets,
};

/// A view: screen px per document px and the document point at the centre of `area`.
#[derive(Clone, Copy)]
struct View {
    area: ERect,
    zoom: f32,
    center: [f64; 2],
}

impl View {
    fn to_screen(self, p: [f64; 2]) -> Pos2 {
        let c = self.area.center();
        pos2(c.x + ((p[0] - self.center[0]) as f32) * self.zoom, c.y + ((p[1] - self.center[1]) as f32) * self.zoom)
    }
    fn to_doc(self, s: Pos2) -> [f64; 2] {
        let c = self.area.center();
        [self.center[0] + f64::from((s.x - c.x) / self.zoom), self.center[1] + f64::from((s.y - c.y) / self.zoom)]
    }
    fn rect(self, r: Rect) -> ERect {
        ERect::from_min_max(self.to_screen([f64::from(r.x0), f64::from(r.y0)]), self.to_screen([f64::from(r.x1), f64::from(r.y1)]))
    }
}

/// Upload what changed: the document proxy, the overlay, a finished preview.
/// The options bar of the Content-Aware Fill workspace, for the active tool.
fn caf_options_bar(bui: &mut egui::Ui, w: &mut CafWorkspace, fill_btn: &mut Option<Value>, reset_what: &mut Option<&str>) {
    match w.tool {
        CafTool::Brush => {
            if crate::icons::button(bui, "plus", 26.0, w.brush_add, tl!("Add")).clicked() {
                w.brush_add = true;
            }
            if crate::icons::button(bui, "minus", 26.0, !w.brush_add, tl!("Subtract")).clicked() {
                w.brush_add = false;
            }
            bui.add_space(8.0);
            bui.label(tl!("Size:"));
            let mut size = w.brush_size as f32;
            if widgets::value_field(bui, &mut size, 1.0..=5000.0, " px", 64.0).accessible_name(tl!("Size:")).changed() {
                w.brush_size = f64::from(size);
            }
        }
        CafTool::Lasso => {
            // The selection tools' mode buttons (the lasso here edits the fill area).
            let tips = [
                tl!("New selection").to_string(),
                crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))]),
                crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))]),
                crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))]),
            ];
            for (i, (icon, tip)) in ["square", "plus", "minus", "squares-subtract"].into_iter().zip(tips.iter()).enumerate() {
                if crate::icons::button(bui, icon, 26.0, w.lasso_op == i, tip).clicked() {
                    w.lasso_op = i;
                }
            }
            bui.add_space(8.0);
            if widgets::secondary_button(bui, tl!("Expand"), 70.0).clicked() {
                *fill_btn = Some(json!({"op": "expand", "by": w.expand_by}));
            }
            if widgets::secondary_button(bui, tl!("Contract"), 70.0).clicked() {
                *fill_btn = Some(json!({"op": "contract", "by": w.expand_by}));
            }
            widgets::value_field(bui, &mut w.expand_by, 1.0..=100.0, " px", 56.0).accessible_name(tl!("Expand"));
            if crate::icons::button(bui, "undo-2", 26.0, false, tl!("Reset")).clicked() {
                *reset_what = Some("lasso");
            }
        }
        _ => {}
    }
}

fn upload(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let need_image = app.distort.caf.as_ref().is_some_and(|w| w.image_tex.is_none());
    if need_image {
        let img = view_image(app);
        if let (Some(w), Some(img)) = (app.distort.caf.as_mut(), img) {
            w.image_tex = Some(ctx.load_texture("caf-image", img, egui::TextureOptions::LINEAR));
        }
    }
    let Some(w) = app.distort.caf.as_mut() else { return };
    if w.overlay_dirty || w.overlay_tex.is_none() {
        w.overlay_dirty = false;
        let [r, g, b] = w.opts.color;
        let a = (w.opts.opacity / 100.0 * 255.0).round() as u8;
        let tint = Color32::from_rgba_unmultiplied(r, g, b, a);
        // The tint marks the sampling area (with Indicates: Excluded Area, what is left out);
        // the fill area is never tinted.
        let px: Vec<Color32> =
            w.mask.iter().zip(&w.hole_px).map(|(m, h)| if w.opts.show_overlay && !*h && *m != w.opts.excluded { tint } else { Color32::TRANSPARENT }).collect();
        let img = egui::ColorImage::new([w.pw, w.ph], px);
        match &mut w.overlay_tex {
            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
            None => w.overlay_tex = Some(ctx.load_texture("caf-overlay", img, egui::TextureOptions::NEAREST)),
        }
    }
    if let Some((rect, img)) = w.preview.pending_image.take() {
        w.preview.tex = Some((ctx.load_texture("caf-preview", img, egui::TextureOptions::LINEAR), rect));
    }
}

/// The document as the view shows it: the visible layers composited, as on the canvas (so an
/// Apply to a new layer shows), reduced to the proxy.
pub(super) fn view_image(app: &PhotocraftApp) -> Option<egui::ColorImage> {
    let w = app.distort.caf.as_ref()?;
    let st = app.session.active()?;
    let buf = photocraft_compose::render_reduced(&st.doc, w.pw as u32, w.ph as u32);
    let (display, _) = crate::canvas::canvas_display(app, &st.doc, None);
    Some(crate::canvas::display_image(display.as_deref(), &buf))
}

/// The Content-Aware Fill panel's scrolling settings: sampling area, fill and output options.
fn caf_props_panel(
    ui: &mut egui::Ui,
    w: &mut CafWorkspace,
    t: &Tokens,
    reset_what: &mut Option<&str>,
    new_sampling: &mut Option<&str>,
    rebase_needed: &mut bool,
) {
    ui.spacing_mut().item_spacing.y = 6.0;
    egui::CollapsingHeader::new(tl!("Sampling Area Overlay")).id_salt("caf-overlay").default_open(false).show(ui, |ui| {
        widgets::checkbox(ui, &mut w.opts.show_overlay, tl!("Show Sampling Area"));
        widgets::slider_row(ui, tl!("Opacity"), &mut w.opts.opacity, 0.0..=100.0, "%", None);
        ui.horizontal(|ui| {
            ui.label(tl!("Color"));
            widgets::color_edit_button_srgb(ui, &mut w.opts.color);
            ui.add_space(8.0);
            ui.label(tl!("Indicates"));
            widgets::dropdown(
                ui,
                "caf-indicates",
                tl!("Indicates"),
                &mut w.opts.excluded,
                &[(false, tl!("Sampling Area")), (true, tl!("Excluded Area"))],
                120.0,
            );
        });
    });
    widgets::hairline(ui);
    ui.horizontal(|ui| {
        widgets::section_label(ui, tl!("Sampling Area Options"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "undo-2", 22.0, false, tl!("Reset")).clicked() {
                *reset_what = Some("sampling");
            }
        });
    });
    ui.horizontal(|ui| {
        for (k, l) in [("auto", tl!("Auto")), ("rectangular", tl!("Rectangular")), ("custom", tl!("Custom"))] {
            if ui.selectable_label(w.opts.sampling == k, l).clicked() && w.opts.sampling != k {
                *new_sampling = Some(k);
            }
        }
    });
    if widgets::checkbox(ui, &mut w.opts.sample_all, tl!("Sample All Layers")).changed() {
        *rebase_needed = true;
    }
    widgets::hairline(ui);
    ui.horizontal(|ui| {
        widgets::section_label(ui, tl!("Fill Settings"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "undo-2", 22.0, false, tl!("Reset")).clicked() {
                *reset_what = Some("fill");
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Color Adaptation"));
        let o = [
            ("none".to_string(), tl!("None")),
            ("default".to_string(), tl!("Default")),
            ("high".to_string(), tl!("High")),
            ("veryHigh".to_string(), tl!("Very High")),
        ];
        widgets::dropdown(ui, "caf-color", tl!("Color Adaptation"), &mut w.opts.color_adaptation, &o, 110.0);
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Rotation Adaptation"));
        let o = [
            ("none".to_string(), tl!("None")),
            ("low".to_string(), tl!("Low")),
            ("medium".to_string(), tl!("Medium")),
            ("high".to_string(), tl!("High")),
            ("full".to_string(), tl!("Full")),
        ];
        widgets::dropdown(ui, "caf-rotation", tl!("Rotation Adaptation"), &mut w.opts.rotation, &o, 110.0);
    });
    ui.horizontal(|ui| {
        widgets::checkbox(ui, &mut w.opts.scale, tl!("Scale"));
        widgets::checkbox(ui, &mut w.opts.mirror, tl!("Mirror"));
    });
    widgets::hairline(ui);
    widgets::section_label(ui, tl!("Output Settings"));
    ui.horizontal(|ui| {
        ui.label(tl!("Output To"));
        let o = [("current".to_string(), tl!("Current Layer")), ("new".to_string(), tl!("New Layer")), ("duplicate".to_string(), tl!("Duplicate Layer"))];
        widgets::dropdown(ui, "caf-output", tl!("Output To"), &mut w.opts.output, &o, 140.0);
    });
    if let Some(e) = w.error.as_ref().or(w.preview.error.as_ref()) {
        ui.label(egui::RichText::new(e.as_str()).color(t.danger).size(11.0));
    }
}

/// The Preview panel: the sampled result in its own pan / zoom view, and the zoom row under it.
fn caf_preview_panel(ui: &mut egui::Ui, painter: &egui::Painter, t: &Tokens, w: &mut CafWorkspace, pv_rect: ERect) {
    painter.rect_filled(pv_rect, 0.0, t.dock);
    painter.line_segment([pv_rect.left_top(), pv_rect.left_bottom()], Stroke::new(1.0, t.separator));
    painter.text(pv_rect.left_top() + vec2(12.0, 10.0), Align2::LEFT_TOP, tl!("Preview"), FontId::proportional(12.0), t.text);
    let pv_area = ERect::from_min_max(pv_rect.min + vec2(0.0, 30.0), pv_rect.max - vec2(0.0, 44.0));
    painter.rect_filled(pv_area, 0.0, t.canvas);
    if w.preview.zoom <= 0.0 {
        // Opens framed on the sampling window, as Photoshop's.
        let win = w.base.window;
        if !win.is_empty() {
            w.preview.zoom = (pv_area.width() / win.width().max(win.height()) as f32).clamp(0.01, 32.0);
            w.preview.center = [f64::from(win.x0 + win.x1) / 2.0, f64::from(win.y0 + win.y1) / 2.0];
        }
    }
    let pv = View { area: pv_area, zoom: w.preview.zoom.max(0.01), center: w.preview.center };
    let pclip = painter.with_clip_rect(pv_area);
    let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let img_rect = pv.rect(w.canvas);
    widgets::checker(&pclip, img_rect.intersect(pv_area), 8.0);
    if let Some(tex) = &w.image_tex {
        pclip.image(tex.id(), img_rect, uv, Color32::WHITE);
    }
    if let Some((tex, r)) = &w.preview.tex {
        pclip.image(tex.id(), pv.rect(*r), uv, Color32::WHITE);
    }
    if w.preview.shown != w.preview.want {
        // Being recomputed: a spinner in the corner.
        let sp = ERect::from_min_size(pv_area.right_bottom() - vec2(26.0, 26.0), vec2(18.0, 18.0));
        ui.put(sp, egui::Spinner::new().size(16.0));
    }
    let presp = ui.interact(pv_area, egui::Id::unique("caf-preview-view"), Sense::click_and_drag());
    if presp.dragged() {
        let d = presp.drag_delta();
        w.preview.center = [w.preview.center[0] - f64::from(d.x / pv.zoom), w.preview.center[1] - f64::from(d.y / pv.zoom)];
    }
    if presp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            w.preview.zoom = (w.preview.zoom * (scroll / 200.0).exp()).clamp(0.01, 32.0);
        }
    }
    let zrow = ERect::from_min_size(pos2(pv_rect.left() + 10.0, pv_rect.bottom() - 38.0), vec2(PREVIEW_W - 20.0, 30.0));
    let mut zr = ui.new_child(egui::UiBuilder::new().max_rect(zrow).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let mut pct = (w.preview.zoom * 100.0).round();
    if widgets::value_field(&mut zr, &mut pct, 1.0..=3200.0, "%", 64.0).accessible_name(tl!("Zoom")).changed() {
        w.preview.zoom = pct / 100.0;
    }
    let mut lz = pct.max(1.0).ln();
    if zr.add(egui::Slider::new(&mut lz, 0.0..=3200f32.ln()).show_value(false)).accessible_name(tl!("Zoom")).changed() {
        w.preview.zoom = lz.exp() / 100.0;
    }
}

/// Draws the workspace (a full-window layer over the app).
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // An Apply (or anything else) changed the document: refresh.
    let rev = app.session.active().map(|s| s.revision);
    if app.distort.caf.as_ref().is_some_and(|w| Some(w.revision) != rev) {
        refresh(app);
    }
    upload(app, ctx);
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let mut action: Option<&str> = None;
    let mut events: Vec<ToolEvent> = Vec::new();
    let mut mods = egui::Modifiers::NONE;
    let mut new_sampling: Option<&str> = None;
    let mut fill_btn: Option<Value> = None;
    let mut reset_what: Option<&str> = None;
    let mut rebase_needed = false;
    let mut dragging = false;
    egui::Area::new(egui::Id::unique("caf-workspace")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(w) = app.distort.caf.as_mut() else { return };
        let before = w.opts.clone();
        w.texture_limit = ctx.input(|i| i.max_texture_side);
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        // Title strip.
        let title = ERect::from_min_size(full.min, vec2(full.width(), 30.0));
        painter.rect_filled(title, 0.0, t.dock);
        painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
        let pct = if w.zoom > 0.0 { w.zoom * 100.0 } else { 100.0 };
        painter.text(
            title.center(),
            Align2::CENTER_CENTER,
            format!("{} ({}, {:.0}%)", tl!("Content-Aware Fill…").trim_end_matches('…'), w.layer_name, pct),
            FontId::proportional(13.0),
            t.text,
        );
        // Options bar.
        let bar = ERect::from_min_size(pos2(full.left(), title.bottom()), vec2(full.width(), BAR_H));
        painter.rect_filled(bar, 0.0, t.dock);
        painter.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, t.separator));
        let mut bui =
            ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(LEFT_W + 8.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        caf_options_bar(&mut bui, w, &mut fill_btn, &mut reset_what);

        let body = ERect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        // Left tool strip.
        let left = ERect::from_min_size(pos2(full.left(), title.bottom()), vec2(LEFT_W, full.height() - title.height()));
        painter.rect_filled(left, 0.0, t.dock);
        painter.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
        let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(6.0, 8.0))));
        strip.spacing_mut().item_spacing.y = 4.0;
        for tool in [CafTool::Brush, CafTool::Lasso, CafTool::Hand, CafTool::Zoom] {
            if crate::icons::button(&mut strip, tool_icon(tool), 34.0, w.tool == tool, &tool_tip(tool)).clicked() {
                w.tool = tool;
            }
        }
        // Right: the Content-Aware Fill panel.
        let right = ERect::from_min_size(pos2(body.right() - PANEL_W, body.top()), vec2(PANEL_W, body.height()));
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        let mut props =
            ui.new_child(egui::UiBuilder::new().max_rect(ERect::from_min_max(right.min + vec2(14.0, 12.0), pos2(right.right() - 14.0, right.bottom() - 60.0))));
        egui::ScrollArea::vertical()
            .id_salt("caf-props")
            .show(&mut props, |ui| caf_props_panel(ui, w, &t, &mut reset_what, &mut new_sampling, &mut rebase_needed));
        // Footer, in Photoshop's order on every platform: reset all at the left, then OK, Apply,
        // Cancel against the right edge.
        let foot = ERect::from_min_size(pos2(right.left() + 10.0, right.bottom() - 48.0), vec2(PANEL_W - 20.0, 32.0));
        let mut fb = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if widgets::secondary_button(&mut fb, tl!("Cancel"), 70.0).clicked() {
            action = Some("cancel");
        }
        if widgets::secondary_button(&mut fb, tl!("Apply"), 70.0).clicked() {
            action = Some("apply");
        }
        if widgets::primary_button(&mut fb, tl!("OK"), 64.0).clicked() {
            action = Some("ok");
        }
        if crate::icons::button(&mut fb, "undo-2", 24.0, false, tl!("Reset")).clicked() {
            reset_what = Some("all");
        }
        // The Preview panel.
        let pv_rect = ERect::from_min_size(pos2(right.left() - PREVIEW_W, body.top()), vec2(PREVIEW_W, body.height()));
        let uv = ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        caf_preview_panel(ui, &painter, &t, w, pv_rect);
        // The document view.
        let area = ERect::from_min_max(pos2(left.right(), body.top()), pos2(pv_rect.left(), body.bottom()));
        painter.rect_filled(area, 0.0, t.canvas);
        let (cw, chh) = (w.canvas.width() as f32, w.canvas.height() as f32);
        if w.zoom <= 0.0 {
            w.zoom = ((area.width() - 40.0) / cw).min((area.height() - 40.0) / chh).clamp(0.01, 4.0);
        }
        let resp = ui.interact(area, egui::Id::unique("caf-view"), Sense::click_and_drag());
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0
                && let Some(hp) = resp.hover_pos()
            {
                let v = View { area, zoom: w.zoom, center: w.center };
                let before = v.to_doc(hp);
                w.zoom = (w.zoom * (scroll / 200.0).exp()).clamp(0.01, 32.0);
                let after = View { area, zoom: w.zoom, center: w.center }.to_doc(hp);
                w.center = [w.center[0] + before[0] - after[0], w.center[1] + before[1] - after[1]];
            }
        }
        let v = View { area, zoom: w.zoom, center: w.center };
        let clip = painter.with_clip_rect(area);
        let img = v.rect(w.canvas);
        widgets::checker(&clip, img.intersect(area), 8.0);
        if let Some(tex) = &w.image_tex {
            clip.image(tex.id(), img, uv, Color32::WHITE);
        }
        if let Some(tex) = &w.overlay_tex {
            // The proxy's blocks are `k` document px; its last row and column may reach past the
            // canvas, so the overlay is clipped to it.
            let pr = ERect::from_min_size(img.min, vec2((w.pw * w.k) as f32 * v.zoom, (w.ph * w.k) as f32 * v.zoom));
            clip.with_clip_rect(img.intersect(area)).image(tex.id(), pr, uv, Color32::WHITE);
        }
        // The fill area's outline (marching ants: black dashes on white).
        let phase = (ui.input(|i| i.time) * 8.0) as usize;
        match &w.outline {
            Some(edges) => {
                for e in edges {
                    let (a, b) = (v.to_screen([f64::from(e[0][0]), f64::from(e[0][1])]), v.to_screen([f64::from(e[1][0]), f64::from(e[1][1])]));
                    let dark = ((e[0][0] + e[0][1]) as usize / 4 + phase).is_multiple_of(2);
                    clip.line_segment([a, b], Stroke::new(1.0, if dark { Color32::BLACK } else { Color32::WHITE }));
                }
            }
            None => {
                let b = v.rect(bounds(&w.base));
                clip.rect_stroke(b, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Outside);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
        if let Some((_, pts)) = &w.lasso
            && pts.len() > 1
        {
            let line: Vec<Pos2> = pts.iter().map(|p| v.to_screen(*p)).collect();
            clip.add(egui::Shape::line(line, Stroke::new(1.0, t.accent)));
        }
        if let Some(hp) = resp.hover_pos() {
            if w.tool == CafTool::Brush {
                let r = w.brush_size as f32 * 0.5 * v.zoom;
                clip.circle_stroke(hp, r, Stroke::new(1.5, Color32::BLACK));
                clip.circle_stroke(hp, r, Stroke::new(0.75, Color32::WHITE));
            }
            w.hover = Some(v.to_doc(hp));
        }
        clip.rect_stroke(img, 0.0, Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
        // Mouse → tools (Space, the middle button or the Hand pan).
        let pan = ui.input(|i| i.key_down(egui::Key::Space) || i.pointer.middle_down()) || w.tool == CafTool::Hand;
        if pan && resp.dragged() {
            let dl = resp.drag_delta();
            w.center = [w.center[0] - f64::from(dl.x / w.zoom), w.center[1] - f64::from(dl.y / w.zoom)];
        } else if !pan
            && resp.is_pointer_button_down_on()
            && let Some(pp) = resp.interact_pointer_pos()
        {
            // The press is the first point; the frames before egui's drag threshold and the drag
            // itself continue it (a second `Down` would start the stroke over, dropping them).
            let q = v.to_doc(pp);
            mods = ui.input(|i| i.modifiers);
            if w.pressed {
                events.push(ToolEvent::Move { x: q[0], y: q[1], pressure: 1.0 });
                ctx.request_repaint();
            } else {
                w.pressed = true;
                events.push(ToolEvent::Down { x: q[0], y: q[1], pressure: 1.0 });
            }
            dragging = true;
        }
        if w.pressed && !resp.is_pointer_button_down_on() {
            w.pressed = false;
            let q = w.hover.unwrap_or(w.center);
            events.push(ToolEvent::Up { x: q[0], y: q[1] });
            dragging = false;
        }
        // Settings changed in the panel: the overlay and the preview follow.
        if w.opts != before {
            w.overlay_dirty = true;
            w.stale();
        }
    });
    if let Some(s) = new_sampling {
        set_sampling(app, s);
    }
    if let Some(r) = reset_what {
        reset(app, r);
    }
    if let Some(e) = fill_btn {
        fill_edit(app, e);
    }
    if rebase_needed {
        rebase(app);
    }
    for ev in events {
        pointer(app, ev, mods);
    }
    drive_preview(app, ctx, dragging);
    match action {
        Some("ok") => commit(app),
        Some("cancel") => cancel(app),
        Some("apply") => {
            if let Err(e) = apply(app) {
                report(app, e);
            }
        }
        _ => {}
    }
}
