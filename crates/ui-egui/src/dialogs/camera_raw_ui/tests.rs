use super::*;

/// #9550: a Color Mixer Hue track runs through the neighbouring bands' hues (Reds: magenta to
/// orange, wrapping round), not from grey; Saturation and Luminance keep grey to the colour.
#[test]
fn hue_tracks_run_between_the_neighbouring_bands() {
    use super::controls::mixer_gradient;
    let hue = |d: f32| widgets::hue_color(d);
    assert_eq!(mixer_gradient(0, 0), vec![hue(300.0), hue(0.0), hue(30.0)], "Reds");
    assert_eq!(mixer_gradient(0, 7), vec![hue(270.0), hue(300.0), hue(0.0)], "Magentas");
    assert_eq!(mixer_gradient(0, 3), vec![hue(60.0), hue(120.0), hue(180.0)], "Greens");
    for tab in [1, 2] {
        assert_eq!(mixer_gradient(tab, 3), vec![Color32::from_gray(128), hue(120.0)]);
    }
}

#[test]
fn preview_texture_uses_linear_document_display_encoding() {
    let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
    doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let mut color = photocraft_engine::color_cmds::ColorState::default();
    color.settings.monitor_profile = "srgb".into();
    let display = color.canvas_display(&doc).unwrap();
    let px = [[0.25, 0.25, 0.25, 1.0]];
    let preview = CameraRawDialog::image(&px, 1, 1, &display);
    let expected = display.to_rgba8(&photocraft_compose::Buffer::filled(Rect::new(0, 0, 1, 1), px[0]));
    assert_eq!(preview.pixels[0].to_array(), expected.pixels[..4]);
    assert_ne!(preview.pixels[0].r(), 64, "linear light must be encoded before display");
}

#[test]
fn histogram_follows_preview_and_view_changes_do_not_render_or_edit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let initial = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    assert_eq!(initial["histogram"]["samples"], 64 * 48);
    assert_eq!(initial["histogram"]["red"][128], 64 * 48);
    let updated = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    assert_ne!(updated["histogram"]["red"], initial["histogram"]["red"]);
    let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
    assert_eq!(before["histogram"]["source"], "before");
    assert_eq!(before["histogram"]["red"], initial["histogram"]["red"]);
    assert_eq!(before["previewRevision"], updated["previewRevision"]);
    let after = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": false, "set": {"exposure": 1}}})).unwrap().unwrap();
    assert_eq!(after["histogram"], updated["histogram"]);
    assert_eq!(after["previewRevision"], updated["previewRevision"]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    let pixel = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
    assert!((pixel[0] - 128.0 / 255.0).abs() < 0.0001);
}

/// Closing Camera Raw's window is Cancel. Without a multi-window backend egui embeds the
/// "window" (`ViewportClass::EmbeddedWindow`), and the close request comes from the window
/// it is drawn in; the overlay fallback (`embed_viewports`) keeps the dialog open.
#[test]
fn closing_the_camera_raw_window_cancels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let frame = |app: &mut PhotocraftApp, close: bool| {
        let mut input = egui::RawInput::default();
        if close {
            let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            info.events.push(egui::ViewportEvent::Close);
        }
        ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
    };
    open(&mut app, &ctx).unwrap();
    frame(&mut app, false);
    assert!(app.camera_raw.is_some(), "drawn over the main window");
    ctx.set_embed_viewports(false);
    frame(&mut app, false);
    assert!(app.camera_raw.is_some(), "drawn in its window");
    frame(&mut app, true);
    assert!(app.camera_raw.is_none(), "closing the window is Cancel");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A layer with tonal variation, a selection over part of it and the dialog open on it.
fn worker_app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#406080"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    open(&mut app, &ctx).unwrap();
    (app, ctx)
}

fn finish_worker(d: &mut CameraRawDialog, ctx: &egui::Context) {
    for _ in 0..2000 {
        d.poll_render(ctx).unwrap();
        if !d.render_pending() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the Camera Raw preview worker never finished");
}

/// The dialog's controls develop the proxy on a worker (#5964): the frame it shows is the one
/// the UI thread would have developed, and settings changed while it runs are developed next.
#[test]
fn worker_preview_matches_the_synchronous_render_and_ends_on_the_latest_settings() {
    let (mut app, ctx) = worker_app();
    let d = app.camera_raw.as_mut().unwrap();
    assert!(d.selection.is_some());
    let revision = d.preview_revision;
    d.params.exposure = 1.0;
    d.dirty = true;
    d.request_render(&ctx);
    assert!(d.render_pending() && !d.dirty);
    // A drag moves on while the first develop runs; the request waits for it.
    d.params.highlights = -60.0;
    d.params.shadows = 45.0;
    d.dirty = true;
    d.request_render(&ctx);
    finish_worker(d, &ctx);
    assert_eq!(d.preview_revision, revision + 1);
    assert!(d.dirty, "the newer settings are still to be developed");
    d.request_render(&ctx);
    finish_worker(d, &ctx);
    assert_eq!(d.preview_revision, revision + 2);
    let (from_worker, histogram) = (d.processed.clone(), d.after_histogram.channels);
    d.render(&ctx);
    assert_eq!(d.processed, from_worker, "the worker develops exactly what the UI thread does");
    assert_eq!(d.after_histogram.channels, histogram);
    assert!(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)["previewPending"] == false);
}

