use super::*;
use egui_kittest::Harness;
use photocraft_color::{Color, PixelFormat};
use photocraft_compose::Buffer;
use photocraft_compose::adjust::{Transfer, apply_depth};
use photocraft_doc::{Document, Size};

const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];

fn lv(in_black: f32, gamma: f32, in_white: f32) -> LevelsChannel {
    LevelsChannel { in_black: in_black / 255.0, gamma, in_white: in_white / 255.0, ..LevelsChannel::default() }
}

fn levels(master: LevelsChannel, per_channel: [LevelsChannel; 3]) -> Adjustment {
    Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, black: LevelsChannel::default() }
}

/// Samples to try at `depth`: every 8-bit level, a 16-bit sweep, and float values past both ends.
fn samples(depth: SampleType) -> Vec<f32> {
    match depth {
        SampleType::U8 => (0..=255).map(|n| n as f32 / 255.0).collect(),
        SampleType::U16 => (0..=65535).step_by(13).map(|n| n as f32 / 65535.0).collect(),
        SampleType::F32 => (-40..=1100).map(|n| n as f32 / 1000.0).collect(),
    }
}

/// With gamma 1 and the output levels at their defaults, a channel is clipped exactly where the
/// compositor's Levels result sits at an end: 0 for black, 1 for white (the 8-bit LUT staircase
/// and the 32-bit formula; 16-bit against the whole-level chain the LUT samples).
#[test]
fn clipping_matches_the_levels_result() {
    let master = lv(30.0, 1.0, 200.0);
    let channels = [lv(10.0, 1.0, 240.0), LevelsChannel::default(), lv(60.0, 1.0, 120.0)];
    let adj = levels(master.clone(), channels.clone());
    for depth in DEPTHS {
        let quantum = photocraft_compose::adjustment_quantum(depth);
        for v in samples(depth) {
            let result: [f32; 3] = if depth == SampleType::U16 {
                std::array::from_fn(|c| levels_q(&master, levels_q(&channels[c], v, quantum), quantum))
            } else {
                let mut b = Buffer::filled(Rect::new(0, 0, 1, 1), [v, v, v, 1.0]);
                apply_depth(&adj, &mut b, Transfer::for_document(ColorMode::Rgb, depth), Some(depth));
                [b.px[0][0], b.px[0][1], b.px[0][2]]
            };
            for (c, (ch, r)) in channels.iter().zip(result).enumerate() {
                let (black, white) = classify(&master, ch, v, quantum);
                assert_eq!(black, r <= 0.0, "{depth:?} v={v} channel {c}: {r}");
                assert_eq!(white, r >= 1.0, "{depth:?} v={v} channel {c}: {r}");
            }
        }
    }
}

/// Gamma and the output levels don't change what the input levels clip, so the display tests the
/// input-range clamps rather than the final value. Narrowed output levels never reach black or
/// white, and gamma below 1 rounds the 8-bit levels just above the black point to black although
/// the slider keeps them. (Gamma above 1 reaches white at the top: its toe is at black.)
#[test]
fn gamma_and_output_levels_do_not_hide_clipping() {
    let q = Some(255.0);
    let master = LevelsChannel { gamma: 1.5, ..lv(20.0, 1.0, 200.0) };
    let plain = lv(20.0, 1.0, 200.0);
    let outputs = LevelsChannel { out_black: 25.0 / 255.0, out_white: 230.0 / 255.0, gamma: 0.6, ..lv(20.0, 1.0, 200.0) };
    let crushed = LevelsChannel { gamma: 0.5, ..lv(20.0, 1.0, 200.0) };
    assert!(levels_q(&outputs, 0.0, q) > 0.0 && levels_q(&outputs, 1.0, q) < 1.0, "narrowed outputs never reach black or white");
    assert_eq!(levels_q(&crushed, 21.0 / 255.0, q), 0.0, "gamma 0.5 rounds level 21 to black");
    for n in 0..=255 {
        let v = n as f32 / 255.0;
        let id = LevelsChannel::default();
        let want = (n <= 20, n >= 200);
        assert_eq!(classify(&master, &id, v, q), want, "gamma 1.5 composite, level {n}");
        assert_eq!(classify(&plain, &id, v, q), want, "level {n}");
        assert_eq!(classify(&outputs, &id, v, q), want, "outputs and gamma 0.6, level {n}");
        assert_eq!(classify(&crushed, &id, v, q), want, "gamma 0.5 composite, level {n}");
        assert_eq!(classify(&id, &master, v, q), want, "gamma 1.5 channel, level {n}");
    }
}

