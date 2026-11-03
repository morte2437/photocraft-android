#[test]
fn ico_export_is_available_and_rejects_oversize_output() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let mut f = Map::new();
    f.insert("format".into(), json!("ico"));
    f.insert("scale".into(), json!(100));
    assert!(FORMATS.iter().any(|(ext, _)| *ext == "ico"));
    assert!(default_export_document(&app.session.active().unwrap().doc, &f, None).err().is_some_and(|e| e.contains("256")));
    f.insert("scale".into(), json!(50));
    let resized = default_export_document(&app.session.active().unwrap().doc, &f, None).unwrap();
    assert_eq!((resized.size.width, resized.size.height), (150, 100));
}

use super::*;

fn default_export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>) -> Result<Document, String> {
    super::export_document(doc, f, max_side, &photocraft_engine::color_cmds::document_profile(doc))
}

fn default_preview_proxy(doc: &Document, f: &Map<String, Value>) -> Result<Document, String> {
    super::preview_proxy(doc, f, &photocraft_engine::color_cmds::document_profile(doc))
}

#[cfg(not(target_arch = "wasm32"))]
fn default_exact_estimate(doc: &Document, f: &Map<String, Value>) -> Option<u64> {
    super::exact_estimate(doc, f, &photocraft_engine::color_cmds::document_profile(doc))
}

#[cfg(not(target_arch = "wasm32"))]
fn default_poll_or_start_preview(
    ui: &mut egui::Ui,
    key: egui::Id,
    signature: &str,
    doc: Arc<Document>,
    fields: &Map<String, Value>,
) -> Option<Result<PreviewPixels, String>> {
    let profile = photocraft_engine::color_cmds::document_profile(&doc);
    super::poll_or_start_preview(ui, key, signature, doc, fields, profile)
}

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
/// size fields to the document, and the layer refits them. A remembered percent scales the
/// layer; remembered exact pixels stay the size the user asked for, in the layer's proportions
/// when they are linked.
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

    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 120, "height": 80, "lockAspect": false})));
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(120.0), Some(80.0)));

    // With Constrain proportions the remembered width stays and the height follows the layer.
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 50, "height": 80, "lockAspect": true})));
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(50.0), Some(30.0)));
    assert_eq!(f["sizing"], "pixels");
}

/// #8738: a pixel size remembered from a landscape export, with Constrain proportions on,
/// keeps its width and takes the height from the portrait image being exported, as ticking
/// the checkbox does; unlinked, both sides are the exact size asked for. A recorded action
/// step is not the remembered size: it replays with exactly the pixels it was recorded with.
#[test]
fn a_remembered_pixel_size_keeps_this_images_proportions() {
    let sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = sizes.clone();
    let services = crate::Services {
        export: Some(Box::new(move |doc, _, _| {
            log.borrow_mut().push((doc.size.width, doc.size.height));
            Ok((Vec::new(), Vec::new()))
        })),
        write: Some(Box::new(|_, _| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 30, "height": 40})).unwrap();
    let landscape = json!({"sizing": "pixels", "width": 20, "height": 15, "scale": 50, "lockAspect": true});
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), landscape.clone()));
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(20.0), Some(27.0)));
    assert_eq!(f["sizing"], "pixels");
    assert!((f["scale"].as_f64().unwrap() - 200.0 / 3.0).abs() < 1e-9, "{}", f["scale"]);

    let mut unlinked = landscape.clone();
    unlinked["lockAspect"] = json!(false);
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), unlinked));
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(20.0), Some(15.0)));

    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), landscape.clone()));
    let mut step = landscape;
    step["path"] = json!("/x.png");
    step["format"] = json!("png");
    replay(&mut app, &step).unwrap();
    assert_eq!(*sizes.borrow(), [(20, 15)]);
}

/// A remembered linked width this image can't take within the Scale field's 1..=1000 %
/// (2000 px from a 4000 px photo, on a 100 px image) falls back to the remembered percent.
#[test]
fn a_remembered_pixel_size_out_of_scale_falls_back_to_the_percent() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 2000, "height": 1500, "scale": 50, "lockAspect": true})));
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(50.0), Some(50.0)));
    assert_eq!((f["sizing"].as_str(), f["scale"].as_f64()), (Some("scale"), Some(50.0)));
}

/// A remembered linked width whose height rounds below one pixel keeps its width: the height
/// is one pixel, as the Scale field gives at the same 10 %, and not the remembered percent.
#[test]
fn a_remembered_pixel_width_keeps_a_one_pixel_height() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 1})).unwrap();
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 100, "height": 75, "scale": 100, "lockAspect": true})));
    let id = open(&mut app).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(100.0), Some(1.0)));
    assert_eq!(f["sizing"], "pixels");
    // The same for an edited height on a 1 × 1000 image.
    let size = resize_from_side(1.0, 1000.0, 1.0, 100.0, true, false).unwrap();
    assert_eq!((size.width, size.height), (1.0, 100.0));
}

