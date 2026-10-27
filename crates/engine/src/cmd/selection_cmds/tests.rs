use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(Rect::new(5, 5, 15, 15), &[1.0, 0.0, 0.0, 1.0]);
        bg.fill_rect(Rect::new(25, 5, 35, 15), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn coverage(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn clear_selection(s: &mut Session) {
    if s.active().unwrap().doc.selection.is_some() {
        s.execute("select.deselect", json!({})).unwrap();
    }
}

#[test]
fn color_range_skin_tones_selects_skin_not_cream_and_refuses_detect_faces() {
    let mut s = session();
    let rgb = |c: [u8; 3]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0];
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(Rect::new(0, 0, 40, 30), &rgb([252, 253, 247]));
        bg.fill_rect(Rect::new(5, 5, 15, 15), &rgb([183, 119, 81]));
        bg.fill_rect(Rect::new(25, 5, 35, 15), &rgb([231, 225, 202]));
        Ok(())
    })
    .unwrap();
    s.execute("select.colorRange", json!({"select": "skinTones"})).unwrap();
    assert!(coverage(&s, 10, 10) > 0.95, "skin");
    assert!(coverage(&s, 30, 10) < 0.05, "cream");
    assert!(coverage(&s, 2, 2) < 0.05, "near-white background");
    // Subtracting skin from a selection of both squares keeps only the cream one.
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 30, "height": 10})).unwrap();
    s.execute("select.colorRange", json!({"select": "skinTones", "mode": "subtract"})).unwrap();
    assert!(coverage(&s, 10, 10) < 0.05 && coverage(&s, 30, 10) > 0.95, "{} {}", coverage(&s, 10, 10), coverage(&s, 30, 10));
    let err = s.execute("select.colorRange", json!({"select": "skinTones", "detectFaces": true})).unwrap_err();
    assert!(err.to_string().contains("detectFaces"), "{err}");
}

#[test]
fn selection_sampling_refuses_oversized_canvases_before_reading_pixels() {
    let mut s = Session::new();
    s.add_document(
        photocraft_doc::Document::new(
            "large",
            photocraft_geom::Size::new(300_000, 300_000),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        ),
        None,
    );
    for all_layers in [false, true] {
        assert!(sample_pixels(&s, all_layers).unwrap_err().to_string().contains("too large to sample"));
    }
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn selection_sampling_allocation_failure_keeps_the_selection() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 4, "height": 4})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    crate::allocation::fail_next_for_test();
    let result = s.execute("select.similar", json!({"sampleAllLayers": false}));
    let _ = crate::allocation::capacity::<u8>(0, "reset failpoint");
    assert!(result.is_err());
    assert_eq!(*s.active().unwrap().doc, *before);
    assert_eq!(s.active().unwrap().history.entries(), history);
}

#[test]
fn modify_rejects_oversized_canvases_before_reading_the_mask() {
    let mut s = Session::new();
    let mut doc = photocraft_doc::Document::new(
        "large",
        photocraft_geom::Size::new(300_000, 300_000),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U8,
    );
    let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
    selection.write_pixel(0, 0, &[1.0]);
    doc.selection = Some(selection);
    s.add_document(doc, None);
    let before = s.active().unwrap().doc.clone();
    for op in ["expand", "contract", "border", "smooth", "feather"] {
        let error = s.execute(&format!("select.modify.{op}"), json!({"radius": 1})).unwrap_err();
        assert!(error.to_string().contains("too large to modify"));
        assert_eq!(*s.active().unwrap().doc, *before);
        assert!(!s.active().unwrap().history.can_undo());
    }
}

#[test]
fn color_range_replace_is_bounded_by_existing_selection() {
    for params in [json!({"color": "#ff0000", "fuzziness": 0}), json!({"select": "reds"})] {
        let mut s = session();
        // Both red squares match, but only the left is inside the original selection.
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        s.execute("select.colorRange", params).unwrap();
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert_eq!(coverage(&s, 30, 10), 0.0);
        assert_eq!(coverage(&s, 2, 2), 0.0);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert_eq!(coverage(&s, 30, 10), 0.0);
    }
}

