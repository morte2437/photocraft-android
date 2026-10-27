use super::*;
use crate::PhotocraftApp;
use serde_json::json;

fn app_with(params: Value) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", params).unwrap();
    app
}

/// One panel frame through a minimal egui pass.
fn frame(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    ctx.begin_pass(Default::default());
    let mut ui =
        egui::Ui::new(ctx.clone(), egui::Id::unique("hist-test"), egui::UiBuilder::new().max_rect(Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 900.0))));
    show(app, &mut ui);
    ctx.end_pass().textures_delta.clear();
}

/// Frames until the panel shows the histogram it wants (the throttle skipped), or panics after
/// 60 s: an uncached count of the 18 MP document below takes seconds in a debug build on a loaded
/// CI runner, and the loop returns as soon as the histogram lands.
fn settle(app: &mut PhotocraftApp) {
    let t0 = std::time::Instant::now();
    loop {
        for e in [app.doc_hist.as_mut(), app.doc_hist_preview.as_mut()].into_iter().flatten() {
            e.at = -1e12;
        }
        frame(app);
        let done = app.hist_job.is_none() && shown(app).is_some_and(|s| !s.stale);
        if done {
            return;
        }
        assert!(t0.elapsed() < std::time::Duration::from_secs(60), "the histogram never settled");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn ui_set(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, ctx, &req) {
        crate::control::Outcome::Done(v) => v,
        _ => panic!("ui.set: expected an immediate reply"),
    }
}

fn inspect_stats(app: &PhotocraftApp) -> Value {
    inspect(app)["stats"].clone()
}

#[test]
fn stats_match_their_definitions() {
    let mut h = [0u32; 256];
    h[0] = 25;
    h[100] = 50;
    h[255] = 25;
    let s = stats(&h, None).unwrap();
    assert_eq!(s.pixels, 100);
    assert!((s.mean - (5000.0 + 6375.0) / 100.0).abs() < 1e-9);
    assert_eq!(s.median, 100);
    assert_eq!(s.clipped, [25.0, 25.0], "a quarter of the pixels clip at each end");
    // One level: its count and the percent at or below it.
    let one = stats(&h, Some([100, 100])).unwrap();
    assert_eq!((one.count, one.percentile), (Some(50), Some(75.0)));
    // A dragged range (either direction): the count and percent within it.
    let range = stats(&h, Some([200, 50])).unwrap();
    assert_eq!((range.level, range.count, range.percentile), (Some([50, 200]), Some(50), Some(50.0)));
    assert!(stats(&[0; 256], Some([0, 255])).is_none(), "an empty histogram has no statistics");
    assert!(stats(&h, None).unwrap().std_dev > 0.0);
}

#[test]
fn channels_follow_the_colour_mode() {
    let ids = |m| channel_options(m).into_iter().map(|(c, _)| c).collect::<Vec<_>>();
    use HistChannel::*;
    assert_eq!(ids(ColorMode::Rgb), [Composite, Red, Green, Blue, Luminosity, Colors]);
    assert_eq!(ids(ColorMode::Cmyk), [Composite, Cyan, Magenta, Yellow, Black, Luminosity, Colors]);
    assert_eq!(ids(ColorMode::Lab), [Composite, A, B, Luminosity]);
    assert_eq!(ids(ColorMode::Grayscale), [Composite]);
    assert_eq!(resolve_channel(ColorMode::Cmyk, Red), Composite, "a channel the mode lacks shows its composite");
    assert_eq!(resolve_channel(ColorMode::Rgb, Red), Red);
    assert!(color_channels(ColorMode::Grayscale).is_empty(), "All Channels lists colour channels only");
    assert_eq!(color_channels(ColorMode::Rgb), [Red, Green, Blue]);
    // Pointer marks bin pixels the way the histogram does.
    let px = [1.0, 0.5, 0.0, 1.0];
    assert_eq!(channel_level(ColorMode::Rgb, Red, px), 255);
    assert_eq!(channel_level(ColorMode::Rgb, Green, px), 128);
    assert_eq!(channel_level(ColorMode::Rgb, Composite, px), 127);
    assert_eq!(channel_level(ColorMode::Rgb, Luminosity, px), ((0.30 + 0.59 * 0.5) * 255.0f32).round() as u8);
    assert_eq!(channel_level(ColorMode::Rgb, Blue, [f32::NAN, 0.0, 2.0, 1.0]), 255, "hostile samples clamp, never panic");
}

#[test]
fn luminosity_and_rgb_channels_are_counted() {
    let mut app = app_with(json!({"width": 40, "height": 20}));
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let h = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb);
    assert_eq!(bins(&h, doc.mode, HistChannel::Red)[255], 800);
    assert_eq!(bins(&h, doc.mode, HistChannel::Green)[0], 400);
    let red_lum = (0.30f32 * 255.0).round() as usize;
    assert_eq!(h.lum[red_lum], 400, "red is 30% luminosity");
    assert_eq!(h.lum[255], 400, "white is full luminosity");
}