/// The per-channel thresholds give the same answer as [`classify`] for every key tried.
#[test]
fn thresholds_agree_with_classify() {
    let cases = [
        levels(lv(30.0, 1.0, 200.0), Default::default()),
        levels(LevelsChannel::default(), [lv(10.0, 0.4, 240.0), lv(0.0, 2.5, 90.0), lv(128.0, 1.0, 130.0)]),
        levels(lv(5.0, 3.0, 250.0), [LevelsChannel { out_black: 0.2, out_white: 0.9, ..lv(40.0, 0.7, 220.0) }, lv(0.0, 1.0, 255.0), lv(200.0, 9.0, 255.0)]),
    ];
    for adj in &cases {
        let Adjustment::Levels { master, per_channel, .. } = adj else { panic!("Levels") };
        for depth in DEPTHS {
            let quantum = photocraft_compose::adjustment_quantum(depth);
            let thr = thresholds(adj, depth).unwrap();
            let keys: Vec<u16> = match depth {
                SampleType::U8 => (0..=255u16).map(|n| n * 257).collect(),
                _ => (0..=65535u16).step_by(7).chain([u16::MAX]).collect(),
            };
            for k in keys {
                for (c, (ch, (lo, hi))) in per_channel.iter().zip(thr).enumerate() {
                    let (black, white) = classify(master, ch, f32::from(k) / 65535.0, quantum);
                    assert_eq!(black, lo.is_some_and(|lo| k <= lo), "{depth:?} key {k} channel {c} black");
                    assert_eq!(white, hi.is_some_and(|hi| k >= hi), "{depth:?} key {k} channel {c} white");
                }
            }
        }
    }
}

/// Photoshop's threshold display: black slider, a clipped channel drops to 0 on white; white
/// slider, a clipped channel rises to 255 on black.
#[test]
fn colours_follow_the_clipped_channels() {
    let rgb = Color32::from_rgb;
    assert_eq!(colour(ClipEnd::Shadows, 0, 7), Color32::WHITE);
    assert_eq!(colour(ClipEnd::Shadows, 0b001, 0), rgb(0, 255, 255), "red alone: cyan");
    assert_eq!(colour(ClipEnd::Shadows, 0b110, 0), rgb(255, 0, 0), "green and blue: red");
    assert_eq!(colour(ClipEnd::Shadows, 7, 0), Color32::BLACK);
    assert_eq!(colour(ClipEnd::Highlights, 7, 0), Color32::BLACK);
    assert_eq!(colour(ClipEnd::Highlights, 0, 0b001), rgb(255, 0, 0), "red alone: red");
    assert_eq!(colour(ClipEnd::Highlights, 0, 0b011), rgb(255, 255, 0));
    assert_eq!(colour(ClipEnd::Highlights, 0, 7), Color32::WHITE);
}

fn surface(fmt: PixelFormat, w: u32, h: u32, px: impl Fn(u32, u32) -> Vec<f32>) -> Surface {
    let mut s = Surface::new(fmt);
    let data: Vec<f32> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| px(x, y)).collect();
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    s
}

