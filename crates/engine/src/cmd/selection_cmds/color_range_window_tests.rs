use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Size;
use serde_json::json;

fn fixture(depth: SampleType, p3: bool) -> Session {
    let mut s = Session::new();
    let mut doc = Document::with_background("color range", Size::new(64, 48), ColorMode::Rgb, depth, photocraft_doc::Color::WHITE);
    if p3 {
        doc.icc_profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
    } else {
        doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    }
    let bg = doc.layers[0].surface_mut().unwrap();
    bg.fill_rect(Rect::new(0, 0, 64, 48), &[0.92, 0.92, 0.92, 1.0]);
    bg.fill_rect(Rect::new(7, 8, 22, 23), &[0.85, 0.12, 0.08, 1.0]);
    bg.fill_rect(Rect::new(35, 8, 50, 23), &[0.12, 0.72, 0.18, 1.0]);
    bg.fill_rect(Rect::new(7, 29, 22, 44), &[0.12, 0.20, 0.86, 1.0]);
    bg.fill_rect(Rect::new(35, 29, 50, 44), &[0.72, 0.48, 0.31, 1.0]);
    s.add_document(doc, None);
    s
}

fn coverage_at(s: &Session, x: i32, y: i32) -> f32 {
    s.active().and_then(|d| d.doc.selection.as_ref()).map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn full_sampled_reference(s: &Session, params: &Value) -> Vec<f32> {
    let doc = &s.active().unwrap().doc;
    let canvas = doc.bounds();
    let (_, px) = sample_pixels(s, b(params, "sampleAllLayers", true), None).unwrap();
    // Reproduce the previous command: read eyedropper colours from the full composite.
    let samples = |key: &str| -> Vec<sel::RangeSample> {
        params
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|point| {
                let x = point[0].as_f64().unwrap().floor() as usize;
                let y = point[1].as_f64().unwrap().floor() as usize;
                let pixel = px[y * canvas.width() as usize + x];
                sel::RangeSample { color: [pixel[0], pixel[1], pixel[2]], at: Some((x as f32, y as f32)) }
            })
            .collect()
    };
    let query = ColorRangeSamples::with_samples(s, params, canvas, &samples("points"), &samples("subtractPoints")).unwrap();
    let mut mask = query.coverage(&px, canvas.width() as usize, 1.0, [canvas.x0 as f32, canvas.y0 as f32]);
    if b(params, "invert", false) {
        mask.iter_mut().for_each(|v| *v = 1.0 - *v);
    }
    scope_color_range(&mut mask, doc, canvas, mode(params));
    sel::mask_from_surface(sel::combine(doc.selection.as_ref(), &mask, canvas, mode(params)).as_ref(), canvas)
}

#[test]
fn sampled_color_cropped_results_match_full_canvas_for_depths_profiles_and_sample_options() {
    for (depth, p3) in [SampleType::U8, SampleType::U16, SampleType::F32].into_iter().flat_map(|depth| [false, true].map(|p3| (depth, p3))) {
        for params in [
            json!({"points": [[40, 15]], "fuzziness": 12}),
            json!({"points": [[40, 15]], "subtractPoints": [[10, 15]], "order": ["-", "+"], "fuzziness": 18}),
            json!({"points": [[40, 15]], "localized": true, "range": 80, "invert": true}),
            json!({"points": [[10, 15]], "localized": true, "range": 25}),
            json!({"points": [[40, 15]], "sampleAllLayers": false, "fuzziness": 0}),
        ] {
            let mut s = fixture(depth, p3);
            s.execute("select.rect", json!({"x": 3, "y": 5, "width": 22, "height": 25})).unwrap();
            let expected = full_sampled_reference(&s, &params);
            s.execute("select.colorRange", params.clone()).unwrap();
            let actual = sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), s.active().unwrap().doc.bounds());
            assert_eq!(actual, expected, "depth={depth:?} p3={p3} params={params}");
        }
    }
}