/// The control channel renders on its own thread so its reply describes its settings; a worker
/// still running with older settings must not replace that frame afterwards.
#[test]
fn a_stale_worker_never_replaces_a_newer_synchronous_render() {
    let (mut app, ctx) = worker_app();
    let d = app.camera_raw.as_mut().unwrap();
    d.params.highlights = -80.0;
    d.dirty = true;
    d.request_render(&ctx);
    assert!(d.render_pending());
    let reply = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": 0, "exposure": -1.0}}})).unwrap().unwrap();
    assert_eq!(reply["previewPending"], false);
    let d = app.camera_raw.as_mut().unwrap();
    let (revision, shown) = (d.preview_revision, d.processed.clone());
    std::thread::sleep(std::time::Duration::from_millis(200));
    d.poll_render(&ctx).unwrap();
    assert_eq!((d.preview_revision, &d.processed), (revision, &shown));
    d.render(&ctx);
    assert_eq!(d.processed, shown, "the frame shown is the one for the control channel's settings");
}

/// Dragging a slider in the dialog itself never develops on the UI thread: the frame that
/// takes the change returns with the develop still running, and the preview catches up.
#[test]
fn a_dialog_edit_develops_off_the_ui_thread() {
    let (mut app, ctx) = worker_app();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
    let frame = |app: &mut PhotocraftApp| {
        ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx())).textures_delta.clear();
    };
    frame(&mut app);
    let d = app.camera_raw.as_mut().unwrap();
    let revision = d.preview_revision;
    d.params.highlights = -60.0;
    d.dirty = true;
    frame(&mut app);
    let d = app.camera_raw.as_mut().unwrap();
    assert!(d.render_pending() && d.preview_revision == revision, "the frame returned before the develop finished");
    finish_worker(d, &ctx);
    assert_eq!(d.preview_revision, revision + 1);
    assert!(!d.dirty);
    frame(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// A failed worker develop shows its error until a later frame is published, and never takes
/// down an error something else reported meanwhile.
#[test]
fn a_worker_failure_is_reported_until_the_next_frame() {
    let (mut app, ctx) = worker_app();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
    let frame = |app: &mut PhotocraftApp| {
        ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx())).textures_delta.clear();
    };
    let fail = |app: &mut PhotocraftApp| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Err("Camera Raw preview failed".to_string())).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        d.pending = Some(rx);
        d.dirty = false;
    };
    let edit = |app: &mut PhotocraftApp, exposure: f32| {
        let d = app.camera_raw.as_mut().unwrap();
        d.params.exposure = exposure;
        d.dirty = true;
    };
    frame(&mut app);
    fail(&mut app);
    frame(&mut app);
    assert!(app.ui.status_error && app.ui.status == "Camera Raw preview failed", "{}", app.ui.status);
    // No new frame yet: the error stays.
    frame(&mut app);
    assert!(app.ui.status_error);
    edit(&mut app, 0.5);
    frame(&mut app);
    finish_worker(app.camera_raw.as_mut().unwrap(), &ctx);
    frame(&mut app);
    assert!(!app.ui.status_error && app.ui.status.is_empty(), "{}", app.ui.status);

    fail(&mut app);
    frame(&mut app);
    app.ui.status = "something else failed".into();
    edit(&mut app, 1.0);
    frame(&mut app);
    finish_worker(app.camera_raw.as_mut().unwrap(), &ctx);
    frame(&mut app);
    assert!(app.ui.status_error && app.ui.status == "something else failed", "{}", app.ui.status);

    // A control-channel edit renders on its own thread; the status is clear before any frame.
    fail(&mut app);
    frame(&mut app);
    assert!(app.ui.status_error);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": -0.5}}})).unwrap().unwrap();
    assert!(!app.ui.status_error && app.ui.status.is_empty(), "{}", app.ui.status);
    assert_eq!(crate::control::inspect(&app, &ctx)["statusError"], false);
}

