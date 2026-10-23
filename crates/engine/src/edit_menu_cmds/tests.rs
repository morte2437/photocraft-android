use super::*;
use photocraft_doc::vector::Subpath;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "background": "white"})).unwrap();
    s
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

fn active(s: &Session) -> &Layer {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap()
}

#[test]
fn fade_blends_last_step_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        assert!(!s.is_enabled("edit.fade"), "nothing to fade yet");
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        assert!(s.is_enabled("edit.fade"));
        let undo_steps = s.active().unwrap().history.past_len();
        s.execute("edit.fade", json!({"opacity": 25})).unwrap();
        let v = px(&s, 5, 5);
        assert!((v[0] - 0.75).abs() < 0.01, "depth {depth}: {v:?}");
        // One history step, labelled after the faded command, and Fade can't repeat.
        assert_eq!(s.active().unwrap().history.past_len(), undo_steps + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Fade Fill"));
        assert!(!s.is_enabled("edit.fade"));
        s.undo();
        assert_eq!(px(&s, 5, 5)[0], 0.0);
    }
}

#[test]
fn fade_with_blend_mode_and_invalidation() {
    let mut s = session(8);
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    // Difference of grey and inverted grey at 100%: |0.502 − 0.498| ≈ 0.
    s.execute("edit.fade", json!({"opacity": 100, "mode": "difference"})).unwrap();
    assert!(px(&s, 1, 1)[0] < 0.02);
    // Any other edit in between disables Fade.
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(!s.is_enabled("edit.fade"));
    assert!(s.execute("edit.fade", json!({})).is_err());
    // Queries don't.
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("document.inspect", json!({})).unwrap();
    assert!(s.is_enabled("edit.fade"));
    assert!(s.execute("edit.fade", json!({"mode": "nonsense"})).is_err());
}

#[test]
fn fade_uniform_layer_mask_invert_at_all_depths() {
    for depth in [8, 16, 32] {
        for (opacity, expected) in [(0, 1.0), (50, 0.5), (100, 0.0)] {
            let mut s = session(depth);
            s.execute("layer.layerMask.revealAll", json!({})).unwrap();
            s.execute("channel.target", json!({"channel": "mask"})).unwrap();
            let mask_pixel = |s: &Session| {
                let layer = active(s);
                let mask = &layer.mask.as_ref().unwrap().surface;
                mask.pixel(5, 5)[0]
            };
            assert!((mask_pixel(&s) - 1.0).abs() < 0.01);
            s.execute("image.adjustments.invert", json!({})).unwrap();
            assert!(mask_pixel(&s) < 0.01, "depth {depth}: invert changed the mask");
            assert!(s.edit_state.fade.as_ref().is_some_and(|f| matches!(f.target, FadeTarget::Mask(_))), "depth {depth}: Fade must target the mask");
            let history_before_fade = s.active().unwrap().history.past_len();

            s.execute("edit.fade", json!({"opacity": opacity})).unwrap();
            for (x, y) in [(0, 0), (5, 5), (63, 47)] {
                let pixel = active(&s).mask.as_ref().unwrap().surface.pixel(x, y)[0];
                assert!((pixel - expected).abs() < 0.02, "depth {depth}, opacity {opacity}, ({x}, {y}): {pixel}");
            }
            assert_eq!(s.active().unwrap().history.past_len(), history_before_fade + 1);
            assert!(s.undo());
            assert!(mask_pixel(&s) < 0.01, "undo restores inverted mask");
            assert!(s.redo());
            assert!((mask_pixel(&s) - expected).abs() < 0.02, "redo restores faded mask");
        }
    }
}

/// #7382: with an alpha channel or the Quick Mask targeted, the edit changes it, not the layer, and
/// Fade blends it back: at 0 % to the channel as it was, at every depth.
#[test]
fn fade_an_alpha_channel_or_the_quick_mask_at_all_depths() {
    for depth in [8, 16, 32] {
        for quick_mask in [false, true] {
            for (opacity, k) in [(0, 0.0), (25, 0.25), (100, 1.0)] {
                let mut s = session(depth);
                if quick_mask {
                    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                } else {
                    s.execute("channel.new", json!({"fill": "black"})).unwrap();
                    s.execute("channel.target", json!({"channel": 0})).unwrap();
                }
                let channel = |s: &Session, x, y| {
                    let doc = &s.active().unwrap().doc;
                    let c = if quick_mask { doc.quick_mask.as_ref() } else { doc.channels.first() };
                    c.unwrap().surface.pixel(x, y)[0]
                };
                let was = channel(&s, 5, 5);
                s.execute("image.adjustments.invert", json!({})).unwrap();
                assert!((channel(&s, 5, 5) - (1.0 - was)).abs() < 0.01, "depth {depth}, quick mask {quick_mask}: inverted");
                let expected = was + (1.0 - 2.0 * was) * k;
                let steps = s.active().unwrap().history.past_len();
                s.execute("edit.fade", json!({"opacity": opacity})).unwrap();
                for (x, y) in [(0, 0), (5, 5), (63, 47)] {
                    let v = channel(&s, x, y);
                    assert!((v - expected).abs() < 0.02, "depth {depth}, quick mask {quick_mask}, {opacity} %, ({x}, {y}): {v}");
                }
                assert_eq!(px(&s, 5, 5)[0], 1.0, "the layer is left alone");
                let st = s.active().unwrap();
                assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some("Fade Invert")));
                assert!(s.undo());
                assert!((channel(&s, 5, 5) - (1.0 - was)).abs() < 0.01, "undo puts the inverted channel back");
            }
        }
    }
}

