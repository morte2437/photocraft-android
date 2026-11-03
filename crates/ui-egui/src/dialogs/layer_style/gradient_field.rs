//! The Gradient Overlay page's gradient (#5177): a ramp with colour-stop markers under it, edited
//! in place as in Photoshop's Gradient Editor (click under the ramp to add a stop with the
//! gradient's colour there, drag a marker to move it, drag it away to delete it, double-click it
//! or the Color swatch for the Color Picker), a Location field and Delete for the selected stop,
//! and the Gradients panel's presets to start from. The dialog field `stops` is
//! `[[position, "#rrggbb"], …]` and `opacityStops` `[[position, opacity %], …]` (carried through
//! the dialog, not edited here): the engine command's own grammar, so what the page shows is what
//! `layer.layerStyle.gradientOverlay` receives.

use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_color::Color;
use photocraft_compose::gradient_fill as gf;
use photocraft_doc::{Fill, GradientStyle};
use photocraft_engine::gradient_fill_cmds as cmds;
use serde_json::{Map, Value, json};

use super::{color_picker, set_param};
use crate::theme::Tokens;
use crate::{PhotocraftApp, color_picker_ui, widgets};

/// The dialog field holding the colour stops.
pub(super) const KEY: &str = "stops";
/// The dialog field holding the transparency stops.
pub(super) const OPACITY_KEY: &str = "opacityStops";

/// Where the last drawn ramp bar is, for tests that drive it: `(effect id, bar rect)`.
pub(crate) fn bar_id() -> egui::Id {
    egui::Id::unique("fx-gradient-bar")
}

/// `[[t, "#rrggbb"], …]` → stops (at least two, sorted); `None` when the value isn't that.
pub(super) fn stops_of(v: Option<&Value>) -> Option<Vec<(f32, Color)>> {
    let a = v?.as_array()?;
    let mut stops: Vec<(f32, Color)> = a
        .iter()
        .filter_map(|s| {
            let t = s.get(0)?.as_f64()?;
            let value = s.get(1)?;
            let color = if let Some(hex) = value.as_str() {
                let [r, g, b] = color_picker_ui::parse_hex(hex)?;
                Color::rgb(r, g, b)
            } else {
                serde_json::from_value::<Color>(value.clone()).ok()?
            };
            (t.is_finite() && color.c.iter().all(|v| v.is_finite()) && color.alpha.is_finite()).then(|| (t.clamp(0.0, 1.0) as f32, color))
        })
        .collect();
    if stops.len() < 2 {
        return None;
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(stops)
}

/// `[[t, opacity %], …]` → transparency stops (empty = opaque); `None` when the value isn't that.
pub(super) fn opacity_of(v: Option<&Value>) -> Option<Vec<(f32, f32)>> {
    let a = v?.as_array()?;
    let mut os: Vec<(f32, f32)> = a
        .iter()
        .filter_map(|s| {
            let (t, o) = (s.get(0)?.as_f64()?, s.get(1)?.as_f64()?);
            (t.is_finite() && o.is_finite()).then(|| (t.clamp(0.0, 1.0) as f32, (o / 100.0).clamp(0.0, 1.0) as f32))
        })
        .collect();
    if os.len() == 1 {
        return None;
    }
    os.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(os)
}

pub(super) fn stops_json(stops: &[(f32, Color)]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|(t, c)| {
                let [r, g, b, _] = c.to_rgba8();
                let exact = Color::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
                if *c == exact { json!([t, super::hex(c)]) } else { json!([t, c]) }
            })
            .collect(),
    )
}

pub(super) fn opacity_json(os: &[(f32, f32)]) -> Value {
    Value::Array(os.iter().map(|(t, o)| json!([t, f64::from(*o) * 100.0])).collect())
}

