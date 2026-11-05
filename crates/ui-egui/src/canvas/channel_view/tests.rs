use super::*;
use photocraft_engine::Session;
use photocraft_engine::channel_cmds::ChannelTarget;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 4})).unwrap();
    s
}

#[test]
fn composite_bands_keep_the_downsample_grid() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 41, "height": 700})).unwrap();
    s.execute("edit.fill", json!({"color": "#398db2"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let rect = doc.bounds();
    let full = photocraft_compose::render(doc, rect);
    let full_ref = &full;
    for factor in [1, 2, 3, 5] {
        let expected: Vec<_> = (0..rect.height())
            .step_by(factor as usize)
            .flat_map(|y| {
                (0..rect.width()).step_by(factor as usize).map(move |x| {
                    let p = full_ref.get(rect.x0 + x as i32, rect.y0 + y as i32);
                    let a = p[3].clamp(0.0, 1.0);
                    [p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0]
                })
            })
            .collect();
        assert_eq!(composite(doc, rect, factor), expected, "factor {factor}");
    }
}

#[test]
fn channel_eyes_reuse_composite_until_pixels_change() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#4080c0"})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    app.run("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    let id = app.session.active().unwrap().doc.id.0;
    let first = app.channel_views.get_mut(&id).unwrap().source.as_mut().unwrap();
    let original_sample = first.pixels[0];
    // Mark the cache: a recomposite overwrites it, while a freed buffer's address can come back.
    first.pixels[0] = [0.0, 1.0, 0.0, 1.0];

    app.run("channel.setVisible", json!({"channel": "green", "visible": false})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    let reused = app.channel_views.get(&id).unwrap().source.as_ref().unwrap();
    assert_eq!(reused.pixels[0], [0.0, 1.0, 0.0, 1.0], "visibility does not recompose the pixels");

    app.run("edit.fill", json!({"color": "#e02010"})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    // A partial damage upload discards the source; a full redraw may replace it.
    // In either case, cached samples must never refer to the pre-edit pixels.
    if let Some(source) = app.channel_views.get(&id).unwrap().source.as_ref() {
        assert_ne!(source.pixels[0], original_sample, "a full redraw replaces stale pixels");
    }
    app.run("channel.setVisible", json!({"channel": "red", "visible": true})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    let refreshed = app.channel_views.get(&id).unwrap().source.as_ref().unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(refreshed.revision, st.revision);
    assert_ne!(refreshed.pixels[0], original_sample, "new pixels must be recomposited");
    let expected = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    let mut unused = None;
    let actual = render_with_source(
        &st.doc,
        &st.channel_view,
        st.doc.bounds(),
        1,
        Look::default(),
        st.active_layer,
        CompositeSource { cached: Some(&refreshed.pixels), fresh: &mut unused, screen: &Screen::default() },
    )
    .unwrap();
    assert_eq!(actual, expected, "reused pixels must preserve channel colours");
    assert!(unused.is_none(), "reuse must not create a second composite");
}

#[test]
fn plain_composite_draws_nothing() {
    let s = session();
    let st = s.active().unwrap();
    assert!(render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).is_none());
}

#[test]
fn quick_mask_overlay_is_red_over_masked_areas() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::TRANSPARENT, "selected: no overlay");
    assert_eq!(px[6], Color32::from_rgba_premultiplied(128, 0, 0, 128), "masked: 50% red");
}

#[test]
fn quick_mask_without_selection_shows_no_overlay_in_either_mode() {
    // #2743: nothing selected is a colourless mask, whichever way the colour reads.
    for (before, after) in [(None, Some("selected")), (Some("selected"), None), (Some("selected"), Some("masked"))] {
        let mut s = session();
        if let Some(i) = before {
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": i})).unwrap();
        }
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        let st = s.active().unwrap();
        let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
        assert!(px.iter().all(|p| *p == Color32::TRANSPARENT), "{before:?}");
        if let Some(i) = after {
            s.execute("channel.options", json!({"channel": "quickMask", "indicates": i})).unwrap();
            let st = s.active().unwrap();
            let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
            assert!(px.iter().all(|p| *p == Color32::TRANSPARENT), "{before:?} → {i}");
        }
    }
    // Selected Areas with Select All: the whole canvas is coloured.
    let mut s = session();
    s.execute("channel.options", json!({"channel": "quickMask", "indicates": "selected"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert!(px.iter().all(|p| *p == Color32::from_rgba_premultiplied(128, 0, 0, 128)));
}

#[test]
fn layer_mask_views_and_damage_on_the_downsampled_grid() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 37, "height": 21})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [37, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    let full = render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap();
    assert!(full[0].r() < 10 && full[36].r() > 245 && full[0].a() == 255, "opaque grayscale ramp");
    // A sub-rect starting on the factor grid matches the same pixels of the full render.
    for f in [1u32, 2, 3] {
        let all = render(&st.doc, &st.channel_view, b, f, Look::default(), st.active_layer).unwrap();
        let w = b.width().div_ceil(f) as usize;
        let r = Rect::new(3 * f as i32, 2 * f as i32, 30, 19);
        let part = render(&st.doc, &st.channel_view, r, f, Look::default(), st.active_layer).unwrap();
        let pw = r.width().div_ceil(f) as usize;
        for (i, p) in part.iter().enumerate() {
            assert_eq!(*p, all[(i / pw + 2) * w + i % pw + 3], "factor {f}, pixel {i}");
        }
    }
    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap();
    assert!(px[0].r() >= 124 && px[0].g() == 0 && px[0].a() >= 124, "about 50% red over the hidden end: {:?}", px[0]);
    assert!(px[36].a() <= 4, "next to nothing over the revealed end: {:?}", px[36]);
}

#[test]
fn layer_mask_views_show_the_feathered_mask() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 37, "height": 21})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [37, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    s.execute("layer.layerMask.edit", json!({"feather": 4})).unwrap();
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();

    let st = s.active().unwrap();
    let bounds = st.doc.bounds();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let raw = layer.mask.as_ref().unwrap().surface.sample_channel(0, 0, 0);
    let effective = photocraft_compose::masks::combined_mask(layer, bounds).unwrap();
    let softened = effective.sample_channel(0, 0, 0);
    assert!((softened - raw).abs() > 0.01, "the fixture has a visible feather at its edge");

    let gray = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(gray[0], Color32::from_gray(q(softened)), "gray view uses the compositor's feathered mask");

    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let st = s.active().unwrap();
    let overlay = render(&st.doc, &st.channel_view, bounds, 1, Look::default(), st.active_layer).unwrap();
    let alpha = 0.5 * (1.0 - softened);
    let red = AlphaChannel::DEFAULT_COLOR.to_rgb();
    assert_eq!(overlay[0], Color32::from_rgba_premultiplied(q(red[0] * alpha), q(red[1] * alpha), q(red[2] * alpha), q(alpha)));
}

