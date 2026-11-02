//! Filter › Blur Gallery as Photoshop's workspace (#3373): Field Blur, Iris Blur, Tilt-Shift and
//! Spin Blur are edited on the image. While the dialog is open the image under it takes the
//! pointer: a click on the image adds a pin (and selects it), a drag moves a pin, and a drag on the
//! selected pin's ring turns its amount up or down (one turn = the full range; the arc shows it).
//! Delete or Backspace removes the selected pin. The dialog shows the selected pin's amount and the
//! Preview switch; OK runs the filter.
//!
//! - **Field Blur** interpolates the pins' blur between them.
//! - **Iris Blur**: a sharp superellipse, blurred outside. The selected pin shows Photoshop's
//!   handles: four on the outline (drag to resize that axis and turn the shape), a square at its
//!   corner (drag out to round it towards a rectangle) and four dots inside (the sharp core, where
//!   the blur starts).
//! - **Spin Blur**: an ellipse whose content turns about its centre; the outline handles resize and
//!   turn it, and the ring sets the blur angle (one turn = 360°).
//! - **Tilt-Shift**: one sharp band (the engine has a single one, so a click adds nothing). The
//!   solid lines bound the sharp band, the dashed lines the transition, and the dots on the solid
//!   lines turn it.
//!
//! Everything lives in the dialog's fields, so automation drives it with `ui.dialog.set`: `pins` as
//! the command takes them (x and y as fractions of the filter's bounds, radii as fractions of their
//! shorter side) and `__selected`; Tilt-Shift keeps the command's own `centerX`, `centerY`,
//! `angle`, `focus`, `transition` and `blur`. The bounds are the engine's: the selection's when
//! there is one, else the canvas.

use egui::{Color32, Pos2, Stroke, vec2};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

pub const COMMAND: &str = "filter.blurGallery.fieldBlur";
pub const IRIS: &str = "filter.blurGallery.irisBlur";
pub const SPIN: &str = "filter.blurGallery.spinBlur";
pub const TILT: &str = "filter.blurGallery.tiltShift";
/// The blur range, in pixels.
const MAX_BLUR: f64 = 500.0;
/// Spin Blur's blur angle range, in degrees.
const MAX_SPIN: f64 = 360.0;
/// Photoshop's first pin.
const DEFAULT_BLUR: f64 = 15.0;
/// More pins than anyone places by hand; keeps the params and the per-pixel loop bounded.
pub const MAX_PINS: usize = 64;
/// Screen radii (points): the pin's dot, how near a press must be to grab it, and its ring.
const DOT: f32 = 5.0;
const GRAB: f32 = 9.0;
const RING: f32 = 24.0;
const RING_BAND: f32 = 5.0;
/// Screen size (points) of a shape handle.
const HANDLE: f32 = 4.5;
/// How near (points) a press must be to a Tilt-Shift line to drag it.
const LINE_GRAB: f32 = 5.0;
/// Radii, as the commands bound them.
const MIN_RADIUS: f64 = 0.01;
const MAX_FEATHER: f64 = 0.99;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Field,
    Iris,
    Spin,
    Tilt,
}

impl Kind {
    /// The amount the ring and the dialog set: blur in pixels, or Spin's blur angle in degrees.
    fn max(self) -> f64 {
        if self == Kind::Spin { MAX_SPIN } else { MAX_BLUR }
    }

    /// The dialog field the selected pin's amount is mirrored to (remembered for next time).
    fn amount_key(self) -> &'static str {
        if self == Kind::Spin { "blurAngle" } else { "blur" }
    }
}

fn kind(fields: &Map<String, Value>) -> Option<Kind> {
    match fields.get("__command").and_then(Value::as_str) {
        Some(COMMAND) => Some(Kind::Field),
        Some(IRIS) => Some(Kind::Iris),
        Some(SPIN) => Some(Kind::Spin),
        Some(TILT) => Some(Kind::Tilt),
        _ => None,
    }
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    kind(fields).is_some()
}

/// The Blur Gallery filters that open as this workspace.
pub fn handles(command: &str) -> bool {
    matches!(command, COMMAND | IRIS | SPIN | TILT)
}

/// An Iris or Spin pin's shape, as the command takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Iris {
    /// Radii as fractions of the bounds' shorter side.
    pub rx: f64,
    pub ry: f64,
    /// Degrees, clockwise on screen.
    pub angle: f64,
    /// 0 = ellipse … 100 = rounded rectangle (Iris only).
    pub roundness: f64,
    /// Where the blur starts, as a fraction of the shape (Iris only).
    pub feather: f64,
}