/// A block shows what any counted pixel in it clips; transparent, unselected (under half) and
/// non-finite pixels don't count.
#[test]
fn blocks_keep_lone_extremes_of_the_pixels_that_count() {
    let s = surface(PixelFormat::RGBA32F, 8, 8, |x, y| match (x, y) {
        (5, 5) => vec![0.0, 0.5, 0.5, 1.0],
        (1, 1) => vec![1.0, 1.0, 1.0, 0.0],
        (2, 2) => vec![f32::NAN, 1.0, 1.0, 1.0],
        (7, 1) => vec![1.0, 1.0, 1.0, 1.0],
        _ => vec![0.5, 0.5, 0.5, 1.0],
    });
    let thr = [(key(0.1), key(0.9)); 3];
    let grey = surface(PixelFormat::GRAY8, 8, 8, |x, _| vec![if x < 6 { 1.0 } else { 0.4 }]);
    for (selection, top_right) in [(None, Color32::WHITE), (Some(&grey), Color32::BLACK)] {
        let stats = scan(&s, selection, Rect::new(0, 0, 8, 8), 4);
        assert_eq!(stats.size, [2, 2]);
        let black = image(&stats, &thr, ClipEnd::Shadows);
        assert_eq!(black, vec![Color32::WHITE, Color32::WHITE, Color32::WHITE, Color32::from_rgb(0, 255, 255)], "{selection:?}");
        let white = image(&stats, &thr, ClipEnd::Highlights);
        assert_eq!(white, vec![Color32::BLACK, top_right, Color32::BLACK, Color32::BLACK], "{selection:?}");
    }
}

/// Gray documents' Levels act on one channel, so the display is black and white only.
#[test]
fn gray_layers_show_black_and_white_only() {
    let s = surface(PixelFormat::GRAYA8, 16, 1, |x, _| vec![x as f32 / 15.0, 1.0]);
    let adj = levels(lv(100.0, 1.0, 180.0), Default::default());
    let thr = thresholds(&adj, SampleType::U8).unwrap();
    let stats = scan(&s, None, Rect::new(0, 0, 16, 1), 1);
    for end in [ClipEnd::Shadows, ClipEnd::Highlights] {
        for (x, c) in image(&stats, &thr, end).into_iter().enumerate() {
            let v = x as f32 / 15.0 * 255.0;
            let clipped = if end == ClipEnd::Shadows { v <= 100.0 } else { v >= 180.0 };
            assert!(c == Color32::BLACK || c == Color32::WHITE, "x={x}: {c:?}");
            assert_eq!(c == colour(end, 7, 7), clipped, "{end:?} x={x}");
        }
    }
}

/// Out-of-range and non-finite settings never panic.
#[test]
fn hostile_settings_do_not_panic() {
    let bad = LevelsChannel { in_black: f32::NAN, in_white: -3.0, gamma: 0.0, out_black: f32::INFINITY, out_white: f32::NEG_INFINITY };
    for adj in [levels(bad.clone(), [bad.clone(), LevelsChannel::default(), lv(255.0, 100.0, 0.0)]), levels(LevelsChannel::default(), Default::default())] {
        for depth in DEPTHS {
            let _ = thresholds(&adj, depth);
        }
    }
    let cmyk =
        Adjustment::Levels { master: LevelsChannel::default(), per_channel: Default::default(), space: ToneSpace::Cmyk, black: LevelsChannel::default() };
    assert!(thresholds(&cmyk, SampleType::U8).is_none());
    assert!(thresholds(&Adjustment::Invert, SampleType::U8).is_none());
    for params in [json!({"inBlack": 1e9, "inWhite": -5, "gamma": 0}), json!({"red": {"gamma": 100, "inWhite": 0}}), json!({"inBlack": "x"})] {
        if let Ok(adj) = photocraft_engine::adjust_params::from_params("levels", &params, None, ColorMode::Rgb) {
            let _ = thresholds(&adj, SampleType::U16);
        }
    }
}

/// A 16×8 document: the background's red channel ramps 0..1 left to right, green is 0.5, blue 1.
fn ramp_app(mode: ColorMode, depth: SampleType) -> PhotocraftApp {
    let mut doc = Document::with_background("t", Size::new(16, 8), mode, depth, Color::rgba(1.0, 1.0, 1.0, 1.0));
    let gray = mode == ColorMode::Grayscale;
    if let Some(s) = doc.layers[0].surface_mut() {
        let data: Vec<f32> =
            (0..8).flat_map(|_| 0..16).flat_map(|x| if gray { vec![x as f32 / 15.0, 1.0] } else { vec![x as f32 / 15.0, 0.5, 1.0, 1.0] }).collect();
        s.write_region(Rect::new(0, 0, 16, 8), &data);
    }
    let target = doc.layers[0].id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    session.select_layer(target).unwrap();
    PhotocraftApp::new(session, crate::Services::default())
}