/// Layer Mask Display Options: the rubylith takes their colour and opacity, alone over the
/// composite and with a colour channel hidden.
#[test]
fn layer_mask_overlay_takes_its_display_options() {
    let mut s = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("view.layerMask", json!({"mode": "overlay"})).unwrap();
    let green = Look { mask_rgb: [0.0, 1.0, 0.0], mask_opacity: 0.25, ..Look::default() };
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap()[0], Color32::from_rgba_premultiplied(128, 0, 0, 128));
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, green, st.active_layer).unwrap()[0], Color32::from_rgba_premultiplied(0, 64, 0, 64));
    // Red hidden: the white picture shows cyan, then green at 25 % over it.
    s.execute("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, green, st.active_layer).unwrap()[0], Color32::from_rgb(0, 255, 191));
}

/// `cargo test --release -p photocraft-ui-egui --lib mask_view_switch_24mp -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn mask_view_switch_24mp() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 4000})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#d0402b"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [6000, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    let factor = 6000u32.div_ceil(MAX_SIDE);
    for mode in ["gray", "overlay"] {
        s.execute("view.layerMask", json!({"mode": mode})).unwrap();
        let st = s.active().unwrap();
        let mut best = f64::MAX;
        for _ in 0..8 {
            let t = std::time::Instant::now();
            let px = render(&st.doc, &st.channel_view, st.doc.bounds(), factor, Look::default(), st.active_layer).unwrap();
            let img = egui::ColorImage::new([3000, 2000], px);
            std::hint::black_box(img);
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("24 MP mask view {mode}: {best:.1} ms (render at 1/{factor})");
    }
}

