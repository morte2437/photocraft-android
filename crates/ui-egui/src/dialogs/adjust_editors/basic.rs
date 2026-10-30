use super::hue_saturation::RANGE_LABELS;
use super::{
    Color32, Edit, EditorCx, HUE_RANGES, PHOTO_FILTERS, RichText, Sense, Stroke, StrokeKind, Tokens, Value, adjust_params, color_button, flag, json, label,
    num, nums, pos2, rgb_of, round, tone, vec2, widgets,
};

/// Labelled slider rows over top-level keys: (key, label, min, max, default, unit).
/// A row of [`sliders`]: params key, label, range, default, unit, and its decimal places (None:
/// two for a range of 10 or less, else whole numbers).
type SliderRow<'a> = (&'a str, &'a str, f32, f32, f32, &'a str, Option<u32>);

/// The value a [`sliders`] row stores for `x`: rounded to `decimals` places, or by its range.
pub(crate) fn row_value(x: f32, min: f32, max: f32, decimals: Option<u32>) -> f32 {
    match decimals {
        Some(d) => {
            let scale = 10f32.powi(d.min(6) as i32);
            (x * scale).round() / scale
        }
        None if max - min <= 10.0 => round(x, 0.01),
        None => x.round(),
    }
}

pub(super) fn sliders(ui: &mut egui::Ui, v: &mut Value, rows: &[SliderRow]) -> Edit {
    let mut e = Edit::default();
    for &(key, text, min, max, default, unit, decimals) in rows {
        let mut x = num(v, key, default).clamp(min, max);
        // A double-click on the slider puts it back to its default (Photoshop).
        let gestures = widgets::RowGestures { reset: Some(default), wheel_step: None, ..Default::default() };
        let r = widgets::slider_row_prec(ui, text, &mut x, min..=max, unit, None, gestures, decimals);
        if r.changed() {
            v[key] = json!(row_value(x, min, max, decimals));
        }
        e.add(Edit::of(&r));
    }
    e
}

fn gradient_slider(ui: &mut egui::Ui, text: &str, x: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, stops: [Color32; 2], dynamic: bool) -> Edit {
    let r = widgets::slider_row(ui, text, x, range, unit, dynamic.then_some(&stops));
    Edit::of(&r)
}

/// Preferences ▸ Interface ▸ Dynamic Color Sliders: the colour track for a slider, or the plain
/// track when it is off.
pub(super) fn track_stops<'a>(cx: &EditorCx, stops: &'a [Color32]) -> Option<&'a [Color32]> {
    cx.dynamic_color_sliders.then_some(stops)
}

// ---------------------------------------------------------------------------------------------
// Brightness/Contrast

