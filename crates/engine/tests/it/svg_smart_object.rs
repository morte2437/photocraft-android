//! An SVG placed as a smart object is a vector smart object: it is rasterised at its placement
//! scale (sharp at 10×, where a 20×10 px raster would blur), re-rendered when the placement
//! changes, and its contents open as shape layers.

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"10\">\
  <rect width=\"20\" height=\"10\" fill=\"#ff0000\"/>\
  <rect x=\"0\" y=\"4\" width=\"20\" height=\"0.5\" fill=\"#000000\"/>\
</svg>";

fn alpha(s: &photocraft_raster::Surface, x: i32, y: i32) -> f32 {
    let px = s.read_region(Rect::new(x, y, x + 1, y + 1));
    *px.last().unwrap()
}

fn red(s: &photocraft_raster::Surface, x: i32, y: i32) -> f32 {
    let px = s.read_region(Rect::new(x, y, x + 1, y + 1));
    px[0]
}

#[test]
fn placed_svg_is_sharp_at_its_placement_scale_and_opens_as_shapes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
    let r = photocraft_engine::file_cmds::place_bytes(&mut s, "logo.svg", SVG.as_bytes().to_vec(), None, &json!({"scale": 1000})).unwrap();
    assert_eq!(r["bounds"], json!([100.0, 150.0, 300.0, 250.0]), "{r}");
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers.last().unwrap();
    let LayerContent::Smart(sm) = &layer.content else { panic!("{}", layer.content.kind_name()) };
    let surf = layer.surface().expect("rendered");
    // A pixel-exact edge at the top of the 200 × 100 placement, not a bicubic ramp.
    assert!(alpha(surf, 200, 148) < 0.01 && alpha(surf, 200, 151) > 0.99, "{} {}", alpha(surf, 200, 148), alpha(surf, 200, 151));
    // The half-unit black line is a crisp 5 px band (rows 190..195) rather than a faint smear.
    assert!(red(surf, 200, 192) < 0.02, "black inside the line: {}", red(surf, 200, 192));
    assert!(red(surf, 200, 187) > 0.98 && red(surf, 200, 198) > 0.98, "red just outside it");
    assert!(red(surf, 200, 189) > 0.9 || red(surf, 200, 189) < 0.1, "edge rows are not half-covered");

    // Edit Contents on this smart object: the embedded source opens as shape layers.
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = &sm.source else { panic!("embedded") };
    let inner = photocraft_engine::smart_cmds::decode_source(file_name, bytes).unwrap();
    assert_eq!(inner.layers.len(), 2);
    assert!(inner.layers.iter().all(|l| matches!(l.content, LayerContent::Shape(_))));
}

#[test]
fn re_placing_at_another_scale_re_renders_the_vector_source() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 300, "background": "white"})).unwrap();
    photocraft_engine::file_cmds::place_bytes(&mut s, "logo.svg", SVG.as_bytes().to_vec(), None, &json!({"scale": 100})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers.last().unwrap();
    let LayerContent::Smart(sm) = &layer.content else { panic!() };
    let mut big = sm.clone();
    let [a, b, c, d, _, _] = big.transform.m;
    big.transform.m = [a * 12.0, b * 12.0, c * 12.0, d * 12.0, 30.0, 30.0];
    let surf = photocraft_engine::smart_cmds::render(&doc, &big).unwrap().expect("source available");
    // 20 × 10 units at 12× from (30, 30): a hard edge at x = 270 and the thin line at rows 78..84.
    assert!(alpha(&surf, 268, 60) > 0.99 && alpha(&surf, 271, 60) < 0.01);
    assert!(red(&surf, 150, 81) < 0.02 && red(&surf, 150, 75) > 0.98 && red(&surf, 150, 87) > 0.98);
}

/// A `.pcraft` opens from its cached pixels and parses an embedded SVG source only on a later
/// edit. A source whose DTD entity hides 20 000 nested groups from the depth check used to open
/// clean and then overflow the stack on Image Size or a PSD save; both are an error or a fallback
/// now. Run on a 2 MB stack, as the app's import and command threads are.
#[test]
fn a_pcraft_smart_object_with_a_hostile_svg_does_not_abort_a_later_edit() {
    let deep = format!("{}{}", "<g>".repeat(20_000), "</g>".repeat(20_000));
    let hostile = format!("<!DOCTYPE svg [<!ENTITY a \"{deep}\">]><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"10\">&a;</svg>");
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 100, "height": 100, "background": "white"})).unwrap();
            photocraft_engine::file_cmds::place_bytes(&mut s, "logo.svg", SVG.as_bytes().to_vec(), None, &json!({"scale": 100})).unwrap();
            let mut doc = (*s.active().unwrap().doc).clone();
            let LayerContent::Smart(sm) = &mut doc.layers.last_mut().unwrap().content else { panic!("smart object") };
            sm.source = photocraft_doc::SmartSource::Embedded { file_name: "logo.svg".into(), bytes: std::sync::Arc::new(hostile.into_bytes()) };
            let carrier = photocraft_format::save_to_bytes(&doc, &photocraft_format::SaveOptions::default()).unwrap();

            photocraft_engine::file_cmds::open_bytes_as(&mut s, "carrier.pcraft", &carrier, None, None).unwrap();
            let _ = s.execute("image.imageSize", json!({"width": 50, "height": 50}));
            let dir = tempfile::tempdir().unwrap();
            let _ = s.execute("file.saveACopy", json!({"path": dir.path().join("copy.psd").to_string_lossy()}));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Placed through a background job, an SVG is rendered at its placement scale on the worker and
