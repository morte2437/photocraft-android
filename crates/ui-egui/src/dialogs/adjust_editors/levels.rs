use super::curves::{ToneChannel, bar_color, channel_picker, gradient_bar, hist_color, tone_channels};
use super::*;

// ---------------------------------------------------------------------------------------------
// Levels

type Lv = [f32; 5]; // in_black, gamma, in_white, out_black, out_white (0–255, gamma as is)

fn read_levels(v: &Value, key: &str) -> Lv {
    let o = if key.is_empty() { Some(v) } else { v.get(key) };
    let g = |k: &str, d: f32| o.and_then(|o| o.get(k)).and_then(Value::as_f64).map_or(d, |x| x as f32);
    [g("inBlack", 0.0), g("gamma", 1.0), g("inWhite", 255.0), g("outBlack", 0.0), g("outWhite", 255.0)]
}

fn write_levels(v: &mut Value, key: &str, l: &Lv) {
    let obj = json!({"inBlack": l[0].round(), "gamma": round(l[1], 0.01), "inWhite": l[2].round(), "outBlack": l[3].round(), "outWhite": l[4].round()});
    if key.is_empty() {
        if let (Some(dst), Value::Object(src)) = (v.as_object_mut(), obj) {
            dst.extend(src);
        }
    } else {
        v[key] = obj;
    }
}

/// A triangular slider handle below a bar.
fn handle(p: &egui::Painter, x: f32, y: f32, fill: Color32, stroke: Color32) {
    p.add(egui::Shape::convex_polygon(vec![pos2(x, y), pos2(x + 5.5, y + 9.0), pos2(x - 5.5, y + 9.0)], fill, Stroke::new(1.0, stroke)));
}

/// Input level (0–255) where the gamma slider sits: the input that maps to 50% grey.
pub fn gamma_pos(black: f32, white: f32, gamma: f32) -> f32 {
    black + (white - black) * 0.5f32.powf(gamma)
}

pub fn gamma_from_pos(black: f32, white: f32, x: f32) -> f32 {
    let t = ((x - black) / (white - black).max(1.0)).clamp(0.01, 0.99);
    (t.ln() / 0.5f32.ln()).clamp(0.01, 9.99)
}

