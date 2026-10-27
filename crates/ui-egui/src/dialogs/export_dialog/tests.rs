#[test]
fn ico_export_is_available_and_rejects_oversize_output() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let mut f = Map::new();
    f.insert("format".into(), json!("ico"));
    f.insert("scale".into(), json!(100));
    assert!(FORMATS.iter().any(|(ext, _)| *ext == "ico"));
    assert!(export_document(&app.session.active().unwrap().doc, &f, None).err().is_some_and(|e| e.contains("256")));
    f.insert("scale".into(), json!(50));
    let resized = export_document(&app.session.active().unwrap().doc, &f, None).unwrap();
    assert_eq!((resized.size.width, resized.size.height), (150, 100));
}

use super::*;

/// Export As reopens with the settings it was used with last (Preferences ▸ remembered
/// dialog settings), and a value no control accepts keeps its default.
#[test]
fn export_as_opens_with_the_settings_used_last() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.session.prefs.edit(|p| {
        p.dialogs.insert(
            COMMAND.into(),
            json!({"format": "jpg", "quality": 88, "scale": 37, "transparency": false, "metadata": "all", "tgaBits": "24", "lossless": true}),
        );
    });
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["format"], "jpg");
    assert_eq!(f["quality"], json!(88.0));
    assert_eq!(f["scale"], json!(37.0));
    assert_eq!(f["transparency"], false);
    assert_eq!(f["metadata"], "all");
    assert_eq!(f["tgaBits"], "24");

    app.session.prefs.edit(|p| {
        p.dialogs.insert(COMMAND.into(), json!({"format": "nope", "scale": 0, "metadata": "sometimes"}));
    });
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["format"], "png");
    assert_eq!(f["scale"], 100);
    assert_eq!(f["metadata"], "none");
}

/// A recorded Export As step replays only with a destination and a format Export As writes
/// (#7207); anything else is an error, not a guess.
#[test]
fn a_replayed_export_needs_a_path_and_a_known_format() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(replay(&mut app, &json!({"path": "/x.png"})).unwrap_err().contains("document"));
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    for bad in [json!({}), json!({"path": ""}), json!("nope"), json!({"path": 3})] {
        assert!(replay(&mut app, &bad).unwrap_err().contains("path"), "{bad}");
    }
    for format in [json!("bmp"), json!(7)] {
        assert!(replay(&mut app, &json!({"path": "/x.bmp", "format": format})).unwrap_err().contains("format"), "{format}");
    }
    // A setting of the wrong type or out of the dialog's range is an error, not its default.
    for (key, v) in [
        ("scale", json!("50")),
        ("scale", json!(5000)),
        ("quality", json!(0)),
        ("transparency", json!("yes")),
        ("metadata", json!("some")),
        ("width", json!(-3)),
    ] {
        let mut step = json!({"path": "/x.png", "format": "png"});
        step[key] = v.clone();
        assert!(replay(&mut app, &step).unwrap_err().contains(key), "{key}: {v}");
    }
}

/// Layer › Export As opens with the layer's trimmed size, not the document's: `open` seeds the
/// size fields from the document, and the layer replaces them in scale mode. A remembered
/// percent scales the layer; remembered exact pixels stay the size the user asked for.
#[test]
fn layer_export_as_opens_with_the_layer_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 2, "width": 10, "height": 6})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["__w"].as_f64(), f["__h"].as_f64()), (Some(10.0), Some(6.0)));
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(10.0), Some(6.0)));

    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"scale": 50})));
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(5.0), Some(3.0)));

    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 120, "height": 80})));
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(120.0), Some(80.0)));
}

