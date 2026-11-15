use super::{Align2, BlendMode, Color32, PhotocraftApp, Rect, Sense, Stroke, StrokeKind, Tokens, c32, icons, json, pos2, starts_blend_group, vec2, widgets};

/// A painting tool's Mode dropdown over `modes` (`None` is Clear); the wheel over it steps the
/// mode. Returns the mode picked this frame. A list of every blend mode draws the Layers panel's
/// group lines (#5532; Clear starts none); a short one (the Healing Brush's) draws none.
/// `threshold` names Normal "Threshold", as the reference does in an Indexed Color document.
pub(super) fn tool_mode_dropdown(
    ui: &mut egui::Ui,
    id: &str,
    modes: &[Option<BlendMode>],
    current: Option<BlendMode>,
    threshold: bool,
) -> Option<Option<BlendMode>> {
    let label = |m: &Option<BlendMode>| match m {
        Some(BlendMode::Normal) if threshold => tl!("Threshold"),
        _ => m.map_or("Clear", BlendMode::label),
    };
    let opts: Vec<(Option<BlendMode>, &str)> = modes.iter().map(|m| (*m, label(m))).collect();
    let grouped = BlendMode::LAYER_MODES.iter().all(|m| modes.contains(&Some(*m)));
    let mut mode = current;
    let chosen = widgets::dropdown_wheel_grouped(ui, id, tl!("Mode"), &mut mode, &opts, 96.0, |m| grouped && m.is_some_and(starts_blend_group)).0;
    chosen.last().copied()
}

/// The Brush's or Pencil's Mode dropdown: the blend modes, with Photoshop's painting-only Clear
/// after Dissolve (#3439). Clear erases with the brush (`clear`); it shows while chosen, or while
/// `~` is held (`held`). The wheel over it steps the mode, as over the Layers panel's (#2061).
pub(super) fn paint_mode_dropdown(ui: &mut egui::Ui, id: &str, b: &mut photocraft_engine::BrushSettings, clear: &mut bool, held: bool) {
    let opts = crate::brush_mode_menu::choices();
    let mut mode = if *clear || held {
        (None, false)
    } else if b.behind {
        (None, true)
    } else {
        (Some(b.mode), false)
    };
    if !widgets::dropdown_wheel_grouped(ui, id, tl!("Mode"), &mut mode, &opts, 96.0, |m| m.0.is_some_and(starts_blend_group)).0.is_empty() {
        *clear = mode == (None, false);
        b.behind = mode.1;
        if mode.1 {
            b.mode = BlendMode::Normal;
        }
        if let Some(m) = mode.0 {
            b.mode = m;
        }
    }
}

/// The Pattern Stamp's Mode dropdown: the blend modes with Behind after Dissolve and no Clear, as
/// the reference application lists them (#12879). Behind is the tool's own brush's, as its mode;
/// picking another mode leaves it. The wheel steps it as it steps the other Mode dropdowns.
/// Without `behind_enabled` (the Background is the active layer) Behind is greyed, as in the
/// Shift+right-click menu, and the wheel and the keys step past it.
pub(super) fn pattern_stamp_mode_dropdown(ui: &mut egui::Ui, id: &str, b: &mut photocraft_engine::BrushSettings, behind_enabled: bool, threshold: bool) {
    let mut opts = crate::brush_mode_menu::pattern_stamp_choices();
    if threshold && let Some(normal) = opts.iter_mut().find(|o| o.0 == (Some(BlendMode::Normal), false)) {
        normal.1 = tl!("Threshold");
    }
    let before = if b.behind { (None, true) } else { (Some(b.mode), false) };
    let mut mode = before;
    let groups = |m: &(Option<BlendMode>, bool)| m.0.is_some_and(starts_blend_group);
    if !widgets::dropdown_wheel_grouped_enabled(ui, id, tl!("Mode"), &mut mode, &opts, 96.0, groups, |m| behind_enabled || !m.1).0.is_empty() {
        let mode = crate::brush_mode_menu::pattern_stamp_pick(before, mode, behind_enabled);
        b.behind = mode.1;
        b.mode = mode.0.unwrap_or(BlendMode::Normal);
    }
}