/// A command told to edit the layer (`"target": "pixels"`) does so with an alpha channel targeted,
/// and Fade then fades the layer, not the channel.
#[test]
fn fade_follows_the_target_the_command_ran_on() {
    let mut s = session(8);
    s.execute("channel.new", json!({"fill": "black"})).unwrap();
    s.execute("channel.target", json!({"channel": 0})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000", "target": "pixels"})).unwrap();
    assert_eq!(px(&s, 5, 5)[0], 0.0, "the layer was filled");
    assert!(matches!(s.edit_state.fade.as_ref().map(|f| f.target), Some(FadeTarget::Pixels(_))));
    s.execute("edit.fade", json!({"opacity": 25})).unwrap();
    assert!((px(&s, 5, 5)[0] - 0.75).abs() < 0.01, "the layer faded");
    assert_eq!(s.active().unwrap().doc.channels[0].surface.pixel(5, 5)[0], 0.0, "the channel is left alone");
}

/// A command that only edits layers (Auto Tone) fades the layer, even given a channel `target`.
#[test]
fn fade_ignores_a_target_the_command_doesnt_take() {
    let mut s = session(8);
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    s.execute("image.autoTone", json!({"target": "quickMask"})).unwrap();
    assert!(matches!(s.edit_state.fade.as_ref().map(|f| f.target), Some(FadeTarget::Pixels(_))));
}

#[test]
fn purge_commands() {
    let mut s = session(8);
    assert!(!s.is_enabled("edit.purge.undo"));
    assert!(s.is_enabled("edit.purge.videoCache"));
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let r = s.execute("edit.purge.undo", json!({})).unwrap();
    assert_eq!(r["purged"][0], "undo");
    assert!(r["bytes"].as_u64().unwrap() > 0);
    // One step left to undo (the black fill is gone from history).
    assert!(s.undo());
    assert!(!s.undo());
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.is_enabled("edit.purge.clipboard"));
    s.execute("edit.purge.clipboard", json!({})).unwrap();
    assert!(s.clipboard.is_none());
    assert!(!s.is_enabled("edit.purge.clipboard"));
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("edit.purge.histories", json!({})).unwrap();
    assert!(s.documents().iter().all(|d| !d.history.can_undo()));
    assert!(!s.is_enabled("edit.purge.histories"));
    assert_eq!(s.execute("edit.purge.videoCache", json!({})).unwrap()["bytes"], 0);
    let _ = s.execute("edit.purge.all", json!({}));
}

/// White canvas with vertical black stripes, a red blob to remove, and a selection around it.
fn blob_session(depth: u32) -> Session {
    let mut s = session(depth);
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in (0..64).step_by(8) {
            surf.fill_rect(Rect::new(x, 0, x + 3, 48), &[0.0, 0.0, 0.0, 1.0]);
        }
        surf.fill_rect(Rect::new(28, 18, 36, 28), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 26, "y": 16, "width": 12, "height": 14})).unwrap();
    s
}

#[test]
fn content_aware_fill_huge_area_neither_panics_nor_wraps() {
    let mut s = blob_session(8);
    // `[1e30, 0, 1e30, 10]` overflowed the i32 additions in the area parser (a debug-build
    // panic, a garbage sampling window in release); the additions saturate now, so the area lies
    // wholly right of the canvas: nothing to sample, which is an error, not a black fill (#6532).
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    let err = s.execute("edit.contentAwareFill", json!({"sampling": "custom", "area": [1e30, 0.0, 1e30, 10.0], "colorAdaptation": "none"})).unwrap_err();
    assert!(err.to_string().contains("nothing to sample from"), "{err}");
    let st = s.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "the document changed");
    assert_eq!(st.history.past_len(), past, "a history step was recorded");
    // A malformed area names the problem instead of silently falling back.
    for area in [json!([0.0, 0.0, null, 10.0]), json!([0.0, 0.0, 10.0]), json!("nope")] {
        let err = s.execute("edit.contentAwareFill", json!({"sampling": "custom", "area": area})).unwrap_err();
        assert!(err.to_string().contains("`area`"), "{err}");
    }
}

/// `blob_session` with the canvas declared as `size` and a sparse selection of `rects`, as a
/// loaded `.pcraft` can hold (#963). Nothing here allocates more than a few tiles.
fn huge_selection(size: (u32, u32), rects: &[Rect]) -> Session {
    let mut s = blob_session(8);
    let st = s.active_mut().unwrap();
    let doc = std::sync::Arc::make_mut(&mut st.doc);
    doc.size = photocraft_geom::Size::new(size.0, size.1);
    let mut sel = Surface::new(PixelFormat::GRAY8);
    for r in rects {
        sel.fill_rect(*r, &[1.0]);
    }
    doc.selection = Some(sel);
    s
}