impl Default for Iris {
    /// Iris Blur's defaults.
    fn default() -> Self {
        Iris { rx: 0.35, ry: 0.25, angle: 0.0, roundness: 0.0, feather: 0.5 }
    }
}

impl Iris {
    /// Spin Blur's defaults: a circle.
    pub fn spin() -> Self {
        Iris { rx: 0.3, ry: 0.3, ..Iris::default() }
    }

    fn clean(self) -> Self {
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        Iris {
            rx: f(self.rx, 0.35).clamp(MIN_RADIUS, 1.0),
            ry: f(self.ry, 0.25).clamp(MIN_RADIUS, 1.0),
            angle: (f(self.angle, 0.0) + 180.0).rem_euclid(360.0) - 180.0,
            roundness: f(self.roundness, 0.0).clamp(0.0, 100.0),
            feather: f(self.feather, 0.5).clamp(0.0, MAX_FEATHER),
        }
    }

    /// The superellipse exponent the engine uses (2 = ellipse … 8).
    fn exponent(self) -> f64 {
        2.0 + self.roundness / 100.0 * 6.0
    }
}

/// Tilt-Shift's band: its angle, and the sharp band's and the transition's widths beyond the
/// centre line, as fractions of the bounds' shorter side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tilt {
    /// Degrees, −90…90; 0 is a horizontal band, positive turns it anticlockwise on screen.
    pub angle: f64,
    pub focus: f64,
    pub transition: f64,
}

impl Tilt {
    fn clean(self) -> Self {
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        // The band is the same turned half a turn: keep the command's −90…90.
        let mut angle = (f(self.angle, 0.0) + 180.0).rem_euclid(360.0) - 180.0;
        if angle > 90.0 {
            angle -= 180.0;
        } else if angle < -90.0 {
            angle += 180.0;
        }
        Tilt { angle, focus: f(self.focus, 0.1).clamp(0.0, 1.0), transition: f(self.transition, 0.15).clamp(0.01, 1.0) }
    }

    /// The unit normal of the band (the engine's), and its direction along the band.
    fn axes(self) -> ([f64; 2], [f64; 2]) {
        let (s, c) = self.angle.to_radians().sin_cos();
        ([s, c], [c, -s])
    }
}

/// Turn a freshly opened Blur Gallery dialog into the workspace: one pin in the middle of the
/// filter's bounds with the remembered settings, selected.
pub fn init(app: &mut PhotocraftApp, fields: &mut Map<String, Value>) {
    reset(fields);
    let bounds = app.session.active().map(|st| (st.doc.id, st.doc.bounds(), st.doc.selection.clone()));
    if let Some((doc, canvas, selection)) = bounds {
        let b = selection.map(|s| app.cached_bounds(u64::MAX - doc.0, &s)).filter(|b| !b.is_empty()).unwrap_or(canvas);
        fields.insert("__bounds".into(), json!([b.x0, b.y0, b.x1, b.y1]));
    }
}

/// One pin in the middle with the dialog's settings, selected. Alt+Cancel (Reset) runs this
/// alone, so the bounds captured on opening stay.
pub fn reset(fields: &mut Map<String, Value>) {
    let Some(k) = kind(fields) else { return };
    let num = |key: &str, d: f64| fields.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
    let amount = num(k.amount_key(), DEFAULT_BLUR).clamp(0.0, k.max());
    let shape = |d: Iris| Iris { rx: num("radiusX", d.rx), ry: num("radiusY", d.ry), angle: num("angle", d.angle), ..d };
    let pin = match k {
        Kind::Field => Pin { x: 0.5, y: 0.5, blur: amount, iris: None, tilt: None },
        Kind::Iris => {
            let iris = Iris { roundness: num("roundness", 0.0), feather: num("feather", 0.5), ..shape(Iris::default()) }.clean();
            Pin { x: 0.5, y: 0.5, blur: amount, iris: Some(iris), tilt: None }
        }
        Kind::Spin => Pin { x: 0.5, y: 0.5, blur: amount, iris: Some(shape(Iris::spin()).clean()), tilt: None },
        Kind::Tilt => {
            let tilt = Tilt { angle: num("angle", 0.0), focus: num("focus", 0.1), transition: num("transition", 0.15) }.clean();
            Pin { x: num("centerX", 0.5).clamp(0.0, 1.0), y: num("centerY", 0.5).clamp(0.0, 1.0), blur: amount, iris: None, tilt: Some(tilt) }
        }
    };
    // The pins replace a single shape's params (Tilt-Shift keeps its own).
    if k != Kind::Tilt {
        for key in ["centerX", "centerY", "radiusX", "radiusY", "angle", "roundness", "feather"] {
            fields.remove(key);
        }
    }
    set_pins(fields, &[pin]);
    select(fields, Some(0));
}

