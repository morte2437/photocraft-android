/// #5472: a ⇧ square pressed between pixels snapped to 101 × 102; the short side grows to
/// match, on the side the drag points to.
#[test]
fn square_px_equalises_the_sides_away_from_the_press_point() {
    use super::{marquee_px, square_px};
    // Down-right drag: the continuous square (50.3, 50.7)..(150.8, 151.2) snaps unevenly.
    let r = marquee_px([50.3, 50.7], [150.8, 151.2]);
    assert_eq!(r, [50.0, 50.0, 151.0, 152.0], "uneven before the fix");
    assert_eq!(square_px(r, [50.3, 50.7], [150.8, 90.0]), [50.0, 50.0, 152.0, 152.0]);
    // Up-left drag: the short side grows towards lower coordinates.
    let r = marquee_px([150.8, 151.2], [50.3, 50.7]);
    assert_eq!(square_px(r, [150.8, 151.2], [60.0, 50.7]), [49.0, 50.0, 151.0, 152.0]);
    // Already square: unchanged.
    assert_eq!(square_px([10.0, 20.0, 30.0, 40.0], [10.0, 20.0], [30.0, 40.0]), [10.0, 20.0, 30.0, 40.0]);
}

/// #6015: a Fixed Size marquee pressed between pixels selected one pixel more on each side.
#[test]
fn fixed_size_px_is_exactly_the_size() {
    use super::fixed_size_px;
    // Down-right from (10.4, 20.6): 10 × 10 from the closest pixel edges.
    assert_eq!(fixed_size_px([10.4, 20.6], [20.4, 30.6], 10.0, 10.0), [10.0, 21.0, 20.0, 31.0]);
    // Up-left: the far corner is the low one; still exactly the size.
    assert_eq!(fixed_size_px([10.4, 20.6], [-53.6, -11.4], 64.0, 32.0), [-54.0, -11.0, 10.0, 21.0]);
    // 1 × 1 is one pixel, and a fractional size rounds to whole pixels.
    assert_eq!(fixed_size_px([3.5, 3.5], [4.5, 4.5], 1.0, 1.0), [4.0, 4.0, 5.0, 5.0]);
    assert_eq!(fixed_size_px([0.0, 0.0], [2.6, 2.4], 2.6, 2.4), [0.0, 0.0, 3.0, 2.0]);
}

#[test]
fn display_image_matches_egui_unmultiplied_conversion() {
    // Several parallel pieces and a ragged one; translucent, out-of-range and NaN values.
    let n: usize = 3 * 16 * 1024 + 77;
    let px: Vec<[f32; 4]> = (0..n)
        .map(|i| {
            let v = match i % 9 {
                0 => f32::NAN,
                1 => -0.5,
                2 => 3.25,
                _ => (i % 1001) as f32 / 1000.0,
            };
            [v, (i % 257) as f32 / 256.0, 1.0 - (i % 113) as f32 / 112.0, (i % 6) as f32 / 5.0]
        })
        .collect();
    let buf = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, n as i32, 1), px };
    let reference = |img: photocraft_raster::Rgba8Image| egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    assert_eq!(super::display_image(None, &buf).pixels, reference(buf.to_rgba8()).pixels);
    let mut doc = photocraft_doc::Document::new("t", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    doc.icc_profile = Some(photocraft_cms::Builtin::AdobeRgbCompat.profile().to_bytes());
    let mut color = photocraft_engine::color_cmds::ColorState::default();
    color.settings.monitor_profile = "srgb".into();
    let d = color.canvas_display(&doc).unwrap();
    assert!(!d.is_identity());
    assert_eq!(super::display_image(Some(&d), &buf).pixels, reference(d.to_rgba8(&buf)).pixels);
}

/// #7490: the inline (web) preview path composites and uploads each changed preview once, as
/// the background worker does. It used to flatten and upload it twice and record it twice,
/// so the frame's refresh time counted the preview twice.
/// The CPU canvas (no usable GPU; a flipped or pixel-aspect-corrected view, or a GPU fallback,
/// on a GPU machine) shows a filter dialog's live preview. It showed the document unfiltered.
#[test]
fn the_cpu_canvas_shows_a_filter_dialogs_preview() {
    let pixels = |ctx: &egui::Context, id: egui::TextureId| {
        let mut delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set.get(&id).and_then(|d| d.last()).expect("the canvas texture was written").image;
        let (size, px) = (image.size, image.pixels.clone());
        delta.clear();
        (size, px)
    };
    for (size, zoom) in [([64u32, 48u32], 1.0f32), ([2400, 2200], 0.25)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": size[0], "height": size[1], "background": "white"})).unwrap();
        app.run("edit.fill", json!({"color": "#3399cc"})).unwrap();
        app.ui.views.get_mut(0).expect("the view").zoom = zoom;
        let ctx = egui::Context::default();
        let (tex, _) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        let (_, plain) = pixels(&ctx, tex);
        crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
        let (tex, _) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        let (tex_size, shown) = pixels(&ctx, tex);
        let preview = app.filter_preview.as_ref().and_then(|p| p.result.clone()).expect("the dialog's preview");
        let (display, _) = canvas_display(&app, &preview, None);
        let expected = display_image(display.as_deref(), &photocraft_compose::flatten(&preview));
        assert_eq!(tex_size, expected.size, "{size:?}: the preview's own size (a proxy when zoomed out)");
        assert_eq!(shown, expected.pixels, "{size:?}: the canvas shows the preview");
        assert_ne!(shown.first(), plain.first(), "{size:?}: not the unfiltered document");
    }
    // A preview that changes the canvas's dimensions covers its own area, centred on the
    // document's (as the GPU canvas centres it), rather than being stretched over it.
    let size = |w, h| photocraft_geom::Size::new(w, h);
    assert_eq!(preview_bounds(size(64, 32), size(64, 32), 1), DRect::new(0, 0, 64, 32));
    assert_eq!(preview_bounds(size(2401, 2200), size(1201, 1100), 2), DRect::new(0, 0, 2401, 2200), "a proxy");
    assert_eq!(preview_bounds(size(64, 32), size(32, 64), 1), DRect::new(16, -16, 48, 48), "rotated");
    assert_eq!(preview_bounds(size(64, 32), size(80, 40), 1), DRect::new(-8, -4, 72, 36), "canvas grown");
    assert_eq!(preview_bounds(size(4000, 2000), size(1000, 2000), 2), DRect::new(1000, -1000, 3000, 3000), "a rotated proxy");

    // Image › Image Rotation › Arbitrary 90° on a 64×32 document: its 32×64 preview is drawn over
    // the centred 32×64 area.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    let id = crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
    let d = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    d.fields.insert("angle".into(), json!(90.0));
    d.fields.insert("__preview".into(), json!(true));
    let ctx = egui::Context::default();
    let t = ensure_texture_in_view(&mut app, &ctx, 0, None, None).unwrap();
    let (size, _) = pixels(&ctx, t.texture);
    assert_eq!(size, [32, 64], "the rotated preview's texture");
    assert_eq!(t.bounds, DRect::new(16, -16, 48, 48), "centred on the document");
    let centre = |r: DRect| (r.x0 + r.x1, r.y0 + r.y1);
    assert_eq!(centre(t.bounds), centre(app.session.active().unwrap().doc.bounds()), "the same centre, so the view's pan holds");
    // Preview off, then Cancel: the document again, unchanged.
    let revision = app.session.active().unwrap().revision;
    app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap().fields.insert("__preview".into(), json!(false));
    let t = ensure_texture_in_view(&mut app, &ctx, 0, None, None).unwrap();
    assert_eq!(t.bounds, DRect::new(0, 0, 64, 32), "Preview off");
    crate::dialogs::cancel(&mut app, id).unwrap();
    let t = ensure_texture_in_view(&mut app, &ctx, 0, None, None).unwrap();
    assert_eq!(t.bounds, DRect::new(0, 0, 64, 32), "after Cancel");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document untouched");
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_geom::Size::new(64, 32));
}

/// A channel overlay drawn from a size-changing preview covers the preview's own area, as the
/// image does, not the document's: a 90° rotation of a 64×32 document shows a 32×64 overlay.
#[test]
fn a_channel_overlay_follows_a_size_changing_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
    let id = crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
    let d = app.ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    d.fields.insert("angle".into(), json!(90.0));
    d.fields.insert("__preview".into(), json!(true));
    let ctx = egui::Context::default();
    for rotation in [0.0f32, 30.0] {
        let view = View { zoom: 4.0, center: [32.0, 16.0], rotation, ..app.ui.views[0].clone() };
        let mut output = ctx.run_ui(Default::default(), |ui| {
            canvas_view(&mut app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), view.clone(), true);
        });
        output.textures_delta.clear();
        let overlay = crate::channel_view::ensure(&mut app, &ctx, 0, None).expect("the channel overlay");
        let mesh = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id == overlay => Some(m.clone()),
                _ => None,
            })
            .expect("the overlay is painted");
        // The quad's sides, in screen points: the two shortest distances from a corner (the
        // third is the diagonal; a rotated view turns the quad, not its side lengths).
        let v: Vec<Pos2> = mesh.vertices.iter().map(|v| v.pos).collect();
        let mut d: Vec<f32> = v[1..4].iter().map(|p| v[0].distance(*p)).collect();
        d.sort_by(f32::total_cmp);
        let (short, long) = (d[0], d[1]);
        // Which side is the width: the one along the view's x axis turned by the rotation.
        let along = |p: Pos2| {
            let r = rotation.to_radians();
            ((p - v[0]).x * r.cos() + (p - v[0]).y * r.sin()).abs()
        };
        let width = v[1..4].iter().map(|p| along(*p)).fold(0.0f32, f32::max);
        assert!((long / short - 2.0).abs() < 0.01, "rotation {rotation}: sides {short}×{long}, the preview's 1:2");
        assert!((width - short).abs() < 0.5, "rotation {rotation}: the short side ({short}) runs across the view (width {width})");
    }
}

#[test]
fn inline_filter_preview_is_composited_and_recorded_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 320, "height": 240})).unwrap();
    crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
    app.perf.stage = Default::default();
    let (_, _, size) = ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("a preview");
    assert_eq!(size, [320, 240]);
    assert_eq!(app.perf.last_refresh, "filter-preview");
    assert_eq!(app.perf.last_refresh_px, 320 * 240);
    assert_eq!(app.perf.stage.composite_ms + app.perf.stage.upload_ms, app.perf.composite_ms + app.perf.upload_ms, "one record per preview");
    // Unchanged request: nothing is recomputed or recorded again.
    ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("the same preview");
    assert_eq!(app.perf.stage.composite_ms + app.perf.stage.upload_ms, app.perf.composite_ms + app.perf.upload_ms);
}

/// #6326: a filter's live preview is never coarser than the screen. On a document big enough
/// for a reduced proxy, the view at 100 % previews at full resolution, and only a view zoomed
/// out far enough uses the proxy. Through `ensure_filter_preview`, so the factor the canvas
/// actually requests is the one under test.
#[test]
fn filter_preview_at_100_percent_is_full_resolution() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 2400, "height": 2200})).unwrap();
    assert_eq!(crate::proxy::preview_factor(&app.session.active().unwrap().doc, true), 2, "big enough for a proxy");
    crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
    let ctx = egui::Context::default();
    let preview_at = |app: &mut PhotocraftApp, zoom: f32| {
        app.ui.views.get_mut(0).expect("the new document's view").zoom = zoom;
        assert_eq!(app.current_zoom(), zoom);
        let (k, _, size) = ensure_filter_preview(app, 0, &ctx).expect("a preview");
        (k, size)
    };
    assert_eq!(preview_at(&mut app, 1.0), (1, [2400, 2200]), "100 %: every pixel");
    assert_eq!(preview_at(&mut app, 0.25), (2, [1200, 1100]), "zoomed out: the document's proxy");
    // Low Resolution Previews off: full resolution at any zoom, exactly as before #6326.
    app.run("prefs.set", json!({"path": "performance.lowResolutionPreviews", "value": false})).unwrap();
    assert_eq!(preview_at(&mut app, 0.25), (1, [2400, 2200]), "the preference still decides");
}