/// A sampling window too large to read, or whose margin overflows, is the size error: no panic,
/// no huge allocation, the document and history untouched (#963).
#[test]
fn content_aware_fill_huge_window_is_an_error() {
    // Largest extent whose `ext * 3` still fits in i32.
    const EDGE: i32 = i32::MAX / 3;
    let max = i32::MAX;
    let big = (2_000_000_000, 2_000_000_000);
    let dot = Rect::new(0, 0, 1, 1);
    let cases = [
        ("auto, wide, just above", big, vec![dot, Rect::new(EDGE, 0, EDGE + 1, 1)], json!({})),
        ("auto, tall, just above", big, vec![dot, Rect::new(0, EDGE, 1, EDGE + 1)], json!({})),
        ("auto, wide, just below", big, vec![dot, Rect::new(EDGE - 1, 0, EDGE, 1)], json!({})),
        (
            "auto, extreme coordinates",
            (max as u32, max as u32),
            vec![Rect::new(i32::MIN, i32::MIN, i32::MIN + 1, i32::MIN + 1), dot, Rect::new(max - 2, max - 2, max - 1, max - 1)],
            json!({}),
        ),
        (
            "rectangular, small selection",
            (300_000, 300_000),
            vec![Rect::new(150_000, 150_000, 150_001, 150_001)],
            json!({"sampling": "rectangular", "margin": 100_000}),
        ),
        ("rectangular, huge extent", big, vec![dot, Rect::new(EDGE + 1, 0, EDGE + 2, 1)], json!({"sampling": "rectangular"})),
        ("custom, whole canvas", big, vec![dot], json!({"sampling": "custom", "area": [0, 0, 2_000_000_000, 2_000_000_000]})),
    ];
    for (name, size, rects, p) in cases {
        let mut s = huge_selection(size, &rects);
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        let err = s.execute("edit.contentAwareFill", p).expect_err(name).to_string();
        assert!(err.contains("too large for Content-Aware fill"), "{name}: {err}");
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "{name}: the document changed");
        assert_eq!(st.history.past_len(), past, "{name}: a history step was recorded");
    }
    // Control: an ordinary selection still fills.
    let mut s = huge_selection((64, 48), &[Rect::new(26, 16, 38, 30)]);
    let r = s.execute("edit.contentAwareFill", json!({"colorAdaptation": "none"})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 100);
}

#[test]
fn content_aware_fill_removes_object_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = blob_session(depth);
        assert!(s.is_enabled("edit.contentAwareFill"));
        let r = s.execute("edit.contentAwareFill", json!({"colorAdaptation": "none"})).unwrap();
        assert!(r["filled"].as_u64().unwrap() > 100);
        for (x, y) in [(30, 20), (33, 25)] {
            let v = px(&s, x, y);
            assert!(!(v[0] > 0.9 && v[1] < 0.1), "depth {depth}: red left at ({x},{y}): {v:?}");
        }
        // Outside the selection untouched.
        assert_eq!(px(&s, 1, 1)[0], 0.0);
        assert_eq!(px(&s, 4, 1)[0], 1.0);
        s.undo();
        assert_eq!(px(&s, 30, 20)[1], 0.0, "undo restores the blob");
    }
}

#[test]
fn delete_and_fill_selection_removes_the_object_in_one_step() {
    // #1286: Content-Aware Fill's defaults, no dialog, one history step named for the command.
    for depth in [8, 16, 32] {
        let mut s = blob_session(depth);
        let base = s.active().unwrap().active_layer.unwrap();
        assert!(s.is_enabled("edit.deleteAndFillSelection"));
        let r = s.execute("edit.deleteAndFillSelection", json!({})).unwrap();
        assert_eq!(r["layer"].as_u64(), Some(base.0), "fills the layer itself");
        assert!(r["filled"].as_u64().unwrap() > 100);
        let v = px(&s, 30, 20);
        assert!(!(v[0] > 0.9 && v[1] < 0.1), "depth {depth}: red left: {v:?}");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Delete and Fill Selection"));
        s.undo();
        assert_eq!(px(&s, 30, 20)[1], 0.0, "undo restores the blob");
    }
    // Parameters are ignored, never a crash; nothing selected greys it out and refuses.
    let mut s = blob_session(8);
    assert!(s.execute("edit.deleteAndFillSelection", json!({"output": 7, "sampling": [1]})).is_ok());
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled("edit.deleteAndFillSelection"));
    assert!(s.execute("edit.deleteAndFillSelection", json!({})).is_err());
}

#[test]
fn content_aware_fill_on_an_empty_layer_samples_the_composite() {
    let mut s = blob_session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let empty = s.active().unwrap().active_layer.unwrap();
    assert_eq!(px(&s, 30, 20)[3], 0.0, "the new layer has no paint");
    let r = s.execute("edit.contentAwareFill", json!({"colorAdaptation": "none"})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 0);
    assert_eq!(s.active().unwrap().active_layer, Some(empty));
    assert!(px(&s, 30, 20)[3] > 0.5, "the hole is filled from the layers underneath");
    s.undo();
    let missed = s.execute("edit.contentAwareFill", json!({"sampleAllLayers": false, "colorAdaptation": "none"})).unwrap();
    assert!(missed["filled"].as_u64().unwrap() > 0);
    assert!(px(&s, 30, 20)[3] < 0.05, "sampling only the empty layer leaves it empty");
}