#[test]
fn smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    // An older editor could save stacked curve inputs; the filter must stay editable.
    let legacy = json!([[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]]);
    app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
    app.run("layer.smartFilter.setParams", json!({"index": 0, "params": {"pointCurve": legacy}})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    };
    let steps = app.session.active().unwrap().history.past_len();

    let sm = smart(&app);
    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
    let opened = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {}})).unwrap().unwrap();
    assert_eq!(opened["smartFilter"], 0);
    assert_eq!(opened["params"]["exposure"], 1.0);
    assert_eq!(opened["params"]["pointCurve"], legacy);
    // The preview starts from the pixels below the filter, not the filtered cache.
    let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
    assert_eq!(before["histogram"]["red"][128], 64 * 48);

    // Resetting a control to its default must overwrite the stored value.
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0}, "before": false, "commit": true}})).unwrap();
    assert!(r.is_ok(), "{r:?}");
    assert!(app.camera_raw.is_none());
    let sm = smart(&app);
    assert_eq!(sm.smart_filters.len(), 1, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].params["exposure"], 0.0);
    assert_eq!(sm.smart_filters[0].params["contrast"], 20.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app).smart_filters[0].params["exposure"], 1.0);

    // A broken new curve is still rejected; an unknown index is an error, not a panic.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurve": [[10, 10]]}}})).unwrap().is_err());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 7}})).unwrap().is_err());
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": "x"}})).unwrap().is_err());
}

#[test]
fn distant_sparse_pixels_cannot_trigger_an_unbounded_proxy_scan() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":64,"height":48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    source.write_region(Rect::new(30_000, 30_000, 30_001, 30_001), &[0.5, 0.5, 0.5, 1.0]);
    let result = open_pixels(&mut app, &ctx, layer, source, CameraRaw::default(), Coverage::None, Target::NewFilter);
    assert!(result.unwrap_err().contains("bounds are too large"));
    assert!(app.camera_raw.is_none());
}

#[test]
fn dialog_drives_the_engine() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    assert_eq!(r["proxy"], json!([64, 48]));
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "vignetteAmount": -40}}})).unwrap().unwrap();
    assert_eq!(r["params"]["exposure"], 1.0);
    assert_eq!(app.camera_raw.as_ref().unwrap().command_params(), json!({"exposure": 1.0, "vignetteAmount": -40.0}));
    let before = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(app.camera_raw.is_none());
    let after = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
    assert!(after[0] > before[0] + 0.1, "{before:?} → {after:?}");
    // Cancel leaves the document alone.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(r["cancelled"], true);
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": "x"}}})).unwrap().is_err());
}

/// An app whose importer is the real one, opening a synthetic DNG.
fn raw_app() -> (PhotocraftApp, Vec<u8>) {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let services = crate::Services {
        import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        ..Default::default()
    };
    let (w, h) = (48, 32);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    // The pixel comparisons below read the developed ProPhoto values: no conversion on open
    // (Camera Raw Defaults › Color space is Adobe RGB otherwise).
    app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "proPhoto"})).unwrap();
    (app, spec.build())
}

fn mean_red(app: &PhotocraftApp) -> f32 {
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..8).map(|i| s.rgba(4 + i * 4, 16)[0]).sum::<f32>() / 8.0
}

#[test]
fn raw_opens_in_camera_raw_and_open_redevelops() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    let warnings = app.open_bytes("IMG_0001.dng", &dng).unwrap();
    assert!(warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "{warnings:?}");
    // A raw has no embedded profile (ProPhoto RGB is the space it is developed into), so Ask
    // When Opening (on by default) has nothing to ask about.
    assert!(app.ui.dialogs.is_empty(), "no profile prompt for a raw");
    let raw = app.pending_raw_open.take().expect("a raw open queues the dialog");
    open_raw(&mut app, &ctx, raw).unwrap();
    let d = app.camera_raw.as_ref().unwrap();
    assert_eq!(d.target, Target::OpenRaw);
    assert_eq!(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)["openingRaw"]["name"], "IMG_0001.dng");
    let before = mean_red(&app);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "temperature": 30, "contrast": 20}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_some(), "contrast is applied as a Camera Raw step: {r}");
    assert!(app.camera_raw.is_none());
    assert_eq!(app.session.documents().len(), 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision, "Open leaves the document unmodified");
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
    assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
}

fn pixels(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..32).flat_map(|y| (0..48).map(move |x| (x, y))).flat_map(|(x, y)| s.rgba(x, y)).collect()
}

#[test]
fn the_demosaic_method_is_chosen_at_open() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0002.dng", &dng).unwrap();
    let ahd = pixels(&app);
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let d = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "bilinear"}})).unwrap().unwrap();
    assert_eq!(d["openingRaw"]["demosaic"], "bilinear");
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "amaze"}})).unwrap().is_err());
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "another method re-develops: {r}");
    assert!(r.get("filter").is_none(), "nothing else to apply: {r}");
    let bilinear = pixels(&app);
    assert!(ahd.iter().zip(&bilinear).any(|(a, b)| (a - b).abs() > 1e-3), "bilinear differs from AHD");
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision);

    // The preference sets the method of the open itself; Open with it changes nothing.
    app.run("prefs.set", json!({"path": "rawDefaults.demosaic", "value": "bilinear"})).unwrap();
    app.open_bytes("IMG_0003.dng", &dng).unwrap();
    assert_eq!(pixels(&app), bilinear, "developed with the preference's method");
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let d = app.camera_raw.as_ref().unwrap().describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
    assert_eq!(d["openingRaw"]["demosaic"], "bilinear");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], false, "{r}");

    // Only the open-time dialog has the choice.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "ahd"}})).unwrap().is_err());

    // The preference may change while a background open runs: the dialog starts from the
    // method the open developed with (its note), not from the newer preference.
    let warnings = app.open_bytes("IMG_0004.dng", &dng).unwrap();
    app.pending_raw_open.take();
    app.run("prefs.set", json!({"path": "rawDefaults.demosaic", "value": "ahd"})).unwrap();
    let document = app.session.active().unwrap().doc.id;
    queue_open_dialog(&mut app, document, "IMG_0004.dng", None, Some(&dng), &warnings).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    use photocraft_io::raw::Demosaic;
    assert_eq!((raw.developed_with, raw.demosaic), (Demosaic::Bilinear, Demosaic::Bilinear));
}