#[test]
fn sources_count_the_image_the_layer_or_the_adjustment_composite() {
    let mut app = app_with(json!({"width": 32, "height": 32})); // white
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let st = app.session.active().unwrap();
    let invert = st.doc.layers[1].id;
    let doc = &st.doc;
    let image = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb);
    assert_eq!((image[0][0], image[0][128]), (512, 512), "black (inverted white) and the grey above it");
    let upto = tone::compute_histograms(doc, HistSource::UpTo(invert), ToneSpace::Rgb);
    assert_eq!(upto[0][0], 1024, "the adjustment composite hides the grey layer above");
    // The panel offers Adjustment Composite only with an adjustment layer selected.
    assert_eq!(resolve_source(st, HistSourceChoice::AdjustmentComposite).0, HistSourceChoice::EntireImage);
    assert_eq!(resolve_source(st, HistSourceChoice::SelectedLayer).1, HistSource::Layer(st.active_layer.unwrap()));
    app.run("layer.select", json!({"layer": invert.0})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(invert));
    assert_eq!(resolve_source(st, HistSourceChoice::AdjustmentComposite), (HistSourceChoice::AdjustmentComposite, HistSource::UpTo(invert)));
    // A single-layer document counts the entire image whatever is chosen.
    let one = app_with(json!({"width": 8, "height": 8}));
    let st = one.session.active().unwrap();
    assert_eq!(resolve_source(st, HistSourceChoice::SelectedLayer).0, HistSourceChoice::EntireImage);
}

#[test]
fn cache_level_follows_size_and_the_preference_cap() {
    let big = app_with(json!({"width": 6000, "height": 3000}));
    let doc = &big.session.active().unwrap().doc;
    let auto = tone::cache_level(doc, tone::MAX_CACHE_LEVEL);
    assert!(auto > 1, "a large document counts a proxy, level {auto}");
    assert_eq!(tone::cache_level(doc, 2), 2, "capped by Cache Levels");
    assert_eq!(tone::cache_level(doc, 0), 1, "out-of-range caps clamp");
    let exact = tone::compute_histograms_at(doc, tone::IMAGE, ToneSpace::Rgb, 1);
    assert_eq!(exact.level, 1);
    assert_eq!(exact[0][255], 6000 * 3000, "level 1 counts every pixel");
    let small = app_with(json!({"width": 64, "height": 64}));
    assert_eq!(tone::cache_level(&small.session.active().unwrap().doc, 8), 1, "small documents are exact");
}

#[test]
fn panel_counts_off_thread_and_recounts_after_an_edit() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    settle(&mut app);
    let s = inspect_stats(&app);
    assert_eq!(s["pixels"], 64 * 48);
    assert_eq!(s["median"], 255);
    assert_eq!(inspect(&app)["current"], true);
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    assert_eq!(inspect(&app)["current"], false, "stale until recounted");
    frame(&mut app);
    // The old histogram stays shown, marked stale, while the new one counts.
    assert!(shown(&mut app).is_some());
    settle(&mut app);
    assert_eq!(inspect_stats(&app)["median"], 0, "refreshed without a manual refresh");
}