/// Where the dialog first opens (offset from the window's top-left): at the right edge, beside the
/// image, as Photoshop docks its Blur Tools panel, so the pins under it stay in view. Moved, it
/// reopens where it was left, like any dialog.
pub fn first_place(fields: &Map<String, Value>, ctx: &egui::Context) -> Option<egui::Vec2> {
    owns(fields).then(|| vec2((ctx.content_rect().width() - 480.0).max(0.0), 96.0))
}

/// One pin: position as fractions of the bounds, its amount (blur in pixels, Spin's blur angle in
/// degrees), and an Iris or Spin pin's shape or the Tilt-Shift band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub x: f64,
    pub y: f64,
    pub blur: f64,
    pub iris: Option<Iris>,
    pub tilt: Option<Tilt>,
}

/// The pins in `fields`, cleaned (finite, in range, at most [`MAX_PINS`]).
pub fn pins(fields: &Map<String, Value>) -> Vec<Pin> {
    let Some(k) = kind(fields) else { return Vec::new() };
    let num = |v: &Value, key: &str, d: f64| v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d);
    if k == Kind::Tilt {
        // Tilt-Shift's one band is the command's own params.
        let f = Value::Object(fields.clone());
        let tilt = Tilt { angle: num(&f, "angle", 0.0), focus: num(&f, "focus", 0.1), transition: num(&f, "transition", 0.15) }.clean();
        let (x, y) = (num(&f, "centerX", 0.5).clamp(0.0, 1.0), num(&f, "centerY", 0.5).clamp(0.0, 1.0));
        return vec![Pin { x, y, blur: num(&f, "blur", DEFAULT_BLUR).clamp(0.0, MAX_BLUR), iris: None, tilt: Some(tilt) }];
    }
    let Some(list) = fields.get("pins").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .take(MAX_PINS)
        .map(|v| Pin {
            x: num(v, "x", 0.5).clamp(0.0, 1.0),
            y: num(v, "y", 0.5).clamp(0.0, 1.0),
            blur: num(v, k.amount_key(), DEFAULT_BLUR).clamp(0.0, k.max()),
            iris: match k {
                Kind::Iris | Kind::Spin => {
                    let d = if k == Kind::Spin { Iris::spin() } else { Iris::default() };
                    let iris = Iris { rx: num(v, "radiusX", d.rx), ry: num(v, "radiusY", d.ry), angle: num(v, "angle", d.angle), ..d };
                    let iris = if k == Kind::Iris {
                        Iris { roundness: num(v, "roundness", d.roundness), feather: num(v, "feather", d.feather), ..iris }
                    } else {
                        iris
                    };
                    Some(iris.clean())
                }
                _ => None,
            },
            tilt: None,
        })
        .collect()
}

fn set_pins(fields: &mut Map<String, Value>, pins: &[Pin]) {
    let Some(k) = kind(fields) else { return };
    if k == Kind::Tilt {
        let Some(p) = pins.first() else { return };
        let t = p.tilt.unwrap_or(Tilt { angle: 0.0, focus: 0.1, transition: 0.15 });
        for (key, v) in [("centerX", p.x), ("centerY", p.y), ("blur", p.blur), ("angle", t.angle), ("focus", t.focus), ("transition", t.transition)] {
            fields.insert(key.into(), json!(v));
        }
        return;
    }
    let list: Vec<Value> = pins
        .iter()
        .map(|p| match (k, p.iris) {
            (Kind::Iris, Some(s)) => {
                json!({"x": p.x, "y": p.y, "blur": p.blur, "radiusX": s.rx, "radiusY": s.ry, "angle": s.angle, "roundness": s.roundness, "feather": s.feather})
            }
            (Kind::Spin, Some(s)) => json!({"x": p.x, "y": p.y, "blurAngle": p.blur, "radiusX": s.rx, "radiusY": s.ry, "angle": s.angle}),
            _ => json!({"x": p.x, "y": p.y, "blur": p.blur}),
        })
        .collect();
    fields.insert("pins".into(), Value::Array(list));
}

/// The selected pin's index, if it exists.
pub fn selected(fields: &Map<String, Value>) -> Option<usize> {
    let n = pins(fields).len();
    fields.get("__selected").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < n)
}