/// The factor itself over the zoom range, and that Preferences › Performance › Low
/// Resolution Previews still decides whether there is a proxy at all (#6326).
#[test]
fn filter_preview_factor_caps_at_the_view_without_overriding_the_preference() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // 9 MP: reduced previews halve it (empty tiles, so nothing is allocated).
    app.run("file.new", json!({"width": 3000, "height": 3000, "background": "transparent"})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let params = json!({"amount": 100, "radius": 4.0, "threshold": 0});
    let k = |app: &mut PhotocraftApp, zoom: f32| {
        app.ui.views.get_mut(0).expect("the new document's view").zoom = zoom;
        super::filter_preview_factor(app, &doc, "filter.sharpen.unsharpMask", &params)
    };
    assert_eq!(k(&mut app, 0.25), 2, "zoomed out the preview stays reduced");
    assert_eq!(k(&mut app, 0.5), 2);
    assert_eq!(k(&mut app, 0.75), 1, "past the proxy's resolution it shows every pixel");
    assert_eq!(k(&mut app, 1.0), 1);
    assert_eq!(k(&mut app, 4.0), 1);
    // A corrupt or unset zoom must not pick a coarser proxy than the document's own.
    assert_eq!(k(&mut app, f32::NAN), 1);
    assert_eq!(k(&mut app, 0.0), 1);
    // Low Resolution Previews off: full resolution at any zoom, the cap a no-op.
    app.run("prefs.set", json!({"path": "performance.lowResolutionPreviews", "value": false})).unwrap();
    for zoom in [0.1, 0.25, 1.0, 4.0] {
        assert_eq!(k(&mut app, zoom), 1, "zoom {zoom}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_filter_previews_off_the_ui_thread_and_a_newer_value_cancels_the_old() {
    // Dragging Gaussian Blur's radius to 1000 px froze the window: the preview ran inline in
    // the frame, and every slider step waited for the previous blur to finish.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let ctx = egui::Context::default();
    let wait_for = |app: &mut PhotocraftApp, radius: f64| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(radius)) {
            assert!(std::time::Instant::now() < deadline, "no preview for radius {radius}");
            ensure_filter_preview(app, 0, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    // The first frame hands the blur to the worker and returns.
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none());
    assert!(app.filter_preview_worker.busy(), "the preview computes on the worker");
    let first = app.ui.dialogs[0].fields["radius"].as_f64().unwrap();
    wait_for(&mut app, first);
    assert!(app.filter_preview.as_ref().unwrap().result.is_some());

    // A preview for old values occupies the worker until it is cancelled.
    let st = app.session.active().unwrap();
    let d = &app.ui.dialogs[0];
    let stale = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog: d.id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": 999.0}),
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
    };
    let (told, cancelled) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(stale, ctx.clone(), move |job| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !job.cancelled() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let _ = told.send(job.cancelled());
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .unwrap();
    // The slider moves on and stops: once the value settles, the stale preview is cancelled
    // instead of being waited out.
    app.ui.dialogs[0].fields.insert("radius".into(), json!(12.0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let was_cancelled = loop {
        ensure_filter_preview(&mut app, 0, &ctx);
        if let Ok(c) = cancelled.try_recv() {
            break c;
        }
        assert!(std::time::Instant::now() < deadline, "the stale preview was never cancelled");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(was_cancelled, "the stale preview stopped because it was cancelled, not by timing out");
    wait_for(&mut app, 12.0);
    assert!(app.filter_preview.as_ref().unwrap().result.is_some());
}

// Starts a worker job that computes a real preview for the dialog as it stands, but only
// finishes when the returned sender is signalled. Gating it here keeps these tests off the
// clock: `supersede` cancels a running job once the values have been still for SETTLE_MS, so a
// test that relies on a real blur beating a 150 ms timer is a test that fails on a loaded box.
#[cfg(not(target_arch = "wasm32"))]
fn gated_blur_preview(app: &mut PhotocraftApp, ctx: &egui::Context) -> (std::sync::mpsc::Sender<()>, f64) {
    let d = app.ui.dialogs.first().expect("a dialog is open");
    let st = app.session.active().expect("a document is active");
    let command = "filter.blur.gaussianBlur";
    let params = app.with_mask_target(command, crate::filter_dialog::params_of(&d.fields));
    assert_eq!(
        crate::filter_dialog::preview_factor(command, &params, 1),
        1,
        "a 64x48 document previews at full resolution, and these tests rely on k staying 1"
    );
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog: d.id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: command.into(),
        params,
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
    };
    let doc = app.session.documents()[0].doc.clone();
    let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
    let radius = key.params["radius"].as_f64().expect("the dialog seeds a radius");
    let (release, wait) = std::sync::mpsc::channel::<()>();
    app.filter_preview_worker
        .start(key.clone(), ctx.clone(), move |_| {
            wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            let result = crate::filter_dialog::preview_document_for_target(
                &doc,
                key.active,
                &key.command,
                &key.params,
                key.k,
                None,
                key.target,
                &documents,
                &Default::default(),
                Some(key.tool_colors),
            )
            .map(|d| {
                let buffer = photocraft_compose::flatten(&d);
                (std::sync::Arc::new(d), buffer)
            });
            crate::dialogs::filter_preview_worker::Computed { result, ms: 0.0 }
        })
        .expect("the worker starts");
    (release, radius)
}

// The worker hands its result to the channel only after `compute` returns, so a signal sent
// from inside `compute` would still race that send. These tests drive the canvas until the
// state they need appears instead of assuming a timing.
#[cfg(not(target_arch = "wasm32"))]
fn drive_until(app: &mut PhotocraftApp, ctx: &egui::Context, what: &str, done: impl Fn(&PhotocraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done(app) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        ensure_filter_preview(app, 0, ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_held_preview_frame_stays_on_the_canvas_when_the_proxy_factor_changes() {
    // #10474: Minimum and Maximum take their proxy factor from the radius, so a radius drag across
    // a whole pixel changes `FilterPreviewKey::k`. The frame for the old factor stays on the
    // canvas, drawn at its own factor, until the one for the new factor lands.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 2400, "height": 2200})).unwrap();
    app.background_jobs = true;
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    crate::filter_dialog::open(&mut app, "filter.other.maximum").unwrap();
    app.ui.dialogs[0].fields.insert("radius".into(), json!(6.0));
    let ctx = egui::Context::default();
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    assert_eq!(app.filter_preview.as_ref().map(|p| p.key.k), Some(2), "6 px on a reduced document previews on the proxy");

    // 1.5 px previews at full resolution: a new request at a new factor, computed on the worker.
    app.ui.dialogs[0].fields.insert("radius".into(), json!(1.5));
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("the held frame stays on the canvas");
    assert_eq!((k, size), (2, [1200, 1100]), "drawn at its own factor");
    drive_until(&mut app, &ctx, "the full-resolution frame", |a| a.filter_preview.as_ref().is_some_and(|p| p.key.k == 1 && p.result.is_some()));
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("the new frame");
    assert_eq!((k, size), (1, [2400, 2200]));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn indexed_matte_preview_matches_commit_and_refreshes_tool_colors_inline_and_on_worker() {
    for background in [false, true] {
        for depth in [8, 16, 32] {
            for matte in ["foreground", "background"] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.background_jobs = background;
                app.run("file.new", json!({"width": 64, "height": 48, "depth": depth, "background": "transparent"})).unwrap();
                app.run("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
                app.run("edit.colorSettings", json!({"workingRgb": "display-p3"})).unwrap();
                app.session
                    .edit("Translucent edge", |doc, active| {
                        let format = doc.pixel_format();
                        doc.layer_mut(active.unwrap())
                            .unwrap()
                            .surface_mut()
                            .unwrap()
                            .fill_rect(photocraft_geom::Rect::new(0, 0, 32, 48), &photocraft_raster::from_rgba(&format, [1.0, 0.0, 0.0, 0.5]));
                        Ok(())
                    })
                    .unwrap();
                let source = app.session.active().unwrap().doc.clone();
                let params = json!({"palette": "exact", "forced": "none", "dither": "none", "matte": matte});
                crate::filter_dialog::open(&mut app, "image.mode.indexedColor").unwrap();
                app.ui.dialogs.last_mut().unwrap().fields.extend(params.as_object().unwrap().clone());
                let ctx = egui::Context::default();
                let mut previous = None;
                for (foreground, background_color) in [("#3366cc", "#cc6633"), ("#26bf59", "#bf26a6")] {
                    app.session.execute("tools.setColors", json!({"foreground": foreground, "background": background_color})).unwrap();
                    let colors = (app.session.tools.foreground, app.session.tools.background);
                    let shown = ensure_filter_preview(&mut app, 0, &ctx);
                    if background {
                        assert!(shown.is_none(), "old tool colors must not remain visible");
                    }
                    drive_until(&mut app, &ctx, "the current matte colors", |app| {
                        app.filter_preview.as_ref().is_some_and(|p| p.key.tool_colors == colors && p.result.is_some())
                    });
                    let preview = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap();
                    let mut applied = photocraft_engine::Session::new();
                    applied.color.settings = app.session.color.settings.clone();
                    applied.tools.foreground = colors.0;
                    applied.tools.background = colors.1;
                    applied.add_document((*source).clone(), None);
                    applied.execute("image.mode.indexedColor", params.clone()).unwrap();
                    let committed = &applied.active().unwrap().doc;
                    assert_eq!(preview.icc_profile, source.icc_profile);
                    assert_eq!(preview.color_table, committed.color_table);
                    let actual = preview.layers[0].surface().unwrap().read_region(preview.bounds());
                    let expected = committed.layers[0].surface().unwrap().read_region(committed.bounds());
                    assert_eq!(actual, expected, "{matte}, {depth}-bit, worker={background}");
                    if let Some(previous) = previous {
                        assert_ne!(actual, previous, "changing the selected tool color refreshes the pixels");
                    }
                    previous = Some(actual);
                    assert_eq!(app.session.active().unwrap().doc.mode, photocraft_doc::ColorMode::Rgb, "preview does not commit");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cross_scale_indexed_matte_preview_rejects_old_tool_colors_from_cached_and_worker_results() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = true;
    app.run("file.new", json!({"width": 2400, "height": 2200, "depth": 8, "background": "transparent"})).unwrap();
    app.run("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    app.run("edit.colorSettings", json!({"workingRgb": "display-p3"})).unwrap();
    app.session
        .edit("Translucent edge", |doc, active| {
            let format = doc.pixel_format();
            doc.layer_mut(active.unwrap())
                .unwrap()
                .surface_mut()
                .unwrap()
                .fill_rect(photocraft_geom::Rect::new(0, 0, 32, 48), &photocraft_raster::from_rgba(&format, [1.0, 0.0, 0.0, 0.5]));
            Ok(())
        })
        .unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    let params = json!({"palette": "exact", "forced": "none", "dither": "none", "matte": "foreground"});
    crate::filter_dialog::open(&mut app, "image.mode.indexedColor").unwrap();
    app.ui.dialogs.last_mut().unwrap().fields.extend(params.as_object().unwrap().clone());
    app.session.execute("tools.setColors", json!({"foreground": "#3366cc", "background": "#cc6633"})).unwrap();
    let ctx = egui::Context::default();
    drive_until(&mut app, &ctx, "the first indexed preview", |a| a.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    let old = app.filter_preview.as_ref().expect("the first preview").key.clone();
    assert_eq!(old.k, 2, "the zoomed-out large document uses the reduced preview");

    app.session.execute("tools.setColors", json!({"foreground": "#26bf59", "background": "#bf26a6"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 1.0;
    let new_colors = (app.session.tools.foreground, app.session.tools.background);
    let new = crate::filter_dialog::FilterPreviewKey { k: 1, tool_colors: new_colors, ..old.clone() };
    assert_ne!(old.tool_colors, new.tool_colors);
    assert_ne!(old.k, new.k, "the cache and request use different preview factors");

    // Keep the current worker pending so the only possible frame is the cached old-color one.
    let (release_new, wait_new) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(new.clone(), ctx.clone(), move |_| {
            wait_new.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .expect("the worker starts");
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the cached old-color frame is not painted while the new worker runs");
    release_new.send(()).expect("released");
    drive_until(&mut app, &ctx, "the new worker to finish", |a| !a.filter_preview_worker.busy());
    assert_eq!(app.filter_preview.as_ref().expect("the current request").key, new);
    assert!(app.filter_preview.as_ref().unwrap().result.is_none());

    // Start an old-color worker while its request is current, then move the dialog to a new
    // cross-scale request before the old result lands. Its real preview must not replace the
    // current request through the loose worker-result gate.
    app.session.execute("tools.setColors", json!({"foreground": "#3366cc", "background": "#cc6633"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    let old_worker = old.clone();
    let doc = app.session.documents()[0].doc.clone();
    let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
    let color = app.session.color.settings.clone();
    let (release_old, wait_old) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(old_worker.clone(), ctx.clone(), move |_| {
            wait_old.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            let result = crate::filter_dialog::preview_document_for_target(
                &doc,
                old_worker.active,
                &old_worker.command,
                &old_worker.params,
                old_worker.k,
                None,
                old_worker.target,
                &documents,
                &color,
                Some(old_worker.tool_colors),
            )
            .map(|doc| {
                let buffer = photocraft_compose::flatten(&doc);
                (std::sync::Arc::new(doc), buffer)
            });
            crate::dialogs::filter_preview_worker::Computed { result, ms: 0.0 }
        })
        .expect("the worker starts");
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the old cached request is not current");

    app.session.execute("tools.setColors", json!({"foreground": "#26bf59", "background": "#bf26a6"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 1.0;
    app.ui.dialogs[0].fields.insert("matte".into(), json!("background"));
    let mut new_late = new.clone();
    new_late.params.as_object_mut().unwrap().insert("matte".into(), json!("background"));
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the current request has no result while the old worker runs");
    release_old.send(()).expect("released");
    drive_until(&mut app, &ctx, "the stale old-color worker to finish", |a| {
        if let Some(shown) = &a.filter_preview {
            assert_eq!(shown.key.tool_colors, new.tool_colors, "the stale old-color worker result was not painted");
            assert_ne!(shown.key.k, old.k, "the stale old-scale worker result was not painted");
        }
        a.filter_preview.as_ref().is_some_and(|p| p.key == new_late && p.result.is_some()) && !a.filter_preview_worker.busy()
    });
    assert_eq!(app.filter_preview.as_ref().expect("the current request").key, new_late);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn match_color_preview_rejects_cached_and_late_frames_from_an_old_source_layer_or_revision() {
    use crate::dialogs::filter_preview_worker::Computed;

    let command = "image.adjustments.matchColor";
    for change_layer in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#cc4422"})).unwrap();
        app.run("layer.new.layer", json!({"name": "Blue"})).unwrap();
        let blue = app.session.active().unwrap().active_layer.unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#2266cc"})).unwrap();
        assert!(app.session.set_active(0));
        crate::filter_dialog::open(&mut app, command).unwrap();
        app.ui.dialogs[0].fields.extend([("source".into(), json!(1)), ("sourceLayer".into(), json!(red))]);
        let ctx = egui::Context::default();
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some(), "the first source preview reaches the canvas");
        let preview = app.filter_preview.as_ref().unwrap();
        let old = preview.key.clone();
        let doc = preview.result.as_ref().unwrap().clone();
        let mut slider = old.clone();
        slider.params["fade"] = json!(25.0);
        assert!(old.same_target(&slider), "a slider on the same source layer keeps the last frame");

        app.background_jobs = true;
        let (release, wait) = std::sync::mpsc::channel();
        assert!(
            app.filter_preview_worker
                .start(old.clone(), ctx.clone(), move |_| {
                    wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
                    let buffer = photocraft_compose::flatten(&doc);
                    Computed { result: Some((doc, buffer)), ms: 0.0 }
                })
                .unwrap()
        );
        let current_layer = if change_layer {
            app.ui.dialogs[0].fields.insert("sourceLayer".into(), json!(blue));
            blue
        } else {
            assert!(app.session.set_active(1));
            app.session.select_layer(red).unwrap();
            app.session.execute("edit.fill", json!({"contents": "color", "color": "#448822"})).unwrap();
            assert!(app.session.set_active(0));
            red
        };
        let source = &app.session.documents()[1];
        let sources = vec![(source.doc.id, source.revision)];
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the cached old preview is not displayed");
        app.filter_preview = None;
        release.send(()).unwrap();
        drive_until(&mut app, &ctx, "a current Match Color preview", |app| {
            let Some(preview) = &app.filter_preview else { return false };
            assert_eq!(preview.key.params["sourceLayer"], json!(current_layer), "a late worker published the wrong layer");
            assert_eq!(preview.key.sources, sources, "a late worker published the wrong source revision");
            preview.result.is_some() && !app.filter_preview_worker.busy()
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_preview_computed_mid_drag_reaches_the_canvas_and_then_converges() {
    // A drag used to paint nothing at all: every completion was dropped for carrying stale
    // parameters, so the canvas only ever showed an effect once the pointer had come to rest.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let ctx = egui::Context::default();
    let (release, first) = gated_blur_preview(&mut app, &ctx);

    // The pointer moves on before the frame lands.
    let moved = first + 4.0;
    app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the mid-drag frame", |a| a.filter_preview.is_some());

    // The finished frame is painted even though the dialog has moved on.
    let shown = app.filter_preview.as_ref().expect("the mid-drag frame was published");
    assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "the frame is the one the worker finished");
    assert!(shown.result.is_some());
    assert_eq!(app.ui.dialogs[0].fields["radius"].as_f64(), Some(moved), "the slider kept moving");
    // The display gate lets it through, so the canvas has something to draw: this is what
    // re-adding `params` to that gate would break.
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some(), "the canvas draws the mid-drag frame");

    // And the preview still converges on the value the dialog holds. `fresh` has to stay an
    // exact match for that: relaxed, the canvas would believe it already had the current
    // request after the first frame of a drag and never compute again.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(moved)) {
        assert!(std::time::Instant::now() < deadline, "the preview never converged on the current radius");
        ensure_filter_preview(&mut app, 0, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_mid_drag_frame_does_not_replace_a_frame_that_is_already_exact() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let ctx = egui::Context::default();

    // A frame for the radius the dialog holds right now reaches the canvas. This also proves
    // the hand-built key is the one the canvas builds: it can only be published by the exact
    // arm, which requires `finished == request`.
    let (release, first) = gated_blur_preview(&mut app, &ctx);
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
    assert_eq!(app.filter_preview.as_ref().expect("published").key.params["radius"].as_f64(), Some(first));

    // The slider moves away and a frame for the new radius is computed...
    let moved = first + 4.0;
    app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
    let (release_moved, _) = gated_blur_preview(&mut app, &ctx);
    // ...and back to `first` before that frame lands.
    app.ui.dialogs[0].fields.insert("radius".into(), json!(first));
    release_moved.send(()).expect("released");

    // The frame for `moved` arrives late. It belongs to this preview, but the frame on screen
    // is already exact for the dialog, so it must never be shown in place of it. Checked on
    // every pass: a wrong frame published and then overwritten before the loop ends would
    // otherwise never be seen.
    drive_until(&mut app, &ctx, "the late frame to arrive", |a| {
        if let Some(shown) = &a.filter_preview {
            assert_ne!(shown.key.params["radius"].as_f64(), Some(moved), "a late frame replaced an exact one");
        }
        !a.filter_preview_worker.busy()
    });
    let shown = app.filter_preview.as_ref().expect("a frame is on screen");
    assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "a late frame replaced an exact one");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_mid_drag_empty_result_leaves_the_preview_alone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let ctx = egui::Context::default();

    let (release, first) = gated_blur_preview(&mut app, &ctx);
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
    assert!(app.filter_preview.as_ref().expect("published").result.is_some());

    // The slider moves and a job for the new radius is started, then the slider moves again
    // before that job lands: its result is stale by the time it arrives.
    let moved = first + 4.0;
    app.ui.dialogs[0].fields.insert("radius".into(), json!(moved));
    let d = app.ui.dialogs.first().expect("a dialog is open");
    let st = app.session.active().expect("a document is active");
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog: d.id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": moved}),
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
    };
    let (release_empty, wait) = std::sync::mpsc::channel::<()>();
    app.filter_preview_worker
        .start(key, ctx.clone(), move |_| {
            wait.recv_timeout(std::time::Duration::from_secs(60)).expect("released");
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .expect("the worker starts");
    app.ui.dialogs[0].fields.insert("radius".into(), json!(moved + 4.0));
    release_empty.send(()).expect("released");

    // Storing an empty result here would drop the canvas back to the unfiltered layer for a
    // frame. The exact request may still store one; a stale one may not. Checked on every
    // pass, because the reopened preview that follows would otherwise repair the symptom.
    drive_until(&mut app, &ctx, "the empty frame to arrive", |a| {
        if let Some(shown) = &a.filter_preview {
            assert!(shown.result.is_some(), "an empty mid-drag result blanked the canvas");
        }
        !a.filter_preview_worker.busy()
    });
    assert!(app.filter_preview.as_ref().expect("a preview is still shown").result.is_some(), "an empty mid-drag result blanked the canvas");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_reopened_dialog_never_paints_the_previous_dialogs_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let ctx = egui::Context::default();
    let first_dialog = app.ui.dialogs.first().expect("a dialog is open").id;
    let (release, _) = gated_blur_preview(&mut app, &ctx);

    // Close that dialog and open another one for the same layer and the same command. Only the
    // dialog id differs, which is exactly what `same_target` has to notice.
    app.ui.dialogs.clear();
    crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let second_dialog = app.ui.dialogs.first().expect("a dialog is open").id;
    assert_ne!(second_dialog, first_dialog, "a reopened dialog gets a new id");
    release.send(()).expect("released");

    // The first dialog's frame arrives late and must not be painted under the second one.
    // `discard_closed` usually reclaims it a frame earlier, which is why this drives the gate
    // directly: the id check is the part that has to hold when that cleanup misses. The
    // invariant is checked on every pass, not only at the end, because a wrongly published
    // frame can still be replaced by the reopened dialog's own preview before we stop.
    drive_until(&mut app, &ctx, "the first dialog's frame to arrive", |a| {
        if let Some(preview) = &a.filter_preview {
            assert_ne!(preview.key.dialog, first_dialog, "a reopened dialog painted the previous dialog's preview");
        }
        !a.filter_preview_worker.busy()
    });
}

#[test]
fn windows_uses_a_visible_crosshair() {
    // #737: Windows' inverting crosshair vanishes over mid-grey; use a black/white glyph.
    let ctx = egui::Context::default();
    let painter = egui::Painter::new(ctx, egui::LayerId::background(), egui::Rect::EVERYTHING);
    let p = egui::pos2(10.0, 10.0);
    assert_eq!(super::visible_crosshair(egui::CursorIcon::Crosshair, &painter, p, true), egui::CursorIcon::None);
    assert_eq!(super::visible_crosshair(egui::CursorIcon::Crosshair, &painter, p, false), egui::CursorIcon::Crosshair);
    assert_eq!(super::visible_crosshair(egui::CursorIcon::Move, &painter, p, true), egui::CursorIcon::Move);
}

use super::*;

/// #569: displays 1 (sRGB) and 4 (Display P3) side by side, and a document filled with an
/// sRGB colour.
fn app_on_two_displays() -> PhotocraftApp {
    use photocraft_engine::color_cmds::resolve_profile;
    use photocraft_engine::display_color::Display;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("edit.fill", json!({"color": "#cc8040"})).unwrap();
    let icc = |id: &str| Some(resolve_profile(id, None, None).unwrap().to_bytes());
    app.session.color.set_displays(Ok(vec![
        Display { id: 1, name: "A".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
        Display { id: 4, name: "B".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
    ]));
    app
}

#[test]
#[ignore = "manual texture-retention measurement on six 6 MP documents"]
fn closed_canvas_memory_measurement() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.max_texture_side = 8192);
    for cycle in 1..=6 {
        app.run("file.new", json!({"width":3000,"height":2000})).unwrap();
        ensure_texture(&mut app, &ctx, 0, None).unwrap();
        // Drain uploaded image deltas just as a renderer does; count live texture owners.
        ctx.tex_manager().write().take_delta().clear();
        app.run("file.close", json!({})).unwrap();
        let bytes: usize = ctx.tex_manager().read().allocated().map(|(_, m)| m.bytes_used()).sum();
        println!("closed cycle={cycle} texture_bytes={bytes} rss={:?}", photocraft_testkit::perf::current_rss_bytes());
    }
}

#[test]
fn closed_documents_release_navigator_but_keep_open_document_textures() {
    let mut app = app_on_two_displays();
    let ctx = egui::Context::default();
    let first = app.session.active().unwrap().doc.clone();
    let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
    // The same handle ownership as navigator_texture, without requiring a GPU adapter.
    let tex = ctx.load_texture("navigator-test", egui::ColorImage::filled([64, 64], egui::Color32::WHITE), TextureOptions::LINEAR);
    let nav = tex.id();
    app.navigator_textures.insert(first.id, (std::sync::Arc::downgrade(&first), 0, 0, tex));
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    let other = app.session.active().unwrap().doc.id;
    let b = ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0;
    drop(app.session.close(0));
    app.sync_views();
    assert!(ctx.tex_manager().read().meta(a).is_none());
    assert!(ctx.tex_manager().read().meta(nav).is_none());
    assert!(ctx.tex_manager().read().meta(b).is_some());
    assert!(app.canvases.keys().all(|(id, _)| *id == other));
    assert!(app.navigator_textures.is_empty());
    app.session.add_document((*first).clone(), None);
    app.sync_views();
    assert_ne!(ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0, a, "reopening a preserved ID creates a fresh texture");
}

#[test]
fn only_themes_with_a_dot_pattern_tile_the_canvas_surround() {
    // #4542: switching to Pro filled the empty workspace with red blocks on one GPU. Pro's
    // dots are its canvas colour, so the tiled texture had nothing to show in the first place.
    use crate::theme::ThemeKind;
    let tiles = |ctx: &egui::Context, kind: ThemeKind| {
        crate::theme::apply(ctx, kind);
        let mut output = ctx.run_ui(Default::default(), |ui| paint_dots(ui, ui.max_rect(), true));
        output.textures_delta.clear();
        output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_)))
    };
    let uploaded = |ctx: &egui::Context| ctx.tex_manager().read().allocated().filter(|(_, meta)| meta.name == "canvas-dots").count();
    for kind in ThemeKind::ALL {
        let ctx = egui::Context::default();
        let dotted = matches!(kind, ThemeKind::Studio | ThemeKind::StudioLight | ThemeKind::SolarizedDark);
        assert_eq!(tiles(&ctx, kind), dotted, "{kind:?}");
        assert_eq!(uploaded(&ctx), usize::from(dotted), "{kind:?}");
    }
    // The reported switch: a dotted theme, then Pro.
    let ctx = egui::Context::default();
    assert!(tiles(&ctx, ThemeKind::Studio));
    assert!(!tiles(&ctx, ThemeKind::Pro));
    assert!(!tiles(&ctx, ThemeKind::ProMedium));
    assert_eq!(uploaded(&ctx), 1, "only Studio's dots were uploaded");
}

#[test]
fn closed_documents_release_cpu_canvas_textures() {
    let mut app = app_on_two_displays();
    let ctx = egui::Context::default();
    let original = app.session.active().unwrap().doc.clone();
    for cycle in 0..12 {
        if cycle > 0 {
            app.session.add_document((*original).clone(), None);
            app.sync_views();
        }
        let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
        let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
        assert_ne!(a, b);
        app.run("file.close", json!({})).unwrap();
        assert!(app.canvases.is_empty(), "closed canvas metadata retained at cycle {cycle}");
        assert!(ctx.tex_manager().read().meta(a).is_none(), "first display texture retained");
        assert!(ctx.tex_manager().read().meta(b).is_none(), "second display texture retained");
    }
}

#[test]
fn cpu_canvas_textures_are_per_display() {
    // Two windows of one document on different displays: each has its own texture of
    // monitor values, and drawing them in turn re-renders neither.
    let mut app = app_on_two_displays();
    let ctx = egui::Context::default();
    let id = app.session.documents()[0].doc.id;
    let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
    let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
    assert_ne!(a, b);
    app.perf.last_refresh = "";
    for _ in 0..3 {
        assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0, a);
        assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0, b);
    }
    assert_eq!(app.perf.last_refresh, "", "no re-render");
    let doc = app.session.documents()[0].doc.clone();
    assert!(canvas_display(&app, &doc, Some(1)).0.unwrap().is_identity());
    assert!(!canvas_display(&app, &doc, Some(4)).0.unwrap().is_identity());
    assert_ne!(app.canvases[&(id, 1)].tex_preview_key, app.canvases[&(id, 4)].tex_preview_key);
    // An edit updates both.
    app.run("edit.fill", json!({"color": "#2060c0"})).unwrap();
    ensure_texture(&mut app, &ctx, 0, Some(1));
    ensure_texture(&mut app, &ctx, 0, Some(4));
    let rev = app.session.documents()[0].revision;
    assert_eq!((app.canvases[&(id, 1)].tex_revision, app.canvases[&(id, 4)].tex_revision), (rev, rev));
}

#[test]
fn cpu_canvas_shows_crisp_pixels_when_zoomed_in() {
    // #4810: above 100% the CPU canvas shows each pixel as a square, like the GPU canvas's
    // sharp nearest filter; below 100% it still filters. Both the first upload and a
    // damage-rect update carry the sampling, since the renderer takes it from each delta.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
    let ctx = egui::Context::default();
    let tex = ensure_texture(&mut app, &ctx, 0, None).unwrap().0;
    let uploads = |ctx: &egui::Context| {
        let mut delta = ctx.tex_manager().write().take_delta();
        let seen = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos.is_some(), d.options)).collect::<Vec<_>>();
        delta.clear();
        seen
    };
    let crisp = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
    assert_eq!(uploads(&ctx), vec![(false, crisp)]);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
    ensure_texture(&mut app, &ctx, 0, None).unwrap();
    uploads(&ctx);
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None).unwrap().0, tex);
    assert_eq!(uploads(&ctx), vec![(true, crisp)], "a partial update keeps the crisp sampling");
}

/// A device that allows smaller textures than MAX_TEXTURE (2048 on some older GPUs and WebGL
/// contexts, and on the fallback adapter the GPU tests sometimes get under load): the canvas
/// texture is reduced to fit it. A 2600 px document was uploaded at full size, which egui
/// asserts on and the backend can't create.
#[test]
fn cpu_canvas_texture_fits_the_device_limit() {
    for (side, (w, h), want) in [(2048, (2600, 1700), [1300, 850]), (2048, (5000, 2000), [1666, 666]), (8192, (5000, 2000), [2500, 1000])] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": w, "height": h})).unwrap();
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = side);
        let (tex, _) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        let mut delta = ctx.tex_manager().write().take_delta();
        let sizes: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| d.image.size()).collect();
        delta.clear();
        assert_eq!(sizes, vec![want], "{w}x{h} on a {side} px device");
    }
}

#[test]
fn cpu_canvas_of_a_reduced_document_stays_bilinear() {
    // Over MAX_TEXTURE the texture holds one texel per `factor` pixels: a texel is not a
    // document pixel, so showing it as a square would draw blocks that aren't in the image.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": MAX_TEXTURE + 1, "height": 2})).unwrap();
    let ctx = egui::Context::default();
    let (tex, scale) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
    assert!(scale < 1.0);
    let mut delta = ctx.tex_manager().write().take_delta();
    let uploads: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| d.options).collect();
    delta.clear();
    assert_eq!(uploads, vec![TextureOptions::LINEAR]);
}

/// #7568: a document over the texture limit was drawn from one reduced texture, so at 100 %
/// it was magnified and blurred. The view now gets the part in sight at one texel per
/// document pixel, about the window's size, whatever the device's limit.
#[test]
fn cpu_canvas_detail_plan_shows_large_documents_at_full_resolution() {
    let size = [12000, 8000];
    // A 1920×1080 view in the middle (`visible_doc_rect` adds 2 px around it).
    let screen = DRect::new(4998, 2998, 6922, 4082);
    for limit in [16384, 8192, 4096, 2048] {
        let side = clamp_texture_side(limit);
        assert!(texture_factor(size, side) > 1, "the whole document is reduced on a {limit} px device");
        for zoom in [1.0, 2.0, 8.0] {
            let plan = detail_plan(size, side, zoom, screen).unwrap_or_else(|| panic!("{limit} px device at {zoom}"));
            assert_eq!(plan.factor, 1, "one texel per document pixel at {zoom} on a {limit} px device");
            assert!(plan.region.contains_rect(&screen), "{plan:?}");
            assert!(plan.region.width() <= 1924 + 2 * 256 && plan.region.height() <= 1084 + 2 * 256, "about the view's size: {plan:?}");
            assert!(plan.region.width() < side && plan.region.height() < side, "{plan:?}");
        }
    }
    // A small pan reuses the region (no new composite).
    assert_eq!(detail_plan(size, 4096, 1.0, screen), detail_plan(size, 4096, 1.0, screen.translate(8, -8)));
    // A window wider than a small device's limit coarsens rather than exceed it.
    let wide = DRect::new(0, 0, 3000, 2000);
    let plan = detail_plan(size, 2048, 1.0, wide).unwrap();
    assert_eq!(plan.factor, 2);
    assert!(plan.region.width() / 2 < 2048 && plan.region.contains_rect(&wide), "{plan:?}");
    // Zoomed out a little: finer than the reduced whole, as fine as the screen.
    let plan = detail_plan(size, 4096, 0.4, DRect::new(0, 0, 4800, 2700)).unwrap();
    assert_eq!(plan.factor, 2);
    // Zoomed out far enough, the reduced whole is as fine as the screen: nothing to add.
    let whole = DRect::new(0, 0, 12000, 8000);
    assert_eq!(detail_plan(size, 4096, 0.25, whole), None);
    assert_eq!(detail_plan(size, 4096, 1.0 / 3.0, whole), None, "a detail as large as the document would not fit");
    // A document that fits is uploaded whole at full resolution.
    assert_eq!(detail_plan([4000, 3000], 4096, 1.0, screen), None);
    // Nothing of the document in view, or a broken zoom.
    assert_eq!(detail_plan(size, 4096, 1.0, DRect::new(-500, -500, -10, -10)), None);
    for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(detail_plan(size, 4096, zoom, screen), None, "{zoom}");
    }
    assert_eq!(detail_plan([0, 0], 4096, 1.0, screen), None);
    assert_eq!(detail_plan([u32::MAX, 8], 4096, 1.0, screen), None);
}

#[test]
fn cpu_canvas_detail_shows_a_large_documents_own_pixels_at_100_percent() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 3 * MAX_TEXTURE, "height": 8})).unwrap();
    app.run("select.rect", json!({"x": 101, "y": 0, "width": 1, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.max_texture_side = 16384);
    let uploads = |ctx: &egui::Context, tex: egui::TextureId| {
        let mut delta = ctx.tex_manager().write().take_delta();
        let seen: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos, d.options, d.image.clone())).collect();
        delta.clear();
        seen
    };
    let row = |image: &egui::ImageData| {
        let egui::ImageData::Color(image) = image;
        (image.size, image.pixels.clone())
    };
    let view = InView { visible: DRect::new(0, 0, 1000, 8), zoom: 1.0, key: 1 };
    let t = ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap();
    assert!(t.scale < 1.0, "the whole document is reduced");
    let (detail, area) = t.detail.expect("a detail texture at 100 %");
    assert_eq!(area, [0.0, 0.0, 1024.0, 8.0]);
    let crisp = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
    let up = uploads(&ctx, detail);
    assert_eq!(up.len(), 1);
    let (pos, options, image) = &up[0];
    assert_eq!((*pos, *options), (None, crisp));
    let (size, px) = row(image);
    assert_eq!(size, [1024, 8]);
    assert_eq!((px[100], px[101], px[102]), (Color32::WHITE, Color32::BLACK, Color32::WHITE), "the 1 px line stays 1 px");
    // Unchanged: nothing is composited again, and a small pan keeps the region.
    let panned = InView { visible: DRect::new(4, 0, 1004, 8), ..view };
    assert_eq!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(panned)).unwrap().detail, Some((detail, area)));
    assert!(uploads(&ctx, detail).is_empty());
    // The Navigator's call (no view) leaves the view's detail alone.
    ensure_texture(&mut app, &ctx, 0, None).unwrap();
    assert_eq!(app.canvases.values().map(|c| c.detail.len()).sum::<usize>(), 1);
    // An edit in view updates just its texels.
    app.run("select.rect", json!({"x": 300, "y": 0, "width": 1, "height": 8})).unwrap();
    ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap();
    uploads(&ctx, detail);
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    assert_eq!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap().detail.map(|d| d.0), Some(detail));
    let up = uploads(&ctx, detail);
    assert_eq!(up.len(), 1);
    let (pos, options, image) = &up[0];
    assert_eq!(*options, crisp);
    let [x, y] = pos.expect("a partial update");
    let (size, px) = row(image);
    assert!(y == 0 && x < 300 && x + size[0] > 300, "the edit's texels: {x} {size:?}");
    assert_eq!((px[299 - x], px[300 - x]), (Color32::WHITE, Color32::BLACK));
    // Zoomed out, the reduced texture is enough: the detail goes.
    let far = InView { zoom: 0.25, ..view };
    assert!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(far)).unwrap().detail.is_none());
    assert!(app.canvases.values().all(|c| c.detail.is_empty()));
}

#[test]
fn gpu_display_luts_are_per_display() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut app = app_on_two_displays();
    app.set_wgpu(rs);
    let doc = app.session.documents()[0].doc.clone();
    let key = doc.id.0;
    // sRGB on the sRGB display needs no LUT; on the P3 display it does.
    assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
    assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
    let gpu = app.gpu.clone().unwrap();
    let (s1, s4) = (gpu.display_lut_signature(key, 1).unwrap(), gpu.display_lut_signature(key, 4).unwrap());
    assert_ne!(s1.0, s4.0);
    assert!(gpu.has_display_lut(key, 4) && !gpu.has_display_lut(key, 1));
    // Drawing one window leaves the other's LUT alone.
    for _ in 0..3 {
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
    }
    assert_eq!((gpu.display_lut_signature(key, 1), gpu.display_lut_signature(key, 4)), (Some(s1), Some(s4)));
    assert!(gpu.has_display_lut(key, 4));
    // The shared texture's key is the same for both displays.
    assert_eq!(texture_key(canvas_display(&app, &doc, Some(1)).0.as_deref(), &doc), texture_key(canvas_display(&app, &doc, Some(4)).0.as_deref(), &doc));
}

/// The GPU canvas texture of a Multichannel document is filled by the CPU compositor, so its
/// key carries the ink preview (Color Settings › Spot) like the CPU canvas key does.
#[test]
fn gpu_texture_key_follows_the_ink_preview() {
    use super::display::ink_preview_key;
    use photocraft_color::{ColorMode, SampleType};
    let size = photocraft_doc::Size::new(2, 2);
    let inks = Document::new("m", size, ColorMode::Multichannel, SampleType::U8);
    let rgb = Document::new("r", size, ColorMode::Rgb, SampleType::U8);
    assert_ne!(ink_preview_key(&inks), 0);
    assert_eq!(texture_key(None, &inks), ink_preview_key(&inks));
    assert_eq!(texture_key(None, &rgb), 0);
}

/// The navigator's update after an edit (its image with the edit's damage, grown by the
/// effect reach, rendered over it) equals a new thumbnail of the edited document.
#[test]
fn navigator_damage_updates_match_a_full_thumbnail() {
    use photocraft_compose::{render_reduced_damage, thumbnail_buffer, thumbnail_size};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 1500, "height": 900, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 200, "y": 150, "width": 300, "height": 200})).unwrap();
    app.run("edit.fill", json!({"color": "#3070a0"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 30, "size": 20})).unwrap();
    let steps = [
        ("layer.translate", json!({"dx": 37, "dy": -11})),
        ("layer.setProps", json!({"opacity": 0.4})),
        ("paint.stroke", json!({"points": [[700, 500], [900, 520]], "size": 25})),
        ("layer.setProps", json!({"blend": "Multiply"})),
        ("layer.new.layer", json!({})),
    ];
    for (id, p) in steps {
        let before = app.session.active().unwrap().doc.clone();
        app.run(id, p).unwrap();
        let st = app.session.active().unwrap();
        let d = st.last_damage.unwrap_or_else(|| panic!("{id}: a bounded damage"));
        let (w, h) = thumbnail_size(&st.doc, NAVIGATOR_SIDE);
        let mut shown = thumbnail_buffer(&before, NAVIGATOR_SIDE);
        assert_eq!((shown.rect.width(), shown.rect.height()), (w, h));
        if !d.is_empty() {
            let patch = render_reduced_damage(&st.doc, w, h, d.inflate(effect_reach(&st.doc.layers)));
            let r = patch.rect;
            for y in 0..r.height() as usize {
                let at = (r.y0 as usize + y) * w as usize + r.x0 as usize;
                shown.px[at..at + r.width() as usize].copy_from_slice(&patch.px[y * r.width() as usize..][..r.width() as usize]);
            }
        }
        assert!(shown.px == thumbnail_buffer(&st.doc, NAVIGATOR_SIDE).px, "{id}: the updated navigator differs from a new thumbnail");
    }
}