fn open_levels(app: &mut PhotocraftApp, params: Value, end: Option<&str>) -> u64 {
    let id = crate::adjust_dialog::open(app, "image.adjustments.levels").unwrap();
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    if let Value::Object(p) = params {
        fields.extend(p);
    }
    if let Some(end) = end {
        fields.insert(FIELD.into(), json!(end));
    }
    id
}

/// A canvas view of the 16×8 document at `zoom` points per pixel (1 pixel per point).
fn view(zoom: f32) -> ViewXform {
    ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(16.0 * zoom, 8.0 * zoom)),
        zoom,
        center: [8.0, 4.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    }
}

/// [`paint`] in a frame of its own (so its mesh doesn't spill into a later frame's shapes), then
/// the colours of the blocks it shows.
fn paint_view(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform) -> Option<Vec<Color32>> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let painter = egui::Painter::new(ui.ctx().clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
        paint(app, ui.ctx(), &painter, xf, 0);
    });
    out.textures_delta.clear();
    let c = app.levels_clipping.as_ref()?;
    let (end, thr) = c.drawn?;
    Some(image(&c.stats, &thr, end))
}

/// A view at one device pixel per document pixel whose visible document rect (with
/// `visible_doc_rect`'s 2 px margin) is exactly `x0..x1` × `y0..y1`.
fn xf_over(x0: i32, x1: i32, y0: i32, y1: i32) -> ViewXform {
    let size = egui::vec2((x1 - x0 - 4) as f32, (y1 - y0 - 4) as f32);
    ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, size),
        zoom: 1.0,
        center: [(x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    }
}

/// Snapping the area to the block grid can widen it by up to a block; the blocks grow until the
/// snapped area still fits the texture limit (here a 2048 px device limit) on both sides.
#[test]
fn the_snapped_area_fits_the_texture_limit() {
    let bounds = Rect::new(0, 0, 20000, 20000);
    let (r, f) = view_area(&xf_over(1, 4097, 0, 100), bounds, 1.0, 2048).unwrap();
    assert_eq!((r.x0, f), (0, 3), "4096 px from x 1: blocks of 2 would need 2049");
    for (x0, y0) in [(0, 0), (1, 1), (2, 3), (5, 7), (1023, 2047)] {
        for span in 4090..4100 {
            let (x1, y1) = (x0 + span, y0 + span / 2);
            let (r, f) = view_area(&xf_over(x0, x1, y0, y1), bounds, 1.0, 2048).unwrap();
            let fi = f as i32;
            assert!(r.width().div_ceil(f) <= 2048 && r.height().div_ceil(f) <= 2048, "{x0},{y0} span {span}: {r:?} / {f}");
            assert!(r.x0 % fi == 0 && r.y0 % fi == 0, "snapped to the grid: {r:?} / {f}");
            assert!(r.x0 <= x0 && r.y0 <= y0 && r.x1 >= x1 && r.y1 >= y1, "covers the view: {r:?}");
        }
    }
}

/// However big the display, the display builds at most `MAX_BLOCKS` blocks: an 8K canvas at
/// 100 % gets bigger blocks instead of a 33M-texel texture, while 2560×1440 keeps one block per
/// device pixel.
#[test]
fn a_huge_display_grows_the_blocks() {
    let bounds = Rect::new(0, 0, 20000, 20000);
    let (_, f) = view_area(&xf_over(0, 2564, 0, 1444), bounds, 1.0, 8192).unwrap();
    assert_eq!(f, 1, "2560×1440: one block per device pixel");
    for (w, h) in [(7680, 4320), (8192, 8192), (15360, 8640)] {
        let (r, f) = view_area(&xf_over(0, w, 0, h), bounds, 1.0, 8192).unwrap();
        let blocks = u64::from(r.width().div_ceil(f)) * u64::from(r.height().div_ceil(f));
        assert!(blocks <= MAX_BLOCKS, "{w}×{h}: {blocks} blocks of {f}");
        assert!(blocks > MAX_BLOCKS / 4, "{w}×{h}: no coarser than needed ({blocks} blocks of {f})");
    }
}