/// Only the dialog's own settings are remembered: not the destination, the document size or
/// the layer the dialog was opened with.
#[test]
fn remembering_keeps_only_the_dialog_settings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut f = Map::new();
    f.insert("format".into(), json!("webp"));
    f.insert("scale".into(), json!(50));
    f.insert("path".into(), json!("/tmp/x.webp"));
    f.insert("__w".into(), json!(1200));
    f.insert("__layer".into(), json!(3));
    remember(&mut app, &f);
    let saved = app.session.prefs().dialogs.get(COMMAND).cloned().unwrap();
    assert_eq!(saved["format"], "webp");
    assert_eq!(saved["scale"], 50);
    assert!(saved.get("path").is_none() && saved.get("__w").is_none() && saved.get("__layer").is_none());
}

/// A red/blue checkerboard at `depth`, one pixel per square.
fn checker_doc(depth: photocraft_doc::SampleType) -> Document {
    use photocraft_doc::{Color, ColorMode, Size};
    let mut doc = Document::with_background("checker", Size::new(5, 3), ColorMode::Rgb, depth, Color::WHITE);
    let b = doc.bounds();
    let surf = doc.layers[0].surface_mut().unwrap();
    let fmt = surf.format();
    let mut data = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            data.extend(photocraft_raster::from_rgba(&fmt, checker(x, y)));
        }
    }
    surf.write_region(b, &data);
    doc
}

fn checker(x: i32, y: i32) -> [f32; 4] {
    if (x + y) % 2 == 0 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] }
}

/// Export As › Resample › Nearest Neighbor at 200 %: every source pixel becomes an exact
/// 2 × 2 block, at 8 and 16 bits, and the document keeps its pixels. Bicubic Automatic (the
/// default) blends the squares instead.
#[test]
fn nearest_neighbor_doubles_each_pixel_into_an_exact_block() {
    use photocraft_doc::SampleType;
    for depth in [SampleType::U8, SampleType::U16] {
        let doc = checker_doc(depth);
        let before = doc.clone();
        let mut f = Map::new();
        f.insert("format".into(), json!("png"));
        f.insert("scale".into(), json!(200));
        f.insert("resample".into(), json!("nearest"));
        let out = export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (10, 6));
        assert_eq!(out.depth, depth);
        let surf = out.layers[0].surface().unwrap();
        for y in 0..6 {
            for x in 0..10 {
                assert_eq!(surf.rgba(x, y), checker(x / 2, y / 2), "{depth:?} pixel {x},{y}");
            }
        }
        assert_eq!(doc, before, "{depth:?}: export leaves the source document unchanged");

        // The preview proxy keeps Nearest Neighbor's hard edges too.
        let proxy = export_document(&doc, &f, Some(512)).unwrap();
        assert_eq!(proxy.layers[0].surface().unwrap().rgba(1, 1), checker(0, 0), "{depth:?}: preview proxy is nearest as well");
        assert_eq!(preview_filter(&f), egui::TextureOptions::NEAREST);

        f.insert("resample".into(), json!(DEFAULT_RESAMPLE));
        let smooth = export_document(&doc, &f, None).unwrap();
        let blended = (0..6).flat_map(|y| (0..10).map(move |x| (x, y))).any(|(x, y)| {
            let p = smooth.layers[0].surface().unwrap().rgba(x, y);
            p[0] > 0.01 && p[2] > 0.01
        });
        assert!(blended, "{depth:?}: Bicubic Automatic blends neighbouring squares");
        assert_eq!(preview_filter(&f), egui::TextureOptions::LINEAR);
    }
}

/// Every Resample choice reaches Image Size; a method Export As does not offer is an error
/// naming the choices, never a silent Bicubic, and a missing one is Bicubic Automatic.
#[test]
fn resample_choices_are_checked() {
    let doc = checker_doc(photocraft_doc::SampleType::U8);
    let mut f = Map::new();
    f.insert("scale".into(), json!(300));
    for (method, _) in RESAMPLE {
        f.insert("resample".into(), json!(method));
        let out = export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (15, 9), "{method}");
    }
    for bad in [json!("lanczos"), json!("NEAREST"), json!(""), json!(1), json!({"m": "nearest"})] {
        f.insert("resample".into(), bad.clone());
        let err = export_document(&doc, &f, None).unwrap_err();
        assert!(err.contains("resample must be"), "{bad}: {err}");
        assert!(export_document(&doc, &f, Some(512)).is_err(), "{bad}: the preview reports it too");
    }
    f.remove("resample");
    assert_eq!(resample_method(&f, None).unwrap(), "bicubicAutomatic");
    assert_eq!(resample_method(&f, Some(512)).unwrap(), "bilinear", "the preview stays on the fast path");
}