/// The Eraser's Smoothing %: whole numbers 0..100 that say when a typed value was past them, for
/// the reference's alert (#11938). Returns the field and whether to show the alert.
pub(super) fn smoothing_field_alerting(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, width: f32) -> (egui::Response, bool) {
    let mut sm = b.smoothing.amount * 100.0;
    let (r, alert) = widgets::value_field_whole_alerting(ui, tl!("Smoothing"), &mut sm, 0.0..=100.0, "%", width);
    if r.changed() {
        b.smoothing.enabled = true;
        b.smoothing.amount = (sm / 100.0).clamp(0.0, 1.0);
    }
    (r, alert)
}

/// Options-bar Smoothing % (the brush's stroke smoothing; the live stroke and the commit use it).
pub(super) fn smoothing_field(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, width: f32) -> egui::Response {
    let mut sm = (b.smoothing.amount * 100.0).round();
    let r = widgets::value_field_named(ui, tl!("Smoothing"), &mut sm, 0.0..=100.0, "%", width);
    if r.changed() {
        b.smoothing.enabled = true;
        b.smoothing.amount = (sm / 100.0).clamp(0.0, 1.0);
    }
    r
}

/// Options-bar gear beside Smoothing: Photoshop's smoothing options popup, edited on the brush
/// like the Smoothing % (so they stay per tool and reach the live stroke and the commit).
/// `brush` adds the Brush's "Smoothing Options" title (#11813); the Eraser keeps its popup as it was.
pub(super) fn smoothing_options(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, brush: bool) {
    let resp = icons::button(ui, "settings", 24.0, false, tl!("Set additional smoothing options"));
    let resp = crate::brush_picker::named(resp, "Set additional smoothing options");
    egui::Popup::from_toggle_button_response(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let s = &mut b.smoothing;
        if brush {
            ui.label(egui::RichText::new(tl!("Smoothing Options")).strong());
        }
        // At 0 % there is no smoothing to tune: the four are greyed and keep their values, as in
        // the reference application (#11939). Rounded like the Smoothing field, so a value that
        // shows as 0 % greys them too.
        let on = (s.amount * 100.0).round() > 0.0;
        ui.add_enabled_ui(on, |ui| {
            widgets::checkbox(ui, &mut s.pulled_string, tl!("Pulled String Mode"));
            widgets::checkbox(ui, &mut s.catch_up, tl!("Stroke Catch-up"));
            widgets::checkbox(ui, &mut s.catch_up_on_end, tl!("Catch-up on Stroke End"));
            widgets::checkbox(ui, &mut s.adjust_for_zoom, tl!("Adjust for Zoom"));
        });
    });
}

/// A Useful mixer brush combinations entry: its label and the Wet, Load and Mix it sets.
type MixerCombination = (&'static str, Option<(f32, f32, f32)>);

/// The Mixer Brush's Useful mixer brush combinations, in the reference's order: each entry's label
/// and the Wet, Load and Mix (0..1) it sets. Custom, the first, sets nothing.
pub(crate) const MIXER_COMBINATIONS: [MixerCombination; 13] = [
    ("Custom", None),
    ("Dry", Some((0.0, 0.5, 0.0))),
    ("Dry, Light Load", Some((0.0, 0.05, 0.0))),
    ("Dry, Heavy Load", Some((0.0, 1.0, 0.0))),
    ("Moist", Some((0.1, 0.05, 0.5))),
    ("Moist, Light Mix", Some((0.1, 0.05, 0.0))),
    ("Moist, Heavy Mix", Some((0.1, 0.05, 1.0))),
    ("Wet", Some((0.5, 0.5, 0.5))),
    ("Wet, Light Mix", Some((0.5, 0.5, 0.0))),
    ("Wet, Heavy Mix", Some((0.5, 0.5, 1.0))),
    ("Very Wet", Some((1.0, 0.5, 0.5))),
    ("Very Wet, Light Mix", Some((1.0, 0.5, 0.0))),
    ("Very Wet, Heavy Mix", Some((1.0, 0.5, 1.0))),
];

/// The entries of [`MIXER_COMBINATIONS`] a separator line goes above: Dry, Moist, Wet, Very Wet.
const MIXER_COMBINATION_GROUPS: [usize; 4] = [1, 4, 7, 10];

/// Mixer Brush: the Useful mixer brush combinations dropdown (#11946). Picking an entry sets Wet,
/// Load and Mix together (Flow stays) and keeps showing it, also after a value is typed; Custom
/// sets nothing. `picked` is the entry shown.
pub(super) fn mixer_combination_dropdown(
    ui: &mut egui::Ui,
    picked: &mut usize,
    m: &mut photocraft_engine::paint::mixer::MixerSettings,
) -> Option<egui::Response> {
    const ID: &str = "mixer-combination";
    let opts: Vec<(usize, &str)> = MIXER_COMBINATIONS.iter().enumerate().map(|(i, (label, _))| (i, *label)).collect();
    let mut shown = (*picked).min(MIXER_COMBINATIONS.len() - 1);
    let chosen =
        widgets::dropdown_wheel_grouped(ui, ID, tl!("Useful mixer brush combinations"), &mut shown, &opts, 132.0, |i| MIXER_COMBINATION_GROUPS.contains(i)).0;
    if let Some(&i) = chosen.last() {
        *picked = i;
        if let Some((wet, load, mix)) = MIXER_COMBINATIONS[i].1 {
            (m.wet, m.load, m.mix) = (wet, load, mix);
        }
    }
    ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new(ID)))
}

