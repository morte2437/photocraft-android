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
    s.execute("paint.gradient", json!({"from": [0, 0], "to": [20, 0], "colors": ["#000000", "#ffffff"]})).unwrap();
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