#[test]
fn color_range_invert_and_partial_masks_stay_within_prior_selection() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    s.execute("select.colorRange", json!({"select": "highlights", "invert": true})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0, "inverted white excludes the white background");
    assert_eq!(coverage(&s, 20, 20), 0.0, "invert must not select outside the old boundary");

    let mut s = session();
    s.edit("soft selection", |doc, _| {
        let mut mask = vec![0.0; 40 * 30];
        mask[10 * 40 + 10] = 128.0 / 255.0;
        doc.selection = Some(sel::mask_to_surface(&mask, doc.bounds()));
        Ok(())
    })
    .unwrap();
    s.execute("select.colorRange", json!({"select": "reds"})).unwrap();
    assert!((coverage(&s, 10, 10) - 128.0 / 255.0).abs() < 0.001);
    assert_eq!(coverage(&s, 30, 10), 0.0);
}

#[test]
fn color_range_eyedropper_samples_anywhere_but_the_result_stays_in_the_selection() {
    // #5244: as in Photoshop, a colour picked outside the active selection is accepted; only
    // the result is limited to the selection.
    let mut s = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    let outside = json!({"points": [[30, 10]], "fuzziness": 0});
    assert!(ColorRangeSamples::new(&s, &outside).is_ok());
    s.execute("select.colorRange", outside).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0, "the red inside the selection is selected");
    assert_eq!(coverage(&s, 30, 10), 0.0, "the sampled red outside it stays out");
    assert_eq!(coverage(&s, 2, 2), 0.0, "white is not the sampled colour");
}

#[test]
fn color_range_eyedropper_respects_selection_but_add_can_extend_it() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    s.execute("select.colorRange", json!({"points": [[10, 10]], "fuzziness": 0})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert_eq!(coverage(&s, 30, 10), 0.0);

    // Explicitly adding a matching colour is still allowed to grow the selection.
    s.execute("select.colorRange", json!({"points": [[30, 10]], "fuzziness": 0, "mode": "add"})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert_eq!(coverage(&s, 30, 10), 1.0);

    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"color": "#ff0000", "fuzziness": 0})).unwrap();
    assert_eq!(coverage(&s, 30, 10), 1.0, "an unbounded color range still searches the canvas");
}

#[test]
fn magic_wand_selects_the_visible_alpha_or_mask_not_the_rgb() {
    let mut s = session();
    // Alpha covers the first red square and the pixels beside it. RGB would stop at the red.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("channel.new", json!({"fill": "selection", "name": "Alpha"})).unwrap();
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "tolerance": 0, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 7, 7), 1.0);
    assert_eq!(coverage(&s, 2, 2), 1.0, "the white alpha includes pixels that are not the red square");
    assert_eq!(coverage(&s, 30, 10), 0.0, "outside the alpha stays unselected");
    // Colour channels back on: the wand reads RGB again.
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": true})).unwrap();
    s.execute("channel.setVisible", json!({"channel": 0, "visible": false})).unwrap();
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "tolerance": 0, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 7, 7), 1.0);
    assert_eq!(coverage(&s, 2, 2), 0.0, "RGB sampling stops at the red square");
    // Eyes off again, Alpha still the picture.
    s.execute("channel.setVisible", json!({"channel": 0, "visible": true})).unwrap();
    s.execute("channel.setVisible", json!({"channel": "composite", "visible": false})).unwrap();
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "tolerance": 0, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 1.0);

    let mut s = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s.execute("view.layerMask", json!({"mode": "gray"})).unwrap();
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "tolerance": 0, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 1.0, "gray mask view selects the mask");
    assert_eq!(coverage(&s, 30, 10), 0.0);
    s.execute("view.layerMask", json!({"mode": "off"})).unwrap();
    for name in ["red", "green", "blue"] {
        s.execute("channel.setVisible", json!({"channel": name, "visible": false})).unwrap();
    }
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "tolerance": 0, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 1.0, "a layer mask shown alone is what the wand reads");
    assert_eq!(coverage(&s, 30, 10), 0.0);
}