/// Interface ▸ Show Channels in Color (#2385): a solo RGB / CMYK channel tints the canvas
/// instead of staying gray, like the panel thumbnails wired in #1986.
#[test]
fn show_channels_in_color_tints_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    let st = s.active().unwrap();
    let b = st.doc.bounds();
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look::default(), st.active_layer).unwrap()[0], Color32::from_gray(128));
    assert_eq!(render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0], Color32::from_rgb(0, 128, 0));

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "cmyk"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    s.execute("channel.target", json!({"channel": "cyan"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    assert!(px.r() < 100 && px.g() > 150 && px.b() > 150, "cyan ink, not gray: {px:?}");
    let gray = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap()[0];
    assert_eq!(gray.r(), gray.g(), "off stays grayscale");
}

/// Lab: a and b tint over a neutral Lightness; Lightness itself stays gray (Adobe's exception).
#[test]
fn show_channels_in_color_lab_lightness_stays_gray() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "lab"})).unwrap();
    s.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    let b = s.active().unwrap().doc.bounds();
    s.execute("channel.target", json!({"channel": {"color": 0}})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    let spread = px.r().max(px.g().max(px.b())) as i32 - px.r().min(px.g().min(px.b())) as i32;
    assert!(spread <= 3, "Lightness stays gray: {px:?}");
    s.execute("channel.target", json!({"channel": {"color": 1}})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, b, 1, Look { in_color: true, ..Look::default() }, st.active_layer).unwrap()[0];
    assert!(px.r() != px.g() || px.g() != px.b(), "a channel is tinted, not gray: {px:?}");
}

/// The canvas texture cache is keyed by the preference, so toggling it rebuilds the view.
#[test]
fn ensure_honours_show_channels_in_color() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    app.run("channel.target", json!({"channel": "green"})).unwrap();
    let ctx = egui::Context::default();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    let key_off = app.channel_views.values().next().unwrap().key;
    app.session.execute("prefs.set", json!({"values": {"interface.showChannelsInColor": true}})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    let key_on = app.channel_views.values().next().unwrap().key;
    assert_ne!(key_off, key_on, "the preference changes the cached view key");
    // So do the Layer Mask Display Options.
    app.session.execute("channel.options", json!({"channel": "mask", "color": "#00ff00", "opacity": 70})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    assert_ne!(app.channel_views.values().next().unwrap().key, key_on, "the overlay options change the cached view key");
}

#[test]
fn solo_alpha_is_grayscale_and_colour_channels_split() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ff8000"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 4})).unwrap();
    s.execute("channel.new", json!({"fill": "selection"})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.channel_view.target, ChannelTarget::Alpha(0));
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!((px[0], px[5]), (Color32::WHITE, Color32::BLACK));
    // Green alone: grayscale of the green channel; with blue hidden only.
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::from_gray(128));
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "red", "visible": false})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 2, Look::default(), st.active_layer).unwrap();
    assert_eq!(px.len(), 4 * 2);
    assert_eq!(px[0], Color32::from_rgb(0, 128, 0));
}

#[test]
fn rgb_off_shows_alpha_quick_mask_or_the_layer_mask() {
    let mut s = session();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("channel.new", json!({"fill": "white"})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": true})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = s.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::WHITE, "Alpha alone is the channel, not the red image");

    let mut q = session();
    q.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    q.execute("select.editInQuickMaskMode", json!({})).unwrap();
    q.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = q.active().unwrap();
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::WHITE, "Quick Mask alone is grayscale, not the red composite");

    let mut m = session();
    m.execute("layer.new.layer", json!({})).unwrap();
    m.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    m.execute("layer.layerMask.hideAll", json!({})).unwrap();
    m.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    let st = m.active().unwrap();
    assert_eq!(st.channel_view.visible_colors(3), 0);
    let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).unwrap();
    assert_eq!(px[0], Color32::BLACK, "the hidden mask reads black, not the red layer");
}

/// A white-backed `mode`/`depth` document, 3×1, with a 0.8/0.4/0.2 picture and two overlapping
/// spot inks (`a` covers 1, ½, 0 of the pixels; `b` the reverse).
fn spot_doc(mode: ColorMode, depth: photocraft_color::SampleType, sa: f32, sb: f32) -> Document {
    use photocraft_color::{Color, PixelFormat};
    let mut doc = Document::with_background("spots", photocraft_geom::Size::new(3, 1), mode, depth, Color::rgb(0.8, 0.4, 0.2));
    let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    for (name, rgb, s, vals) in [("a", [0.0, 0.6, 0.65], sa, [1.0, 0.5, 0.0]), ("b", [0.9, 0.2, 0.5], sb, [0.0, 0.5, 1.0])] {
        let mut sf = Surface::new(fmt);
        sf.write_region(Rect::new(0, 0, 3, 1), &vals);
        doc.channels.push(AlphaChannel { spot: Some((Color::rgb(rgb[0], rgb[1], rgb[2]), s)), ..AlphaChannel::new(name, sf) });
    }
    doc
}