/// Layer › Export As fits the remembered size to the layer, not to the document: a remembered
/// linked width the tall document can't take (800 px wide is 320 000 px high) still fits the
/// 100 × 10 layer at 800 × 80.
#[test]
fn a_layer_export_fits_the_remembered_size_to_the_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 40000})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 10})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"sizing": "pixels", "width": 800, "height": 600, "scale": 50, "lockAspect": true})));
    let id = open_layer(&mut app, layer).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(800.0), Some(80.0)));
    assert_eq!(f["sizing"], "pixels");
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
        let out = default_export_document(&doc, &f, None).unwrap();
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
        let proxy = default_export_document(&doc, &f, Some(512)).unwrap();
        assert_eq!(proxy.layers[0].surface().unwrap().rgba(1, 1), checker(0, 0), "{depth:?}: preview proxy is nearest as well");
        assert_eq!(preview_filter(&f), egui::TextureOptions::NEAREST);

        f.insert("resample".into(), json!(DEFAULT_RESAMPLE));
        let smooth = default_export_document(&doc, &f, None).unwrap();
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
        let out = default_export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (15, 9), "{method}");
    }
    for bad in [json!("lanczos"), json!("NEAREST"), json!(""), json!(1), json!({"m": "nearest"})] {
        f.insert("resample".into(), bad.clone());
        let err = default_export_document(&doc, &f, None).unwrap_err();
        assert!(err.contains("resample must be"), "{bad}: {err}");
        assert!(default_export_document(&doc, &f, Some(512)).is_err(), "{bad}: the preview reports it too");
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
            let out = default_export_document(&doc, &f, None).unwrap();
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
        let out = default_export_document(&doc, &f, Some(max_side)).unwrap();
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
    // Without the embedded profile, whose fixed size the proxy would scale with the pixels
    // (the untagged document is tagged sRGB on export, #9325).
    let f =
        Map::from_iter([("format".into(), json!("jpg")), ("scale".into(), json!(100)), ("quality".into(), json!(32)), ("embedProfile".into(), json!(false))]);
    let actual = photocraft_io::export(&default_export_document(&doc, &f, None).unwrap(), "x.jpg", &settings(&f).io_options()).unwrap().bytes.len() as u64;
    let proxy = default_preview_proxy(&doc, &f).unwrap();
    let old = estimate_native(&doc, &proxy, &f).unwrap();
    assert!((old as f64 / actual as f64 - 1.0).abs() > 0.1, "the proxy estimate was off: {old} vs {actual}");

    let ctx = egui::Context::default();
    let key = egui::Id::unique("exact-estimate-test");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let size = loop {
        let mut size = None;
        let mut output = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| size = poll_or_start_exact_estimate(ui, key, "jpg32", doc.clone(), &f, photocraft_engine::color_cmds::document_profile(&doc)));
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
    assert_eq!(default_exact_estimate(&doc, &huge), None);
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
    assert_eq!(default_exact_estimate(&big, &stale), None, "10000 × 10000 at 1000 %, over the limit");
    let small = Map::from_iter([
        ("format".into(), json!("png")),
        ("sizing".into(), json!("scale")),
        ("scale".into(), json!(10)),
        ("width".into(), json!(9000)),
        ("height".into(), json!(9000)),
    ]);
    let at_10 =
        photocraft_io::export(&default_export_document(&big, &small, None).unwrap(), "x.png", &settings(&small).io_options()).unwrap().bytes.len() as u64;
    assert_eq!(default_exact_estimate(&big, &small), Some(at_10), "100 × 100 at 10 %, within it");
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
            let out = default_export_document(&doc, &fields, limit).unwrap();
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
    let out = default_export_document(&doc, &f, None).unwrap();
    assert_eq!((out.size.width, out.size.height), (100, 50));
    assert_eq!(out.layers.len(), 1);
    let proxy = default_export_document(&doc, &f, Some(40)).unwrap();
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
        let out = default_export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (1, want_h), "1 × {src_h} at 50%");
        // The preview proxy takes the same path.
        let proxy = default_export_document(&doc, &f, Some(40)).unwrap();
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
            result = default_poll_or_start_preview(ui, key, "png100", doc.clone(), &fields);
        });
    });
    output.textures_delta.clear();
    assert!(result.is_none(), "the first UI frame must only start the worker");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut output = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = default_poll_or_start_preview(ui, key, "png100", doc.clone(), &fields);
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
    let proxy = default_preview_proxy(&doc, &tiff_fields).unwrap();
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
    let out = default_export_document(&doc, &f, None).unwrap();
    assert_eq!((out.size.width, out.size.height), (120, 40), "explicit pixels win over the percent scale");
    let proxy = default_export_document(&doc, &f, Some(20)).unwrap();
    assert_eq!((proxy.size.width, proxy.size.height), (20, 7));
    assert_eq!(doc, before, "export leaves the source document unchanged");
}