#[test]
fn content_aware_fill_on_an_empty_cmyk_layer_uses_the_layer_format() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 36, "mode": "cmyk", "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 12, "width": 12, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let before = px(&s, 20, 16);
    assert!(before.iter().all(|c| *c < 0.05), "the new CMYK layer is empty: {before:?}");
    s.execute("edit.contentAwareFill", json!({"colorAdaptation": "none"})).unwrap();
    let v = px(&s, 20, 16);
    assert_eq!(v.len(), before.len());
    // White in the layer's CMYK is near zero ink with a covered alpha, not the compositor's RGBA.
    assert!(v[..4].iter().all(|c| *c < 0.15), "CMYK ink stayed low: {v:?}");
    assert!(v.last().copied().unwrap_or(0.0) > 0.5, "the hole is covered: {v:?}");
}

#[test]
fn content_aware_fill_samples_photoshops_window() {
    // A 50 px selection samples the 200 px square centred on it (Photoshop 25.4's overlay);
    // at the canvas edge the square slides inwards. Edit › Fill uses the same window.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    for (x, y, want) in [(275, 175, [200, 100, 200, 200]), (10, 10, [0, 0, 200, 200])] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 50, "height": 50})).unwrap();
        for sampling in ["auto", "rectangular"] {
            let r = s.execute("edit.contentAwareFill", json!({"sampling": sampling})).unwrap();
            assert_eq!(r["window"], json!(want), "{sampling} at {x},{y}");
            s.undo();
        }
    }
}

#[test]
fn content_aware_fill_takes_the_workspace_edits() {
    // The plan the workspace previews: lasso edits change the fill area (and the window follows),
    // sampling-brush strokes change what may be copied, also outside the window.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 600, "height": 400, "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 275, "y": 175, "width": 50, "height": 50})).unwrap();
    let st = s.active().unwrap();
    let (doc, id) = (st.doc.clone(), st.active_layer.unwrap());
    let plan = |p: Value| content_aware_plan(&doc, id, &p, "edit.contentAwareFill");
    let count = |m: &[bool]| m.iter().filter(|v| **v).count();
    let base = plan(json!({"sampling": "rectangular"})).unwrap();
    assert_eq!((base.window, count(&base.hole)), (Rect::new(200, 100, 400, 300), 2500));
    assert_eq!(count(&base.source), 200 * 200 - 2500);
    // A lasso square added beside the selection: a bigger fill area, a window around both.
    let p = plan(json!({"sampling": "rectangular", "fillEdits": [{"op": "add", "points": [[340, 175], [390, 175], [390, 225], [340, 225]]}]})).unwrap();
    assert_eq!(count(&p.hole), 5000);
    assert!(p.window.width() > 200 && p.window.contains_rect(&Rect::new(275, 175, 390, 225)), "{:?}", p.window);
    // Expand by 2 grows the 50 px square to about 54 px (rounded corners).
    let p = plan(json!({"sampling": "rectangular", "fillEdits": [{"op": "expand", "by": 2}]})).unwrap();
    assert!((54 * 54 - 20..=54 * 54).contains(&count(&p.hole)), "{}", count(&p.hole));
    // Subtracting it all leaves nothing to fill.
    assert!(plan(json!({"fillEdits": [{"op": "subtract", "points": [[0, 0], [600, 0], [600, 400], [0, 400]]}]})).is_err());
    // A subtracting stroke takes a band out of the sampling area; an adding one far away reaches
    // past the window, and the plan's rectangle grows to hold it.
    let p = plan(json!({"sampling": "rectangular", "samplingStrokes": [{"mode": "subtract", "size": 10, "points": [[210, 110], [390, 110]]}]})).unwrap();
    assert!((count(&base.source) - count(&p.source)).abs_diff(10 * 190) < 60);
    let p = plan(json!({"sampling": "rectangular", "samplingStrokes": [{"mode": "add", "size": 20, "points": [[540, 350], [580, 350]]}]})).unwrap();
    assert!(p.rect.contains(560, 350) && !p.window.contains(560, 350));
    assert!(count(&p.source) > count(&base.source) + 600);
    // Custom from strokes alone; bad edits are errors.
    let p = plan(json!({"sampling": "custom", "samplingStrokes": [{"mode": "add", "size": 20, "points": [[100, 100], [160, 100]]}]})).unwrap();
    assert!((1450..1600).contains(&count(&p.source)), "a 20 px brush along 60 px: {}", count(&p.source));
    for bad in [
        json!({"samplingStrokes": [{"mode": "paint", "size": 10, "points": [[0, 0]]}]}),
        json!({"samplingStrokes": [{"size": 0, "points": [[0, 0]]}]}),
        json!({"fillEdits": [{"op": "add", "points": [[0, "x"]]}]}),
        json!({"fillEdits": [{"op": "expand", "by": -3}]}),
    ] {
        assert!(plan(bad.clone()).is_err(), "{bad}");
    }
    // The command fills with the same plan; `layer` names the layer even when another is active.
    let r = s.execute("edit.contentAwareFill", json!({"sampling": "rectangular", "fillEdits": [{"op": "expand", "by": 2}], "layer": id.0})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 2800);
}

