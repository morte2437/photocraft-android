//! `filter.cameraRaw` must clamp `exposure` to its documented -5..5 range. Before the fix,
//! `raw_params` passed it through, so an exposure of 128 or more overflowed `2^exposure` to
//! infinity and a 32-bit layer was left with non-finite samples.

use photocraft_engine::Session;
use serde_json::json;

#[test]
fn camera_raw_clamps_exposure() {
    let hi = photocraft_engine::lens_cmds::raw_params("filter.cameraRaw", &json!({"exposure": 255.0})).expect("params parse");
    assert_eq!(hi.exposure, 5.0, "exposure 255 must clamp to 5");
    let lo = photocraft_engine::lens_cmds::raw_params("filter.cameraRaw", &json!({"exposure": -255.0})).expect("params parse");
    assert_eq!(lo.exposure, -5.0, "exposure -255 must clamp to -5");
    let mid = photocraft_engine::lens_cmds::raw_params("filter.cameraRaw", &json!({"exposure": 1.5})).expect("params parse");
    assert_eq!(mid.exposure, 1.5, "in-range exposure stays untouched");
}

#[test]
fn camera_raw_out_of_range_exposure_keeps_32_bit_layer_finite() {
    for exposure in [255.0, -255.0] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        s.execute("filter.cameraRaw", json!({"exposure": exposure})).unwrap();
        let st = s.active().unwrap();
        let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
        let surf = layer.surface().unwrap();
        let mut bad = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let px = surf.rgba(x, y);
                if px.iter().any(|v| !v.is_finite()) {
                    bad.push(px);
                }
            }
        }
        assert!(bad.is_empty(), "exposure {exposure}: {} non-finite pixels, e.g. {:?}", bad.len(), bad.first());
    }
}