#[test]
fn shadows_of_a_raw_open_act_on_the_raw_data_and_the_preview_shows_it() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0002.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert!(app.camera_raw.as_ref().unwrap().raw_curve.is_some(), "the preview knows how the raw was developed");
    let mean = |px: &[[f32; 4]]| px.iter().map(|p| p[0] + p[1] + p[2]).sum::<f32>() / px.len() as f32;
    let preview_before = mean(&app.camera_raw.as_ref().unwrap().processed);
    let before = mean_red(&app);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"shadows": 100}}})).unwrap().unwrap();
    let d = app.camera_raw.as_mut().unwrap();
    d.render(&ctx);
    assert!(mean(&d.processed) > preview_before + 0.01, "{preview_before} → {}", mean(&d.processed));
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_none(), "Shadows acts while developing, not as a later filter step: {r}");
    assert!(mean_red(&app) > before + 0.01, "{before} → {}", mean_red(&app));
}

#[test]
fn clarity_texture_and_dehaze_of_a_raw_open_act_on_the_raw_data() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0003.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let before = mean_red(&app);
    let preview_before = app.camera_raw.as_ref().unwrap().processed.clone();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"clarity": 60, "texture": 40, "dehaze": 50}}})).unwrap().unwrap();
    let d = app.camera_raw.as_mut().unwrap();
    d.render(&ctx);
    assert_ne!(d.processed, preview_before, "the preview shows the raw-stage change");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_none(), "they act while developing, not as a later filter step: {r}");
    assert!(mean_red(&app) < before, "Dehaze darkens: {before} → {}", mean_red(&app));
}

#[test]
fn background_redevelop_keeps_the_tab_position_and_view() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 10, "height": 10})).unwrap();
    app.open_bytes("mid.dng", &dng).unwrap();
    app.run("file.new", json!({"width": 12, "height": 12})).unwrap();
    app.sync_views();
    let raw = app.pending_raw_open.take().unwrap();
    let old_id = raw.document;
    assert_eq!(document_index(&app, old_id), Some(1));
    app.ui.views[1].zoom = 3.0;
    open_raw(&mut app, &ctx, raw).unwrap();
    let before = mean_red(&app);
    // The re-develop and the Camera Raw step run as jobs; the UI thread only polls.
    app.background_jobs = true;
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "contrast": 20}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["pending"], true, "{r}");
    assert!(app.camera_raw.is_none());
    for _ in 0..3000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
    let docs = app.session.documents();
    assert_eq!(docs.len(), 3);
    assert_eq!((docs[0].doc.size.width, docs[2].doc.size.width), (10, 12), "the other tabs keep their places");
    assert_ne!(docs[1].doc.id, old_id, "re-developed");
    assert_eq!(docs[1].doc.depth, photocraft_color::SampleType::U16);
    assert_eq!(docs[1].saved_revision, docs[1].revision, "Open leaves the document unmodified");
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(app.ui.views[1].zoom, 3.0, "the view carries over to the re-developed document");
    assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
}

/// #7789: Preferences › Raw Defaults › Resolution sets a raw's ppi when the app opens it
/// itself (no background job) and from automation, and a Camera Raw re-develop keeps it.
#[test]
fn a_raw_opens_and_redevelops_at_the_raw_defaults_resolution() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.run("prefs.set", json!({"path": "rawDefaults.resolution", "value": 240})).unwrap();
    app.open_bytes("IMG_0004.dng", &dng).unwrap();
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0);
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0, "kept by the re-develop");
    app.open_automation_bytes("IMG_0005.dng", &dng).unwrap();
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0, "automation open");
}