#[test]
fn tab_context_uses_clicked_document_and_existing_close_commands() {
    let items = tab_context_items(2, 3, true);
    assert_eq!(
        items.iter().map(|(_, id, _, _)| *id).collect::<Vec<_>>(),
        [
            "window.arrange.consolidateAllToTabs",
            "file.close",
            "file.closeAll",
            "file.closeOthers",
            "window.arrange.floatInWindow",
            "file.new",
            "file.open",
            "file.revealInFinder"
        ]
    );
    assert_eq!(items[1].2, json!({"document": 2}));
    assert_eq!(items[3].2, json!({"document": 2}));
    assert_eq!(items[2].2, json!({}));
    assert_eq!(items[7].2, json!({"document": 2}));
    assert!(items[7].3, "a saved document can be revealed in the file manager");
    assert!(!tab_context_items(2, 3, false)[7].3, "an unsaved document cannot");
    assert!(!tab_context_items(0, 1, false)[3].3);
    // The window.* rows and file.new / file.open are UI commands; the doc_tab_menu tests check them.
    assert!(
        items
            .iter()
            .filter(|r| r.1.starts_with("file.close") || r.1 == "file.revealInFinder")
            .all(|(_, id, _, _)| photocraft_engine::commands::find(id).is_some())
    );
}

#[test]
fn tab_context_reveal_names_the_platform_file_manager() {
    let (label, id, _, _) = &tab_context_items(0, 1, true)[7];
    assert_eq!((*label, *id), (crate::menus::reveal_label(), "file.revealInFinder"));
}

/// Photoshop's document tab × is after the title on Windows and Linux, before it on macOS (#619).
#[test]
fn document_tab_close_button_sits_on_the_platform_side() {
    use egui::os::OperatingSystem as Os;
    use egui_kittest::kittest::Queryable;
    for (os, after_title) in [(Os::Windows, true), (Os::Nix, true), (Os::Mac, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        // Drawn from the second frame, once the Pro theme is set.
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 40.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.ctx.set_os(os);
        h.state_mut().1 = true;
        h.run_steps(2);
        let tab = h.get_by_label_contains("Untitled @").rect();
        let x = h.get_by_label("Close").rect();
        assert!(tab.contains_rect(x), "{os:?}: the × is inside its tab");
        assert_eq!(x.center().x > tab.center().x, after_title, "{os:?}: × at {x:?} in tab {tab:?}");
        h.get_by_label("Close").click();
        h.run_steps(2);
        assert!(h.state().0.session.documents().is_empty(), "{os:?}: the × closes the document");
    }
}

/// Middle-clicking a document tab closes it (#2570), in both tab strips, and the tab under the
/// pointer is the one that goes even when another document is active.
#[test]
fn middle_clicking_a_tab_closes_that_document() {
    use egui_kittest::kittest::Queryable;
    for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["keep", "doomed", "active"] {
            app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
        }
        app.sync_views();
        assert_eq!(app.session.active_index(), Some(2));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.state_mut().1 = true;
        h.run_steps(2);
        h.get_by_label_contains("doomed").click_button(egui::PointerButton::Middle);
        h.run_steps(2);
        let names: Vec<String> = h.state().0.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        assert_eq!(names, ["keep", "active"], "{kind:?}: the middle-clicked tab closed");
        // A clean document closes straight away; a dirty one would park in `discard`.
        assert!(h.state().0.discard.is_none(), "{kind:?}: nothing to prompt about");
    }
}

/// A middle click goes through the same guarded `file.close` as the ×, so unsaved changes
/// still get the Save / Don’t Save / Cancel prompt instead of being thrown away (#2570).
#[test]
fn middle_clicking_an_edited_tab_prompts_instead_of_discarding() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "name": "edited"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.sync_views();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                tabs(app, ui);
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.state_mut().1 = true;
    h.run_steps(2);
    h.get_by_label_contains("edited").click_button(egui::PointerButton::Middle);
    h.run_steps(2);
    assert!(h.state().0.discard.is_some(), "the unsaved-changes prompt must come up");
    assert_eq!(h.state().0.session.documents().len(), 1, "and the document is still open");
}

#[test]
fn spectrum_document_tabs_meet_canvas_without_a_gap() {
    for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.state_mut().1 = true;
        h.run_steps(3);
        let app = &h.state().0;
        let strip = app.tab_strip.as_ref().expect("a document opens with tabs").rect;
        let gap = app.last_canvas_rect.top() - strip.bottom();
        assert!(gap.abs() < 0.1, "{kind:?}: document strip should touch canvas, got {gap} pt");
    }
}

#[test]
fn large_tabs_preference_increases_tab_strip_height() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 60.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                tabs(app, ui);
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.state_mut().1 = true;
    h.run_steps(2);
    let normal_h = h.get_by_label_contains("Untitled @").rect().height();
    assert_eq!(normal_h, 26.0);

    h.state_mut().0.session.execute("prefs.set", json!({"values": {"workspace.largeTabs": true}})).unwrap();
    h.run_steps(2);
    let large_h = h.get_by_label_contains("Untitled @").rect().height();
    assert_eq!(large_h, 34.0);
}

#[test]
fn tab_close_others_prompts_for_unsaved_nonactive_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "name": "Keep"})).unwrap();
    app.run("file.new", json!({"width": 8, "height": 8, "name": "Edited"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(app.session.active_index(), Some(1));
    let (_, id, params, _) = tab_context_items(0, 2, false)[3].clone();
    crate::menus::invoke(&mut app, &egui::Context::default(), id, params).unwrap();
    assert!(app.discard.is_some(), "close others must ask before discarding the edited tab");
    assert_eq!(app.session.documents().len(), 2);
}

/// The pure half of tab drag-to-reorder (UI-217-6): where a drop lands and which
/// `document.move` target that is.
#[test]
fn tab_drop_index_and_move_target_cover_reorder_drops() {
    let r = |x: f32| Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(100.0, 26.0));
    let placed = vec![(0, r(0.0)), (1, r(100.0)), (2, r(200.0))];
    assert_eq!(tab_drop_index(&placed, 10.0), 0);
    assert_eq!(tab_drop_index(&placed, 160.0), 2);
    assert_eq!(tab_drop_index(&placed, 350.0), 3, "past the end lands after the last shown tab");
    assert_eq!(tab_move_target(0, 3), 2);
    assert_eq!(tab_move_target(2, 0), 0);
    assert_eq!(tab_move_target(1, 1), 1, "before itself: no move");
    assert_eq!(tab_move_target(1, 2), 1, "just after itself: no move");
    assert_eq!(tab_move_target(0, 1), 0);
}

/// Dragging a tab onto another's slot reorders the open documents (UI-217-6), in both the
/// studio and the Photoshop-style strips.
#[test]
fn dragging_a_tab_reorders_the_documents() {
    use egui_kittest::kittest::Queryable;
    for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["a", "b", "c"] {
            app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
        }
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 40.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.state_mut().1 = true;
        h.run_steps(2);
        let b = h.get_by_label_contains("b").rect();
        let a = h.get_by_label_contains("a").rect();
        let target = egui::pos2(a.left() + 2.0, a.center().y);
        h.drag_at(b.center());
        h.run_steps(1);
        h.hover_at(target);
        h.run_steps(1);
        h.drop_at(target);
        h.run_steps(2);
        let names: Vec<String> = h.state().0.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        assert_eq!(names, ["b", "a", "c"], "{kind:?}: the dragged tab took the dropped slot");
    }
}

/// Dragging a tab off the strip floats its document in its own OS window instead of
/// reordering (#3733), in both strips; a short wobble below the strip still reorders.
#[test]
fn dragging_a_tab_off_the_strip_floats_it_in_a_window() {
    use egui_kittest::kittest::Queryable;
    let strip = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(700.0, 26.0));
    assert!(!torn_off(strip, egui::pos2(50.0, 13.0)));
    assert!(!torn_off(strip, egui::pos2(50.0, 26.0 + TEAR_OFF - 1.0)), "a wobble below the strip still reorders");
    assert!(torn_off(strip, egui::pos2(50.0, 26.0 + TEAR_OFF + 1.0)));
    assert!(torn_off(strip, egui::pos2(50.0, -TEAR_OFF - 1.0)), "dragged up, off the window's top");
    for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["a", "b", "c"] {
            app.run("file.new", json!({"width": 8, "height": 8, "name": name})).unwrap();
        }
        app.sync_views();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 300.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    tabs(app, ui);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.state_mut().1 = true;
        h.run_steps(2);
        let b = h.get_by_label_contains("b").rect();
        let target = egui::pos2(b.center().x, b.bottom() + 150.0);
        h.drag_at(b.center());
        h.run_steps(1);
        h.hover_at(target);
        h.run_steps(1);
        h.drop_at(target);
        h.run_steps(2);
        let app = &h.state().0;
        let names: Vec<&str> = app.session.documents().iter().map(|d| d.doc.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"], "{kind:?}: tearing off doesn't reorder");
        assert_eq!(app.ui.windows.iter().map(|w| w.document).collect::<Vec<_>>(), [1], "{kind:?}: b floats in its own window");
        assert_eq!(app.session.active_index(), Some(1), "{kind:?}: the floated document is active");
    }
}

/// An app with a 64 × 64 white document, a clone source and the Clone Stamp at `size` px.
fn clone_overlay_app(size: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("cloneSource.set", json!({"source": [16, 16]})).unwrap();
    app.ui.tool = Tool::CloneStamp;
    app.session.tools.brush.size = size;
    app
}

