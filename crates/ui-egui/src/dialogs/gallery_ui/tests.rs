use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 90, "height": 60, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..60 {
                for x in 0..90 {
                    let v = ((x / 6 + y / 6) % 2) as f32;
                    s.write_pixel(x, y, &[v, 0.4, 1.0 - v, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

/// ⌘+ / ⌘− step the preview along the canvas's zoom ladder as Liquify's do, from the zoom Fit
/// in View shows; ⌘0 fits it and ⌘1 is 100 %, through the canvas's own shortcuts; the canvas
/// behind keeps its zoom.
#[test]
fn zoom_shortcuts_zoom_the_preview_not_the_canvas() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    // Large enough that Fit in View is well under 100 % in a 1000 × 700 window.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 3000, "height": 2000})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    let canvas_zoom = app.ui.views.first().map(|v| v.zoom);
    let mut run = |key: Option<egui::Key>| {
        let events = key
            .map(|key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND })
            .into_iter()
            .collect();
        let screen = ERect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0));
        let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(&mut app, ui.ctx());
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();
        let d = app.distort.gallery.as_ref().unwrap();
        (d.zoom, d.fit)
    };
    let (zoom, fit) = run(None);
    assert_eq!(zoom, 0.0, "opens at Fit in View");
    assert!(fit > 0.0 && fit < 0.5, "fits a 3000 px image well under 100 %: {fit}");
    let step = |z, dir| crate::zoom_levels::step_in_range(z, dir, ZOOM_MIN, ZOOM_MAX);
    let (zoom, _) = run(Some(egui::Key::Equals));
    assert_eq!(zoom, step(fit, 1), "⌘= steps up from the fitted zoom, not from the menu's first entry");
    assert!(zoom > fit);
    assert_eq!(run(Some(egui::Key::Plus)).0, step(step(fit, 1), 1), "⌘+ typed as Plus too");
    assert_eq!(run(Some(egui::Key::Num0)).0, 0.0, "Fit in View");
    assert!(run(Some(egui::Key::Minus)).0 < fit, "⌘− steps down from the fitted zoom");
    assert_eq!(run(Some(egui::Key::Num1)).0, 1.0);
    assert_eq!(run(Some(egui::Key::Equals)).0, 2.0);
    assert_eq!(run(Some(egui::Key::Equals)).0, 3.0, "past the menu's steps, on the canvas's ladder");
    assert_eq!(run(Some(egui::Key::Minus)).0, 2.0);
    assert!(app.distort.gallery.is_some(), "the dialog stays open");
    assert_eq!(app.ui.views.first().map(|v| v.zoom), canvas_zoom, "the canvas behind keeps its zoom");
}

/// A huge image can fit below the 1 % minimum: Zoom Out keeps Fit in View rather than
/// enlarging it to 1 %, and Zoom In still steps up from there.
#[test]
fn zoom_out_from_a_fit_below_the_minimum_stays_at_fit() {
    let mut app = app();
    open(&mut app).unwrap();
    let d = app.distort.gallery.as_mut().unwrap();
    d.zoom = 0.0;
    d.fit = 0.005;
    assert_eq!(step_zoom(d, -1), 0.0, "no smaller zoom than Fit: stays at Fit in View");
    assert!(step_zoom(d, 1) > d.fit);
    d.fit = 0.05;
    assert!(step_zoom(d, -1) > 0.0 && step_zoom(d, -1) < 0.05, "a Fit above the minimum still steps down");
}