#[test]
fn cancel_closes_the_raw_and_the_preference_skips_the_dialog() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("a.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(r["cancelled"], true);
    assert!(app.session.documents().is_empty(), "Cancel closes the raw being opened");
    // Unchanged settings: Open keeps the default develop and adds no step.
    app.open_bytes("b.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!((r["document"].as_u64(), r["redeveloped"].as_bool(), r.get("filter")), (Some(0), Some(false), None), "{r}");
    assert_eq!(r.get("defaults"), None, "ProPhoto, 16-bit: no Camera Raw Default to apply: {r}");
    // Preference off: raws open straight away.
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    app.open_bytes("c.dng", &dng).unwrap();
    assert!(app.pending_raw_open.is_none());
    // A non-raw never queues the dialog; a closed document is an error, not a panic.
    assert!(
        open_raw(
            &mut app,
            &ctx,
            RawOpen {
                document: photocraft_doc::DocId(u64::MAX),
                name: "x".into(),
                path: None,
                bytes: None,
                developed_with: Default::default(),
                demosaic: Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn calibration_changes_the_preview_and_round_trips_through_xmp() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0005.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let before = app.camera_raw.as_ref().unwrap().processed.clone();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"shadowTint": 100, "calibrationHue": [100, 0, 0], "calibrationSat": [0, 0, -100]}}}))
        .unwrap()
        .unwrap();
    let d = app.camera_raw.as_mut().unwrap();
    d.render(&ctx);
    assert!(
        d.processed.iter().zip(&before).any(|(a, b)| (a[0] - b[0]).abs() > 0.01 || (a[1] - b[1]).abs() > 0.01),
        "the Calibration panel changes the preview"
    );
    let xmp = photocraft_io::camera_raw_xmp::save(&d.params);
    assert!(xmp.contains("crs:ShadowTint=\"+100\"") && xmp.contains("crs:RedHue=\"+100\"") && xmp.contains("crs:BlueSaturation=\"-100\""), "{xmp}");
    let mut back = CameraRaw::default();
    photocraft_io::camera_raw_xmp::load(&mut back, xmp.as_bytes()).unwrap();
    assert_eq!((back.shadow_tint, back.calibration_hue, back.calibration_sat), (100.0, [100.0, 0.0, 0.0], [0.0, 0.0, -100.0]));
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"auto": "yes"}})).unwrap().is_err(), "auto must be a boolean");
}

#[test]
fn auto_sets_the_light_sliders_from_the_image() {
    let (mut app, dng) = raw_app();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0006.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert!(app.camera_raw.as_ref().unwrap().params.is_identity());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"auto": true}})).unwrap().unwrap();
    let p = app.camera_raw.as_ref().unwrap().params.clone();
    assert!(!p.is_identity(), "Auto chose something: {p:?}");
    assert!(p.saturation == 0.0 && p.hsl_hue == [0.0; 8], "only the Light panel: {p:?}");
    // The preference does the same when a raw opens without a sidecar.
    app.run("prefs.set", json!({"path": "rawDefaults.applyAutoTone", "value": true})).unwrap();
    app.open_bytes("IMG_0007.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert_eq!(app.camera_raw.as_ref().unwrap().params, p, "Apply auto tone opens the dialog with Auto's settings");
}