/// A focused canvas can remain primary after another document becomes active.
#[test]
fn clone_cursor_from_an_inactive_document_cannot_replace_the_active_offset_reference() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp, idx: usize, primary: bool, at: Option<[f64; 2]>| {
        let view = View { zoom: 2.0, center: [32.0, 32.0], doc_size: [64, 64], fit_pending: false, fill_pending: false, ..Default::default() };
        let event =
            at.map_or(egui::Event::PointerGone, |at| egui::Event::PointerMoved(rect.center() + vec2((at[0] as f32 - 32.0) * 2.0, (at[1] as f32 - 32.0) * 2.0)));
        let input = egui::RawInput { screen_rect: Some(rect), events: vec![event], ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, idx, rect, view.clone(), primary);
        });
        output.textures_delta.clear();
    };
    for tool in [Tool::CloneStamp, Tool::Healing] {
        for aligned in [false, true] {
            let mut app = clone_overlay_app(12.0);
            app.background_jobs = false;
            app.ui.extras.rulers = false;
            app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            app.ui.tool = tool;
            app.ui.tool_options.clone_aligned = aligned;
            let document = app.session.active().unwrap().doc.id;
            let slots = app.session.presets.clone.clone();
            let revision = app.session.active().unwrap().revision;
            // Warm up hover detection, then establish the active document's own reference.
            for _ in 0..2 {
                frame(&mut app, 1, true, Some([40.0, 30.0]));
            }
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [40.0, 30.0])));
            assert_eq!(crate::preset_panels::clone_offset_readout(&app), Some([-24.0, -14.0]));
            // A different, still-focused window draws as primary without activating its document.
            frame(&mut app, 0, true, Some([12.0, 24.0]));
            assert_eq!(app.session.active_index(), Some(1));
            assert!(app.hover_doc.is_some(), "the inactive canvas really received hover input");
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [40.0, 30.0])), "{tool:?}/{aligned}: retain the active document's reference");
            assert_eq!(crate::preset_panels::clone_offset_readout(&app), Some([-24.0, -14.0]));
            assert_eq!(crate::preset_panels::clone_offset_edit(&app, [-20.0, -10.0]).unwrap()["anchor"], json!([40.0, 30.0]));
            frame(&mut app, 1, true, Some([44.0, 34.0]));
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [44.0, 34.0])), "the active canvas still updates normally");
            frame(&mut app, 0, false, Some([8.0, 12.0]));
            frame(&mut app, 1, true, None);
            assert_eq!(app.ui.presets_ui.clone_cursor, Some((document, [44.0, 34.0])), "secondary views and leaving the canvas retain the reference");
            assert_eq!(app.session.presets.clone, slots, "hover does not establish a source anchor");
            assert_eq!(app.session.active().unwrap().revision, revision, "hover does not paint");
        }
    }
}

/// One frame of the overlay at (32, 32): its shapes and texture uploads.
fn clone_overlay_frame(ctx: &egui::Context, app: &mut PhotocraftApp, xf: &ViewXform) -> egui::FullOutput {
    ctx.run_ui(Default::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("clone-test")));
        draw_clone_preview(app, &painter, xf, [32.0, 32.0], None);
    })
}

/// The overlay texture a frame uploaded and the alpha at a document pixel, read from the
/// document area the overlay covers (transparent outside it).
fn clone_overlay_alpha(app: &PhotocraftApp, output: &mut egui::FullOutput) -> impl Fn(i32, i32) -> u8 + use<> {
    let cache = app.clone_preview.as_ref().expect("an overlay was drawn");
    let image = output
        .textures_delta
        .set
        .get(&cache.texture.id())
        .and_then(|deltas| deltas.last())
        .map(|d| match &d.image {
            egui::ImageData::Color(c) => c.clone(),
        })
        .expect("the overlay uploads a texture");
    output.textures_delta.clear();
    let rect = cache.rect;
    assert_eq!(image.size, [rect.width() as usize, rect.height() as usize], "the texture covers the overlay's area");
    move |x, y| {
        if x < rect.x0 || y < rect.y0 || x >= rect.x1 || y >= rect.y1 {
            return 0;
        }
        image.pixels[(y - rect.y0) as usize * image.width() + (x - rect.x0) as usize].a()
    }
}

fn clone_overlay_view(side: f32, zoom: f32) -> ViewXform {
    ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(side, side)), zoom, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

#[test]
fn clone_overlay_is_circular_and_respects_visibility() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    let xf = clone_overlay_view(100.0, 1.0);
    let revision = app.session.active().unwrap().revision;
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))), "the overlay is drawn");
    let alpha = clone_overlay_alpha(&app, &mut output);
    assert_eq!(alpha(32, 32), 255, "the centre shows the source");
    assert_eq!(alpha(32 + 8, 32), 255, "inside the hard 10 px radius");
    assert_eq!(alpha(32 + 8, 32 + 8), 0, "the brush square's corner, outside the circle, is clipped");
    assert_eq!(alpha(32 + 12, 32), 0, "past the hard tip's rim");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert!(app.session.presets.clone.active().anchor.is_none());
    let ordinary_key = app.clone_preview.as_ref().unwrap().key.clone();
    app.ui.view.pattern_preview = true;
    let mut periodic_output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_ne!(app.clone_preview.as_ref().unwrap().key, ordinary_key, "toggling Pattern Preview invalidates source pixels");
    assert!(!periodic_output.textures_delta.set.is_empty(), "the overlay texture is refreshed without a document edit");
    periodic_output.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().revision, revision);
    app.session.presets.clone.overlay.show = false;
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    output.textures_delta.clear();
    assert!(output.shapes.iter().all(|s| !matches!(s.shape, egui::Shape::Mesh(_))));
}

/// A soft brush previews soft: the overlay fades towards the rim with the brush's hardness, as
/// the stroke it previews does, and reaches past the radius as a soft tip paints. A brush
/// change redraws an overlay that hasn't moved.
#[test]
fn clone_overlay_follows_the_brush_hardness() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    let hard = clone_overlay_alpha(&app, &mut output);
    app.session.tools.brush.hardness = 0.0;
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    let soft = clone_overlay_alpha(&app, &mut output);
    assert!(hard(32, 32) == 255 && soft(32, 32) > 240, "both show the source at the centre: {} {}", hard(32, 32), soft(32, 32));
    assert_eq!(hard(32 + 8, 32), 255);
    assert!(soft(32 + 8, 32) < 200, "a soft tip fades before its rim: {}", soft(32 + 8, 32));
    assert!(soft(32 + 4, 32) > soft(32 + 8, 32), "and fades outwards");
    assert!(hard(32 + 12, 32) == 0 && soft(32 + 13, 32) > 0, "a soft tip reaches past the radius, a hard one doesn't");
    // The largest brush the overlay draws still shows at its softest.
    let mut big = clone_overlay_app(1000.0);
    big.session.tools.brush.hardness = 0.0;
    let mut output = clone_overlay_frame(&ctx, &mut big, &xf);
    assert_eq!(clone_overlay_alpha(&big, &mut output)(32, 32), 255, "a 1000 px soft brush previews too");
}

#[test]
fn clone_overlay_opacity_and_flow_match_a_stamp_on_another_layer() {
    let ctx = egui::Context::default();
    let xf = clone_overlay_view(100.0, 1.0);
    for depth in [8, 16, 32] {
        for clipped in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
            crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
            app.run("layer.new.layer", json!({})).unwrap();
            app.ui.tool = Tool::CloneStamp;
            app.ui.tool_options.clone_sample = "all".into();
            app.session.tools.brush.size = 20.0;
            app.session.presets.clone.overlay.clipped = clipped;
            // Settings change under a stationary cursor, including after a cached full-strength preview.
            for (opacity, flow) in [(1.0, 1.0), (0.5, 1.0), (1.0, 0.25), (0.5, 0.4)] {
                let mut cached = clone_overlay_frame(&ctx, &mut app, &xf);
                cached.textures_delta.clear();
                app.run("tools.setBrush", json!({"size": 20, "hardness": 1.0, "opacity": opacity, "flow": flow})).unwrap();
                let revision = app.session.active().unwrap().revision;
                let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
                let tint = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Mesh(mesh) => mesh.vertices.first().map(|v| v.color.a()),
                        _ => None,
                    })
                    .expect("the overlay mesh");
                // An unclipped preview may reuse its texture when only tint changes.
                if output.textures_delta.set.is_empty() {
                    app.clone_preview = None;
                    output = clone_overlay_frame(&ctx, &mut app, &xf);
                }
                let alpha = clone_overlay_alpha(&app, &mut output)(32, 32) as f32 / 255.0 * tint as f32 / 255.0;
                assert_eq!(app.session.active().unwrap().revision, revision, "hovering is read-only");
                let mut params = crate::retouch_ui::clone_params(&app).unwrap();
                params["points"] = json!([[32, 32]]);
                app.run("paint.cloneStamp", params).unwrap();
                let st = app.session.active().unwrap();
                let printed = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 32)[3];
                assert!((printed - opacity * flow).abs() < 0.01, "opacity={opacity} flow={flow}: printed {printed}");
                assert!((alpha - printed).abs() < 0.01, "{depth} clipped={clipped} opacity={opacity} flow={flow}: preview {alpha}, print {printed}");
                assert!(app.session.undo());
            }
        }
    }
}

#[test]
fn clone_overlay_separate_opacity_setting_still_multiplies_the_brush_opacity() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.opacity = 0.5;
    app.session.presets.clone.overlay.opacity = 40.0;
    let mut output = clone_overlay_frame(&ctx, &mut app, &clone_overlay_view(100.0, 1.0));
    let alpha = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Mesh(mesh) => mesh.vertices.first().map(|v| v.color.a()),
            _ => None,
        })
        .unwrap();
    output.textures_delta.clear();
    assert_eq!(alpha, 51, "50% brush opacity times 40% overlay opacity");
}

#[test]
fn clone_overlay_refreshes_when_the_source_layer_changes_at_the_same_coordinates() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    app.run("layer.new.layer", json!({})).unwrap();
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(32, 32), 255, "the picked white source survives the layer switch");
    crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(32, 32), 0, "picking the transparent layer refreshes the cached overlay");
}

#[test]
fn clone_overlay_strength_changes_leave_the_healing_preview_at_full_strength() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.opacity = 0.5;
    app.session.tools.brush.flow = 0.4;
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert!((i32::from(clone_overlay_alpha(&app, &mut output)(32, 32)) - 102).abs() <= 1);
    app.ui.tool = Tool::Healing;
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(32, 32), 255, "Healing Brush still previews its full-strength texture");
}

fn clone_overlay_center(app: &PhotocraftApp, output: &mut egui::FullOutput) -> Color32 {
    let cache = app.clone_preview.as_ref().unwrap();
    let upload = output.textures_delta.set.get(&cache.texture.id()).and_then(|d| d.last()).expect("the source preview was refreshed");
    let egui::ImageData::Color(image) = &upload.image;
    let pixel = image.pixels[(32 - cache.rect.y0) as usize * image.width() + (32 - cache.rect.x0) as usize];
    output.textures_delta.clear();
    pixel
}

#[test]
fn clone_overlay_repicking_on_the_original_layer_updates_preview_and_both_layers() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(12.0);
    let source = app.session.active().unwrap().active_layer.unwrap();
    for (point, color) in [([16, 16], "#ff0000"), ([8, 8], "#0000ff")] {
        app.run("paint.pencil", json!({"points": [point], "size": 6, "color": color})).unwrap();
    }
    crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
    app.run("layer.new.layer", json!({})).unwrap();
    let target = app.session.active().unwrap().active_layer.unwrap();
    app.run("tools.setBrush", json!({"size": 6, "hardness": 1, "opacity": 1, "flow": 1})).unwrap();
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_center(&app, &mut output), Color32::RED);
    let stamp = |app: &mut PhotocraftApp| {
        let mut params = crate::retouch_ui::clone_params(app).unwrap();
        params["points"] = json!([[32, 32]]);
        app.run("paint.cloneStamp", params).unwrap();
    };
    stamp(&mut app);
    app.run("layer.select", json!({"layer": source.0})).unwrap();
    crate::retouch_ui::set_source(&mut app, 8.0, 8.0);
    stamp(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layer(source).unwrap().surface().unwrap().rgba(32, 32), [0.0, 0.0, 1.0, 1.0]);
    app.run("layer.select", json!({"layer": target.0})).unwrap();
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_center(&app, &mut output), Color32::BLUE);
    stamp(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layer(target).unwrap().surface().unwrap().rgba(32, 32), [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn clone_overlay_between_documents_updates_after_source_edits_and_blends_with_the_destination() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(12.0);
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("paint.pencil", json!({"points": [[16,16]], "size": 8, "color": "#ff0000"})).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("paint.pencil", json!({"points": [[32,32]], "size": 16, "color": "#000000"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"size": 8, "hardness": 1, "opacity": 1, "flow": 1})).unwrap();
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_center(&app, &mut output), Color32::RED);
    app.run("cloneSource.overlay", json!({"blend": "difference"})).unwrap();
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_center(&app, &mut output), Color32::RED, "Difference uses the black destination, not the white source document");
    let revision = app.session.active().unwrap().revision;
    assert!(app.session.set_active(0));
    app.run("paint.pencil", json!({"points": [[16,16]], "size": 8, "color": "#00ff00"})).unwrap();
    assert!(app.session.set_active(1));
    assert_eq!(app.session.active().unwrap().revision, revision);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_center(&app, &mut output), Color32::GREEN, "editing another document invalidates the cached source");
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, mods);
    assert!(crate::retouch_ui::source_marker_point(&app).is_none(), "the source crosshair belongs to the other document");
    tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, mods);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 32), [0.0, 1.0, 0.0, 1.0]);
    app.run("cloneSource.select", json!({"index": 1})).unwrap();
    assert!(crate::retouch_ui::clone_params(&app).is_none(), "an empty slot cannot reuse coordinates from another document");
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    tool_event(&mut app, ToolEvent::Down { x: 48.0, y: 48.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 48.0, y: 48.0 }, mods);
    assert_eq!(app.session.active().unwrap().doc, before, "painting with an empty slot cannot sample stale coordinates");
    assert_eq!(app.session.active().unwrap().revision, revision, "the refused gesture does not create a history edit");
}

/// A scaled Clone Source reads a larger source area than the overlay covers: a large brush's
/// overlay shrinks around the pointer until that area fits the sampling budget, rather than
/// vanishing.
#[test]
fn clone_overlay_of_a_scaled_source_stays_within_the_sampling_budget() {
    let ctx = egui::Context::default();
    let xf = clone_overlay_view(100.0, 1.0);
    for (size, scale) in [(600.0, 200), (1000.0, 300), (1000.0, 50)] {
        let mut app = clone_overlay_app(size);
        app.run("cloneSource.set", json!({"width": scale, "height": scale})).unwrap();
        let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
        assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))), "{size} px at {scale} %: the overlay shows");
        assert_eq!(clone_overlay_alpha(&app, &mut output)(32, 32), 255, "{size} px at {scale} %: the source under the pointer");
    }
}

/// The Healing Brush shares the Clone Stamp's source and overlay (Photoshop's Clone Source
/// panel drives both); other tools show none.
#[test]
fn clone_overlay_shows_for_the_healing_brush_too() {
    let ctx = egui::Context::default();
    let xf = clone_overlay_view(100.0, 1.0);
    for (tool, shown) in [(Tool::Healing, true), (Tool::CloneStamp, true), (Tool::Brush, false), (Tool::SpotHealing, false)] {
        let mut app = clone_overlay_app(20.0);
        app.ui.tool = tool;
        let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
        assert_eq!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))), shown, "{tool:?}");
        if shown {
            assert_eq!(clone_overlay_alpha(&app, &mut output)(32, 32), 255, "{tool:?}: the source shows under the brush");
        }
        output.textures_delta.clear();
    }
}

/// Pattern Preview changes the Healing Brush source overlay at a stationary cursor.
#[test]
fn clone_healing_overlay_wraps_with_the_live_texture_at_source_edges() {
    let ctx = egui::Context::default();
    let xf = clone_overlay_view(100.0, 1.0);
    let mut app = clone_overlay_app(12.0);
    app.ui.tool = Tool::Healing;
    app.run("cloneSource.set", json!({"source": [0,0]})).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(30, 32), 0);
    app.ui.view.pattern_preview = true;
    assert_eq!(crate::retouch_ui::clone_params(&app).unwrap()["wrap"], true);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(30, 32), 255, "the repeated source is visible before painting");
    let live = photocraft_engine::retouch_cmds::LiveRetouch::begin(
        &app.session,
        "paint.healingBrush",
        &json!({"points": [[32,32]], "size": 12, "hardness": 100, "wrap": true}),
    )
    .unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(live.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 32), [1.0; 4]);
    app.ui.view.pattern_preview = false;
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    assert_eq!(clone_overlay_alpha(&app, &mut output)(30, 32), 0);
}

/// Unsupported cross-document targets never reuse a cached pixel-layer source overlay.
#[test]
fn clone_overlay_suppresses_cross_document_masks_and_channels() {
    for tool in [Tool::CloneStamp, Tool::Healing] {
        for mask in [false, true] {
            let ctx = egui::Context::default();
            let xf = clone_overlay_view(100.0, 1.0);
            let mut app = clone_overlay_app(12.0);
            app.ui.tool = tool;
            app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
            if mask {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
            } else {
                app.run("channel.new", json!({"fill": "white"})).unwrap();
            }
            app.run("channel.target", json!({"channel": "composite"})).unwrap();
            app.ui.mask_target = false;
            let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
            assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))));
            output.textures_delta.clear();
            let revision = app.session.active().unwrap().revision;
            if mask {
                app.ui.mask_target = true;
            } else {
                app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Alpha(0);
            }
            assert_eq!(app.session.active().unwrap().revision, revision, "only the target changed");
            let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
            assert!(!output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))), "{tool:?}, mask={mask}: the gesture has no supported source");
            output.textures_delta.clear();
            app.ui.mask_target = false;
            app.run("channel.target", json!({"channel": "composite"})).unwrap();
            let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
            assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Mesh(_))), "returning to pixels restores the source overlay");
            output.textures_delta.clear();
        }
    }
}

#[test]
fn clone_overlay_keeps_magnified_pixels_crisp_on_upload_and_refresh() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    let xf = clone_overlay_view(200.0, 4.0);
    for invert in [false, true] {
        app.session.presets.clone.overlay.invert = invert;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            draw_clone_preview(&mut app, ui.painter(), &xf, [32.0, 32.0], None);
        });
        let id = app.clone_preview.as_ref().unwrap().texture.id();
        let upload = output.textures_delta.set.get(&id).and_then(|d| d.last()).expect("clone overlay upload");
        let options = upload.options;
        output.textures_delta.clear();
        assert_eq!(options.magnification, egui::TextureFilter::Nearest, "invert={invert}");
        assert_eq!(options.minification, egui::TextureFilter::Linear, "zoomed-out previews remain filtered");
    }
}

#[test]
fn clone_overlay_blend_mode_mixes_with_the_image_under_the_brush() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    let xf = clone_overlay_view(100.0, 1.0);
    // The overlay texel at the brush centre for `mode`.
    let mut centre = |mode: &str| {
        app.run("cloneSource.overlay", json!({"blend": mode})).unwrap();
        let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
        let cache = app.clone_preview.as_ref().unwrap();
        let image = output.textures_delta.set.get(&cache.texture.id()).and_then(|d| d.last()).map(|d| match &d.image {
            egui::ImageData::Color(c) => c.clone(),
        });
        output.textures_delta.clear();
        let image = image.expect("the overlay uploads a texture");
        let rect = cache.rect;
        image.pixels[(32 - rect.y0) as usize * image.width() + (32 - rect.x0) as usize]
    };
    // A white source over the white image: Normal shows white, Difference black.
    assert_eq!(centre("normal"), Color32::WHITE);
    assert_eq!(centre("difference"), Color32::BLACK, "Difference turns black where source and image match");
    assert_eq!(centre("lighten"), Color32::WHITE);
}

/// A shaped tip clips the Clone Source overlay to its outline, not to the brush circle.
#[test]
fn clone_overlay_is_clipped_to_a_shaped_tip() {
    let ctx = egui::Context::default();
    let mut app = clone_overlay_app(20.0);
    app.session.tools.brush.roundness = 0.25;
    let xf = clone_overlay_view(100.0, 1.0);
    let mut output = clone_overlay_frame(&ctx, &mut app, &xf);
    let alpha = clone_overlay_alpha(&app, &mut output);
    assert_eq!(alpha(32, 32), 255, "the tip's centre shows the source");
    assert_eq!(alpha(32, 24), 0, "inside the circle but outside the flat tip: clipped");
    assert!(alpha(25, 32) > 240, "the tip's long axis stays: {}", alpha(25, 32));
}

#[test]
fn wayland_start_screen_hint_offers_file_drop() {
    let x11 = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let wayland = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { is_wayland: true, ..Default::default() });
    assert!(start_screen_drop_hint(&x11).starts_with("Drop"));
    assert!(start_screen_drop_hint(&wayland).starts_with("Drop"));
}

/// A tiling window manager can hand PhotoCraft a canvas narrower than the Home card (460
/// points): the two buttons must stay inside the window instead of being clipped at its edges.
#[test]
fn start_screen_buttons_stay_inside_a_narrow_canvas() {
    use egui_kittest::kittest::Queryable;
    for width in [390.0, 420.0, 520.0] {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(3);
        for label in ["New file", "Open…"] {
            let nodes: Vec<_> = h.query_all_by_label(label).collect();
            assert!(!nodes.is_empty(), "`{label}` is rendered in a {width}-point-wide canvas");
            for node in nodes {
                let r = node.rect();
                assert!(r.left() >= 0.0 && r.right() <= width, "`{label}` is clipped in a {width}-point-wide canvas: {r:?}");
            }
        }
    }
}

/// #4382: Interface › Show home recent files off hides the Home screen's list, not the recents.
#[test]
fn home_recent_list_hides_when_the_preference_is_off() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.push_recent("/tmp/photocraft-217/hidden.psd");
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    assert!(h.query_by_label("hidden.psd").is_some(), "listed by default");
    h.state_mut().0.run("prefs.set", serde_json::json!({"values": {"interface.showHomeRecentFiles": false}})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("hidden.psd").is_none(), "hidden when off");
    assert_eq!(h.state().0.ui.recent_files.len(), 1, "the list itself is kept");
}