/// #6532: a sampling area with no pixels outside the fill area is refused instead of filling it
/// black, and leaves the document untouched; any margin past the selection fills.
#[test]
fn content_aware_fill_with_nothing_to_sample_is_refused() {
    let mut s = blob_session(8);
    let before = px(&s, 30, 22);
    let revision = s.active().unwrap().revision;
    for p in [json!({"sampling": "rectangular", "margin": 0}), json!({"sampling": "custom", "area": [28, 18, 4, 4]})] {
        let e = s.execute("edit.contentAwareFill", p.clone()).unwrap_err().to_string();
        assert!(e.contains("nothing to sample from"), "{p}: {e}");
        assert_eq!(px(&s, 30, 22), before, "{p}: nothing was filled");
        assert_eq!(s.active().unwrap().revision, revision, "{p}: no history step");
    }
    s.execute("edit.contentAwareFill", json!({"sampling": "rectangular", "margin": 1})).unwrap();
    assert_ne!(px(&s, 30, 22), before, "the red blob is filled from around the selection");
}

#[test]
fn content_aware_fill_outputs_and_sampling() {
    let mut s = blob_session(8);
    let base = s.active().unwrap().active_layer.unwrap();
    let r = s.execute("edit.contentAwareFill", json!({"output": "new", "sampling": "rectangular", "margin": 12})).unwrap();
    let nid = LayerId(r["layer"].as_u64().unwrap());
    assert_ne!(nid, base);
    // The new layer holds only the fill.
    let l = s.active().unwrap().doc.layer(nid).unwrap();
    let b = l.surface().unwrap().content_bounds();
    assert!(Rect::new(26, 16, 38, 30).contains_rect(&b), "{b:?}");
    s.undo();
    let r = s.execute("edit.contentAwareFill", json!({"output": "duplicate", "sampling": "custom", "area": [0, 0, 20, 48], "mirror": true})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().name, "Background copy");
    assert!(s.execute("edit.contentAwareFill", json!({"sampling": "custom"})).is_err());
    assert!(s.execute("edit.contentAwareFill", json!({"output": "elsewhere"})).is_err());
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled("edit.contentAwareFill"));
}

#[test]
fn content_aware_scale_keeps_subject() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 64, 48), &[0.5, 0.5, 0.5, 1.0]);
            for x in 20..26 {
                let c = if x % 2 == 0 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] };
                surf.fill_rect(Rect::new(x, 0, x + 1, 48), &c);
            }
            Ok(())
        })
        .unwrap();
        let r = s.execute("edit.contentAwareScale", json!({"width": 40})).unwrap();
        assert_eq!(r["to"], json!([40, 48]));
        let surf = active(&s).surface().unwrap();
        assert_eq!(surf.content_bounds(), Rect::new(0, 0, 40, 48), "depth {depth}");
        let striped = (0..40).filter(|x| {
            let p = surf.pixel(*x, 10);
            p[1] < 0.05 && (p[0] > 0.95 || p[2] > 0.95)
        });
        assert_eq!(striped.count(), 6, "depth {depth}: the striped subject survives");
        s.undo();
        assert_eq!(active(&s).surface().unwrap().content_bounds().width(), 64);
    }
}

#[test]
fn content_aware_scale_params() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 32), &[0.8, 0.6, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let r = s.execute("edit.contentAwareScale", json!({"scaleX": 150, "scaleY": 50, "amount": 50, "protectSkinTones": true})).unwrap();
    assert_eq!(r["to"], json!([48, 16]));
    assert!(s.execute("edit.contentAwareScale", json!({"width": 0})).is_err());
    assert!(s.execute("edit.contentAwareScale", json!({"protect": "no such channel"})).is_err());
}

#[test]
fn content_aware_scale_places_the_result_and_previews_it_reduced() {
    // The transform box (#3096) moves the result with `x`/`y` and previews it on a reduced copy.
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(4, 4, 36, 28), &[0.2, 0.4, 0.6, 1.0]);
            Ok(())
        })
        .unwrap();
        let p = json!({"width": 16, "height": 40, "x": 10, "y": 2});
        let preview = super::ContentAwareScalePreview::new(&s, &p, 10).unwrap();
        assert_eq!(preview.region(), Rect::new(4, 4, 36, 28));
        let small = preview.run(&crate::jobs::JobCtx::new()).unwrap();
        assert_eq!(small.content_bounds(), Rect::new(0, 0, 4, 10), "depth {depth}: the longest side, 40 px, previews at 10");
        let past = s.active().unwrap().history.past_len();
        let r = s.execute("edit.contentAwareScale", p).unwrap();
        assert_eq!(r["bounds"], json!([10, 2, 16, 40]));
        assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::from_xywh(10, 2, 16, 40), "depth {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "depth {depth}: one step");
        for bad in [json!({"x": "left"}), json!({"y": 1e9})] {
            assert!(s.execute("edit.contentAwareScale", bad.clone()).is_err(), "{bad}");
            assert!(super::ContentAwareScalePreview::new(&s, &bad, 64).is_err(), "{bad}");
        }
    }
}

