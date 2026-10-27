//! The shape Fill / Stroke swatches' Gradient tab, as Photoshop's popover: the recently used
//! gradients and the preset library, a stops strip (click to add a stop, drag one to move it or
//! off the strip to delete it, drag a midpoint), the selected stop's colour, opacity and
//! location, and the gradient's style, angle, scale, reverse and Align with layer. Stop edits use
//! the Gradient Editor's grammar ([`apply_stop`]); the tab returns the edited gradient and the
//! swatch's caller applies it to the next shape or as one `shape.edit`.

use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_compose::gradient_fill as gf;
use photocraft_doc::{Fill, GradientStyle};
use photocraft_engine::gradient_fill_cmds::apply_stop;
use photocraft_engine::presets::Group;
use photocraft_engine::presets::gradients::GradientPreset;
use serde_json::{Value, json};

use crate::shape_paint_ui::{picker_color, picker_rgba, rgba_of, rgba32};
use crate::theme::Tokens;

/// Width of the tab's rows (the strip, the swatch grids).
const WIDTH: f32 = 228.0;
/// How many gradients the Recent row keeps.
const RECENT: usize = 8;

/// What a swatch's Gradient tab offers: the recently used gradients, the preset library, and the
/// Gradient tool's current gradient (what a colour switches to), with the tool colours that
/// resolve their Foreground / Background stops.
pub(crate) struct GradientChoices<'a> {
    pub presets: &'a [Group<GradientPreset>],
    pub current: &'a GradientPreset,
    /// Newest first ([`GradientChoices::remember`]).
    pub recent: &'a mut Vec<Fill>,
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}

impl GradientChoices<'_> {
    /// Photoshop's shape gradient default: the current gradient, linear at 90°, 100%.
    pub(crate) fn default_fill(&self) -> Fill {
        let mut fill = Fill::gradient(Vec::new(), 90.0, 1.0, GradientStyle::Linear, false);
        take_preset(&mut fill, self.current, self.fg, self.bg);
        fill
    }

    /// Puts `gradient`'s stops at the front of the Recent row; a gradient already there moves up.
    pub(crate) fn remember(&mut self, gradient: &Fill) {
        let Fill::Gradient { .. } = gradient else { return };
        self.recent.retain(|f| !same_stops(f, gradient));
        self.recent.insert(0, gradient.clone());
        self.recent.truncate(RECENT);
    }
}

/// Whether two gradients have the same colour stops, opacity stops and midpoints.
fn same_stops(a: &Fill, b: &Fill) -> bool {
    match (a, b) {
        (Fill::Gradient { stops: s1, opacity_stops: o1, midpoints: m1, .. }, Fill::Gradient { stops: s2, opacity_stops: o2, midpoints: m2, .. }) => {
            s1 == s2 && o1 == o2 && m1 == m2
        }
        _ => false,
    }
}

/// `fill` takes `preset`'s colour and opacity stops (live colours resolved now), as a Gradient
/// Fill layer made from it holds them; its old midpoints belonged to the old stops.
pub(crate) fn take_preset(fill: &mut Fill, preset: &GradientPreset, fg: [f32; 4], bg: [f32; 4]) {
    if let Fill::Gradient { stops, opacity_stops, midpoints, .. } = fill {
        (*stops, *opacity_stops) = preset.fill_stops(fg, bg);
        midpoints.clear();
    }
}

/// `fill` takes `from`'s colour stops, opacity stops and midpoints (a Recent pick).
fn take_stops(fill: &mut Fill, from: &Fill) {
    if let (Fill::Gradient { stops, opacity_stops, midpoints, .. }, Fill::Gradient { stops: s, opacity_stops: o, midpoints: m, .. }) = (fill, from) {
        (*stops, *opacity_stops, *midpoints) = (s.clone(), o.clone(), m.clone());
    }
}

