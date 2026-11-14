use super::hue_saturation::spectrum_bar;
use super::{
    Color32, Edit, EditorCx, GradientPreset, Group, PhotocraftApp, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Tokens, Value, adjust_params, color_button,
    color32, flag, json, label, pos2, rgb_of, round, vec2, widgets,
};

// ---------------------------------------------------------------------------------------------
// Gradient Map

type Stop = (f32, [f32; 3]);

/// The stops the editor shows: Photoshop's own when the map keeps them (`source`, #8351), else
/// the map's.
pub(super) fn read_stops(v: &Value) -> Vec<Stop> {
    let held = v.get("source").filter(|s| s.is_object()).unwrap_or(v);
    let mut s: Vec<Stop> = held
        .get("stops")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let p = s.as_array()?;
                    let t = p.first()?.as_f64()? as f32;
                    let c = rgb_of(&json!({"c": p.get(1)?.clone()}), "c", [0.0; 3]);
                    Some((t.clamp(0.0, 1.0), c))
                })
                .collect()
        })
        .unwrap_or_default();
    if s.len() < 2 {
        s = vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])];
    }
    s
}

/// Write the edited stops back: into Photoshop's gradient when the map keeps one (its midpoints
/// follow the segments, a new one halfway; the engine bakes it again), else as the map's stops.
pub(super) fn write_stops(v: &mut Value, s: &[Stop]) {
    let mut s = s.to_vec();
    s.sort_by(|a, b| a.0.total_cmp(&b.0));
    let stops = json!(s.iter().map(|(t, c)| json!([round(*t, 0.001), adjust_params::hex(*c)])).collect::<Vec<_>>());
    match v.get_mut("source").filter(|src| src.is_object()) {
        Some(src) => {
            let segments = s.len().saturating_sub(1);
            let mut mids: Vec<Value> = src.get("midpoints").and_then(Value::as_array).cloned().unwrap_or_default();
            mids.resize(segments, json!(0.5));
            src["midpoints"] = Value::Array(mids);
            src["stops"] = stops;
        }
        None => v["stops"] = stops,
    }
}