fn select(fields: &mut Map<String, Value>, i: Option<usize>) {
    fields.insert("__selected".into(), i.map_or(Value::Null, |i| json!(i)));
    // The dialog's amount (remembered for the next time) follows the selected pin.
    if let (Some(k), Some(p)) = (kind(fields), i.and_then(|i| pins(fields).get(i).copied())) {
        fields.insert(k.amount_key().into(), json!(p.blur));
    }
}

/// The filter's bounds in document pixels `[x0, y0, x1, y1]`.
fn bounds(fields: &Map<String, Value>) -> Option<[f64; 4]> {
    let b = fields.get("__bounds")?.as_array()?;
    let v = |i: usize| b.get(i).and_then(Value::as_f64);
    let r = [v(0)?, v(1)?, v(2)?, v(3)?];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// The bounds' shorter side, which radii and Tilt-Shift widths are fractions of.
fn short_side(b: [f64; 4]) -> f64 {
    (b[2] - b[0]).min(b[3] - b[1]).max(1.0)
}

fn to_doc(b: [f64; 4], p: Pin) -> [f64; 2] {
    [b[0] + p.x * (b[2] - b[0]), b[1] + p.y * (b[3] - b[1])]
}

fn from_doc(b: [f64; 4], d: [f64; 2]) -> (f64, f64) {
    (((d[0] - b[0]) / (b[2] - b[0])).clamp(0.0, 1.0), ((d[1] - b[1]) / (b[3] - b[1])).clamp(0.0, 1.0))
}

/// A shaped pin's local point `(u, v)` (1 = on the outline along that axis) in document pixels.
fn iris_point(b: [f64; 4], p: Pin, s: Iris, u: f64, v: f64) -> [f64; 2] {
    let c = to_doc(b, p);
    let ss = short_side(b);
    let (x, y) = (u * s.rx * ss, v * s.ry * ss);
    let (sin, cos) = s.angle.to_radians().sin_cos();
    [c[0] + x * cos - y * sin, c[1] + x * sin + y * cos]
}

/// A document point in a shaped pin's local coordinates (the engine's `u`, `v`).
fn iris_local(b: [f64; 4], p: Pin, s: Iris, d: [f64; 2]) -> (f64, f64) {
    let c = to_doc(b, p);
    let ss = short_side(b);
    let (dx, dy) = (d[0] - c[0], d[1] - c[1]);
    let (sin, cos) = s.angle.to_radians().sin_cos();
    ((dx * cos + dy * sin) / (s.rx * ss), (-dx * sin + dy * cos) / (s.ry * ss))
}

/// The superellipse's diagonal point `t` (on the shape at (t, t)): 2^(−1/m).
fn diagonal(s: Iris) -> f64 {
    0.5f64.powf(1.0 / s.exponent())
}

/// A shaped pin's outline handles (along its axes), Iris's roundness square (upper right) and its
/// four feather dots (on the diagonals), in local coordinates.
fn axis_handles() -> [(f64, f64); 4] {
    [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
}

fn round_handle(s: Iris) -> (f64, f64) {
    let t = diagonal(s);
    (t, -t)
}

fn feather_handles(s: Iris) -> [(f64, f64); 4] {
    let k = s.feather * diagonal(s);
    [(k, k), (-k, k), (-k, -k), (k, -k)]
}

/// The point of the Tilt-Shift band at `offset` pixels across it (along its normal) and `along`
/// pixels along it from the centre, in document pixels.
fn tilt_point(b: [f64; 4], p: Pin, t: Tilt, offset: f64, along: f64) -> [f64; 2] {
    let c = to_doc(b, p);
    let (n, d) = t.axes();
    [c[0] + n[0] * offset + d[0] * along, c[1] + n[1] * offset + d[1] * along]
}

/// How far a document point is across the band from its centre line, in pixels (signed).
fn tilt_offset(b: [f64; 4], p: Pin, t: Tilt, d: [f64; 2]) -> f64 {
    let c = to_doc(b, p);
    let (n, _) = t.axes();
    (d[0] - c[0]) * n[0] + (d[1] - c[1]) * n[1]
}

/// A Tilt-Shift turn handle: one on each solid line, beside the pin's ring (so it never sits on
/// the ring, however narrow the band). `side` is +1 or −1.
pub fn tilt_turn_handle(b: [f64; 4], p: Pin, t: Tilt, xf: &ViewXform, side: f64) -> [f64; 2] {
    let ss = short_side(b);
    let scale = f64::from(xf.zoom_x()).max(1e-6);
    tilt_point(b, p, t, side * t.focus * ss, f64::from(RING + 18.0) / scale)
}

/// Remove the selected pin; the one before it (or the new first) is selected. Tilt-Shift's one
/// band stays.
pub fn remove_selected(fields: &mut Map<String, Value>) -> bool {
    if kind(fields) == Some(Kind::Tilt) {
        return false;
    }
    let Some(i) = selected(fields) else { return false };
    let mut list = pins(fields);
    list.remove(i);
    set_pins(fields, &list);
    select(fields, if list.is_empty() { None } else { Some(i.saturating_sub(1)) });
    true
}

/// The dialog body: the selected pin's amount, how to edit the pins, and Preview.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let k = kind(f).unwrap_or(Kind::Field);
    match selected(f) {
        Some(i) => {
            let mut list = pins(f);
            if let Some(pin) = list.get_mut(i) {
                let mut v = pin.blur as f32;
                let (label, unit) = if k == Kind::Spin { (tl!("Blur Angle"), "°") } else { (tl!("Blur"), "px") };
                if crate::widgets::slider_row(ui, label, &mut v, 0.0..=k.max() as f32, unit, None).changed() {
                    pin.blur = f64::from(v.round()).clamp(0.0, k.max());
                    set_pins(f, &list);
                    select(f, Some(i));
                }
            }
        }
        None => {
            ui.label(egui::RichText::new(tl!("No pin selected")).color(t.text_dim));
        }
    }
    ui.add_space(4.0);
    let hint = |ui: &mut egui::Ui, text: &str| ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim).size(11.5)).wrap());
    if k == Kind::Tilt {
        hint(
            ui,
            tl!(
                "Drag the pin to move the band, or its ring to change its blur. Drag the solid lines to widen the sharp band, the dashed lines to set the transition, and the dots to turn it."
            ),
        );
    } else {
        hint(ui, tl!("Click the image to add a pin; drag a pin to move it, or its ring to change its blur. Delete removes the selected pin."));
    }
    if k == Kind::Iris {
        hint(
            ui,
            tl!("Drag the handles on the outline to resize and turn it, the square to round its corners, and the dots inside to set where the blur starts."),
        );
    }
    if k == Kind::Spin {
        hint(ui, tl!("Drag the handles on the outline to resize and turn it; the ring sets the blur angle."));
    }
    // Delete removes the selected pin, unless a field is being typed in.
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if !typing && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)) {
        remove_selected(f);
        ui.ctx().request_repaint();
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

/// What a press at screen point `at` grabs: one of the selected pin's handles (axis 0–3 = +u, +v,
/// −u, −v; Tilt-Shift's turn dots and lines), its ring, a pin, or nothing (a new pin).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Axis(usize, usize),
    Round(usize),
    Feather(usize),
    Turn(usize, i8),
    Focus(usize),
    Transition(usize),
    Ring(usize),
    Pin(usize),
    New,
}

