use super::{
    Color32, CurveUi, Edit, EditorCx, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Tokens, ToneSpace, Value, adjust_params, curve_edit, json, label, pos2,
    space_of, tone, vec2, widgets,
};

// ---------------------------------------------------------------------------------------------
// Channels of Levels / Curves

/// A channel of a tone editor: the params key (None = the top-level / composite keys for Levels;
/// Curves uses "points"), its label, and its colour name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneChannel {
    pub key: &'static str,
    pub label: &'static str,
}

/// The channels a tone editor lists for `space` (histogram index = position).
pub fn tone_channels(space: ToneSpace, gray: bool) -> &'static [ToneChannel] {
    const RGB: [ToneChannel; 4] = [
        ToneChannel { key: "", label: "RGB" },
        ToneChannel { key: "red", label: "Red" },
        ToneChannel { key: "green", label: "Green" },
        ToneChannel { key: "blue", label: "Blue" },
    ];
    const GRAY: [ToneChannel; 1] = [ToneChannel { key: "", label: "Gray" }];
    const CMYK: [ToneChannel; 5] = [
        ToneChannel { key: "", label: "CMYK" },
        ToneChannel { key: "cyan", label: "Cyan" },
        ToneChannel { key: "magenta", label: "Magenta" },
        ToneChannel { key: "yellow", label: "Yellow" },
        ToneChannel { key: "black", label: "Black" },
    ];
    const LAB: [ToneChannel; 3] =
        [ToneChannel { key: "lightness", label: "Lightness" }, ToneChannel { key: "a", label: "a" }, ToneChannel { key: "b", label: "b" }];
    match space {
        ToneSpace::Rgb if gray => &GRAY,
        ToneSpace::Rgb => &RGB,
        ToneSpace::Cmyk => &CMYK,
        ToneSpace::Lab => &LAB,
    }
}

pub(super) fn channel_picker(ui: &mut egui::Ui, id: egui::Id, chans: &[ToneChannel], text: &str) -> usize {
    let opts: Vec<(usize, &str)> = chans.iter().enumerate().map(|(i, c)| (i, c.label)).collect();
    channel_picker_opts(ui, id, &opts, text, true)
}

/// The popup, with `translate` for whether the built-in labels follow the interface language;
/// a user-typed channel name is shown exactly as entered (#5335).
fn channel_picker_opts(ui: &mut egui::Ui, id: egui::Id, opts: &[(usize, &str)], text: &str, translate: bool) -> usize {
    let mut ch: usize = ui.data(|d| d.get_temp(id)).unwrap_or(0);
    if ch >= opts.len() {
        ch = 0;
    }
    ui.horizontal(|ui| {
        label(ui, text);
        let width = if opts.len() == 1 { 160.0 } else { 110.0 };
        let key = format!("{id:?}-ch");
        if translate {
            widgets::dropdown(ui, &key, text, &mut ch, opts, width);
        } else {
            widgets::dropdown_literal(ui, &key, text, &mut ch, opts, width);
        }
    });
    ui.data_mut(|d| d.insert_temp(id, ch));
    ch
}

pub(super) fn hist_color(ch: &ToneChannel, t: &Tokens) -> Color32 {
    match ch.key {
        "" | "lightness" | "black" => Color32::from_gray(if t.pro { 150 } else { 110 }),
        k => tone::channel_color(k, t),
    }
}

pub(super) fn bar_color(ch: &ToneChannel, t: &Tokens) -> Color32 {
    match ch.key {
        "" | "lightness" | "black" => Color32::WHITE,
        k => tone::channel_color(k, t),
    }
}

/// Horizontal (or vertical) black → `to` gradient bar.
pub(super) fn gradient_bar(p: &egui::Painter, r: Rect, from: Color32, to: Color32, vertical: bool) {
    let mut mesh = egui::Mesh::default();
    let (a, b, c, d) = if vertical {
        (r.left_bottom(), r.right_bottom(), r.right_top(), r.left_top())
    } else {
        (r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom())
    };
    let (ca, cb) = if vertical { (from, from) } else { (from, to) };
    let (cc, cd) = if vertical { (to, to) } else { (to, from) };
    mesh.colored_vertex(a, ca);
    mesh.colored_vertex(b, cb);
    mesh.colored_vertex(c, cc);
    mesh.colored_vertex(d, cd);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

// ---------------------------------------------------------------------------------------------
// Curves

pub(super) const CURVE_HANDLE_SIZE: f32 = 7.0;

const CURVE_GRAPH_PADDING: f32 = 5.0;

/// The room left on screen for the Curves dialog's other rows (title, channel and mode, Input and
/// Output, eyedroppers, buttons) when its graph is as tall as it can be.
const CURVES_CHROME: f32 = 360.0;

/// The smallest and the default largest side of the Curves graph: 300 points was the fixed
/// size until #8984.
const CURVE_SIDE: (f32, f32) = (120.0, 300.0);

/// The Curves graph's side (#8984): the width it is given, in the Properties panel and in the
/// resizable dialog, and no taller than the window leaves room for beside the other rows.
fn curve_side(ui: &egui::Ui) -> f32 {
    let tallest = curve_side_max(ui.ctx());
    let width = ui.available_width();
    if width.is_finite() { width.clamp(CURVE_SIDE.0, tallest) } else { CURVE_SIDE.1 }
}

/// The largest the Curves graph gets in this window: as tall as the window leaves room for beside
/// the dialog's other rows, and never under the old 300 points. The Curves dialog is no wider.
pub(crate) fn curve_side_max(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().height() - CURVES_CHROME).max(CURVE_SIDE.1)
}

