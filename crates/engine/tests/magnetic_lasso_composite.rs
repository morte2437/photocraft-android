//! The Magnetic Lasso follows the edges of the visible composite, whichever layer is active
//! (#11686), in the live `EdgeSource` and in `select.magneticLasso` with `trace: true`.

use photocraft_algo::magnetic::Settings;
use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_engine::magnetic_cmds::EdgeSource;
use photocraft_geom::Rect;
use serde_json::json;

/// The red rectangle's pixels: x 50..125, y 40..100, 75 x 60 = 4500 px.
const RED: Rect = Rect { x0: 50, y0: 40, x1: 125, y1: 100 };
const RED_AREA: f32 = 4500.0;
/// Fastening points 4 px outside the rectangle's corners.
const OUTSIDE: [[f64; 2]; 4] = [[46.0, 36.0], [129.0, 36.0], [129.0, 104.0], [46.0, 104.0]];

struct Doc {
    s: Session,
    background: LayerId,
    red: LayerId,
}

/// A 200 x 150 RGB document: a white Background and a layer above it filled red over `RED`. The
/// red layer is active.
fn doc() -> Doc {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150})).unwrap();
    let background = s.active().unwrap().doc.layers[0].id;
    s.edit("paint", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 150), &[1.0, 1.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let red = s.active().unwrap().active_layer.unwrap();
    assert_ne!(red, background);
    s.edit("paint", |doc, _| {
        doc.layer_mut(red).unwrap().surface_mut().unwrap().fill_rect(RED, &[0.86, 0.12, 0.12, 1.0]);
        Ok(())
    })
    .unwrap();
    Doc { s, background, red }
}

fn select(s: &mut Session, id: LayerId) {
    s.execute("layer.select", json!({"layer": id.0})).unwrap();
}

fn coverage(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn selected_area(s: &Session) -> f32 {
    (0..150).flat_map(|y| (0..200).map(move |x| (x, y))).map(|(x, y)| coverage(s, x, y)).sum()
}

/// Traces a border through `OUTSIDE` and asserts it selected the rectangle, not the points' own
/// quadrilateral (83 x 68 = 5644 px).
fn assert_snaps(s: &mut Session, what: &str) {
    s.execute("select.magneticLasso", json!({"points": OUTSIDE, "trace": true})).unwrap();
    let area = selected_area(s);
    assert!((area - RED_AREA).abs() < 150.0, "{what}: the border should select the rectangle ({RED_AREA} px), selected {area} px");
    assert!(coverage(s, 52, 42) > 0.99 && coverage(s, 122, 97) > 0.99, "{what}: the rectangle's corners are inside");
    assert!(coverage(s, 48, 70) < 0.01 && coverage(s, 127, 70) < 0.01, "{what}: just outside the rectangle is not selected");
    s.execute("select.deselect", json!({})).unwrap();
}

/// Whether a fastening point 4 px outside the rectangle's left edge snaps onto it.
fn edge_snaps(s: &Session) -> bool {
    let mut src = EdgeSource::new(s).unwrap();
    let p = src.snap([46.0, 70.0], Settings::default()).unwrap();
    (p[0] - 50.0).abs() < 1.5
}

#[test]
fn the_border_snaps_whichever_layer_is_active() {
    let mut d = doc();
    assert_snaps(&mut d.s, "the red layer active");
    select(&mut d.s, d.background);
    assert_snaps(&mut d.s, "the Background active");
    assert!(edge_snaps(&d.s), "Background active: the live source sees the red layer's edge");
    d.s.execute("layer.new.layer", json!({})).unwrap();
    assert!(d.s.active().unwrap().active_layer.is_some_and(|id| id != d.background && id != d.red));
    assert!(edge_snaps(&d.s), "a new empty layer active");
    assert_snaps(&mut d.s, "a new empty layer active");
}

#[test]
fn an_adjustment_or_group_layer_active_snaps_too() {
    let mut d = doc();
    d.s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let st = d.s.active().unwrap();
    assert!(st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.surface().is_none()));
    assert!(edge_snaps(&d.s), "an adjustment layer active");
    assert_snaps(&mut d.s, "an adjustment layer active");
    select(&mut d.s, d.background);
    d.s.execute("layer.new.group", json!({})).unwrap();
    assert!(edge_snaps(&d.s), "an empty group active");
    assert_snaps(&mut d.s, "an empty group active");
}

#[test]
fn a_hidden_layer_contributes_no_edges() {
    let mut d = doc();
    select(&mut d.s, d.background);
    d.s.execute("layer.hideLayers", json!({"layer": d.red.0})).unwrap();
    assert!(!edge_snaps(&d.s), "the hidden red layer has no edge to snap to");
    d.s.execute("select.magneticLasso", json!({"points": OUTSIDE, "trace": true})).unwrap();
    let area = selected_area(&d.s);
    // The border follows the pointer: the points' own 83 x 68 quadrilateral.
    assert!((area - 5644.0).abs() < 150.0, "selected {area} px");
    // Showing it again brings the edge back, for a source made after the change.
    d.s.execute("layer.showLayers", json!({"layer": d.red.0})).unwrap();
    assert!(edge_snaps(&d.s));
}

#[test]
fn a_live_source_goes_stale_when_the_composite_changes() {
    let mut d = doc();
    select(&mut d.s, d.background);
    let src = EdgeSource::new(&d.s).unwrap();
    assert!(src.is_current(&d.s));
    d.s.execute("layer.hideLayers", json!({"layer": d.red.0})).unwrap();
    assert!(!src.is_current(&d.s), "hiding a layer changes what the border follows");
}
