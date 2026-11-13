//! Image › Analysis and Notes in the shell: the Ruler, Count, Note and Color Sampler tools
//! (eyedropper group), their canvas overlays and options bars, Window › Measurement Log and
//! Window › Notes, and the Set Measurement Scale / Select Data Points / Place Scale Marker dialogs.
//!
//! Every change goes through engine commands (`image.analysis.*`, `count.*`, `notes.*`,
//! `view.colorSamplers.*`, `measurementLog.*`); this module only holds view state ([`AnalysisUi`],
//! part of the serialisable UI state so the control channel can read and drive it). The Color
//! Sampler points themselves live in the engine (`DocState.color_samplers`) so every front end
//! shares them; this module only maps pointer input and draws the numbered markers.
//!
//! The Ruler options bar's Straighten Layer button is on when the engine's check says so, and a
//! hidden target layer answers its click with the reference's alert rather than the status line
//! (#12033).

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, RichText, Stroke, pos2, vec2};
use photocraft_doc::{Document, Ruler};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::{DialogKind, Tool};
use crate::theme::Tokens;

#[cfg(test)]
#[path = "count_readout_tests.rs"]
mod count_readout_tests;

/// The Count tool options bar's number (#11764): with one group (or none), its markers; with two
/// or more, the markers of the visible groups, then the current group's in brackets (0 while it
/// is hidden), as "5 (2)". The single-group case counts a hidden group too (not measured, so it
/// keeps the old total). An out-of-range current index means the last group, as in the Group
/// dropdown and `count.add`.
pub(crate) fn count_readout(m: &photocraft_doc::analysis::Measurement) -> String {
    if m.count_groups.len() < 2 {
        return m.count_total().to_string();
    }
    let visible: usize = m.count_groups.iter().filter(|g| g.visible).map(|g| g.points.len()).sum();
    let gi = m.active_count_group.min(m.count_groups.len().saturating_sub(1));
    let current = m.count_groups.get(gi).filter(|g| g.visible).map_or(0, |g| g.points.len());
    format!("{visible} ({current})")
}

/// What a pointer drag is doing (not serialised).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// New ruler line from a point; the ruler it replaces comes back if the press doesn't move (#11980).
    RulerNew(Option<Ruler>),
    /// Moving a ruler handle: 0 start, 1 end, 2 protractor end.
    RulerHandle(u8),
    /// Moving count marker (group, index) from where it was.
    Count(usize, usize, [f64; 2]),
    /// Moving note `index`: grab offset from its position.
    Note(usize, [f64; 2]),
    /// Moving Color Sampler point `index`; the second field is where the pointer is now, where its
    /// marker is drawn (#11729). The release moves the point there (a click on it too); releasing
    /// off the canvas removes it.
    Sampler(usize, [f64; 2]),
    /// A new Color Sampler point following the pointer, placed where the drag ends if that is on
    /// the canvas (#11729).
    SamplerNew([f64; 2]),
}

/// View state of the analysis tools and panels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisUi {
    /// Window › Measurement Log / Notes panels.
    pub measurement_log: bool,
    pub notes: bool,
    /// Measurement Log rows selected (row ids).
    pub log_selected: Vec<u64>,
    /// Notes panel: the note shown (index).
    pub note_selected: Option<usize>,
    /// Note tool options: author and colour of new notes.
    pub note_author: String,
    pub note_color: [f32; 3],
    /// Ruler options bar: show values in the measurement scale's units.
    pub use_measurement_scale: bool,
    /// Open dialog: (kind, fields). Kinds: "scale", "dataPoints", "scaleMarker".
    pub dialog: Option<(String, Map<String, Value>)>,
    #[serde(skip)]
    pub drag: Option<Drag>,
}

impl Default for AnalysisUi {
    fn default() -> Self {
        Self {
            measurement_log: false,
            notes: false,
            log_selected: Vec::new(),
            note_selected: None,
            note_author: String::new(),
            note_color: photocraft_engine::notes_cmds::DEFAULT_NOTE_COLOR,
            use_measurement_scale: false,
            dialog: None,
            drag: None,
        }
    }
}

const PANELS: [&str; 2] = ["window.panel.measurementLog", "window.panel.notes"];

/// Menu ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    PANELS.contains(&id)
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "window.panel.measurementLog" => Some(app.ui.analysis.measurement_log),
        "window.panel.notes" => Some(app.ui.analysis.notes),
        "image.analysis.rulerTool" => Some(app.ui.tool == Tool::Ruler),
        "image.analysis.countTool" => Some(app.ui.tool == Tool::Count),
        _ => {
            // View › Proof Setup simulations: checked while that proof is shown.
            // A check item with no document too: an item's kind never changes (native menus).
            let kind = id.strip_prefix("view.proofSetup.").and_then(photocraft_engine::proof_sim::ProofKind::from_id)?;
            let Some(d) = app.session.active() else { return Some(false) };
            let pv = app.session.color.proof(d.doc.id);
            Some(pv.enabled && pv.setup.kind == kind)
        }
    }
}

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

fn fields(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// Menu front ends: panel toggles, tool selection, dialogs and pickers in front of the engine's
/// analysis commands. `None` when `id` isn't ours (or the engine should run it directly).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let a = &mut app.ui.analysis;
    match id {
        "window.panel.measurementLog" | "window.panel.notes" => {
            let slot = if id.ends_with("measurementLog") { &mut a.measurement_log } else { &mut a.notes };
            *slot = params.get("show").and_then(Value::as_bool).unwrap_or(!*slot);
            return Some(Ok(json!({"visible": *slot})));
        }
        _ => {}
    }
    if !no_params(params) {
        return None;
    }
    let doc = app.session.active().map(|d| d.doc.clone());
    Some(match id {
        "image.analysis.rulerTool" => {
            app.ui.tool = Tool::Ruler;
            Ok(json!({"tool": "Ruler"}))
        }
        "image.analysis.countTool" => {
            app.ui.tool = Tool::Count;
            Ok(json!({"tool": "Count"}))
        }
        "image.analysis.setMeasurementScale" => {
            let d = doc?;
            let sc = &d.measurement.scale;
            open(
                app,
                "scale",
                json!({"preset": if sc.is_default() { "default" } else { "custom" }, "pixelLength": sc.pixel_length, "logicalLength": sc.logical_length, "units": sc.units}),
            )
        }
        "image.analysis.selectDataPoints" => {
            let dp = serde_json::to_value(&app.session.analysis.data_points).unwrap_or_default();
            open(app, "dataPoints", dp)
        }
        "image.analysis.placeScaleMarker" => {
            let d = doc?;
            let units = d.measurement.scale.units.clone();
            open(
                app,
                "scaleMarker",
                json!({"length": suggested_length(&d), "units": units, "fontSize": 12.0, "displayText": true, "textPosition": "bottom", "color": "black"}),
            )
        }
        "image.analysis.recordMeasurements" => {
            // The current tool picks the source, as in Photoshop.
            let source = match app.ui.tool {
                Tool::Ruler => "ruler",
                Tool::Count => "count",
                _ => "auto",
            };
            let r = app.run(id, json!({"source": source}));
            if r.is_ok() {
                app.ui.analysis.measurement_log = true;
            }
            r
        }
        "file.import.notes" => match app.active_doc_id() {
            Ok(doc) => app.pick_file_bytes(move |app, name, bytes| {
                app.refocus(doc)?;
                let r = photocraft_engine::notes_cmds::import_notes_from(&mut app.session, &name, &bytes).map_err(|e| e.to_string());
                if r.is_ok() {
                    app.ui.analysis.notes = true;
                    app.sync_views();
                }
                r
            }),
            Err(e) => Err(e),
        },
        "measurementLog.export" => export_log(app, None),
        _ => return None,
    })
}

