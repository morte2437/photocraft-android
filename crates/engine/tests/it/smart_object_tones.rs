use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{Session, smart_cmds};
use photocraft_raster::Surface;
use serde_json::json;

fn mean_difference(a: &Surface, b: &Surface) -> f64 {
    let r = a.content_bounds().intersect(&b.content_bounds());
    let (mut sum, mut n) = (0f64, 0u32);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let (p, q) = (a.rgba(x, y), b.rgba(x, y));
            for c in 0..3 {
                sum += f64::from((p[c] - q[c]).abs());
                n += 1;
            }
        }
    }
    sum / f64::from(n.max(1)) * 255.0
}

fn converted(cmd: &str) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 10], [60, 40]], "size": 20, "hardness": 0.3, "color": "#e04a20"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 40], [60, 5]], "size": 14, "hardness": 0.6, "color": "#2a7fd0"})).unwrap();
    let pixel = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.duplicate", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = s.active().unwrap().active_layer.unwrap();
    s.execute(cmd, json!({})).unwrap();
    (s, pixel, smart)
}

#[test]
fn smart_objects_re_render_with_the_tones_of_the_converted_document() {
    for cmd in ["image.mode.bits32", "image.mode.grayscale"] {
        let (s, pixel, smart) = converted(cmd);
        let st = s.active().unwrap();
        let layer = st.doc.layer(smart).unwrap().clone();
        let mut rerendered = layer.clone();
        smart_cmds::refresh_layer(&st.doc, &mut rerendered).unwrap();
        let (LayerContent::Smart(stored), LayerContent::Smart(fresh)) = (&layer.content, &rerendered.content) else { panic!("not a smart object") };
        let (stored, fresh) = (stored.cache.as_ref().unwrap(), fresh.cache.as_ref().unwrap());
        let reference = st.doc.layer(pixel).unwrap().surface().unwrap();
        let drift = mean_difference(stored, fresh);
        assert!(drift < 1.0, "{cmd}: a re-render drifts {drift:.2}/255 from the converted rendering");
        let (off, baseline) = (mean_difference(fresh, reference), mean_difference(stored, reference));
        assert!(
            off <= baseline + 0.5,
            "{cmd}: the re-render is {off:.2}/255 from a pixel layer converted the same way, the converted rendering {baseline:.2}/255"
        );
    }
}
