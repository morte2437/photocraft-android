use super::*;
use photocraft_geom::Rect;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

/// A visible wall on the Background with an empty, active layer above it.
fn bucket_layers(depth: u32) -> (Session, photocraft_doc::LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
    let bottom = s.active().unwrap().active_layer.unwrap();
    s.edit("wall", |doc, _| {
        doc.layer_mut(bottom).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    (s, bottom)
}

/// The options bar's Mode reaches the fill: Multiply over grey, Clear erases, a bad name or
/// Pass Through is an error and changes nothing.
#[test]
fn bucket_mode_multiplies_clears_and_rejects_bad_names() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let base = json!({"x": 1, "y": 1, "antiAlias": false, "tolerance": 0});
        let with = |extra: Value| {
            let mut p = base.clone();
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            p
        };
        s.execute("paint.bucket", with(json!({"color": "#808080"}))).unwrap();
        s.execute("paint.bucket", with(json!({"color": "#ff0000", "mode": "multiply"}))).unwrap();
        let p = px(&s, 3, 3);
        assert!((p[0] - 0.5).abs() < 0.01 && p[1] < 0.01 && p[3] > 0.99, "depth {depth}: {p:?}");
        for bad in [json!("passThrough"), json!("sparkle"), json!(3)] {
            assert!(s.execute("paint.bucket", with(json!({"mode": bad}))).is_err(), "{bad}");
        }
        assert_eq!(px(&s, 3, 3), p, "a refused mode paints nothing");
        s.execute("paint.bucket", with(json!({"mode": "Clear", "opacity": 50}))).unwrap();
        assert!((px(&s, 3, 3)[3] - 0.5).abs() < 0.01, "depth {depth}: half cleared");
    }
}

#[test]
fn bucket_sample_all_layers_uses_visible_boundaries_at_all_depths() {
    for depth in [8, 16, 32] {
        for all in [None, Some(false), Some(true)] {
            let (mut s, bottom) = bucket_layers(depth);
            let before = s.active().unwrap().doc.layer(bottom).unwrap().clone();
            let mut p = json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0});
            if let Some(all) = all {
                p["sampleAllLayers"] = json!(all);
            }
            assert_eq!(s.execute("paint.bucket", p.clone()).unwrap()["filled"], true);
            assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}, all {all:?}");
            let beyond = if all == Some(true) { vec![0.0; 4] } else { vec![1.0, 0.0, 0.0, 1.0] };
            assert_eq!(px(&s, 15, 5), beyond, "depth {depth}, all {all:?}: sample flag controls the boundary");
            assert_eq!(s.active().unwrap().doc.layer(bottom).unwrap(), &before, "only the active layer is written");
            if all == Some(true) {
                assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall itself is untouched");
                assert!(s.undo());
                p["contiguous"] = json!(false);
                s.execute("paint.bucket", p).unwrap();
                assert_eq!(px(&s, 15, 5), vec![1.0, 0.0, 0.0, 1.0], "noncontiguous finds both white regions");
                assert_eq!(px(&s, 10, 5), vec![0.0; 4]);
                assert!(s.undo());
                assert_eq!(s.execute("paint.bucket", json!({"x": -1, "y": 2, "sampleAllLayers": true})).unwrap()["filled"], false);
                assert_eq!(px(&s, 5, 5), vec![0.0; 4], "an off-canvas sample does not fill the layer");
            }
        }
    }
}

#[test]
fn bucket_sample_all_layers_ignores_hidden_layers() {
    let (mut s, _) = bucket_layers(8);
    s.edit("hidden wall", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(4, 0, 5, 10), &[0.0, 0.0, 0.0, 1.0]);
        l.visible = false;
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true})).unwrap();
    assert_eq!(px(&s, 4, 5), vec![1.0, 0.0, 0.0, 1.0], "hidden wall does not bound the fill");
    assert_eq!(px(&s, 8, 5), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 15, 5), vec![0.0; 4], "visible wall still bounds the fill");
}