fn open(app: &mut PhotocraftApp, kind: &str, f: Value) -> Result<Value, String> {
    app.ui.analysis.dialog = Some((kind.to_string(), fields(f)));
    Ok(json!({"dialog": kind}))
}

/// A "nice" bar length near a fifth of the image width (same rule as the engine default).
fn suggested_length(d: &Document) -> f64 {
    let target = f64::from(d.size.width) / 5.0 * d.measurement.scale.factor();
    let e = 10f64.powf(target.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|m| m * e).find(|v| *v <= target).unwrap_or(e)
}

/// Export the Measurement Log (or the given rows) as CSV through the save picker.
fn export_log(app: &mut PhotocraftApp, rows: Option<Vec<u64>>) -> Result<Value, String> {
    let p = rows.map_or(json!({}), |r| json!({"rows": r}));
    let csv = app.run("measurementLog.export", p)?;
    let text = csv["csv"].as_str().unwrap_or_default().to_string();
    app.pick_save("Measurements.csv", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, text.as_bytes())?;
        Ok(json!({"path": path, "rows": csv["rows"]}))
    })
}

// ------------------------------------------------------------------ tools

fn tolerance(app: &PhotocraftApp) -> f64 {
    7.0 / f64::from(app.point_zoom().max(0.01))
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Shift constrains to 45° steps around `from`, as the reference does (#11985): on a diagonal step
/// both legs take the smaller of |dx| and |dy| (signs kept), on a 0° or 90° step the dominant
/// component stays and the other becomes 0, so the ruler never gets longer than the pointer's legs.
fn constrain(from: [f64; 2], to: [f64; 2], shift: bool) -> [f64; 2] {
    if !shift {
        return to;
    }
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let step = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() as i64;
    let (cx, cy) = match step.rem_euclid(4) {
        0 => (dx, 0.0),
        2 => (0.0, dy),
        _ => {
            let leg = dx.abs().min(dy.abs());
            (leg.copysign(dx), leg.copysign(dy))
        }
    };
    [from[0] + cx, from[1] + cy]
}

fn set_ruler(app: &mut PhotocraftApp, r: Ruler) {
    let _ = app.run("image.analysis.rulerTool", json!({"start": r.start, "end": r.end, "protractor": r.protractor}));
}

#[cfg(test)]
#[path = "analysis_ruler_click_tests.rs"]
mod analysis_ruler_click_tests;

#[cfg(test)]
#[path = "note_show_on_new_tests.rs"]
mod note_show_on_new_tests;

/// Create a note at the rounded document position using the Note tool's options.
/// After `notes.add` succeeds, select the note and open the Notes panel. Enable Show Notes
/// while leaving the master Extras switch alone.
pub(crate) fn create_note(app: &mut PhotocraftApp, x: f64, y: f64) -> Result<Value, String> {
    let a = &app.ui.analysis;
    let params = json!({"x": x.round(), "y": y.round(), "author": a.note_author, "color": a.note_color});
    let result = app.run("notes.add", params)?;
    app.ui.analysis.note_selected = result["index"].as_u64().map(|v| v as usize);
    app.ui.analysis.notes = true;
    app.ui.view.show.notes = true;
    Ok(result)
}

/// Pointer input for the Ruler, Count, Note and Color Sampler tools. Returns true when consumed.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    let tool = app.active_tool();
    if !matches!(tool, Tool::Ruler | Tool::Count | Tool::Note | Tool::ColorSampler) {
        return false;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        // With no document the Color Sampler does nothing and says so, as the other tools are inert.
        if tool == Tool::ColorSampler && matches!(ev, ToolEvent::Down { .. }) {
            app.ui.status = tl!("No document open").into();
            app.ui.status_error = false;
        }
        return true;
    };
    let tol = tolerance(app);
    match (tool, ev) {
        (Tool::Ruler, ToolEvent::Down { x, y, .. }) => {
            let p = [x, y];
            let cur = doc.measurement.ruler;
            let near = |q: [f64; 2]| dist(p, q) <= tol;
            match cur {
                // ⌥-drag from an end point starts the protractor arm at that vertex.
                Some(mut r) if mods.alt && (near(r.start) || near(r.end)) => {
                    if near(r.end) && !near(r.start) {
                        std::mem::swap(&mut r.start, &mut r.end);
                    }
                    r.protractor = Some(p);
                    set_ruler(app, r);
                    app.ui.analysis.drag = Some(Drag::RulerHandle(2));
                }
                Some(r) if r.protractor.is_some_and(near) => app.ui.analysis.drag = Some(Drag::RulerHandle(2)),
                Some(r) if near(r.end) => app.ui.analysis.drag = Some(Drag::RulerHandle(1)),
                Some(r) if near(r.start) => app.ui.analysis.drag = Some(Drag::RulerHandle(0)),
                _ => {
                    set_ruler(app, Ruler { start: p, end: p, protractor: None });
                    app.ui.analysis.drag = Some(Drag::RulerNew(cur));
                }
            }
        }
        (Tool::Ruler, ToolEvent::Move { x, y, .. }) => {
            let (Some(drag), Some(mut r)) = (app.ui.analysis.drag, doc.measurement.ruler) else { return true };
            match drag {
                Drag::RulerNew(_) | Drag::RulerHandle(1) => r.end = constrain(r.start, [x, y], mods.shift),
                Drag::RulerHandle(0) => r.start = constrain(r.end, [x, y], mods.shift),
                Drag::RulerHandle(_) => r.protractor = Some(constrain(r.start, [x, y], mods.shift)),
                _ => return true,
            }
            set_ruler(app, r);
        }
        (Tool::Ruler, ToolEvent::Up { .. }) => {
            let drag = app.ui.analysis.drag.take();
            if let Some(Drag::RulerNew(before)) = drag
                && doc.measurement.ruler.is_some_and(|r| dist(r.start, r.end) < 0.5)
            {
                // A click leaves the ruler that was there; with none, it leaves none.
                match before {
                    Some(r) => set_ruler(app, r),
                    None => {
                        let _ = app.run("image.analysis.rulerTool", json!({"clear": true}));
                    }
                }
            }
        }
        (Tool::Count, ToolEvent::Down { x, y, .. }) => {
            let hit = nearest_marker(&doc, [x, y], tol);
            match hit {
                Some((g, i)) if mods.alt => {
                    let _ = app.run("count.remove", json!({"group": g, "index": i}));
                }
                Some((g, i)) => app.ui.analysis.drag = Some(Drag::Count(g, i, [x, y])),
                None if !mods.alt => {
                    // A marker that stays invisible looks like the click did nothing, so adding one
                    // brings View > Show > Count back. It is view state, not a history step.
                    let added = app.run("count.add", json!({"x": x, "y": y})).is_ok();
                    app.ui.view.show.count |= added;
                }
                None => {}
            }
        }
        (Tool::Count, ToolEvent::Up { x, y }) => {
            if let Some(Drag::Count(g, i, from)) = app.ui.analysis.drag.take()
                && dist(from, [x, y]) > 0.5
            {
                let _ = app.run("count.move", json!({"group": g, "index": i, "to": [x, y]}));
            }
        }
        (Tool::Note, ToolEvent::Down { x, y, .. }) => {
            let z = f64::from(app.point_zoom().max(0.01));
            let hit =
                doc.notes.iter().rposition(|n| x >= n.position[0] && x <= n.position[0] + 16.0 / z && y >= n.position[1] && y <= n.position[1] + 20.0 / z);
            match hit {
                Some(i) => {
                    app.ui.analysis.note_selected = Some(i);
                    app.ui.analysis.notes = true;
                    show_note_options(app, i);
                    app.ui.analysis.drag = Some(Drag::Note(i, [x - doc.notes[i].position[0], y - doc.notes[i].position[1]]));
                }
                None => {
                    let _ = create_note(app, x, y);
                }
            }
        }
        (Tool::Note, ToolEvent::Up { x, y }) => {
            if let Some(Drag::Note(i, off)) = app.ui.analysis.drag.take() {
                let to = [(x - off[0]).round(), (y - off[1]).round()];
                if doc.notes.get(i).is_some_and(|n| dist(n.position, to) >= 1.0) {
                    let _ = app.run("notes.set", json!({"index": i, "x": to[0], "y": to[1]}));
                }
            }
        }
        // As in Photoshop (#11729): the press only starts a drag and the release decides. A new
        // point follows the pointer and lands where the drag ends; a press on a point drags it
        // there (a click on it moves it to the clicked pixel); Escape cancels either drag.
        (Tool::ColorSampler, ToolEvent::Down { x, y, .. }) => {
            let p = [x, y];
            match sampler_at(app, p) {
                // ⌥-click a point removes it.
                Some(i) if mods.alt => {
                    let _ = app.run("view.colorSamplers.delete", json!({"index": i}));
                }
                Some(i) => app.ui.analysis.drag = Some(Drag::Sampler(i, p)),
                // Off a point (⌥ or not, on the pasteboard too) a new point starts following the pointer.
                None => app.ui.analysis.drag = Some(Drag::SamplerNew(p)),
            }
        }
        (Tool::ColorSampler, ToolEvent::Move { x, y, .. }) => {
            if let Some(Drag::Sampler(_, at) | Drag::SamplerNew(at)) = &mut app.ui.analysis.drag {
                *at = [x, y];
            }
        }
        (Tool::ColorSampler, ToolEvent::Up { x, y }) => {
            let inside = (0.0..f64::from(doc.size.width)).contains(&x) && (0.0..f64::from(doc.size.height)).contains(&y);
            let r = match app.ui.analysis.drag.take() {
                // Released inside the canvas: move it there. Outside: remove just that point.
                Some(Drag::Sampler(i, _)) if inside => app.run("view.colorSamplers.move", json!({"index": i, "x": x, "y": y})),
                Some(Drag::Sampler(i, _)) => app.run("view.colorSamplers.delete", json!({"index": i})),
                // A new point is placed where the drag ends; ending outside the canvas places none.
                Some(Drag::SamplerNew(_)) if inside => app.run("view.colorSamplers.add", json!({"x": x, "y": y})),
                _ => return true,
            };
            status_on_err(app, r);
        }
        _ => {}
    }
    true
}

/// The nearest Color Sampler point within `tol` document pixels, or `None`.
fn nearest_sampler(points: &[[f64; 2]], at: [f64; 2], tol: f64) -> Option<usize> {
    points.iter().enumerate().filter(|(_, q)| dist(**q, at) <= tol).min_by(|a, b| dist(*a.1, at).total_cmp(&dist(*b.1, at))).map(|(i, _)| i)
}

/// The Color Sampler point under document point `at` on the active document, by the same hit test
/// a click uses, so the right-click menu acts on the sampler a click would grab.
pub(crate) fn sampler_at(app: &PhotocraftApp, at: [f64; 2]) -> Option<usize> {
    if !at.iter().all(|v| v.is_finite()) {
        return None;
    }
    nearest_sampler(&app.session.active()?.color_samplers, at, tolerance(app))
}

#[cfg(test)]
mod sampler_menu_tests;

/// Show a command error in the status bar without treating the sample-limit message as a fault.
fn status_on_err(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = false;
    }
}