#[test]
fn a_sidecar_fills_the_dialog_and_open_writes_it_back() {
    use std::sync::{Arc, Mutex};
    let (mut app, dng) = raw_app();
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let raw_path = dir.join("IMG_0008.cr2").to_string_lossy().into_owned();
    std::fs::write(&raw_path, &dng).unwrap();
    assert_eq!(sidecar_path(&raw_path), dir.join("IMG_0008.xmp").to_string_lossy());
    assert_eq!(sidecar_path("/a/b.c/noext"), "/a/b.c/noext.xmp");
    std::fs::write(
        sidecar_path(&raw_path),
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"PhotoCraft\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\" crs:Exposure2012=\"+1.50\" crs:ShadowTint=\"20\" crs:Temperature=\"5500\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();
    type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let written: Written = Arc::default();
    let sink = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        sink.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0008.cr2", &dng).unwrap();
    let mut raw = app.pending_raw_open.take().unwrap();
    raw.path = Some(raw_path.clone());
    open_raw(&mut app, &ctx, raw).unwrap();
    let d = app.camera_raw.as_ref().unwrap();
    assert_eq!((d.params.exposure, d.params.shadow_tint), (1.5, 20.0), "the sidecar fills the dialog");
    assert!(
        app.ui.status.contains("IMG_0008.xmp") && app.ui.status.contains("Temperature"),
        "the absolute white balance is reported as not supported: {}",
        app.ui.status
    );
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    let w = written.lock().unwrap().clone();
    assert_eq!(w.len(), 1, "Open writes the sidecar once: {:?}", w.iter().map(|(p, _)| p).collect::<Vec<_>>());
    assert_eq!(w[0].0, sidecar_path(&raw_path));
    let text = String::from_utf8(w[0].1.clone()).unwrap();
    assert!(text.contains("crs:Exposure2012=\"+1.50\"") && text.contains("crs:Contrast2012=\"+15\"") && text.contains("crs:ShadowTint=\"+20\""), "{text}");
    // A DNG keeps its settings inside: no sidecar beside it. And the preference turns writing off.
    written.lock().unwrap().clear();
    let dng_path = dir.join("IMG_0009.dng").to_string_lossy().into_owned();
    std::fs::write(&dng_path, &dng).unwrap();
    app.open_bytes("IMG_0009.dng", &dng).unwrap();
    let mut raw = app.pending_raw_open.take().unwrap();
    raw.path = Some(dng_path);
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "no sidecar for a DNG");
    app.run("prefs.set", json!({"path": "rawDefaults.writeXmpSidecars", "value": false})).unwrap();
    app.open_bytes("IMG_0008.cr2", &dng).unwrap();
    let mut raw = app.pending_raw_open.take().unwrap();
    raw.path = Some(raw_path.clone());
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 30}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "the preference stops the write");
    // Without the dialog, the sidecar still develops the file.
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    let before = app.session.documents().len();
    app.open_file(&raw_path, &dng).unwrap();
    assert!(app.pending_raw_open.is_none());
    assert_eq!(app.session.documents().len(), before + 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision, "opening with a sidecar leaves the document unmodified");
    assert!(mean_red(&app) > 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn camera_raw_defaults_apply_when_a_raw_opens() {
    let (mut app, dng) = raw_app();
    app.run("prefs.set", json!({"values": {"rawDefaults.openInCameraRaw": false, "rawDefaults.colorSpace": "srgb", "rawDefaults.bitDepth": "8", "rawDefaults.resolution": 240.0, "rawDefaults.sharpenFor": "screen", "rawDefaults.openAsSmartObject": true}})).unwrap();
    app.open_bytes("IMG_0010.dng", &dng).unwrap();
    assert!(app.pending_raw_open.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U8, "Bit depth 8");
    assert_eq!(st.doc.resolution_dpi, 240.0, "Resolution");
    let profile = st.doc.icc_profile.as_deref().and_then(|p| photocraft_cms::Profile::parse(p).ok()).map(|p| p.description).unwrap_or_default();
    assert!(profile.to_lowercase().contains("srgb"), "Color space: {profile}");
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, photocraft_doc::LayerContent::Smart(_)), "Open as Smart Object");
    assert_eq!(st.saved_revision, st.revision, "the defaults are part of opening: unmodified");
    // The dialog's Open applies them too, and reports what it applied.
    app.run("prefs.set", json!({"values": {"rawDefaults.openInCameraRaw": true, "rawDefaults.openAsSmartObject": false, "rawDefaults.bitDepth": "16"}}))
        .unwrap();
    let ctx = egui::Context::default();
    app.open_bytes("IMG_0011.dng", &dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["defaults"], json!(["sharpenFor", "colorSpace"]), "{r}");
    assert!(r.get("filter").is_some(), "Sharpen For is a Camera Raw step: {r}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
    assert_eq!(st.saved_revision, st.revision);
}

type Written = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// A sidecar as Lightroom Classic leaves one beside a raw: its own toolkit, a crop, a rating
/// and a keyword next to the develop settings (values synthetic).
const LIGHTROOM_SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:Rating="4"
   crs:Exposure2012="+1.00"
   crs:Sharpness="40"
   crs:HasCrop="True"
   crs:CropTop="0.1">
   <dc:subject><rdf:Bag><rdf:li>harbour</rdf:li></rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

/// `raw_app` whose `write` service records what it is asked to write, and a raw file in a fresh
/// folder with `sidecar` beside it when given. Returns the app, the raw's bytes, the folder,
/// the raw's path and the recorded writes.
fn raw_with_sidecar(tag: &str, sidecar: Option<&str>) -> (PhotocraftApp, Vec<u8>, std::path::PathBuf, String, Written) {
    let (mut app, dng) = raw_app();
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let raw_path = dir.join("IMG_0100.cr2").to_string_lossy().into_owned();
    std::fs::write(&raw_path, &dng).unwrap();
    if let Some(text) = sidecar {
        std::fs::write(sidecar_path(&raw_path), text).unwrap();
    }
    let written: Written = Default::default();
    let sink = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        sink.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    (app, dng, dir, raw_path, written)
}

/// Opens the raw at `raw_path` in the open-time dialog, as File › Open does.
fn open_dialog_at(app: &mut PhotocraftApp, ctx: &egui::Context, dng: &[u8], raw_path: &str) {
    app.open_bytes("IMG_0100.cr2", dng).unwrap();
    let mut raw = app.pending_raw_open.take().unwrap();
    raw.path = Some(raw_path.to_string());
    open_raw(app, ctx, raw).unwrap();
}

/// The notices about the sidecar (the open's own import notes aside).
fn notice_titles(app: &PhotocraftApp) -> Vec<String> {
    app.ui.notices.iter().map(|n| n.title.clone()).filter(|t| t.contains(".xmp")).collect()
}