fn sample_stops(s: &[Stop], t: f32) -> [f32; 3] {
    let mut sorted = s.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = sorted.first() else { return [t; 3] };
    if t <= first.0 {
        return first.1;
    }
    for w in sorted.windows(2) {
        if t <= w[1].0 {
            let k = if w[1].0 > w[0].0 { (t - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
            return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
        }
    }
    sorted.last().map_or([t; 3], |l| l.1)
}

#[derive(Clone, Copy)]
struct MapStopDrag {
    index: usize,
    offset: f32,
    changed: bool,
    /// Dragged off the strip: out of the gradient, kept to come back if the drag returns.
    removed: Option<Stop>,
}

/// Keep selection on the same stop when it crosses another stop (including coincident stops).
fn move_map_stop(stops: &mut [Stop], index: usize, location: f32) -> Option<usize> {
    let stop = stops.get_mut(index)?;
    stop.0 = round(location.clamp(0.0, 1.0), 0.001);
    let location = stop.0;
    let rank = stops.iter().enumerate().filter(|(i, s)| *i != index && (s.0 < location || (s.0 == location && *i < index))).count();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(rank)
}

fn gradient_map_strip(ui: &mut egui::Ui, stops: &mut Vec<Stop>, cx: &EditorCx, selected: &mut usize) -> Edit {
    let t = Tokens::get(ui.ctx());
    let key = cx.mem.with("gm-stop-drag");
    let mut edit = Edit::default();
    let w = ui.available_width().min(300.0);
    let (outer, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Gradient stops")));
    let bar = Rect::from_min_max(outer.min + vec2(8.0, 2.0), pos2(outer.right() - 8.0, outer.top() + 26.0));
    let x_of = |loc: f32| bar.left() + loc * bar.width();
    let loc_of = |x: f32| ((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
    let hit =
        |pos: Pos2, stops: &[Stop]| stops.iter().enumerate().rev().find(|(_, s)| pos.y >= bar.bottom() && (x_of(s.0) - pos.x).abs() <= 6.0).map(|(i, _)| i);
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && let Some(index) = hit(pos, stops)
    {
        *selected = index;
        let offset = stops.get(index).map_or(0.0, |s| pos.x - x_of(s.0));
        ui.data_mut(|d| d.insert_temp(key, MapStopDrag { index, offset, changed: false, removed: None }));
    }
    if let Some(mut drag) = ui.data(|d| d.get_temp::<MapStopDrag>(key)) {
        if (resp.dragged() || resp.drag_stopped())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let location = round(loc_of(pos.x - drag.offset), 0.001);
            // As far off the strip as the Gradient tool's; two stops always stay.
            let off = crate::gradient_ui::off_strip(pos, bar);
            if off && drag.removed.is_none() && stops.len() > 2 && drag.index < stops.len() {
                drag.removed = Some(stops.remove(drag.index));
                *selected = drag.index.min(stops.len() - 1);
                drag.changed = true;
                edit.changed = true;
            } else if let Some(stop) = drag.removed.filter(|_| !off) {
                drag.removed = None;
                stops.push((location, stop.1));
                drag.index = stops.len() - 1;
                // Changed, wherever it lands: the preview had it removed.
                edit.changed = true;
            }
            if drag.removed.is_none() {
                let changed = stops.get(drag.index).is_some_and(|s| s.0 != location);
                if let Some(index) = move_map_stop(stops, drag.index, location) {
                    drag.index = index;
                    drag.changed |= changed;
                    *selected = index;
                    edit.changed |= changed;
                }
            }
        }
        // A drag can also end without `drag_stopped` (released outside the window, focus lost):
        // either way what the preview shows is committed, so a stop taken off the strip isn't left
        // removed with no History step.
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            edit.commit = drag.changed;
            ui.data_mut(|d| d.remove::<MapStopDrag>(key));
        } else {
            ui.data_mut(|d| d.insert_temp(key, drag));
        }
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        resp.request_focus();
        if let Some(index) = hit(pos, stops) {
            *selected = index;
        } else if stops.len() < 64 {
            let at = round(loc_of(pos.x), 0.001);
            let color = sample_stops(stops, at);
            stops.push((at, color));
            let index = stops.len() - 1;
            *selected = move_map_stop(stops, index, at).unwrap_or(0);
            edit.add(Edit::discrete(true));
        }
    }
    if resp.hovered() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| hit(pos, stops).is_some()) || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    // Markers address un-reversed locations, as in the Gradient Tool; Reverse changes the image.
    let cols: Vec<Color32> = (0..=48).map(|i| color32(sample_stops(stops, i as f32 / 48.0))).collect();
    spectrum_bar(ui.painter(), bar, &cols);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    for (i, (at, color)) in stops.iter().enumerate() {
        let x = x_of(*at);
        let border = if i == *selected { t.accent } else { t.text_dim };
        let body = Rect::from_center_size(pos2(x, bar.bottom() + 10.0), vec2(10.0, 10.0));
        ui.painter().add(egui::Shape::convex_polygon(
            vec![pos2(x, bar.bottom() + 1.0), pos2(x + 5.0, body.top()), pos2(x - 5.0, body.top())],
            border,
            Stroke::NONE,
        ));
        ui.painter().rect_filled(body, 1.0, color32(*color));
        ui.painter().rect_stroke(body, 1.0, Stroke::new(1.0, border), StrokeKind::Outside);
    }
    edit
}

pub(super) fn gradient_map(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let mut e = Edit::default();
    let mut stops = read_stops(v);
    let selection = cx.mem.with("gm-stop-selection");
    let mut selected = ui.data(|d| d.get_temp::<usize>(selection)).unwrap_or(0).min(stops.len().saturating_sub(1));
    // Photoshop's gradient picker: the same library as the Gradient tool and Gradients panel
    // (built-in groups and the user's own), Foreground/Background resolved to the current colours.
    let [fg, bg] = cx.swatches.map(|c| [c[0], c[1], c[2], 1.0]);
    ui.horizontal(|ui| {
        label(ui, tl!("Preset:"));
        let mut picked = None;
        let (press, close) = crate::press_menu::PressCombo::before(ui, cx.mem.with("gm-preset"));
        let width = ui.available_width().min(190.0);
        let combo = widgets::combo_list(ui, cx.mem.with("gm-preset"), tl!("Custom"), width, close, |ui| {
            let gear = crate::icons::button(ui, "settings", 24.0, false, tl!("Gradient Preset Options"));
            egui::containers::menu::SubMenu::new().show(ui, &gear, |ui| {
                if ui.button(tl!("Import .GRD")).clicked() {
                    e.import_gradients = true;
                    ui.close();
                }
            });
            ui.separator();
            for g in &cx.gradients {
                ui.label(RichText::new(&g.name).color(t.text_faint));
                for p in &g.items {
                    ui.horizontal(|ui| {
                        // A map has no transparency: colour stops only, shown opaque.
                        let rgba: Vec<(f32, [f32; 4])> = p.resolve(fg, bg).into_iter().map(|(at, c)| (at, [c[0], c[1], c[2], 1.0])).collect();
                        let (r, swatch) = ui.allocate_exact_size(vec2(48.0, 16.0), Sense::click());
                        crate::preset_panels::paint_gradient(ui, r, &rgba);
                        // Both, not short-circuited: the name is drawn either way.
                        let name = ui.selectable_label(false, &p.name);
                        if press.chosen(ui, &swatch) | press.chosen(ui, &name) {
                            picked = Some(rgba.iter().map(|(at, c)| (*at, [c[0], c[1], c[2]])).collect::<Vec<Stop>>());
                        }
                    });
                }
            }
        });
        press.after(&combo.response);
        let _ = combo.response.accessible_name(tl!("Preset:"));
        if let Some(p) = picked {
            stops = p;
            selected = 0;
            ui.data_mut(|d| d.remove::<MapStopDrag>(cx.mem.with("gm-stop-drag")));
            // A preset replaces the gradient, Photoshop's settings with it.
            if let Some(o) = v.as_object_mut() {
                o.insert("source".into(), Value::Null);
            }
            write_stops(v, &stops);
            e.add(Edit::discrete(true));
        }
    });
    let strip = gradient_map_strip(ui, &mut stops, cx, &mut selected);
    let mut changed = strip.changed;
    e.add(strip);
    // The selected stop's colour and location, like the Gradient Tool's compact stop editor.
    let mut location = None;
    let mut remove = false;
    let can_remove = stops.len() > 2;
    if let Some(s) = stops.get_mut(selected) {
        ui.horizontal(|ui| {
            label(ui, tl!("Color:"));
            let r = color_button(ui, &mut s.1, tl!("Stop Color"));
            changed |= r.changed;
            e.add(r);
            ui.add_enabled_ui(can_remove, |ui| {
                remove = crate::icons::button(ui, "x", 20.0, false, tl!("Remove stop")).clicked();
            });
        });
        ui.horizontal(|ui| {
            label(ui, tl!("Location:"));
            let mut loc = s.0 * 100.0;
            let rl = widgets::value_field(ui, &mut loc, 0.0..=100.0, "%", 70.0).accessible_name(tl!("Location:"));
            if rl.changed() {
                location = Some(loc / 100.0);
                changed = true;
            }
            e.add(Edit::of(&rl));
        });
    }
    if let Some(loc) = location {
        selected = move_map_stop(&mut stops, selected, loc).unwrap_or(0);
    }
    if remove && can_remove {
        stops.remove(selected);
        selected = selected.min(stops.len() - 1);
        changed = true;
        e.add(Edit::discrete(true));
    }
    if stops.len() < 64 && widgets::secondary_button(ui, tl!("Add Stop"), 0.0).clicked() {
        // Halfway along the widest gap, in the gradient's colour there.
        let mut sorted = stops.clone();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let at = sorted.windows(2).max_by(|a, b| (a[1].0 - a[0].0).total_cmp(&(b[1].0 - b[0].0))).map_or(0.5, |w| (w[0].0 + w[1].0) / 2.0);
        stops.push((at, sample_stops(&stops, at)));
        let index = stops.len() - 1;
        selected = move_map_stop(&mut stops, index, at).unwrap_or(0);
        changed = true;
        e.add(Edit::discrete(true));
    }
    if changed {
        write_stops(v, &stops);
        e.changed = true;
    }
    ui.data_mut(|d| d.insert_temp(selection, selected));
    // The rest of Photoshop's gradient, kept as it came (#8351).
    if let Some(src) = v.get("source").filter(|s| s.is_object()) {
        let smooth = src.get("smoothness").and_then(Value::as_f64).unwrap_or(0.0) * 100.0;
        let method = match src.get("method").and_then(Value::as_str).unwrap_or("classic") {
            "perceptual" => tl!("Perceptual"),
            "linear" => tl!("Linear"),
            "smooth" => tl!("Smooth"),
            _ => tl!("Classic"),
        };
        let line = crate::i18n::fmt(tl!("Smoothness {s}% · {method}"), &[("s", &format!("{smooth:.0}")), ("method", method)]);
        ui.label(RichText::new(line).color(t.text_faint));
    }
    ui.horizontal(|ui| {
        for (key, text) in [("reverse", tl!("Reverse")), ("dither", tl!("Dither"))] {
            let mut b = flag(v, key, false);
            if widgets::checkbox(ui, &mut b, text).changed() {
                v[key] = json!(b);
                e.add(Edit::discrete(true));
            }
        }
    });
    e
}

// ---------------------------------------------------------------------------------------------
// Hosts

/// Whether a document of this mode edits Levels/Curves through a single Gray channel.
pub fn is_gray(mode: photocraft_doc::ColorMode) -> bool {
    matches!(mode, photocraft_doc::ColorMode::Grayscale | photocraft_doc::ColorMode::Duotone | photocraft_doc::ColorMode::Bitmap)
}

/// The gradient preset library for a `kind` editor's picker (Gradient Map only; empty otherwise).
pub fn gradient_presets(app: &PhotocraftApp, kind: &str) -> Vec<Group<GradientPreset>> {
    if kind == "gradientMap" { app.session.presets.gradients.clone() } else { Vec::new() }
}

pub fn swatches(app: &PhotocraftApp) -> [[f32; 3]; 2] {
    let c = |x: [f32; 4]| [x[0], x[1], x[2]];
    [c(app.session.tools.foreground), c(app.session.tools.background)]
}