/// The main canvas's view of the 16×8 document at 10 device pixels per pixel.
fn main_view(app: &mut PhotocraftApp) -> crate::state::View {
    app.sync_views();
    crate::state::View { zoom: 10.0, center: [8.0, 4.0], fit_pending: false, fill_pending: false, ..app.ui.views[0].clone() }
}

/// The shapes of one 160×80 canvas frame of document 0 in `view`.
fn canvas_frame(ctx: &egui::Context, app: &mut PhotocraftApp, view: &crate::state::View, primary: bool) -> Vec<egui::epaint::ClippedShape> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        crate::canvas::canvas_view(app, ui, 0, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(160.0, 80.0)), view.clone(), primary);
    });
    out.textures_delta.clear();
    out.shapes
}

/// How many meshes among `shapes` (shape lists included) draw texture `tex`.
fn meshes_with(shapes: &[egui::epaint::ClippedShape], tex: egui::TextureId) -> usize {
    fn count(s: &egui::Shape, tex: egui::TextureId) -> usize {
        match s {
            egui::Shape::Mesh(m) => usize::from(m.texture_id == tex),
            egui::Shape::Vec(v) => v.iter().map(|s| count(s, tex)).sum(),
            _ => 0,
        }
    }
    shapes.iter().map(|c| count(&c.shape, tex)).sum()
}

/// The display keeps one cache, sized to the main canvas's view: an extra document window's
/// canvas (not primary) neither draws it nor replaces that cache with its own view.
#[test]
fn only_the_main_canvas_draws_the_display() {
    let ctx = egui::Context::default();
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    open_levels(&mut app, json!({"inBlack": 100}), Some("shadows"));
    let main = main_view(&mut app);
    let other = crate::state::View { zoom: 0.25, ..main.clone() };
    let shapes = canvas_frame(&ctx, &mut app, &main, true);
    let cache = app.levels_clipping.as_ref().expect("the main canvas builds the display");
    let (key, tex) = (cache.key, cache.tex.as_ref().map(egui::TextureHandle::id).expect("its texture"));
    assert_eq!(meshes_with(&shapes, tex), 1, "the main canvas draws the display");
    let shapes = canvas_frame(&ctx, &mut app, &other, false);
    assert_eq!(meshes_with(&shapes, tex), 0, "an extra window doesn't draw it");
    assert_eq!(app.levels_clipping.as_ref().map(|c| c.key), Some(key), "nor replaces the main canvas's cache");
}

