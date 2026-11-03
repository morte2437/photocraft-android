use super::open::{Target, cancel, commit_open_raw, open_raw, raw_stage};
use super::sidecar::{load_settings, report_file_error, save_settings};
use super::{
    Align2, BANDS, CameraRaw, CameraRawDialog, Color32, ERect, FontId, PANEL_W, PhotocraftApp, Pos2, Sense, Stroke, Tokens, ToneZone, Value, Wheel, curve_lut,
    json, pos2, vec2, widgets,
};

const RAW_UNDO_LIMIT: usize = 40;

pub(super) fn remember_edit(d: &mut CameraRawDialog) {
    if d.undo.len() == RAW_UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.undo.push(d.params.clone());
    d.redo.clear();
}

pub(super) fn undo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(prev) = d.undo.pop() else { return false };
    d.redo.push(d.params.clone());
    if d.params.point_curve != prev.point_curve {
        d.curve_state = Default::default();
    }
    d.params = prev;
    d.dirty = true;
    d.edit_origin = None;
    true
}

pub(super) fn redo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(next) = d.redo.pop() else { return false };
    d.undo.push(d.params.clone());
    if d.params.point_curve != next.point_curve {
        d.curve_state = Default::default();
    }
    d.params = next;
    d.dirty = true;
    d.edit_origin = None;
    true
}

fn history_keys(d: &mut CameraRawDialog, ctx: &egui::Context) {
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        redo_edit(d);
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        undo_edit(d);
    }
}

pub(super) fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            d.params.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&d.params).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
        Target::OpenRaw => {
            let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
            let (params, original) = (d.params.clone(), d.original.clone());
            commit_open_raw(app, &params, &raw, Some(&original))?
        }
    };
    app.camera_raw = None;
    Ok(result)
}

/// The open-time demosaic choice (at the top of Detail). Photoshop's Camera Raw picks one method
/// for every file; here it is the user's. Open re-develops with it; the preview keeps the method
/// the file was opened with (the differences are finer than the preview's pixels).
fn demosaic_row(ui: &mut egui::Ui, m: &mut photocraft_io::raw::Demosaic) {
    use photocraft_io::raw::Demosaic;
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", "Demosaic")).on_hover_text(tl!(
            "How the camera's colour filter mosaic is filled in. It is applied when the file opens; the preview shows the method it was opened with."
        ));
        let options = [
            (Demosaic::Ahd, "AHD (adaptive homogeneity)", "Hirakawa–Parks: the cleanest edges and the least colour fringing, the slowest."),
            (Demosaic::Mhc, "Malvar–He–Cutler", "Gradient-corrected interpolation: sharp and fast, with some colour fringing on fine detail."),
            (Demosaic::Bilinear, "Bilinear", "The fastest and the softest, with colour fringing on fine detail."),
        ];
        widgets::dropdown_with_tooltips(ui, "camera-raw-demosaic", tl!("Demosaic"), m, &options, 180.0);
    });
    ui.add_space(4.0);
}

fn temp_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(200, 200, 200), Color32::from_rgb(235, 200, 60)]
}

fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 190, 80), Color32::from_rgb(200, 200, 200), Color32::from_rgb(210, 80, 200)]
}

/// One labelled slider; marks the dialog dirty when it moves.
/// The track of Color Mixer band `k` on `tab` (0 Hue, 1 Saturation, 2 Luminance). A Hue slider
/// turns its band towards the neighbouring bands, so its track runs from the previous band's hue
/// through its own to the next one's (Reds: magenta, red, orange; #9550); Saturation and Luminance
/// run from grey to the band's colour.
pub(super) fn mixer_gradient(tab: usize, k: usize) -> Vec<Color32> {
    let bands = &photocraft_algo::camera_raw::HSL_BANDS;
    let hue = |i: usize| widgets::hue_color(bands.get(i % bands.len()).copied().unwrap_or(0.0));
    if tab == 0 { vec![hue(k + bands.len() - 1), hue(k), hue(k + 1)] } else { vec![Color32::from_gray(128), hue(k)] }
}

fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) {
    let _ = slider(ui, dirty, label, v, range, grad);
}

fn slider(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) -> egui::Response {
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", label), v, range, "", grad);
    if r.changed() {
        *dirty = true;
    }
    if r.double_clicked() {
        *v = 0.0;
        *dirty = true;
    }
    r
}