/// The count marker a Count tool click at document point `at` would grab (the click's own hit
/// test, with its tolerance), or `None`. The cursor uses it too, so it never says "1" where a click
/// adds a marker (#11773).
pub(crate) fn count_marker_at(app: &PhotocraftApp, at: [f64; 2]) -> Option<(usize, usize)> {
    nearest_marker(&app.session.active()?.doc, at, tolerance(app))
}

fn nearest_marker(d: &Document, at: [f64; 2], tol: f64) -> Option<(usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (gi, g) in d.measurement.count_groups.iter().enumerate().filter(|(_, g)| g.visible) {
        for (pi, q) in g.points.iter().enumerate() {
            let dd = dist(*q, at);
            if dd <= tol && best.is_none_or(|b| dd < b.0) {
                best = Some((dd, gi, pi));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

fn color32(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8)
}

fn handle(painter: &egui::Painter, p: Pos2) {
    for (w, c) in [(3.0, Color32::BLACK), (1.0, Color32::WHITE)] {
        painter.line_segment([p - vec2(5.0, 0.0), p + vec2(5.0, 0.0)], Stroke::new(w, c));
        painter.line_segment([p - vec2(0.0, 5.0), p + vec2(0.0, 5.0)], Stroke::new(w, c));
    }
}

/// Ruler line (while the Ruler tool is active), count markers and note icons.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let v = &app.ui.view;
    if app.ui.tool == Tool::Ruler
        && let Some(r) = doc.measurement.ruler
    {
        let s = xf.to_screen(r.start[0] as f32, r.start[1] as f32);
        let e = xf.to_screen(r.end[0] as f32, r.end[1] as f32);
        let mut segs = vec![[s, e]];
        if let Some(q) = r.protractor {
            segs.push([s, xf.to_screen(q[0] as f32, q[1] as f32)]);
        }
        for seg in &segs {
            painter.line_segment(*seg, Stroke::new(3.0, Color32::from_black_alpha(160)));
            painter.line_segment(*seg, Stroke::new(1.0, Color32::WHITE));
            handle(painter, seg[1]);
        }
        handle(painter, s);
    }
    if v.shows(v.show.count) {
        for g in doc.measurement.count_groups.iter().filter(|g| g.visible) {
            let c = color32(g.color.to_rgb());
            for (i, p) in g.points.iter().enumerate() {
                let at = xf.to_screen(p[0] as f32, p[1] as f32);
                painter.circle_filled(at, 1.0 + g.marker_size as f32, c);
                let font = FontId::proportional(g.label_size as f32);
                painter.text(at + vec2(2.0 + g.marker_size as f32, -1.0), Align2::LEFT_BOTTOM, format!("{}", i + 1), font, c);
            }
        }
    }
    if v.shows(v.show.notes) {
        for (i, n) in doc.notes.iter().enumerate() {
            let p = xf.to_screen(n.position[0] as f32, n.position[1] as f32);
            let r = egui::Rect::from_min_size(p, vec2(16.0, 20.0));
            let fill = color32(n.color.to_rgb());
            painter.rect_filled(r, CornerRadius::same(2), fill);
            let sel = app.ui.analysis.note_selected == Some(i);
            painter.rect_stroke(
                r,
                CornerRadius::same(2),
                Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { Color32::from_black_alpha(200) }),
                egui::StrokeKind::Outside,
            );
            // Folded corner and text lines.
            let ink = Color32::from_black_alpha(140);
            painter.line_segment([r.right_top() + vec2(-5.0, 0.0), r.right_top() + vec2(0.0, 5.0)], Stroke::new(1.0, ink));
            for k in 0..3 {
                let y = r.top() + 7.0 + 4.0 * k as f32;
                painter.line_segment([pos2(r.left() + 3.0, y), pos2(r.right() - 3.0, y)], Stroke::new(1.0, ink));
            }
        }
    }
    // Color Sampler points (#1046): persistent numbered markers, drawn whatever the active tool is.
    // While one is dragged its marker follows the pointer, and a new one shows the next number
    // there (#11729), unless the limit is reached and the release cannot place it. View › Extras
    // off hides the markers but keeps the points and their Info readouts (#11741).
    if !v.extras {
        return;
    }
    let mut samplers = app.session.active().map(|d| d.color_samplers.clone()).unwrap_or_default();
    match app.ui.analysis.drag {
        Some(Drag::Sampler(i, at)) if app.ui.tool == Tool::ColorSampler && i < samplers.len() => samplers[i] = at,
        Some(Drag::SamplerNew(at)) if app.ui.tool == Tool::ColorSampler && samplers.len() < photocraft_engine::sampler_cmds::MAX_SAMPLERS => samplers.push(at),
        _ => {}
    }
    for (i, p) in samplers.iter().enumerate() {
        let at = xf.to_screen(p[0] as f32, p[1] as f32);
        // A small target reticle, double-stroked so it reads on any pixel.
        for (w, c) in [(3.0, Color32::from_black_alpha(170)), (1.0, Color32::WHITE)] {
            painter.circle_stroke(at, 5.0, Stroke::new(w, c));
            painter.line_segment([at - vec2(8.0, 0.0), at - vec2(6.0, 0.0)], Stroke::new(w, c));
            painter.line_segment([at + vec2(6.0, 0.0), at + vec2(8.0, 0.0)], Stroke::new(w, c));
            painter.line_segment([at - vec2(0.0, 8.0), at - vec2(0.0, 6.0)], Stroke::new(w, c));
            painter.line_segment([at + vec2(0.0, 6.0), at + vec2(0.0, 8.0)], Stroke::new(w, c));
        }
        // The number, upper right so several points stay readable when they overlap.
        let font = FontId::proportional(11.0);
        let anchor = at + vec2(7.0, -7.0);
        painter.text(anchor + vec2(1.0, 1.0), Align2::LEFT_BOTTOM, format!("{}", i + 1), font.clone(), Color32::from_black_alpha(180));
        painter.text(anchor, Align2::LEFT_BOTTOM, format!("{}", i + 1), font, Color32::WHITE);
    }
}

// ------------------------------------------------------------------ options bars

fn readout(ui: &mut egui::Ui, label: &str, value: Option<f64>) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!(&label)).color(t.text_dim).size(11.0));
    ui.label(RichText::new(value.map_or(String::new(), |v| format!("{v:.2}"))).color(t.text).size(11.0).monospace());
    ui.add_space(6.0);
}