/// The display covers the view at about one texel per screen pixel, in every depth and in Gray,
/// and goes away with the dialog.
#[test]
fn paint_shows_the_target_clipping_for_the_view() {
    let ctx = egui::Context::default();
    for (mode, depth) in
        [(ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Rgb, SampleType::F32), (ColorMode::Grayscale, SampleType::U8)]
    {
        let mut app = ramp_app(mode, depth);
        let id = open_levels(&mut app, json!({"inBlack": 100, "inWhite": 200}), Some("shadows"));
        let px = paint_view(&mut app, &ctx, &view(10.0)).unwrap_or_else(|| panic!("{mode:?} {depth:?}"));
        assert_eq!(app.levels_clipping.as_ref().unwrap().stats.size, [16, 8]);
        let gray = mode == ColorMode::Grayscale;
        let clipped = if gray { Color32::BLACK } else { Color32::from_rgb(0, 255, 255) };
        for (x, p) in px.iter().take(16).enumerate() {
            let want = if x as f32 / 15.0 * 255.0 <= 100.0 { clipped } else { Color32::WHITE };
            assert_eq!(*p, want, "{mode:?} {depth:?} shadows x={x}");
        }
        app.ui.dialog_mut(id).unwrap().fields.insert(FIELD.into(), json!("highlights"));
        let px = paint_view(&mut app, &ctx, &view(10.0)).unwrap();
        for (x, p) in px.iter().skip(16 * 7).enumerate() {
            let red = x as f32 / 15.0 * 255.0 >= 200.0;
            let want = match (gray, red) {
                (true, true) => Color32::WHITE,
                (true, false) => Color32::BLACK,
                (false, true) => Color32::from_rgb(255, 0, 255),
                (false, false) => Color32::from_rgb(0, 0, 255),
            };
            assert_eq!(*p, want, "{mode:?} {depth:?} highlights x={x}");
        }
        // Zoomed out, a texel covers a block of pixels.
        paint_view(&mut app, &ctx, &view(0.25)).unwrap();
        assert_eq!(app.levels_clipping.as_ref().unwrap().stats.factor, 4);
        assert_eq!(app.levels_clipping.as_ref().unwrap().stats.size, [4, 2]);
        // Off with the dialog still open: the cached texture stays but no canvas frame draws it.
        let main = main_view(&mut app);
        let shapes = canvas_frame(&ctx, &mut app, &main, true);
        let tex = app.levels_clipping.as_ref().and_then(|c| c.tex.as_ref()).map(egui::TextureHandle::id).unwrap();
        assert_eq!(meshes_with(&shapes, tex), 1, "{mode:?} {depth:?}: on, it draws");
        app.ui.dialog_mut(id).unwrap().fields.insert(FIELD.into(), Value::Null);
        assert!(request(&app, 0).is_none());
        let shapes = canvas_frame(&ctx, &mut app, &main, true);
        assert_eq!(meshes_with(&shapes, tex), 0, "{mode:?} {depth:?}: off, it draws nothing");
        assert!(app.levels_clipping.is_some(), "{mode:?} {depth:?}: the cache stays while the dialog is open");
        // Closed: the cache goes.
        app.ui.close_dialog(id);
        paint_view(&mut app, &ctx, &view(10.0));
        assert!(app.levels_clipping.is_none(), "{mode:?} {depth:?}");
    }
}

/// CMYK and Lab documents, a Quick Mask, and a target that isn't a pixel layer keep the normal
/// preview.
#[test]
fn ineligible_documents_show_no_display() {
    for mode in ["cmyk", "lab"] {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8, "mode": mode})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        open_levels(&mut app, json!({}), Some("shadows"));
        assert!(request(&app, 0).is_none(), "{mode}");
    }
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    app.run("select.editInQuickMaskMode", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.quick_mask.is_some());
    open_levels(&mut app, json!({}), Some("shadows"));
    assert!(request(&app, 0).is_none(), "Quick Mask");
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    app.run("layer.newFillLayer.solidColor", json!({"color": "#808080"})).unwrap();
    open_levels(&mut app, json!({}), Some("shadows"));
    assert!(request(&app, 0).is_none(), "fill layer");
}

/// While a background job runs the display draws nothing and neither scans nor builds its
/// texture. It comes back once the job is done.
#[test]
fn a_running_job_holds_the_display() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let ctx = egui::Context::default();
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    open_levels(&mut app, json!({"inBlack": 100}), Some("shadows"));
    let gate = Arc::new(AtomicBool::new(false));
    let open = gate.clone();
    let started = app
        .session
        .start_job(
            "test.slow",
            json!({}),
            "Slow",
            false,
            move |job| {
                while !open.load(Ordering::Relaxed) {
                    job.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            |_, ()| Ok(json!({})),
        )
        .unwrap();
    let photocraft_engine::jobs::Started::Job(job) = started else { panic!("ran inline") };
    assert!(app.session.has_jobs());
    assert!(paint_view(&mut app, &ctx, &view(10.0)).is_none());
    assert!(app.levels_clipping.is_none(), "no scan and no texture while the job runs");
    gate.store(true, Ordering::Relaxed);
    app.session.wait_job(job).unwrap();
    assert!(!app.session.has_jobs());
    assert!(paint_view(&mut app, &ctx, &view(10.0)).is_some(), "back once the job is done");
}

/// Closing the last document drops the display's cache: no canvas is left to paint it away.
#[test]
fn closing_the_last_document_drops_the_cache() {
    let ctx = egui::Context::default();
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    open_levels(&mut app, json!({"inBlack": 100}), Some("shadows"));
    assert!(paint_view(&mut app, &ctx, &view(10.0)).is_some());
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.documents().is_empty());
    assert!(app.levels_clipping.is_none());
}