#[test]
fn magic_wand_contiguous_and_modes() {
    let mut s = session();
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert_eq!(coverage(&s, 30, 10), 0.0);
    s.execute("select.magicWand", json!({"x": 30, "y": 7, "mode": "add", "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 30, 10), 1.0);
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "mode": "subtract", "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 0.0);
    s.execute("select.magicWand", json!({"x": 7, "y": 7, "contiguous": false, "antiAlias": false})).unwrap();
    assert_eq!(coverage(&s, 30, 10), 1.0);
}

#[test]
fn color_range_prepared_query_matches_the_command_at_all_depths() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for profile in [None, Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes())] {
            let mut doc = Document::with_background("samples", Size::new(24, 12), ColorMode::Rgb, depth, Color::WHITE);
            doc.icc_profile = profile;
            let layer = doc.layers[0].surface_mut().unwrap();
            layer.fill_rect(Rect::new(0, 0, 8, 12), &[0.65, 0.3, 0.2, 1.0]);
            layer.fill_rect(Rect::new(8, 0, 16, 12), &[0.2, 0.3, 0.65, 1.0]);
            let mut s = Session::new();
            s.add_document(doc, None);
            for p in [
                json!({"points": [[2, 3]], "fuzziness": 0}),
                json!({"points": [[2, 3], [10, 3]], "subtractPoints": [[10, 3]], "order": ["+", "+", "-"], "fuzziness": 40}),
                json!({"points": [[2, 3]], "localized": true, "range": 50, "fuzziness": 80}),
            ] {
                clear_selection(&mut s);
                let q = ColorRangeSamples::new(&s, &p).unwrap();
                let doc = s.active().unwrap().doc.clone();
                let pixels = photocraft_compose::render(&doc, doc.bounds());
                let preview = q.coverage(&pixels.px, 24, 1.0, [0.0, 0.0]);
                s.execute("select.colorRange", p).unwrap();
                let actual = current_mask(&s.active().unwrap().doc).1;
                for (shown, selected) in preview.iter().zip(actual) {
                    let rounded = (shown.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0;
                    assert_eq!(rounded, selected, "{depth:?}: original-pixel preview and OK differ");
                }
            }
        }
    }
}

#[test]
fn color_range_prepared_query_samples_visible_layers_not_the_active_layer() {
    use photocraft_doc::{Layer, LayerContent};
    let mut s = session_colours();
    let doc = s.active().unwrap().doc.clone();
    let mut top = Layer::new("blue foreground", LayerContent::Raster(doc.layers[0].surface().unwrap().clone()));
    top.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 0.0, 1.0, 1.0]);
    let mut hidden = Layer::new("hidden green", LayerContent::Raster(top.surface().unwrap().clone()));
    hidden.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 1.0, 0.0, 1.0]);
    hidden.visible = false;
    s.edit("layers", |doc, _| {
        doc.layers.push(top);
        doc.layers.push(hidden);
        Ok(())
    })
    .unwrap();
    for all_layers in [false, true] {
        clear_selection(&mut s);
        let p = json!({"points": [[7, 7]], "sampleAllLayers": all_layers, "fuzziness": 0});
        let query = ColorRangeSamples::new(&s, &p).unwrap();
        let (area, px) = sample_pixels(&s, all_layers).unwrap();
        let shown = query.coverage(&px, area.width() as usize, 1.0, [0.0, 0.0]);
        s.execute("select.colorRange", p).unwrap();
        assert_eq!(shown[7 * 40 + 7], coverage(&s, 7, 7));
        assert_eq!(shown[24 * 40 + 10], coverage(&s, 10, 24));
    }
    for p in [json!({"points": [[-1, 0]]}), json!({"points": [[40, 1]]}), json!({"points": "wrong"}), json!({"points": [[null, 0]]})] {
        assert!(ColorRangeSamples::new(&s, &p).is_err(), "{p}");
    }
}