/// Distance in screen points from `at` to the infinite line through screen points `a` and `b`.
fn line_distance(at: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let len = d.length();
    if len <= f32::EPSILON { at.distance(a) } else { ((at - a).x * d.y - (at - a).y * d.x).abs() / len }
}

fn grab(fields: &Map<String, Value>, xf: &ViewXform, at: Pos2) -> Grab {
    let Some(b) = bounds(fields) else { return Grab::New };
    let k = kind(fields);
    let list = pins(fields);
    let screen = |d: [f64; 2]| xf.to_screen(d[0] as f32, d[1] as f32);
    let near = |d: [f64; 2]| screen(d).distance(at) <= GRAB;
    if let Some(i) = selected(fields)
        && let Some(p) = list.get(i).copied()
    {
        if let Some(s) = p.iris {
            if let Some(a) = axis_handles().iter().position(|&(u, v)| near(iris_point(b, p, s, u, v))) {
                return Grab::Axis(i, a);
            }
            if k == Some(Kind::Iris) {
                let (u, v) = round_handle(s);
                if near(iris_point(b, p, s, u, v)) {
                    return Grab::Round(i);
                }
                if feather_handles(s).iter().any(|&(u, v)| near(iris_point(b, p, s, u, v))) {
                    return Grab::Feather(i);
                }
            }
        }
        if let Some(t) = p.tilt {
            for side in [1i8, -1] {
                if near(tilt_turn_handle(b, p, t, xf, f64::from(side))) {
                    return Grab::Turn(i, side);
                }
            }
        }
        if (screen(to_doc(b, p)).distance(at) - RING).abs() <= RING_BAND {
            return Grab::Ring(i);
        }
        if let Some(t) = p.tilt
            && screen(to_doc(b, p)).distance(at) > GRAB
        {
            let ss = short_side(b);
            let on = |offset: f64| line_distance(at, screen(tilt_point(b, p, t, offset, -ss)), screen(tilt_point(b, p, t, offset, ss))) <= LINE_GRAB;
            let focus = t.focus * ss;
            if on(focus) || on(-focus) {
                return Grab::Focus(i);
            }
            let edge = (t.focus + t.transition) * ss;
            if on(edge) || on(-edge) {
                return Grab::Transition(i);
            }
        }
    }
    list.iter()
        .enumerate()
        .map(|(i, p)| (i, screen(to_doc(b, *p)).distance(at)))
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Grab::New, |(i, _)| Grab::Pin(i))
}