#[test]
fn start_screen_narrow_window_renders_actions_row() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.push_recent("/tmp/photocraft-narrow/test.psd");
    // Window narrower than 640px (e.g. 500px width split-screen)
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 600.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    assert!(h.query_by_label("New file").is_some(), "New file button must be present in narrow mode");
    assert!(h.query_all_by_label("Open…").next().is_some(), "Open… button must be present in narrow mode");
}

/// The Home screen's × takes a file off Recent without opening it, and its right-click menu
/// removes one entry or clears the list; the preferences keep the change (#2691).
#[test]
fn home_recent_entries_can_be_removed() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for name in ["c.tif", "b.psd", "a.png"] {
        app.push_recent(&format!("/tmp/photocraft-2691/{name}"));
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    let listed = |h: &egui_kittest::Harness<'_, (PhotocraftApp, bool)>| {
        let app = &h.state().0;
        assert_eq!(app.session.prefs().file_handling.recent_files, app.ui.recent_files, "stored in the preferences");
        app.ui.recent_files.iter().map(|p| crate::file_open::display_name(p)).collect::<Vec<_>>()
    };
    assert_eq!(listed(&h), ["a.png", "b.psd", "c.tif"]);
    assert!(h.query_by_label("Remove b.psd from Recent").is_none(), "the × shows on the hovered row only");

    h.get_by_label("b.psd").hover();
    h.run_steps(2);
    h.get_by_label("Remove b.psd from Recent").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a.png", "c.tif"]);
    assert!(h.state().0.session.documents().is_empty(), "the file was not opened");

    h.get_by_label("c.tif").click_secondary();
    h.run_steps(2);
    h.get_by_label("Remove from Recent").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a.png"]);

    h.get_by_label("a.png").click_secondary();
    h.run_steps(2);
    h.get_by_label("Clear Recent Files").click();
    h.run_steps(2);
    assert!(listed(&h).is_empty());
    assert!(h.state().0.session.documents().is_empty());
}

#[test]
fn pointer_moves_are_bounded_by_the_press_and_release() {
    use egui::{Event, PointerButton, pos2};
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let events = [
        Event::PointerMoved(pos2(1.0, 0.0)),
        button(pos2(2.0, 0.0), true),
        Event::PointerMoved(pos2(3.0, 0.0)),
        Event::PointerMoved(pos2(4.0, 0.0)),
        button(pos2(5.0, 0.0), false),
        Event::PointerMoved(pos2(6.0, 0.0)),
    ];
    // The press is in this frame: only the moves between press and release count.
    assert_eq!(pointer_moves(&events, PointerButton::Primary, false), vec![pos2(3.0, 0.0), pos2(4.0, 0.0)]);
    // The button was already down when the frame began: the move before the release counts,
    // the one after it does not.
    assert_eq!(pointer_moves(&events, PointerButton::Primary, true), vec![pos2(1.0, 0.0), pos2(3.0, 0.0), pos2(4.0, 0.0)]);
}

#[test]
fn freehand_tools_are_the_ones_that_follow_a_path() {
    for t in [
        Tool::Brush,
        Tool::Pencil,
        Tool::MixerBrush,
        Tool::ColorReplacement,
        Tool::Eraser,
        Tool::BackgroundEraser,
        Tool::CloneStamp,
        Tool::PatternStamp,
        Tool::Smudge,
        Tool::Dodge,
        Tool::Lasso,
        Tool::QuickSelection,
    ] {
        assert!(freehand_tool(t), "{t:?} paints or retouches along a path");
    }
    for t in [Tool::Move, Tool::Eyedropper, Tool::Gradient, Tool::Crop, Tool::RectMarquee, Tool::Type, Tool::Hand] {
        assert!(!freehand_tool(t), "{t:?} is driven by the pointer's latest position");
    }
}

#[test]
fn clone_stamp_centre_appears_only_with_option() {
    assert!(!brush_tip_centre(Tool::CloneStamp, false, false));
    assert!(brush_tip_centre(Tool::CloneStamp, true, false));
    assert!(brush_tip_centre(Tool::Healing, true, false));
    assert!(!brush_tip_centre(Tool::Healing, false, false));
    assert!(brush_tip_centre(Tool::CloneStamp, false, true));
    assert!(!brush_tip_centre(Tool::PatternStamp, true, false), "Pattern Stamp keeps the brush cursor; Option does not switch it to a clone-source mark");
    assert!(!brush_tip_centre(Tool::QuickSelection, false, true));
    assert!(brush_tip_centre(Tool::BackgroundEraser, false, false));
}

#[test]
fn brush_tip_centre_follows_the_crosshair_preference_at_any_size() {
    // A large tip used to show the centre mark whatever Show Crosshair in Brush Tip said.
    for tool in [Tool::Brush, Tool::Eraser, Tool::PatternStamp, Tool::Dodge] {
        assert!(!brush_tip_centre(tool, false, false), "{tool:?}: off");
        assert!(brush_tip_centre(tool, false, true), "{tool:?}: on");
    }
}

#[test]
fn view_transform_roundtrip() {
    for flip in [false, true] {
        let xf =
            ViewXform { rect: Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0)), zoom: 2.5, center: [320.0, 240.0], flip, rotation: 0.0, aspect: 1.0 };
        let s = xf.to_screen(10.0, 20.0);
        let d = xf.to_doc(s);
        assert!((d[0] - 10.0).abs() < 1e-3 && (d[1] - 20.0).abs() < 1e-3);
        assert_eq!(xf.to_screen(320.0, 240.0), xf.rect.center());
        // Flipped, document x grows to the left.
        assert_eq!(xf.to_screen(330.0, 240.0).x > xf.rect.center().x, !flip);
        let r = xf.doc_rect(DRect::new(0, 0, 10, 10));
        assert!(r.width() > 0.0 && r.height() > 0.0);
    }
}

#[test]
fn eyedropper_ring_colors_pair_the_sampled_and_current_colours() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
    app.run("tools.setColors", json!({"foreground": [0.0, 0.2, 1.0, 1.0]})).unwrap();
    let (new, current) = super::eyedropper_ring_colors(&mut app, 10.0, 10.0).expect("over the red square");
    let near = |c: egui::Color32, r: u8, g: u8, b: u8| {
        let d = |a: u8, b: u8| i32::from(a).abs_diff(i32::from(b));
        d(c.r(), r) <= 1 && d(c.g(), g) <= 1 && d(c.b(), b) <= 1
    };
    assert!(near(new, 255, 0, 0), "{new:?}");
    assert!(near(current, 0, 51, 255), "{current:?}");
    // Transparency and points outside the canvas have nothing to compare.
    assert!(super::eyedropper_ring_colors(&mut app, 30.0, 10.0).is_none());
    assert!(super::eyedropper_ring_colors(&mut app, -1.0, 10.0).is_none());
}

#[test]
fn eyedropper_drag_updates_the_sampled_colour() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 20, 20], "fill": "#00ff00"})).unwrap();
    app.ui.tool = Tool::Eyedropper;

    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(app.session.tools.foreground[0] > 0.99 && app.session.tools.foreground[1] < 0.01);

    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(app.session.tools.foreground[1] > 0.99 && app.session.tools.foreground[0] < 0.01);
}

/// #10074: painting a group looked as if it worked (the stroke's trail followed the pointer) and
/// vanished on release with only a status-bar note. Photoshop refuses at the press with a
/// message; so does PhotoCraft, and nothing is painted or drawn.
#[test]
fn painting_a_group_is_refused_at_the_press_with_a_message() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().active_layer.unwrap();
    assert!(app.session.active().unwrap().doc.layer(group).unwrap().is_group());
    let errors = |app: &PhotocraftApp| -> Vec<String> {
        app.ui
            .dialogs
            .iter()
            .filter(|d| d.kind == crate::state::DialogKind::Error)
            .map(|d| d.fields.get("message").and_then(|m| m.as_str()).unwrap_or_default().to_string())
            .collect()
    };
    app.ui.tool_options.gradient_classic = true;
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::CloneStamp, Tool::PaintBucket, Tool::MagicEraser, Tool::RedEye, Tool::Gradient] {
        app.ui.dialogs.clear();
        app.ui.tool = tool;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        // Brush and Classic gradient use the measured alerts (#11898, #12017).
        let want = match tool {
            Tool::Brush => "Could not use the brush tool because the pixels in a group cannot be edited without merging the group.".to_string(),
            Tool::Gradient => "Could not use the gradient tool because the pixels in a group cannot be edited without merging the group.".to_string(),
            _ => format!("Could not use the {} because the target layer is a group.", tool.label()),
        };
        assert_eq!(errors(&app), [want], "{tool:?}");
        assert!(app.drag.is_none() && app.live_stroke.is_none(), "{tool:?} starts no stroke");
        tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, egui::Modifiers::NONE);
        assert!(app.trail.is_none(), "{tool:?} draws no trail");
    }
    // Tools that don't paint the group's pixels are unaffected.
    app.ui.dialogs.clear();
    app.ui.tool = Tool::RectMarquee;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(errors(&app).is_empty());
    tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 15.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 15.0 }, egui::Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.selection.is_some(), "a marquee still selects with a group active");
    app.run("select.deselect", json!({})).unwrap();
    // A group's mask can be painted: targeting it paints, without the message.
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.sync_mask_targets();
    assert_eq!(paint_target(&app), json!("mask"));
    let mask_at = |app: &PhotocraftApp, x: i32| {
        let st = app.session.active().unwrap();
        st.doc.layer(group).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, 10, 0)
    };
    let before = mask_at(&app, 15);
    // Black on a reveal-all mask hides: a stroke that paints shows.
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.ui.tool = Tool::Brush;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(errors(&app).is_empty(), "the mask is paintable");
    assert!(app.drag.is_some(), "the stroke starts on the mask");
    tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 10.0 }, egui::Modifiers::NONE);
    assert_ne!(mask_at(&app, 15), before, "the stroke painted the group's mask");
}

/// #4298: an adjustment layer has no pixels, so after its icon thumbnail is clicked (the
/// Layers panel no longer targets the mask) Fill, the fill keys and the Paint Bucket still
/// edit its mask, as in Photoshop, instead of failing with "not a pixel layer".
#[test]
fn fills_on_an_adjustment_layer_edit_its_mask_when_the_icon_is_targeted() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    // Click the Curves icon thumbnail: the panel targets the layer, not its mask.
    app.ui.mask_target = false;
    app.sync_mask_targets();
    assert!(!app.ui.mask_target, "the panel's target is the icon");
    let mask_at = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(10, 10, 0)
    };
    assert_eq!(paint_target(&app), json!("mask"));
    assert!(crate::menus::is_enabled(&app, "edit.fill"), "Edit > Fill (⇧F5) is enabled");
    assert!(crate::menus::is_enabled(&app, "edit.fillForeground"));
    app.ui.tool = Tool::PaintBucket;
    app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
    assert_eq!(mask_at(&app), 0.0, "the Paint Bucket filled the mask black");
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    assert_eq!(mask_at(&app), 1.0, "Edit > Fill filled the mask white");
    assert!(!app.ui.mask_target, "filling doesn't move the panel's target");
    // A pixel layer with an untargeted mask still paints its pixels.
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = false;
    assert_eq!(paint_target(&app), json!("pixels"));
    assert!(!app.layer_mask_targeted());
}

#[test]
fn alt_with_a_painting_tool_samples_the_foreground_instead_of_painting() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 20], "fill": "#ff0000"})).unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 20, 20], "fill": "#00ff00"})).unwrap();
    let alt = egui::Modifiers::ALT;
    for tool in [Tool::Brush, Tool::Pencil, Tool::Gradient, Tool::PaintBucket] {
        app.ui.tool = tool;
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
        let rev = app.session.active().unwrap().revision;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, alt);
        assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "{tool:?}: ⌥-click sets the foreground");
        // A drag keeps sampling, even after ⌥ is let go; the background is untouched.
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "{tool:?}");
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 10.0 }, egui::Modifiers::NONE);
        assert_eq!(app.session.tools.background, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(app.session.active().unwrap().revision, rev, "{tool:?}: sampling must not edit the document");
        assert!(app.drag.is_none() && !app.alt_sampling);
    }
    // Agents get the same through `ui.pointer` (MCP `ui_pointer`).
    app.ui.tool = Tool::Brush;
    let ctx = egui::Context::default();
    let events = json!([{"kind": "down", "x": 10, "y": 10}, {"kind": "up", "x": 10, "y": 10}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"modifiers": {"alt": true}, "events": events}));
    let _ = crate::control::handle(&mut app, &ctx, &req);
    assert_eq!(app.session.tools.foreground, [1.0, 0.0, 0.0, 1.0]);
    app.run("tools.setColors", json!({"foreground": [0.0, 1.0, 0.0, 1.0]})).unwrap();
    // Without ⌥ the Brush paints again, and ⌥ pressed mid-stroke doesn't switch to sampling.
    app.run("layer.new.layer", json!({})).unwrap();
    let rev = app.session.active().unwrap().revision;
    tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 5.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 5.0 }, alt);
    assert_eq!(app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0]);
    assert!(app.session.active().unwrap().revision > rev, "the stroke was painted");
    // Control+Alt stays the brush-resize gesture, never a sample.
    assert!(!alt_samples(Tool::Brush, egui::Modifiers { alt: true, ctrl: true, ..Default::default() }));
    assert!(!alt_samples(Tool::Eraser, alt), "⌥ with the Eraser is not the Eyedropper");
}

/// #2078: the opaque mask-view texture must show the live stroke, not cover it with
/// the committed mask until the mouse is released.
#[test]
fn mask_view_uploads_each_live_stroke_before_release() {
    for depth in [8, 16, 32] {
        for mode in ["gray", "overlay"] {
            for (width, factor) in [(120u32, 1u32), (513, 3)] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": width, "height": 60, "depth": depth})).unwrap();
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                app.run("view.layerMask", json!({"mode": mode})).unwrap();
                app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
                app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
                app.ui.tool = Tool::Brush;
                let ctx = egui::Context::default();
                ctx.input_mut(|i| i.max_texture_side = 256);
                let tex = crate::channel_view::ensure(&mut app, &ctx, 0, None).unwrap();
                ctx.tex_manager().write().take_delta().clear();
                let revision = app.session.documents()[0].revision;
                let steps = app.session.documents()[0].history.past_len();
                let layer = app.session.documents()[0].active_layer.unwrap();
                let original = app.session.documents()[0].doc.layer(layer).unwrap().clone();
                tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
                for x in [30.0, 50.0, 70.0] {
                    tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    let shown = display_doc(&mut app, 0).0;
                    assert!(shown.layer(layer).unwrap().mask.as_ref().unwrap().value(x as i32, 30) < 0.1);
                    assert_eq!(crate::channel_view::ensure(&mut app, &ctx, 0, None), Some(tex));
                    let mut delta = ctx.tex_manager().write().take_delta();
                    let uploads = delta.set.get(&tex).unwrap_or_else(|| panic!("{depth}-bit {mode}: live mask texture must upload before release at {x}"));
                    assert_eq!(uploads.len(), 1);
                    let upload = &uploads[0];
                    let pos = upload.pos.expect("only the damaged region is uploaded");
                    let egui::ImageData::Color(image) = &upload.image;
                    assert!(image.pixels.len() < (width.div_ceil(factor) * 60u32.div_ceil(factor)) as usize, "the whole mask is not uploaded");
                    let f = factor as i32;
                    let r = DRect::new(pos[0] as i32 * f, pos[1] as i32 * f, (pos[0] + image.size[0]) as i32 * f, (pos[1] + image.size[1]) as i32 * f)
                        .intersect(&shown.bounds());
                    let expected = crate::channel_view::render(
                        &shown,
                        &app.session.documents()[0].channel_view,
                        r,
                        factor,
                        crate::channel_view::Look::default(),
                        app.session.documents()[0].active_layer,
                    )
                    .unwrap();
                    assert_eq!(image.pixels, expected, "uploaded mask values match the live stroke");
                    delta.clear();
                    assert_eq!(app.session.documents()[0].revision, revision);
                    assert_eq!(app.session.documents()[0].history.past_len(), steps);
                    assert_eq!(app.session.documents()[0].doc.layer(layer).unwrap(), &original);
                    crate::channel_view::ensure(&mut app, &ctx, 0, None);
                    assert!(ctx.tex_manager().write().take_delta().set.is_empty(), "idle frame does not upload");
                }
                tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, egui::Modifiers::NONE);
                let st = &app.session.documents()[0];
                assert_eq!(st.history.past_len(), steps + 1);
                assert!(st.doc.layer(layer).unwrap().mask.as_ref().unwrap().value(50, 30) < 0.1);
                assert_eq!(st.doc.layer(layer).unwrap().content, original.content);
                crate::channel_view::ensure(&mut app, &ctx, 0, None);
                let mut delta = ctx.tex_manager().write().take_delta();
                // The commit takes the live stroke's pixels, which the texture already shows: at
                // most a partial upload, never the whole mask again.
                assert!(delta.set.get(&tex).is_none_or(|u| u.iter().all(|u| u.pos.is_some())), "commit keeps its partial upload");
                delta.clear();
                app.session.undo();
                assert_eq!(app.session.documents()[0].doc.layer(layer).unwrap(), &original);
                crate::channel_view::ensure(&mut app, &ctx, 0, None);
                let mut delta = ctx.tex_manager().write().take_delta();
                let upload = &delta.set.get(&tex).unwrap()[0];
                let egui::ImageData::Color(image) = &upload.image;
                let st = &app.session.documents()[0];
                // Undo redraws only the tiles the stroke changed (#9329): a partial upload of them.
                let f = factor as i32;
                let pos = upload.pos.expect("undo uploads only the changed tiles, not the whole mask");
                let r = DRect::new(pos[0] as i32 * f, pos[1] as i32 * f, (pos[0] + image.size[0]) as i32 * f, (pos[1] + image.size[1]) as i32 * f)
                    .intersect(&st.doc.bounds());
                // It covers the stroke's span (x 10..70 along y 30, size 12), so it
                // proves coverage and not only values.
                assert!(r.contains_rect(&DRect::new(10, 26, 70, 34)), "undo covers the stroke: {r:?}");
                assert_eq!(
                    image.pixels,
                    crate::channel_view::render(&st.doc, &st.channel_view, r, factor, crate::channel_view::Look::default(), st.active_layer).unwrap(),
                    "undo restores the shown mask"
                );
                delta.clear();
            }
        }
    }
}

#[test]
fn channel_views_show_live_targets_and_discarded_strokes() {
    for target in ["quickMask", "alpha", "red"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        match target {
            "quickMask" => {
                app.run("select.rect", json!({"x": 0, "y": 0, "width": 120, "height": 60})).unwrap();
                app.run("select.editInQuickMaskMode", json!({})).unwrap();
            }
            "alpha" => {
                app.run("channel.new", json!({"fill": "white"})).unwrap();
            }
            _ => {
                app.run("channel.target", json!({"channel": "red"})).unwrap();
            }
        }
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool = Tool::Brush;
        let ctx = egui::Context::default();
        let tex = crate::channel_view::ensure(&mut app, &ctx, 0, None).unwrap();
        ctx.tex_manager().write().take_delta().clear();
        let original = app.session.documents()[0].doc.clone();
        let steps = app.session.documents()[0].history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let shown = display_doc(&mut app, 0).0;
        crate::channel_view::ensure(&mut app, &ctx, 0, None);
        let mut delta = ctx.tex_manager().write().take_delta();
        let upload = &delta.set.get(&tex).expect("live channel upload")[0];
        let pos = upload.pos.expect("partial channel upload");
        let egui::ImageData::Color(image) = &upload.image;
        let r = DRect::new(pos[0] as i32, pos[1] as i32, (pos[0] + image.size[0]) as i32, (pos[1] + image.size[1]) as i32);
        assert_eq!(
            image.pixels,
            crate::channel_view::render(
                &shown,
                &app.session.documents()[0].channel_view,
                r,
                1,
                crate::channel_view::Look::default(),
                app.session.documents()[0].active_layer
            )
            .unwrap(),
            "{target}"
        );
        assert_ne!(
            image.pixels,
            crate::channel_view::render(
                &original,
                &app.session.documents()[0].channel_view,
                r,
                1,
                crate::channel_view::Look::default(),
                app.session.documents()[0].active_layer
            )
            .unwrap(),
            "target was painted"
        );
        delta.clear();
        assert_eq!(app.session.documents()[0].history.past_len(), steps);
        assert!(std::sync::Arc::ptr_eq(&original, &app.session.documents()[0].doc));
        // Dropping a preview without committing must restore its texture, even though the
        // document revision did not change. There is no committed damage rectangle to use.
        app.drag = None;
        app.live_stroke = None;
        crate::channel_view::ensure(&mut app, &ctx, 0, None);
        let mut delta = ctx.tex_manager().write().take_delta();
        let upload = &delta.set.get(&tex).unwrap()[0];
        assert!(upload.pos.is_none(), "discarding a preview refreshes the view");
        let egui::ImageData::Color(image) = &upload.image;
        assert_eq!(
            image.pixels,
            crate::channel_view::render(
                &original,
                &app.session.documents()[0].channel_view,
                original.bounds(),
                1,
                crate::channel_view::Look::default(),
                app.session.documents()[0].active_layer
            )
            .unwrap()
        );
        delta.clear();
    }
}

