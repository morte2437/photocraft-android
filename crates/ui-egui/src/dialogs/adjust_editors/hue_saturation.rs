use super::basic::{sliders, track_stops};
use super::curves::gradient_bar;
use super::levels::nearest;
use super::{
    Color32, Edit, EditorCx, HUE_RANGES, HueRange, Rect, RichText, Sense, Stroke, StrokeKind, Tokens, Value, color32, flag, hue_color, json, num, nums, pos2,
    vec2, widgets,
};

// ---------------------------------------------------------------------------------------------
// Hue/Saturation

pub(super) const RANGE_LABELS: [&str; 6] = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];

/// The Saturation slider's track: grey to the hue at full saturation (UI-217-16).
pub(super) fn saturation_track(hue_deg: f32) -> [Color32; 2] {
    [Color32::from_gray(128), hue_color(hue_deg)]
}

/// The Lightness slider's track: black, mid grey, white, whatever the hue, as in Photoshop
/// (UI-217-16, #6539).
pub(super) fn lightness_track() -> [Color32; 3] {
    [Color32::BLACK, Color32::from_gray(128), Color32::WHITE]
}

pub(super) fn move_hue_range_handle(bounds: [f32; 4], k: usize, deg: f32) -> [f32; 4] {
    let mut b = HueRange::canonical_bounds(bounds);
    if k >= b.len() {
        return b;
    }
    if k == 0 {
        let delta = (deg - b[0] + 180.0).rem_euclid(360.0) - 180.0;
        b[0] = (b[0] + delta).min(b[1]).max(b[1] - 180.0);
    } else {
        let rel = (deg - b[0]).rem_euclid(360.0);
        let hi = if k == 3 { b[0] + 359.0 } else { b[k + 1] };
        b[k] = (b[0] + rel).clamp(b[k - 1], hi);
    }
    b
}