/// The pointer on the free image under the dialog: `over` is where it is, `press` where the
/// primary button is held (`dialogs::free_press`), both in screen points. Returns whether the
/// workspace took it.
pub fn canvas(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, over: Pos2, press: Option<Pos2>) -> bool {
    let Some(d) = app.ui.dialogs.last_mut().filter(|d| owns(&d.fields)) else { return false };
    let f = &mut d.fields;
    let Some(b) = bounds(f) else { return false };
    let Some(k) = kind(f) else { return false };
    let (start, down) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down()));
    ctx.set_cursor_icon(match grab(f, xf, over) {
        Grab::Ring(_) | Grab::Round(_) | Grab::Turn(..) => egui::CursorIcon::Alias,
        Grab::Axis(..) | Grab::Feather(_) => egui::CursorIcon::Move,
        Grab::Focus(_) | Grab::Transition(_) => egui::CursorIcon::ResizeVertical,
        Grab::Pin(_) => egui::CursorIcon::Grab,
        Grab::New if k == Kind::Tilt => egui::CursorIcon::Default,
        Grab::New => egui::CursorIcon::Crosshair,
    });
    if !down && press.is_none() {
        f.remove("__drag");
        return true;
    }
    let Some(at) = press else { return true };
    let doc = xf.to_doc(at);
    let mut list = pins(f);
    if start {
        let drag = match grab(f, xf, at) {
            Grab::Axis(i, a) => json!({"axis": i, "k": a}),
            Grab::Round(i) => json!({"round": i}),
            Grab::Feather(i) => json!({"feather": i}),
            Grab::Turn(i, side) => json!({"turn": i, "side": side}),
            Grab::Focus(i) => json!({"focus": i}),
            Grab::Transition(i) => json!({"transition": i}),
            Grab::Ring(i) => {
                let c = list.get(i).map(|p| to_doc(b, *p)).unwrap_or(doc);
                json!({"ring": i, "angle": (doc[1] - c[1]).atan2(doc[0] - c[0])})
            }
            Grab::Pin(i) => {
                select(f, Some(i));
                let c = list.get(i).map(|p| to_doc(b, *p)).unwrap_or(doc);
                json!({"pin": i, "dx": c[0] - doc[0], "dy": c[1] - doc[1]})
            }
            // Tilt-Shift has one band: a click elsewhere adds nothing.
            Grab::New if k != Kind::Tilt && list.len() < MAX_PINS && (b[0]..=b[2]).contains(&doc[0]) && (b[1]..=b[3]).contains(&doc[1]) => {
                // A new pin takes the selected pin's amount (as the dialog shows it) and shape.
                let blur = f.get(k.amount_key()).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(DEFAULT_BLUR).clamp(0.0, k.max());
                let fallback = if k == Kind::Spin { Iris::spin() } else { Iris::default() };
                let iris = matches!(k, Kind::Iris | Kind::Spin).then(|| selected(f).and_then(|i| list.get(i)).and_then(|p| p.iris).unwrap_or(fallback));
                let (x, y) = from_doc(b, doc);
                list.push(Pin { x, y, blur, iris, tilt: None });
                set_pins(f, &list);
                select(f, Some(list.len() - 1));
                json!({"pin": list.len() - 1, "dx": 0.0, "dy": 0.0})
            }
            Grab::New => return true,
        };
        f.insert("__drag".into(), drag);
        ctx.request_repaint();
        return true;
    }
    let Some(drag) = f.get("__drag").cloned() else { return true };
    let index = |key: &str| drag.get(key).and_then(Value::as_u64).map(|i| i as usize);
    let ss = short_side(b);
    if let Some(i) = index("pin")
        && let Some(p) = list.get_mut(i)
    {
        let num = |key: &str| drag.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        (p.x, p.y) = from_doc(b, [doc[0] + num("dx"), doc[1] + num("dy")]);
    } else if let Some(i) = index("ring")
        && let Some(p) = list.get_mut(i)
    {
        // The angle turned since the last frame, the short way round: clockwise raises the amount.
        let c = to_doc(b, *p);
        let a = (doc[1] - c[1]).atan2(doc[0] - c[0]);
        let last = drag.get("angle").and_then(Value::as_f64).unwrap_or(a);
        let turn = (a - last + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        p.blur = (p.blur + turn / std::f64::consts::TAU * k.max()).clamp(0.0, k.max());
        f.insert("__drag".into(), json!({"ring": i, "angle": a}));
    } else if let Some(i) = index("axis")
        && let Some(p) = list.get_mut(i)
        && let Some(mut s) = p.iris
    {
        // The handle follows the pointer: its distance is that axis's radius, its direction turns
        // the shape (the +v handle sits a quarter turn after +u).
        let c = to_doc(b, *p);
        let (dx, dy) = (doc[0] - c[0], doc[1] - c[1]);
        let r = (dx.hypot(dy) / ss).clamp(MIN_RADIUS, 1.0);
        let a = index("k").unwrap_or(0);
        if a % 2 == 0 {
            s.rx = r;
        } else {
            s.ry = r;
        }
        s.angle = dy.atan2(dx).to_degrees() - 90.0 * a as f64;
        p.iris = Some(s.clean());
    } else if let Some(i) = index("round")
        && let Some(p) = list.get_mut(i)
        && let Some(s) = p.iris
    {
        // Out along the diagonal rounds towards a rectangle: the handle is the shape's diagonal
        // point (t, t), t = 2^(−1/m), so t gives the exponent back.
        let (u, v) = iris_local(b, *p, s, doc);
        let t = ((u.abs() + v.abs()) / 2.0).clamp(0.5f64.sqrt(), 0.5f64.powf(1.0 / 8.0));
        let m = -std::f64::consts::LN_2 / t.ln();
        p.iris = Some(Iris { roundness: (m - 2.0) / 6.0 * 100.0, ..s }.clean());
    } else if let Some(i) = index("feather")
        && let Some(p) = list.get_mut(i)
        && let Some(s) = p.iris
    {
        // The dots sit where the blur starts: the pointer's distance in the shape's own measure.
        let (u, v) = iris_local(b, *p, s, doc);
        let m = s.exponent();
        let e = (u.abs().powf(m) + v.abs().powf(m)).powf(1.0 / m);
        p.iris = Some(Iris { feather: if e.is_finite() { e } else { s.feather }, ..s }.clean());
    } else if let Some(i) = index("turn")
        && let Some(p) = list.get_mut(i)
        && let Some(t) = p.tilt
    {
        // The dot's side of the band points from the centre towards the pointer: the engine's
        // normal is (sin a, cos a).
        let c = to_doc(b, *p);
        let side = drag.get("side").and_then(Value::as_f64).unwrap_or(1.0).signum();
        let (nx, ny) = ((doc[0] - c[0]) * side, (doc[1] - c[1]) * side);
        if nx.hypot(ny) > 1e-9 {
            p.tilt = Some(Tilt { angle: nx.atan2(ny).to_degrees(), ..t }.clean());
        }
    } else if let Some(i) = index("focus")
        && let Some(p) = list.get_mut(i)
        && let Some(t) = p.tilt
    {
        p.tilt = Some(Tilt { focus: tilt_offset(b, *p, t, doc).abs() / ss, ..t }.clean());
    } else if let Some(i) = index("transition")
        && let Some(p) = list.get_mut(i)
        && let Some(t) = p.tilt
    {
        p.tilt = Some(Tilt { transition: tilt_offset(b, *p, t, doc).abs() / ss - t.focus, ..t }.clean());
    }
    set_pins(f, &list);
    if let Some(i) = ["pin", "ring", "axis", "round", "feather", "turn", "focus", "transition"].into_iter().find_map(index) {
        select(f, Some(i));
    }
    ctx.request_repaint();
    true
}