#[test]
fn bucket_sample_all_layers_keeps_selection_opacity_and_history() {
    for depth in [8, 16, 32] {
        let (mut s, _) = bucket_layers(depth);
        s.execute("select.rect", json!({"x": 0, "y": 2, "width": 20, "height": 6})).unwrap();
        s.edit("feather", |doc, _| {
            doc.selection.as_mut().unwrap().fill_rect(Rect::new(0, 2, 4, 8), &[0.5]);
            Ok(())
        })
        .unwrap();
        let selection = s.active().unwrap().doc.selection.clone();
        s.execute("paint.bucket", json!({"x": 2, "y": 5, "color": "#ff0000", "antiAlias": false, "tolerance": 0, "opacity": 50, "sampleAllLayers": true}))
            .unwrap();
        let (feathered, full) = (px(&s, 2, 5), px(&s, 6, 5));
        assert!((feathered[3] - 0.25).abs() < 0.005 && (full[3] - 0.5).abs() < 0.005, "depth {depth}: selection coverage multiplies opacity");
        assert_eq!(full[..3], [1.0, 0.0, 0.0]);
        assert_eq!(px(&s, 6, 1), vec![0.0; 4], "outside selection");
        assert_eq!(px(&s, 15, 5), vec![0.0; 4], "outside sampled region");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Paint Bucket"));
        assert!(s.undo());
        assert_eq!(px(&s, 2, 5), vec![0.0; 4]);
        assert_eq!(s.active().unwrap().doc.selection, selection);
        assert!(s.redo());
        assert_eq!(px(&s, 2, 5), feathered);
        assert_eq!(px(&s, 6, 5), full);
        assert_eq!(s.active().unwrap().doc.selection, selection);
    }
}

#[test]
fn bucket_sample_all_layers_pattern_uses_the_same_region() {
    let (mut s, _) = bucket_layers(8);
    s.execute(
        "paint.bucket",
        json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0, "sampleAllLayers": true}),
    )
    .unwrap();
    let left: Vec<f32> = (0..10).map(|x| px(&s, x, 0)[0]).collect();
    assert!(left.iter().any(|&r| r < 0.3) && left.iter().any(|&r| r > 0.9), "pattern varies inside the region: {left:?}");
    assert_eq!(px(&s, 10, 5), vec![0.0; 4], "wall untouched");
    assert_eq!(px(&s, 15, 5), vec![0.0; 4], "region past the wall untouched");
}

#[test]
fn bucket_sample_all_layers_mask_and_channel_targets_keep_their_own_boundaries() {
    for target in [json!("mask"), json!({"channel": 0}), json!("quickMask")] {
        for all in [false, true] {
            let (mut s, _) = bucket_layers(8);
            match crate::channel_cmds::target_of(&json!({"target": target})) {
                crate::channel_cmds::Target::Mask => {
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                }
                crate::channel_cmds::Target::Alpha(_) => {
                    s.execute("channel.new", json!({"fill": "white"})).unwrap();
                }
                crate::channel_cmds::Target::QuickMask => {
                    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                }
                _ => panic!("expected a grayscale target"),
            }
            let p = json!({"x": 2, "y": 2, "color": "#000000", "antiAlias": false, "tolerance": 0, "target": target, "sampleAllLayers": all});
            s.edit("target wall", |doc, active| {
                let area = doc.bounds();
                let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &p).unwrap();
                surf.fill_rect(area, &[1.0]);
                surf.fill_rect(Rect::new(5, 0, 6, 10), &[0.0]);
                Ok(())
            })
            .unwrap();
            s.execute("paint.bucket", p.clone()).unwrap();
            let d = &s.active().unwrap().doc;
            let surf = match crate::channel_cmds::target_of(&p) {
                crate::channel_cmds::Target::Mask => &d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface,
                crate::channel_cmds::Target::Alpha(i) => &d.channels[i].surface,
                crate::channel_cmds::Target::QuickMask => &d.quick_mask.as_ref().unwrap().surface,
                _ => panic!("expected a grayscale target"),
            };
            assert_eq!(surf.sample_channel(2, 5, 0), 0.0, "{target}, all {all}: filled target");
            assert_eq!(surf.sample_channel(7, 5, 0), 1.0, "{target}, all {all}: stopped at target wall");
            assert_eq!(px(&s, 2, 5), vec![0.0; 4], "layer pixels remain untouched");
        }
    }
}

#[test]
fn bucket_fill_contiguous() {
    let mut s = session();
    s.edit("wall", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#ff0000", "antiAlias": false})).unwrap();
    assert_eq!(px(&s, 5, 5), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0]);
    s.execute("paint.bucket", json!({"x": 2, "y": 2, "color": "#0000ff", "contiguous": false, "antiAlias": false, "tolerance": 0})).unwrap();
    assert_eq!(px(&s, 5, 5), vec![0.0, 0.0, 1.0, 1.0]);
}