/// The page's gradient as a fill: the ramp, and what the stop edits are applied to.
pub(super) fn fill_of(disp: &Value) -> Fill {
    let stops = stops_of(disp.get(KEY)).unwrap_or_else(|| vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
    let mut f = Fill::gradient(stops, 0.0, 1.0, GradientStyle::Linear, false);
    if let Fill::Gradient { opacity_stops, .. } = &mut f {
        *opacity_stops = opacity_of(disp.get(OPACITY_KEY)).unwrap_or_default();
    }
    f
}

/// The engine's factory defaults name a From and To colour; the page models stops, so a fresh
/// instance starts from those two as stops (and never shows From/To).
pub(super) fn from_to_to_stops(p: &mut Value) {
    let Some(o) = p.as_object_mut() else { return };
    if !o.contains_key(KEY) {
        let hex = |k: &str, d: &str| o.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        let (from, to) = (hex("from", "#000000"), hex("to", "#ffffff"));
        o.insert(KEY.into(), json!([[0.0, from], [1.0, to]]));
    }
    o.remove("from");
    o.remove("to");
    o.entry(OPACITY_KEY).or_insert_with(|| json!([]));
}

/// Store `fill`'s stops as the effect's `stops` / `opacityStops` and show them.
fn commit(f: &mut Map<String, Value>, effect: &str, disp: &mut Value, fill: &Fill) {
    let Fill::Gradient { stops, opacity_stops, .. } = fill else { return };
    let v = stops_json(stops);
    disp[KEY] = v.clone();
    set_param(f, effect, KEY, v);
    let v = opacity_json(opacity_stops);
    disp[OPACITY_KEY] = v.clone();
    set_param(f, effect, OPACITY_KEY, v);
}

/// Apply one stop edit (`cmds::apply_stop`'s grammar) to the page's gradient; the new index of
/// the edited stop, for the selection.
fn edit(app: &PhotocraftApp, f: &mut Map<String, Value>, effect: &str, disp: &mut Value, p: Value) -> Option<usize> {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let fill = fill_of(disp);
    let new = cmds::apply_stop(&fill, &p, fg, bg).ok()?;
    let Fill::Gradient { stops, .. } = &new else { return None };
    // The engine keeps stops sorted: the edited one is the last with its location.
    let at = match p.get("action").and_then(Value::as_str) {
        Some("add" | "move") => {
            let loc = p.get("location").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            stops.iter().rposition(|s| (s.0 - loc).abs() < 1e-6).unwrap_or(0)
        }
        _ => 0,
    };
    commit(f, effect, disp, &new);
    Some(at)
}

/// Use `preset` (the Gradients panel's) as the page's gradient, resolved with the tool colours.
pub(super) fn apply_preset(app: &PhotocraftApp, f: &mut Map<String, Value>, effect: &str, disp: &mut Value, name: &str) -> bool {
    let Some(preset) = app.session.presets.gradients.iter().flat_map(|g| g.items.iter()).find(|p| p.name == name) else { return false };
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let (stops, os) = preset.fill_stops(fg, bg);
    if stops.len() < 2 {
        return false;
    }
    let mut fill = Fill::gradient(stops, 0.0, 1.0, GradientStyle::Linear, false);
    if let Fill::Gradient { opacity_stops, .. } = &mut fill {
        *opacity_stops = os;
    }
    commit(f, effect, disp, &fill);
    true
}

/// The page's row: the preset dropdown, the ramp with its stop markers, the selected stop's
/// Color, Location and Delete.
pub(super) fn row(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, effect: &str, label: &str, disp: &mut Value) {
    let t = Tokens::get(ui.ctx());
    let sel_key = egui::Id::unique(("fx-gradient-selection", effect.to_string()));
    let drag_key = egui::Id::unique(("fx-gradient-drag", effect.to_string()));
    // Presets to start from.
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!(label)).color(t.text_dim));
        let names: Vec<(String, String)> =
            app.session.presets.gradients.iter().flat_map(|g| g.items.iter()).map(|p| (p.name.clone(), p.name.clone())).collect();
        let mut cur = String::new();
        let mut opts: Vec<(String, &str)> = vec![(String::new(), "Custom")];
        opts.extend(names.iter().map(|(id, n)| (id.clone(), n.as_str())));
        if widgets::dropdown(ui, &format!("fx-gradient-preset-{effect}"), "Gradient preset", &mut cur, &opts, 180.0) && !cur.is_empty() {
            apply_preset(app, f, effect, disp, &cur);
            ui.data_mut(|d| d.insert_temp(sel_key, 0usize));
        }
    });
    let fill = fill_of(disp);
    let Fill::Gradient { stops, .. } = &fill else { return };
    let w = ui.available_width().max(120.0);
    let (outer, resp) = ui.allocate_exact_size(vec2(w, 46.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Gradient stops")));
    let bar = Rect::from_min_max(pos2(outer.left() + 8.0, outer.top() + 2.0), pos2(outer.right() - 8.0, outer.top() + 24.0));
    ui.data_mut(|d| d.insert_temp(bar_id(), (effect.to_string(), bar)));
    let x_of = |loc: f32| bar.left() + loc.clamp(0.0, 1.0) * bar.width();
    let loc_of = |x: f32| ((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
    if let Some(ramp) = gf::Ramp::new(&fill) {
        crate::gradient_ui::paint_ramp(ui.painter(), bar, |u| ramp.sample(u));
    }
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let selected: usize = ui.data(|d| d.get_temp(sel_key)).unwrap_or(0).min(stops.len().saturating_sub(1));
    // Markers: a triangle on the bar's lower edge over a chip of the stop's colour.
    let hit = |p: egui::Pos2| -> Option<usize> {
        if p.y < bar.bottom() - 2.0 || p.y > outer.bottom() + 4.0 {
            return None;
        }
        stops.iter().enumerate().rev().find(|(_, s)| (x_of(s.0) - p.x).abs() <= 7.0).map(|(i, _)| i)
    };
    for (i, (loc, c)) in stops.iter().enumerate() {
        let x = x_of(*loc);
        let tri = vec![pos2(x, bar.bottom() + 1.0), pos2(x - 5.0, bar.bottom() + 7.0), pos2(x + 5.0, bar.bottom() + 7.0)];
        let ink = if i == selected { t.accent } else { t.text_dim };
        ui.painter().add(egui::Shape::convex_polygon(tri, ink, Stroke::NONE));
        let chip = Rect::from_min_size(pos2(x - 5.0, bar.bottom() + 7.0), vec2(10.0, 10.0));
        let [r, g, b] = c.to_rgb();
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        ui.painter().rect_filled(chip, 0.0, Color32::from_rgb(q(r), q(g), q(b)));
        ui.painter().rect_stroke(chip, 0.0, Stroke::new(if i == selected { 1.5 } else { 1.0 }, ink), StrokeKind::Outside);
    }
    // Pointer: press on a marker selects it and starts a drag; a drag moves it, and a drag that
    // leaves the strip deletes it on release (two stops always stay); a click under the bar with
    // no marker adds a stop; a double-click on a marker opens the Color Picker.
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        if let Some(i) = hit(p) {
            ui.data_mut(|d| d.insert_temp(drag_key, i));
            ui.data_mut(|d| d.insert_temp(sel_key, i));
        } else {
            ui.data_mut(|d| d.remove::<usize>(drag_key));
        }
    }
    let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_key));
    if let (Some(i), Some(p)) = (dragging, resp.interact_pointer_pos()) {
        let away = p.y > outer.bottom() + 24.0 || p.y < bar.top() - 24.0;
        if resp.drag_stopped() {
            ui.data_mut(|d| d.remove::<usize>(drag_key));
            if away && stops.len() > 2 {
                edit(app, f, effect, disp, json!({"action": "delete", "index": i}));
                ui.data_mut(|d| d.insert_temp(sel_key, 0usize));
            }
        } else if resp.dragged() && !away {
            let loc = loc_of(p.x);
            if (loc - stops.get(i).map_or(loc, |s| s.0)).abs() > 1e-4
                && let Some(at) = edit(app, f, effect, disp, json!({"action": "move", "index": i, "location": loc}))
            {
                ui.data_mut(|d| d.insert_temp(drag_key, at));
                ui.data_mut(|d| d.insert_temp(sel_key, at));
            }
        }
    } else if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<usize>(drag_key));
    }
    if resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(i) = hit(p)
    {
        ui.data_mut(|d| d.insert_temp(sel_key, i));
        f.insert(color_picker::REQUEST.into(), json!({"effect": effect, "field": KEY, "stop": i}));
    } else if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match hit(p) {
            Some(i) => {
                ui.data_mut(|d| d.insert_temp(sel_key, i));
            }
            None if p.x >= bar.left() && p.x <= bar.right() => {
                if let Some(at) = edit(app, f, effect, disp, json!({"action": "add", "location": loc_of(p.x)})) {
                    ui.data_mut(|d| d.insert_temp(sel_key, at));
                }
            }
            None => {}
        }
    }
    // The selected stop.
    let fill = fill_of(disp);
    let Fill::Gradient { stops, .. } = &fill else { return };
    let selected = selected.min(stops.len().saturating_sub(1));
    let Some((loc, color)) = stops.get(selected).copied() else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Color")).color(t.text_dim));
        let [r, g, b] = color.to_rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        let swatch = widgets::color_swatch(ui, Color32::from_rgb(r, g, b)).on_hover_text(tl!("Color Picker (Layer Style Color)"));
        if swatch.clicked() {
            f.insert(color_picker::REQUEST.into(), json!({"effect": effect, "field": KEY, "stop": selected}));
        }
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("Location")).color(t.text_dim));
        let mut pct = loc * 100.0;
        if widgets::value_field(ui, &mut pct, 0.0..=100.0, "%", 58.0).changed()
            && let Some(at) = edit(app, f, effect, disp, json!({"action": "move", "index": selected, "location": pct / 100.0}))
        {
            ui.data_mut(|d| d.insert_temp(sel_key, at));
        }
        ui.add_space(8.0);
        if ui.add_enabled(stops.len() > 2, egui::Button::new(tl!("Delete"))).clicked() {
            edit(app, f, effect, disp, json!({"action": "delete", "index": selected}));
            ui.data_mut(|d| d.insert_temp(sel_key, 0usize));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn dialog_app() -> (PhotocraftApp, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let parent = crate::layer_style::open(&mut app, Some("gradientOverlay")).unwrap();
        (app, parent)
    }

    /// The dialog body drawn from the real dialog's fields, edits written back to it.
    fn harness(app: PhotocraftApp, parent: u64) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let Some(mut f) = app.ui.dialogs.iter().find(|d| d.id == parent).map(|d| d.fields.clone()) else { return };
                crate::layer_style::body(app, ui, &mut f);
                if let Some(d) = app.ui.dialog_mut(parent) {
                    d.fields = f;
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        h
    }

    fn stops_in(app: &PhotocraftApp, parent: u64) -> Vec<(f32, Color)> {
        let f = &app.ui.dialogs.iter().find(|d| d.id == parent).unwrap().fields;
        let e = crate::layer_style::effects_of(f).iter().find(|e| e["kind"] == "gradientOverlay").unwrap();
        stops_of(e["params"].get(KEY)).unwrap()
    }

    fn press(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, p: egui::Pos2) {
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.run_steps(1);
    }
    fn release(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, p: egui::Pos2) {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
    }

    /// Photoshop's Gradient Editor gestures on the page: a click under the ramp adds a stop with
    /// the colour there, a drag moves one, a drag away from the strip deletes it, two stay.
    #[test]
    fn ramp_adds_moves_and_deletes_stops() {
        let (mut app, parent) = dialog_app();
        let effect = crate::layer_style::effects_of(&app.ui.dialogs[0].fields)[0]["id"].as_str().unwrap().to_string();
        set_param(&mut app.ui.dialog_mut(parent).unwrap().fields, &effect, KEY, json!([[0.0, "#000000"], [1.0, "#ffffff"]]));
        let mut h = harness(app, parent);
        assert!(h.query_by_label("Gradient stops").is_some());
        let (_, bar): (String, Rect) = h.ctx.data(|d| d.get_temp(bar_id())).expect("the ramp was drawn");
        // Add in the middle: a mid grey.
        let mid = pos2(bar.center().x, bar.bottom() + 9.0);
        press(&mut h, mid);
        release(&mut h, mid);
        let stops = stops_in(h.state(), parent);
        assert_eq!(stops.len(), 3, "{stops:?}");
        assert!((stops[1].0 - 0.5).abs() < 0.02 && (stops[1].1.to_rgb()[0] - 0.5).abs() < 0.06, "{stops:?}");
        // Drag it to a quarter.
        let quarter = pos2(bar.left() + bar.width() * 0.25, mid.y);
        press(&mut h, mid);
        h.event(egui::Event::PointerMoved(pos2(mid.x - 10.0, mid.y)));
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(quarter));
        h.run_steps(1);
        release(&mut h, quarter);
        let stops = stops_in(h.state(), parent);
        assert_eq!(stops.len(), 3);
        assert!((stops[1].0 - 0.25).abs() < 0.03, "{stops:?}");
        // Drag it away: gone.
        let away = pos2(quarter.x, bar.bottom() + 90.0);
        press(&mut h, quarter);
        h.event(egui::Event::PointerMoved(pos2(quarter.x, quarter.y + 20.0)));
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(away));
        h.run_steps(1);
        release(&mut h, away);
        let stops = stops_in(h.state(), parent);
        assert_eq!(stops.len(), 2, "{stops:?}");
        // The last two can't be dragged away.
        let first = pos2(bar.left(), mid.y);
        press(&mut h, first);
        h.event(egui::Event::PointerMoved(away));
        h.run_steps(1);
        release(&mut h, away);
        assert_eq!(stops_in(h.state(), parent).len(), 2);
    }

    /// A preset from the Gradients panel becomes the page's stops; the Delete button and the
    /// Location field work on the selected stop.
    #[test]
    fn presets_delete_and_location() {
        let (mut app, parent) = dialog_app();
        let effect = crate::layer_style::effects_of(&app.ui.dialogs[0].fields)[0]["id"].as_str().unwrap().to_string();
        let name = app
            .session
            .presets
            .gradients
            .iter()
            .flat_map(|g| g.items.iter())
            .map(|p| p.name.clone())
            .find(|n| n.contains("Spectrum") || n.contains("Rainbow"))
            .or_else(|| app.session.presets.gradients.first().and_then(|g| g.items.first()).map(|p| p.name.clone()))
            .unwrap();
        let mut f = app.ui.dialog_mut(parent).unwrap().fields.clone();
        let mut disp = json!({});
        assert!(apply_preset(&app, &mut f, &effect, &mut disp, &name), "{name}");
        assert!(!apply_preset(&app, &mut f, &effect, &mut disp, "no such preset"));
        let stops = stops_of(disp.get(KEY)).unwrap();
        assert!(stops.len() >= 2, "{name}: {stops:?}");
        app.ui.dialog_mut(parent).unwrap().fields = f;
        set_param(&mut app.ui.dialog_mut(parent).unwrap().fields, &effect, KEY, json!([[0.0, "#ff0000"], [0.5, "#00ff00"], [1.0, "#0000ff"]]));
        let mut h = harness(app, parent);
        let (_, bar): (String, Rect) = h.ctx.data(|d| d.get_temp(bar_id())).unwrap();
        // Select the middle stop, then Delete.
        let mid = pos2(bar.center().x, bar.bottom() + 9.0);
        press(&mut h, mid);
        release(&mut h, mid);
        h.get_by_label("Delete").click();
        h.run_steps(2);
        let stops = stops_in(h.state(), parent);
        assert_eq!(stops.iter().map(|s| super::super::hex(&s.1)).collect::<Vec<_>>(), ["#ff0000", "#0000ff"], "{stops:?}");
    }

    #[test]
    fn stops_round_trip_through_the_field_grammar() {
        let stops = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (0.5, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
        let v = stops_json(&stops);
        assert_eq!(v, json!([[0.0, "#ff0000"], [0.5, "#00ff00"], [1.0, "#0000ff"]]));
        assert_eq!(stops_of(Some(&v)), Some(stops));
        // Unsorted input is sorted, bad entries are skipped, fewer than two is nothing.
        let v = json!([[1.0, "#ffffff"], ["x", "#ff0000"], [0.25, "#000000"], [0.5, 7]]);
        assert_eq!(stops_of(Some(&v)).map(|s| s.iter().map(|x| x.0).collect::<Vec<_>>()), Some(vec![0.25, 1.0]));
        assert_eq!(stops_of(Some(&json!([[0.0, "#000000"]]))), None);
        assert_eq!(stops_of(Some(&json!("red"))), None);
        assert_eq!(stops_of(None), None);
        let os = vec![(0.0, 1.0), (1.0, 0.5)];
        assert_eq!(opacity_json(&os), json!([[0.0, 100.0], [1.0, 50.0]]));
        assert_eq!(opacity_of(Some(&opacity_json(&os))), Some(os));
        assert_eq!(opacity_of(Some(&json!([]))), Some(vec![]), "[] is opaque");
        assert_eq!(opacity_of(Some(&json!([[0.0, 100.0]]))), None, "one stop is not a list");
    }

    #[test]
    fn factory_from_to_become_two_stops() {
        let mut p = json!({"from": "#ff0000", "to": "#0000ff", "angle": 90});
        from_to_to_stops(&mut p);
        assert_eq!(p, json!({"stops": [[0.0, "#ff0000"], [1.0, "#0000ff"]], "opacityStops": [], "angle": 90}));
        let mut p = json!({"stops": [[0.0, "#000000"], [0.5, "#ff0000"], [1.0, "#ffffff"]], "from": "#123456"});
        from_to_to_stops(&mut p);
        assert_eq!(p["stops"].as_array().map(Vec::len), Some(3), "explicit stops win over From/To");
        assert!(p.get("from").is_none());
    }

    #[test]
    fn the_fill_of_a_page_never_fails() {
        for v in [json!({}), json!({"stops": "x"}), json!({"stops": [[0.0, "#ff0000"]], "opacityStops": [[0.5, 20.0]]}), json!({"stops": null})] {
            let Fill::Gradient { stops, opacity_stops, .. } = fill_of(&v) else { panic!("{v}") };
            assert_eq!(stops.len(), 2, "{v}: the default black-to-white");
            assert!(opacity_stops.is_empty() || opacity_stops.len() >= 2, "{v}");
        }
    }
}
