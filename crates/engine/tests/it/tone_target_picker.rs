//! Native-depth grayscale samples for destructive Levels/Curves commands.
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent, LayerMask};
use photocraft_engine::{Session, adjust_params, channel_cmds::ChannelTarget};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

/// Build native-depth colour pixels and three independent gray targets with contrasting samples.
fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut doc = Document::new("Target picker", Size::new(32, 32), mode, depth);
    let mut pixels = Surface::new(PixelFormat::new(mode, depth, true));
    let mut values = vec![0.0; mode.color_channels() + 1];
    *values.last_mut().unwrap() = 1.0;
    pixels.fill_rect(doc.bounds(), &values);
    let mut gray = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    gray.fill_rect(doc.bounds(), &[0.25]);
    let mut layer = Layer::new("Pixels", LayerContent::Raster(pixels));
    layer.mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
    let id = layer.id;
    doc.layers.push(layer);
    let mut alpha = AlphaChannel::new("Spot ink", gray.clone());
    alpha.spot = Some((Color::rgb(0.2, 0.4, 0.6), 0.8));
    doc.channels.push(alpha);
    doc.quick_mask = Some(AlphaChannel::new("Quick Mask", gray));
    let mut s = Session::new();
    s.add_document(doc, None);
    s.select_layer(id).unwrap();
    s.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
    s
}

/// Read the explicitly addressed gray plane for before/after pixel comparisons.
fn plane<'a>(doc: &'a Document, target: &Value) -> &'a Surface {
    match target.as_str() {
        Some("mask") => &doc.layers[0].mask.as_ref().unwrap().surface,
        Some("quickMask") => &doc.quick_mask.as_ref().unwrap().surface,
        _ => &doc.channels[0].surface,
    }
}

/// Black/white pickers edit only their gray target across colour modes and native depths, including undo/redo.
#[test]
fn native_gray_pickers_ignore_document_colour_and_preserve_other_data() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for target in [json!("mask"), json!("quickMask"), json!({"channel":0})] {
                for kind in ["levels", "curves"] {
                    for point in ["black", "white"] {
                        let mut s = session(mode, depth);
                        let before = s.active().unwrap().doc.clone();
                        let history = s.active().unwrap().history.past_len();
                        let p = json!({"target":target,"eyedropper":{"point":point,"at":[8,8],"size":3}});
                        s.execute(&format!("image.adjustments.{kind}"), p).unwrap();
                        let after = s.active().unwrap().doc.clone();
                        let expected = if point == "white" { 1.0 } else { 0.0 };
                        let actual = plane(&after, &target).sample_channel(8, 8, 0);
                        assert!((actual - expected).abs() < 0.01, "{mode:?} {depth:?} {target} {kind} {point}: {actual}");
                        assert_eq!(before.layers[0].content, after.layers[0].content, "colour pixels and transparency");
                        if target != json!("mask") {
                            assert_eq!(before.layers[0].mask, after.layers[0].mask);
                        }
                        if target != json!("quickMask") {
                            assert_eq!(before.quick_mask, after.quick_mask);
                        }
                        if target != json!({"channel":0}) {
                            assert_eq!(before.channels, after.channels);
                        }
                        assert_eq!(before.channels[0].spot, after.channels[0].spot);
                        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                        s.execute("edit.undo", json!({})).unwrap();
                        assert_eq!(plane(&s.active().unwrap().doc, &target), plane(&before, &target));
                        s.execute("edit.redo", json!({})).unwrap();
                        assert_eq!(plane(&s.active().unwrap().doc, &target), plane(&after, &target));
                    }
                }
            }
        }
    }
}

/// Picker averaging quantizes in the target format and clips its sample square to the canvas.
#[test]
fn gray_picker_averages_the_plane_at_native_depth_and_clips_at_canvas_edges() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = session(ColorMode::Lab, depth);
        s.edit("Sample pattern", |doc, _| {
            doc.channels[0].surface.fill_rect(Rect::from_xywh(0, 0, 3, 3), &[0.5]);
            doc.channels[0].surface.fill_rect(Rect::from_xywh(0, 0, 1, 1), &[0.0]);
            Ok(())
        })
        .unwrap();
        for (at, expected) in [(json!([1, 1]), 4.0 / 9.0), (json!([0, 0]), 3.0 / 8.0)] {
            let p = json!({"target":{"channel":0},"eyedropper":{"point":"white","at":at,"size":3}});
            let a = adjust_params::destructive_eyedropper(&s, "levels", &p).unwrap().unwrap();
            let values = adjust_params::to_params(&a);
            assert!((values["inWhite"].as_f64().unwrap() / 255.0 - expected).abs() < 0.004, "{depth:?}: {values}");
        }
    }
}

