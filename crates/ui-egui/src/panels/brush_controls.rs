use super::{Align2, BlendMode, Color32, PhotocraftApp, Rect, Sense, Stroke, StrokeKind, Tokens, c32, icons, json, pos2, vec2, widgets};

/// Photoshop's brush preset picker chip: a soft/hard round tip preview with the size underneath.
/// Draw a round brush tip preview (hard core fading to a soft edge).
fn brush_tip(p: &egui::Painter, c: egui::Pos2, rad: f32, hardness: f32, color: Color32) {
    let inner = rad * hardness.clamp(0.05, 1.0);
    let steps = 8;
    for k in (0..=steps).rev() {
        let f = k as f32 / steps as f32;
        let rr = inner + (rad - inner) * f;
        let a = if hardness >= 0.99 { 1.0 } else { (1.0 - f).powf(1.5) };
        p.circle_filled(c, rr, color.gamma_multiply(a));
    }
}

/// A painting tool's Mode dropdown over `modes` (`None` is Clear); the wheel over it steps the
/// mode. Returns the mode picked this frame.
pub(super) fn tool_mode_dropdown(ui: &mut egui::Ui, id: &str, modes: &[Option<BlendMode>], current: Option<BlendMode>) -> Option<Option<BlendMode>> {
    let opts: Vec<(Option<BlendMode>, &str)> = modes.iter().map(|m| (*m, m.map_or("Clear", BlendMode::label))).collect();
    let mut mode = current;
    let chosen = widgets::dropdown_wheel_hovered(ui, id, tl!("Mode"), &mut mode, &opts, 96.0).0;
    chosen.last().copied()
}

/// The Brush's or Pencil's Mode dropdown: the blend modes, with Photoshop's painting-only Clear
/// after Dissolve (#3439). Clear erases with the brush (`clear`); it shows while chosen, or while
/// `~` is held (`held`). The wheel over it steps the mode, as over the Layers panel's (#2061).
pub(super) fn paint_mode_dropdown(ui: &mut egui::Ui, id: &str, b: &mut photocraft_engine::BrushSettings, clear: &mut bool, held: bool) {
    let mut opts: Vec<(Option<BlendMode>, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (Some(*m), m.label())).collect();
    let at = opts.iter().position(|o| o.0 == Some(BlendMode::Dissolve)).map_or(opts.len(), |i| i + 1);
    opts.insert(at, (None, "Clear"));
    let mut mode = if *clear || held { None } else { Some(b.mode) };
    if !widgets::dropdown_wheel_hovered(ui, id, tl!("Mode"), &mut mode, &opts, 96.0).0.is_empty() {
        *clear = mode.is_none();
        if let Some(m) = mode {
            b.mode = m;
        }
    }
}

/// Options-bar Smoothing % (the brush's stroke smoothing; the live stroke and the commit use it).
pub(super) fn smoothing_field(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, width: f32) {
    let mut sm = (b.smoothing.amount * 100.0).round();
    if widgets::value_field_named(ui, tl!("Smoothing"), &mut sm, 0.0..=100.0, "%", width).changed() {
        b.smoothing.enabled = true;
        b.smoothing.amount = (sm / 100.0).clamp(0.0, 1.0);
    }
}

/// Options-bar gear beside Smoothing: Photoshop's smoothing options popup, edited on the brush
/// like the Smoothing % (so they stay per tool and reach the live stroke and the commit).
pub(super) fn smoothing_options(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings) {
    let resp = icons::button(ui, "settings", 24.0, false, tl!("Set additional smoothing options"));
    let resp = crate::brush_picker::named(resp, "Set additional smoothing options");
    egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let s = &mut b.smoothing;
        widgets::checkbox(ui, &mut s.pulled_string, tl!("Pulled String Mode"));
        widgets::checkbox(ui, &mut s.catch_up, tl!("Stroke Catch-up"));
        widgets::checkbox(ui, &mut s.catch_up_on_end, tl!("Catch-up on Stroke End"));
        widgets::checkbox(ui, &mut s.adjust_for_zoom, tl!("Adjust for Zoom"));
    });
}

/// Mixer Brush: the Current Brush Load swatch shows the paint the next stroke starts with (an
/// empty swatch when the brush is clean). Its menu holds Load Brush and Clean Brush.
pub(super) fn mixer_load_swatch(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(40.0, 26.0), Sense::hover());
    let resp = ui.interact(r, ui.scope_id().with("mixer-brush-load"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let chip = Rect::from_min_size(pos2(r.left() + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
    let p = ui.painter();
    match app.session.tools.mixer.next_load(app.session.tools.foreground) {
        Some(c) => {
            p.rect_filled(chip, 2.0, c32(c));
        }
        None => {
            p.rect_filled(chip, 2.0, t.field);
            p.line_segment([chip.left_bottom(), chip.right_top()], Stroke::new(1.0, t.text_faint));
        }
    }
    p.rect_stroke(chip, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    icons::paint(ui, Rect::from_center_size(pos2(r.right() - 9.0, r.center().y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Current Brush Load")));
    let resp = resp.on_hover_text(tl!("Current Brush Load"));
    let mut run = None;
    egui::Popup::menu(&resp).show(|ui| {
        if ui.button(tl!("Load Brush")).clicked() {
            run = Some("brush.mixer.load");
        }
        if ui.button(tl!("Clean Brush")).clicked() {
            run = Some("brush.mixer.clean");
        }
    });
    if let Some(id) = run
        && let Err(e) = app.run(id, json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Options-bar brush chip: a click shows or hides Photoshop's Brush Preset picker below it (size,
/// hardness, the preset library), the one a right-click on the canvas opens.
pub(super) fn brush_preset_chip(ui: &mut egui::Ui, b: &photocraft_engine::BrushSettings, state: &mut crate::state::UiState) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(44.0, 30.0), Sense::hover());
    let resp = ui.interact(r, crate::brush_picker::chip_id(), Sense::click());
    if resp.hovered() || state.brush_picker.is_some() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let c = pos2(r.left() + 14.0, r.top() + 11.0);
    // Photoshop's white tip vanishes on light chrome; light themes use their icon colour.
    let tip = if t.dark() { Color32::WHITE } else { t.icon };
    brush_tip(ui.painter(), c, 7.0, b.hardness, tip);
    ui.painter().text(pos2(c.x, r.bottom() - 5.0), Align2::CENTER_CENTER, format!("{}", b.size.round() as i64), egui::FontId::proportional(9.5), t.text_dim);
    icons::paint(ui, Rect::from_center_size(pos2(r.right() - 9.0, c.y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Brush Preset picker")));
    if resp.on_hover_text(tl!("Brush Preset picker")).clicked() {
        if state.brush_picker.is_some() {
            crate::brush_picker::close(state);
        } else {
            crate::brush_picker::open(state, ui.ctx(), [r.left(), r.bottom() + 2.0]);
        }
    }
}