/// Resample is remembered with the other settings; a saved value the menu does not offer
/// opens the dialog with the default.
#[test]
fn resample_is_remembered_and_a_bad_saved_value_falls_back() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    let id = open(&mut app).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["resample"], DEFAULT_RESAMPLE, "Photoshop's default");
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    f.insert("resample".into(), json!("nearest"));
    remember(&mut app, &f);
    assert_eq!(app.session.prefs().dialogs[COMMAND]["resample"], "nearest");
    let id = open(&mut app).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["resample"], "nearest");

    for bad in [json!("lanczos"), json!(7), json!(null)] {
        app.session.prefs.edit(|p| {
            p.dialogs.insert(COMMAND.into(), json!({"resample": bad}));
        });
        let id = open(&mut app).unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["resample"], DEFAULT_RESAMPLE, "saved {bad}");
    }
}

#[test]
fn export_document_scales_height_when_rounded_width_is_unchanged() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (width, height, scale, expected) in [
            (1, 100, 50, (1, 50)),
            (1, 200, 50, (1, 100)),
            (10, 100, 99, (10, 99)),
            (10, 100, 101, (10, 101)),
            (400, 300, 50, (200, 150)),
            (400, 300, 100, (400, 300)),
        ] {
            let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, depth, Color::WHITE);
            let before = doc.clone();
            let mut f = Map::new();
            f.insert("format".into(), json!("png"));
            f.insert("scale".into(), json!(scale));
            let out = export_document(&doc, &f, None).unwrap();
            assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, {depth:?}");
            assert_eq!(out.depth, depth, "export preserves the document's depth");
            assert_eq!(doc, before, "export leaves the source document unchanged");
        }
    }
}

#[test]
fn export_document_proxy_limits_both_dimensions_before_rounding() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};

    for (width, height, scale, max_side, expected) in
        [(1, 1000, 100, 360, (1, 360)), (1, 1000, 50, 360, (1, 360)), (1, 1000, 25, 360, (1, 250)), (13, 20, 50, 8, (5, 8)), (20, 13, 50, 8, (8, 5))]
    {
        let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let before = doc.clone();
        let mut f = Map::new();
        f.insert("format".into(), json!("png"));
        f.insert("scale".into(), json!(scale));
        let out = export_document(&doc, &f, Some(max_side)).unwrap();
        assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, proxy limit {max_side}");
        assert!(out.size.width.max(out.size.height) <= max_side, "proxy respects its longest-side limit");
        assert_eq!(doc, before, "preview leaves the source document unchanged");
    }
}