/// Visible spot channels of RGB and CMYK documents print over the composite as inks (#10249),
/// the same as the compositor's overprint, in panel order; not as a tint.
#[test]
fn spot_eyes_overprint_the_composite() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        for depth in [photocraft_color::SampleType::U8, photocraft_color::SampleType::U16] {
            for (sa, sb) in [(0.0, 0.0), (0.5, 1.0), (1.0, 0.5)] {
                for swapped in [false, true] {
                    let mut doc = spot_doc(mode, depth, sa, sb);
                    if swapped {
                        doc.channels.reverse();
                    }
                    let mut v = ChannelView::default();
                    v.set_alpha_visible(0, true);
                    v.set_alpha_visible(1, true);
                    let px = render(&doc, &v, doc.bounds(), 1, Look::default(), None).unwrap();
                    let mut want = photocraft_compose::Buffer { rect: doc.bounds(), px: composite(&doc, doc.bounds(), 1) };
                    photocraft_compose::multichannel::overprint(&mut want, &doc, |_| true);
                    for (x, (got, w)) in px.iter().zip(&want.px).enumerate() {
                        assert_eq!(got.a(), 255, "{mode:?} {depth:?}: opaque");
                        let w = [q(w[0]), q(w[1]), q(w[2])];
                        assert_eq!([got.r(), got.g(), got.b()], w, "{mode:?} {depth:?} s {sa}/{sb} swapped {swapped} x {x}");
                    }
                    // Only the visible ink prints; where it has none the canvas shows through.
                    v.set_alpha_visible(1, false);
                    let one = render(&doc, &v, doc.bounds(), 1, Look::default(), None).unwrap();
                    let mut want = photocraft_compose::Buffer { rect: doc.bounds(), px: composite(&doc, doc.bounds(), 1) };
                    photocraft_compose::multichannel::overprint(&mut want, &doc, |i| i == 0);
                    let (solid, bare) = if swapped { (2, 0) } else { (0, 2) };
                    let w = want.px[solid];
                    assert_eq!([one[solid].r(), one[solid].g(), one[solid].b()], [q(w[0]), q(w[1]), q(w[2])]);
                    assert_eq!(one[bare], Color32::TRANSPARENT);
                }
            }
        }
    }
}

/// Only inked pixels replace the canvas: where every visible spot is 0 the overlay stays
/// transparent, so the normal (colour-managed, full-resolution) canvas shows there, and an empty
/// spot channel draws nothing and renders no composite at all.
#[test]
fn uninked_pixels_leave_the_canvas_alone() {
    for factor in [1, 2] {
        let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 1.0, 1.0);
        doc.size = photocraft_geom::Size::new(4, 2);
        let fmt = doc.channels[0].surface.format();
        let mut sparse = Surface::new(fmt);
        sparse.write_pixel(0, 0, &[1.0]);
        doc.channels[0].surface = sparse;
        doc.channels[1].surface = Surface::new(fmt);
        let mut v = ChannelView::default();
        v.set_alpha_visible(0, true);
        v.set_alpha_visible(1, true);
        let px = render(&doc, &v, doc.bounds(), factor, Look::default(), None).unwrap();
        assert_eq!(px[0], Color32::from_rgb(0, q(0.6), q(0.65)), "/{factor}: solid teal where the ink is");
        assert!(px[1..].iter().all(|p| *p == Color32::TRANSPARENT), "/{factor}: {px:?}");
        // Only the empty spot shown: nothing to print, and no composite rendered for it.
        v.set_alpha_visible(0, false);
        let mut fresh = None;
        let px = render_with_source(
            &doc,
            &v,
            doc.bounds(),
            factor,
            Look::default(),
            None,
            CompositeSource { cached: None, fresh: &mut fresh, screen: &Screen::default() },
        );
        assert!(px.is_none(), "/{factor}: an empty spot draws no texture");
        assert!(fresh.is_none(), "/{factor}: an empty spot needs no composite");
    }
}