#[test]
fn range_presets_are_scoped_and_non_replace_modes_keep_full_canvas_semantics() {
    for preset in ["reds", "yellows", "greens", "cyans", "blues", "magentas", "highlights", "midtones", "shadows", "skinTones"] {
        let mut s = fixture(SampleType::U8, false);
        s.execute("select.rect", json!({"x": 2, "y": 4, "width": 24, "height": 25})).unwrap();
        s.execute("select.colorRange", json!({"select": preset})).unwrap();
        assert_eq!(coverage_at(&s, 42, 15), 0.0, "preset {preset} escaped selection");
    }
    let mut s = fixture(SampleType::U8, false);
    s.execute("select.rect", json!({"x": 2, "y": 4, "width": 24, "height": 25})).unwrap();
    s.execute("select.colorRange", json!({"points": [[40, 15]], "fuzziness": 0, "mode": "add"})).unwrap();
    assert_eq!(coverage_at(&s, 42, 15), 1.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(coverage_at(&s, 42, 15), 0.0);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(coverage_at(&s, 42, 15), 1.0);
}

#[test]
fn window_defaults_and_hostile_tiles_fall_back_and_empty_selection_is_safe() {
    let canvas = Rect::new(0, 0, 64, 48);
    let mut doc = Document::new("window", Size::new(64, 48), ColorMode::Rgb, SampleType::U8);
    doc.selection = Some(Surface::with_default(PixelFormat::GRAY8, &[1.0]));
    assert_eq!(color_range_area(&doc, SelectionMode::Replace), canvas);
    doc.selection = Some(Surface::new(PixelFormat::GRAY8));
    assert_eq!(color_range_area(&doc, SelectionMode::Replace), Rect::new(0, 0, 0, 0));
    use photocraft_geom::TileCoord;
    use std::sync::Arc;
    let mut hostile = Surface::new(PixelFormat::GRAY8);
    let tile = hostile.solid_tile(&[1.0]);
    hostile.put_tiles([(TileCoord::new(i32::MIN, i32::MIN), Arc::clone(&tile)), (TileCoord::new(i32::MAX, i32::MAX), tile)]);
    doc.selection = Some(hostile);
    assert_eq!(color_range_area(&doc, SelectionMode::Replace), canvas);
    assert_eq!(color_range_area(&doc, SelectionMode::Add), canvas);
    doc.selection = Some(Surface::new(PixelFormat::GRAY8));
    let mut s = Session::new();
    s.add_document(doc, None);
    assert!(sample_pixels(&s, true, Some(color_range_area(&s.active().unwrap().doc, SelectionMode::Replace))).unwrap().1.is_empty());
    let err = s.execute("select.colorRange", json!({"select": "shadows", "tonalRange": -1})).unwrap_err();
    assert!(err.to_string().contains("tonalRange"));
    for params in [json!({"points": [[64, 0]]}), json!({"points": "invalid"}), json!({"localized": true}), json!({"order": ["+"]})] {
        assert!(s.execute("select.colorRange", params).is_err());
    }
    s.execute("select.colorRange", json!({"points": [[1, 1]], "invert": true})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn sparse_large_selection_samples_only_its_clipped_window() {
    let mut doc = Document::new("sparse", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    let selection = sel::mask_to_surface(&vec![1.0; 24 * 20], Rect::new(2990, 1990, 3014, 2010));
    doc.selection = Some(selection);
    let mut s = Session::new();
    s.add_document(doc, None);
    let win = color_range_area(&s.active().unwrap().doc, SelectionMode::Replace);
    assert_eq!(win, Rect::new(2990, 1990, 3014, 2010));
    let (area, pixels) = sample_pixels(&s, true, Some(win)).unwrap();
    assert_eq!(pixels.len(), area.size().area() as usize);
    s.execute("select.colorRange", json!({"select": "highlights"})).unwrap();
}

#[test]
fn cropped_range_matches_full_canvas_combinations_with_soft_holes_and_clipped_masks() {
    let canvas = Rect::new(0, 0, 64, 48);
    for case in 0..6 {
        for selection_mode in ["replace", "add", "subtract", "intersect"] {
            for preset in ["sampledColors", "reds", "greens", "blues", "highlights", "midtones", "shadows", "skinTones"] {
                for invert in [false, true] {
                    let mut s = fixture(SampleType::U16, true);
                    let mut selection = match case {
                        0 => Surface::new(PixelFormat::GRAY8),
                        1 => Surface::with_default(PixelFormat::GRAY8, &[128.0 / 255.0]),
                        _ => sel::mask_to_surface(&vec![128.0 / 255.0; 30 * 30], Rect::new(-5, 5, 25, 35)),
                    };
                    if case == 3 {
                        selection.fill_rect(Rect::new(8, 10, 15, 20), &[0.0]);
                    }
                    if case == 5 {
                        selection = Surface::new(PixelFormat::GRAY8);
                    }
                    if case == 4 {
                        selection = sel::mask_to_surface(&[1.0; 4], Rect::new(-8, -8, -6, -6));
                    }
                    let original = (case != 0).then_some(selection);
                    let mut prior = s.active().unwrap().doc.as_ref().clone();
                    prior.selection = original;
                    let before = sel::mask_from_surface(prior.selection.as_ref(), canvas);
                    let mut unselected = prior.clone();
                    unselected.selection = None;
                    let mut full = Session::new();
                    full.add_document(unselected, None);
                    let params = json!({"select": preset, "points": [[40, 15]], "fuzziness": 40, "invert": invert});
                    full.execute("select.colorRange", params.clone()).unwrap();
                    let mut mask = sel::mask_from_surface(full.active().unwrap().doc.selection.as_ref(), canvas);
                    let m = SelectionMode::parse(selection_mode);
                    scope_color_range(&mut mask, &prior, canvas, m);
                    let expected = sel::combine(prior.selection.as_ref(), &mask, canvas, m);
                    s.edit("prior selection", |doc, _| {
                        doc.selection = prior.selection.clone();
                        Ok(())
                    })
                    .unwrap();
                    let mut params = params;
                    params["mode"] = json!(selection_mode);
                    s.execute("select.colorRange", params).unwrap();
                    let actual = sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), canvas);
                    assert_eq!(actual, sel::mask_from_surface(expected.as_ref(), canvas), "case {case}, {selection_mode}, {preset}, invert {invert}");
                    s.execute("edit.undo", json!({})).unwrap();
                    assert_eq!(sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), canvas), before);
                    s.execute("edit.redo", json!({})).unwrap();
                    assert_eq!(sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), canvas), actual);
                }
            }
        }
    }
}

#[test]
fn cropped_samples_match_full_composite_with_translucent_layers_and_active_layer_sampling() {
    use photocraft_doc::{Layer, LayerContent};
    for all_layers in [false, true] {
        let mut s = fixture(SampleType::F32, true);
        s.edit("overlay", |doc, _| {
            let mut pixels = Surface::new(PixelFormat::RGBA32F);
            pixels.fill_rect(Rect::new(0, 0, 64, 48), &[0.3, 0.1, 0.6, 0.35]);
            doc.layers.push(Layer::new("overlay", LayerContent::Raster(pixels)));
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 5, "y": 6, "width": 20, "height": 30})).unwrap();
        let params = json!({"points": [[40, 15], [10, 15]], "subtractPoints": [[40, 35]], "sampleAllLayers": all_layers, "localized": true, "range": 70});
        let expected = full_sampled_reference(&s, &params);
        s.execute("select.colorRange", params).unwrap();
        assert_eq!(sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), s.active().unwrap().doc.bounds()), expected);
    }
}