/// A Lightroom or Bridge sidecar holds settings PhotoCraft can't keep (crop, masks, rating,
/// keywords): Open uses its develop settings but never writes over it, and says so.
#[test]
fn a_foreign_sidecar_is_never_overwritten() {
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("foreign", Some(LIGHTROOM_SIDECAR));
    let ctx = egui::Context::default();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 1.0, "the sidecar's settings fill the dialog");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "nothing written over Lightroom's sidecar");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), LIGHTROOM_SIDECAR);
    let titles = notice_titles(&app);
    assert!(titles.iter().any(|t| t.contains("IMG_0100.xmp") && t.contains("another application")), "{titles:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bridge writes a sidecar with only a rating (no Camera Raw settings): it is not PhotoCraft's
/// to replace, with or without a change in the dialog.
#[test]
fn a_rating_only_sidecar_is_left_alone() {
    let bridge = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5"/></rdf:RDF></x:xmpmeta>"#;
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("rating", Some(bridge));
    let ctx = egui::Context::default();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "Open with no change writes nothing");
    assert!(notice_titles(&app).is_empty(), "and has nothing to report: {:?}", notice_titles(&app));
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 10}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "nor with a change");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), bridge);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With Open in Camera Raw off, opening a raw develops it with its sidecar and writes nothing
/// back: nothing changed.
#[test]
fn opening_without_the_dialog_does_not_rewrite_the_sidecar() {
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("nodialog", Some(LIGHTROOM_SIDECAR));
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    app.open_file(&raw_path, &dng).unwrap();
    assert!(app.pending_raw_open.is_none());
    assert_eq!(app.session.documents().len(), 1);
    assert!(written.lock().unwrap().is_empty(), "just opening never writes the sidecar");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), LIGHTROOM_SIDECAR);
    assert!(notice_titles(&app).is_empty(), "{:?}", notice_titles(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A control or MCP request has no authority to write beside the user's files: its commit
/// develops the raw but writes no sidecar. The same Open from the dialog does.
#[test]
fn a_control_request_never_writes_the_sidecar() {
    fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("control", None);
    app.services.automation_authorize = Some(allow);
    let ctx = egui::Context::default();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    let commit = json!({"id": "filter.cameraRaw", "params": {"ui": {"set": {"contrast": 15}, "commit": true}}});
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.invoke", commit);
    let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &req) else { panic!("expected a reply") };
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(app.camera_raw.is_none(), "committed");
    assert!(written.lock().unwrap().is_empty(), "no sidecar from a control request");
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}, "commit": true}})).unwrap().unwrap();
    assert_eq!(written.lock().unwrap().len(), 1, "the user's own Open writes it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sidecar that can't be written is reported in a notice that outlasts the status line the
/// develop and the Camera Raw step write after it.
#[test]
fn a_sidecar_write_error_stays_reported() {
    let (mut app, dng, dir, raw_path, _) = raw_with_sidecar("readonly", None);
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("read-only volume".to_string())));
    let ctx = egui::Context::default();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.5, "contrast": 15}, "commit": true}})).unwrap().unwrap();
    let errors: Vec<_> = app.ui.notices.iter().filter(|n| n.error).map(|n| n.title.clone()).collect();
    assert!(errors.iter().any(|t| t.contains("IMG_0100.xmp") && t.contains("read-only volume")), "{errors:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn open_with_settings(app: &mut PhotocraftApp, ctx: &egui::Context, dng: &[u8], name: &str) {
    app.open_bytes(name, dng).unwrap();
    let raw = app.pending_raw_open.take().unwrap();
    open_raw(app, ctx, raw).unwrap();
    menu(app, ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.5, "contrast": 60, "saturation": 40, "vibrance": 30}}})).unwrap().unwrap();
}

fn profile_name(app: &PhotocraftApp) -> String {
    let st = app.session.active().unwrap();
    st.doc.icc_profile.as_deref().and_then(|p| photocraft_cms::Profile::parse(p).ok()).map(|p| p.description).unwrap_or_default()
}

/// Camera Raw renders in its own space and converts to the workflow space last, so Open gives
/// what the dialog previewed on the developed ProPhoto pixels: with the default Adobe RGB the
/// result is the Camera Raw step on the ProPhoto develop, then the conversion. Inline and with
/// the re-develop as a background job.
#[test]
fn the_camera_raw_step_runs_before_the_workflow_conversion() {
    let ctx = egui::Context::default();
    let (mut app, dng) = raw_app();
    open_with_settings(&mut app, &ctx, &dng, "a.dng");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    app.run("edit.convertToProfile", json!({"profile": "adobe-rgb-compat", "intent": "relative"})).unwrap();
    let want = pixels(&app);
    let close = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
    for background in [false, true] {
        let (mut app, dng) = raw_app();
        app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "adobeRgb"})).unwrap();
        open_with_settings(&mut app, &ctx, &dng, "b.dng");
        app.background_jobs = background;
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        if background {
            assert_eq!(r["pending"], true, "the re-develop runs as a job: {r}");
        } else {
            assert_eq!(r["defaults"], json!(["colorSpace"]), "{r}");
        }
        for _ in 0..3000 {
            crate::jobs_ui::tick(&mut app, &ctx);
            if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
        assert!(profile_name(&app).to_lowercase().contains("adobe"), "background {background}: {}", profile_name(&app));
        assert!(close(&pixels(&app), &want), "background {background}: the step ran on other pixels than the preview");
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "background {background}: unmodified");
    }
}