fn check_profile_only_export(width: u32, height: u32) -> std::time::Duration {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    let mut doc = Document::with_background("untagged", Size::new(width, height), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        p.copy_from_slice(&[(i % 251) as u8, ((i / 251) % 251) as u8, ((i / 63001) % 251) as u8, 255]);
    }
    doc.layers[0].surface_mut().unwrap().write_interleaved(photocraft_geom::Rect::from_xywh(0, 0, width, height), &pixels);
    let before = doc.clone();
    let fields = json!({"format": "png", "profile": "srgb", "transparency": true, "width": width, "height": height}).as_object().unwrap().clone();
    let start = std::time::Instant::now();
    let converted = default_export_document(&doc, &fields, None).unwrap();
    let elapsed = start.elapsed();
    let profile = photocraft_cms::Profile::parse(converted.icc_profile.as_deref().unwrap()).unwrap();
    assert!(profile.same_bytes(photocraft_cms::Builtin::Srgb.profile()));
    let source = doc.layers[0].surface().unwrap();
    let output = converted.layers[0].surface().unwrap();
    assert!(source.tile_count() > 0);
    assert_eq!(source.tile_count(), output.tile_count());
    assert_eq!(source.default_pixel(), output.default_pixel());
    for (coord, tile) in source.tiles() {
        assert!(Arc::ptr_eq(tile, output.tile(*coord).unwrap()), "tagging must reuse pixel tiles");
    }
    assert_eq!(doc, before);
    elapsed
}

#[test]
fn untagged_srgb_export_tags_without_rewriting_pixels() {
    check_profile_only_export(8, 8);
}

#[test]
#[ignore = "24 MP profile-preparation measurement"]
fn untagged_srgb_export_24mp_keeps_pixel_tiles() {
    let elapsed = check_profile_only_export(6000, 4000);
    println!("24 MP sRGB profile preparation (optimized test build): {elapsed:?}; all pixel tiles shared");
}

#[test]
fn untagged_jpeg_export_embeds_the_requested_srgb_profile() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    for (mode, depth) in [
        (ColorMode::Rgb, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Cmyk, SampleType::U8),
    ] {
        let doc = Document::with_background("untagged", Size::new(8, 8), mode, depth, Color::WHITE);
        assert!(doc.icc_profile.is_none());
        let before = doc.clone();
        let mut fields =
            json!({"format": "jpg", "profile": "srgb", "embedProfile": true, "metadata": "none", "width": 8, "height": 8}).as_object().unwrap().clone();
        let converted = default_export_document(&doc, &fields, None).unwrap();
        let bytes = photocraft_io::export(&converted, "out.jpg", &settings(&fields).io_options()).unwrap().bytes;
        let reloaded = photocraft_io::import("out.jpg", &bytes).unwrap().document;
        let profile = photocraft_cms::Profile::parse(reloaded.icc_profile.as_deref().expect("explicit sRGB export must embed its profile")).unwrap();
        assert!(profile.same_colors(photocraft_cms::Builtin::Srgb.profile()), "{depth:?}");
        fields.insert("embedProfile".into(), json!(false));
        let bytes = photocraft_io::export(&converted, "out.jpg", &settings(&fields).io_options()).unwrap().bytes;
        assert!(photocraft_io::import("out.jpg", &bytes).unwrap().document.icc_profile.is_none(), "embedding remains optional");
        fields.insert("profile".into(), json!("document"));
        fields.insert("embedProfile".into(), json!(true));
        let unchanged = default_export_document(&doc, &fields, None).unwrap();
        assert!(unchanged.icc_profile.is_none(), "Document Profile preserves an untagged source");
        assert_eq!(doc, before, "export must not tag or convert the source document");
    }
}