/// #6595: Export As shows the size the file will have, as Photoshop does. The proxy estimate
/// read 36–81 % of it for a detailed image; up to `EXACT_ESTIMATE_PX` the full export is
/// encoded off the UI thread instead.
#[test]
fn the_estimate_is_the_exported_files_size() {
    let mut doc = Document::with_background(
        "x",
        photocraft_doc::Size::new(900, 300),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let id = doc.layers[0].id;
    let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
    let mut seed = 12345u32;
    for y in (0..300).step_by(2) {
        for x in (0..900).step_by(2) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (seed >> 24) as f32 / 255.0;
            surf.fill_rect(photocraft_geom::Rect::new(x, y, x + 2, y + 2), &[v, x as f32 / 900.0, y as f32 / 300.0, 1.0]);
        }
    }
    let doc = Arc::new(doc);
    let f = Map::from_iter([("format".into(), json!("jpg")), ("scale".into(), json!(100)), ("quality".into(), json!(32))]);
    let actual = photocraft_io::export(&export_document(&doc, &f, None).unwrap(), "x.jpg", &settings(&f).io_options()).unwrap().bytes.len() as u64;
    let proxy = preview_proxy(&doc, &f).unwrap();
    let old = estimate_native(&doc, &proxy, &f).unwrap();
    assert!((old as f64 / actual as f64 - 1.0).abs() > 0.1, "the proxy estimate was off: {old} vs {actual}");

    let ctx = egui::Context::default();
    let key = egui::Id::unique("exact-estimate-test");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let size = loop {
        let mut size = None;
        let mut output = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| size = poll_or_start_exact_estimate(ui, key, "jpg32", doc.clone(), &f));
        });
        output.textures_delta.clear();
        if let Some(size) = size {
            break size;
        }
        assert!(std::time::Instant::now() < deadline, "the exact estimate did not arrive");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(size, Some(actual));
    // Over the limit there is no exact size; the proxy estimate stays.
    let huge = Map::from_iter([
        ("format".into(), json!("jpg")),
        ("sizing".into(), json!("pixels")),
        ("width".into(), json!(10_000)),
        ("height".into(), json!(10_000)),
    ]);
    assert_eq!(exact_estimate(&doc, &huge), None);
    // In scale mode the size comes from the scale, as the export's does, not from stale pixels.
    let big = Arc::new(Document::with_background(
        "big",
        photocraft_doc::Size::new(1000, 1000),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    ));
    let stale = Map::from_iter([
        ("format".into(), json!("png")),
        ("sizing".into(), json!("scale")),
        ("scale".into(), json!(1000)),
        ("width".into(), json!(100)),
        ("height".into(), json!(100)),
    ]);
    assert_eq!(exact_estimate(&big, &stale), None, "10000 × 10000 at 1000 %, over the limit");
    let small = Map::from_iter([
        ("format".into(), json!("png")),
        ("sizing".into(), json!("scale")),
        ("scale".into(), json!(10)),
        ("width".into(), json!(9000)),
        ("height".into(), json!(9000)),
    ]);
    let at_10 = photocraft_io::export(&export_document(&big, &small, None).unwrap(), "x.png", &settings(&small).io_options()).unwrap().bytes.len() as u64;
    assert_eq!(exact_estimate(&big, &small), Some(at_10), "100 × 100 at 10 %, within it");
}

/// #8341: Export As and its reduced preview carry the stroke width with the path.
#[test]
fn export_as_downscales_stroke_only_shapes_and_preview() {
    for depth in [8, 16, 32] {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 1024, "height": 1024, "depth": depth, "background": "transparent"})).unwrap();
        let id =
            session.execute("shape.create", json!({"rect": [256, 256, 512, 512], "fill": null, "stroke": {"width": 32}})).unwrap()["layer"].as_u64().unwrap();
        let doc = session.active().unwrap().doc.clone();
        let fields = Map::from_iter([("format".into(), json!("png")), ("scale".into(), json!(25))]);
        for limit in [None, Some(128)] {
            let out = export_document(&doc, &fields, limit).unwrap();
            let side = if limit.is_some() { 128 } else { 256 };
            assert_eq!((out.size.width, out.size.height), (side, side));
            let shape = out.layer(photocraft_doc::LayerId(id)).unwrap();
            let photocraft_doc::LayerContent::Shape(shape) = &shape.content else {
                panic!("shape lost");
            };
            assert_eq!(shape.stroke.as_ref().unwrap().width, side as f32 / 32.0);
            let pixels = photocraft_compose::flatten(&out);
            let row = (side / 2 * side) as usize;
            assert!(pixels.px[row + (side / 4) as usize][3] > 0.99);
            assert!(pixels.px[row + (side / 4 - side / 32) as usize][3] < 0.01);
            assert!(pixels.px[row + (side / 2) as usize][3] < 0.01);
        }
        assert_eq!(session.active().unwrap().doc, doc, "export leaves the source unchanged");
    }
}