/// A few inked pixels composite only their bounding box, not the whole document, and print the
/// same as over the whole composite; ink everywhere renders the whole one, kept for the cache.
#[test]
fn sparse_inks_composite_only_their_bounding_box() {
    use photocraft_doc::{Layer, LayerContent};
    for factor in [1, 2, 3] {
        let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 0.5, 1.0);
        doc.size = photocraft_geom::Size::new(60, 40);
        let mut ramp = Surface::new(doc.pixel_format());
        for y in 0..40 {
            for x in 0..60 {
                ramp.write_pixel(x, y, &[x as f32 / 59.0, y as f32 / 39.0, 0.5, 1.0]);
            }
        }
        doc.layers.push(Layer::new("ramp", LayerContent::Raster(ramp)));
        let fmt = doc.channels[0].surface.format();
        let mut sparse = Surface::new(fmt);
        sparse.write_pixel(12, 0, &[1.0]);
        sparse.write_pixel(18, 6, &[0.5]);
        doc.channels[0].surface = sparse;
        doc.channels[1].surface = Surface::new(fmt);
        let mut v = ChannelView::default();
        v.set_alpha_visible(0, true);
        v.set_alpha_visible(1, true);
        let b = doc.bounds();
        let mut fresh = None;
        let source = CompositeSource { cached: None, fresh: &mut fresh, screen: &Screen::default() };
        let got = render_with_source(&doc, &v, b, factor, Look::default(), None, source).unwrap();
        assert!(fresh.is_none(), "/{factor}: two inked pixels render no whole composite");
        let whole = composite_sampled(&doc, b, factor);
        let ink = photocraft_compose::multichannel::Ink::of(&doc, &doc.channels[0]);
        let w = 60usize.div_ceil(factor as usize);
        let f = factor as usize;
        let inked = [(12 / f, 0), (18 / f, 6 / f)];
        for (k, px) in got.iter().enumerate() {
            let Some(&(x, y)) = inked.iter().find(|&&(x, y)| y * w + x == k) else {
                assert_eq!(*px, Color32::TRANSPARENT, "/{factor} px {k}");
                continue;
            };
            let mut want = [whole[k]];
            ink.apply(&mut want, [doc.channels[0].surface.pixel((x * f) as i32, (y * f) as i32)[0]]);
            let want = Color32::from_rgb(q(want[0][0]), q(want[0][1]), q(want[0][2]));
            assert_eq!(*px, want, "/{factor} ({x}, {y})");
        }
        // Ink everywhere: the bounding box is the whole document, rendered once for the cache.
        let mut solid = Surface::new(fmt);
        solid.fill_rect(b, &[1.0]);
        doc.channels[1].surface = solid;
        let mut fresh = None;
        let source = CompositeSource { cached: None, fresh: &mut fresh, screen: &Screen::default() };
        assert!(render_with_source(&doc, &v, b, factor, Look::default(), None, source).is_some());
        assert!(fresh.is_some(), "/{factor}: ink everywhere renders the whole composite");
    }
}

/// A visible spot channel without ink draws no texture over the canvas, and remembers that; a
/// spot with ink shown later draws one.
#[test]
fn uninked_spot_draws_no_texture() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 8, "background": "white"})).unwrap();
    // No selection: an empty spot channel, shown over the composite.
    app.run("channel.newSpot", json!({"color": "#00ffff", "solidity": 0})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    let id = app.session.active().unwrap().doc.id.0;
    let st = app.session.active().unwrap();
    assert!(prints_inks(&st.doc, &st.channel_view), "the composite and the spot are both shown");
    assert!(ensure(&mut app, &ctx, 0, None).is_none(), "no ink: nothing drawn");
    assert!(app.channel_views.get(&id).is_some_and(|c| c.tex.is_none() && c.valid), "and remembered");
    assert!(ensure(&mut app, &ctx, 0, None).is_none());
    // A second spot made from a selection has ink: the texture shows it.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    app.run("channel.newSpot", json!({"color": "#ff00ff", "solidity": 0})).unwrap();
    assert!(ensure(&mut app, &ctx, 0, None).is_some(), "ink: drawn");
    assert!(app.channel_views.get(&id).is_some_and(|c| c.tex.is_some()));
}