/// A Light tone slider; Alt-dragging it shows that zone's channel clipping on the preview.
fn tone_row(ui: &mut egui::Ui, dirty: &mut bool, alt: &mut Option<ToneZone>, zone: ToneZone, v: &mut f32) {
    let r = slider(ui, dirty, zone.name(), v, -zone.limit()..=zone.limit(), None);
    if r.dragged() && ui.input(|i| i.modifiers.alt) {
        *alt = Some(zone);
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title)).font(FontId::proportional(13.0)))
        .id_salt(("camera-raw-section", title))
        .default_open(open)
        .show(ui, body);
}

fn wheel(ui: &mut egui::Ui, dirty: &mut bool, title: &str, w: &mut Wheel) {
    widgets::section_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title));
    let hs = widgets::hue_stops();
    row(ui, dirty, "Hue", &mut w.hue, 0.0..=360.0, Some(&hs));
    row(ui, dirty, "Saturation", &mut w.sat, 0.0..=100.0, None);
    row(ui, dirty, "Luminance", &mut w.lum, -100.0..=100.0, None);
}

/// Small point-curve editor (master channel): click to add, drag to move, right-click to delete.
fn curve_editor(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool, state: &mut crate::state::PointCurveState) -> ERect {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment([pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], Stroke::new(1.0, t.separator));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    // The identity handles are presentation until a gesture actually edits them.
    let mut points = if p.point_curve.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { p.point_curve.clone() };
    if crate::point_curve::interact(ui, &resp, rect, &mut points, state).changed {
        p.point_curve = points.clone();
        *dirty = true;
    }
    // Curve (point curve after the parametric one).
    let lut = curve_lut(&points, 256);
    let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, value)| to_screen(i as f32, value * 255.0)).collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, t.text)));
    for (i, c) in points.iter().enumerate() {
        let center = to_screen(c[0], c[1]);
        if state.selected == Some(i) {
            painter.circle_filled(center, 3.0, t.accent);
        }
        painter.circle_stroke(center, 4.0, Stroke::new(1.5, t.accent));
    }
    rect
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.camera_raw.is_none()
        && let Some(raw) = app.pending_raw_open.take()
        && let Err(e) = open_raw(app, ctx, raw)
    {
        // The document stays open, developed with the defaults.
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let Some(d) = app.camera_raw.as_ref() else { return };
    // Where the platform has no extra windows (the web build), draw over the main window.
    if ctx.embed_viewports() {
        draw(app, ctx, false);
        return;
    }
    let window_title = d.window_title();
    // Camera Raw is modal, as in Photoshop: the main window is dimmed and takes no input while it
    // is open (in a separate window, or drawn over it where there are no extra windows).
    let main = ctx.content_rect();
    egui::Area::new(egui::Id::unique("camera-raw-modal")).order(egui::Order::Foreground).fixed_pos(main.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(main.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Tokens::get(ui.ctx()).scrim);
    });
    // A window of its own (title bar, moved and resized like any window), like Adobe Camera Raw.
    let builder = crate::os_window::builder().with_title(window_title).with_inner_size([1360.0, 900.0]).with_min_inner_size([900.0, 600.0]);
    let mut close = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("camera-raw-window"), builder, |ui, class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            close = true;
        }
        draw(app, ui.ctx(), class != egui::ViewportClass::EmbeddedWindow);
    });
    if close && app.camera_raw.is_some() {
        cancel(app);
    }
}

impl CameraRawDialog {
    /// "Camera Raw (name)" for the open-time dialog, "Camera Raw Filter (layer)" otherwise.
    fn window_title(&self) -> String {
        if self.target == Target::OpenRaw {
            crate::i18n::fmt(tl!("Camera Raw ({name})"), &[("name", &self.layer_name)])
        } else {
            crate::i18n::fmt(tl!("Camera Raw Filter ({layer})"), &[("layer", &self.layer_name)])
        }
    }
}