pub(super) fn curve_graph(full: Rect, side: f32) -> Rect {
    let base = Rect::from_min_size(full.min + vec2(14.0, 0.0), vec2(side - 14.0, side - 14.0));
    Rect::from_min_max(base.min + vec2(CURVE_GRAPH_PADDING, CURVE_GRAPH_PADDING), base.max - vec2(CURVE_GRAPH_PADDING, CURVE_GRAPH_PADDING))
}

pub(crate) fn read_curve(v: &Value, key: &str) -> Vec<[f32; 2]> {
    let pts: Vec<[f32; 2]> = v
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|q| {
                    let q = q.as_array()?;
                    Some([q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32])
                })
                .collect()
        })
        .unwrap_or_default();
    if pts.len() < 2 { vec![[0.0, 0.0], [255.0, 255.0]] } else { pts }
}

fn curve_key(ch: &ToneChannel) -> &'static str {
    if ch.key.is_empty() { "points" } else { ch.key }
}

/// A pencil-drawn channel: its 256 output levels (identity when absent or not a map).
fn read_map(v: &Value, key: &str) -> Vec<f32> {
    let map: Option<Vec<f32>> = v
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == photocraft_doc::adjust::CURVE_MAP_LEN)
        .and_then(|a| a.iter().map(|o| o.as_f64().map(|o| o as f32)).collect());
    map.unwrap_or_else(|| (0..photocraft_doc::adjust::CURVE_MAP_LEN).map(|k| k as f32).collect())
}

fn write_map(v: &mut Value, key: &str, map: &[f32]) {
    v[key] = json!(map.iter().map(|o| (o * 100.0).round() / 100.0).collect::<Vec<_>>());
}

/// Switches the Curves values between point editing and the pencil: the engine draws the point
/// curves into maps, or fits points to the drawn maps (Photoshop's two curve buttons).
fn set_pencil(v: &mut Value, pencil: bool, gray: bool) -> bool {
    let mut p = v.clone();
    p["pencil"] = json!(pencil);
    let mode = if gray { photocraft_doc::ColorMode::Grayscale } else { photocraft_doc::ColorMode::Rgb };
    let Ok(a) = adjust_params::from_params("curves", &p, None, mode) else { return false };
    *v = adjust_params::to_params(&a);
    // Explicit, so an update over a pencil-drawn layer leaves the pencil.
    v["pencil"] = json!(pencil);
    true
}

/// Mode buttons above the graph: points, pencil, and Smooth for the shown channel's drawing.
fn curves_mode_row(ui: &mut egui::Ui, v: &mut Value, key: &str, gray: bool) -> Edit {
    let pencil = v.get("pencil").and_then(Value::as_bool).unwrap_or(false);
    let mut e = Edit::default();
    ui.horizontal(|ui| {
        if crate::icons::button(ui, "spline", 24.0, !pencil, tl!("Edit points to modify the curve")).clicked() && pencil {
            e.add(Edit::discrete(set_pencil(v, false, gray)));
        }
        if crate::icons::button(ui, "pencil", 24.0, pencil, tl!("Draw to modify the curve")).clicked() && !pencil {
            e.add(Edit::discrete(set_pencil(v, true, gray)));
        }
        let label = crate::i18n::tr_ctx(crate::i18n::current(), "curves", "Smooth");
        let smooth = ui
            .add_enabled_ui(pencil, |ui| widgets::secondary_button(ui, label, 0.0))
            .inner
            .on_hover_text(tl!("Smooth the curve values"))
            .on_disabled_hover_text(tl!("Smooth the curve values"));
        if smooth.clicked() {
            let mut map = read_map(v, key);
            adjust_params::smooth_curve_map(&mut map);
            write_map(v, key, &map);
            e.add(Edit::discrete(true));
        }
    });
    e
}