pub(super) fn brightness_contrast(ui: &mut egui::Ui, v: &mut Value) -> Edit {
    let legacy = flag(v, "legacy", false);
    let lo = if legacy { -100.0 } else { -50.0 };
    let mut e = sliders(ui, v, &[("brightness", tl!("Brightness"), -150.0, 150.0, 0.0, "", None), ("contrast", tl!("Contrast"), lo, 100.0, 0.0, "", None)]);
    let mut l = legacy;
    if widgets::checkbox(ui, &mut l, tl!("Use Legacy")).changed() {
        v["legacy"] = json!(l);
        if !l {
            v["contrast"] = json!(num(v, "contrast", 0.0).max(-50.0));
        }
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Threshold

pub(super) fn threshold(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width().min(300.0);
    let (hr, _) = ui.allocate_exact_size(vec2(w, 90.0), Sense::hover());
    let hr = hr.shrink2(vec2(6.0, 0.0));
    let p = ui.painter_at(hr.expand(1.0));
    p.rect_filled(hr, 0.0, t.field);
    if let Some(h) = &cx.hist {
        tone::draw_histogram(&p, hr, &h[0], Color32::from_gray(if t.pro { 150 } else { 110 }));
    }
    let level = num(v, "level", 128.0);
    let x = hr.left() + level / 255.0 * hr.width();
    p.line_segment([pos2(x, hr.top()), pos2(x, hr.bottom())], Stroke::new(1.0, t.accent));
    p.rect_stroke(hr, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    ui.add_space(4.0);
    sliders(ui, v, &[("level", tl!("Threshold Level"), 1.0, 255.0, 128.0, "", None)])
}

// ---------------------------------------------------------------------------------------------
// Color Balance

/// Where the Color Balance editor rooted at `mem` keeps its Tone choice.
fn color_balance_tone_id(mem: egui::Id) -> egui::Id {
    mem.with("cb-tone")
}

/// Photoshop's Color Balance reset to defaults (Alt+Cancel in the dialog; the Properties panel's
/// Reset once the layer is as the panel found it): every tone back to 0 and the Tone choice back
/// to Midtones; Preserve Luminosity stays as it is.
pub fn reset_color_balance(ctx: &egui::Context, mem: egui::Id, v: &mut Value) {
    for key in ["shadows", "midtones", "highlights"] {
        v[key] = json!([0.0, 0.0, 0.0]);
    }
    ctx.data_mut(|d| d.insert_temp(color_balance_tone_id(mem), 1usize));
}

/// Kind-specific editor memory after Alt+Reset (or Properties Reset to defaults). Color Balance
/// shows Midtones; other kinds have no dialog-local temp to clear yet.
pub fn reset_adjustment_editor(ctx: &egui::Context, kind: &str, mem: egui::Id, v: &mut Value) {
    if kind == "colorBalance" {
        reset_color_balance(ctx, mem, v);
    }
}

pub(super) fn color_balance(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let tone_id = color_balance_tone_id(cx.mem);
    let mut tone_ix: usize = ui.data(|d| d.get_temp(tone_id)).unwrap_or(1);
    ui.horizontal(|ui| {
        label(ui, tl!("Tone:"));
        for (i, l) in [tl!("Shadows"), tl!("Midtones"), tl!("Highlights")].iter().enumerate() {
            if ui.radio(tone_ix == i, *l).clicked() {
                tone_ix = i;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(tone_id, tone_ix));
    let key = ["shadows", "midtones", "highlights"][tone_ix.min(2)];
    let mut vals = nums(v, key, [0.0; 3]);
    let mut e = Edit::default();
    let rows = [
        (tl!("Cyan  ·  Red"), Color32::from_rgb(0, 190, 210), Color32::from_rgb(225, 40, 40)),
        (tl!("Magenta  ·  Green"), Color32::from_rgb(210, 40, 190), Color32::from_rgb(40, 190, 60)),
        (tl!("Yellow  ·  Blue"), Color32::from_rgb(230, 210, 30), Color32::from_rgb(40, 80, 230)),
    ];
    // Photoshop: a double-click on a slider zeroes it (this tone only); the dialog's sliders step
    // with the wheel; Up/Down step the fields; Shift makes those steps 10.
    let gestures = widgets::RowGestures { reset: Some(0.0), wheel_step: cx.dialog.then_some(1.0), ..Default::default() };
    let mut changed = false;
    for (i, (text, a, b)) in rows.iter().enumerate() {
        let r = Edit::of(&widgets::slider_row_with(ui, text, &mut vals[i], -100.0..=100.0, "", Some(&[*a, *b]), gestures));
        changed |= r.changed;
        e.add(r);
    }
    if changed {
        v[key] = json!(vals.map(f32::round));
    }
    let mut pl = flag(v, "preserveLuminosity", true);
    if widgets::checkbox(ui, &mut pl, tl!("Preserve Luminosity")).changed() {
        v["preserveLuminosity"] = json!(pl);
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Black & White

pub(super) fn black_white(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let mut e = Edit::default();
    let colors = [[230, 50, 50], [235, 215, 40], [50, 200, 70], [40, 200, 220], [50, 90, 235], [220, 50, 210]];
    for (i, key) in HUE_RANGES.iter().enumerate() {
        let mut x = num(v, key, adjust_params::BW_DEFAULTS[i]).clamp(-200.0, 300.0);
        let c = colors[i];
        let r =
            gradient_slider(ui, RANGE_LABELS[i], &mut x, -200.0..=300.0, "%", [Color32::BLACK, Color32::from_rgb(c[0], c[1], c[2])], cx.dynamic_color_sliders);
        if r.changed {
            v[*key] = json!(x.round());
        }
        e.add(r);
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let mut on = flag(v, "tint", false);
        if widgets::checkbox(ui, &mut on, tl!("Tint")).changed() {
            v["tint"] = json!(on);
            e.add(Edit::discrete(true));
        }
        let mut c = rgb_of(v, "tintColor", [0.882, 0.827, 0.702]);
        let r = ui.add_enabled_ui(on, |ui| color_button(ui, &mut c)).inner;
        if r.changed {
            v["tintColor"] = json!(adjust_params::hex(c));
        }
        e.add(r);
        if widgets::secondary_button(ui, tl!("Default"), 0.0).clicked() {
            for (i, key) in HUE_RANGES.iter().enumerate() {
                v[*key] = json!(adjust_params::BW_DEFAULTS[i]);
            }
            e.add(Edit::discrete(true));
        }
    });
    e
}

// ---------------------------------------------------------------------------------------------
// Photo Filter

pub(super) fn photo_filter(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let mut e = Edit::default();
    let mut c = rgb_of(v, "color", [236.0 / 255.0, 138.0 / 255.0, 0.0]);
    let hex = adjust_params::hex(c);
    let preset = PHOTO_FILTERS.iter().find(|f| adjust_params::hex(f.2.map(|x| f32::from(x) / 255.0)) == hex).map(|f| f.0);
    let mode_id = cx.mem.with("pf-mode");
    let mut use_color: bool = ui.data(|d| d.get_temp(mode_id)).unwrap_or(preset.is_none());
    ui.horizontal(|ui| {
        if ui.radio(!use_color, tl!("Filter:")).clicked() {
            use_color = false;
        }
        let mut sel = preset.unwrap_or("custom").to_string();
        let mut opts: Vec<(String, &str)> = PHOTO_FILTERS.iter().map(|f| (f.0.to_string(), f.1)).collect();
        if preset.is_none() {
            opts.insert(0, ("custom".into(), "Custom"));
        }
        let changed = ui.add_enabled_ui(!use_color, |ui| widgets::dropdown(ui, &format!("{mode_id:?}-filter"), tl!("Filter:"), &mut sel, &opts, 170.0)).inner;
        if changed && let Some(f) = PHOTO_FILTERS.iter().find(|f| f.0 == sel) {
            c = f.2.map(|x| f32::from(x) / 255.0);
            v["color"] = json!(adjust_params::hex(c));
            e.add(Edit::discrete(true));
        }
    });
    ui.horizontal(|ui| {
        if ui.radio(use_color, tl!("Color:")).clicked() {
            use_color = true;
        }
        let r = ui.add_enabled_ui(use_color, |ui| color_button(ui, &mut c)).inner;
        if r.changed {
            v["color"] = json!(adjust_params::hex(c));
        }
        e.add(r);
    });
    ui.data_mut(|d| d.insert_temp(mode_id, use_color));
    e.add(sliders(ui, v, &[("density", tl!("Density"), 0.0, 100.0, 25.0, "%", None)]));
    let mut pl = flag(v, "preserveLuminosity", true);
    if widgets::checkbox(ui, &mut pl, tl!("Preserve Luminosity")).changed() {
        v["preserveLuminosity"] = json!(pl);
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Channel Mixer

pub(super) fn channel_mixer(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let mono = flag(v, "monochrome", false);
    let out_id = cx.mem.with("cm-out");
    let mut out: usize = ui.data(|d| d.get_temp(out_id)).unwrap_or(0);
    let outs: &[(usize, &str)] = if mono { &[(0, tl!("Gray"))] } else { &[(0, tl!("Red")), (1, tl!("Green")), (2, tl!("Blue"))] };
    if out >= outs.len() {
        out = 0;
    }
    ui.horizontal(|ui| {
        label(ui, tl!("Output Channel:"));
        widgets::dropdown(ui, &format!("{out_id:?}"), tl!("Output Channel:"), &mut out, outs, 110.0);
    });
    ui.data_mut(|d| d.insert_temp(out_id, out));
    let key = if mono { "gray" } else { ["red", "green", "blue"][out] };
    let identity: [f32; 4] = if mono { [40.0, 40.0, 20.0, 0.0] } else { std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 }) };
    let mut row = nums(v, key, if mono { nums(v, "red", identity) } else { identity });
    let mut e = Edit::default();
    let mut changed = false;
    for (i, (text, c)) in [(tl!("Red"), [225, 50, 50]), (tl!("Green"), [50, 190, 70]), (tl!("Blue"), [60, 100, 235])].iter().enumerate() {
        let r = gradient_slider(ui, text, &mut row[i], -200.0..=200.0, "%", [Color32::BLACK, Color32::from_rgb(c[0], c[1], c[2])], cx.dynamic_color_sliders);
        changed |= r.changed;
        e.add(r);
    }
    let total = row[0] + row[1] + row[2];
    ui.horizontal(|ui| {
        label(ui, tl!("Total:"));
        let warn = total > 100.0;
        ui.label(RichText::new(format!("{total:+.0}%")).font(crate::theme::mono(12.0)).color(if warn { t.warning } else { t.text }));
    });
    let r = widgets::slider_row(ui, tl!("Constant"), &mut row[3], -200.0..=200.0, "%", None);
    changed |= r.changed();
    e.add(Edit::of(&r));
    if changed {
        v[key] = json!(row.map(f32::round));
    }
    let mut m = mono;
    if widgets::checkbox(ui, &mut m, tl!("Monochrome")).changed() {
        v["monochrome"] = json!(m);
        if m {
            v["gray"] = json!([40.0, 40.0, 20.0, 0.0]);
        } else if let Some(o) = v.as_object_mut() {
            o.remove("gray");
        }
        e.add(Edit::discrete(true));
    }
    e
}