/// A Smart Filter's Levels dialog edits the filter, not the layer's pixels: no display, and its
/// cache goes as if no Levels dialog were open, even over a pixel layer.
#[test]
fn smart_filter_levels_show_no_display() {
    let ctx = egui::Context::default();
    let mut app = ramp_app(ColorMode::Rgb, SampleType::U8);
    let id = open_levels(&mut app, json!({"inBlack": 100}), Some("shadows"));
    assert!(paint_view(&mut app, &ctx, &view(10.0)).is_some(), "a plain Levels dialog shows it");
    let layer = app.session.active().and_then(|s| s.active_layer).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("__smartFilter".into(), json!({"layer": layer.0}));
    assert!(request(&app, 0).is_none());
    assert!(paint_view(&mut app, &ctx, &view(10.0)).is_none());
    assert!(app.levels_clipping.is_none(), "the cache goes");
}

fn dialog_harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

fn clipping(h: &Harness<'static, PhotocraftApp>) -> Option<Value> {
    h.state().ui.dialogs.first().and_then(|d| d.fields.get(FIELD).cloned())
}

fn press(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) {
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn modifiers(h: &mut Harness<'static, PhotocraftApp>, m: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

/// Alt-dragging the black slider shows the shadows display while both are held, the white slider
/// the highlights; the gamma slider, a drag without Alt and the Properties panel show nothing.
/// The document stays untouched throughout.
#[test]
fn alt_drag_on_the_black_and_white_sliders_turns_the_display_on() {
    let mut h = dialog_harness(ramp_app(ColorMode::Rgb, SampleType::U8));
    let original = h.state().session.active().unwrap().doc.clone();
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.levels").unwrap();
    h.run_steps(3);
    let track = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("adjust-dialog", "levels")).with("levels-slider"))).unwrap();
    let at = |level: f32| egui::pos2(track.left() + level / 255.0 * track.width(), track.center().y + 4.0);
    let alt = egui::Modifiers::ALT;
    // Black slider: on while Alt and the button are held, off with either.
    modifiers(&mut h, alt);
    h.event(egui::Event::PointerMoved(at(0.0)));
    press(&mut h, at(0.0), true, alt);
    assert_eq!(clipping(&h), Some(json!("shadows")));
    h.event(egui::Event::PointerMoved(at(60.0)));
    h.run_steps(1);
    let in_black = h.state().ui.dialogs[0].fields["inBlack"].as_f64().unwrap();
    assert!((55.0..=65.0).contains(&in_black), "the drag still moves the slider: {in_black}");
    assert_eq!(clipping(&h), Some(json!("shadows")));
    modifiers(&mut h, egui::Modifiers::NONE);
    assert_eq!(clipping(&h), None, "releasing Alt shows the preview");
    modifiers(&mut h, alt);
    assert_eq!(clipping(&h), Some(json!("shadows")), "Alt again");
    press(&mut h, at(60.0), false, alt);
    assert_eq!(clipping(&h), None, "releasing the mouse shows the preview");
    // White slider.
    h.event(egui::Event::PointerMoved(at(255.0)));
    press(&mut h, at(255.0), true, alt);
    assert_eq!(clipping(&h), Some(json!("highlights")));
    press(&mut h, at(255.0), false, alt);
    assert_eq!(clipping(&h), None);
    // Gamma slider (halfway between 60 and 255 at gamma 1).
    let gamma = 60.0 + (255.0 - 60.0) / 2.0;
    h.event(egui::Event::PointerMoved(at(gamma)));
    press(&mut h, at(gamma), true, alt);
    assert_eq!(clipping(&h), None, "the gamma slider has no display");
    press(&mut h, at(gamma), false, alt);
    // No Alt.
    modifiers(&mut h, egui::Modifiers::NONE);
    h.event(egui::Event::PointerMoved(at(60.0)));
    press(&mut h, at(60.0), true, egui::Modifiers::NONE);
    assert_eq!(clipping(&h), None, "no Alt, no display");
    press(&mut h, at(60.0), false, egui::Modifiers::NONE);
    assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &original));
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id));
}