/// The held drag shows true mixed paint while Alt sampling remains separate from painting.
#[test]
fn mixer_drag_shows_actual_mixed_paint_before_release_and_alt_never_paints() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":16,"mixer":{"wet":1.0,"mix":0.7,"sampleAllLayers":true},"smoothing":{"amount":0.0}}})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.past_len();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Move { x: 35.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 35.0, y: 30.0 }, egui::Modifiers::NONE);
    assert!(app.drag.is_none());
    assert!(app.session.tools.mixer.spatial.is_some());
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    app.run("brush.mixer.load", json!({})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 30.0, pressure: 0.6 }, egui::Modifiers::NONE);
    assert!(app.live_stroke.is_some());
    let shown = display_doc(&mut app, 0).0;
    let id = app.session.active().unwrap().active_layer.unwrap();
    let pixel = shown.layer(id).unwrap().surface().unwrap().rgba(45, 30);
    assert!(pixel[0] > 0.0 && pixel[1] > 0.0 && pixel[3] > 0.0, "actual red/green mixture {pixel:?}");
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), count);
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 30.0 }, egui::Modifiers::NONE);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), count + 1);
    assert_eq!(
        shown.layer(id).unwrap().surface().unwrap().read_region(before.bounds()),
        st.doc.layer(id).unwrap().surface().unwrap().read_region(before.bounds())
    );
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(45, 30)[3], 0.0);
}

/// Release incorporates its final endpoint and deferred samples in both live and fallback paths.
#[test]
fn mixer_release_includes_a_new_endpoint_and_deferred_frame_samples() {
    for (deferred, failed) in [(false, false), (true, false), (false, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setBrush", json!({"size":12,"pressureSize":false})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        let before = app.session.active().unwrap().doc.clone();
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
        app.defer_live_stroke = deferred;
        if failed {
            app.live_stroke.as_mut().unwrap().preview_failed = true;
        }
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 95.0, y: 30.0 }, m);
        let doc = app.session.active().unwrap().doc.clone();
        let id = app.session.active().unwrap().active_layer.unwrap();
        assert!(doc.layer(id).unwrap().surface().unwrap().rgba(92, 30)[3] > 0.0);
        let params = app.session.journal.iter().rev().find(|(name, _)| name == "paint.mixerBrush").unwrap().1.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("paint.mixerBrush", params).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *doc);
    }
}

/// Reproduce a tool change and release in one control call, before a UI frame can cancel the old gesture.
#[test]
fn mixer_control_tool_switch_before_release_cancels_every_preview_path() {
    for preview in 0..3 {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setBrush", json!({"size":12,"pressureSize":false})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let anchor = app.last_stroke_end;
        let ctx = egui::Context::default();
        let (request, _reply) = crate::control::ControlRequest::new(
            "ui.pointer",
            json!({"tool":"mixerBrush","events":[{"kind":"down","x":20,"y":30},{"kind":"move","x":50,"y":30}]}),
        );
        let _ = crate::control::handle(&mut app, &ctx, &request);
        assert!(app.drag.is_some() && app.live_stroke.is_some());
        if preview == 1 {
            app.live_stroke.as_mut().unwrap().preview_failed = true;
        } else if preview == 2 {
            app.live_stroke = None;
        }
        // No intervening UI frame: control selects the new tool before dispatching Up.
        let (request, _reply) = crate::control::ControlRequest::new("ui.pointer", json!({"tool":"brush","events":[{"kind":"up","x":95,"y":30}]}));
        let _ = crate::control::handle(&mut app, &ctx, &request);
        assert_eq!(app.ui.tool, Tool::Brush);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(app.last_stroke_end, anchor);
        assert!(!app.session.journal.iter().any(|(id, _)| id == "paint.mixerBrush"));
    }
}

/// Escape and tool changes cancel both ordinary previews and failed-preview fallback gestures.
#[test]
fn mixer_escape_and_tool_switch_cancel_without_an_edit() {
    for mode in 0..4 {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(app.live_stroke.is_some());
        if mode >= 2 {
            app.live_stroke.as_mut().unwrap().preview_failed = true;
        }
        let mode = mode % 2;
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput { focused: true, ..Default::default() };
        if mode == 0 {
            raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        }
        if mode == 1 {
            app.ui.tool = Tool::Brush;
        }
        ctx.begin_pass(raw);
        assert!(cancel_mixer_on_context_change(&mut app, &ctx));
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, egui::Modifiers::NONE);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }
}

/// Control-driven strokes remain live in a background window, matching the ordinary Brush.
#[test]
fn mixer_background_window_keeps_and_commits_control_strokes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":60,"background":"transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":12,"pressureSize":false}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput { focused: false, ..Default::default() });
    let (request, _reply) =
        crate::control::ControlRequest::new("ui.pointer", json!({"tool":"mixerBrush","events":[{"kind":"down","x":20,"y":30},{"kind":"move","x":60,"y":30}]}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    assert!(!cancel_mixer_on_context_change(&mut app, &ctx));
    assert!(app.drag.is_some());
    let (request, _reply) = crate::control::ControlRequest::new("ui.pointer", json!({"tool":"mixerBrush","events":[{"kind":"up","x":95,"y":30}]}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    let mut output = ctx.end_pass();
    output.textures_delta.clear();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert!(app.session.journal.iter().any(|(id, _)| id == "paint.mixerBrush"));
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, 30)[3] > 0.0);
}

/// A tip above the live preview's 1024 px paints on release, as main and Photoshop paint it
/// up to 5000 px; a stroke that can't paint at all is still refused on press.
#[test]
fn mixer_drag_above_the_preview_size_paints_on_release() {
    for locked in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        if locked {
            app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
        }
        app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":2000,"hardness":1.0,"pressureSize":false,"mixer":{"wet":0.0,"mix":0.0}}})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        let steps = app.session.active().unwrap().history.past_len();
        let m = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
        assert!(app.live_stroke.is_none(), "no live preview above 1024 px");
        assert_eq!(app.drag.is_some(), !locked, "a valid 2000 px stroke is kept for release");
        assert_eq!(app.ui.status_error, locked, "{}", app.ui.status);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 30.0 }, m);
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + usize::from(!locked));
        let pixel = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, 30);
        assert_eq!(pixel[0] > 0.9 && pixel[3] > 0.9, !locked, "{pixel:?}");
    }
}

/// ⌥ with a held temporary tool (macOS ⌘ = Move) is that tool's, not a Mixer Brush paint load.
#[test]
fn mixer_alt_with_a_held_temporary_tool_does_not_load_paint() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":60,"background":"#00ff00"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    app.tool_override = Some(Tool::Move);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, alt);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, alt);
    assert!(app.session.tools.mixer.spatial.is_none(), "the held Move tool doesn't sample");
    app.tool_override = None;
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 }, alt);
    assert!(app.session.tools.mixer.spatial.is_some(), "⌥ with the Mixer Brush itself samples");
}

#[test]
fn brush_drag_shows_the_real_stroke_and_commits_it() {
    // The drag used to draw a hard, flat stand-in and only showed the soft brush on release.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    let m = egui::Modifiers::NONE;
    let alpha = |d: &Document, x, y| d.layers[0].surface().unwrap().rgba(x, y)[3];
    let rev = app.session.documents()[0].revision;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, m);
    for x in [40.0, 70.0, 100.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, m);
    }
    let (shown, key) = display_doc(&mut app, 0);
    assert_ne!(key, 0);
    // A partial value two thirds of the way out: the soft tip, not a hard flat stand-in.
    assert!(alpha(&shown, 40, 30) > 0.5 && (0.01..0.95).contains(&alpha(&shown, 40, 36)), "soft stroke while drawing");
    assert_eq!(alpha(&app.session.documents()[0].doc, 40, 30), 0.0, "not committed yet");
    // The canvas redraws only what the stroke touched.
    let dk = canvas_display(&app, &app.session.documents()[0].doc, None).1;
    let d = damage_since(&app, 0, (rev, dk), (rev, key), dk, (rev, None)).unwrap();
    assert!(d.contains(40, 30) && !d.contains(40, 2) && d.height() < 60, "{d:?}");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, m);
    let doc = app.session.documents()[0].doc.clone();
    assert!(app.live_stroke.is_none() && display_doc(&mut app, 0).1 == 0);
    assert_eq!((alpha(&doc, 40, 30), alpha(&doc, 40, 38)), (alpha(&shown, 40, 30), alpha(&shown, 40, 38)), "commit matches the preview");
}

/// #9329 PR C: Paste Into is three revisions; the canvas redraws what the whole command
/// changed (the pasted area), not the whole document as when only one revision counted.
#[test]
fn paste_into_refreshes_only_the_pasted_area() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 100, "height": 100})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("select.rect", json!({"x": 500, "y": 400, "width": 90, "height": 60})).unwrap();
    ensure_texture(&mut app, &ctx, 0, None).expect("a texture");
    let rev = app.session.active().unwrap().revision;
    app.perf.last_refresh_px = 0;
    app.run("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    assert!(app.session.active().unwrap().revision >= rev + 2, "several revisions");
    ensure_texture(&mut app, &ctx, 0, None).expect("a texture");
    let px = app.perf.last_refresh_px;
    assert!(px > 0 && px < 800 * 600 / 4, "refreshed {px} px of {}", 800 * 600);
}

/// The Move tool's Auto-Select pick is its own revision that changes no pixels: the drag
/// that follows still redraws only where the layer was and is.
#[test]
fn auto_select_move_is_still_bounded() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    for (x, color) in [(100, "#ff0000"), (500, "#0000ff")] {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": x, "y": 100, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let active = app.session.active().unwrap().active_layer;
    ensure_texture(&mut app, &ctx, 0, None).expect("a texture");
    let rev = app.session.active().unwrap().revision;
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 120.0, pressure: 1.0 }, m);
    assert_ne!(app.session.active().unwrap().active_layer, active, "Auto-Select picked the red layer");
    assert_eq!(app.session.active().unwrap().revision, rev + 1, "the pick is one revision");
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 130.0, pressure: 1.0 }, m);
    app.perf.last_refresh_px = 0;
    ensure_texture(&mut app, &ctx, 0, None).expect("a texture");
    let px = app.perf.last_refresh_px;
    assert!(px > 0 && px < 800 * 600 / 4, "refreshed {px} px of {}", 800 * 600);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 130.0 }, m);
}

#[test]
fn live_stroke_uses_pen_tilt_and_updates_a_reduced_texture_partially() {
    // Pen tilt drives the size; the preview must use the tilt the commit gets. The document
    // is wider than MAX_TEXTURE, so the CPU texture is reduced (factor 2) and each step
    // must update only the reduced pixels it touched.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.max_texture_side = 8192);
    app.run("file.new", json!({"width": 4097, "height": 90, "background": "transparent"})).unwrap();
    let tilt = json!({"size": 30, "hardness": 1.0, "spacing": 0.05, "shapeDynamics": {"enabled": true, "size": {"control": "penTilt"}}});
    app.run("tools.setBrush", json!({ "brush": tilt })).unwrap();
    app.ui.tool = Tool::Brush;
    assert_eq!(ensure_texture(&mut app, &ctx, 0, None).map(|t| t.1), Some(0.5));
    let m = egui::Modifiers::NONE;
    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: 1.0, tilt_x: 60.0, tilt_y: 0.0, rotation: 0.0, eraser: false }));
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 45.0, pressure: 1.0 }, m);
    for x in [300.0, 600.0, 900.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 45.0, pressure: 1.0 }, m);
        ensure_texture(&mut app, &ctx, 0, None);
        assert_eq!(app.perf.last_refresh, "rect");
        assert!(app.perf.last_refresh_px < 4097 * 90 / 4, "{}", app.perf.last_refresh_px);
    }
    let shown = display_doc(&mut app, 0).0;
    tool_event(&mut app, ToolEvent::Up { x: 900.0, y: 45.0 }, m);
    ensure_texture(&mut app, &ctx, 0, None);
    assert_eq!(app.perf.last_refresh, "rect", "the commit refreshes only the stroke");
    let doc = app.session.documents()[0].doc.clone();
    let (a, b) = (doc.layers[0].surface().unwrap(), shown.layers[0].surface().unwrap());
    // 60° tilt shrinks the dab well below 30 px: the preview shows that size, as committed.
    assert!(a.rgba(500, 45)[3] > 0.5 && a.rgba(500, 45 + 12)[3] == 0.0);
    // (The smoothed tail catches up to the end point only when the stroke finishes.)
    assert!((0..90).all(|y| (0..700).all(|x| a.rgba(x, y) == b.rgba(x, y))), "commit matches the preview");
}

/// Brush drag along y = 40 with one canvas frame per pointer move; returns the document the
/// canvas showed at the last move, the committed one, and whether the release refreshed only
/// the stroke's rectangle.
fn drag_frames(smoothing: f32, xs: &[f64]) -> (std::sync::Arc<Document>, std::sync::Arc<Document>, bool, PhotocraftApp) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5}})).unwrap();
    // What the options bar's Smoothing field writes.
    app.session.tools.brush.smoothing.amount = smoothing;
    // Catch-up on Stroke End starts off (#11887); these tests draw the catch-up tail.
    app.session.tools.brush.smoothing.catch_up_on_end = true;
    app.ui.tool = Tool::Brush;
    ensure_texture(&mut app, &ctx, 0, None);
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
    for &x in xs {
        tool_event(&mut app, ToolEvent::Move { x, y: 40.0 + (x / 7.0).sin() * 8.0, pressure: 1.0 }, m);
        ensure_texture(&mut app, &ctx, 0, None);
    }
    let live = display_doc(&mut app, 0).0;
    let last = *xs.last().unwrap();
    tool_event(&mut app, ToolEvent::Up { x: last, y: 40.0 + (last / 7.0).sin() * 8.0 }, m);
    let (next, key) = display_doc(&mut app, 0);
    assert_eq!(key, 0, "the frame after release shows the committed document");
    ensure_texture(&mut app, &ctx, 0, None);
    let partial = app.perf.last_refresh == "rect";
    (live, next, partial, app)
}

fn same_pixels(a: &Document, b: &Document) -> bool {
    let (a, b) = (a.layers[0].surface().unwrap(), b.layers[0].surface().unwrap());
    (0..80).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y)))
}

#[test]
fn release_shows_nothing_new_without_smoothing() {
    // #73: at 0 % the stroke follows the pointer; the last frame drawn while dragging is
    // exactly the committed stroke, so release changes nothing on screen.
    let (live, done, partial, app) = drag_frames(0.0, &[40.0, 70.0, 100.0, 130.0, 160.0]);
    assert!(same_pixels(&live, &done), "preview at the last move = committed stroke");
    assert!(partial, "the handover refreshes only the stroke");
    let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    assert!(p.get("smoothing").is_none(), "the commit uses the session brush's smoothing, not a hard-coded one");
}

#[test]
fn navigator_keeps_its_image_during_a_live_brush_stroke() {
    // #1631: every step of a live stroke is a new preview key, so a navigator keyed by it
    // re-composited the whole document per frame while painting. It waits for the release.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
    app.ui.tool = Tool::Brush;
    let m = egui::Modifiers::NONE;
    assert!(!navigator_waits_for_drag(&app, 0));
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
    let mut keys = Vec::new();
    for x in [40.0, 70.0, 100.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 40.0, pressure: 1.0 }, m);
        keys.push(display_doc(&mut app, 0).1);
        assert!(navigator_waits_for_drag(&app, 0), "painting at x = {x}");
    }
    assert!(keys.windows(2).all(|k| k[0] != k[1]), "each step is a new preview: {keys:?}");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    assert!(!navigator_waits_for_drag(&app, 0), "the navigator catches up on release");
}

#[test]
fn smoothed_stroke_shows_its_catch_up_tail_while_drawing() {
    // #73: with smoothing the brush lags behind the pointer and catches up at the end; the
    // preview draws that tail live, so no frame after release is missing the end.
    let xs = [30.0, 50.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0];
    let (live, done, partial, _) = drag_frames(0.5, &xs);
    assert!(same_pixels(&live, &done), "preview at the last move = committed stroke, tail included");
    assert!(partial);
    let end = |d: &Document| (0..200).rev().find(|&x| (0..80).any(|y| d.layers[0].surface().unwrap().rgba(x, y)[3] > 0.0)).unwrap();
    assert!(end(&live) >= 170, "the end reaches the pointer while drawing ({})", end(&live));
    // The options-bar value reaches the stroke: 50 % smooths the wiggle, 0 % doesn't.
    let (_, rough, _, _) = drag_frames(0.0, &xs);
    assert!(!same_pixels(&rough, &done), "smoothing changes the stroke");
    // And the commit is what `paint.stroke` gives with that smoothing.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 80, "background": "transparent"})).unwrap();
    s.execute("tools.setBrush", json!({"brush": {"size": 16, "hardness": 0.5, "smoothing": {"amount": 0.5, "catchUpOnEnd": true}}})).unwrap();
    let mut pts = vec![json!([10.0, 40.0, 1.0])];
    pts.extend(xs.iter().map(|&x| json!([x, 40.0 + (x / 7.0).sin() * 8.0, 1.0])));
    // (Round tips without dynamics draw the same for any seed.)
    s.execute("paint.stroke", json!({"points": pts, "seed": 0})).unwrap();
    assert!(same_pixels(&s.active().unwrap().doc, &done));
}

#[test]
fn smoothing_preview_redraws_the_tail_each_step() {
    // The tail drawn at one step must not linger once the brush moves on: a sharp turn would
    // leave a stale tail behind if it weren't restored.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 120, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0, "smoothing": {"amount": 0.6}}})).unwrap();
    app.ui.tool = Tool::Brush;
    let m = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, m);
    for (x, y) in [(60.0, 20.0), (110.0, 20.0), (110.0, 70.0), (110.0, 110.0), (60.0, 110.0)] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    let live = display_doc(&mut app, 0).0;
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 110.0 }, m);
    let done = app.session.documents()[0].doc.clone();
    let (a, b) = (live.layers[0].surface().unwrap(), done.layers[0].surface().unwrap());
    assert!((0..120).all(|y| (0..200).all(|x| a.rgba(x, y) == b.rgba(x, y))), "no stale tails in the preview");
}

/// Shapes the drag preview paints for `app` this frame.
fn drag_preview_shapes(app: &mut PhotocraftApp, ctx: &egui::Context, zoom: f32) -> Vec<egui::epaint::ClippedShape> {
    let out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let ctx = ui.ctx();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(rect);
        let xf = ViewXform { rect, zoom, center: [60.0, 30.0], flip: false, rotation: 0.0, aspect: 1.0 };
        draw_drag_preview(app, &painter, &xf);
    });
    let egui::FullOutput { mut textures_delta, shapes, .. } = out;
    textures_delta.clear();
    shapes
}

#[test]
fn a_huge_view_centre_draws_the_selection_outline_without_overflow() {
    // #980: `ui.set` takes any finite centre, and the outline's visible region was quantised
    // with unchecked i32 arithmetic: the frame panicked ("attempt to multiply/subtract with
    // overflow") and release builds wrapped to an inverted region.
    let ctx = egui::Context::default();
    // Zoomed far out the quantisation step, and so the padding, is largest (64 · 256 px).
    let cases = [([1e30, 1e30], false), ([-1e30, -1e30], false), ([3e9, -3e9], false), ([512.0, 512.0], true)];
    for ((centre, traced), zoom) in cases.into_iter().flat_map(|c| [(c, 1.0), (c, 1.0 / 64.0)]) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 1024, "height": 1024})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 512, "height": 512})).unwrap();
        app.sync_views();
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({ "center": centre, "zoom": zoom }));
        let _ = crate::control::handle(&mut app, &ctx, &req);
        let view = app.ui.views[0].clone();
        assert_eq!(view.zoom, zoom, "ui.set applied the zoom");
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            canvas_view(&mut app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), view.clone(), false);
        });
        out.textures_delta.clear();
        // Control: a centre on the selection's corner still traces the outline (the selection
        // spans several 64 px trace cells even zoomed out).
        let segs = app.outline_cache.as_ref().map_or(0, |(_, _, s)| s.len());
        assert_eq!(segs > 0, traced, "{centre:?} at zoom {zoom}");
    }
}

#[test]
fn a_one_pixel_selection_traces_at_hundred_percent_on_a_scaled_display() {
    // #1943 review: the outline step is display resolution in device pixels, not points, so a
    // 1 px selection still traces at 100 % on a 2x display (a points-based step sampled every
    // second pixel and lost it: zero segments).
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("select.rect", json!({"x": 50, "y": 50, "width": 1, "height": 100})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.sync_views();
    let view = app.ui.views[0].clone();
    let mut input = egui::RawInput::default();
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(2.0);
    let mut out = ctx.run_ui(input, |ui| {
        canvas_view(&mut app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), view.clone(), true);
    });
    out.textures_delta.clear();
    assert_eq!(app.ppp, 2.0, "the primary canvas records the display scale");
    let segs = app.outline_cache.as_ref().map_or(0, |(_, _, s)| s.len());
    assert!(segs > 0, "a 1 px selection traces at 100 % on a 2x display");
}

