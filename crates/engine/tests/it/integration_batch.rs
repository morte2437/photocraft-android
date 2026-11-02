//! Cross-PR acceptance checks for the merged parity batch: each test pins one fix through the
//! public command surface, so a bad merge between the PRs shows up here.
use photocraft_engine::Session;
use serde_json::json;

fn session(width: u32, height: u32) -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": width, "height": height})).unwrap();
    session
}

#[test]
fn a_new_fill_layer_consumes_the_selection_it_took_as_mask() {
    let mut s = session(32, 24);
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 8})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#4080c0"})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn define_pattern_accepts_a_canvas_wider_than_the_old_4000_px_cap() {
    let mut s = session(4500, 8);
    s.execute("edit.definePattern", json!({"name": "Wide"})).unwrap();
    let mut s = session(8001, 8);
    assert!(s.execute("edit.definePattern", json!({"name": "TooWide"})).is_err());
}

#[test]
fn gaussian_blur_accepts_radii_below_one_without_failing() {
    for radius in [0.1, 0.3, 0.7, 1.0] {
        let mut s = session(24, 16);
        s.execute("filter.blur.gaussianBlur", json!({"radius": radius})).unwrap();
    }
}