/// Options bar of the Ruler, Count, Note and Color Sampler tools. Returns true when drawn.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        return matches!(tool, Tool::Ruler | Tool::Count | Tool::Note | Tool::ColorSampler);
    };
    match tool {
        Tool::Ruler => {
            let sc = doc.measurement.scale.clone();
            let f = if app.ui.analysis.use_measurement_scale { sc.factor() } else { 1.0 };
            let info = doc.measurement.ruler.map(|r| photocraft_engine::analysis_cmds::ruler_info(&r, &sc));
            let g = |k: &str| info.as_ref().and_then(|i| i[k].as_f64());
            readout(ui, "X:", g("x").map(|v| v * f));
            readout(ui, "Y:", g("y").map(|v| v * f));
            readout(ui, "W:", g("w").map(|v| v * f));
            readout(ui, "H:", g("h").map(|v| v * f));
            readout(ui, "A:", g("angle"));
            readout(ui, "L1:", g("l1").map(|v| v * f));
            readout(ui, "L2:", g("l2").map(|v| v * f));
            crate::widgets::vline(ui, 22.0);
            crate::widgets::checkbox(ui, &mut app.ui.analysis.use_measurement_scale, tl!("Use Measurement Scale"));
            let has = doc.measurement.ruler.is_some();
            // The engine decides this button's state, so it goes grey for exactly the targets
            // Straighten Layer is off for: the Background, several selected layers, a group and a
            // layer locked in place (#12033).
            let straighten = app.session.is_enabled("image.analysis.straightenLayer");
            if ui.add_enabled_ui(straighten, |ui| crate::widgets::secondary_button(ui, tl!("Straighten Layer"), 0.0)).inner.clicked() {
                let hidden = photocraft_engine::analysis_cmds::STRAIGHTEN_HIDDEN_ERROR;
                if let Err(why) = app.run("image.analysis.straightenLayer", json!({})) {
                    // A hidden target layer keeps the button on and answers the click with the
                    // reference's alert (its message and an OK button), changing nothing (#12033).
                    if why == hidden {
                        app.ui.open_dialog(DialogKind::Error, json!({"message": tl!(hidden)}).as_object().cloned().unwrap_or_default());
                    }
                    // Every other refusal goes to the status line, as the eraser tools do. What the
                    // command itself answered is reported there by `PhotocraftApp::run`, but a
                    // refusal raised before it runs - the automation gate a control or MCP-driven
                    // click passes - returns early, and a click that says nothing reads as a dead
                    // button (#12148).
                    app.ui.status = why;
                    app.ui.status_error = true;
                }
            }
            if ui.add_enabled_ui(has, |ui| crate::widgets::secondary_button(ui, tl!("Clear"), 0.0)).inner.clicked() {
                let _ = app.run("image.analysis.rulerTool", json!({"clear": true}));
            }
            true
        }
        Tool::Count => {
            let t = Tokens::get(ui.ctx());
            let m = &doc.measurement;
            ui.label(RichText::new(crate::i18n::fmt(tl!("Count: {n}"), &[("n", &count_readout(m))])).color(t.text).size(11.0));
            crate::widgets::vline(ui, 22.0);
            // Every control from the start, in Photoshop's order (#11765): group, eye, folder,
            // trash, Clear, colour, Marker Size, Label Size. With no group yet the swatch and the
            // sizes show what the next new group gets; using them, or the eye, changes nothing
            // until there is a group (not measured).
            let group = (!m.count_groups.is_empty()).then(|| m.active_count_group.min(m.count_groups.len() - 1));
            let next = photocraft_doc::CountGroup::default();
            let g = group.map_or(&next, |gi| &m.count_groups[gi]);
            if let Some(mut gi) = group {
                let opts: Vec<(usize, &str)> = m.count_groups.iter().enumerate().map(|(i, g)| (i, g.name.as_str())).collect();
                if crate::widgets::dropdown(ui, "count-group", tl!("Group"), &mut gi, &opts, 150.0) {
                    let _ = app.run("count.setGroup", json!({"group": gi, "active": true}));
                }
            } else {
                ui.add_enabled_ui(false, |ui| crate::widgets::dropdown(ui, "count-group", tl!("Group"), &mut 0usize, &[(0, tl!("Count Group"))], 150.0));
            }
            let eye = if g.visible { "eye" } else { "eye-off" };
            if crate::icons::button(ui, eye, 22.0, false, tl!("Toggle visibility of count group")).clicked()
                && let Some(gi) = group
            {
                let _ = app.run("count.setGroup", json!({"group": gi, "visible": !g.visible}));
            }
            if crate::icons::button(ui, "folder-plus", 22.0, false, tl!("Create a new count group")).clicked() {
                let _ = app.run("count.newGroup", json!({}));
            }
            // The last group can't be deleted from here (`count.deleteGroup` still can).
            let trash = ui.add_enabled_ui(m.count_groups.len() >= 2, |ui| {
                crate::icons::button(ui, "trash", 22.0, false, tl!("Delete the currently selected count group"))
            });
            if trash.inner.clicked()
                && let Some(gi) = group
            {
                let _ = app.run("count.deleteGroup", json!({"group": gi}));
            }
            // Clear acts on a visible current group with markers (`count.clear` itself is unchanged).
            let clearable = group.is_some() && g.visible && !g.points.is_empty();
            if ui.add_enabled_ui(clearable, |ui| crate::widgets::secondary_button(ui, tl!("Clear"), 0.0)).inner.clicked() {
                let _ = app.run("count.clear", json!({}));
            }
            let rgb = g.color.to_rgb();
            // PhotoCraft's Color Picker (sRGB), as everywhere else (#2144).
            if crate::widgets::color_swatch_button(ui, color32(rgb), tl!("Count group color")).on_hover_text(tl!("Count group color")).clicked()
                && let Some(gi) = group
            {
                crate::color_picker_ui::open_for_command(
                    app,
                    &crate::color_picker_ui::title_for(tl!("Count Color")),
                    rgb,
                    "count.setGroup",
                    json!({"group": gi}),
                );
            }
            crate::widgets::vline(ui, 22.0);
            let (mut ms, mut ls) = (g.marker_size as f32, g.label_size as f32);
            // Scrubby captions with colons, as in Photoshop: the options bar's own caption, which
            // scrubs the field after it when dragged. The catalogs translate the names without
            // the colon (main's captions), so the colon follows the translation.
            let caption = |ui: &mut egui::Ui, text: &str| crate::panels::opt_label(ui, &format!("{text}:"));
            // Each caption names its field (the scrub link), so a screen reader reads what is shown.
            caption(ui, tl!("Marker Size"));
            let a = crate::widgets::value_field_whole(ui, &mut ms, 1.0..=10.0, "", 40.0);
            caption(ui, tl!("Label Size"));
            let b = crate::widgets::value_field_whole(ui, &mut ls, 8.0..=72.0, "", 44.0);
            // One undo step per scrub (of a field or its caption), as the Gradient tool's Opacity.
            let drag = |r: &egui::Response| {
                let key = r.id.with("count-size-drag");
                if r.drag_started() {
                    let n = ui.ctx().cumulative_pass_nr();
                    ui.data_mut(|d| d.insert_temp(key, n));
                }
                if r.changed() && r.dragged() { ui.data(|d| d.get_temp::<u64>(key)).map(|n| format!("count-size:{}:{n}", key.value())) } else { None }
            };
            let coalesce = drag(&a).or(drag(&b));
            if (a.changed() || b.changed()) && group.is_some() {
                let mut p = count_size_params(ms, ls);
                if let Some(k) = coalesce {
                    p["coalesce"] = json!(k);
                }
                let _ = app.run("count.setGroup", p);
            }
            true
        }
        Tool::Note => {
            let t = Tokens::get(ui.ctx());
            let author_label = ui.label(RichText::new(tl!("Author:")).color(t.text_dim).size(11.0));
            ui.add(egui::TextEdit::singleline(&mut app.ui.analysis.note_author).desired_width(120.0)).labelled_by(author_label.id);
            ui.label(RichText::new(tl!("Color:")).color(t.text_dim).size(11.0));
            let rgb = app.ui.analysis.note_color;
            if crate::widgets::color_swatch_button(ui, color32(rgb), tl!("Color")).clicked() {
                crate::color_picker_ui::open_for_target(app, "noteColor", &crate::color_picker_ui::title_for(tl!("Color")), rgb);
            }
            // Clear All asks first (#11791); `notes.delete {"all": true}` itself never does.
            if ui.add_enabled_ui(!doc.notes.is_empty(), |ui| crate::widgets::secondary_button(ui, tl!("Clear All"), 0.0)).inner.clicked() {
                ask_clear_notes(app);
            }
            if crate::icons::button(ui, "message-square", 22.0, app.ui.analysis.notes, tl!("Show or hide the Notes panel")).clicked() {
                app.ui.analysis.notes = !app.ui.analysis.notes;
            }
            true
        }
        Tool::ColorSampler => {
            let t = Tokens::get(ui.ctx());
            // Sample Size, shared with the Eyedropper as in Photoshop (#11732).
            ui.label(RichText::new(tl!("Sample Size:")).color(t.text_dim).size(12.0));
            let sizes = crate::eyedropper_ui::size_options();
            crate::widgets::dropdown(ui, "color-sampler-size", tl!("Size"), &mut app.ui.tool_options.eyedropper_size, &sizes, 130.0);
            crate::widgets::vline(ui, 22.0);
            let n = app.session.active().map_or(0, |d| d.color_samplers.len());
            ui.label(
                RichText::new(crate::i18n::fmt(
                    tl!("{n} of {max} sample points"),
                    &[("n", &n.to_string()), ("max", &photocraft_engine::sampler_cmds::MAX_SAMPLERS.to_string())],
                ))
                .color(t.text)
                .size(11.0),
            );
            crate::widgets::vline(ui, 22.0);
            if ui.add_enabled_ui(n > 0, |ui| crate::widgets::secondary_button(ui, tl!("Clear All"), 0.0)).inner.clicked() {
                let _ = app.run("view.colorSamplers.clear", json!({}));
            }
            crate::widgets::vline(ui, 22.0);
            ui.label(RichText::new(tl!("Click to place · Alt-click or drag off-canvas to remove")).color(t.text_dim).size(11.0));
            true
        }
        _ => false,
    }
}