/// Set through automation, the display stays until cleared; it never re-keys the live preview,
/// touches the document or reaches OK.
#[test]
fn the_display_never_reaches_the_document() {
    let commit = |end: Option<&str>| {
        let mut h = dialog_harness(ramp_app(ColorMode::Rgb, SampleType::U8));
        let original = h.state().session.active().unwrap().doc.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        let id = open_levels(h.state_mut(), json!({"inBlack": 60, "gamma": 1.2}), None);
        h.run_steps(3);
        let _ = crate::adjust_preview::display_doc(h.state_mut(), 0);
        let key = crate::adjust_preview::shown_key(h.state());
        if let Some(end) = end {
            h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(FIELD.into(), json!(end));
            h.run_steps(3);
            assert_eq!(clipping(&h), Some(json!(end)), "an automation-set display stays");
            let _ = crate::adjust_preview::display_doc(h.state_mut(), 0);
            assert_eq!(crate::adjust_preview::shown_key(h.state()), key, "the preview is not re-keyed");
            assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &original));
        }
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1);
        let mut px = Vec::new();
        st.doc.layers[0].surface().unwrap().read_region_into(Rect::new(0, 0, 16, 8), &mut px);
        px
    };
    let plain = commit(None);
    for end in ["shadows", "highlights"] {
        assert_eq!(commit(Some(end)), plain, "{end}: OK commits only the levels");
    }
}

/// Scan once per view, then per slider move thresholds + colours (the texture upload comes on
/// top). `CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo test
/// --release -p photocraft-ui-egui --lib levels_clipping_bench -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn levels_clipping_bench() {
    let adj = levels(lv(40.0, 1.3, 215.0), [lv(5.0, 1.0, 250.0), LevelsChannel::default(), lv(0.0, 0.8, 240.0)]);
    for (w, h) in [(6000u32, 4000u32), (7360, 4912)] {
        let mut s = Surface::new(PixelFormat::RGBA8);
        for y0 in (0..h).step_by(256) {
            let rows = 256.min(h - y0);
            let data: Vec<f32> = (0..rows * w)
                .flat_map(|i| {
                    let (x, y) = (i % w, y0 + i / w);
                    let n = ((x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 7) % 64;
                    let v = (x as f32 / w as f32 * 0.8 + n as f32 / 255.0).min(1.0);
                    [v, 1.0 - v, (y as f32 / h as f32), 1.0]
                })
                .collect();
            s.write_region(Rect::new(0, y0 as i32, w as i32, (y0 + rows) as i32), &data);
        }
        let fit = w.div_ceil(2400);
        for (name, rect, factor) in [("fit", Rect::new(0, 0, w as i32, h as i32), fit), ("100%", Rect::new(1000, 1000, 3560, 2440), 1)] {
            let best = |f: &mut dyn FnMut()| {
                (0..5)
                    .map(|_| {
                        let t = std::time::Instant::now();
                        f();
                        t.elapsed().as_secs_f64() * 1000.0
                    })
                    .fold(f64::MAX, f64::min)
            };
            let scan_ms = best(&mut || {
                std::hint::black_box(scan(&s, None, rect, factor));
            });
            let stats = scan(&s, None, rect, factor);
            let update_ms = best(&mut || {
                let thr = thresholds(&adj, SampleType::U8).unwrap();
                std::hint::black_box(egui::ColorImage::new(stats.size, image(&stats, &thr, ClipEnd::Shadows)));
            });
            println!(
                "{} MP {name} (1/{factor}, {}×{} texels): scan {scan_ms:.1} ms, update {update_ms:.1} ms",
                (w * h) / 1_000_000,
                stats.size[0],
                stats.size[1]
            );
        }
    }
}