/// Printed inks go through the canvas display transform (monitor profile, Proof Colors), as the
/// canvas around them does.
#[test]
fn printed_inks_follow_the_display_transform() {
    use photocraft_cms::{Builtin, TransformOptions};
    let doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 1.0, 1.0);
    let mut v = ChannelView::default();
    v.set_alpha_visible(0, true);
    let plain = render(&doc, &v, doc.bounds(), 1, Look::default(), None).unwrap();
    let t = photocraft_cms::transform::cached(Builtin::Srgb.profile(), Builtin::DisplayP3.profile(), TransformOptions::default()).unwrap();
    let screen = Screen { encode_srgb: false, transform: Some(t.clone()) };
    let shown =
        render_with_source(&doc, &v, doc.bounds(), 1, Look::default(), None, CompositeSource { cached: None, fresh: &mut None, screen: &screen }).unwrap();
    // x 0: solid teal; x 2: no ink of channel 0, so transparent either way.
    let mut teal = [0.0f32, 0.6, 0.65, 1.0];
    t.apply(&mut teal, 4);
    assert_ne!(plain[0], shown[0]);
    assert_eq!(shown[0], Color32::from_rgb(q(teal[0]), q(teal[1]), q(teal[2])));
    assert_eq!((plain[2], shown[2]), (Color32::TRANSPARENT, Color32::TRANSPARENT));
}

/// A linear (32-bit) document's composite is sRGB-encoded on its way to the screen. Its inks are
/// sRGB colours converted into the linear profile, so a solid ink shows its stored colour, not a
/// brightened one.
#[test]
fn printed_inks_keep_their_colour_in_a_linear_document() {
    let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::F32, 1.0, 1.0);
    doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let mut v = ChannelView::default();
    v.set_alpha_visible(0, true);
    let encoded = Screen { encode_srgb: true, transform: None };
    let shown =
        render_with_source(&doc, &v, doc.bounds(), 1, Look::default(), None, CompositeSource { cached: None, fresh: &mut None, screen: &encoded }).unwrap();
    let (got, want) = ([shown[0].r(), shown[0].g(), shown[0].b()], [0, q(0.6), q(0.65)]);
    assert!(got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1), "{got:?} vs the stored {want:?}");
}

/// Alpha channels and Quick Mask still tint, over the printed inks.
#[test]
fn alpha_tint_draws_over_the_inks() {
    let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 1.0, 1.0);
    let mut sel = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, photocraft_color::SampleType::U8, false));
    sel.write_region(Rect::new(0, 0, 3, 1), &[0.0, 0.0, 0.0]);
    doc.channels.push(AlphaChannel::new("Alpha 1", sel));
    let mut v = ChannelView::default();
    v.set_alpha_visible(0, true);
    v.set_alpha_visible(2, true);
    let px = render(&doc, &v, doc.bounds(), 1, Look::default(), None).unwrap();
    // Solid opaque teal at x 0, then the default red rubylith at 50 % over it.
    let teal = [0.0f32, 0.6, 0.65];
    let red = AlphaChannel::DEFAULT_COLOR.to_rgb();
    let want: Vec<u8> = (0..3).map(|j| q(red[j] * 0.5 + teal[j] * 0.5)).collect();
    assert_eq!(vec![px[0].r(), px[0].g(), px[0].b()], want);
}

/// Editing a spot (colour, solidity, visibility) or the picture under it refreshes the canvas texture.
#[test]
fn spot_edits_refresh_the_overlay() {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 8, "background": "white"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let spot = app.run("channel.newSpot", json!({"color": "#00ffff", "solidity": 0})).unwrap()["channel"].as_u64().unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    let id = app.session.active().unwrap().doc.id.0;
    let pixel = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).map(|px| px[0])
    };
    assert!(ensure(&mut app, &ctx, 0, None).is_some());
    assert_eq!(pixel(&app), Some(Color32::from_rgb(0, 255, 255)), "solid transparent cyan over white");
    let edits = [
        ("channel.options", json!({"channel": spot, "color": "#ff00ff"})),
        // A gray picture first: a solid ink over white looks the same at any solidity.
        ("edit.fill", json!({"color": "#808080"})),
        ("channel.options", json!({"channel": spot, "solidity": 100})),
        ("channel.setVisible", json!({"channel": spot, "visible": false})),
    ];
    let mut last = pixel(&app);
    for (cmd, params) in edits {
        app.run(cmd, params.clone()).unwrap();
        let shown = ensure(&mut app, &ctx, 0, None);
        let now = pixel(&app);
        assert_ne!(now, last, "{cmd} {params}");
        let revision = app.session.active().unwrap().revision;
        match shown {
            Some(_) => assert_eq!(app.channel_views.get(&id).unwrap().revision, revision, "{cmd} {params}"),
            None => assert!(now.is_none(), "a hidden spot leaves the plain composite"),
        }
        last = now;
    }
}