#[test]
fn content_aware_scale_enlarges_a_one_pixel_line() {
    // A 1 px wide (or tall) layer has no seam to spare, so enlarging it used to return the line
    // unchanged and crash or write a short buffer. The line's single column (row) is repeated.
    for (line, params, to) in [
        (Rect::new(10, 0, 11, 48), json!({"width": 5}), [5, 48]),
        (Rect::new(10, 0, 11, 48), json!({"width": 6, "height": 30}), [6, 30]),
        (Rect::new(0, 7, 64, 8), json!({"height": 4}), [64, 4]),
        (Rect::new(0, 7, 64, 8), json!({"width": 20, "height": 3}), [20, 3]),
    ] {
        let mut s = session(8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(line, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let r = s.execute("edit.contentAwareScale", params.clone()).unwrap();
        assert_eq!(r["to"], json!(to), "{params}");
        let b = active(&s).surface().unwrap().content_bounds();
        assert_eq!([b.width(), b.height()], to, "{params}");
        assert_eq!(px(&s, b.x0 + b.width() as i32 - 1, b.y0 + b.height() as i32 - 1), vec![1.0, 0.0, 0.0, 1.0], "{params}");
    }
}

fn square_path(x: f64, y: f64, w: f64) -> Path {
    Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + w), (x, y + w)])])
}

#[test]
fn define_custom_shape_and_preset_manager() {
    let mut s = session(8);
    assert!(!s.is_enabled("edit.defineCustomShape"));
    s.edit("path", |doc, _| {
        doc.work_path = Some(square_path(4.0, 4.0, 10.0));
        Ok(())
    })
    .unwrap();
    s.execute("edit.defineCustomShape", json!({"name": "Box"})).unwrap();
    s.execute("edit.defineCustomShape", json!({})).unwrap();
    assert_eq!(custom_shapes(&s).len(), 2);
    assert_eq!(custom_shapes(&s)[1].name, "Shape 2");
    let list = s.execute("edit.presets.presetManager", json!({})).unwrap();
    assert_eq!(list["customShapes"], json!(["Box", "Shape 2"]));
    assert!(list["brushes"].as_array().unwrap().len() > 3);
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "customShapes", "name": "Shape 2", "newName": "Star"})).unwrap();
    s.execute("edit.presets.presetManager", json!({"action": "move", "kind": "customShapes", "index": 1, "to": 0})).unwrap();
    assert_eq!(custom_shapes(&s)[0].name, "Star");
    s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "customShapes", "name": "Box"})).unwrap();
    assert_eq!(custom_shapes(&s).len(), 1);
    assert!(s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "gradients", "index": 0})).is_err());
    // The pattern library (Edit › Define Pattern) is managed here too.
    let n = list["patterns"].as_array().unwrap().len();
    assert!(n > 0);
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "patterns", "index": 0, "newName": "First"})).unwrap();
    s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "patterns", "name": "First"})).unwrap();
    assert_eq!(s.patterns.items.len(), n - 1);
    // Export → import into a fresh session.
    s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
    let out = s.execute("edit.presets.exportImportPresets", json!({"action": "export"})).unwrap();
    assert_eq!(out["brushes"], 1, "built-ins are not exported by default");
    let mut t = Session::new();
    let r = t.execute("edit.presets.exportImportPresets", json!({"action": "import", "data": out["data"].to_string()})).unwrap();
    assert_eq!((r["brushes"].as_u64(), r["customShapes"].as_u64()), (Some(1), Some(1)));
    assert!(t.tools.presets.iter().any(|b| b.name == "Mine"));
    assert_eq!(custom_shapes(&t)[0].path, custom_shapes(&s)[0].path);
    assert!(t.execute("edit.presets.exportImportPresets", json!({"action": "import", "data": {"format": "other"}})).is_err());
}

#[test]
fn preset_manager_brush_rename_follows_current_preset() {
    let mut s = Session::new();
    s.execute("brush.presets.setCurrent", json!({"name": "Chalk"})).unwrap();
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Chalk", "newName": "Chalky"})).unwrap();
    assert_eq!(s.tools.current_preset.as_deref(), Some("Chalky"));
    s.execute("brush.presets.update", json!({})).unwrap();
}

#[test]
fn preset_manager_brush_rename_refuses_duplicate_name() {
    let mut s = Session::new();
    let before: Vec<String> = s.tools.presets.iter().map(|b| b.name.clone()).collect();
    let r = s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Soft Round", "newName": "hard round"}));
    assert!(r.is_err(), "a case-insensitive duplicate name must be refused");
    assert_eq!(s.tools.presets.iter().map(|b| b.name.clone()).collect::<Vec<_>>(), before);
    // Renaming to a different case of its own name is fine.
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Soft Round", "newName": "soft round"})).unwrap();
}