/// The spectrum bar after the adjustment (what each hue becomes), sampled at `n` hues.
fn adjusted_spectrum(v: &Value, n: usize) -> Vec<Color32> {
    let adj = photocraft_engine::commands::adjustment_from_params("hueSaturation", v);
    let rect = photocraft_geom::Rect::new(0, 0, n as i32, 1);
    let px = (0..n)
        .map(|i| {
            let c = photocraft_color::convert::hsb_to_rgb([i as f32 / (n - 1).max(1) as f32 * 360.0, 1.0, 1.0]);
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    let mut buf = photocraft_compose::Buffer { rect, px };
    photocraft_compose::adjust::apply(&adj, &mut buf);
    buf.px.iter().map(|p| color32([p[0], p[1], p[2]])).collect()
}

pub(super) fn spectrum_bar(p: &egui::Painter, r: Rect, colors: &[Color32]) {
    let n = colors.len().saturating_sub(1).max(1);
    for (i, w) in colors.windows(2).enumerate() {
        let x0 = r.left() + r.width() * i as f32 / n as f32;
        let x1 = r.left() + r.width() * (i + 1) as f32 / n as f32;
        gradient_bar(p, Rect::from_min_max(pos2(x0, r.top()), pos2(x1, r.bottom())), w[0], w[1], false);
    }
}

pub(super) fn hue_saturation(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let range_id = cx.mem.with("hs-range");
    let mut range: usize = ui.data(|d| d.get_temp(range_id)).unwrap_or(0);
    let colorize = flag(v, "colorize", false);
    if colorize {
        range = 0;
    }
    ui.horizontal(|ui| {
        let opts: Vec<(usize, &str)> = std::iter::once((0, tl!("Master"))).chain(RANGE_LABELS.iter().enumerate().map(|(i, l)| (i + 1, *l))).collect();
        ui.add_enabled_ui(!colorize, |ui| widgets::dropdown(ui, &format!("{range_id:?}"), tl!("Range"), &mut range, &opts, 120.0));
    });
    ui.data_mut(|d| d.insert_temp(range_id, range));
    let mut e = Edit::default();
    let hue_grad = widgets::hue_stops();
    if range == 0 {
        let (hlo, hhi, slo) = if colorize { (0.0, 360.0, 0.0) } else { (-180.0, 180.0, -100.0) };
        let mut h = num(v, "hue", 0.0).clamp(hlo, hhi);
        let r = widgets::slider_row(ui, tl!("Hue"), &mut h, hlo..=hhi, "°", track_stops(cx, &hue_grad));
        if r.changed() {
            v["hue"] = json!(h.round());
        }
        e.add(Edit::of(&r));
        // The Saturation track follows the current hue; the Lightness track is a fixed
        // black-grey-white ramp, as in Photoshop (UI-217-16).
        let sat_track = saturation_track(h);
        let mut s = num(v, "saturation", 0.0).clamp(slo, 100.0);
        let r = widgets::slider_row(ui, tl!("Saturation"), &mut s, slo..=100.0, "%", track_stops(cx, &sat_track));
        if r.changed() {
            v["saturation"] = json!(s.round());
        }
        e.add(Edit::of(&r));
        let light_track = lightness_track();
        let mut l = num(v, "lightness", 0.0).clamp(-100.0, 100.0);
        let r = widgets::slider_row(ui, tl!("Lightness"), &mut l, -100.0..=100.0, "%", track_stops(cx, &light_track));
        if r.changed() {
            v["lightness"] = json!(l.round());
        }
        e.add(Edit::of(&r));
    } else {
        let key = HUE_RANGES[range - 1];
        if !v.get(key).is_some_and(Value::is_object) {
            v[key] = json!({});
        }
        let mut o = v[key].clone();
        let mut sub = Edit::default();
        let mut h = num(&o, "hue", 0.0);
        let r = widgets::slider_row(ui, tl!("Hue"), &mut h, -180.0..=180.0, "°", track_stops(cx, &hue_grad));
        if r.changed() {
            o["hue"] = json!(h.round());
        }
        sub.add(Edit::of(&r));
        sub.add(sliders(
            ui,
            &mut o,
            &[("saturation", tl!("Saturation"), -100.0, 100.0, 0.0, "%", None), ("lightness", tl!("Lightness"), -100.0, 100.0, 0.0, "%", None)],
        ));
        if sub.changed {
            v[key] = o;
        }
        e.add(sub);
    }
    let mut c = colorize;
    if widgets::checkbox(ui, &mut c, tl!("Colorize")).changed() {
        v["colorize"] = json!(c);
        let h = num(v, "hue", 0.0);
        v["hue"] = json!(if c {
            h.rem_euclid(360.0)
        } else if h > 180.0 {
            h - 360.0
        } else {
            h
        });
        if c {
            v["saturation"] = json!(num(v, "saturation", 25.0).max(0.0).max(25.0));
        }
        e.add(Edit::discrete(true));
    }
    // Spectrum bars: input hues above, adjusted below; the selected range's sliders between.
    ui.add_space(6.0);
    // As wide as the sliders above, whatever the panel's width (#9242).
    let w = ui.available_width();
    let (bars, bresp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click_and_drag());
    ui.data_mut(|d| d.insert_temp(cx.mem.with("hs-bars"), bars));
    let top = Rect::from_min_size(bars.min, vec2(w, 10.0));
    let bottom = Rect::from_min_size(pos2(bars.left(), bars.bottom() - 10.0), vec2(w, 10.0));
    let p = ui.painter_at(bars.expand(4.0));
    let input: Vec<Color32> = (0..=36).map(|i| hue_color(i as f32 * 10.0)).collect();
    spectrum_bar(&p, top, &input);
    spectrum_bar(&p, bottom, &adjusted_spectrum(v, 37));
    if range > 0 {
        let key = HUE_RANGES[range - 1];
        let neutral = photocraft_doc::adjust::HueRange::neutral(range - 1).bounds;
        let mut b = HueRange::canonical_bounds(nums(&v[key], "range", neutral));
        let xof = |deg: f32| bars.left() + deg.rem_euclid(360.0) / 360.0 * w;
        let deg_of = |x: f32| ((x - bars.left()) / w * 360.0).clamp(0.0, 360.0);
        let mid = bars.center().y;
        // Range band (fully affected) and fall-off.
        let band = |a: f32, z: f32, color: Color32| {
            let (xa, xz) = (xof(a), xof(z));
            if xa <= xz {
                p.rect_filled(Rect::from_min_max(pos2(xa, mid - 3.0), pos2(xz, mid + 3.0)), 0.0, color);
            } else {
                p.rect_filled(Rect::from_min_max(pos2(xa, mid - 3.0), pos2(bars.right(), mid + 3.0)), 0.0, color);
                p.rect_filled(Rect::from_min_max(pos2(bars.left(), mid - 3.0), pos2(xz, mid + 3.0)), 0.0, color);
            }
        };
        band(b[0], b[3], t.text_faint);
        band(b[1], b[2], t.text_dim);
        for (k, d) in b.iter().enumerate() {
            let x = xof(*d);
            let fill = if k == 1 || k == 2 { t.text } else { t.field };
            p.rect_filled(Rect::from_center_size(pos2(x, mid), vec2(5.0, 12.0)), 1.0, fill);
            p.rect_stroke(Rect::from_center_size(pos2(x, mid), vec2(5.0, 12.0)), 1.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
        }
        let drag_id = cx.mem.with("hs-range-drag");
        let mut which: Option<u8> = ui.data(|d| d.get_temp(drag_id)).flatten();
        if bresp.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.primary_pressed())
            && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        {
            which = nearest(&b.map(xof), pos.x).filter(|k| (xof(b[*k as usize]) - pos.x).abs() < 10.0);
        }
        if bresp.is_pointer_button_down_on()
            && let (Some(k), Some(pos)) = (which, ui.input(|i| i.pointer.interact_pos()))
        {
            let k = k as usize;
            let deg = deg_of(pos.x);
            let before = b;
            b = move_hue_range_handle(b, k, deg);
            if b != before {
                v[key]["range"] = json!(b.map(|x| x.round()));
                e.changed = true;
            }
        }
        if which.is_some() && !bresp.is_pointer_button_down_on() {
            which = None;
            e.commit = true;
        }
        ui.data_mut(|d| d.insert_temp(drag_id, which));
        let lab =
            format!("{:.0}° / {:.0}°  ·  {:.0}° \\ {:.0}°", b[0].rem_euclid(360.0), b[1].rem_euclid(360.0), b[2].rem_euclid(360.0), b[3].rem_euclid(360.0));
        ui.label(RichText::new(lab).font(crate::theme::mono(11.0)).color(t.text_faint));
    }
    e
}