/// The Gradient tab of the swatch `id`. Returns the edited gradient.
pub(crate) fn gradient_tab(ui: &mut egui::Ui, fill: &Fill, choices: &GradientChoices, id: egui::Id) -> Option<Fill> {
    let Fill::Gradient { angle, scale, style, reverse, align, .. } = fill else { return None };
    let t = Tokens::get(ui.ctx());
    let mut out: Option<Fill> = None;
    let mut picked = false;
    egui::ScrollArea::vertical().id_salt(id.with("gradient-presets")).max_height(120.0).show(ui, |ui| {
        ui.set_width(WIDTH);
        let group_label = |ui: &mut egui::Ui, s: &str| ui.label(egui::RichText::new(s).color(t.text_dim).size(11.0));
        if !choices.recent.is_empty() {
            group_label(ui, tl!("Recent"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                for (i, recent) in choices.recent.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
                    paint_fill_ramp(ui, r, recent, resp.hovered());
                    let name = format!("{} {}", tl!("Recent"), i + 1);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                    if resp.clicked() {
                        take_stops(edited(&mut out, fill), recent);
                        picked = true;
                    }
                }
            });
        }
        for group in choices.presets {
            // Group and preset names are the library's data, shown as they are.
            group_label(ui, &group.name);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                for preset in &group.items {
                    let (r, resp) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
                    crate::preset_panels::paint_gradient(ui, r, &preset.resolve(choices.fg, choices.bg));
                    let border = if resp.hovered() { t.text_dim } else { t.field_border };
                    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, border), StrokeKind::Outside);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &preset.name));
                    if resp.on_hover_text(&preset.name).clicked() {
                        take_preset(edited(&mut out, fill), preset, choices.fg, choices.bg);
                        picked = true;
                    }
                }
            });
        }
    });
    if picked {
        // New stops: start from the first colour stop, its colour row showing.
        ui.data_mut(|d| d.insert_temp(selection_key(id), Marker::Color(0)));
    }
    ui.add_space(6.0);
    if let Some(f) = stops_strip(ui, out.as_ref().unwrap_or(fill), id, choices.fg, choices.bg) {
        out = Some(f);
    }
    if let Some(f) = stop_fields(ui, out.as_ref().unwrap_or(fill), id, choices.fg, choices.bg) {
        out = Some(f);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (s, icon, tip) in [
            (GradientStyle::Linear, "gradient", tl!("Linear Gradient")),
            (GradientStyle::Radial, "circle", tl!("Radial Gradient")),
            (GradientStyle::Angle, "rotate-cw", tl!("Angle Gradient")),
            (GradientStyle::Reflected, "arrow-left-right", tl!("Reflected Gradient")),
            (GradientStyle::Diamond, "diamond", tl!("Diamond Gradient")),
        ] {
            if crate::icons::button(ui, icon, 22.0, *style == s, tip).clicked()
                && *style != s
                && let Fill::Gradient { style, .. } = edited(&mut out, fill)
            {
                *style = s;
            }
        }
    });
    let label = |ui: &mut egui::Ui, s: &str| ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    ui.horizontal(|ui| {
        label(ui, tl!("Angle"));
        let mut a = *angle;
        if crate::widgets::value_field(ui, &mut a, -180.0..=180.0, "°", 52.0).changed()
            && let Fill::Gradient { angle, .. } = edited(&mut out, fill)
        {
            *angle = a;
        }
        label(ui, tl!("Scale"));
        let mut sc = *scale * 100.0;
        if crate::widgets::value_field(ui, &mut sc, 10.0..=150.0, "%", 52.0).changed()
            && let Fill::Gradient { scale, .. } = edited(&mut out, fill)
        {
            *scale = sc / 100.0;
        }
    });
    ui.horizontal(|ui| {
        let mut rev = *reverse;
        if crate::widgets::checkbox(ui, &mut rev, tl!("Reverse")).changed()
            && let Fill::Gradient { reverse, .. } = edited(&mut out, fill)
        {
            *reverse = rev;
        }
        let mut al = *align;
        if crate::widgets::checkbox(ui, &mut al, tl!("Align with layer")).changed()
            && let Fill::Gradient { align, .. } = edited(&mut out, fill)
        {
            *align = al;
        }
    });
    out
}

/// The gradient being edited this frame: `fill` until a control changes it.
fn edited<'a>(out: &'a mut Option<Fill>, fill: &Fill) -> &'a mut Fill {
    out.get_or_insert_with(|| fill.clone())
}