/// Two raws opened one after the other while the first one's re-develop still runs in the
/// background: each lands with its Camera Raw step and conversions and is left unmodified.
#[test]
fn two_raws_opened_back_to_back_both_finish_their_open() {
    let ctx = egui::Context::default();
    let (mut app, dng) = raw_app();
    app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "adobeRgb"})).unwrap();
    open_with_settings(&mut app, &ctx, &dng, "a.dng");
    app.background_jobs = true;
    let a = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(a["pending"], true, "the re-develop runs as a job: {a}");
    app.background_jobs = false;
    open_with_settings(&mut app, &ctx, &dng, "b.dng");
    app.background_jobs = true;
    let b = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(b["pending"], true, "{b}");
    for _ in 0..3000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
    assert_eq!(app.session.documents().len(), 2);
    for i in 0..2 {
        app.session.set_active(i);
        assert!(profile_name(&app).to_lowercase().contains("adobe"), "document {i}: {}", profile_name(&app));
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "document {i} unmodified");
    }
}

/// A sidecar is read only from a regular file of at most `MAX_XMP_BYTES`: a folder, or a file
/// one byte larger, is not read (a pipe or device named like one would never end).
#[test]
fn sidecars_are_read_from_small_regular_files_only() {
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-bounded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("folder.xmp")).unwrap();
    let cap = photocraft_io::camera_raw_xmp::MAX_XMP_BYTES;
    let (fits, larger) = (dir.join("fits.xmp"), dir.join("larger.xmp"));
    std::fs::write(&fits, vec![b' '; cap]).unwrap();
    std::fs::write(&larger, vec![b' '; cap + 1]).unwrap();
    let s = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    assert_eq!(read_bounded(&s(fits)).map(|b| b.len()), Some(cap));
    assert_eq!(read_bounded(&s(larger.clone())), None);
    assert_eq!(read_bounded(&s(dir.join("folder.xmp"))), None);
    assert_eq!(read_bounded(&s(dir.join("missing.xmp"))), None);
    assert_eq!(existing_sidecar_is_ours(&s(dir.join("folder.xmp"))), Some(false), "something is there: never written over");
    assert_eq!(existing_sidecar_is_ours(&s(larger)), Some(false));
    assert_eq!(existing_sidecar_is_ours(&s(dir.join("missing.xmp"))), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_and_load_settings_round_trip_through_xmp_files() {
    use crate::file_dialog::FileDialogAnswer;
    use std::sync::{Arc, Mutex};
    type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let written: Written = Arc::default();
    let w = written.clone();
    let foreign = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Clarity2012="+25" crs:LensProfileEnable="1"/>
            </rdf:RDF></x:xmpmeta>"#;
    let answers = vec![
        Some(FileDialogAnswer::SaveTo("/out/Look.xmp".into())),
        Some(FileDialogAnswer::Contents("Look.xmp".into(), Vec::new())),
        Some(FileDialogAnswer::Contents("Other.xmp".into(), foreign.to_vec())),
        Some(FileDialogAnswer::Contents("Broken.xmp".into(), b"<nope".to_vec())),
    ];
    let services = crate::Services {
        file_dialog: Some(crate::file_dialog::fake(answers).0),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp| app.poll_file_dialog(&egui::Context::default(), None);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.75, "contrast": -20, "hslSat": [0, 0, 0, 30, 0, 0, 0, 0]}}}))
        .unwrap()
        .unwrap();
    let saved_params = app.camera_raw.as_ref().unwrap().params.clone();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true, "before": true}})).unwrap().is_err());
    assert_eq!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true}})).unwrap().unwrap()["fileDialog"], "save");
    frame(&mut app);
    let (path, bytes) = written.lock().unwrap()[0].clone();
    assert_eq!(path, "/out/Look.xmp");
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("crs:Exposure2012=\"+0.75\"") && text.contains("crs:SaturationAdjustmentGreen=\"+30\""), "{text}");
    // Reset, then load the saved file back (the fake dialog's second answer, with its bytes).
    app.camera_raw.as_mut().unwrap().params = CameraRaw::default();
    let mut svc = crate::file_dialog::fake(vec![Some(FileDialogAnswer::Contents("Look.xmp".into(), bytes))]).0;
    std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    frame(&mut app);
    assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
    std::mem::swap(app.services.file_dialog.as_mut().unwrap(), &mut svc);
    // The original service's next answer is the empty file: an error, settings untouched.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    frame(&mut app);
    assert_eq!(app.camera_raw.as_ref().unwrap().params, saved_params);
    // A foreign preset applies what it holds over the current settings and reports the rest.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    frame(&mut app);
    let p = &app.camera_raw.as_ref().unwrap().params;
    assert_eq!((p.clarity, p.exposure), (25.0, 0.75));
    assert!(app.ui.status.contains("Other.xmp") && app.ui.status.contains('1'), "{}", app.ui.status);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    frame(&mut app);
    assert_eq!(app.camera_raw.as_ref().unwrap().params.clarity, 25.0);
}