#[test]
fn color_range_selects_by_colour() {
    let mut s = session();
    s.execute("select.colorRange", json!({"color": "#ff0000", "fuzziness": 40})).unwrap();
    // The sampled colour itself is fully selected (as with Photoshop's eyedropper).
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert_eq!(coverage(&s, 20, 20), 0.0);
    s.execute("select.colorRange", json!({"color": "#ff0000", "fuzziness": 100})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert_eq!(coverage(&s, 20, 20), 0.0);
}

#[test]
fn color_range_compares_in_lab_like_photoshop() {
    // Photoshop weighs hue (Lab a/b) three times as much as lightness; a per-channel RGB
    // distance can't tell a lighter shade from a redder one.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 30, "height": 10})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let v = |c: u8| f32::from(c) / 255.0;
        bg.fill_rect(Rect::new(0, 0, 10, 10), &[v(200), v(140), v(120), 1.0]); // skin
        bg.fill_rect(Rect::new(10, 0, 20, 10), &[v(220), v(160), v(140), 1.0]); // lighter skin
        bg.fill_rect(Rect::new(20, 0, 30, 10), &[v(220), v(140), v(120), 1.0]); // redder skin
        Ok(())
    })
    .unwrap();
    s.execute("select.colorRange", json!({"points": [[5, 5]], "fuzziness": 40})).unwrap();
    assert!(coverage(&s, 5, 5) > 0.98);
    // Both differ from the sample by 20 levels in one RGB channel, yet the lighter one is
    // kept clearly more (about 0.55 vs 0.31).
    let (lighter, redder) = (coverage(&s, 15, 5), coverage(&s, 25, 5));
    assert!(lighter > redder + 0.15 && redder > 0.0, "lighter {lighter}, redder {redder}");
}

#[test]
fn color_range_fuzziness_zero_selects_exact_match_only() {
    // Sampled Colors at Fuzziness 0 selects only the exact match (TOOL-214-3, #214).
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 50, "height": 10})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let v = |c: u8| f32::from(c) / 255.0;
        // Exact target color [200, 140, 120]
        bg.fill_rect(Rect::new(0, 0, 10, 10), &[v(200), v(140), v(120), 1.0]);
        // Off by 1 level in red [201, 140, 120]
        bg.fill_rect(Rect::new(10, 0, 20, 10), &[v(201), v(140), v(120), 1.0]);
        // Off by 1 level in green [200, 141, 120]
        bg.fill_rect(Rect::new(20, 0, 30, 10), &[v(200), v(141), v(120), 1.0]);
        // Off by 2 levels in blue [200, 140, 122] (crosses 8-bit Lab grid threshold)
        bg.fill_rect(Rect::new(30, 0, 40, 10), &[v(200), v(140), v(122), 1.0]);
        // Exact target color again at a different position
        bg.fill_rect(Rect::new(40, 0, 50, 10), &[v(200), v(140), v(120), 1.0]);
        Ok(())
    })
    .unwrap();

    // Sampled via eyedropper point at [5, 5] with Fuzziness 0
    s.execute("select.colorRange", json!({"points": [[5, 5]], "fuzziness": 0})).unwrap();
    assert_eq!(coverage(&s, 5, 5), 1.0, "sampled pixel itself is fully selected");
    assert_eq!(coverage(&s, 45, 5), 1.0, "identical color elsewhere on canvas is fully selected");
    assert_eq!(coverage(&s, 15, 5), 0.0, "1 level off in red is unselected at fuzziness 0");
    assert_eq!(coverage(&s, 25, 5), 0.0, "1 level off in green is unselected at fuzziness 0");
    assert_eq!(coverage(&s, 35, 5), 0.0, "1 level off in blue is unselected at fuzziness 0");

    // Sampled via hex color with Fuzziness 0
    s.execute("select.colorRange", json!({"color": "#c88c78", "fuzziness": 0})).unwrap();
    assert_eq!(coverage(&s, 5, 5), 1.0, "exact color match from hex is fully selected");
    assert_eq!(coverage(&s, 45, 5), 1.0, "identical color elsewhere on canvas is fully selected");
    assert_eq!(coverage(&s, 15, 5), 0.0, "1 level off in red is unselected for hex at fuzziness 0");
    assert_eq!(coverage(&s, 25, 5), 0.0, "1 level off in green is unselected for hex at fuzziness 0");
    assert_eq!(coverage(&s, 35, 5), 0.0, "1 level off in blue is unselected for hex at fuzziness 0");
}