// ------------------------------------------------------------------ panels and dialogs

/// `count.setGroup` params for the options bar's Marker Size and Label Size fields, sent as JSON
/// integers (the fields edit `f32`s; `8.0` is a float on the wire).
fn count_size_params(marker: f32, label: f32) -> Value {
    let whole = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.round().clamp(lo, hi) as u32 } else { lo as u32 };
    json!({"markerSize": whole(marker, 1.0, 10.0), "labelSize": whole(label, 8.0, 72.0)})
}

fn float_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(8))
}

/// Title row with a close button; returns true when closed.
pub(crate) fn title_row(ui: &mut egui::Ui, title: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!(&title)).color(t.text).size(12.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "x", 20.0, false, tl!("Close")).clicked() {
                close = true;
            }
        });
    });
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
    close
}

/// A floating panel window; `movable` follows Window › Workspace › Lock Workspace.
pub(crate) fn panel_window(app: &PhotocraftApp, ctx: &egui::Context, id: &str, title: &str, offset: egui::Vec2, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    egui::Window::new(title)
        .id(egui::Id::unique(id))
        .title_bar(false)
        .resizable(false)
        .movable(!app.session.prefs().workspace_locked)
        .min_width(width)
        .max_width(width)
        .frame(float_frame(&t))
        .default_pos(pos2((canvas.right() - width - 16.0 + offset.x).max(canvas.left() + 8.0), canvas.top() + offset.y))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            body(ui);
        });
}