#[test]
fn a_secondary_canvas_does_not_overwrite_the_main_canvas_scale() {
    // #1943 review: a document window can sit on a display with another scale; only the
    // primary canvas records `app.ppp`, so the main mapping doesn't change under it.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.sync_views();
    let view = app.ui.views[0].clone();
    let frame = |app: &mut PhotocraftApp, ppp: f32, primary: bool| {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, 0, Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), view.clone(), primary);
        });
        out.textures_delta.clear();
    };
    frame(&mut app, 2.0, true);
    assert_eq!(app.ppp, 2.0);
    frame(&mut app, 1.0, false);
    assert_eq!(app.ppp, 2.0, "a secondary window's scale doesn't replace the main canvas's");
    // The main canvas mapping still uses its own scale: 100 % is 1 doc px per physical px.
    let xf = ViewXform::active(&app).unwrap();
    let step = xf.to_screen(1.0, 0.0) - xf.to_screen(0.0, 0.0);
    assert!((step.x - 0.5).abs() < 1e-3, "{step:?}");
}

#[test]
fn a_flicked_hand_drag_glides_on_only_with_flick_panning() {
    // #204: Preferences › Tools › Enable Flick Panning. A fast Hand drag let go while moving
    // keeps the view gliding for a while; off, the view stops where the drag ended.
    for flick in [true, false] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": flick})).unwrap();
        app.ui.tool = Tool::Hand;
        app.sync_views();
        let mut view = app.ui.views[0].clone();
        view.zoom = 1.0;
        view.center = [1000.0, 750.0];
        view.fit_pending = false;
        view.fill_pending = false;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> View {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                view = canvas_view(app, ui, 0, rect, view.clone(), true);
            });
            out.textures_delta.clear();
            view.clone()
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let mut end = start;
        for i in 1..=8 {
            end = start + vec2(-20.0 * i as f32, 0.0);
            frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        }
        let released = frame(&mut app, vec![press(end, false)]).center;
        let mut centers = vec![released];
        for _ in 0..5 {
            centers.push(frame(&mut app, vec![]).center);
        }
        if flick {
            // Dragged left: the image follows the pointer, the view keeps moving right.
            for w in centers.windows(2) {
                assert!(w[1][0] > w[0][0], "glides on: {centers:?}");
            }
            // ...and comes to rest.
            let mut last = *centers.last().unwrap();
            for _ in 0..120 {
                last = frame(&mut app, vec![]).center;
            }
            assert_eq!(frame(&mut app, vec![]).center, last, "the glide stops");
        } else {
            assert!(centers.iter().all(|c| *c == released), "no glide when off: {centers:?}");
        }
    }
}

#[test]
fn a_secondary_canvas_hand_pans_while_a_transform_is_open() {
    // #2302 review: `app.ui.transform` is global, but only the primary canvas draws and drives
    // the box. A secondary window's Hand must still pan while the main window transforms.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.ui.tool = Tool::Hand;
    app.sync_views();
    crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
    let mut view0 = app.ui.views[0].clone();
    view0.zoom = 1.0;
    view0.center = [200.0, 150.0];
    view0.fit_pending = false;
    view0.fill_pending = false;
    let center0 = view0.center;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> View {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut seen = view0.clone();
        let mut out = ctx.run_ui(input, |ui| {
            seen = canvas_view(app, ui, 0, rect, view0.clone(), false);
        });
        out.textures_delta.clear();
        seen
    };
    let start = Pos2::new(200.0, 150.0);
    let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![egui::Event::PointerMoved(start)]);
    frame(&mut app, vec![press(start, true)]);
    for dx in [10.0, 20.0, 40.0] {
        frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(dx, 0.0))]);
    }
    let dragged = frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(60.0, 0.0))]);
    frame(&mut app, vec![press(start + vec2(60.0, 0.0), false)]);
    assert_ne!(dragged.center, center0, "a secondary window's Hand pans while transforming");
}

#[test]
fn letting_go_of_alt_during_an_alt_sample_press_does_not_paint() {
    // #7802: Alt-pressing with the Brush samples a colour on the press. If Alt was let go
    // before egui recognised the drag, that drag started a stroke from the sampled point.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [200.0, 150.0];
    view.fit_pending = false;
    view.fill_pending = false;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
    };
    let alt = egui::Modifiers::ALT;
    let none = egui::Modifiers::NONE;
    let start = Pos2::new(200.0, 150.0);
    let press =
        |pos: Pos2, pressed: bool, modifiers: egui::Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // Alt released mid-press, and Alt held throughout: both only sample.
    for release_alt in [true, false] {
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 1.0, 1.0]})).unwrap();
        let rev = app.session.active().unwrap().revision;
        frame(&mut app, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true, alt)]);
        assert_eq!(app.session.tools.foreground, [1.0, 1.0, 1.0, 1.0], "the press sampled the white canvas");
        let later = if release_alt { none } else { alt };
        let end = start + vec2(20.0, 0.0);
        frame(&mut app, vec![egui::Event::ModifiersChanged(later), egui::Event::PointerMoved(end)]);
        frame(&mut app, vec![press(end, false, later)]);
        frame(&mut app, vec![egui::Event::ModifiersChanged(none)]);
        assert_eq!(app.session.active().unwrap().revision, rev, "release_alt={release_alt}: an Alt-sample press must not paint");
        assert!(app.drag.is_none() && !app.alt_sampling, "release_alt={release_alt}");
    }
    // Without Alt the same gesture paints.
    let rev = app.session.active().unwrap().revision;
    frame(&mut app, vec![egui::Event::PointerMoved(start)]);
    frame(&mut app, vec![press(start, true, none)]);
    frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(20.0, 0.0))]);
    frame(&mut app, vec![press(start + vec2(20.0, 0.0), false, none)]);
    assert!(app.session.active().unwrap().revision > rev, "a plain drag paints");
}

#[test]
fn letting_go_of_alt_during_a_mixer_brush_sample_press_does_not_paint() {
    // As #7802 for the Brush: an Alt-press with the Mixer Brush loads paint from the canvas.
    // If Alt was let go before egui recognised the drag, that drag started a Mixer stroke.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    app.sync_views();
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [200.0, 150.0];
    view.fit_pending = false;
    view.fill_pending = false;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
    };
    let (alt, none) = (egui::Modifiers::ALT, egui::Modifiers::NONE);
    let start = Pos2::new(200.0, 150.0);
    let press =
        |pos: Pos2, pressed: bool, modifiers: egui::Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    // Alt released mid-press, and Alt held throughout: both only load paint.
    for release_alt in [true, false] {
        let rev = app.session.active().unwrap().revision;
        frame(&mut app, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true, alt)]);
        let later = if release_alt { none } else { alt };
        let end = start + vec2(20.0, 0.0);
        frame(&mut app, vec![egui::Event::ModifiersChanged(later), egui::Event::PointerMoved(end)]);
        frame(&mut app, vec![press(end, false, later)]);
        frame(&mut app, vec![egui::Event::ModifiersChanged(none)]);
        assert_eq!(app.session.active().unwrap().revision, rev, "release_alt={release_alt}: an Alt-sample press must not paint");
        assert!(app.drag.is_none() && !app.mixer_sampling, "release_alt={release_alt}");
    }
    // Without Alt the same gesture paints.
    let rev = app.session.active().unwrap().revision;
    frame(&mut app, vec![egui::Event::PointerMoved(start)]);
    frame(&mut app, vec![press(start, true, none)]);
    frame(&mut app, vec![egui::Event::PointerMoved(start + vec2(20.0, 0.0))]);
    frame(&mut app, vec![press(start + vec2(20.0, 0.0), false, none)]);
    assert!(app.session.active().unwrap().revision > rev, "a plain drag paints");
}

#[test]
fn guides_draw_over_the_transform_preview_and_under_its_box() {
    // #5086: a guide crossing a layer under Free Transform was hidden behind the preview of
    // the moving pixels. The preview is image content, drawn before the grid and guides like
    // the document is; the box and handles still go over them, as in Photoshop.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 150})).unwrap();
    app.ui.extras.guides = true;
    app.ui.extras.rulers = false;
    app.sync_views();
    crate::transform_tool::begin(&mut app, &ctx).expect("a transform starts");
    let mut view = app.ui.views[0].clone();
    view.zoom = 1.0;
    view.center = [200.0, 150.0];
    view.fit_pending = false;
    view.fill_pending = false;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        canvas_view(&mut app, ui, 0, rect, view.clone(), true);
    });
    out.textures_delta.clear();
    // At 100 % centred on the document, screen and document coordinates coincide.
    let layer = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 160.0));
    let near = |r: Rect| (r.min - layer.min).length() < 1.0 && (r.max - layer.max).length() < 1.0;
    let position = |pred: &dyn Fn(&egui::Shape) -> bool| out.shapes.iter().position(|s| pred(&s.shape));
    // The moving pixels: the mesh that spans the layer's box.
    let preview = position(&|s| matches!(s, egui::Shape::Mesh(m) if near(m.calc_bounds())));
    // The guide: a vertical line at x = 150.
    let guide = position(&|s| matches!(s, egui::Shape::LineSegment { points, .. } if points.iter().all(|p| (p.x - 150.0).abs() < 0.01)));
    // The box: the closed four-point outline round the layer.
    let frame = position(&|s| matches!(s, egui::Shape::Path(p) if p.closed && p.points.len() == 4 && near(Rect::from_points(&p.points))));
    let (Some(preview), Some(guide), Some(frame)) = (preview, guide, frame) else {
        panic!("all painted: preview {preview:?}, guide {guide:?}, box {frame:?}");
    };
    assert!(preview < guide, "the guide ({guide}) draws over the transform preview ({preview})");
    assert!(guide < frame, "the box ({frame}) draws over the guide ({guide})");
}

#[test]
fn guides_draw_over_the_puppet_perspective_and_vanishing_point_previews() {
    // #7781: like Free Transform's (#5086), the warped pixels of Puppet Warp and Perspective
    // Warp and Vanishing Point's paste are image content. They were drawn with their pins and
    // planes, after the guides, hiding every guide across them; the guides now go over them,
    // the mesh lines, pins and planes over the guides, as in Photoshop.
    for command in ["edit.puppetWarp", "edit.perspectiveWarp", photocraft_engine::vp_cmds::VP] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        if command == photocraft_engine::vp_cmds::VP {
            app.run("edit.copy", json!({})).unwrap();
        }
        app.run("select.deselect", json!({})).unwrap();
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 150})).unwrap();
        app.ui.extras.guides = true;
        app.ui.extras.rulers = false;
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, command, &json!({})).expect("an on-canvas mode").expect("it starts");
        if command == photocraft_engine::vp_cmds::VP {
            // A plane over the document; the clipboard pastes onto it.
            for (x, y) in [(20.0, 280.0), (380.0, 280.0), (300.0, 40.0), (100.0, 40.0)] {
                crate::distort_ui::pointer(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
                crate::distort_ui::pointer(&mut app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
            }
        } else if command == "edit.perspectiveWarp" {
            // One plane, dragged out across the guide.
            crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 60.0, y: 70.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::distort_ui::pointer(&mut app, ToolEvent::Move { x: 240.0, y: 190.0, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 240.0, y: 190.0 }, egui::Modifiers::NONE);
            assert_eq!(app.distort.perspective.as_ref().map(|s| s.planes.len()), Some(1), "the drag makes a plane");
        }
        let mut view = app.ui.views[0].clone();
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.fit_pending = false;
        view.fill_pending = false;
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            canvas_view(&mut app, ui, 0, rect, view.clone(), true);
        });
        out.textures_delta.clear();
        // At 100 % centred on the document, screen and document coordinates coincide.
        let layer = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 160.0));
        // Puppet Warp's mesh reaches its Expansion (2 px) past the pixels.
        let near = |r: Rect| (r.min - layer.min).length() < 4.0 && (r.max - layer.max).length() < 4.0;
        let shapes: Vec<&egui::Shape> = out.shapes.iter().map(|s| &s.shape).collect();
        // The warped pixels (or the paste): a textured mesh over the layer (or on the plane,
        // well inside the document, unlike the checkerboard and document image).
        let preview = shapes.iter().position(|s| match s {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                let r = m.calc_bounds();
                if command == photocraft_engine::vp_cmds::VP { r.width() > 10.0 && r.width() < 300.0 && r.min.x < 150.0 && r.max.x > 150.0 } else { near(r) }
            }
            _ => false,
        });
        // The guide: a vertical line at x = 150 across the whole view.
        let is_guide = |s: &egui::Shape| matches!(s, egui::Shape::LineSegment { points, .. } if points.iter().all(|p| (p.x - 150.0).abs() < 0.01) && (points[1].y - points[0].y).abs() > 200.0);
        let guide = shapes.iter().position(|s| is_guide(s));
        // Over the guide: Puppet Warp's mesh lines, the Vanishing Point plane's outline, the
        // Perspective Warp plane's outline (exactly the dragged rectangle).
        let plane = Rect::from_min_max(pos2(60.0, 70.0), pos2(240.0, 190.0));
        let overlay = shapes.iter().position(|s| match s {
            egui::Shape::LineSegment { points, .. } => {
                command != "edit.perspectiveWarp" && !is_guide(s) && points.iter().all(|p| layer.expand(3.0).contains(*p))
            }
            egui::Shape::Path(p) => {
                p.closed
                    && p.points.len() == 4
                    && (command != "edit.perspectiveWarp"
                        || (Rect::from_points(&p.points).min - plane.min).length() + (Rect::from_points(&p.points).max - plane.max).length() < 0.5)
            }
            _ => false,
        });
        let (Some(preview), Some(guide)) = (preview, guide) else {
            panic!("{command}: all painted: preview {preview:?}, guide {guide:?}");
        };
        assert!(preview < guide, "{command}: the guide ({guide}) draws over the warped pixels ({preview})");
        let Some(overlay) = overlay else { panic!("{command}: its mesh or plane is drawn") };
        assert!(guide < overlay, "{command}: the mesh or plane ({overlay}) draws over the guide ({guide})");
    }
}

#[test]
fn brush_drags_paint_no_stand_in_shape_over_the_canvas() {
    // #189: v0.1 drew the stroke being dragged as a foreground-coloured egui polyline as wide
    // as the brush; zoomed in, egui tessellated it into hard black wedges fanning out from the
    // start. The canvas shows the real stroke (`LiveStroke`) and nothing is drawn over it.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 30, "hardness": 0.0}})).unwrap();
    // A smoothed stroke also draws its string line (see
    // `smoothed_brush_drag_draws_the_string_from_brush_to_pointer`): that is no stand-in shape.
    let overlay = |app: &mut PhotocraftApp, ctx: &egui::Context| {
        drag_preview_shapes(app, ctx, 12.0).iter().filter(|s| !matches!(s.shape, egui::Shape::LineSegment { .. } | egui::Shape::Circle(_))).count()
    };
    for tool in [Tool::Brush, Tool::Eraser] {
        app.ui.tool = tool;
        let idle = overlay(&mut app, &ctx);
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        for i in 1..40 {
            let t = f64::from(i);
            tool_event(&mut app, ToolEvent::Move { x: 20.0 + t, y: 30.0 + (t / 3.0).sin() * 4.0, pressure: 1.0 }, egui::Modifiers::NONE);
        }
        assert!(app.live_stroke.is_some(), "{tool:?}: the canvas shows the live stroke");
        assert_eq!(overlay(&mut app, &ctx), idle, "{tool:?}: no overlay while dragging");
        tool_event(&mut app, ToolEvent::Up { x: 59.0, y: 30.0 }, egui::Modifiers::NONE);
    }
}

#[test]
fn smoothed_brush_drag_draws_the_string_from_brush_to_pointer() {
    // #3479: with smoothing the brush lags behind the pointer; a line joins them while drawing.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
    app.ui.tool = Tool::Brush;
    let lines = |app: &mut PhotocraftApp| drag_preview_shapes(app, &ctx, 4.0).iter().filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. })).count();
    for (amount, string) in [(0.0, false), (0.8, true)] {
        app.run("tools.setBrush", json!({"brush": {"size": 10, "smoothing": {"amount": amount}}})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        for x in [30.0, 60.0, 90.0] {
            tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        }
        assert_eq!(lines(&mut app) > 0, string, "smoothing {amount}");
        tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 30.0 }, egui::Modifiers::NONE);
    }
}

#[test]
fn retouch_drags_show_the_footprint_without_wedges() {
    // The retouching tools' trail was the same brush-wide polyline (#189): now a mask of the
    // footprint, with nothing outside it, however small the steps. The Remove Tool's is where
    // the stroke's dabs reach at all, which is what it removes, so a soft tip's whole reach
    // past the radius is tinted.
    for hardness in [1.0, 0.0] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
        app.ui.tool = Tool::Remove;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 30, "hardness": hardness}})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        for i in 1..80 {
            let t = f64::from(i) * 0.5;
            tool_event(&mut app, ToolEvent::Move { x: 20.0 + t, y: 30.0 + (t / 2.0).sin() * 3.0, pressure: 1.0 }, egui::Modifiers::NONE);
            if i % 20 == 0 {
                // One mesh for the trail, however long the drag.
                let shapes = drag_preview_shapes(&mut app, &ctx, 12.0);
                assert!(!shapes.iter().any(|s| matches!(s.shape, egui::Shape::Path(_))), "no polyline");
            }
        }
        drag_preview_shapes(&mut app, &ctx, 12.0);
        // The stroke the removal gets on release, and the pixels its hole takes.
        let points = app.drag.as_ref().unwrap().points.iter().map(|p| photocraft_paint::StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
        let brush = photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({}));
        let (b, cov) = photocraft_paint::retouch::stroke_coverage(&photocraft_paint::Stroke { brush, points });
        let trail = app.trail.as_ref().unwrap();
        assert_eq!(trail.scale(), 1.0);
        let mut beyond = 0;
        for y in 0..60 {
            for x in 0..120 {
                let removed = b.contains(x, y) && cov[(y - b.y0) as usize * b.width() as usize + (x - b.x0) as usize] > 0.0;
                assert_eq!(trail.coverage(x as usize, y as usize), if removed { 255 } else { 0 }, "hardness {hardness}: ({x}, {y})");
                // Past the 15 px radius of the path's ends and sides (y 27..33).
                beyond += usize::from(removed && !(12..=48).contains(&y));
            }
        }
        if hardness == 0.0 {
            assert!(beyond > 100, "a soft tip's removal reaches past its radius: {beyond} px");
        }
        tool_event(&mut app, ToolEvent::Up { x: 59.5, y: 30.0 }, egui::Modifiers::NONE);
        drag_preview_shapes(&mut app, &ctx, 12.0);
        assert!(app.trail.is_none(), "the trail ends with the drag");
    }
}

#[test]
fn spot_healing_trail_is_the_coverage_the_heal_uses() {
    // #2364: a soft brush with pen-pressure size showed as a hard footprint at full size.
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 60})).unwrap();
    let brush = json!({"size": 30, "hardness": 0.0, "shapeDynamics": {"enabled": true, "size": {"control": "penPressure"}}});
    app.run("tools.setBrush", json!({ "brush": brush })).unwrap();
    app.ui.tool = Tool::SpotHealing;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 0.5 }, egui::Modifiers::NONE);
    for x in [40.0, 60.0, 80.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 30.0, pressure: 0.5 }, egui::Modifiers::NONE);
        drag_preview_shapes(&mut app, &ctx, 12.0);
    }
    // The stroke the heal gets on release, and its coverage.
    let points = app.drag.as_ref().unwrap().points.iter().map(|p| photocraft_paint::StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
    let brush = photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &json!({}));
    let (b, cov) = photocraft_paint::retouch::stroke_coverage(&photocraft_paint::Stroke { brush, points });
    let trail = app.trail.as_ref().unwrap();
    let mut soft = 0;
    for y in 0..60 {
        for x in 0..120 {
            let want = if b.contains(x, y) { cov[(y - b.y0) as usize * b.width() as usize + (x - b.x0) as usize] } else { 0.0 };
            let got = trail.coverage(x as usize, y as usize);
            assert!((f32::from(got) - want * 255.0).abs() <= 1.0, "({x}, {y}): trail {got}, heal {want}");
            soft += usize::from(got > 0 && got < 200);
        }
    }
    assert!(soft > 50, "a soft brush shows a soft edge");
    // Half pressure halves the 30 px tip: nothing past its (soft) reach.
    let reach = photocraft_paint::tip_reach(7.5, app.session.tools.brush.hardness).ceil() as usize + 2;
    assert_eq!(trail.coverage(50, 30 + reach), 0, "half pressure halves the 30 px tip");
}

#[test]
fn layer_style_dialog_previews_live_and_cancel_restores() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let fx = |d: &Document| d.layers.iter().map(|l| l.effects.items.len()).sum::<usize>();
    let id = crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap();
    let (shown, key) = display_doc(&mut app, 0);
    assert_eq!((fx(&shown), fx(&app.session.documents()[0].doc)), (1, 0), "previewed, not committed");
    // Adding a stroke through the dialog's instance list re-renders the canvas.
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields["effects"].as_array_mut().unwrap().push(json!({"id": "fx9", "kind": "stroke", "on": true, "params": {"size": 3}}));
    }
    let (shown, key2) = display_doc(&mut app, 0);
    assert_eq!(fx(&shown), 2);
    assert_ne!(key, key2, "an edit re-renders the canvas");
    app.ui.close_dialog(id);
    let (shown, key) = display_doc(&mut app, 0);
    assert_eq!((fx(&shown), key), (0, 0));
    assert!(app.style_preview.is_none());
}