/// #4196: the Paint Bucket and Edit > Stroke write a CMYK pick as its inks into a CMYK document,
/// not as its sRGB stand-in converted back (100/0/0/0 cyan would come out as about 100/28/0/17).
#[test]
fn bucket_and_stroke_write_a_cmyk_pick_as_its_inks() {
    let cyan = [1.0f32, 0.0, 0.0, 0.0];
    let mut s = session();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    let rgb = photocraft_color::convert::cmyk_to_rgb(cyan).map(|c| c.clamp(0.0, 1.0));
    s.execute("tools.setColors", json!({"foreground": rgb, "foregroundCmyk": cyan})).unwrap();
    let inks_at = |s: &Session, x: i32, y: i32| -> Vec<f32> {
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        surf.read_region(Rect::new(x, y, x + 1, y + 1)).into_iter().take(4).collect()
    };
    let is_cyan = |got: &[f32]| got.iter().zip(cyan).all(|(a, b)| (a - b).abs() < 0.02);
    s.execute("paint.bucket", json!({"x": 2, "y": 2, "antiAlias": false})).unwrap();
    let got = inks_at(&s, 5, 5);
    assert!(is_cyan(&got), "bucket: {got:?}");
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.stroke", json!({"width": 2, "location": "inside", "color": rgb})).unwrap();
    let got = inks_at(&s, 0, 5);
    assert!(is_cyan(&got), "stroke: {got:?}");
}

#[test]
fn bucket_fills_with_a_pattern() {
    // Paint Bucket with contents="pattern" flood-fills the region with a pattern, not a colour.
    let mut s = session();
    s.edit("wall", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 0, 11, 10), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("paint.bucket", json!({"x": 2, "y": 2, "contents": "pattern", "pattern": "Diagonal Lines", "antiAlias": false, "tolerance": 0})).unwrap();
    // The filled region (left of the wall) now carries the pattern's light/dark variation,
    // while the wall and the region past it are untouched.
    let left: Vec<[f32; 4]> = (0..10)
        .map(|x| {
            let p = px(&s, x, 0);
            [p[0], p[1], p[2], p[3]]
        })
        .collect();
    assert!(left.iter().any(|p| p[0] < 0.3) && left.iter().any(|p| p[0] > 0.9), "pattern varies in the fill: {left:?}");
    assert_eq!(px(&s, 15, 5), vec![1.0, 1.0, 1.0, 1.0], "region past the wall untouched");
}

#[test]
fn gradient_tool_with_selection_and_reverse() {
    let mut s = session();
    // Pixels are sampled a pixel up and left of their centre (#12018), so pixel 19 reads 18.5 of 19.
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [19, 0], "colors": ["#000000", "#ffffff"]})).unwrap();
    assert!(px(&s, 0, 5)[0] < 0.05 && px(&s, 19, 5)[0] > 0.95);
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"], "reverse": true})).unwrap();
    assert!(px(&s, 0, 5)[0] > 0.95);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 10})).unwrap();
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [0, 10], "colors": ["#ff0000"]})).unwrap();
    assert_eq!(px(&s, 2, 5)[..3], [1.0, 0.0, 0.0]);
    assert!(px(&s, 10, 5)[1] > 0.4, "outside the selection untouched");
    assert!(s.execute("paint.gradient", json!({"from": [0, 0]})).is_err());
}

#[test]
fn gradient_dither_breaks_banding() {
    // A flat gradient (same colour both ends) is perfectly smooth with dither off, and gains
    // per-pixel noise with dither on (which breaks 8-bit banding).
    let run = |dither: bool| {
        let mut s = session();
        s.execute("paint.gradient", json!({"from": [0, 0], "to": [19, 0], "colors": [[0.5, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]], "dither": dither})).unwrap();
        (0..19).map(|x| px(&s, x, 5)[0]).collect::<Vec<f32>>()
    };
    let smooth = run(false);
    assert!(smooth.iter().all(|&v| v == smooth[0]), "no dither: perfectly flat");
    let noisy = run(true);
    assert!(noisy.windows(2).any(|w| w[0] != w[1]), "dither: per-pixel variation");
}

/// The two commands under test, each painting #808080 over a white document.
fn grey_fill(cmd: &str, opacity: Value) -> Value {
    match cmd {
        "paint.bucket" => json!({"x": 2, "y": 2, "color": "#808080", "opacity": opacity}),
        _ => json!({"from": [0, 0], "to": [19, 0], "colors": ["#808080", "#808080"], "dither": false, "opacity": opacity}),
    }
}