pub(super) fn curves(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let space = space_of(v);
    let chans = tone_channels(space, cx.gray);
    let state_id = cx.mem.with("curves");
    let mut st: CurveUi = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let isolated = cx.isolated_curve.as_ref();
    let ch = if let Some(iso) = isolated {
        channel_picker_opts(ui, cx.mem.with("curves-ch"), &[(0, iso.name.as_str())], tl!("Channel:"), iso.translate)
    } else {
        channel_picker(ui, cx.mem.with("curves-ch"), chans, tl!("Channel:"))
    };
    if ch != st.channel {
        st = CurveUi { channel: ch, ..Default::default() };
    }
    let chan =
        if let Some(iso) = isolated { ToneChannel { key: iso.key, label: "" } } else { chans.get(ch).copied().unwrap_or(ToneChannel { key: "", label: "" }) };
    let overlay_others = isolated.is_none();
    let tinted = isolated.is_some_and(|c| c.tint) || (isolated.is_none() && ch != 0);
    let hist_i = isolated.map_or(ch, |c| c.hist);
    let key = curve_key(&chan);
    let mut e = curves_mode_row(ui, v, key, cx.gray);
    let pencil = v.get("pencil").and_then(Value::as_bool).unwrap_or(false);
    let mut pts = read_curve(v, key);
    let mut map = read_map(v, key);
    let side = curve_side(ui);
    ui.add_space(4.0);
    let (full, resp) = ui.allocate_exact_size(vec2(side, side + 14.0), Sense::click_and_drag());
    let graph = curve_graph(full, side);
    ui.data_mut(|d| d.insert_temp(cx.mem.with("curves-graph"), graph));
    let to_scr = |q: [f32; 2]| pos2(graph.left() + q[0] / 255.0 * graph.width(), graph.bottom() - q[1] / 255.0 * graph.height());
    let mut changed = false;
    let mut drawn = false;
    if pencil {
        // Pencil: a stroke draws while the button is down and commits on release; Shift
        // continues from the previous stroke's end in a straight line.
        let stroke_id = cx.mem.with("curves-pencil");
        let (mut active, mut last): (bool, Option<[f32; 2]>) = ui.data(|d| d.get_temp(stroke_id)).unwrap_or_default();
        if resp.is_pointer_button_down_on()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let q = [
                ((pos.x - graph.left()) / graph.width().max(1.0) * 255.0).clamp(0.0, 255.0),
                ((graph.bottom() - pos.y) / graph.height().max(1.0) * 255.0).clamp(0.0, 255.0),
            ];
            let from = if active || ui.input(|i| i.modifiers.shift) { last.unwrap_or(q) } else { q };
            drawn = curve_edit::pencil_line(&mut map, from, q);
            active = true;
            last = Some(q);
        } else if active {
            active = false;
            e.commit = true;
        }
        ui.data_mut(|d| d.insert_temp(stroke_id, (active, last)));
    } else {
        let interaction = crate::point_curve::interact(ui, &resp, graph, &mut pts, &mut st.gesture);
        changed = interaction.changed;
        e.commit |= interaction.commit;
    }

    // Draw: histogram, quarter grid, baseline, gradient bars, other channels, the curve, points.
    let p = ui.painter_at(full.expand(2.0));
    p.rect_filled(graph, 0.0, t.field);
    if let Some(h) = &cx.hist {
        tone::draw_histogram(&p, graph, &h[hist_i], Color32::from_gray(if t.pro { 88 } else { 190 }).gamma_multiply(0.75));
    }
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let g = Stroke::new(1.0, t.separator);
        p.line_segment([pos2(graph.left() + f * graph.width(), graph.top()), pos2(graph.left() + f * graph.width(), graph.bottom())], g);
        p.line_segment([pos2(graph.left(), graph.top() + f * graph.height()), pos2(graph.right(), graph.top() + f * graph.height())], g);
    }
    p.line_segment([graph.left_bottom(), graph.right_top()], Stroke::new(1.0, t.separator.gamma_multiply(1.6)));
    p.rect_stroke(
        graph,
        0.0,
        Stroke::new(if resp.has_focus() { 1.5 } else { 1.0 }, if resp.has_focus() { t.accent } else { t.field_border }),
        StrokeKind::Outside,
    );
    let hbar = Rect::from_min_max(pos2(graph.left(), graph.bottom() + 4.0), pos2(graph.right(), graph.bottom() + 12.0));
    let vbar = Rect::from_min_max(pos2(full.left(), graph.top()), pos2(full.left() + 8.0, graph.bottom()));
    gradient_bar(&p, hbar, Color32::BLACK, bar_color(&chan, &t), false);
    gradient_bar(&p, vbar, Color32::BLACK, bar_color(&chan, &t), true);
    let draw_curve = |pts: &[[f32; 2]], color: Color32, width: f32| {
        let cp: Vec<photocraft_doc::adjust::CurvePoint> =
            pts.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
        let lut = photocraft_compose::adjust::curve_lut(&cp);
        let n = lut.len().max(2);
        let line: Vec<Pos2> =
            (0..n).step_by((n / 256).max(1)).map(|i| to_scr([i as f32 / (n - 1) as f32 * 255.0, lut.get(i).copied().unwrap_or(0.0) * 255.0])).collect();
        p.add(egui::Shape::line(line, Stroke::new(width, color)));
    };
    let draw_map = |map: &[f32], color: Color32, width: f32| {
        let last = map.len().saturating_sub(1).max(1) as f32;
        let line: Vec<Pos2> = map.iter().enumerate().map(|(k, o)| to_scr([k as f32 / last * 255.0, *o])).collect();
        p.add(egui::Shape::line(line, Stroke::new(width, color)));
    };
    if overlay_others && ch == 0 {
        // Photoshop overlays the edited channel curves on the composite.
        for c in chans.iter().skip(1) {
            if pencil {
                let m = read_map(v, c.key);
                if m.iter().enumerate().any(|(k, o)| (o - k as f32).abs() > 0.5) {
                    draw_map(&m, bar_color(c, &t).gamma_multiply(0.8), 1.0);
                }
                continue;
            }
            let cp = read_curve(v, c.key);
            if cp.len() > 2 || cp.first().is_some_and(|q| q[1] != 0.0) || cp.last().is_some_and(|q| q[1] != 255.0) {
                draw_curve(&cp, bar_color(c, &t).gamma_multiply(0.8), 1.0);
            }
        }
    }
    let color = if tinted { bar_color(&chan, &t) } else { t.text };
    if pencil {
        draw_map(&map, color, 1.5);
    } else {
        draw_curve(&pts, color, 1.5);
    }
    for (i, q) in pts.iter().enumerate().filter(|_| !pencil) {
        let r = Rect::from_center_size(to_scr(*q), vec2(CURVE_HANDLE_SIZE, CURVE_HANDLE_SIZE));
        if Some(i) == st.gesture.selected {
            p.rect_filled(r, 0.0, t.text);
            p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
        } else {
            p.rect_filled(r, 0.0, t.field);
            p.rect_stroke(r, 0.0, Stroke::new(1.0, t.text), StrokeKind::Inside);
        }
    }

    // Input / Output of the selected point.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let i = st.gesture.selected.filter(|i| *i < pts.len() && !pencil);
        let (mut vi, mut vo) = i.map_or((0.0, 0.0), |i| (pts[i][0], pts[i][1]));
        label(ui, tl!("Input:"));
        let ri = ui.add_enabled_ui(i.is_some(), |ui| widgets::value_field(ui, &mut vi, 0.0..=255.0, "", 52.0).accessible_name(tl!("Input:"))).inner;
        label(ui, tl!("Output:"));
        let ro = ui.add_enabled_ui(i.is_some(), |ui| widgets::value_field(ui, &mut vo, 0.0..=255.0, "", 52.0).accessible_name(tl!("Output:"))).inner;
        if let Some(i) = i
            && (ri.changed() || ro.changed())
            && curve_edit::move_to(&mut pts, i, [vi, vo])
        {
            changed = true;
            e.commit |= !(ri.dragged() || ro.dragged());
        }
        if ri.drag_stopped() || ro.drag_stopped() {
            e.commit = true;
        }
    });
    let t2 = Tokens::get(ui.ctx());
    let hint = if pencil {
        crate::i18n::fmt(tl!("Drag to draw the curve · {key}-click draws a straight line"), &[("key", &crate::shortcuts::pretty("Shift"))])
    } else {
        crate::i18n::fmt(tl!("Click to add a point · drag off or {key}-click or Delete to remove"), &[("key", &crate::shortcuts::pretty("Cmd"))])
    };
    ui.label(RichText::new(hint).color(t2.text_faint).size(11.0));
    ui.data_mut(|d| d.insert_temp(state_id, st));
    if drawn {
        write_map(v, key, &map);
        if space == ToneSpace::Lab
            && let Some(o) = v.as_object_mut()
        {
            o.remove("points");
        }
        e.changed = true;
    }
    if changed && !pencil {
        v[key] = json!(pts.iter().map(|q| [q[0].round(), q[1].round()]).collect::<Vec<_>>());
        if space == ToneSpace::Lab {
            // Lab has no composite: "points" would alias lightness.
            if let Some(o) = v.as_object_mut() {
                o.remove("points");
            }
        }
        e.changed = true;
    }
    e
}