/// The first draw of a big document composites a proxy, not the full canvas (#10249): it must
/// pick the same pixels the full-resolution composite gives, in RGB and CMYK at 8 and 16 bit,
/// with the inks printed over it the same way.
#[test]
fn proxy_composite_matches_the_full_resolution_one() {
    use photocraft_color::{BlendMode, SampleType};
    use photocraft_doc::{Layer, LayerContent, LayerMask};
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16] {
            for factor in [2, 3, 4] {
                let mut doc = spot_doc(mode, depth, 0.5, 1.0);
                doc.size = photocraft_geom::Size::new(37, 23);
                let mut s = Surface::new(doc.pixel_format());
                for y in 0..23 {
                    for x in 0..37 {
                        let (fx, fy) = (x as f32 / 36.0, y as f32 / 22.0);
                        s.write_pixel(x, y, &[fx, fy, (fx * fy * 7.0).fract(), 0.3 + 0.7 * fy]);
                    }
                }
                let mut layer = Layer::new("pattern", LayerContent::Raster(s));
                layer.blend = BlendMode::Multiply;
                layer.opacity = 0.8;
                let mut mask = LayerMask::reveal_all();
                mask.surface.fill_rect(Rect::new(5, 3, 20, 15), &[0.25]);
                layer.mask = Some(mask);
                doc.layers.push(layer);
                for ch in &mut doc.channels {
                    let mut sf = Surface::new(ch.surface.format());
                    for y in 0..23 {
                        for x in 0..37 {
                            sf.write_pixel(x, y, &[((x * 3 + y * 5) % 11) as f32 / 10.0]);
                        }
                    }
                    ch.surface = sf;
                }
                assert!(proxy_matches(&doc));
                let b = doc.bounds();
                let (proxy, full) = (composite(&doc, b, factor), composite_sampled(&doc, b, factor));
                assert_eq!(proxy.len(), full.len());
                for (i, (p, f)) in proxy.iter().zip(&full).enumerate() {
                    assert!(p.iter().zip(f).all(|(p, f)| (p - f).abs() < 1e-4), "{mode:?} {depth:?} /{factor} px {i}: {p:?} != {f:?}");
                }
                let mut v = ChannelView::default();
                v.set_alpha_visible(0, true);
                v.set_alpha_visible(1, true);
                let got = render(&doc, &v, b, factor, Look::default(), None).unwrap();
                let mut want = photocraft_compose::Buffer {
                    rect: Rect::new(0, 0, 37usize.div_ceil(factor as usize) as i32, 23usize.div_ceil(factor as usize) as i32),
                    px: full,
                };
                let mut small = doc.clone();
                small.channels.iter_mut().for_each(|c| c.surface = photocraft_compose::proxy::downsample(&c.surface, factor));
                photocraft_compose::multichannel::overprint(&mut want, &small, |_| true);
                let sw = want.rect.width();
                let inked = |i: usize| {
                    let (x, y) = ((i as u32 % sw) as i32, (i as u32 / sw) as i32);
                    small.channels.iter().any(|c| c.surface.pixel(x, y).first().is_some_and(|v| *v > 0.0))
                };
                for (i, (g, w)) in got.iter().zip(&want.px).enumerate() {
                    let w = if inked(i) { Color32::from_rgba_premultiplied(q(w[0]), q(w[1]), q(w[2]), 255) } else { Color32::TRANSPARENT };
                    assert!(g.to_array().iter().zip(w.to_array()).all(|(a, b)| a.abs_diff(b) <= 1), "{mode:?} {depth:?} /{factor} px {i}: {g:?} != {w:?}");
                }
            }
        }
    }
}

/// Feathered masks blur by document pixels, which a proxy would not scale: those documents keep
/// the full-resolution composite.
#[test]
fn feathered_masks_keep_the_full_resolution_composite() {
    use photocraft_doc::{Layer, LayerContent, LayerMask};
    let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 1.0, 1.0);
    assert!(proxy_matches(&doc));
    let mut mask = LayerMask::reveal_all();
    mask.feather = 4.0;
    let mut inner = Layer::new("inner", LayerContent::Raster(Surface::new(doc.pixel_format())));
    inner.mask = Some(mask);
    doc.layers.push(Layer::group("group", vec![inner]));
    assert!(!proxy_matches(&doc));
    // Hidden layers are not composited: a hidden group (or a hidden layer in it) doesn't count.
    let last = doc.layers.len() - 1;
    doc.layers[last].visible = false;
    assert!(proxy_matches(&doc));
    doc.layers[last].visible = true;
    if let LayerContent::Group(g) = &mut doc.layers[last].content {
        g.children[0].visible = false;
    }
    assert!(proxy_matches(&doc));
}