fn white(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "depth": depth, "background": "white"})).unwrap();
    s
}

/// Issue #4167: an opacity above 100 overshot the blend (200 painted near-black on white).
#[test]
fn bucket_and_gradient_clamp_opacity_like_the_magic_eraser() {
    for cmd in ["paint.bucket", "paint.gradient"] {
        for depth in [8, 16, 32] {
            let mut full = white(depth);
            full.execute(cmd, grey_fill(cmd, json!(100))).unwrap();
            let expected = px(&full, 5, 5);
            assert!((expected[0] - 128.0 / 255.0).abs() < 0.01, "{cmd} depth {depth}: 100% paints #808080, got {expected:?}");
            for over in [200.0, 1e9] {
                let mut s = white(depth);
                s.execute(cmd, grey_fill(cmd, json!(over))).unwrap();
                assert_eq!(px(&s, 5, 5), expected, "{cmd} depth {depth}: opacity {over} clamps to 100");
            }
            let (mut low, mut one) = (white(depth), white(depth));
            low.execute(cmd, grey_fill(cmd, json!(-50))).unwrap();
            one.execute(cmd, grey_fill(cmd, json!(1))).unwrap();
            assert_eq!(px(&low, 5, 5), px(&one, 5, 5), "{cmd} depth {depth}: opacity below 1 clamps to 1");
            assert!(px(&low, 5, 5).iter().all(|v| (0.0..=1.0).contains(v)), "{cmd} depth {depth}: no overshoot");
        }
    }
}

#[test]
fn bucket_and_gradient_reject_non_numeric_opacity_without_editing() {
    for cmd in ["paint.bucket", "paint.gradient"] {
        for bad in [json!("50"), json!(true), json!([50]), json!({"v": 50})] {
            let mut s = white(8);
            let (before, steps) = (px(&s, 5, 5), s.active().unwrap().history.past_len());
            let err = s.execute(cmd, grey_fill(cmd, bad.clone())).unwrap_err();
            assert!(matches!(err, EngineError::BadParams { .. }), "{cmd} {bad}: {err:?}");
            assert_eq!(px(&s, 5, 5), before, "{cmd} {bad}: pixels unchanged");
            assert_eq!(s.active().unwrap().history.past_len(), steps, "{cmd} {bad}: no history step");
        }
        // An explicit null keeps the default (100%), like the Magic Eraser.
        let mut s = white(8);
        s.execute(cmd, grey_fill(cmd, Value::Null)).unwrap();
        assert!((px(&s, 5, 5)[0] - 128.0 / 255.0).abs() < 0.01, "{cmd}: null opacity is 100%");
    }
}

/// #11311: a white layer (the Background made a normal layer), a selection cleared to
/// transparency, Deselect, then the Paint Bucket clicked in the hole with white at 100 %. Returns
/// the session after the fill and the layer's pixels right after the clear.
fn clear_then_bucket(depth: u32, selection: Value, bucket_aa: bool) -> (Session, Vec<[f32; 4]>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 100, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    s.execute("select.rect", selection).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    let cleared = layer_rgba(&s);
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("paint.bucket", json!({"x": 60, "y": 50, "color": "#ffffff", "antiAlias": bucket_aa})).unwrap();
    (s, cleared)
}

/// The active layer's pixels over the canvas, row by row.
fn layer_rgba(s: &Session) -> Vec<[f32; 4]> {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    let b = d.doc.bounds();
    (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).map(move |x| (x, y))).map(|(x, y)| surf.rgba(x, y)).collect()
}

/// The first `n` pixels (as `x, y, rgba`) that aren't opaque white at 8 bits.
fn not_white(px: &[[f32; 4]], width: usize, n: usize) -> Vec<(usize, usize, [f32; 4])> {
    let is_white = |p: &[f32; 4]| p.iter().all(|c| (c.clamp(0.0, 1.0) * 255.0).round() == 255.0);
    px.iter().enumerate().filter(|(_, p)| !is_white(p)).take(n).map(|(i, p)| (i % width, i / width, *p)).collect()
}

fn assert_all_white(s: &Session, what: &str) {
    let doc = &s.active().unwrap().doc;
    let width = doc.bounds().width() as usize;
    let layer = layer_rgba(s);
    assert_eq!(not_white(&layer, width, 8), vec![], "{what}: layer pixels left after the fill");
    let composite = photocraft_compose::render(doc, doc.bounds()).px.clone();
    assert_eq!(composite.len(), layer.len());
    assert_eq!(not_white(&composite, width, 8), vec![], "{what}: composite pixels left after the fill");
}