/// Layer Style opened while Free Transform is open applies the box first, as Photoshop does,
/// so the dialog previews on the transformed layer (#4821): a dropped file (placed in the box,
/// untouched) and a box opened by hand and moved.
#[test]
fn layer_style_opened_during_free_transform_applies_the_box_and_previews() {
    for placed in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.sync_views();
        let ctx = egui::Context::default();
        let dx = if placed {
            crate::transform_tool::begin_placed(&mut app, &ctx).unwrap();
            0
        } else {
            crate::transform_tool::begin(&mut app, &ctx).unwrap();
            app.ui.transform.as_mut().unwrap().quad = [[28.0, 8.0], [44.0, 8.0], [44.0, 24.0], [28.0, 24.0]];
            20
        };
        crate::layer_style::open(&mut app, Some("dropShadow")).unwrap();
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "placed {placed}: the box is applied");
        let (shown, _) = display_doc(&mut app, 0);
        let rect = photocraft_geom::Rect::new(0, 0, 64, 64);
        let with = photocraft_compose::render(&shown, rect);
        let without = photocraft_compose::render(&app.session.documents()[0].doc, rect);
        assert!(
            with.get(16 + dx, 16)[0] < 0.1 && with.get(16 + dx, 16)[2] > 0.9,
            "placed {placed}: the blue block where the box left it, not the white under it"
        );
        // The shadow darkens the white around the block, which the document doesn't have yet.
        let block = |x: i32, y: i32| (8 + dx..24 + dx).contains(&x) && (8..24).contains(&y);
        let shadow =
            (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| !block(x, y) && without.get(x, y)[0] - with.get(x, y)[0] > 0.05).count();
        assert!(shadow > 20, "placed {placed}: the Drop Shadow is previewed ({shadow} px)");
    }
}

#[test]
fn fill_view_covers_the_canvas_and_centres_the_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let mut view = View { fill_pending: true, ..View::default() };

    fill_view(&mut view, &doc, vec2(600.0, 600.0), 1.0, 1.0);

    assert_eq!(view.zoom, 3.0, "the short edge fills the available height");
    assert_eq!(view.center, [200.0, 100.0]);
    assert!(!view.fill_pending);

    // A scaled display stores the physical factor: the same point zoom is 2× the device px.
    let mut view = View { fill_pending: true, ..View::default() };
    fill_view(&mut view, &doc, vec2(600.0, 600.0), 2.0, 1.0);
    assert_eq!(view.zoom, 6.0);

    // Fit never magnifies past 100% = one image pixel per physical pixel, at any display
    // scale (it used to cap at one per *point*, so a 150% display opened images at 150%).
    for ppp in [1.0f32, 1.5, 2.0] {
        let mut view = View { fit_pending: true, ..View::default() };
        fit_view(&mut view, &doc, vec2(600.0, 600.0), ppp, 1.0);
        assert_eq!(view.zoom, 1.0, "ppp {ppp}: a small image opens at 100%");
    }
    // A large image still shrinks to fit: 1000×500 into 440 points is 0.44 pt/px, so 0.66
    // physical at ppp 1.5.
    let big = {
        app.run("file.new", json!({"width": 1000, "height": 500})).unwrap();
        app.session.active().unwrap().doc.clone()
    };
    let mut view = View { fit_pending: true, ..View::default() };
    fit_view(&mut view, &big, vec2(480.0, 480.0), 1.5, 1.0);
    assert!((view.zoom - 0.66).abs() < 1e-5, "{}", view.zoom);
}

#[test]
fn a_hundred_percent_is_one_physical_pixel_per_document_pixel() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    for ppp in [1.0f32, 1.25, 2.0] {
        app.ppp = ppp;
        let xf = ViewXform::active(&app).unwrap();
        // 100% zoom: one document pixel is one physical pixel, i.e. 1/ppp egui points.
        let step = xf.to_screen(1.0, 0.0) - xf.to_screen(0.0, 0.0);
        // `to_screen` differences around the view centre cancel in f32; 1e-3 is still exact
        // enough to see a wrong ppp factor (which would be a 20–100% error).
        assert!((step.x - 1.0 / ppp).abs() < 1e-3, "ppp {ppp}: {step:?} want {:?}", 1.0 / ppp);
        assert!((app.point_zoom() - 1.0 / ppp).abs() < 1e-5, "ppp {ppp}: {}", app.point_zoom());
        // Screen ↔ document stays an exact round trip.
        let back = xf.to_doc(xf.to_screen(123.0, 45.0));
        assert!((back[0] - 123.0).abs() < 1e-3 && (back[1] - 45.0).abs() < 1e-3, "ppp {ppp}: {back:?}");
    }
}

#[test]
fn wheel_anchor_is_the_pointer_over_the_image_and_the_nearest_image_point_off_it() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    for (rotation, flip) in [(0.0, false), (30.0, false), (90.0, true)] {
        let xf = ViewXform { rect, zoom: 0.5, center: [400.0, 300.0], flip, rotation, aspect: 1.0 };
        let inside = xf.to_screen(100.0, 200.0);
        assert_eq!(wheel_anchor(&xf, inside, [800, 600]), inside);
        for (p, want) in [((-300.0, -200.0), [0.0, 0.0]), ((1200.0, 300.0), [800.0, 300.0]), ((400.0, 900.0), [400.0, 600.0])] {
            let a = xf.to_doc(wheel_anchor(&xf, xf.to_screen(p.0, p.1), [800, 600]));
            assert!((a[0] - want[0]).abs() < 0.01 && (a[1] - want[1]).abs() < 0.01, "rot {rotation}: {p:?} -> {a:?}");
        }
    }
    let p = pos2(5.0, 5.0);
    let xf = ViewXform { rect, zoom: 1.0, center: [400.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
    assert_eq!(wheel_anchor(&xf, p, [0, 0]), p, "no document: the pointer");
}

#[test]
fn zoom_about_centres_the_clicked_point_with_the_preference() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let p = pos2(600.0, 200.0);
    for ppp in [1.0f32, 2.0] {
        // `View::zoom` is physical; the transform works in points.
        let xf = ViewXform { rect, zoom: 1.0 / ppp, center: [400.0, 300.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let doc = xf.to_doc(p);
        // Off: the document point under the pointer stays under it.
        let mut view = View { zoom: 1.0, center: [400.0, 300.0], ..Default::default() };
        zoom_about(&mut view, &xf, p, 2.0, false, ppp);
        let after = ViewXform { rect, zoom: 2.0 / ppp, center: view.center, flip: false, rotation: 0.0, aspect: 1.0 };
        let s = after.to_screen(doc[0] as f32, doc[1] as f32);
        assert!((s - p).length() < 0.5, "ppp {ppp}: {s:?} vs {p:?}");
        // On: the clicked point is the view centre.
        let mut view = View { zoom: 1.0, center: [400.0, 300.0], ..Default::default() };
        zoom_about(&mut view, &xf, p, 2.0, true, ppp);
        assert!((view.center[0] - doc[0] as f32).abs() < 0.5, "ppp {ppp}: {:?}", view.center);
        assert!((view.center[1] - doc[1] as f32).abs() < 0.5, "ppp {ppp}: {:?}", view.center);
    }
}

#[test]
fn damage_grows_by_nested_effect_reach() {
    use photocraft_doc::{Effect, Layer};
    let fmt = photocraft_color::PixelFormat::RGBA8;
    assert_eq!(effect_reach(&[Layer::raster("plain", fmt)]), 0);
    let mut inner = Layer::raster("inner", fmt);
    inner.effects.items.push(Effect::default_drop_shadow());
    let m = photocraft_compose::effects::margin(&inner);
    let mut group = Layer::group("g", vec![inner.clone()]);
    group.effects.items.push(Effect::default_drop_shadow());
    assert_eq!(effect_reach(std::slice::from_ref(&inner)), m);
    // A child's edit moves the group's shape, whose effects reach further.
    assert_eq!(effect_reach(&[group.clone()]), 2 * m);
    group.visible = false;
    assert_eq!(effect_reach(&[group]), 0);
}

#[test]
fn tab_reorder_keeps_floating_documents_in_place() {
    let visible = [0, 2];
    assert_eq!(move_target_after_filter(&visible, 0, 1, 3), 2);
    assert_eq!(move_target_after_filter(&visible, 2, 0, 3), 0);
}

#[test]
fn arrange_layout_only_includes_docked_documents() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for name in ["first", "floating", "last"] {
        app.run("file.new", json!({"width": 80, "height": 60, "name": name})).unwrap();
    }
    app.ui.view.arrange = "threeUpStacked".into();
    crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.floatInWindow", json!({"document": 1})).unwrap();
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let shown: Vec<_> = arranged_cells(&app, rect).unwrap().into_iter().map(|(doc, _)| doc).collect();
    assert_eq!(shown, vec![0, 2]);

    crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.floatInWindow", json!({"document": 2})).unwrap();
    assert!(arranged_cells(&app, rect).is_none(), "the main canvas should be blank when every document is floating");
}

#[test]
fn torn_off_document_is_not_drawn_again_in_the_main_window() {
    for kind in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60, "name": "floating"})).unwrap();
        app.run("file.new", json!({"width": 80, "height": 60, "name": "docked"})).unwrap();
        app.sync_views();
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
        assert!(app.ui.windows[0].floating);
        assert_eq!(app.session.active_index(), Some(0), "{kind:?}: the floated page remains active");
        assert_eq!(main_canvas_document(&app), Some(1), "{kind:?}: the main canvas picks a docked page instead");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(680.0, 480.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.state_mut().1 = true;
        h.run_steps(3);
        let tabs = h.state().0.tab_strip.as_ref().map(|s| s.tabs.iter().map(|(doc, _)| *doc).collect::<Vec<_>>());
        assert_eq!(tabs, Some(vec![1]), "{kind:?}: only the docked document stays on the main tab strip");
    }
}

/// #8230: Edit › Fade opens with a Preview checkbox, and the canvas shows the layer faded with the
/// dialog's opacity, at full size, on the background worker and the inline path alike. Cancel
/// leaves the document, its history and Fade itself as they were; OK applies what was shown.
#[test]
fn fade_dialog_previews_the_faded_layer_and_cancel_keeps_the_document() {
    for background in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        app.background_jobs = background;
        let ctx = egui::Context::default();
        let st = app.session.active().unwrap();
        let (layer, steps, revision) = (st.active_layer.unwrap(), st.history.past_len(), st.revision);
        let id = crate::filter_dialog::open(&mut app, "edit.fade").expect("Fade has a dialog");
        let d = app.ui.dialog_mut(id).expect("the dialog");
        assert_eq!(d.fields.get("__preview"), Some(&json!(true)), "a Preview checkbox, on");
        d.fields.insert("opacity".into(), json!(25));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let (k, size) = loop {
            if let Some((k, _, size)) = ensure_filter_preview(&mut app, 0, &ctx)
                && app.filter_preview.as_ref().is_some_and(|p| p.key.params.get("opacity") == Some(&json!(25)))
            {
                break (k, size);
            }
            assert!(std::time::Instant::now() < deadline, "background jobs {background}: no preview");
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!((k, size), (1, [40, 30]), "background jobs {background}: full size");
        let shown = app.filter_preview.as_ref().and_then(|p| p.result.as_ref()).expect("a result").layer(layer).unwrap().surface().unwrap().pixel(3, 3);
        assert!((shown[0] - 0.75).abs() < 0.01, "background jobs {background}: {shown:?}");

        // Cancel: nothing changed, nothing previewed, and Fade is still there to use.
        app.ui.close_dialog(id);
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "background jobs {background}: no preview after Cancel");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer(layer).unwrap().surface().unwrap().pixel(3, 3)[0], 0.0, "background jobs {background}: the layer is untouched");
        assert_eq!((st.history.past_len(), st.revision), (steps, revision), "background jobs {background}: no history step");
        assert!(app.session.is_enabled("edit.fade"), "background jobs {background}: Fade is still available");

        // OK applies the faded layer as one step.
        let id = crate::filter_dialog::open(&mut app, "edit.fade").unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(25));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let st = app.session.active().unwrap();
        let faded = st.doc.layer(layer).unwrap().surface().unwrap().pixel(3, 3)[0];
        assert!((faded - shown[0]).abs() < 1e-6, "background jobs {background}: OK applies the preview ({faded})");
        assert_eq!(st.history.past_len(), steps + 1);
    }
}

/// #11276: Float All in Windows leaves a document that already floats in its one window; a
/// second floating window would share the document's view and overwrite its centre.
#[test]
fn float_all_keeps_one_window_per_floating_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "one"})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300, "name": "two"})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    let first = crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({"document": 0})).unwrap()["window"].as_u64();
    for _ in 0..2 {
        let all = crate::menus::invoke(&mut app, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
        assert_eq!(all["windows"][0].as_u64(), first, "the floating document keeps its window");
        for doc in 0..2 {
            let n = app.ui.windows.iter().filter(|w| w.open && w.floating && w.document == doc).count();
            assert_eq!(n, 1, "document {doc}: one floating window");
        }
    }
}

/// #11276: the zoom a floating document took (its window draws the document's own view) stays
/// when the windows are consolidated back into tabs.
#[test]
fn consolidating_keeps_the_zoom_set_while_floating() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "floating"})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
    crate::zoom_levels::set_from_field(&mut app.ui.views[0], false, 200.0, false);
    crate::menus::invoke(&mut app, &ctx, "window.arrange.consolidateAllToTabs", json!({})).unwrap();
    assert!(app.ui.windows.is_empty());
    assert!((app.ui.views[0].zoom - 2.0).abs() < 1e-3, "the tab shows the floating zoom: {}", app.ui.views[0].zoom);
}

/// #11276: a New Window for Document (not floating) keeps a view of its own beside the main
/// canvas: zooming either leaves the other as it was.
#[test]
fn a_new_window_for_document_keeps_its_own_view() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "doc"})).unwrap();
    app.sync_views();
    crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.newWindowForDocument", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                extra_windows(app, ui.ctx());
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().1 = true;
    h.run_steps(4);
    assert!(h.state().0.ui.windows.iter().any(|w| w.open && !w.floating), "the extra window is open");
    let window_zoom = |h: &egui_kittest::Harness<'_, (PhotocraftApp, bool)>| h.state().0.ui.windows[0].view.zoom;
    let before = window_zoom(&h);
    crate::zoom_levels::set_from_field(&mut h.state_mut().0.ui.views[0], false, 300.0, false);
    h.run_steps(3);
    assert!((h.state().0.ui.views[0].zoom - 3.0).abs() < 1e-3, "the main canvas keeps its zoom");
    assert!((window_zoom(&h) - before).abs() < 1e-3, "the extra window keeps its own: {}", window_zoom(&h));
    let main = h.state().0.ui.views[0].zoom;
    crate::zoom_levels::set_from_field(&mut h.state_mut().0.ui.windows[0].view, false, 50.0, false);
    h.run_steps(3);
    assert!((window_zoom(&h) - 0.5).abs() < 1e-3, "the extra window keeps its zoom");
    assert!((h.state().0.ui.views[0].zoom - main).abs() < 1e-3, "the main canvas is not written over");
}

/// #11276: in a tiled arrangement the active document's cell zooms from the Navigator and the
/// status bar too.
#[test]
fn a_tiled_document_follows_navigator_and_status_bar_zoom() {
    for layout in ["twoUpVertical", "tileAllVertically", "fourUp"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for name in ["one", "two", "three"] {
            app.run("file.new", json!({"width": 400, "height": 300, "name": name})).unwrap();
        }
        app.sync_views();
        crate::menus::invoke(&mut app, &egui::Context::default(), &format!("window.arrange.{layout}"), json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| document_area(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().1 = true;
        h.run_steps(4);
        let active = h.state().0.session.active_index().unwrap();
        for pct in [200.0, 50.0] {
            crate::zoom_levels::set_from_field(&mut h.state_mut().0.ui.views[active], false, pct, false);
            h.run_steps(3);
            let zoom = h.state().0.ui.views[active].zoom;
            assert!((zoom - pct / 100.0).abs() < 1e-3, "{layout} {pct}%: {zoom}");
        }
    }
}

/// #11276: a document floating in its own window zooms from the zoom typed into the status bar
/// and into the Navigator, as a tabbed one does; the window doesn't keep or write back an older
/// copy of the view. Only the typed zoom fields are driven here, not the Navigator's slider or
/// zoom buttons.
#[test]
fn a_floating_document_follows_navigator_and_status_bar_zoom() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300, "name": "floating"})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300, "name": "docked"})).unwrap();
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
        app
    });
    h.run_steps(6);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.navigator", json!({})).unwrap();
    h.run_steps(6);
    assert_eq!(h.state().session.active_index(), Some(0), "the floating document is active");
    // The status bar's zoom field (bottom left) and the Navigator's.
    let fields = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        let mut f: Vec<egui::Rect> = h
            .query_all_by_role(egui::accesskit::Role::SpinButton)
            .filter(|n| n.accesskit_node().label().as_deref() == Some("Zoom"))
            .map(|n| n.rect())
            .collect();
        f.sort_by(|a, b| a.left().total_cmp(&b.left()));
        f
    };
    assert_eq!(fields(&h).len(), 2, "the status bar's and the Navigator's zoom fields");
    for (field, typed, want) in [(0, "200", 2.0), (1, "50", 0.5), (0, "300", 3.0), (1, "100", 1.0)] {
        let at = fields(&h)[field].center();
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(typed.into()));
        h.key_press(egui::Key::Enter);
        h.run_steps(60);
        let window = h.state().ui.windows[0].view.zoom;
        assert!((window - want).abs() < 1e-3, "field {field}, typed {typed}: the window shows {window}");
        assert!((h.state().ui.views[0].zoom - want).abs() < 1e-3, "field {field}, typed {typed}: not written back over by the window");
    }
}

#[test]
fn free_transform_redraws_only_the_moving_layer_when_the_box_opens_and_closes() {
    // Opening the box swapped the canvas to a preview document with no damage for it, so a
    // gigapixel canvas recomposited in full (over a second) on every ⌘T, and again on Cancel.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200, "background": "transparent"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(20, 20, 60, 50), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let rev = app.session.documents()[0].revision;
    crate::transform_tool::begin(&mut app, &egui::Context::default()).expect("the box opens");
    let (_, key) = display_doc(&mut app, 0);
    assert_eq!(key, crate::transform_tool::preview_key(app.transform_preview.as_ref().unwrap().session));
    let dk = canvas_display(&app, &app.session.documents()[0].doc, None).1;
    let opened = damage_since(&app, 0, (rev, dk), (rev, key), dk, (rev, None)).expect("opening the box has a damage area");
    assert!(opened.contains(20, 20) && opened.contains(59, 49), "{opened:?}");
    assert!(opened.width() < 400 && opened.height() < 200, "not the whole canvas: {opened:?}");
    let canceled = damage_since(&app, 0, (rev, key ^ dk), (rev, 0), dk, (rev, None)).expect("canceling has one too");
    assert_eq!(canceled, opened);
    // A commit (one revision later) adds what it changed; anything else redraws everything.
    let moved = photocraft_geom::Rect::new(100, 20, 140, 50);
    let committed = damage_since(&app, 0, (rev, key ^ dk), (rev + 1, 0), dk, (rev, Some(moved))).expect("committing too");
    assert!(committed.contains(20, 20) && committed.contains(139, 49), "{committed:?}");
    assert!(damage_since(&app, 0, (rev, key ^ dk), (rev + 2, 0), dk, (rev + 1, Some(moved))).is_none(), "a later revision isn't ours");
}

/// tanish2k09 on #10406: an armed eyedropper owns the image of the primary canvas only, the one
/// its picks come from and its pipette is drawn on. A secondary canvas (a Window › Arrange › New
/// Window view) is never left without a cursor for it.
#[test]
fn an_armed_eyedropper_hides_no_cursor_on_a_secondary_canvas() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Move;
    let id = crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
    let base = app.ui.dialog_mut(id).map(|d| crate::filter_dialog::params_of(&d.fields)).unwrap();
    let dialog = app.ui.dialog_mut(id).unwrap();
    dialog.fields.insert("__curvePicker".into(), json!("black"));
    dialog.fields.insert("__curvePickerBase".into(), base);
    assert!(crate::adjust_dialog::picker_armed(&app), "the black-point eyedropper is armed");
    let view = app.ui.views[0].clone();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let cursor = |app: &mut PhotocraftApp, primary: bool| {
        let mut icon = egui::CursorIcon::Default;
        for _ in 0..3 {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::PointerMoved(Pos2::new(200.0, 150.0)));
            let mut out = ctx.run_ui(input, |ui| {
                canvas_view(app, ui, 0, rect, view.clone(), primary);
            });
            out.textures_delta.clear();
            icon = out.platform_output.cursor_icon;
        }
        icon
    };
    assert_eq!(cursor(&mut app, true), egui::CursorIcon::None, "the primary canvas hides the icon under the pipette it draws");
    assert_ne!(cursor(&mut app, false), egui::CursorIcon::None, "a secondary canvas keeps a cursor");
}