#[test]
fn preset_manager_brush_delete_clears_current_preset() {
    let mut s = Session::new();
    s.execute("brush.presets.setCurrent", json!({"name": "Chalk"})).unwrap();
    s.execute("edit.presets.presetManager", json!({"action": "delete", "kind": "brushes", "name": "Chalk"})).unwrap();
    assert!(!s.tools.presets.iter().any(|b| b.name == "Chalk"));
    assert_eq!(s.tools.current_preset, None);
}

#[test]
fn define_brush_preset_wraps_define_from_selection() {
    let mut s = session(8);
    assert!(!s.is_enabled("edit.defineBrushPreset"));
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    s.execute("edit.defineBrushPreset", json!({})).unwrap();
    assert!(s.tools.presets.iter().any(|b| b.name == "Sampled Brush 1"));
}

#[test]
fn find_matches_cases() {
    assert_eq!(find_matches("Cat cat CAT", "cat", false, false), vec![(0, 3), (4, 7), (8, 11)]);
    assert_eq!(find_matches("Cat cat CAT", "cat", true, false), vec![(4, 7)]);
    assert_eq!(find_matches("cat catalog bobcat", "cat", false, true), vec![(0, 3)]);
    assert_eq!(find_matches("Straße STRASSE", "straße", false, false), vec![(0, 7)]);
    assert!(find_matches("abc", "", false, false).is_empty());
}

#[test]
fn find_and_replace_across_type_layers() {
    let mut s = session(8);
    assert!(!s.is_enabled("edit.findAndReplaceText"));
    s.execute("type.create", json!({"text": "Hello world", "x": 4, "y": 20, "size": 12})).unwrap();
    s.execute("type.create", json!({"text": "world peace, World", "x": 4, "y": 40, "size": 12})).unwrap();
    let r = s.execute("edit.findAndReplaceText", json!({"find": "world", "replace": "planet"})).unwrap();
    assert_eq!((r["count"].as_u64(), r["layers"].as_u64()), (Some(3), Some(2)));
    let texts: Vec<String> = s
        .active()
        .unwrap()
        .doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"Hello planet".to_string()), "{texts:?}");
    assert!(texts.contains(&"planet peace, planet".to_string()), "{texts:?}");
    s.undo();
    // Step through matches: case-sensitive finds only lower-case "world".
    let f1 = s.execute("edit.findAndReplaceText", json!({"find": "world", "action": "find", "caseSensitive": true})).unwrap();
    assert_eq!(f1["found"]["text"], "world");
    let f2 = s.execute("edit.findAndReplaceText", json!({"find": "world", "action": "changeFind", "replace": "Earth", "caseSensitive": true})).unwrap();
    assert!(f2["changed"].is_object());
    assert_eq!(f2["found"]["text"], "world");
    assert_ne!(f2["found"]["layer"], f2["changed"]["layer"]);
    assert!(s.execute("edit.findAndReplaceText", json!({"find": ""})).is_err());
}