#[test]
fn export_document_scales_and_flattens() {
    let doc = Document::with_background(
        "x",
        photocraft_doc::Size::new(200, 100),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let mut f = Map::new();
    f.insert("format".into(), json!("jpg"));
    f.insert("scale".into(), json!(50));
    let out = export_document(&doc, &f, None).unwrap();
    assert_eq!((out.size.width, out.size.height), (100, 50));
    assert_eq!(out.layers.len(), 1);
    let proxy = export_document(&doc, &f, Some(40)).unwrap();
    assert_eq!(proxy.size.width, 40);
    assert_eq!(settings(&f).jpeg_quality, Some(85));
}

/// #1353: a scale that changes only the height (the width rounds back to the original)
/// used to be skipped, so the exported file kept the original size while the dialog
/// labelled it as scaled.
#[test]
fn export_document_scales_when_only_the_height_changes() {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("scale".into(), json!(50));
    for (src_h, want_h) in [(100u32, 50u32), (200, 100)] {
        let doc = Document::with_background(
            "x",
            photocraft_doc::Size::new(1, src_h),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let out = export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (1, want_h), "1 × {src_h} at 50%");
        // The preview proxy takes the same path.
        let proxy = export_document(&doc, &f, Some(40)).unwrap();
        assert_eq!((proxy.size.width, proxy.size.height), (1, 40), "proxy of 1 × {src_h}");
    }
    // The label and the export agree, and neither side drops below 1 px.
    assert_eq!(scaled_size(1.0, 100.0, 0.5), (1.0, 50.0));
    assert_eq!(scaled_size(10.0, 300.0, 0.01), (1.0, 3.0));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn layered_preview_runs_outside_the_ui_frame() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 16})).unwrap();
    for _ in 0..33 {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    let doc = app.session.active().unwrap().doc.clone();
    assert!(background_preview_needed(&doc));
    let fields = Map::from_iter([("format".into(), json!("png")), ("scale".into(), json!(100))]);
    let ctx = egui::Context::default();
    let key = egui::Id::unique("layered-preview-test");
    let mut result = None;
    let mut output = ctx.run_ui(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            result = poll_or_start_preview(ui, key, "png100", doc.clone(), &fields);
        });
    });
    output.textures_delta.clear();
    assert!(result.is_none(), "the first UI frame must only start the worker");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut output = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = poll_or_start_preview(ui, key, "png100", doc.clone(), &fields);
            });
        });
        output.textures_delta.clear();
        if let Some(preview) = result {
            let preview = preview.unwrap();
            assert_eq!((preview.width, preview.height), (32, 16));
            assert_eq!(preview.rgba.len(), 32 * 16 * 4);
            assert!(preview.estimate.is_some_and(|bytes| bytes > 0));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "background preview did not complete");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let tiff_fields = Map::from_iter([("format".into(), json!("tif")), ("scale".into(), json!(100))]);
    let proxy = preview_proxy(&doc, &tiff_fields).unwrap();
    let options = photocraft_io::ExportOptions { tiff_layers: true, xmp: photocraft_io::XmpEmbed::None, ..Default::default() };
    let actual_bytes = photocraft_io::export(&proxy, "estimate.tif", &options).unwrap().bytes.len();
    assert_eq!(estimate_native(&doc, &proxy, &tiff_fields), Some(scaled_estimate(actual_bytes, &doc, &proxy, &tiff_fields)));
}