#[test]
fn stack_editing_and_commit_match_the_engine() {
    let ctx = egui::Context::default();
    let mut app = app();
    let r = crate::distort_ui::menu(&mut app, &ctx, "filter.filterGallery", &json!({})).unwrap().unwrap();
    assert_eq!(r["gallery"]["effects"].as_array().unwrap().len(), 1);
    control(&mut app, &json!({"filter": "cutout", "params": {"numberOfLevels": 3}})).unwrap();
    control(&mut app, &json!({"add": true})).unwrap();
    control(&mut app, &json!({"filter": "texturizer", "params": {"texture": "brick", "relief": 20}})).unwrap();
    control(&mut app, &json!({"add": true})).unwrap();
    control(&mut app, &json!({"filter": "glowingEdges"})).unwrap();
    control(&mut app, &json!({"toggle": 2})).unwrap();
    let d = control(&mut app, &json!({"move": {"from": 1, "to": 0}})).unwrap();
    let keys: Vec<&str> = d["effects"].as_array().unwrap().iter().map(|e| e["filter"].as_str().unwrap()).collect();
    assert_eq!(keys, ["texturizer", "cutout", "glowingEdges"]);
    assert_eq!(d["effects"][2]["visible"], json!(false));
    // Preview pixels = engine result on the same pixels.
    let params = app.distort.gallery.as_ref().unwrap().params();
    let before = app.session.active().unwrap().history.past_len();
    control(&mut app, &json!({"commit": true})).unwrap();
    assert!(app.distort.gallery.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    let (id, p) = app.session.journal.last().cloned().unwrap();
    assert_eq!(id, "filter.filterGallery");
    assert_eq!(p["effects"], params["effects"]);
    // Re-opening starts from the last stack.
    crate::distort_ui::menu(&mut app, &ctx, "filter.filterGallery", &json!({})).unwrap().unwrap();
    assert_eq!(app.distort.gallery.as_ref().unwrap().effects.len(), 3);
    control(&mut app, &json!({"delete": true})).unwrap();
    assert_eq!(app.distort.gallery.as_ref().unwrap().effects.len(), 2);
    control(&mut app, &json!({"cancel": true})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "cancel records nothing");
}

#[test]
fn smart_filter_double_click_edits_the_stored_gallery_stack() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let stack = json!({"effects": [{"filter": "chrome", "params": {"detail": 2, "smoothness": 9}, "visible": true}], "foreground": [1.0, 0.0, 0.0, 1.0], "background": [0.0, 0.0, 1.0, 1.0]});
    app.run("filter.filterGallery", stack).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2.0})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    };
    let steps = app.session.active().unwrap().history.past_len();

    let sm = smart(&app);
    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
    let d = app.distort.gallery.as_ref().expect("the gallery dialog opens");
    assert_eq!((d.smart_filter, d.foreground, d.background), (Some(0), [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]));
    assert_eq!(d.effects.len(), 1);
    assert_eq!((d.effects[0].filter, &d.effects[0].params["detail"]), (GalleryFilter::Chrome, &json!(2)));
    // The preview starts from the pixels below the filter (no blur, no chrome).
    let below = photocraft_engine::smart_cmds::render_below_filter(&app.session.active().unwrap().doc, &sm, 0).unwrap().unwrap();
    let d = app.distort.gallery.as_ref().unwrap();
    assert_eq!(d.src.read_region(d.canvas), below.read_region(d.canvas));

    control(&mut app, &json!({"params": {"detail": 7}})).unwrap();
    control(&mut app, &json!({"commit": true})).unwrap();
    assert!(app.distort.gallery.is_none());
    let sm = smart(&app);
    assert_eq!(sm.smart_filters.len(), 2, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].command, "filter.filterGallery");
    assert_eq!(sm.smart_filters[0].params["effects"][0]["params"]["detail"], json!(7));
    assert_eq!(sm.smart_filters[0].params["foreground"], json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app).smart_filters[0].params["effects"][0]["params"]["detail"], json!(2));

    // Not a gallery filter / no such filter: an error, no dialog.
    assert!(open_smart_filter(&mut app, layer, 1).is_err());
    assert!(open_smart_filter(&mut app, layer, 9).is_err());
    assert!(app.distort.gallery.is_none());
}

#[test]
fn preview_and_thumbnails_render() {
    let ctx = egui::Context::default();
    let mut app = app();
    open(&mut app).unwrap();
    let d = app.distort.gallery.as_mut().unwrap();
    update_preview(d, &ctx, ERect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)), false);
    assert!(d.tex.is_some() && d.shown.is_some());
    d.open = [true; 6];
    update_thumbs(d, &ctx, 100);
    assert!(d.thumbs.iter().all(Option::is_some));
    // Preview equals the engine's algorithm on the sampled pixels.
    let src = sample(&d.src, d.canvas, 1);
    let a = render(&src, &d.params());
    let mut s2 = photocraft_engine::Session::new();
    s2.add_document(app.session.active().unwrap().doc.as_ref().clone(), None);
    s2.select_layer(d.layer).unwrap();
    s2.execute("filter.filterGallery", d.params()).unwrap();
    let st = s2.active().unwrap();
    let b = st.doc.layer(d.layer).unwrap().surface().unwrap().read_region(d.canvas);
    let worst = a.read_region(d.canvas).iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
    assert!(worst < 2e-3, "preview differs from the result by {worst}");
}