/// #11311 / #11354 review: after clearing an anti-aliased selection and bucket-filling the hole
/// white at 100 %, the hole and the hole's edge are opaque white and the opaque art is untouched.
/// The soft rim the Clear left (straight white at partial alpha) rises only partly, as in the
/// reference application: a pixel of the one-pixel ring 8-adjacent to the flood, at alpha `α`,
/// ends at `α + (1 − α)²`, and pixels further out keep their alpha, so a faint rim stays. Before the first round the hole's own edge stayed semi-transparent (a grey ring);
/// the first round filled the rim to 255 too, which the reference does not do.
#[test]
fn bucket_after_clearing_an_antialiased_selection_fills_the_hole_and_raises_the_rim_partly() {
    let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    for depth in [8, 16, 32] {
        let ellipse = json!({"x": 20, "y": 10, "width": 80, "height": 80, "ellipse": true, "antiAlias": true});
        let (s, cleared) = clear_then_bucket(depth, ellipse, true);
        // A transparent hole (its RGB zeroed) inside a rim of straight white at partial alpha,
        // which the bucket's similarity test (RGB 255 away from the hole's 0) leaves out.
        assert_eq!(cleared[50 * 120 + 60], [0.0; 4], "depth {depth}: the hole is empty");
        let rim: Vec<&[f32; 4]> = cleared.iter().filter(|p| p[3] > 0.0 && p[3] < 1.0).collect();
        assert!(!rim.is_empty(), "depth {depth}: an anti-aliased clear leaves a soft rim");
        assert!(rim.iter().all(|p| p[..3].iter().all(|c| (c - 1.0).abs() < 1e-3)), "depth {depth}: the rim keeps its white");

        let after = layer_rgba(&s);
        // The flood is every pixel the 8-bit wand sees as empty; the ring is the pixels 8-adjacent to it.
        let empty = |x: i64, y: i64| (0..120).contains(&x) && (0..100).contains(&y) && level(cleared[(y * 120 + x) as usize][3]) == 0;
        let mut raised = 0;
        for (i, (was, now)) in cleared.iter().zip(&after).enumerate() {
            let at = (i % 120, i / 120);
            let (x, y) = (at.0 as i64, at.1 as i64);
            let next_to_flood = (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && empty(x + dx, y + dy)));
            if level(was[3]) == 0 || was[3] >= 1.0 {
                assert_eq!(level(now[3]), 255, "depth {depth} at {at:?}: hole, hole edge and opaque art end opaque: {now:?}");
            } else if next_to_flood {
                raised += 1;
                let want = was[3] + (1.0 - was[3]).powi(2);
                assert!((now[3] - want).abs() < 1.5 / 255.0, "depth {depth} at {at:?}: rim {was:?} -> {now:?}, want alpha {want}");
            } else {
                assert_eq!(now, was, "depth {depth} at {at:?}: a rim pixel beyond the ring next to the flood stays");
            }
            assert!(now[..3].iter().all(|c| (c - 1.0).abs() < 1e-3), "depth {depth} at {at:?}: white: {now:?}");
        }
        assert!(raised > 0, "depth {depth}: the fill reaches the rim");
        assert!(after.iter().any(|p| level(p[3]) < 255), "depth {depth}: a faint rim stays, as in the reference");
    }
}

/// Controls for #11311: a hard-edged selection leaves no soft rim, but the bucket's own
/// anti-aliasing used to leave the hole's edge pixels partly transparent too, so hard edges end
/// white as well; with neither anti-aliased (the case that was always right) it stays white.
#[test]
fn bucket_after_clearing_a_hard_selection_leaves_no_ring() {
    let hard_ellipse = json!({"x": 20, "y": 10, "width": 80, "height": 80, "ellipse": true, "antiAlias": false});
    let rect = json!({"x": 30, "y": 20, "width": 60, "height": 60});
    for depth in [8, 16, 32] {
        for (selection, bucket_aa) in [(hard_ellipse.clone(), true), (rect.clone(), true), (rect.clone(), false)] {
            let (s, cleared) = clear_then_bucket(depth, selection.clone(), bucket_aa);
            assert!(cleared.iter().all(|p| p[3] == 0.0 || p[3] == 1.0), "depth {depth} {selection}: a hard clear leaves no rim");
            assert_all_white(&s, &format!("depth {depth} {selection}, bucket antiAlias {bucket_aa}"));
        }
    }
}