/// Red squares (from `session`), a blue square, a black and a mid-grey strip on white.
fn session_colours() -> Session {
    let mut s = session();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(Rect::new(5, 20, 15, 28), &[0.0, 0.0, 1.0, 1.0]);
        bg.fill_rect(Rect::new(20, 20, 25, 28), &[0.0, 0.0, 0.0, 1.0]);
        bg.fill_rect(Rect::new(30, 20, 35, 28), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn color_range_several_samples_points_and_invert() {
    let mut s = session_colours();
    // Several samples span one Lab box (Photoshop's Minimum/Maximum): red and blue take
    // both squares, black, grey and white stay out.
    s.execute("select.colorRange", json!({"colors": ["#ff0000", "#0000ff"], "fuzziness": 100})).unwrap();
    assert_eq!((coverage(&s, 10, 10), coverage(&s, 30, 10), coverage(&s, 10, 24)), (1.0, 1.0, 1.0));
    assert_eq!((coverage(&s, 2, 2), coverage(&s, 22, 24), coverage(&s, 32, 24)), (0.0, 0.0, 0.0));
    // Eyedropper point on the blue square; `invert` flips the result.
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"points": [[7, 22]], "fuzziness": 100, "invert": true})).unwrap();
    assert_eq!((coverage(&s, 10, 24), coverage(&s, 10, 10), coverage(&s, 2, 2)), (0.0, 1.0, 1.0));
    // Localized clusters: picked on the left red square, a 25% range (10 px) leaves the right one.
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"points": [[10, 10]], "fuzziness": 100, "localized": true, "range": 25})).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0);
    assert!(coverage(&s, 13, 10) > 0.5);
    assert_eq!(coverage(&s, 30, 10), 0.0);
    // Undo restores the empty selection that preceded the most recent Color Range.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 0.0);
}

#[test]
fn color_range_subtract_follows_photoshop_and_the_click_order() {
    // Grey strips from dark to light; samples at both ends span the whole run.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 50, "height": 10})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        for (i, v) in [0.30f32, 0.40, 0.50, 0.60, 0.70].iter().enumerate() {
            bg.fill_rect(Rect::new(i as i32 * 10, 0, i as i32 * 10 + 10, 10), &[*v, *v, *v, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    let run = |s: &mut Session, extra: Value| {
        clear_selection(s);
        let mut p = json!({"points": [[5, 5], [45, 5]], "fuzziness": 10});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("select.colorRange", p).unwrap();
        (0..5).map(|i| coverage(s, i * 10 + 5, 5)).collect::<Vec<f32>>()
    };
    assert!(run(&mut s, json!({})).iter().all(|v| *v > 0.99), "both samples span every strip");
    // Subtracting a colour inside the range moves the nearer edge onto it: the strips beyond
    // it drop out, the subtracted grey itself stays selected (Photoshop does the same).
    let cut = run(&mut s, json!({"subtractPoints": [[15, 5]]}));
    assert!(cut[0] < 0.5 && cut[1] > 0.99 && cut[4] > 0.99, "{cut:?}");
    // Order matters: adding the dark strip back after the subtraction restores it.
    let readd = run(&mut s, json!({"points": [[5, 5], [45, 5], [5, 5]], "subtractPoints": [[15, 5]], "order": ["+", "+", "-", "+"]}));
    assert!(readd[0] > 0.99, "{readd:?}");
    let sub_last = run(&mut s, json!({"points": [[5, 5], [45, 5], [5, 5]], "subtractPoints": [[15, 5]], "order": ["+", "+", "+", "-"]}));
    assert!(sub_last[0] < 0.5, "{sub_last:?}");
    // A malformed or inconsistent order is an error, never a panic.
    clear_selection(&mut s);
    for bad in [json!(["+"]), json!(["+", "+", "x"]), json!("+-"), json!(["-", "-", "+"])] {
        assert!(s.execute("select.colorRange", json!({"points": [[5, 5], [45, 5]], "subtractPoints": [[15, 5]], "order": bad})).is_err(), "{bad}");
    }
}

#[test]
fn color_range_presets() {
    let mut s = session_colours();
    s.execute("select.colorRange", json!({"select": "reds"})).unwrap();
    assert_eq!((coverage(&s, 10, 10), coverage(&s, 30, 10)), (1.0, 1.0));
    assert_eq!((coverage(&s, 10, 24), coverage(&s, 2, 2), coverage(&s, 32, 24)), (0.0, 0.0, 0.0));
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "blues"})).unwrap();
    assert_eq!((coverage(&s, 10, 24), coverage(&s, 10, 10)), (1.0, 0.0));
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "greens"})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none() || coverage(&s, 10, 10) == 0.0);
    // Tones: black is a shadow, white a highlight, 50% grey (128) a midtone.
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "shadows"})).unwrap();
    assert_eq!((coverage(&s, 22, 24), coverage(&s, 2, 2), coverage(&s, 32, 24)), (1.0, 0.0, 0.0));
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "highlights"})).unwrap();
    assert_eq!((coverage(&s, 22, 24), coverage(&s, 2, 2)), (0.0, 1.0));
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "midtones", "fuzziness": 0})).unwrap();
    assert_eq!((coverage(&s, 32, 24), coverage(&s, 2, 2), coverage(&s, 22, 24)), (1.0, 0.0, 0.0));
    // A wider shadow range takes the grey too.
    clear_selection(&mut s);
    s.execute("select.colorRange", json!({"select": "shadows", "tonalRange": 140, "fuzziness": 0})).unwrap();
    assert_eq!(coverage(&s, 32, 24), 1.0);
    // Modes combine as for the other selection commands.
    s.execute("select.colorRange", json!({"select": "reds", "mode": "add"})).unwrap();
    assert_eq!((coverage(&s, 32, 24), coverage(&s, 10, 10)), (1.0, 1.0));
}