/// A 240×160 layer with gradients, edges and fine detail, so every gallery filter has
/// something to draw.
fn photo_app() -> PhotocraftApp {
    let (w, h) = (240, 160);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": w, "height": h, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                    let disc = (((x - 150) * (x - 150) + (y - 70) * (y - 70)) < 45 * 45) as i32 as f32;
                    let stripes = ((x + y) / 11 % 2) as f32;
                    s.write_pixel(x, y, &[u * 0.8 + disc * 0.2, 0.25 + 0.5 * stripes * (1.0 - disc), v * (1.0 - disc) + disc * 0.9, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

/// What OK applies: `filter.filterGallery` with the dialog's stack on a copy of the document.
fn applied(app: &PhotocraftApp, d: &GalleryDialog) -> Surface {
    let mut s2 = photocraft_engine::Session::new();
    s2.add_document(app.session.active().unwrap().doc.as_ref().clone(), None);
    s2.select_layer(d.layer).unwrap();
    s2.execute("filter.filterGallery", d.params()).unwrap();
    s2.active().unwrap().doc.layer(d.layer).unwrap().surface().unwrap().clone()
}

/// Mean of `k`×`k` blocks (opaque layers), written independently of [`reduce`].
fn box_mean(surf: &Surface, r: Rect, k: i32) -> Vec<f32> {
    let mut out = Vec::new();
    let mut px = [0.0f32; 4];
    for by in (r.y0..r.y1).step_by(k as usize) {
        for bx in (r.x0..r.x1).step_by(k as usize) {
            let mut sum = [0.0f32; 4];
            let mut n = 0.0;
            for y in by..(by + k).min(r.y1) {
                for x in bx..(bx + k).min(r.x1) {
                    surf.read_pixel(x, y, &mut px);
                    sum.iter_mut().zip(px).for_each(|(s, v)| *s += v);
                    n += 1.0;
                }
            }
            out.extend(sum.map(|s| s / n));
        }
    }
    out
}

/// Mean absolute RGB difference.
fn mean_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let (sum, n) = a.chunks(4).zip(b.chunks(4)).fold((0.0, 0.0), |(s, n), (p, q)| (s + (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>(), n + 3.0));
    sum / n
}

#[test]
fn zoomed_out_preview_matches_the_applied_result_scaled_down() {
    // #2502: below 100 % the preview filtered every k-th pixel with the same params, so
    // strokes, grain and textures showed several times too large.
    let ctx = egui::Context::default();
    let mut app = photo_app();
    open(&mut app).unwrap();
    let area = ERect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    for f in ["graphicPen", "charcoal", "reticulation", "stainedGlass", "spatter", "chalkCharcoal"] {
        control(&mut app, &json!({"filter": f, "zoom": 0.25})).unwrap();
        let reference = {
            let d = app.distort.gallery.as_ref().unwrap();
            box_mean(&applied(&app, d), d.canvas, 4)
        };
        let d = app.distort.gallery.as_mut().unwrap();
        let (_, vis, k) = view(d, area);
        assert_eq!((vis, k), (d.canvas, 4), "{f}: at 25 % the whole layer shows, averaged 4×4");
        assert!(update_preview(d, &ctx, area, false).is_some());
        let shown = preview_pixels(&d.src, d.canvas, &d.params(), vis, k, &Interrupt::NONE).unwrap();
        let (w, h) = (reduced(vis.width(), k) as i32, reduced(vis.height(), k) as i32);
        let diff = mean_diff(&shown.read_region(Rect::new(0, 0, w, h)), &reference);
        // The old preview (every 4th pixel, filtered at that size) for comparison.
        let old = mean_diff(&render(&sample(&d.src, vis, k), &d.params()).read_region(Rect::new(0, 0, w, h)), &reference);
        assert!(diff < 1e-3, "{f}: preview differs from the applied result by {diff}");
        assert!(old > 20.0 * diff.max(1e-3), "{f}: the old preview differed by only {old}");
    }
}

#[test]
fn a_region_renders_the_same_pixels_as_the_full_result() {
    // Zoomed in, only the visible part renders: noise, cells and textures are seeded by
    // document position, and the filters read their reach around the region.
    let mut app = photo_app();
    open(&mut app).unwrap();
    let region = Rect::new(37, 21, 151, 117);
    for f in GalleryFilter::ALL {
        control(&mut app, &json!({"filter": f.key()})).unwrap();
        let d = app.distort.gallery.as_ref().unwrap();
        let full = applied(&app, d).read_region(region);
        let part = preview_pixels(&d.src, d.canvas, &d.params(), region, 1, &Interrupt::NONE).unwrap();
        let part = part.read_region(Rect::new(0, 0, region.width() as i32, region.height() as i32));
        let worst = full.iter().zip(&part).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-3, "{}: region differs from the full result by {worst}", f.key());
    }
}

#[test]
fn reduce_averages_blocks_weighted_by_alpha() {
    let mut s = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true));
    s.write_pixel(0, 0, &[1.0, 0.0, 0.0, 1.0]);
    s.write_pixel(1, 0, &[0.0, 1.0, 0.0, 0.0]); // transparent: its colour doesn't count
    s.write_pixel(2, 0, &[0.2, 0.4, 0.6, 1.0]); // a partial block at the edge
    let r = reduce(&s, Rect::new(0, 0, 3, 2), 2).read_region(Rect::new(0, 0, 2, 1));
    assert_eq!(r, [1.0, 0.0, 0.0, 0.25, 0.2, 0.4, 0.6, 0.5]);
    assert_eq!(reduce(&s, Rect::EMPTY, 2).content_bounds(), Rect::EMPTY);
}