/// A pattern fill's tiles keep their size on a proxy, so it would sample other pattern pixels:
/// such documents keep the full-resolution composite unless the fill's own cache is shrunk with
/// the proxy.
#[test]
fn uncached_pattern_fills_keep_the_full_resolution_composite() {
    use photocraft_doc::{Fill, FillCache, Layer, LayerContent};
    let pattern = Fill::Pattern { name: "p".into(), scale: 1.0, id: "p".into(), angle: 0.0, link: true, phase: (0.0, 0.0) };
    let mut doc = spot_doc(ColorMode::Rgb, photocraft_color::SampleType::U8, 1.0, 1.0);
    doc.layers.push(Layer::group("group", vec![Layer::new("pattern", LayerContent::Fill(pattern.clone()))]));
    assert!(!proxy_matches(&doc));
    let mut stale = Layer::new("pattern", LayerContent::Fill(pattern.clone()));
    stale.fill_cache = Some(FillCache { fill: Fill::Solid(photocraft_color::Color::WHITE), surface: Surface::new(doc.pixel_format()) });
    doc.layers.pop();
    doc.layers.push(stale);
    assert!(!proxy_matches(&doc));
    let mut cached = Layer::new("pattern", LayerContent::Fill(pattern.clone()));
    cached.fill_cache = Some(FillCache { fill: pattern, surface: Surface::new(doc.pixel_format()) });
    doc.layers.pop();
    doc.layers.push(cached);
    assert!(proxy_matches(&doc));
}

/// Solo spot channel view is grayscale and updates with edits on an RGB or CMYK document with
/// no layer mask.
#[test]
fn solo_spot_is_grayscale_and_edits_without_layer_mask() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        let mode_str = if mode == ColorMode::Rgb { "rgb" } else { "cmyk" };
        app.run("file.new", json!({"width": 8, "height": 4, "mode": mode_str})).unwrap();
        // Plain document has a background layer and no layer mask.
        assert!(app.session.active().unwrap().doc.layers.iter().all(|l| l.mask.is_none()));
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        let spot = app.run("channel.newSpot", json!({"color": "#ff0000", "solidity": 0})).unwrap()["channel"].as_u64().unwrap();
        app.sync_views();
        // Target the spot channel alone (Channels panel row click).
        app.run("channel.target", json!({"channel": spot})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.channel_view.visible_colors(color_count(&st.doc)), 0);
        let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer)
            .expect("solo spot channel must render without a layer mask");
        // Selected half (x 0..4) has ink (1.0 -> white in grayscale tone), unselected half is black (0.0).
        assert_eq!(px[0], Color32::WHITE, "{mode:?}: inked half is white tone");
        assert_eq!(px[7], Color32::BLACK, "{mode:?}: uninked half is black tone");

        // Editing the targeted spot channel updates the solo preview.
        app.run("select.rect", json!({"x": 4, "y": 0, "width": 4, "height": 4})).unwrap();
        app.run("edit.fill", json!({"color": "#ffffff"})).unwrap();
        let st = app.session.active().unwrap();
        let px2 = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).expect("edited spot channel must render");
        assert_eq!(px2[7], Color32::WHITE, "{mode:?}: newly filled area is now white");
    }
}

#[test]
fn solo_spot_wins_over_an_unrelated_layer_mask() {
    for mode in ["rgb", "cmyk"] {
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 4, "mode": mode})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        let spot = app.run("channel.newSpot", json!({"color": "#ff0000", "solidity": 0})).unwrap()["channel"].as_u64().unwrap();
        app.sync_views();
        app.run("channel.target", json!({"channel": spot})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.channel_view.visible_colors(color_count(&st.doc)), 0);
        let px = render(&st.doc, &st.channel_view, st.doc.bounds(), 1, Look::default(), st.active_layer).expect("solo spot renders");
        // A revealed layer mask is all white; the spot has ink on the left half only.
        assert_eq!(px[0], Color32::WHITE, "{mode}: inked half");
        assert_eq!(px[7], Color32::BLACK, "{mode}: uninked half shows the spot, not the mask");
    }
}