#[test]
fn a_dead_histogram_worker_clears_the_job() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    let st = app.session.active().unwrap();
    let key = HistKey { doc: st.doc.id, revision: st.revision, source: 0, preview: 0, max_level: 4 };
    let (tx, rx) = std::sync::mpsc::channel();
    drop(tx);
    app.hist_job = Some((key, rx));
    let ctx = egui::Context::default();
    update(&mut app, &ctx);
    assert!(app.doc_hist.is_none(), "nothing landed from the dead worker");
    // …and the next count was scheduled in its place.
    assert!(app.hist_job.is_some());
}

#[test]
fn uncached_refresh_counts_every_pixel_for_the_current_state() {
    let mut app = app_with(json!({"width": 6000, "height": 3000}));
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 8})).unwrap();
    settle(&mut app);
    assert!(inspect(&app)["cacheLevel"].as_u64().unwrap() > 1, "cached first");
    uncached_refresh(&mut app);
    settle(&mut app);
    assert_eq!(inspect(&app)["cacheLevel"], 1);
    assert_eq!(inspect_stats(&app)["pixels"], 6000u64 * 3000);
    // Cache Levels caps what the panel counts at.
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 2})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    settle(&mut app);
    assert_eq!(inspect(&app)["cacheLevel"], 2);
    assert_eq!(inspect(&app)["maxCacheLevel"], 2);
}

#[test]
fn adjustment_preview_is_counted_over_the_original() {
    let mut app = app_with(json!({"width": 32, "height": 32})); // white
    app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    settle(&mut app);
    assert_eq!(inspect_stats(&app)["median"], 255);
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.live_adjust = Some((id, json!({"outWhite": 100})));
    settle(&mut app);
    assert_eq!(inspect(&app)["preview"], true);
    assert_eq!(inspect_stats(&app)["median"], 100, "the previewed levels");
    let s = shown(&mut app).unwrap();
    let original = s.original.expect("the original is kept for the overlay");
    assert_eq!(original[0][255], 1024, "the original is the image before the preview");
    app.live_adjust = None;
    settle(&mut app);
    assert_eq!(inspect(&app)["preview"], false);
    assert_eq!(inspect_stats(&app)["median"], 255);
}

#[test]
fn views_keep_the_channel_and_the_menu_toggles_options() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let mut app = app_with(json!({"width": 16, "height": 16}));
    app.ui.histogram_panel.channel = HistChannel::Green;
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(|ui, app: &mut PhotocraftApp| panel_menu(app, ui), app);
    h.run_steps(2);
    h.get_by_label("All Channels View").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.histogram_panel.view, HistView::AllChannels);
    assert_eq!(h.state().ui.histogram_panel.channel, HistChannel::Green, "the channel is kept");
    h.get_by_label("Show Channels in Color").click();
    h.run_steps(2);
    assert!(h.state().ui.histogram_panel.show_channels_in_color);
    h.get_by_label("Show Statistics").click();
    h.run_steps(2);
    assert!(!h.state().ui.histogram_panel.show_statistics);
    // Kept with the preferences.
    let mut again = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let saved = h.state().session.prefs().dialogs.get(OPTIONS_PREF).cloned().unwrap();
    again.session.prefs.edit(|p| {
        p.dialogs.insert(OPTIONS_PREF.into(), saved);
    });
    restore_options(&mut again);
    assert_eq!(again.ui.histogram_panel.view, HistView::AllChannels);
    assert!(again.ui.histogram_panel.show_channels_in_color);
}

#[test]
fn every_view_and_channel_draws_without_panicking() {
    for mode in ["rgb", "cmyk", "lab", "grayscale"] {
        let mut app = app_with(json!({"width": 24, "height": 24, "mode": mode}));
        app.run("layer.new.layer", json!({})).unwrap();
        settle(&mut app);
        for view in [HistView::Compact, HistView::Expanded, HistView::AllChannels] {
            for channel in [HistChannel::Composite, HistChannel::Red, HistChannel::Cyan, HistChannel::A, HistChannel::Luminosity, HistChannel::Colors] {
                for range in [[200, 10], [0, 255]] {
                    app.ui.histogram_panel.view = view;
                    app.ui.histogram_panel.channel = channel;
                    app.ui.histogram_range = Some(range);
                    frame(&mut app);
                }
            }
        }
        assert!(inspect(&app)["stats"]["count"].is_u64(), "{mode}: the range has a count");
    }
}