#[test]
fn background_preview_renders_off_the_ui_thread() {
    let ctx = egui::Context::default();
    let mut app = photo_app();
    open(&mut app).unwrap();
    control(&mut app, &json!({"filter": "graphicPen"})).unwrap();
    let d = app.distort.gallery.as_mut().unwrap();
    let area = ERect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    assert!(update_preview(d, &ctx, area, true).is_none(), "nothing to show until the first render is done");
    assert!(d.describe()["rendering"].as_bool().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while d.rendering().is_some() {
        assert!(std::time::Instant::now() < deadline, "the preview never finished");
        std::thread::sleep(std::time::Duration::from_millis(5));
        update_preview(d, &ctx, area, true);
    }
    assert!(d.tex.is_some() && update_preview(d, &ctx, area, true).is_some());
    // A new setting starts a new render; the last preview stays up meanwhile.
    let before = d.cur().params.get("strokeLength").cloned().unwrap();
    d.cur().params.insert("strokeLength".into(), json!(5));
    assert!(update_preview(d, &ctx, area, true).is_some());
    assert!(d.rendering().is_some());
    // Back to the settings on screen: that render is dropped (and cancelled).
    d.cur().params.insert("strokeLength".into(), before);
    assert!(update_preview(d, &ctx, area, true).is_some());
    assert!(d.rendering().is_none());
    d.cur().params.insert("strokeLength".into(), json!(5));
    update_preview(d, &ctx, area, true);
    // Closing the dialog cancels it.
    app.distort.gallery = None;
}

/// Preview cost on 24 MP: `cargo test -p photocraft-ui-egui --lib bench_gallery_preview -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_gallery_preview() {
    let (w, h) = (6000, 4000);
    let canvas = Rect::new(0, 0, w, h);
    let mut s = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, true));
    s.fill_rect(canvas, &[0.5, 0.4, 0.3, 1.0]);
    for y in (0..h).step_by(9) {
        s.fill_rect(Rect::new(0, y, w, y + 3), &[0.9, 0.8, 0.2, 1.0]);
    }
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    std::hint::black_box(reduce(&s, canvas, 8));
    println!("{:>14}: {:>7.0} ms", "reduce 1/8", ms(t));
    for f in ["graphicPen", "charcoal", "reticulation", "stainedGlass", "spatter", "roughPastels"] {
        let p = json!({"effects": [{"filter": f}]});
        let t = std::time::Instant::now();
        std::hint::black_box(render(&sample(&s, canvas, 8), &p));
        let old = ms(t);
        let t = std::time::Instant::now();
        std::hint::black_box(preview_pixels(&s, canvas, &p, canvas, 8, &Interrupt::NONE));
        let fit = ms(t);
        let t = std::time::Instant::now();
        std::hint::black_box(preview_pixels(&s, canvas, &p, Rect::new(2400, 1600, 3500, 2400), 1, &Interrupt::NONE));
        let screen = ms(t);
        println!("{f:>14}: old fit {old:>5.0} ms, new fit {fit:>6.0} ms, new 100 % (1100x800) {screen:>5.0} ms");
    }
}