#[test]
fn preview_cache_tracks_working_profiles_but_prefers_embedded_profiles() {
    use photocraft_cms::Builtin;
    use photocraft_doc::{Color, SampleType, Size};
    for embedded in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("edit.colorSettings", json!({"workingRgb": "display-p3", "policyRgb": "off"})).unwrap();
        let mut doc = Document::with_background("colour", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(0.35, 0.65, 0.45, 1.0));
        if embedded {
            doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        }
        app.session.open_document(doc, None);
        let id = open(&mut app).unwrap();
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("profile".into(), json!("srgb"));
        let st = app.session.active().unwrap();
        let revision = st.revision;
        let key = egui::Id::unique(("export-preview", st.doc.id.0, revision, None::<u64>));
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        let draw = |app: &mut PhotocraftApp, fields: &mut Map<String, Value>| {
            let mut output = ctx.run_ui(Default::default(), |ui| body(app, ui, fields));
            output.textures_delta.clear();
            ctx.data(|d| d.get_temp::<CachedPreview>(key)).unwrap()
        };
        let before = draw(&mut app, &mut fields);
        let settings = settings_of(&fields);
        app.session.execute("edit.colorSettings", json!({"workingRgb": "adobe-rgb-compat"})).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision);
        let after = draw(&mut app, &mut fields);
        assert_eq!(settings_of(&fields), settings);
        assert_eq!(before.signature != after.signature, !embedded);
        assert_eq!(before.texture.unwrap().id() != after.texture.unwrap().id(), !embedded);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn preview_workers_keep_the_captured_source_profile() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("edit.colorSettings", json!({"workingRgb": "display-p3", "policyRgb": "off"})).unwrap();
    let doc = Document::with_background(
        "colour",
        photocraft_doc::Size::new(8, 8),
        ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::rgba(0.35, 0.65, 0.45, 1.0),
    );
    app.session.open_document(doc, None);
    let doc = app.session.active().unwrap().doc.clone();
    let profile = effective_source_profile(&app, &doc);
    let fields = json!({"format": "png", "profile": "srgb", "transparency": true}).as_object().unwrap().clone();
    let expected = preview_sync(&app, &doc, &fields, &profile).unwrap();
    let expected_size = super::exact_estimate(&doc, &fields, &profile).unwrap();
    let ctx = egui::Context::default();
    let key = egui::Id::unique("captured-profile-worker");
    let mut preview = None;
    let mut size = None;
    let mut poll = || {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            if preview.is_none() {
                preview = super::poll_or_start_preview(ui, key, "p3", doc.clone(), &fields, profile.clone());
            }
            if size.is_none() {
                size = poll_or_start_exact_estimate(ui, key, "p3", doc.clone(), &fields, profile.clone());
            }
        });
        output.textures_delta.clear();
        preview.is_some() && size.is_some()
    };
    poll();
    app.session.execute("edit.colorSettings", json!({"workingRgb": "srgb"})).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !poll() {
        assert!(std::time::Instant::now() < deadline, "profile-aware preview workers must finish");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(preview.unwrap().unwrap().rgba, expected.rgba);
    assert_eq!(size.unwrap(), Some(expected_size));
}

#[test]
fn replay_converts_untagged_pixels_from_the_original_working_profile() {
    use photocraft_cms::{Builtin, Intent};
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    for working in [Builtin::DisplayP3, Builtin::AdobeRgbCompat] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.session.execute("edit.colorSettings", json!({"workingRgb": working.id(), "policyRgb": "off"})).unwrap();
            let doc = Document::with_background("untagged", Size::new(8, 8), ColorMode::Rgb, depth, Color::rgba(0.35, 0.65, 0.45, 1.0));
            app.session.open_document(doc, None);
            let source = app.session.active().unwrap().doc.clone();
            assert!(source.icc_profile.is_none());
            let mut expected = (*source).clone();
            expected.icc_profile = Some(working.profile().to_bytes());
            photocraft_engine::color_cmds::convert_document(&mut expected, Builtin::Srgb.profile(), Intent::Perceptual, true).unwrap();
            let original_pixel = source.layers[0].surface().unwrap().pixel(0, 0);
            let expected_pixel = expected.layers[0].surface().unwrap().pixel(0, 0);
            assert_ne!(original_pixel, expected_pixel, "the control must require a pixel conversion");
            let captured = Arc::new(std::sync::Mutex::new(None));
            let output = captured.clone();
            app.services.export = Some(Box::new(move |doc, _, _| {
                *output.lock().unwrap() = Some(doc.clone());
                Ok((Vec::new(), Vec::new()))
            }));
            app.services.write = Some(Box::new(|_, _| Ok(())));
            replay(&mut app, &json!({"path": "out.png", "format": "png", "profile": "srgb", "transparency": true})).unwrap();
            let actual = captured.lock().unwrap().take().unwrap();
            assert_eq!(actual.layers[0].surface().unwrap().pixel(0, 0), expected_pixel, "{working:?} {depth:?}");
            let profile = photocraft_cms::Profile::parse(actual.icc_profile.as_deref().unwrap()).unwrap();
            assert!(profile.same_bytes(Builtin::Srgb.profile()));
            assert_eq!(app.session.active().unwrap().doc, source);
        }
    }
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
    let converted = default_export_document(&doc, &f, None).unwrap();
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