#[test]
fn overlaid_colors_mix_where_channels_overlap() {
    let ctx = egui::Context::default();
    ctx.begin_pass(Default::default());
    let p = ctx.layer_painter(egui::LayerId::background());
    let h = [100u32; 256];
    draw_overlaid(&p, Rect::from_min_size(egui::Pos2::ZERO, vec2(256.0, 50.0)), &[(&h, Color32::RED), (&h, Color32::GREEN)], false);
    draw_overlaid(&p, Rect::from_min_size(egui::Pos2::ZERO, vec2(256.0, 50.0)), &[], true);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn ui_set_drives_the_panel() {
    let mut app = app_with(json!({"width": 16, "height": 16}));
    let ctx = egui::Context::default();
    let r = ui_set(
        &mut app,
        &ctx,
        json!({"histogramPanel": {"view": "allChannels", "channel": "luminosity"}, "histogramRange": [10, 20], "histogramRefresh": true}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.histogram_panel.view, HistView::AllChannels);
    assert_eq!(app.ui.histogram_panel.channel, HistChannel::Luminosity);
    assert_eq!(app.ui.histogram_range, Some([10, 20]));
    assert!(app.hist_uncached.is_some());
    for bad in [
        json!({"histogramPanel": {"view": "tiny"}}),
        json!({"histogramPanel": {"nope": 1}}),
        json!({"histogramRange": [0, 300]}),
        json!({"histogramRefresh": "yes"}),
    ] {
        let r = ui_set(&mut app, &ctx, bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(app.ui.histogram_panel.view, HistView::AllChannels, "a rejected call changes nothing");
}

/// The Colors overlay follows the same outline as a single channel: straight lines between bin
/// centres, not one flat step per level, and its bands tile without gaps where outlines cross.
#[test]
fn colors_overlay_is_a_continuous_outline_not_steps() {
    let r = Rect::from_min_size(egui::Pos2::ZERO, vec2(512.0, 100.0));
    let ramp: [u32; 256] = std::array::from_fn(|i| i as u32 * 10);
    let fall: [u32; 256] = std::array::from_fn(|i| (255 - i) as u32 * 10);
    let mesh = overlaid_mesh(r, &[(&ramp, Color32::RED), (&fall, Color32::GREEN)], false);
    let quads: Vec<&[egui::epaint::Vertex]> = mesh.vertices.chunks(4).collect();
    assert!(!quads.is_empty());
    // Every band's top edge slopes on a ramp: no flat steps.
    let sloped = quads.iter().filter(|q| (q[1].pos.y - q[2].pos.y).abs() > 1e-4).count();
    assert!(sloped * 10 > quads.len() * 9, "{sloped} of {} bands slope", quads.len());
    // The two ramps cross mid-way: there the overlap is yellow and each side keeps its own colour.
    let colors: std::collections::HashSet<_> = mesh.vertices.iter().map(|v| v.color).collect();
    assert!(colors.contains(&Color32::from_rgb(255, 255, 0)) && colors.contains(&Color32::RED) && colors.contains(&Color32::GREEN));
    // Nothing is drawn above the taller outline at any x.
    for q in &quads {
        for v in q.iter() {
            assert!(v.pos.y >= r.top() - 1e-3 && v.pos.y <= r.bottom() + 1e-3);
        }
    }
}

/// A white Background under a black layer, with the Background selected: the entire image is
/// black, the selected layer white.
fn black_over_white_background_selected() -> PhotocraftApp {
    let mut app = app_with(json!({"width": 32, "height": 32}));
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    let background = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    app
}

/// Preferences › Cache Levels 1 counts at level 1 like an Uncached Refresh, but only the refresh
/// was asked for: edits in a row still wait out the 250 ms throttle.
#[test]
fn cache_levels_one_still_throttles_recounts_and_an_uncached_refresh_does_not() {
    let mut app = app_with(json!({"width": 64, "height": 48}));
    app.run("prefs.set", json!({"path": "performance.cacheLevels", "value": 1})).unwrap();
    settle(&mut app);
    // The count just landed, so the next edit falls inside the throttle.
    app.doc_hist.as_mut().unwrap().at = 0.0;
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    frame(&mut app);
    assert!(app.hist_job.is_none(), "an edit within 250 ms of the last count waits");
    uncached_refresh(&mut app);
    frame(&mut app);
    assert!(app.hist_job.is_some(), "an Uncached Refresh counts at once");
}

/// The Levels, Curves and Threshold dialogs count uncapped: their proxy is as small as before
/// Cache Levels (about 768 pixels on the longest side), and only a cap halves by powers of two.
#[test]
fn uncapped_counts_keep_their_proxy_and_a_cap_reduces_by_powers_of_two() {
    let big = app_with(json!({"width": 6000, "height": 3000}));
    let doc = &big.session.active().unwrap().doc;
    let pixels = |h: &Histograms| h[0].iter().map(|&c| u64::from(c)).sum::<u64>();
    // 6000 / 768 = 7: a 7x proxy, 858 x 429 (rounded up), not an 8x one.
    let auto = tone::compute_histograms(doc, tone::IMAGE, ToneSpace::Rgb);
    assert_eq!(pixels(&auto), 858 * 429);
    let capped = tone::compute_histograms_at(doc, tone::IMAGE, ToneSpace::Rgb, 3);
    assert_eq!(capped.level, 3);
    assert_eq!(pixels(&capped), 1500 * 750, "level 3 is a 4x proxy");
}

/// Photoshop opens the panel in Compact View, which shows no Source menu and represents the
/// entire image whatever Source the Expanded View was left at.
#[test]
fn compact_view_is_the_default_and_counts_the_entire_image() {
    assert_eq!(HistogramPanelState::default().view, HistView::Compact);
    let mut app = black_over_white_background_selected();
    app.ui.histogram_panel = HistogramPanelState { source: HistSourceChoice::SelectedLayer, ..Default::default() };
    settle(&mut app);
    let i = inspect(&app);
    assert_eq!(i["source"], "entireImage");
    assert_eq!(i["stats"]["median"], 0, "the black layer over the Background, not the Background alone");
    app.ui.histogram_panel.view = HistView::Expanded;
    settle(&mut app);
    assert_eq!(inspect(&app)["source"], "selectedLayer");
    assert_eq!(inspect_stats(&app)["median"], 255, "Expanded View counts the chosen Source");
}

/// `current` says whether the shown histogram is the one the panel wants: a Source change makes
/// it stale until the recount lands, though the document's revision didn't move.
#[test]
fn inspect_current_follows_the_source_not_only_the_revision() {
    let mut app = black_over_white_background_selected();
    app.ui.histogram_panel.view = HistView::Expanded;
    settle(&mut app);
    assert_eq!(inspect(&app)["current"], true);
    let ctx = egui::Context::default();
    let r = ui_set(&mut app, &ctx, json!({"histogramPanel": {"source": "selectedLayer"}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(inspect(&app)["current"], false, "the entire image's histogram is not the selected layer's");
    settle(&mut app);
    assert_eq!(inspect(&app)["current"], true);
    assert_eq!(inspect_stats(&app)["median"], 255);
}

/// `preview` reports only the active document's preview, not one left from another document.
#[test]
fn inspect_preview_is_the_active_documents() {
    let mut app = app_with(json!({"width": 16, "height": 16}));
    settle(&mut app);
    let mut other = app.doc_hist.clone().unwrap();
    other.key.preview = 1;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id, other.key.doc);
    app.doc_hist_preview = Some(other);
    assert_eq!(inspect(&app)["preview"], false);
}