#[test]
fn color_range_out_of_gamut_matches_the_gamut_warning() {
    let mut s = session();
    s.edit("paint", |doc, _| {
        // Saturated sRGB blue is outside the default CMYK proof; mid grey is inside.
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(Rect::new(0, 20, 20, 30), &[0.0, 0.0, 1.0, 1.0]);
        bg.fill_rect(Rect::new(20, 20, 40, 30), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.colorRange", json!({"select": "outOfGamut"})).unwrap();
    assert_eq!((coverage(&s, 5, 25), coverage(&s, 30, 25)), (1.0, 0.0));
    let warn = s.execute("view.gamutWarning", json!({"on": true})).unwrap();
    let sel = s.active().unwrap().doc.selection.clone().unwrap();
    let mut n = 0;
    for y in 0..30 {
        for x in 0..40 {
            n += usize::from(sel.sample_channel(x, y, 0) > 0.5);
        }
    }
    assert_eq!(warn["outOfGamut"].as_u64(), Some(n as u64));
}

#[test]
fn color_range_rejects_bad_params() {
    let mut s = session_colours();
    for p in [
        json!({"select": "skin"}),
        json!({"select": 3}),
        json!({"points": [[100, 2]]}),
        json!({"points": [[1]]}),
        json!({"points": "here"}),
        json!({"colors": "#ff0000"}),
        json!({"localized": true}),
        json!({"select": "midtones", "tonalRange": [200, 100]}),
        json!({"select": "shadows", "tonalRange": 300}),
    ] {
        assert!(s.execute("select.colorRange", p.clone()).is_err(), "{p}");
    }
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn modify_commands() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("select.modify.expand", json!({"radius": 2})).unwrap();
    assert_eq!(coverage(&s, 8, 15), 1.0);
    s.execute("select.modify.contract", json!({"radius": 4})).unwrap();
    assert_eq!(coverage(&s, 11, 15), 0.0);
    assert_eq!(coverage(&s, 15, 15), 1.0);
    s.execute("select.modify.border", json!({"radius": 2})).unwrap();
    assert_eq!(coverage(&s, 15, 15), 0.0);
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("select.modify.feather", json!({"radius": 4})).unwrap();
    let e = coverage(&s, 10, 15);
    assert!(e > 0.2 && e < 0.8, "{e}");
    s.execute("select.modify.smooth", json!({"radius": 2})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
    assert!(!s.is_enabled("select.nothing"));
    // Out-of-range or non-finite radii are refused and leave the selection alone.
    for op in ["feather", "smooth", "expand", "contract", "border"] {
        for r in [json!(1e300), json!(5000), json!(f64::MAX), json!(-5), json!(-0.5), json!(f64::MIN)] {
            assert!(s.execute(&format!("select.modify.{op}"), json!({"radius": r})).is_err(), "{op} {r}");
        }
    }
    assert!(s.active().unwrap().doc.selection.is_some());
}

#[test]
fn modify_select_all_honours_canvas_bounds() {
    let all = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s
    };
    // Off (Photoshop's default): the selection continues past the canvas, so the edges stay.
    for op in ["contract", "smooth", "expand", "feather"] {
        let mut s = all();
        s.execute(&format!("select.modify.{op}"), json!({"radius": 4})).unwrap();
        assert_eq!(coverage(&s, 0, 15), 1.0, "{op}");
        assert_eq!(coverage(&s, 20, 0), 1.0, "{op}");
    }
    // On: the canvas edge is a selection edge (#1277).
    let mut s = all();
    s.execute("select.modify.contract", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
    assert_eq!(coverage(&s, 2, 15), 0.0);
    assert_eq!(coverage(&s, 20, 28), 0.0);
    assert_eq!(coverage(&s, 20, 15), 1.0);
    let mut s = all();
    s.execute("select.modify.feather", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
    let e = coverage(&s, 0, 15);
    assert!(e > 0.2 && e < 0.8, "{e}");
    assert_eq!(coverage(&s, 20, 15), 1.0);
    let mut s = all();
    s.execute("select.modify.smooth", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
    assert!(coverage(&s, 0, 0) < 1.0);
    // Border always follows the canvas edge: a band inside it, not an empty selection.
    let mut s = all();
    s.execute("select.modify.border", json!({"radius": 4})).unwrap();
    assert!(coverage(&s, 0, 15) > 0.0);
    assert_eq!(coverage(&s, 20, 15), 0.0);
}

#[test]
fn modify_away_from_the_canvas_edge_ignores_canvas_bounds() {
    for at in [false, true] {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("select.modify.contract", json!({"radius": 4, "applyAtCanvasBounds": at})).unwrap();
        assert_eq!(coverage(&s, 11, 15), 0.0);
        assert_eq!(coverage(&s, 15, 15), 1.0);
    }
}

#[test]
fn modify_refuses_radii_below_the_minimum() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    // Below each dialog's minimum (1 px, Feather 0.1 px) is refused, not clamped to 0: a
    // negative Border radius used to clear the selection and report success (#993).
    let below = [
        ("border", -5.0),
        ("border", 0.5),
        ("smooth", -5.0),
        ("smooth", 0.0),
        ("expand", -5.0),
        ("expand", 0.0),
        ("contract", -5.0),
        ("contract", 0.9),
        ("feather", -5.0),
        ("feather", 0.0),
        ("feather", 0.05),
    ];
    let before = current_mask(&s.active().unwrap().doc).1;
    for (op, r) in below {
        let err = s.execute(&format!("select.modify.{op}"), json!({"radius": r})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{op} {r}: {err}");
        assert!(current_mask(&s.active().unwrap().doc).1 == before, "{op} {r}: the selection changed");
    }
    // The minimum itself is still accepted.
    s.execute("select.modify.feather", json!({"radius": 0.1})).unwrap();
    s.execute("select.modify.expand", json!({"radius": 1})).unwrap();
    assert_eq!(coverage(&s, 9, 15), 1.0);
}

#[test]
fn geometry_params_that_would_wrap_are_rejected() {
    let mut s = session();
    // 2^32 + 50 wrapped to `x = 50` and 3e9 to a negative coordinate through `as i32`;
    // both are bad-params errors now, whatever the front door (UI, CLI, control, MCP).
    for x in [4_294_967_346_i64, 3_000_000_000_i64] {
        let err = s.execute("select.rect", json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap_err();
        assert!(err.to_string().contains("32-bit"), "{err}");
    }
    let err = s.execute("document.pixel", json!({"x": 4_294_967_346_i64, "y": 0})).unwrap_err();
    assert!(err.to_string().contains("32 bits"), "{err}");
    // In-range coordinates, including negative and past-canvas ones, are unaffected.
    s.execute("select.rect", json!({"x": -100, "y": -100, "width": 500, "height": 500})).unwrap();
    s.execute("document.pixel", json!({"x": 0, "y": 0})).unwrap();
}

#[test]
fn grow_and_similar() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 6, "y": 6, "width": 2, "height": 2})).unwrap();
    s.execute("select.grow", json!({"tolerance": 5})).unwrap();
    assert_eq!(coverage(&s, 14, 14), 1.0);
    assert_eq!(coverage(&s, 30, 10), 0.0);
    s.execute("select.similar", json!({"tolerance": 5})).unwrap();
    assert_eq!(coverage(&s, 30, 10), 1.0);
    assert_eq!(coverage(&s, 20, 20), 0.0);
}

#[test]
fn lasso_polygon() {
    let mut s = session();
    s.execute("select.lasso", json!({"points": [[0, 0], [20, 0], [0, 20]]})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 1.0);
    assert_eq!(coverage(&s, 18, 18), 0.0);
    assert!(s.execute("select.lasso", json!({"points": [[0, 0], [1, 1]]})).is_err());
    s.execute("select.lasso", json!({"points": [[0, 0], [20, 0], [0, 20]], "mode": "subtract"})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(coverage(&s, 2, 2), 1.0);
}

/// The previous per-sample padding and cropping, kept as the oracle for the row-wise ones.
fn old_pad_mask(m: &[f32], w: usize, h: usize, pad: usize, outside_empty: bool) -> Vec<f32> {
    let pw = w + 2 * pad;
    let mut out = vec![0.0f32; pw * (h + 2 * pad)];
    for (py, row) in out.chunks_exact_mut(pw).enumerate() {
        let inside_y = (pad..pad + h).contains(&py);
        if outside_empty && !inside_y {
            continue;
        }
        let y = py.saturating_sub(pad).min(h - 1);
        for (px, v) in row.iter_mut().enumerate() {
            if outside_empty && !(pad..pad + w).contains(&px) {
                continue;
            }
            *v = m.get(y * w + px.saturating_sub(pad).min(w - 1)).copied().unwrap_or(0.0);
        }
    }
    out
}

#[test]
fn row_wise_pad_and_crop_match_per_sample_ones() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let (w, h) = (37, 23);
    let m: Vec<f32> = (0..w * h)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f32 / 999.0
        })
        .collect();
    for pad in [0, 1, 5, 40] {
        for outside_empty in [false, true] {
            let padded = pad_mask(&m, w, h, pad, outside_empty);
            assert_eq!(padded, old_pad_mask(&m, w, h, pad, outside_empty), "pad {pad} {outside_empty}");
            assert_eq!(crop_mask(&padded, w + 2 * pad, pad, w, h), m, "pad {pad}");
        }
    }
}

/// The lasso over the polygon's rows gives exactly the whole-canvas result (#4532): random
/// polygons, partly off the canvas, with and without anti-aliasing, in every mode over an
/// existing selection.
#[test]
fn lasso_over_its_rows_matches_the_whole_canvas() {
    let mut seed = 0x2545_f491_u32;
    let mut rnd = |lo: f32, hi: f32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        lo + (seed % 10_000) as f32 / 10_000.0 * (hi - lo)
    };
    for case in 0..24 {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 520})).unwrap();
        s.execute("select.rect", json!({"x": 40, "y": 60, "width": 200, "height": 300})).unwrap();
        let n = 3 + case % 7;
        let pts: Vec<(f32, f32)> = (0..n).map(|_| (rnd(-30.0, 330.0), rnd(100.0, 420.0))).collect();
        let aa = case % 2 == 0;
        let m = ["replace", "add", "subtract", "intersect"][case % 4];
        let doc = s.active().unwrap().doc.clone();
        let area = doc.bounds();
        let want = sel::combine(doc.selection.as_ref(), &sel::polygon(&pts, area, aa), area, SelectionMode::parse(m));
        let points: Vec<[f32; 2]> = pts.iter().map(|p| [p.0, p.1]).collect();
        s.execute("select.lasso", json!({"points": points, "antiAlias": aa, "mode": m})).unwrap();
        let got = s.active().unwrap().doc.selection.clone();
        let read = |x: &Option<Surface>| x.as_ref().map(|s| s.read_region(area));
        assert_eq!(read(&got), read(&want), "case {case}: {m}, anti-alias {aa}, {pts:?}");
    }
}