/// lands exactly as the inline place does: still sharp, not a resampled raster.
#[test]
fn an_svg_placed_as_a_job_is_as_sharp_as_the_inline_place() {
    use photocraft_engine::jobs::Started;
    let dir = std::env::temp_dir().join(format!("photocraft-svg-place-job-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("logo.svg");
    std::fs::write(&path, SVG).unwrap();
    let params = json!({"path": path.to_string_lossy(), "scale": 1000});
    let new_doc = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
        s
    };
    let mut inline = new_doc();
    let want = inline.execute("file.placeEmbedded", params.clone()).unwrap();

    let mut s = new_doc();
    let Started::Job(id) = s.start("file.placeEmbedded", params).unwrap() else { panic!("expected a background job") };
    let mut got = s.wait_job(id).unwrap();
    // Layer ids come from a counter shared by every session: the rest of the result must match.
    let mut want = want;
    assert!(got.as_object_mut().and_then(|o| o.remove("layer")).is_some() && want.as_object_mut().and_then(|o| o.remove("layer")).is_some());
    assert_eq!(got, want);
    assert_eq!(got["bounds"], json!([100.0, 150.0, 300.0, 250.0]), "{got}");
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers.last().unwrap();
    assert!(matches!(layer.content, LayerContent::Smart(_)));
    let surf = layer.surface().expect("rendered");
    assert!(alpha(surf, 200, 148) < 0.01 && alpha(surf, 200, 151) > 0.99, "a pixel-exact top edge");
    assert!(red(surf, 200, 192) < 0.02, "the thin line is a crisp band: {}", red(surf, 200, 192));
    assert!(red(surf, 200, 187) > 0.98 && red(surf, 200, 198) > 0.98);
    let region = Rect::new(90, 140, 310, 260);
    let inline_doc = inline.active().unwrap().doc.clone();
    assert_eq!(surf.read_region(region), inline_doc.layers.last().unwrap().surface().unwrap().read_region(region), "same pixels as the inline place");
}

/// With no worker threads (the web build) an SVG place is stepped a poll at a time, and renders
/// at its placement scale like the inline place does.
#[test]
fn an_svg_placed_in_steps_is_as_sharp_as_the_inline_place() {
    use photocraft_engine::jobs::{JobOutcome, Started};
    let params = json!({"scale": 1000});
    let new_doc = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
        s
    };
    let mut inline = new_doc();
    let want = photocraft_engine::file_cmds::place_bytes(&mut inline, "logo.svg", SVG.as_bytes().to_vec(), None, &params).unwrap();

    let mut s = new_doc();
    s.set_cooperative_jobs(true);
    s.set_job_step_budget(std::time::Duration::ZERO);
    let Started::Job(id) = s.start_place_bytes("logo.svg", SVG.as_bytes().to_vec(), None, params).unwrap() else { panic!("expected a job") };
    let mut polls = 0;
    let event = loop {
        polls += 1;
        assert!(polls < 1000, "the place never ended");
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            break e;
        }
    };
    assert!(polls > 1, "{polls}");
    let JobOutcome::Done(mut got) = event.outcome else { panic!("{:?}", event.outcome) };
    let mut want = want;
    assert!(got.as_object_mut().and_then(|o| o.remove("layer")).is_some() && want.as_object_mut().and_then(|o| o.remove("layer")).is_some());
    assert_eq!(got, want);
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers.last().unwrap();
    assert!(matches!(layer.content, LayerContent::Smart(_)));
    let surf = layer.surface().expect("rendered");
    assert!(alpha(surf, 200, 148) < 0.01 && alpha(surf, 200, 151) > 0.99, "a pixel-exact top edge");
    assert!(red(surf, 200, 192) < 0.02, "the thin line is a crisp band: {}", red(surf, 200, 192));
    let region = Rect::new(90, 140, 310, 260);
    let inline_doc = inline.active().unwrap().doc.clone();
    assert_eq!(surf.read_region(region), inline_doc.layers.last().unwrap().surface().unwrap().read_region(region), "same pixels as the inline place");
}