/// The dialog's contents over the whole of `ctx`'s window. `own_window`: the OS window already
/// shows the title, so none is painted.
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, own_window: bool) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let current_display = app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok());
    let mut action: Option<&str> = None;
    egui::Area::new(egui::Id::unique("camera-raw-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.camera_raw.as_mut() else { return };
        history_keys(d, ctx);
        let pointer = ctx.input(|i| i.pointer.primary_down());
        if pointer && d.edit_origin.is_none() {
            d.edit_origin = Some(d.params.clone());
        }
        if let Some(display) = &current_display
            && display.key != d.display.key
        {
            d.display = display.clone();
            d.dirty = true;
        }
        d.curve_rect = None;
        d.scope.vectorscope_rect = None;
        if let Err(e) = d.poll_render(ctx) {
            d.preview_error = Some((e.clone(), d.preview_revision));
            app.ui.status = e;
            app.ui.status_error = true;
        }
        d.request_render(ctx);
        d.clear_recovered_error(&mut app.ui);
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        let title = ERect::from_min_size(full.min, vec2(full.width(), if own_window { 0.0 } else { 30.0 }));
        if !own_window {
            painter.rect_filled(title, 0.0, t.dock);
            painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
            painter.text(title.center(), Align2::CENTER_CENTER, d.window_title(), FontId::proportional(13.0), t.text);
        }
        let ok_label = if d.target == Target::OpenRaw { tl!("Open") } else { tl!("OK") };
        let footer_h = 48.0;
        let body = ERect::from_min_max(pos2(full.left(), title.bottom()), pos2(full.right(), full.bottom() - footer_h));
        let animated_zoom = app.session.prefs().general.animated_zoom;
        let scope = &mut app.ui.camera_raw_scope;
        // Clip both pixels and scope overlays before painting the settings dock/footer.
        let preview_body = ERect::from_min_max(body.min, pos2((body.right() - PANEL_W).max(body.left() + 1.0), body.bottom()));
        let view = ERect::from_min_max(
            preview_body.min + vec2(16.0, 16.0),
            pos2((preview_body.right() - 16.0).max(preview_body.left() + 17.0), (preview_body.bottom() - 48.0).max(preview_body.top() + 17.0)),
        );
        d.viewport = Some(view);
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(view));
        preview.set_clip_rect(view.intersect(ui.clip_rect()));
        preview.painter().rect_filled(view, 0.0, t.canvas);
        let (r, response, panning, zoom_box) =
            crate::camera_raw_preview_ui::interact(&mut preview, navigation, view, size, &mut scope.sampler_tool, animated_zoom);
        let tex = if d.show_before { d.before_tex.as_ref() } else { d.tex.as_ref() };
        if let Some(tex) = tex {
            // Never tile a checker across the offscreen extent of a highly zoomed image.
            widgets::checker(preview.painter(), r.intersect(view), 8.0);
            preview.painter().image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let raw_tone = d.raw_curve.is_some() && !raw_stage(&d.params).is_neutral();
            let detail_needed = !raw_tone && (r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32);
            d.detail.paint(&preview, r, d.preview_revision, d.show_before, &d.params, detail_needed, &d.display);
            if d.detail.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            d.scope.preview_rect = Some(r);
            crate::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
        }
        if let Some(region) = zoom_box {
            preview.painter().rect_stroke(region, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        let toolbar = ERect::from_min_max(pos2(preview_body.left() + 16.0, view.bottom() + 8.0), preview_body.right_bottom() - vec2(16.0, 4.0));
        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(toolbar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        nav_ui.set_clip_rect(preview_body.intersect(ui.clip_rect()));
        crate::camera_raw_preview_ui::toolbar(&mut nav_ui, navigation, view, size, &mut scope.sampler_tool);
        if d.detail.pending() {
            nav_ui.spinner();
        }
        if let Some(error) = &d.detail.error {
            nav_ui.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(t.text_faint)).on_hover_text(error);
        }
        // Panels.
        let right = ERect::from_min_max(pos2(body.right() - PANEL_W, body.top()), body.max);
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(14.0, 10.0))));
        // This header is outside the settings scroll area and follows the same Before/After
        // selector as the texture. Both derive from the same completed preview revision.
        crate::camera_raw_scope_ui::header(&mut props, d, scope);
        d.scope.alt_tone = None;
        let mut dirty = false;
        egui::ScrollArea::vertical().id_salt("camera-raw-props").show(&mut props, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let p = &mut d.params;
            let proxy = &d.proxy;
            let alt = &mut d.scope.alt_tone;
            let (ts, tn) = (temp_stops(), tint_stops());
            section(ui, "Light", true, |ui| {
                // Auto: Camera Raw's one-click tone from the histogram; the sliders show what it
                // chose, so each can be adjusted afterwards.
                if ui.button(tl!("Auto")).on_hover_text(tl!("Set the tone sliders from the image")).clicked() {
                    photocraft_algo::camera_raw::auto_tone(proxy, p);
                    dirty = true;
                }
                tone_row(ui, &mut dirty, alt, ToneZone::Exposure, &mut p.exposure);
                row(ui, &mut dirty, "Contrast", &mut p.contrast, -100.0..=100.0, None);
                tone_row(ui, &mut dirty, alt, ToneZone::Highlights, &mut p.highlights);
                tone_row(ui, &mut dirty, alt, ToneZone::Shadows, &mut p.shadows);
                tone_row(ui, &mut dirty, alt, ToneZone::Whites, &mut p.whites);
                tone_row(ui, &mut dirty, alt, ToneZone::Blacks, &mut p.blacks);
            });
            section(ui, "Color", false, |ui| {
                widgets::section_label(ui, tl!("White Balance: As Shot"));
                row(ui, &mut dirty, "Temperature", &mut p.temperature, -100.0..=100.0, Some(&ts));
                row(ui, &mut dirty, "Tint", &mut p.tint, -100.0..=100.0, Some(&tn));
                row(ui, &mut dirty, "Vibrance", &mut p.vibrance, -100.0..=100.0, None);
                row(ui, &mut dirty, "Saturation", &mut p.saturation, -100.0..=100.0, None);
            });
            section(ui, "Effects", false, |ui| {
                row(ui, &mut dirty, "Texture", &mut p.texture, -100.0..=100.0, None);
                row(ui, &mut dirty, "Clarity", &mut p.clarity, -100.0..=100.0, None);
                row(ui, &mut dirty, "Dehaze", &mut p.dehaze, -100.0..=100.0, None);
                widgets::section_label(ui, tl!("Vignetting"));
                let mut style = p.vignette_style.clone();
                if widgets::dropdown(
                    ui,
                    "cr-vig-style",
                    tl!("Vignetting"),
                    &mut style,
                    &[
                        ("highlightPriority".to_string(), tl!("Highlight Priority")),
                        ("colorPriority".to_string(), tl!("Color Priority")),
                        ("paintOverlay".to_string(), tl!("Paint Overlay")),
                    ],
                    200.0,
                ) {
                    p.vignette_style = style;
                    dirty = true;
                }
                row(ui, &mut dirty, "Amount", &mut p.vignette_amount, -100.0..=100.0, None);
                row(ui, &mut dirty, "Midpoint", &mut p.vignette_midpoint, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roundness", &mut p.vignette_roundness, -100.0..=100.0, None);
                row(ui, &mut dirty, "Feather", &mut p.vignette_feather, 0.0..=100.0, None);
                row(ui, &mut dirty, "Highlights", &mut p.vignette_highlights, 0.0..=100.0, None);
                widgets::section_label(ui, tl!("Grain"));
                row(ui, &mut dirty, "Amount", &mut p.grain_amount, 0.0..=100.0, None);
                row(ui, &mut dirty, "Size", &mut p.grain_size, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roughness", &mut p.grain_roughness, 0.0..=100.0, None);
            });
            section(ui, "Curve", false, |ui| {
                d.curve_rect = Some(curve_editor(ui, p, &mut dirty, &mut d.curve_state));
                row(ui, &mut dirty, "Highlights", &mut p.curve_highlights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Lights", &mut p.curve_lights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Darks", &mut p.curve_darks, -100.0..=100.0, None);
                row(ui, &mut dirty, "Shadows", &mut p.curve_shadows, -100.0..=100.0, None);
            });
            section(ui, "Color Mixer", false, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, source) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
                        let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", source);
                        let response = widgets::pill_tab(ui, name, d.mixer_tab == i);
                        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), d.mixer_tab == i, name));
                        if response.clicked() {
                            d.mixer_tab = i;
                        }
                    }
                });
                let arr = match d.mixer_tab {
                    0 => &mut p.hsl_hue,
                    1 => &mut p.hsl_sat,
                    _ => &mut p.hsl_lum,
                };
                for (k, name) in BANDS.iter().enumerate() {
                    let grad = mixer_gradient(d.mixer_tab, k);
                    row(ui, &mut dirty, name, &mut arr[k], -100.0..=100.0, Some(&grad));
                }
            });
            section(ui, "Color Grading", false, |ui| {
                wheel(ui, &mut dirty, "Shadows", &mut p.grade_shadows);
                wheel(ui, &mut dirty, "Midtones", &mut p.grade_midtones);
                wheel(ui, &mut dirty, "Highlights", &mut p.grade_highlights);
                wheel(ui, &mut dirty, "Global", &mut p.grade_global);
                row(ui, &mut dirty, "Blending", &mut p.grade_blending, 0.0..=100.0, None);
                row(ui, &mut dirty, "Balance", &mut p.grade_balance, -100.0..=100.0, None);
            });
            section(ui, "Detail", false, |ui| {
                if let Some(raw) = d.raw_open.as_mut() {
                    demosaic_row(ui, &mut raw.demosaic);
                }
                widgets::section_label(ui, tl!("Sharpening"));
                row(ui, &mut dirty, "Amount", &mut p.sharpen_amount, 0.0..=150.0, None);
                row(ui, &mut dirty, "Radius", &mut p.sharpen_radius, 0.5..=3.0, None);
                row(ui, &mut dirty, "Detail", &mut p.sharpen_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Masking", &mut p.sharpen_masking, 0.0..=100.0, None);
                widgets::section_label(ui, tl!("Noise Reduction"));
                row(ui, &mut dirty, "Luminance", &mut p.noise_luminance, 0.0..=100.0, None);
                row(ui, &mut dirty, "Luminance Detail", &mut p.noise_luminance_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color", &mut p.noise_color, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color Detail", &mut p.noise_color_detail, 0.0..=100.0, None);
            });
            // Camera Raw's Calibration panel: a tint for the shadows and each primary's hue and
            // saturation (Process Version is implied: 2012 and later).
            section(ui, "Calibration", false, |ui| {
                widgets::section_label(ui, tl!("Shadows"));
                row(ui, &mut dirty, "Tint", &mut p.shadow_tint, -100.0..=100.0, Some(&tn));
                for (i, name) in ["Red Primary", "Green Primary", "Blue Primary"].iter().enumerate() {
                    widgets::section_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", name));
                    ui.push_id(name, |ui| {
                        row(ui, &mut dirty, "Hue", &mut p.calibration_hue[i], -100.0..=100.0, None);
                        row(ui, &mut dirty, "Saturation", &mut p.calibration_sat[i], -100.0..=100.0, None);
                    });
                }
            });
        });
        crate::camera_raw_scope_ui::floating(ctx, d, scope, right);
        crate::camera_raw_scope_ui::shortcuts(ctx, scope);
        d.dirty |= dirty;
        if d.dirty {
            ctx.request_repaint();
        }
        if d.render_pending() {
            // The worker asks for a repaint when it is done; this catches a missed wake-up.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Footer.
        let foot = ERect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);
        painter.rect_filled(foot, 0.0, t.dock);
        painter.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 9.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if let Some(role) = widgets::dialog_buttons(
            &mut fu,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, ok_label, 90.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 90.0),
            ],
        ) {
            action = Some(if role == widgets::ButtonRole::Default { "ok" } else { "cancel" });
        }
        fu.add_space(12.0);
        widgets::checkbox(&mut fu, &mut d.show_before, tl!("Before (Y)"));
        fu.label(egui::RichText::new(format!("{:.0} ms", d.render_ms)).color(t.text_faint));
        fu.add_space(12.0);
        if fu.button(tl!("Save Settings…")).on_hover_text(tl!("Save these settings as a Camera Raw .xmp file")).clicked() {
            action = Some("save");
        }
        if fu.button(tl!("Load Settings…")).on_hover_text(tl!("Load settings from a Camera Raw .xmp file")).clicked() {
            action = Some("load");
        }
        if !ctx.input(|i| i.pointer.primary_down())
            && let Some(origin) = d.edit_origin.take()
            && origin != d.params
        {
            let edited = std::mem::replace(&mut d.params, origin);
            remember_edit(d);
            d.params = edited;
        }
    });
    crate::camera_raw_scope_ui::persist(app, ctx);
    if ctx.input(|i| i.key_pressed(egui::Key::Y))
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.show_before = !d.show_before;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some("cancel");
    }
    match action {
        Some("ok") => {
            if let Err(e) = commit(app) {
                app.ui.status = e;
            }
        }
        Some("cancel") => cancel(app),
        Some("save") => {
            let r = save_settings(app);
            report_file_error(app, r);
        }
        Some("load") => {
            let r = load_settings(app);
            report_file_error(app, r);
        }
        _ => {}
    }
}