/// Black and white input levels clipping 0.1 % at each end of `h` (Photoshop's Auto).
pub fn auto_levels(h: &[u32; 256]) -> (f32, f32) {
    let total: u64 = h.iter().map(|x| u64::from(*x)).sum();
    let clip = total / 1000;
    let (mut acc, mut lo) = (0u64, 0usize);
    while lo < 253 && acc + u64::from(h[lo]) <= clip {
        acc += u64::from(h[lo]);
        lo += 1;
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    while hi > lo + 2 && acc + u64::from(h[hi]) <= clip {
        acc += u64::from(h[hi]);
        hi -= 1;
    }
    (lo as f32, hi as f32)
}

/// Which handle of a slider row a press at `x` grabs (nearest), among `xs`.
pub(super) fn nearest(xs: &[f32], x: f32) -> Option<u8> {
    xs.iter().enumerate().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map(|(i, _)| i as u8)
}

pub(super) fn levels(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let chans = tone_channels(space_of(v), cx.gray);
    let mut ch = 0;
    let mut auto = false;
    ui.horizontal(|ui| {
        ch = channel_picker(ui, cx.mem.with("levels-ch"), chans, tl!("Channel:"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            auto = widgets::secondary_button(ui, tl!("Auto"), 0.0).clicked();
        });
    });
    let chan = chans.get(ch).copied().unwrap_or(ToneChannel { key: "", label: "" });
    let mut l = read_levels(v, chan.key);
    let mut e = Edit::default();
    let mut changed = false;
    if auto && let Some(h) = &cx.hist {
        let (lo, hi) = auto_levels(&h[ch]);
        l[0] = lo;
        l[2] = hi;
        l[1] = 1.0;
        changed = true;
        e.commit = true;
    }
    let w = ui.available_width().min(300.0);
    // Histogram.
    let (hr, _) = ui.allocate_exact_size(vec2(w, 110.0), Sense::hover());
    let hr = hr.shrink2(vec2(6.0, 0.0));
    let p = ui.painter_at(hr.expand(1.0));
    p.rect_filled(hr, 0.0, t.field);
    if let Some(h) = &cx.hist {
        tone::draw_histogram(&p, hr, &h[ch], hist_color(&chan, &t));
    }
    p.rect_stroke(hr, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    // Input sliders: black, gamma, white.
    let (sr, sresp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::click_and_drag());
    let track = sr.shrink2(vec2(6.0, 0.0));
    let xpos = |val: f32| track.left() + val / 255.0 * track.width();
    let xval = |x: f32| ((x - track.left()) / track.width() * 255.0).clamp(0.0, 255.0);
    let drag_id = cx.mem.with("levels-drag");
    let mut which: Option<u8> = ui.data(|d| d.get_temp(drag_id)).flatten();
    let ctx = ui.ctx().clone();
    let pressed = |r: &egui::Response| r.is_pointer_button_down_on() && ctx.input(|i| i.pointer.primary_pressed());
    if pressed(&sresp)
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        which = nearest(&[xpos(l[0]), xpos(gamma_pos(l[0], l[2], l[1])), xpos(l[2])], pos.x);
    }
    if sresp.is_pointer_button_down_on()
        && let (Some(k), Some(pos)) = (which, ui.input(|i| i.pointer.interact_pos()))
    {
        let x = xval(pos.x);
        let before = l;
        match k {
            0 => l[0] = x.min(l[2] - 2.0).round(),
            2 => l[2] = x.max(l[0] + 2.0).round(),
            _ => l[1] = round(gamma_from_pos(l[0], l[2], x), 0.01),
        }
        changed |= l != before;
    }
    if which.is_some() && !sresp.is_pointer_button_down_on() {
        which = None;
        e.commit = true;
    }
    ui.data_mut(|d| d.insert_temp(drag_id, which));
    let sp = ui.painter_at(sr.expand(6.0));
    handle(&sp, xpos(l[0]), sr.top() + 2.0, Color32::BLACK, t.text_dim);
    handle(&sp, xpos(gamma_pos(l[0], l[2], l[1])), sr.top() + 2.0, Color32::from_gray(128), t.text_dim);
    handle(&sp, xpos(l[2]), sr.top() + 2.0, Color32::WHITE, t.text_dim);
    ui.horizontal(|ui| {
        let fw = ((w - 16.0) / 3.0).min(70.0);
        let (mut nb, mut ng, mut nw) = (l[0], l[1], l[2]);
        let gap = ((w - 3.0 * fw) / 2.0 - 12.0).max(0.0);
        let r0 = widgets::value_field(ui, &mut nb, 0.0..=253.0, "", fw).accessible_name(tl!("Black"));
        ui.add_space(gap);
        let r1 = widgets::value_field(ui, &mut ng, 0.01..=9.99, "", fw).accessible_name(tl!("Gamma"));
        ui.add_space(gap);
        let r2 = widgets::value_field(ui, &mut nw, 2.0..=255.0, "", fw).accessible_name(tl!("White"));
        if r0.changed() {
            l[0] = nb.round().min(l[2] - 2.0);
        }
        if r1.changed() {
            l[1] = round(ng.clamp(0.01, 9.99), 0.01);
        }
        if r2.changed() {
            l[2] = nw.round().max(l[0] + 2.0);
        }
        for r in [&r0, &r1, &r2] {
            changed |= r.changed();
            e.add(Edit::of(r));
        }
    });
    // Output levels.
    ui.add_space(6.0);
    label(ui, tl!("Output Levels:"));
    let (ob_r, _) = ui.allocate_exact_size(vec2(w, 10.0), Sense::hover());
    gradient_bar(ui.painter(), ob_r.shrink2(vec2(6.0, 0.0)), Color32::BLACK, bar_color(&chan, &t), false);
    let (or, oresp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::click_and_drag());
    let otrack = or.shrink2(vec2(6.0, 0.0));
    let oxpos = |val: f32| otrack.left() + val / 255.0 * otrack.width();
    let oval = |x: f32| ((x - otrack.left()) / otrack.width() * 255.0).clamp(0.0, 255.0);
    let odrag_id = cx.mem.with("levels-odrag");
    let mut owhich: Option<u8> = ui.data(|d| d.get_temp(odrag_id)).flatten();
    if pressed(&oresp)
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        owhich = nearest(&[oxpos(l[3]), oxpos(l[4])], pos.x);
    }
    if oresp.is_pointer_button_down_on()
        && let (Some(k), Some(pos)) = (owhich, ui.input(|i| i.pointer.interact_pos()))
    {
        let slot = if k == 0 { 3 } else { 4 };
        let nv = oval(pos.x).round();
        changed |= l[slot] != nv;
        l[slot] = nv;
    }
    if owhich.is_some() && !oresp.is_pointer_button_down_on() {
        owhich = None;
        e.commit = true;
    }
    ui.data_mut(|d| d.insert_temp(odrag_id, owhich));
    let op = ui.painter_at(or.expand(6.0));
    handle(&op, oxpos(l[3]), or.top() + 2.0, Color32::BLACK, t.text_dim);
    handle(&op, oxpos(l[4]), or.top() + 2.0, Color32::WHITE, t.text_dim);
    ui.horizontal(|ui| {
        let fw = ((w - 16.0) / 3.0).min(70.0);
        let (mut a, mut c) = (l[3], l[4]);
        let ra = widgets::value_field(ui, &mut a, 0.0..=255.0, "", fw).accessible_name(tl!("Black"));
        ui.add_space((w - 2.0 * fw - 16.0).max(0.0));
        let rc = widgets::value_field(ui, &mut c, 0.0..=255.0, "", fw).accessible_name(tl!("White"));
        if ra.changed() {
            l[3] = a.round();
        }
        if rc.changed() {
            l[4] = c.round();
        }
        for r in [&ra, &rc] {
            changed |= r.changed();
            e.add(Edit::of(r));
        }
    });
    if changed {
        write_levels(v, chan.key, &l);
        e.changed = true;
    }
    e
}