#[test]
fn switching_export_formats_uses_independent_quality_preferences() {
    let prefs = photocraft_engine::prefs::Export { jpeg_quality: 43, webp_quality: 72, webp_lossless: false, ..Default::default() };
    let mut fields = Map::new();
    set_format_defaults(&mut fields, "webp", &prefs);
    assert_eq!(fields["quality"], 72);
    assert_eq!(fields["lossless"], false);
    let s = settings(&fields);
    assert!(!s.webp_lossless);
    assert_eq!(s.webp_quality, Some(72));
    assert_eq!(s.jpeg_quality, None);

    set_format_defaults(&mut fields, "jpg", &prefs);
    assert_eq!(fields["quality"], 43);
    assert_eq!(settings(&fields).jpeg_quality, Some(43));
    assert_eq!(settings(&fields).webp_quality, None);

    set_format_defaults(&mut fields, "webp", &photocraft_engine::prefs::Export::default());
    assert!(settings(&fields).webp_lossless, "default WebP Quick Export mode is lossless");
    assert_eq!(settings(&fields).webp_quality, None);
}

#[test]
fn webp_settings_follow_the_lossless_switch() {
    let mut f = Map::new();
    f.insert("format".into(), json!("webp"));
    f.insert("quality".into(), json!(70));
    let s = settings(&f);
    assert!(!s.webp_lossless, "Export As writes lossy WebP unless asked");
    assert_eq!(s.webp_quality, Some(70));
    assert_eq!(s.jpeg_quality, None);
    f.insert("lossless".into(), json!(true));
    let s = settings(&f);
    assert!(s.webp_lossless);
    assert_eq!(s.webp_quality, None);
    f.insert("format".into(), json!("png"));
    assert!(settings(&f).webp_lossless, "other formats leave the WebP default alone");
}

/// TGA's Resolution (bits per pixel): Auto leaves the choice to the export; 24 and 32 force it, and only
/// for TGA.
#[test]
fn tga_settings_follow_the_bits_per_pixel() {
    use photocraft_io::TgaBits;
    let mut f = Map::new();
    f.insert("format".into(), json!("tga"));
    assert_eq!(settings(&f).tga_bits, None, "no choice made");
    for (v, want) in [("auto", None), ("24", Some(TgaBits::Bits24)), ("32", Some(TgaBits::Bits32)), ("16", None)] {
        f.insert("tgaBits".into(), json!(v));
        assert_eq!(settings(&f).tga_bits, want, "{v}");
    }
    f.insert("format".into(), json!("png"));
    assert_eq!(settings(&f).tga_bits, None, "only TGA has bits per pixel");
}

