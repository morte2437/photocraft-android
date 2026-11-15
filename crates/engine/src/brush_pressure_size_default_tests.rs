//! Pressure for Size starts off (#11809). The reference application leaves both pressure toggles
//! in the options bar unhighlighted after Reset Tool, so a fresh brush has neither on — and the
//! brush Reset Tool hands back is `BrushSettings::default()` plus smoothing
//! (`crates/ui-egui/src/canvas/paint_mouse.rs::fresh_brush`), so the default is what the tool
//! reset reports through `brush.get`. The options bar's two `circle-dot` buttons are drawn from
//! these same two fields (`crates/ui-egui/src/panels/options_bar.rs`), so off here means drawn
//! off there.

use crate::Session;
use photocraft_paint::brush::BrushSettings;
use serde_json::{Value, json};

/// The brush a fresh session starts with: neither pressure toggle on.
#[test]
fn the_default_brush_has_neither_pressure_toggle_on() {
    let b = BrushSettings::default();
    assert!(!b.pressure_size, "Pressure for Size is off by default, as the reference leaves it after Reset Tool (#11809)");
    assert!(!b.pressure_opacity, "Pressure for Opacity stays off");
}

/// What `brush.get` answers on a fresh session, which is what the options bar reads to draw the
/// two toggles.
#[test]
fn brush_get_reports_both_pressure_toggles_off() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let got = s.execute("brush.get", json!({})).unwrap();
    assert_eq!(got["pressureSize"], json!(false), "brush.get reports Pressure for Size off (#11809)");
    assert_eq!(got["pressureOpacity"], json!(false), "brush.get reports Pressure for Opacity off");
    // The session's own starting brush is the default brush at Photoshop's 10 % smoothing — the
    // same brush Reset Tool hands back — so only the smoothing differs.
    assert_eq!(
        s.tools.brush,
        BrushSettings { smoothing: photocraft_paint::brush::Smoothing { amount: 0.1, ..Default::default() }, ..BrushSettings::default() }
    );
}

/// A brush that does not name the field — a preset saved before #11809, an older recorded
/// command — takes the off default rather than the old on one.
#[test]
fn a_brush_that_omits_the_field_takes_the_off_default() {
    let b: BrushSettings = serde_json::from_value(json!({"size": 30.0, "hardness": 1.0})).unwrap();
    assert!(!b.pressure_size, "an omitted pressureSize reads as off (#11809)");
    assert!(!b.pressure_opacity, "an omitted pressureOpacity reads as off");
    assert_eq!(b.size, 30.0, "the fields that are there still apply");
}

/// Switching it on still works: `tools.setBrush` writes the field `brush.get` reads back, and the
/// same field the options-bar button is drawn from.
#[test]
fn pressure_for_size_still_switches_on_and_off() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"pressureSize": true}})).unwrap();
    assert!(s.tools.brush.pressure_size, "the toggle still turns on");
    assert_eq!(s.execute("brush.get", json!({})).unwrap()["pressureSize"], json!(true));
    s.execute("tools.setBrush", json!({"brush": {"pressureSize": false}})).unwrap();
    assert!(!s.tools.brush.pressure_size, "and off again");
    assert!(!s.tools.brush.pressure_opacity, "it never touches Pressure for Opacity");
}

/// One stroke of a pressure ramp (0.3 → 0.65 → 1.0) across a new empty layer with `brush`: the
/// layer's pixels and what the stroke covers.
fn ramp(brush: Value) -> (Vec<f32>, photocraft_geom::Rect) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 40})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[20.0, 20.0, 0.3], [40.0, 20.0, 0.65], [60.0, 20.0, 1.0]], "size": 24.0, "color": "#ff0000", "brush": brush}))
        .unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (surf.read_region(st.doc.bounds()), surf.content_bounds())
}

/// Done when 2: a pen stroke with varying pressure and the default brush keeps a constant size,
/// so it paints exactly what the same stroke with Pressure for Size turned off does — and
/// something else with it turned on, which is what makes this say anything.
#[test]
fn a_default_brush_stroke_keeps_its_size_under_a_pressure_ramp() {
    let (off, off_box) = ramp(json!({"spacing": 0.25, "pressureSize": false}));
    let (on, on_box) = ramp(json!({"spacing": 0.25, "pressureSize": true}));
    let (dflt, dflt_box) = ramp(json!({"spacing": 0.25}));
    assert_ne!(on, off, "the ramp alone does not change the stroke: Pressure for Size has to");
    assert_eq!(dflt, off, "the default brush paints like Pressure for Size off (#11809)");
    assert_eq!(dflt_box, off_box, "and reaches as far under the whole ramp");
    assert_ne!(dflt_box, on_box, "the turned-on brush still tapers the stroke's ends");
}