/// A gradient's ramp in `r` (un-reversed, as the stops strip shows it), outlined.
fn paint_fill_ramp(ui: &egui::Ui, r: Rect, fill: &Fill, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    if let Some(ramp) = gf::Ramp::new(&unreversed(fill)) {
        crate::gradient_ui::paint_ramp(ui.painter(), r, |u| ramp.sample(u));
    }
    let border = if hovered { t.text_dim } else { t.field_border };
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, border), StrokeKind::Outside);
}

fn unreversed(fill: &Fill) -> Fill {
    let mut f = fill.clone();
    if let Fill::Gradient { reverse, .. } = &mut f {
        *reverse = false;
    }
    f
}

/// A marker of the stops strip.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Marker {
    Color(usize),
    Opacity(usize),
    /// The midpoint of the segment after colour stop `i`.
    Mid(usize),
}

/// A drag in progress: the marker and the gradient as it was when the drag began. Each frame
/// re-applies the move to that gradient, so a stop dragged past another stays the one dragged.
#[derive(Clone)]
struct Drag {
    marker: Marker,
    start: Fill,
}

/// Colour stops, opacity stops and midpoints.
type Stops = (Vec<(f32, photocraft_doc::Color)>, Vec<(f32, f32)>, Vec<f32>);

/// The colour stops (sorted), opacity stops (sorted; two opaque ones when the gradient has none,
/// as [`apply_stop`] counts them) and midpoints of `fill`.
fn sorted_stops(fill: &Fill) -> Stops {
    let Fill::Gradient { stops, opacity_stops, midpoints, .. } = fill else { return (Vec::new(), Vec::new(), Vec::new()) };
    let mut cs = stops.clone();
    cs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut os = if opacity_stops.is_empty() { vec![(0.0, 1.0), (1.0, 1.0)] } else { opacity_stops.clone() };
    os.sort_by(|a, b| a.0.total_cmp(&b.0));
    (cs, os, midpoints.clone())
}

/// Where a stop moved to `location` lands once the stops are sorted again: the selection follows it.
fn index_after_move(locations: &[f32], moved: usize, location: f32) -> usize {
    locations.iter().enumerate().filter(|(i, l)| *i != moved && (**l < location || (**l == location && *i < moved))).count()
}

fn apply(fill: &Fill, edit: &Value, fg: [f32; 4], bg: [f32; 4]) -> Option<Fill> {
    apply_stop(fill, edit, fg, bg).ok()
}

/// Where swatch `id` keeps its selected marker.
fn selection_key(id: egui::Id) -> egui::Id {
    id.with("stops-selected")
}

/// Where swatch `id` keeps whether the selected colour stop's inline picker is open.
fn picker_key(id: egui::Id) -> egui::Id {
    id.with("stop-picker-open")
}

/// The selected marker, when it still names a stop of `fill`; else the first colour stop. The
/// marker is an index, so a preset with fewer stops, a deleted stop or another shape would
/// otherwise leave it pointing past the stops (and the stop's fields would vanish).
fn selection(ui: &egui::Ui, id: egui::Id, fill: &Fill) -> Marker {
    let (cs, os, _) = sorted_stops(fill);
    let valid = |m: &Marker| match *m {
        Marker::Color(i) => i < cs.len(),
        Marker::Opacity(i) => i < os.len(),
        Marker::Mid(i) => i.saturating_add(1) < cs.len(),
    };
    ui.data(|d| d.get_temp::<Marker>(selection_key(id))).filter(valid).unwrap_or(Marker::Color(0))
}