#[test]
fn exact_pixels_lock_aspect_and_leave_the_source() {
    let linked = resize_from_scale(400.0, 200.0, 50.0).unwrap();
    assert_eq!((linked.width, linked.height, linked.scale), (200.0, 100.0, 50.0));
    let from_width = resize_from_side(400.0, 200.0, 100.0, 200.0, true, true).unwrap();
    assert_eq!((from_width.width, from_width.height), (100.0, 50.0));
    let from_height = resize_from_side(400.0, 200.0, 100.0, 40.0, true, false).unwrap();
    assert_eq!((from_height.width, from_height.height), (80.0, 40.0));
    let free = resize_from_side(400.0, 200.0, 100.0, 80.0, false, true).unwrap();
    assert_eq!((free.width, free.height), (100.0, 80.0));
    assert!(resize_from_scale(400.0, 200.0, 0.0).is_err());
    assert!(resize_from_side(400.0, 200.0, 0.0, 10.0, false, true).is_err());
    assert!(resize_from_side(10.0, 10.0, 400_000.0, 10.0, false, true).is_err());

    let doc = Document::with_background(
        "x",
        photocraft_doc::Size::new(400, 200),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let before = doc.clone();
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("scale".into(), json!(100));
    f.insert("sizing".into(), json!("pixels"));
    f.insert("width".into(), json!(120));
    f.insert("height".into(), json!(40));
    let out = export_document(&doc, &f, None).unwrap();
    assert_eq!((out.size.width, out.size.height), (120, 40), "explicit pixels win over the percent scale");
    let proxy = export_document(&doc, &f, Some(20)).unwrap();
    assert_eq!((proxy.size.width, proxy.size.height), (20, 7));
    assert_eq!(doc, before, "export leaves the source document unchanged");
}

#[test]
fn profile_and_metadata_follow_the_existing_exporter() {
    use photocraft_cms::Builtin;

    let mut f = Map::new();
    f.insert("format".into(), json!("jpg"));
    assert!(settings(&f).embed_metadata, "a missing metadata choice keeps EXIF and resolution");
    assert!(settings(&f).embed_icc);
    f.insert("metadata".into(), json!("none"));
    f.insert("embedProfile".into(), json!(false));
    let none = settings(&f);
    assert!(!none.xmp_all && !none.embed_metadata && !none.embed_icc);
    f.insert("metadata".into(), json!("exif"));
    let exif = settings(&f);
    assert!(!exif.xmp_all && exif.embed_metadata);
    f.insert("metadata".into(), json!("all"));
    f.insert("embedProfile".into(), json!(true));
    let all = settings(&f);
    assert!(all.xmp_all && all.embed_metadata && all.embed_icc);

    let mut doc = Document::with_background(
        "x",
        photocraft_doc::Size::new(8, 8),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
    doc.metadata.xmp = Some("<x:xmpmeta>secret</x:xmpmeta>".into());
    let before = doc.clone();
    f.insert("profile".into(), json!("srgb"));
    f.insert("width".into(), json!(8));
    f.insert("height".into(), json!(8));
    f.insert("metadata".into(), json!("none"));
    f.insert("embedProfile".into(), json!(false));
    let converted = export_document(&doc, &f, None).unwrap();
    let parsed = photocraft_cms::Profile::parse(converted.icc_profile.as_deref().unwrap()).unwrap();
    assert!(parsed.same_colors(Builtin::Srgb.profile()), "Convert to sRGB replaces the document profile");
    let stripped = photocraft_io::export(&converted, "out.jpg", &settings(&f).io_options()).unwrap().bytes;
    assert!(!stripped.windows(b"ICC_PROFILE".len()).any(|w| w == b"ICC_PROFILE"));
    assert!(!stripped.windows(b"secret".len()).any(|w| w == b"secret"));
    f.insert("metadata".into(), json!("all"));
    f.insert("embedProfile".into(), json!(true));
    let kept = photocraft_io::export(&converted, "out.jpg", &settings(&f).io_options()).unwrap().bytes;
    assert!(kept.windows(b"ICC_PROFILE".len()).any(|w| w == b"ICC_PROFILE"));
    assert!(kept.windows(b"secret".len()).any(|w| w == b"secret"));
    assert_eq!(doc, before);
}

#[test]
fn applying_a_preset_restores_its_size_mode() {
    use photocraft_engine::export_preset_cmds::ExportPresetSettings;

    let pixels = ExportPresetSettings {
        format: "png".into(),
        quality: 80,
        lossless: false,
        transparency: true,
        scale: 100.0,
        width: 64,
        height: 32,
        lock_aspect: false,
        sizing: "pixels".into(),
        metadata: "exif".into(),
        profile: "document".into(),
        embed_profile: false,
        tga_bits: "auto".into(),
        resample: "nearest".into(),
    };
    let mut f = Map::new();
    apply_preset(&mut f, &pixels, 400.0, 200.0);
    assert_eq!(f.get("resample"), Some(&json!("nearest")), "a preset restores its Resample method");
    assert_eq!(preset_settings(&f)["resample"], "nearest", "and saves it");
    assert_eq!(f.get("width"), Some(&json!(64)));
    assert_eq!(f.get("height"), Some(&json!(32)));
    assert_eq!(f.get("sizing"), Some(&json!("pixels")));
    assert_eq!(f.get("embedProfile"), Some(&json!(false)));
    let scale = ExportPresetSettings { sizing: "scale".into(), scale: 25.0, ..pixels };
    apply_preset(&mut f, &scale, 400.0, 200.0);
    assert_eq!(f.get("width"), Some(&json!(100.0)));
    assert_eq!(f.get("height"), Some(&json!(50.0)));
    assert_eq!(f.get("sizing"), Some(&json!("scale")));
}