#[test]
fn find_and_replace_change_rejects_a_layer_locked_after_the_find() {
    // #4188: the found match's layer gets Lock All before Change; the text must stay.
    let mut s = session(8);
    let id = s.execute("type.create", json!({"text": "abc", "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap();
    // A second, unlocked type layer keeps the command enabled.
    s.execute("type.create", json!({"text": "zzz", "x": 2, "y": 40})).unwrap();
    let found = s.execute("edit.findAndReplaceText", json!({"find": "abc", "action": "find"})).unwrap();
    assert_eq!(found["found"]["layer"].as_u64(), Some(id));
    s.execute("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let result = s.execute("edit.findAndReplaceText", json!({"find": "abc", "replace": "x", "action": "change"}));
    assert!(matches!(&result, Err(EngineError::Other(message)) if message.contains("locked")), "{result:?}");
    let text = match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    };
    assert_eq!(text, "abc");
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn find_in_the_active_layer_finds_nothing_when_it_is_not_type() {
    // #703: `allLayers: false` with a raster layer active leaves nothing to search.
    let mut s = session(8);
    let id = s.execute("type.create", json!({"text": "abc def", "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "raster"})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    for action in ["find", "change", "changeFind"] {
        for forward in [true, false] {
            let r =
                s.execute("edit.findAndReplaceText", json!({"find": "abc", "replace": "x", "allLayers": false, "action": action, "forward": forward})).unwrap();
            assert!(r["found"].is_null() && r["changed"].is_null(), "{action} {forward}: {r}");
        }
    }
    let r = s.execute("edit.findAndReplaceText", json!({"find": "abc", "replace": "x", "allLayers": false})).unwrap();
    assert_eq!(r["count"].as_u64(), Some(0));
    assert_eq!(s.active().unwrap().history.entries().len(), steps);
    // With the type layer active, the same search finds it.
    s.execute("layer.select", json!({"layer": id})).unwrap();
    let r = s.execute("edit.findAndReplaceText", json!({"find": "abc", "allLayers": false, "action": "find"})).unwrap();
    assert_eq!((r["found"]["layer"].as_u64(), r["found"]["text"].as_str()), (Some(id), Some("abc")));
}

#[test]
fn backward_change_find_moves_past_the_replaced_text() {
    // A backward Change/Find whose replacement still matches ("cat" -> "Cat", case-insensitive)
    // must move on to the previous match, not find the replaced text again.
    let mut s = session(8);
    s.execute("type.create", json!({"text": "cat dog cat", "x": 2, "y": 12})).unwrap();
    let p = |action: &str, forward: bool| json!({"find": "cat", "replace": "Cat", "action": action, "forward": forward, "allLayers": false});
    let f = s.execute("edit.findAndReplaceText", p("find", false)).unwrap();
    assert_eq!((f["found"]["start"].as_u64(), f["found"]["end"].as_u64()), (Some(8), Some(11)), "{f}");
    let r = s.execute("edit.findAndReplaceText", p("changeFind", false)).unwrap();
    assert_eq!((r["changed"]["start"].as_u64(), r["changed"]["end"].as_u64()), (Some(8), Some(11)), "{r}");
    assert_eq!((r["found"]["start"].as_u64(), r["found"]["end"].as_u64()), (Some(0), Some(3)), "{r}");

    // Forward is unchanged: replace the first match, then find the next one.
    let mut s = session(8);
    s.execute("type.create", json!({"text": "cat dog cat", "x": 2, "y": 12})).unwrap();
    let f = s.execute("edit.findAndReplaceText", p("find", true)).unwrap();
    assert_eq!(f["found"]["start"].as_u64(), Some(0), "{f}");
    let r = s.execute("edit.findAndReplaceText", p("changeFind", true)).unwrap();
    assert_eq!((r["changed"]["start"].as_u64(), r["changed"]["end"].as_u64()), (Some(0), Some(3)), "{r}");
    assert_eq!((r["found"]["start"].as_u64(), r["found"]["end"].as_u64()), (Some(8), Some(11)), "{r}");
}

#[test]
fn fade_allocation_failure_keeps_document_and_history() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.fade", json!({"opacity": 25}));
        let _ = crate::allocation::capacity::<u8>(0, "reset failpoint");
        assert!(result.is_err());
        assert_eq!(*s.active().unwrap().doc, *before);
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert!(s.is_enabled("edit.fade"));
        s.execute("edit.fade", json!({"opacity": 25})).unwrap();
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, *before);
    }
}

#[test]
fn tool_presets_round_trip_through_export_import() {
    let mut source = Session::new();
    source.execute("tool.presets.new", json!({"name":"2 : 3", "tool":"crop", "options":{"toolOptions":{"crop_ratio":"2:3"}}})).unwrap();
    let exported = source.execute("edit.presets.exportImportPresets", json!({"action":"export"})).unwrap();
    assert_eq!(exported["tools"], 1);
    assert_eq!(exported["data"]["toolPresets"][0]["name"], "2 : 3");
    let mut target = Session::new();
    target.execute("tool.presets.new", json!({"name":"2 : 3", "tool":"crop", "options":{"toolOptions":{"crop_ratio":"1:1"}}})).unwrap();
    for _ in 0..2 {
        let result = target.execute("edit.presets.exportImportPresets", json!({"action":"import", "data":exported["data"]})).unwrap();
        assert_eq!(result["tools"], 1);
    }
    let selected = target.execute("tool.presets.select", json!({"preset":"2 : 3"})).unwrap();
    assert_eq!(selected["options"]["toolOptions"]["crop_ratio"], "2:3");
    assert_eq!(target.presets.tool_presets.iter().filter(|p| p.name == "2 : 3").count(), 1);
    let only_brushes = source.execute("edit.presets.exportImportPresets", json!({"kinds":["brushes"]})).unwrap();
    assert_eq!(only_brushes["tools"], 0);
    let with_builtins = source.execute("edit.presets.exportImportPresets", json!({"kinds":["tools"], "includeBuiltins":true})).unwrap();
    assert_eq!(with_builtins["tools"].as_u64().unwrap() as usize, source.presets.tool_presets.len());
}

#[test]
fn tool_preset_import_is_validated_before_mutation_and_accepts_old_files() {
    let mut s = Session::new();
    let before = s.presets.tool_presets.clone();
    for preset in [
        json!({"name":"", "tool":"crop", "options":{}}),
        json!({"name":"x".repeat(256), "tool":"crop", "options":{}}),
        json!({"name":"Crop", "tool":" ", "options":{}}),
        json!({"name":"Crop", "tool":"crop", "options":[]}),
    ] {
        let result = s.execute(
            "edit.presets.exportImportPresets",
            json!({"action":"import", "data":{"format":PRESET_FORMAT, "toolPresets":[{"name":"Valid", "tool":"crop", "options":{}}, preset]}}),
        );
        assert!(result.is_err());
        assert_eq!(s.presets.tool_presets, before);
    }
    let too_many: Vec<_> = (0..crate::presets::tools::MAX_PRESETS + 1).map(|i| json!({"name":format!("Crop {i}"), "tool":"crop", "options":{}})).collect();
    assert!(s.execute("edit.presets.exportImportPresets", json!({"action":"import", "data":{"format":PRESET_FORMAT, "toolPresets":too_many}})).is_err());
    assert_eq!(s.presets.tool_presets, before);
    let result = s.execute("edit.presets.exportImportPresets", json!({"action":"import", "data":{"format":PRESET_FORMAT, "version":1}})).unwrap();
    assert_eq!(result["tools"], 0);
}