#[cfg(test)]
#[path = "../mixer_blend_combination_tests.rs"]
mod mixer_blend_combination_tests;

#[cfg(test)]
#[path = "../mixer_load_solid_colors_tests.rs"]
mod mixer_load_solid_colors_tests;

/// Mixer Brush: the Current Brush Load swatch shows the paint the next stroke starts with (an
/// empty swatch when the brush is clean). Its menu holds Load Brush, Clean Brush, a separator and
/// the checkable Load Solid Colors Only.
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
    let (mut run, mut solid_colors) = (None, None);
    egui::Popup::menu(&resp).show(|ui| {
        if ui.button(tl!("Load Brush")).clicked() {
            run = Some("brush.mixer.load");
        }
        if ui.button(tl!("Clean Brush")).clicked() {
            run = Some("brush.mixer.clean");
        }
        ui.separator();
        // Load Solid Colors Only: ⌥-click loads one colour, not the tip image (#11952).
        let mut solid = app.session.tools.brush.mixer.solid_colors_only;
        if ui.checkbox(&mut solid, tl!("Load Solid Colors Only")).changed() {
            solid_colors = Some(solid);
        }
    });
    if let Some(id) = run
        && let Err(e) = app.run(id, json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some(on) = solid_colors
        && let Err(e) = app.run("tools.setBrush", json!({"brush": {"mixer": {"solidColorsOnly": on}}}))
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
    // The brush's own tip, as the Brush Preset picker's thumbnails draw it: the engine's falloff
    // for its hardness, its angle and roundness, a sampled tip's shape.
    let tex = crate::brush_preview::tip_texture(ui.ctx(), "options-bar-chip", &b.tip, (b.hardness, b.angle, b.roundness), 32, tip);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(tex.id(), Rect::from_center_size(c, vec2(14.0, 14.0)), uv, Color32::WHITE);
    ui.painter().text(pos2(c.x, r.bottom() - 5.0), Align2::CENTER_CENTER, format!("{}", b.size.round() as i64), egui::FontId::proportional(9.5), t.text_dim);
    icons::paint(ui, Rect::from_center_size(pos2(r.right() - 9.0, c.y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Brush Preset picker")));
    // The Brush's and the Background Eraser's own text (#11813, #11909); the other tools keep theirs.
    let tip = if matches!(state.tool, crate::state::Tool::Brush | crate::state::Tool::BackgroundEraser) {
        tl!("Click to open the Brush Preset picker")
    } else {
        tl!("Brush Preset picker")
    };
    if resp.on_hover_text(tip).clicked() {
        if state.brush_picker.is_some() {
            crate::brush_picker::close(state);
        } else {
            crate::brush_picker::open(state, ui.ctx(), [r.left(), r.bottom() + 2.0]);
        }
    }
}

#[cfg(test)]
#[path = "../smoothing_options_greyed_tests.rs"]
mod smoothing_options_greyed_tests;