/// The stops strip: the ramp with opacity stops above and colour stops below (Photoshop's).
fn stops_strip(ui: &mut egui::Ui, fill: &Fill, id: egui::Id, fg: [f32; 4], bg: [f32; 4]) -> Option<Fill> {
    let t = Tokens::get(ui.ctx());
    let (outer, resp) = ui.allocate_exact_size(vec2(WIDTH, 58.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Gradient stops")));
    let top = outer.top() + 16.0;
    let bar = Rect::from_min_max(pos2(outer.left() + 8.0, top), pos2(outer.right() - 8.0, top + 24.0));
    let x_of = |loc: f32| bar.left() + loc.clamp(0.0, 1.0) * bar.width();
    let loc_of = |x: f32| ((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
    if let Some(ramp) = gf::Ramp::new(&unreversed(fill)) {
        crate::gradient_ui::paint_ramp(ui.painter(), bar, |u| ramp.sample(u));
    }
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let (cs, os, mids) = sorted_stops(fill);
    let (drag_key, selection_key) = (id.with("stops-drag"), selection_key(id));
    let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_key));
    let selected: Option<Marker> = Some(drag.as_ref().map_or_else(|| selection(ui, id, fill), |d| d.marker));
    let p = ui.painter();
    let outline = |sel: bool| if sel { t.accent } else { t.text_dim };
    for (i, (loc, c)) in cs.iter().enumerate() {
        let x = x_of(*loc);
        let sel = selected == Some(Marker::Color(i));
        let body = Rect::from_center_size(pos2(x, bar.bottom() + 10.0), vec2(10.0, 10.0));
        p.add(egui::Shape::convex_polygon(vec![pos2(x, bar.bottom() + 1.0), pos2(x + 5.0, body.top()), pos2(x - 5.0, body.top())], outline(sel), Stroke::NONE));
        let [r, g, b, _] = rgba_of(c);
        p.rect_filled(body, 1.0, rgba32([r, g, b, 1.0]));
        p.rect_stroke(body, 1.0, Stroke::new(1.0, outline(sel)), StrokeKind::Outside);
    }
    let mid_x = |i: usize| -> Option<f32> {
        let (a, b) = (cs.get(i)?.0, cs.get(i + 1)?.0);
        (b - a > 1e-3).then(|| x_of(a + (b - a) * mids.get(i).copied().unwrap_or(0.5)))
    };
    for i in 0..cs.len().saturating_sub(1) {
        if let Some(x) = mid_x(i) {
            let sel = selected == Some(Marker::Mid(i));
            crate::gradient_ui::diamond(p, pos2(x, bar.bottom() + 6.0), 3.0, if sel { t.accent } else { t.text_faint }, t.text_faint);
        }
    }
    for (i, (loc, a)) in os.iter().enumerate() {
        let x = x_of(*loc);
        let sel = selected == Some(Marker::Opacity(i));
        let body = Rect::from_center_size(pos2(x, bar.top() - 10.0), vec2(10.0, 10.0));
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x, bar.top() - 1.0), pos2(x + 5.0, body.bottom()), pos2(x - 5.0, body.bottom())],
            outline(sel),
            Stroke::NONE,
        ));
        p.rect_filled(body, 1.0, egui::Color32::from_gray((a.clamp(0.0, 1.0) * 255.0) as u8));
        p.rect_stroke(body, 1.0, Stroke::new(1.0, outline(sel)), StrokeKind::Outside);
    }
    let hit = |pos: Pos2| -> Option<Marker> {
        if pos.y > bar.bottom() {
            if pos.y >= bar.bottom() + 3.0
                && let Some(i) = cs.iter().position(|(l, _)| (x_of(*l) - pos.x).abs() <= 6.0)
            {
                return Some(Marker::Color(i));
            }
            return (0..cs.len().saturating_sub(1)).find(|i| mid_x(*i).is_some_and(|x| (x - pos.x).abs() <= 4.0)).map(Marker::Mid);
        }
        if pos.y < bar.top() {
            return os.iter().position(|(l, _)| (x_of(*l) - pos.x).abs() <= 6.0).map(Marker::Opacity);
        }
        None
    };
    let mut out = None;
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && let Some(marker) = hit(pos)
    {
        ui.data_mut(|d| {
            d.insert_temp(drag_key, Drag { marker, start: fill.clone() });
            d.insert_temp(selection_key, marker);
        });
    }
    if let Some(drag) = ui.data(|d| d.get_temp::<Drag>(drag_key)) {
        let (start_cs, start_os, _) = sorted_stops(&drag.start);
        let pos = resp.interact_pointer_pos();
        if let Some(pos) = pos {
            // Dragged well off the strip, a stop is deleted (while at least two remain), as in Photoshop.
            let off = pos.y > bar.bottom() + 28.0 || pos.y < bar.top() - 26.0;
            let loc = loc_of(pos.x);
            let edit = match drag.marker {
                Marker::Color(i) if off && start_cs.len() > 2 => json!({"action": "delete", "index": i}),
                Marker::Color(i) => json!({"action": "move", "index": i, "location": loc}),
                Marker::Opacity(i) if off && start_os.len() > 2 => json!({"action": "delete", "kind": "opacity", "index": i}),
                Marker::Opacity(i) => json!({"action": "move", "kind": "opacity", "index": i, "location": loc}),
                Marker::Mid(i) => {
                    let (a, b) = (start_cs.get(i).map_or(0.0, |s| s.0), start_cs.get(i + 1).map_or(1.0, |s| s.0));
                    json!({"action": "midpoint", "index": i, "location": ((loc - a) / (b - a).max(1e-4)).clamp(0.05, 0.95)})
                }
            };
            out = apply(&drag.start, &edit, fg, bg);
            if resp.drag_stopped() {
                let moved = edit.get("location").and_then(Value::as_f64).map(|l| l as f32);
                let selection = match (drag.marker, edit.get("action").and_then(Value::as_str), moved) {
                    (Marker::Color(i), Some("move"), Some(l)) => Some(Marker::Color(index_after_move(&start_cs.iter().map(|s| s.0).collect::<Vec<_>>(), i, l))),
                    (Marker::Opacity(i), Some("move"), Some(l)) => {
                        Some(Marker::Opacity(index_after_move(&start_os.iter().map(|s| s.0).collect::<Vec<_>>(), i, l)))
                    }
                    (Marker::Mid(i), ..) => Some(Marker::Mid(i)),
                    _ => None,
                };
                ui.data_mut(|d| match selection {
                    Some(m) => {
                        d.insert_temp(selection_key, m);
                    }
                    None => d.remove::<Marker>(selection_key),
                });
            }
        }
        if resp.drag_stopped() || pos.is_none() {
            ui.data_mut(|d| d.remove::<Drag>(drag_key));
        }
        return out;
    }
    // Double-clicking a colour stop opens its colour, as Photoshop's opens the Color Picker. egui
    // counts a click within 0.6 s of the one before last as a triple click (a preset picked just
    // before), which is a double click on the stop all the same.
    if (resp.double_clicked() || resp.triple_clicked())
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(marker @ Marker::Color(_)) = hit(pos)
    {
        ui.data_mut(|d| {
            d.insert_temp(selection_key, marker);
            d.insert_temp(picker_key(id), true);
        });
        return None;
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        if let Some(marker) = hit(pos) {
            ui.data_mut(|d| d.insert_temp(selection_key, marker));
            return None;
        }
        // A click on the ramp adds a stop there: an opacity stop above its middle, a colour stop below.
        let loc = loc_of(pos.x);
        let opacity = pos.y < bar.center().y;
        let (edit, marker) = if opacity {
            (json!({"action": "add", "kind": "opacity", "location": loc}), Marker::Opacity(os.iter().filter(|s| s.0 <= loc).count()))
        } else {
            (json!({"action": "add", "location": loc}), Marker::Color(cs.iter().filter(|s| s.0 <= loc).count()))
        };
        out = apply(fill, &edit, fg, bg);
        if out.is_some() {
            ui.data_mut(|d| d.insert_temp(selection_key, marker));
        }
    }
    out
}