/// Draws the open panels and dialogs.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.analysis.measurement_log {
        let mut act: Option<(&str, Value)> = None;
        let mut close = false;
        let rows = app.session.analysis.log.clone();
        let mut sel = app.ui.analysis.log_selected.clone();
        let t = Tokens::get(ctx);
        panel_window(app, ctx, "measurement-log", tl!("Measurement Log"), vec2(-300.0, 470.0), 620.0, |ui| {
            close = title_row(ui, tl!("Measurement Log"));
            ui.horizontal(|ui| {
                if crate::widgets::secondary_button(ui, tl!("Record Measurements"), 0.0).clicked() {
                    act = Some(("record", Value::Null));
                }
                if crate::widgets::secondary_button(ui, tl!("Select All"), 0.0).clicked() {
                    sel = rows.iter().map(|r| r.id).collect();
                }
                if crate::widgets::secondary_button(ui, tl!("Deselect All"), 0.0).clicked() {
                    sel.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::button(ui, "trash", 22.0, false, tl!("Delete selected measurements")).clicked() && !sel.is_empty() {
                        act = Some(("measurementLog.delete", json!({"rows": sel.clone()})));
                    }
                    if crate::icons::button(ui, "file", 22.0, false, tl!("Export selected measurements (CSV)")).clicked() {
                        act = Some(("export", json!(sel.clone())));
                    }
                });
            });
            let cols: Vec<&str> = photocraft_engine::analysis_cmds::log_columns(&rows).into_iter().filter(|c| *c != "histogram").collect();
            egui::ScrollArea::both().max_height(220.0).max_width(620.0).show(ui, |ui| {
                egui::Grid::new("mlog-grid").striped(true).spacing(vec2(10.0, 2.0)).show(ui, |ui| {
                    for c in &cols {
                        let name = photocraft_engine::analysis_cmds::COLUMNS.iter().find(|x| x.0 == *c).map_or(*c, |x| x.1);
                        ui.label(RichText::new(tl!(name)).color(t.text_dim).size(10.5).strong());
                    }
                    ui.end_row();
                    for r in &rows {
                        let on = sel.contains(&r.id);
                        for (k, c) in cols.iter().enumerate() {
                            let v = r.values.get(*c).map(|v| match v {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                other => other.to_string(),
                            });
                            let txt = RichText::new(v.unwrap_or_default()).size(10.5).color(if on { t.accent_text } else { t.text });
                            let resp = if k == 0 { ui.selectable_label(on, txt) } else { ui.label(txt) };
                            if k == 0 && resp.clicked() {
                                if on {
                                    sel.retain(|x| *x != r.id);
                                } else {
                                    sel.push(r.id);
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
        app.ui.analysis.log_selected = sel;
        if close {
            app.ui.analysis.measurement_log = false;
        }
        match act {
            Some(("record", _)) => {
                let _ = crate::menus::invoke(app, ctx, "image.analysis.recordMeasurements", json!({}));
            }
            Some(("export", rows)) => {
                let ids: Vec<u64> = rows.as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                if let Err(e) = export_log(app, (!ids.is_empty()).then_some(ids))
                    && e != "cancelled"
                {
                    app.ui.status = e;
                }
            }
            Some((id, p)) => {
                let result = app.run(id, p);
                if result.is_ok() {
                    app.ui.analysis.log_selected.clear();
                }
            }
            None => {}
        }
    }
    if app.ui.analysis.notes {
        notes_panel(app, ctx);
    }
    if app.ui.analysis.dialog.is_some() {
        dialog(app, ctx);
    }
}

/// Selecting note `i` shows its author and colour in the Note tool's options bar, as Photoshop
/// does (#7940); a new note then takes them unless they are changed.
fn show_note_options(app: &mut PhotocraftApp, i: usize) {
    let Some(note) = app.session.active().and_then(|d| d.doc.notes.get(i)) else { return };
    let (author, color) = (note.author.clone(), note.color.to_rgb());
    app.ui.analysis.note_author = author;
    app.ui.analysis.note_color = color;
}

fn notes_panel(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let notes = app.session.active().map(|d| d.doc.notes.clone()).unwrap_or_default();
    let n = notes.len();
    let mut sel = app.ui.analysis.note_selected.filter(|i| *i < n).or((n > 0).then_some(0));
    let mut close = false;
    let mut edit: Option<Value> = None;
    let mut delete = false;
    let mut navigated = false;
    let t = Tokens::get(ctx);
    panel_window(app, ctx, "notes-panel", tl!("Notes"), vec2(-40.0, 300.0), 260.0, |ui| {
        close = title_row(ui, tl!("Notes"));
        match sel {
            None => {
                ui.label(RichText::new(tl!("Click with the Note tool to add a note.")).color(t.text_dim).size(11.0));
            }
            Some(i) => {
                let note = &notes[i];
                ui.label(RichText::new(if note.author.is_empty() { tl!("No author").to_string() } else { note.author.clone() }).color(t.text_dim).size(11.0));
                let mut text = note.text.clone();
                if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(6).desired_width(f32::INFINITY)).accessible_name(tl!("Notes")).changed() {
                    edit = Some(json!({"index": i, "text": text, "coalesce": format!("note-{i}")}));
                }
            }
        }
        crate::widgets::hairline(ui);
        ui.horizontal(|ui| {
            if ui.add_enabled_ui(n > 1, |ui| crate::icons::button(ui, "chevrons-left", 22.0, false, tl!("Previous note"))).inner.clicked() {
                sel = sel.map(|i| (i + n - 1) % n);
                navigated = true;
            }
            if ui.add_enabled_ui(n > 1, |ui| crate::icons::button(ui, "chevrons-right", 22.0, false, tl!("Next note"))).inner.clicked() {
                sel = sel.map(|i| (i + 1) % n);
                navigated = true;
            }
            let template = "{index} of {total}";
            let position = crate::i18n::fmt(tl!(template), &[("index", &sel.map_or(0, |i| i + 1).to_string()), ("total", &n.to_string())]);
            ui.label(RichText::new(position).color(t.text_dim).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled_ui(sel.is_some(), |ui| crate::icons::button(ui, "trash", 22.0, false, tl!("Delete note"))).inner.clicked() {
                    delete = true;
                }
            });
        });
    });
    app.ui.analysis.note_selected = sel;
    if navigated && let Some(i) = sel {
        show_note_options(app, i);
    }
    if let Some(p) = edit {
        let _ = app.run("notes.set", p);
    }
    if delete && let Some(i) = sel {
        ask_delete_note(app, i);
    }
    if close {
        app.ui.analysis.notes = false;
    }
}

fn num_field(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new(RichText::new(tl!(&label)).color(t.text_dim).size(11.0)));
        let mut v = f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        if crate::widgets::value_field(ui, &mut v, range, suffix, 80.0).accessible_name(tl!(label)).changed() {
            f.insert(key.into(), json!(v));
        }
    });
}

fn choice(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, opts: &[(&str, &str)]) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 18.0), egui::Label::new(RichText::new(tl!(&label)).color(t.text_dim).size(11.0)));
        let mut cur = f.get(key).and_then(Value::as_str).unwrap_or(opts[0].0).to_string();
        let list: Vec<(String, &str)> = opts.iter().map(|(k, l)| (k.to_string(), *l)).collect();
        if crate::widgets::dropdown(ui, &format!("an-{key}"), tl!(label), &mut cur, &list, 120.0) {
            f.insert(key.into(), json!(cur));
        }
    });
}