/// Unstored mask values remain sampleable even when the colour layer is fully transparent.
#[test]
fn scalar_picker_reads_unstored_mask_defaults_and_ignores_colour_transparency() {
    let mut s = session(ColorMode::Cmyk, SampleType::F32);
    s.edit("Unstored mask", |doc, _| {
        let bounds = doc.bounds();
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        doc.layers[0].mask.as_mut().unwrap().surface = Surface::with_default(fmt, &[0.6]);
        doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &[0.0; 5]);
        Ok(())
    })
    .unwrap();
    let p = json!({"target":"mask","eyedropper":{"point":"white","at":[8,8],"size":101}});
    let a = adjust_params::destructive_eyedropper(&s, "levels", &p).unwrap().unwrap();
    assert!((adjust_params::to_params(&a)["inWhite"].as_f64().unwrap() - 153.0).abs() < 0.01);
}

/// Moved sparse masks sample stored pixels and defaults at document coordinates, independent of linking.
#[test]
fn offset_sparse_masks_sample_document_coordinates_and_unstored_defaults() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            // Independent native-depth expectations for the stored patch and its default.
            let quantize = |v: f64| match depth {
                SampleType::U8 => (v * 255.0).round() / 255.0,
                SampleType::U16 => (v * 65535.0).round() / 65535.0,
                _ => v,
            };
            for linked in [false, true] {
                for (dx, dy) in [(0, 0), (7, 5), (-12, -14), (52, 34)] {
                    let mut doc = Document::new("Offset mask", Size::new(64, 48), mode, depth);
                    let mut pixels = Surface::new(PixelFormat::new(mode, depth, true));
                    let mut values = vec![0.1; mode.color_channels() + 1];
                    *values.last_mut().unwrap() = 1.0;
                    pixels.fill_rect(Rect::from_xywh(9, 11, 4, 5), &values);
                    let mut mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, depth, false), &[0.6]);
                    mask.fill_rect(Rect::from_xywh(10, 12, 3, 3), &[0.25]);
                    let mut layer = Layer::new("Small offset layer", LayerContent::Raster(pixels));
                    layer.mask = Some(LayerMask { surface: mask, linked, ..LayerMask::reveal_all() });
                    let id = layer.id;
                    doc.layers.push(layer);
                    let mut s = Session::new();
                    s.add_document(doc, None);
                    s.select_layer(id).unwrap();
                    // Actual moves exercise masks that travel with the layer or move alone,
                    // including a stored patch extending beyond either canvas corner.
                    s.execute("layer.translate", json!({"target":"mask","dx":dx,"dy":dy})).unwrap();
                    s.active_mut().unwrap().channel_view.target = ChannelTarget::LayerMask;
                    let patch = Rect::from_xywh(10 + dx, 12 + dy, 3, 3);
                    for (x, y) in [((11 + dx).clamp(0, 63), (13 + dy).clamp(0, 47)), (11, 13), (63, 47)] {
                        for size in [1, 3] {
                            let r = size / 2;
                            let (mut sum, mut count) = (0.0, 0);
                            for yy in (y - r).max(0)..=(y + r).min(47) {
                                for xx in (x - r).max(0)..=(x + r).min(63) {
                                    sum += quantize(if patch.contains(xx, yy) { 0.25 } else { 0.6 });
                                    count += 1;
                                }
                            }
                            let expected = quantize(sum / f64::from(count)) * 255.0;
                            for kind in ["levels", "curves"] {
                                let mut p = json!({"eyedropper":{"point":"white","at":[x,y],"size":size}});
                                let a = adjust_params::destructive_eyedropper(&s, kind, &p).unwrap().unwrap();
                                // Dialog sampling follows the current channel; direct commands
                                // can name it explicitly. Both must resolve the same plane.
                                p["target"] = json!("mask");
                                assert_eq!(a, adjust_params::destructive_eyedropper(&s, kind, &p).unwrap().unwrap());
                                let values = adjust_params::to_params(&a);
                                let actual = if kind == "levels" {
                                    values["inWhite"].as_f64().unwrap()
                                } else {
                                    values["points"].as_array().unwrap().last().unwrap()[0].as_f64().unwrap()
                                };
                                assert!(
                                    (actual - expected).abs() < 0.02,
                                    "{mode:?} {depth:?} linked={linked} shift={dx},{dy} at={x},{y} size={size} {kind}: {actual} != {expected}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A sampled adjustment is blended through feathered selection coverage exactly once.
#[test]
fn scalar_picker_respects_selection_coverage_when_applied() {
    for target in [json!("mask"), json!("quickMask"), json!({"channel":0})] {
        let mut s = session(ColorMode::Lab, SampleType::U16);
        s.edit("Feathered selection", |doc, _| {
            let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
            selection.fill_rect(Rect::from_xywh(0, 0, 16, 16), &[0.5]);
            doc.selection = Some(selection);
            Ok(())
        })
        .unwrap();
        s.execute("image.adjustments.curves", json!({"target":target,"eyedropper":{"point":"white","at":[8,8]}})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert!((plane(doc, &target).sample_channel(8, 8, 0) - 0.625).abs() < 0.01);
        assert!((plane(doc, &target).sample_channel(24, 24, 0) - 0.25).abs() < 0.01);
    }
}

/// Malformed and out-of-bounds gray samples fail without modifying pixels or history.
#[test]
fn bad_scalar_picker_params_fail_without_changing_pixels() {
    let bad = [
        json!({"point":"white","at":[99,0]}),
        json!({"point":"white","at":[-1,0]}),
        json!({"point":"white","at":["x",0]}),
        json!({"point":"white","at":[0,0],"size":1000000000}),
        json!({"point":"gray","at":[0,0]}),
        json!({"point":"bogus","at":[0,0]}),
        json!({"point":"white","color":[0,0]}),
        Value::Null,
    ];
    for kind in ["levels", "curves"] {
        let mut s = session(ColorMode::Lab, SampleType::F32);
        let before = s.active().unwrap().doc.clone();
        for eyedropper in &bad {
            assert!(s.execute(&format!("image.adjustments.{kind}"), json!({"target":{"channel":0},"eyedropper":eyedropper})).is_err(), "{eyedropper}");
            assert_eq!(s.active().unwrap().doc.channels, before.channels);
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
        }
        assert!(s.execute(&format!("image.adjustments.{kind}"), json!({"target":{"channel":999},"eyedropper":{"point":"white","at":[8,8]}})).is_err());
    }
}

/// Hidden RGB settings must never affect a gray plane, including pencil curves and sampled edits.
#[test]
fn gray_tone_records_do_not_leak_from_rgb_channels() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Lab, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (target, panel) in
                [(json!("mask"), ChannelTarget::LayerMask), (json!({"channel":0}), ChannelTarget::Alpha(0)), (json!("quickMask"), ChannelTarget::Composite)]
            {
                for style in ["levels", "points", "pencil"] {
                    for sampled in [false, true] {
                        for explicit in [false, true] {
                            let mut clean = session(mode, depth);
                            clean.active_mut().unwrap().channel_view.target = panel;
                            clean
                                .edit("Vary gray samples", |doc, _| {
                                    let patch = Rect::from_xywh(4, 4, 1, 1);
                                    doc.layers[0].mask.as_mut().unwrap().surface.fill_rect(patch, &[0.125]);
                                    doc.channels[0].surface.fill_rect(patch, &[0.125]);
                                    doc.quick_mask.as_mut().unwrap().surface.fill_rect(patch, &[0.125]);
                                    Ok(())
                                })
                                .unwrap();
                            let before = clean.active().unwrap().doc.clone();
                            let layer = clean.active().unwrap().active_layer.unwrap();
                            let mut dirty = Session::new();
                            dirty.add_document((*before).clone(), None);
                            dirty.select_layer(layer).unwrap();
                            dirty.active_mut().unwrap().channel_view.target = panel;
                            let kind = if style == "levels" { "levels" } else { "curves" };
                            let mut params = if kind == "levels" { json!({"gamma":2}) } else { json!({"points":[[0,255],[255,0]],"pencil":style == "pencil"}) };
                            if explicit {
                                params["target"] = target.clone();
                            }
                            if sampled {
                                params["eyedropper"] = json!({"point":"white","at":[8,8]});
                            }
                            let mut contaminated = params.clone();
                            for key in ["red", "green", "blue"] {
                                contaminated[key] = if kind == "levels" {
                                    json!({"gamma":4})
                                } else if style == "pencil" {
                                    json!(vec![255; 256])
                                } else {
                                    json!([[0, 255], [255, 255]])
                                };
                            }
                            let command = format!("image.adjustments.{kind}");
                            clean.execute(&command, params).unwrap();
                            dirty.execute(&command, contaminated).unwrap();
                            let expected = &clean.active().unwrap().doc;
                            let actual = &dirty.active().unwrap().doc;
                            assert!(plane(expected, &target) != plane(&before, &target), "the scalar adjustment really edits the plane");
                            assert_eq!(
                                plane(actual, &target).sample_channel(4, 4, 0),
                                plane(expected, &target).sample_channel(4, 4, 0),
                                "{mode:?} {depth:?} {target} {style} sampled={sampled} explicit={explicit}"
                            );
                            assert!(
                                plane(actual, &target) == plane(expected, &target),
                                "{mode:?} {depth:?} {target} {style} sampled={sampled} explicit={explicit}"
                            );
                            assert_eq!(actual.layers[0].content, before.layers[0].content);
                            if target != json!("mask") {
                                assert_eq!(actual.layers[0].mask, before.layers[0].mask);
                            }
                            if target != json!("quickMask") {
                                assert_eq!(actual.quick_mask, before.quick_mask);
                            }
                            if target != json!({"channel":0}) {
                                assert_eq!(actual.channels, before.channels);
                            }
                            assert_eq!(actual.channels[0].spot, before.channels[0].spot);
                        }
                    }
                }
            }
        }
    }
}

/// Explicit colour sets and pixel commands must not inherit an unrelated gray-panel target.
#[test]
fn explicit_tone_descriptors_resolve_colour_sets_before_panel_state() {
    use photocraft_engine::channel_cmds::{GraySurface, isolated_curve_channel_with_params};
    for mode in [ColorMode::Rgb, ColorMode::Lab, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for target in [ChannelTarget::Alpha(0), ChannelTarget::LayerMask, ChannelTarget::Color(1)] {
                let mut s = session(mode, depth);
                let st = s.active_mut().unwrap();
                st.channel_view.target = target;
                st.channel_view.selected_colors = vec![0, 1];
                let colors = isolated_curve_channel_with_params(st, true, &json!({"target":{"colors":[0,2]}})).unwrap();
                assert_eq!(colors.components.unwrap().indices().collect::<Vec<_>>(), [0, 2]);
                assert!(colors.gray.is_none());
                assert!(isolated_curve_channel_with_params(st, true, &json!({"target":"pixels"})).is_none());
                let scalar = isolated_curve_channel_with_params(st, true, &json!({"target":{"channel":0}})).unwrap();
                assert_eq!(scalar.name, "Spot ink");
                assert_eq!(scalar.gray, Some(GraySurface::Alpha(0)));
                assert!(scalar.components.is_none());
                let mask = isolated_curve_channel_with_params(st, false, &json!({"target":"mask"})).unwrap();
                assert_eq!(mask.gray, Some(GraySurface::LayerMask));
                assert!(mask.components.is_none());
            }
        }
    }
}

/// An explicitly addressed mask supplies the picker sample and receives the edit, even if
/// the active layer has a different mask or no mask. Native metadata and other planes survive.
#[test]
fn review_picker_reads_the_addressed_mask_layer() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for kind in ["levels", "curves"] {
                let mut s = session(mode, depth);
                let mut other = s.active().unwrap().doc.layers[0].clone();
                other.id = Layer::new("id", other.content.clone()).id;
                other.name = "Addressed".into();
                let bounds = s.active().unwrap().doc.bounds();
                other.mask.as_mut().unwrap().surface.fill_rect(bounds, &[0.75]);
                let id = other.id;
                s.edit("Second mask", |doc, _| {
                    doc.layers.push(other);
                    Ok(())
                })
                .unwrap();
                for selector in [json!(id.0), json!("Addressed"), json!({"name":"Addressed"})] {
                    let p = json!({"target":"mask","layer":selector,"eyedropper":{"point":"white","at":[8,8]}});
                    let sample = s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0);
                    let expected =
                        adjust_params::adjustment_eyedropper(kind, &json!({}), adjust_params::CurvesEyedropper::White, [sample; 3], ColorMode::Grayscale)
                            .unwrap();
                    assert_eq!(adjust_params::destructive_eyedropper(&s, kind, &p).unwrap(), Some(expected), "{mode:?} {depth:?} {kind}");
                }
                s.edit("Active has no mask", |doc, _| {
                    doc.layers[0].mask = None;
                    Ok(())
                })
                .unwrap();
                let before = s.active().unwrap().doc.clone();
                s.execute(&format!("image.adjustments.{kind}"), json!({"target":"mask","layer":id.0,"eyedropper":{"point":"white","at":[8,8]}})).unwrap();
                let doc = &s.active().unwrap().doc;
                assert!(doc.layer(id).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0) > 0.99);
                assert_eq!(doc.layers[0], before.layers[0]);
                assert_eq!(doc.channels, before.channels);
                assert_eq!(doc.quick_mask, before.quick_mask);
                s.undo();
                assert_eq!(s.active().unwrap().doc.layers, before.layers);
            }
        }
    }
}