/// The selected stop's fields under the strip: a colour stop's colour (an inline picker opened
/// from its swatch), an opacity stop's opacity, a midpoint's position; Location and Delete.
fn stop_fields(ui: &mut egui::Ui, fill: &Fill, id: egui::Id, fg: [f32; 4], bg: [f32; 4]) -> Option<Fill> {
    let t = Tokens::get(ui.ctx());
    let selection_key = selection_key(id);
    let selected = selection(ui, id, fill);
    let (cs, os, mids) = sorted_stops(fill);
    let label = |ui: &mut egui::Ui, s: &str| ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    let mut edit: Option<Value> = None;
    let mut reselect: Option<Marker> = None;
    let (locations, index): (Vec<f32>, usize) = match selected {
        Marker::Color(i) => (cs.iter().map(|s| s.0).collect(), i),
        Marker::Opacity(i) => (os.iter().map(|s| s.0).collect(), i),
        Marker::Mid(i) => (Vec::new(), i),
    };
    let picker_key = picker_key(id);
    let mut picker_open = ui.data(|d| d.get_temp::<bool>(picker_key)).unwrap_or(false);
    ui.horizontal(|ui| match selected {
        Marker::Color(i) => {
            let Some((_, c)) = cs.get(i) else { return };
            label(ui, tl!("Color"));
            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
            let [cr, cg, cb, _] = rgba_of(c);
            ui.painter().rect_filled(r.shrink(1.0), t.radius_sm, rgba32([cr, cg, cb, 1.0]));
            ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, if picker_open { t.accent } else { t.field_border }), StrokeKind::Inside);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, true, tl!("Color")));
            if resp.on_hover_text(tl!("Color")).clicked() {
                picker_open = !picker_open;
            }
        }
        Marker::Opacity(i) => {
            let Some((_, a)) = os.get(i) else { return };
            label(ui, tl!("Opacity"));
            let mut v = a * 100.0;
            if crate::widgets::value_field(ui, &mut v, 0.0..=100.0, "%", 52.0).changed() {
                edit = Some(json!({"action": "opacity", "kind": "opacity", "index": i, "opacity": v}));
            }
        }
        Marker::Mid(i) => {
            if cs.get(i.saturating_add(1)).is_none() {
                return;
            }
            label(ui, tl!("Location"));
            let mut v = mids.get(i).copied().unwrap_or(0.5) * 100.0;
            if crate::widgets::value_field(ui, &mut v, 5.0..=95.0, "%", 52.0).changed() {
                edit = Some(json!({"action": "midpoint", "index": i, "location": v / 100.0}));
            }
        }
    });
    if let Some(loc) = locations.get(index).copied() {
        let kind = if matches!(selected, Marker::Opacity(_)) { "opacity" } else { "color" };
        ui.horizontal(|ui| {
            label(ui, tl!("Location"));
            let mut v = loc * 100.0;
            if crate::widgets::value_field(ui, &mut v, 0.0..=100.0, "%", 52.0).changed() {
                let to = v / 100.0;
                edit = Some(json!({"action": "move", "kind": kind, "index": index, "location": to}));
                reselect = Some(match selected {
                    Marker::Opacity(_) => Marker::Opacity(index_after_move(&locations, index, to)),
                    _ => Marker::Color(index_after_move(&locations, index, to)),
                });
            }
            // A gradient keeps at least two stops of each kind.
            if locations.len() > 2 && crate::widgets::secondary_button(ui, tl!("Delete"), 0.0).clicked() {
                edit = Some(json!({"action": "delete", "kind": kind, "index": index}));
                reselect = Some(match selected {
                    Marker::Opacity(_) => Marker::Opacity(index.saturating_sub(1)),
                    _ => Marker::Color(index.saturating_sub(1)),
                });
            }
        });
    }
    if let (Marker::Color(i), true) = (selected, picker_open)
        && let Some((_, c)) = cs.get(i)
    {
        // The stop's colour keeps its own alpha; transparency belongs to the opacity stops.
        let rgba = rgba_of(c);
        let cache = id.with(("stop-hsva", i));
        let mut hsva = ui
            .data(|d| d.get_temp::<([f32; 4], egui::ecolor::Hsva)>(cache))
            .filter(|(previous, _)| *previous == rgba)
            .map_or_else(|| picker_color(rgba), |(_, h)| h);
        if egui::color_picker::color_picker_hsva_2d(ui, &mut hsva, egui::color_picker::Alpha::Opaque) {
            let [r, g, b, _] = picker_rgba(hsva);
            if [r, g, b].iter().zip(rgba).any(|(a, b)| (a - b).abs() > 0.000001) {
                let color = [r, g, b, rgba[3]];
                edit = Some(json!({"action": "color", "index": i, "color": color}));
                ui.data_mut(|d| d.insert_temp(cache, (color, hsva)));
            }
        } else {
            ui.data_mut(|d| d.insert_temp(cache, (rgba, hsva)));
        }
    }
    ui.data_mut(|d| d.insert_temp(picker_key, picker_open));
    let out = apply(fill, &edit?, fg, bg)?;
    if let Some(m) = reselect {
        ui.data_mut(|d| d.insert_temp(selection_key, m));
    }
    Some(out)
}