/// Draw a dark-edged white line from `a` to `b`, dashed when `dash` is set.
fn draw_line(painter: &egui::Painter, a: Pos2, b: Pos2, dash: bool) {
    let (dark, light) = (Stroke::new(3.0, Color32::from_black_alpha(150)), Stroke::new(1.5, Color32::from_white_alpha(235)));
    if dash {
        painter.extend(egui::Shape::dashed_line(&[a, b], dark, 6.0, 5.0));
        painter.extend(egui::Shape::dashed_line(&[a, b], light, 6.0, 5.0));
    } else {
        painter.line_segment([a, b], dark);
        painter.line_segment([a, b], light);
    }
}

/// Draw the pins over the image while the workspace is open: a dot per pin (filled when
/// selected) and, round the selected one, its ring with the amount as an arc from the top, and
/// its outline and handles or Tilt-Shift's lines.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(d) = app.ui.dialogs.last().filter(|d| owns(&d.fields)) else { return };
    let Some(b) = bounds(&d.fields) else { return };
    let Some(k) = kind(&d.fields) else { return };
    let sel = selected(&d.fields);
    let (dark, light) = (Stroke::new(3.0, Color32::from_black_alpha(150)), Stroke::new(1.5, Color32::from_white_alpha(235)));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let handle_stroke = Stroke::new(1.0, Color32::from_black_alpha(180));
    for (i, p) in pins(&d.fields).into_iter().enumerate() {
        let c = screen(to_doc(b, p));
        if sel != Some(i) {
            painter.circle_stroke(c, DOT, dark);
            painter.circle_stroke(c, DOT, light);
            continue;
        }
        if let Some(s) = p.iris {
            // The outline: where the blur reaches its full amount (Iris) or where the spin ends.
            let m = s.exponent();
            let outline: Vec<Pos2> = (0..=128)
                .map(|n| {
                    let a = std::f64::consts::TAU * f64::from(n) / 128.0;
                    let (cos, sin) = (a.cos(), a.sin());
                    let r = (cos.abs().powf(m) + sin.abs().powf(m)).powf(-1.0 / m);
                    screen(iris_point(b, p, s, r * cos, r * sin))
                })
                .collect();
            painter.add(egui::Shape::line(outline.clone(), dark));
            painter.add(egui::Shape::line(outline, light));
            for (u, v) in axis_handles() {
                painter.circle(screen(iris_point(b, p, s, u, v)), HANDLE, Color32::WHITE, handle_stroke);
            }
            if k == Kind::Iris {
                let (u, v) = round_handle(s);
                let r = egui::Rect::from_center_size(screen(iris_point(b, p, s, u, v)), vec2(HANDLE * 2.0, HANDLE * 2.0));
                painter.rect(r, 1.0, Color32::WHITE, handle_stroke, egui::StrokeKind::Middle);
                for (u, v) in feather_handles(s) {
                    painter.circle(screen(iris_point(b, p, s, u, v)), HANDLE + 1.0, Color32::from_white_alpha(230), handle_stroke);
                }
            }
        }
        if let Some(t) = p.tilt {
            // The sharp band's edges (solid) and where the blur is full (dashed), across the image.
            let ss = short_side(b);
            let clip = painter.with_clip_rect(egui::Rect::from_two_pos(screen([b[0], b[1]]), screen([b[2], b[3]])).intersect(painter.clip_rect()));
            let reach = (b[2] - b[0]).hypot(b[3] - b[1]);
            for (offset, dash) in [(t.focus, false), (t.focus + t.transition, true)] {
                for side in [1.0, -1.0] {
                    let o = side * offset * ss;
                    draw_line(&clip, screen(tilt_point(b, p, t, o, -reach)), screen(tilt_point(b, p, t, o, reach)), dash);
                }
            }
            for side in [1.0, -1.0] {
                painter.circle(screen(tilt_turn_handle(b, p, t, xf, side)), HANDLE + 0.5, Color32::WHITE, handle_stroke);
            }
        }
        painter.circle_stroke(c, RING, dark);
        painter.circle_stroke(c, RING, Stroke::new(1.0, Color32::from_white_alpha(140)));
        let frac = (p.blur / k.max()) as f32;
        let steps = (frac * 64.0).ceil().max(1.0) as usize;
        let arc: Vec<Pos2> = (0..=steps)
            .map(|n| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * n as f32 / steps as f32;
                c + vec2(a.cos(), a.sin()) * RING
            })
            .collect();
        if frac > 0.0 {
            painter.add(egui::Shape::line(arc.clone(), Stroke::new(4.5, Color32::from_black_alpha(150))));
            painter.add(egui::Shape::line(arc, Stroke::new(2.5, Color32::WHITE)));
        }
        painter.circle(c, DOT, Color32::WHITE, handle_stroke);
    }
}

/// Preview proxies run at 1/`k` of the document: the pins' pixel blur scales with them, as the
/// top-level pixel params (Tilt-Shift's `blur`) already do. Spin's blur angle doesn't.
pub fn scale_for_proxy(params: &mut Value, k: u32) {
    if k <= 1 {
        return;
    }
    let Some(list) = params.get_mut("pins").and_then(Value::as_array_mut) else { return };
    for pin in list {
        if let Some(b) = pin.get("blur").and_then(Value::as_f64) {
            pin["blur"] = json!(b / f64::from(k));
        }
    }
}