/// The params the open dialog's OK runs with (engine command, params).
pub fn dialog_command(kind: &str, f: &Map<String, Value>) -> Option<(&'static str, Value)> {
    Some(match kind {
        "scale" => {
            if f.get("preset").and_then(Value::as_str) == Some("default") {
                ("image.analysis.setMeasurementScale", json!({"preset": "default"}))
            } else {
                (
                    "image.analysis.setMeasurementScale",
                    json!({"preset": "custom", "pixelLength": f.get("pixelLength"), "logicalLength": f.get("logicalLength"), "units": f.get("units")}),
                )
            }
        }
        "dataPoints" => ("image.analysis.selectDataPoints", json!({"selection": f.get("selection"), "ruler": f.get("ruler"), "count": f.get("count")})),
        "scaleMarker" => (
            "image.analysis.placeScaleMarker",
            json!({"length": f.get("length"), "fontSize": f.get("fontSize"), "displayText": f.get("displayText"), "textPosition": f.get("textPosition"), "color": f.get("color")}),
        ),
        _ => return None,
    })
}

fn dialog(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((kind, mut f)) = app.ui.analysis.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let title = match kind.as_str() {
        "scale" => "Measurement Scale",
        "dataPoints" => "Select Data Points",
        _ => "Measurement Scale Marker",
    };
    let mut result: Option<bool> = None;
    egui::Window::new(title)
        .id(egui::Id::unique("analysis-dialog"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(float_frame(&t))
        .show(ctx, |ui| {
            ui.label(RichText::new(tl!(&title)).color(t.text).size(13.0).strong());
            crate::widgets::hairline(ui);
            ui.add_space(6.0);
            match kind.as_str() {
                "scale" => {
                    choice(ui, &mut f, "preset", tl!("Presets:"), &[("custom", tl!("Custom")), ("default", tl!("Default"))]);
                    let custom = f.get("preset").and_then(Value::as_str) != Some("default");
                    ui.add_enabled_ui(custom, |ui| {
                        num_field(ui, &mut f, "pixelLength", tl!("Pixel Length:"), 0.001..=1e7, "");
                        num_field(ui, &mut f, "logicalLength", tl!("Logical Length:"), 0.0001..=1e7, "");
                        ui.horizontal(|ui| {
                            let units_label =
                                ui.add_sized(vec2(110.0, 18.0), egui::Label::new(RichText::new(tl!("Logical Units:")).color(t.text_dim).size(11.0)));
                            let mut u = f.get("units").and_then(Value::as_str).unwrap_or("pixels").to_string();
                            if ui.add(egui::TextEdit::singleline(&mut u).desired_width(80.0)).labelled_by(units_label.id).changed() {
                                f.insert("units".into(), json!(u));
                            }
                        });
                    });
                    let (pl, ll) = (f.get("pixelLength").and_then(Value::as_f64).unwrap_or(1.0), f.get("logicalLength").and_then(Value::as_f64).unwrap_or(1.0));
                    let u = f.get("units").and_then(Value::as_str).unwrap_or("pixels");
                    let text = if custom { format!("{pl} pixels = {ll:.4} {u}") } else { "1 pixels = 1.0000 pixels".into() };
                    ui.label(RichText::new(tl!(&text)).color(t.text_dim).size(11.0));
                }
                "dataPoints" => {
                    ui.horizontal_top(|ui| {
                        for (source, label) in [("selection", tl!("Selections")), ("ruler", tl!("Ruler Tool")), ("count", tl!("Count Tool"))] {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(tl!(&label)).color(t.text).size(11.0).strong());
                                let mut list: Vec<String> = f
                                    .get(source)
                                    .and_then(Value::as_array)
                                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                                    .unwrap_or_default();
                                let mut changed = false;
                                for key in photocraft_engine::analysis_cmds::available(source) {
                                    let name = photocraft_engine::analysis_cmds::COLUMNS.iter().find(|c| c.0 == key).map_or(key, |c| c.1);
                                    let mut on = list.iter().any(|x| x == key);
                                    if crate::widgets::checkbox(ui, &mut on, name).changed() {
                                        list.retain(|x| x != key);
                                        if on {
                                            list.push(key.to_string());
                                        }
                                        changed = true;
                                    }
                                }
                                if changed {
                                    f.insert(source.into(), json!(list));
                                }
                            });
                            ui.add_space(12.0);
                        }
                    });
                }
                _ => {
                    let units = f.get("units").and_then(Value::as_str).unwrap_or("pixels").to_string();
                    num_field(ui, &mut f, "length", tl!("Length:"), 0.0001..=1e7, &units);
                    num_field(ui, &mut f, "fontSize", tl!("Font Size:"), 1.0..=1000.0, "pt");
                    let mut show = f.get("displayText").and_then(Value::as_bool).unwrap_or(true);
                    if crate::widgets::checkbox(ui, &mut show, tl!("Display Text")).changed() {
                        f.insert("displayText".into(), json!(show));
                    }
                    choice(ui, &mut f, "textPosition", tl!("Text Position:"), &[("bottom", tl!("Bottom")), ("top", tl!("Top"))]);
                    choice(ui, &mut f, "color", tl!("Color:"), &[("black", tl!("Black")), ("white", tl!("White"))]);
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(role) = crate::widgets::dialog_buttons(
                        ui,
                        &[
                            crate::widgets::DialogButton::new(crate::widgets::ButtonRole::Default, tl!("OK"), 70.0),
                            crate::widgets::DialogButton::new(crate::widgets::ButtonRole::Cancel, tl!("Cancel"), 70.0),
                        ],
                    ) {
                        result = Some(role == crate::widgets::ButtonRole::Default);
                    }
                });
            });
        });
    match result {
        Some(true) => {
            app.ui.analysis.dialog = None;
            if let Some((id, p)) = dialog_command(&kind, &f) {
                let _ = app.run(id, p);
            }
        }
        Some(false) => app.ui.analysis.dialog = None,
        None => app.ui.analysis.dialog = Some((kind, f)),
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod sampler_drag_tests;

#[cfg(test)]
mod sampler_extras_tests;

/// Marks the "Delete this note?" alert (#11788).
const DELETE_NOTE_MARK: &str = "__deleteNote";

/// Marks the "Are you sure you want to delete all notes?" alert (#11791).
const CLEAR_NOTES_MARK: &str = "__clearAllNotes";

/// The Note tool's Clear All: Photoshop's alert, OK / Cancel (#11791). OK deletes every note in one
/// "Delete all Notes" step; Cancel (or Escape) changes nothing.
pub fn ask_clear_notes(app: &mut PhotocraftApp) {
    let Some(doc) = app.session.active().filter(|s| !s.doc.notes.is_empty()).map(|s| s.doc.id.0) else { return };
    if app.ui.dialogs.iter().any(|d| owns_clear_notes(&d.fields)) {
        return;
    }
    let mut f = Map::new();
    f.insert(CLEAR_NOTES_MARK.into(), json!(true));
    f.insert("__command".into(), json!("notes.delete"));
    f.insert("__label".into(), json!(tl!("Clear All")));
    f.insert("document".into(), json!(doc));
    app.ui.open_dialog(crate::state::DialogKind::Command, f);
}

pub fn owns_clear_notes(fields: &Map<String, Value>) -> bool {
    fields.contains_key(CLEAR_NOTES_MARK)
}

pub fn clear_notes_body(ui: &mut egui::Ui) {
    ui.label(tl!("Are you sure you want to delete all notes?"));
}

/// OK: every note goes in one step, nothing stays selected, and the options bar shows the tool
/// defaults again (a selected note had put its author and colour there).
pub fn confirm_clear_notes(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    if app.session.active().map(|s| s.doc.id.0) != fields.get("document").and_then(Value::as_u64) {
        return Err(tl!("The document changed. Please choose Clear All again.").into());
    }
    let result = app.run("notes.delete", json!({"all": true}))?;
    let defaults = AnalysisUi::default();
    let a = &mut app.ui.analysis;
    (a.note_selected, a.note_author, a.note_color) = (None, defaults.note_author, defaults.note_color);
    Ok(result)
}
/// Its "Don't show again", kept with the other dialog preferences.
const DELETE_NOTE_PREF: &str = "notes.delete.confirmation";

/// Delete note `index` of the active document, as the Note tool's ⌫/Delete and the Notes panel's
/// trash do: after the alert "Delete this note?" ([Yes] [No], "Don't show again"), or at once once
/// that is ticked. Nothing is selected afterwards.
pub fn ask_delete_note(app: &mut PhotocraftApp, index: usize) {
    let skip = app.session.prefs().dialogs.get(DELETE_NOTE_PREF).and_then(|v| v.get("skip")).and_then(Value::as_bool) == Some(true);
    let Some(doc) = app.session.active().map(|s| s.doc.id.0) else { return };
    if skip {
        delete_note(app, index);
        return;
    }
    if app.ui.dialogs.iter().any(|d| owns_delete_note(&d.fields)) {
        return;
    }
    let mut f = Map::new();
    f.insert(DELETE_NOTE_MARK.into(), json!(true));
    f.insert("__command".into(), json!("notes.delete"));
    f.insert("__label".into(), json!(tl!("Delete Note")));
    f.insert("document".into(), json!(doc));
    f.insert("index".into(), json!(index));
    f.insert("skip".into(), json!(false));
    app.ui.open_dialog(crate::state::DialogKind::Command, f);
}

fn delete_note(app: &mut PhotocraftApp, index: usize) {
    match app.run("notes.delete", json!({"index": index})) {
        Ok(_) => app.ui.analysis.note_selected = None,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

pub fn owns_delete_note(fields: &Map<String, Value>) -> bool {
    fields.contains_key(DELETE_NOTE_MARK)
}

pub fn delete_note_body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.label(tl!("Delete this note?"));
    ui.add_space(8.0);
    let mut skip = fields.get("skip").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut skip, tl!("Don't show again"));
    fields.insert("skip".into(), json!(skip));
}

/// Yes: the note goes (one "Delete Note" step), and "Don't show again" is remembered.
pub fn confirm_delete_note(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let index = fields.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or(tl!("No note to delete"))?;
    let doc = app.session.active().map(|s| (s.doc.id.0, s.doc.notes.len()));
    if doc.map(|d| d.0) != fields.get("document").and_then(Value::as_u64) || doc.is_none_or(|d| index >= d.1) {
        return Err(tl!("The document or its notes changed. Please delete the note again.").into());
    }
    let result = app.run("notes.delete", json!({"index": index}))?;
    app.ui.analysis.note_selected = None;
    if fields.get("skip").and_then(Value::as_bool) == Some(true) {
        app.session.edit_prefs(|prefs| {
            prefs.dialogs.insert(DELETE_NOTE_PREF.into(), json!({"skip": true}));
        });
        if let Err(error) = crate::prefs_ui::save_preferences(app) {
            app.ui.status = crate::i18n::fmt(tl!("Note deleted, but the preference could not be saved: {error}"), &[("error", &error)]);
            app.ui.status_error = true;
        }
    }
    Ok(result)
}

/// The Note tool with a note selected: ⌫/Delete asks to delete the note (#11788), never the
/// layer (Delete Layer) or the pixels (Edit › Clear). Text fields keep the keys.
pub fn note_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use crate::shortcut_dispatch::Focus;
    // Escape during a Color Sampler drag cancels it: no new point, or the dragged one stays (#11729).
    // A tool picked mid-drag gets the release, so the drag is over: drop it and leave Escape alone.
    if matches!(app.ui.analysis.drag, Some(Drag::Sampler(..) | Drag::SamplerNew(_))) {
        if app.ui.tool != Tool::ColorSampler {
            app.ui.analysis.drag = None;
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            app.ui.analysis.drag = None;
            return true;
        }
    }
    // Escape during a Ruler drag (a new ruler or an end) ends it and keeps the ruler where it is:
    // no cancel, and the later moves and the release change nothing (#11986).
    if app.ui.tool == Tool::Ruler
        && matches!(app.ui.analysis.drag, Some(Drag::RulerNew(_) | Drag::RulerHandle(0 | 1)))
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        app.ui.analysis.drag = None;
        return true;
    }
    let Some(index) = app.ui.analysis.note_selected else { return false };
    if app.ui.tool != Tool::Note || app.ui.text_edit.is_some() || matches!(Focus::of(ctx), Focus::Text | Focus::DeletingWidget) {
        return false;
    }
    if app.session.active().is_none_or(|s| index >= s.doc.notes.len()) {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if !mods.is_none() || !ctx.input_mut(|i| i.consume_key(mods, egui::Key::Delete) || i.consume_key(mods, egui::Key::Backspace)) {
        return false;
    }
    ask_delete_note(app, index);
    true
}

#[cfg(test)]
#[path = "note_delete_key_tests.rs"]
mod note_delete_key_tests;

#[cfg(test)]
#[path = "analysis_ruler_shift_tests.rs"]
mod analysis_ruler_shift_tests;

#[cfg(test)]
#[path = "note_clear_all_tests.rs"]
mod note_clear_all_tests;

#[cfg(test)]
#[path = "analysis_ruler_escape_tests.rs"]
mod analysis_ruler_escape_tests;

#[cfg(test)]
#[path = "count_options_bar_tests.rs"]
mod count_options_bar_tests;
