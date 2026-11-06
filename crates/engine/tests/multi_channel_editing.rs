use photocraft_color::SampleType;
use photocraft_engine::{Session, brush_cmds::LiveStroke, channel_cmds::ChannelTarget, color_channels};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

fn setup(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":24,"mode":mode,"depth":depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents":"white"})).unwrap();
    s.edit("fixture", |doc, active| {
        let r = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let n = surf.channels();
        let mut data = Vec::new();
        for y in 0..24 {
            for x in 0..32 {
                for k in 0..n - 1 {
                    data.push(((x * 7 + y * 11 + k * 37) % 221 + 15) as f32 / 255.0);
                }
                data.push(0.65);
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
    s.execute("history.setSource", json!({"state":2})).unwrap();
    s
}
fn surface(s: &Session) -> Surface {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}
fn subset(s: &mut Session) {
    s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
}
fn unchanged(before: &Surface, after: &Surface) {
    let fmt = before.format();
    let bytes = fmt.sample.bytes();
    let keep: Vec<_> = (0..fmt.channels()).filter(|k| ![0, 2].contains(k)).collect();
    for (c, t) in before.tiles() {
        let new = after.tile(*c).unwrap();
        for (a, b) in t.bytes().chunks_exact(fmt.bytes_per_pixel()).zip(new.bytes().chunks_exact(fmt.bytes_per_pixel())) {
            for k in &keep {
                assert_eq!(&a[k * bytes..(k + 1) * bytes], &b[k * bytes..(k + 1) * bytes], "component {k}");
            }
        }
    }
}

#[test]
fn shift_selection_is_color_only_and_all_colors_become_composite() {
    let mut s = setup("rgb", 8);
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    s.execute("channel.target", json!({"channel":"blue","extend":true})).unwrap();
    assert_eq!(color_channels::selected(&s).unwrap().indices().collect::<Vec<_>>(), vec![0, 2]);
    let view = &s.active().unwrap().channel_view;
    assert!(view.color_visible(0) && view.color_visible(2) && !view.color_visible(1));
    let revision = s.active().unwrap().revision;
    assert!(s.execute("channel.target", json!({"channel":"mask","extend":true})).is_err());
    assert_eq!(s.active().unwrap().revision, revision);
    s.execute("channel.target", json!({"channel":"green","extend":true})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Composite);
    s.execute("channel.new", json!({"name":"saved"})).unwrap();
    assert!(s.execute("channel.target", json!({"channel":"saved","extend":true})).is_err());
}

#[test]
fn adjustments_filters_and_drawing_preserve_other_encoded_components() {
    let commands = [
        ("image.adjustments.invert", json!({})),
        ("image.adjustments.threshold", json!({"level":128})),
        ("image.adjustments.levels", json!({"gamma":1.7})),
        ("image.adjustments.curves", json!({"points":[[0,255],[255,0]]})),
        ("filter.blur.gaussianBlur", json!({"radius":2})),
        ("paint.stroke", json!({"points":[[8,8],[22,12]],"size":9,"color":"#ffffff"})),
        ("paint.pencil", json!({"points":[[8,8],[22,12]],"size":9,"color":"#000000"})),
        ("paint.stroke", json!({"points":[[8,8],[22,12]],"size":9,"erase":true})),
        ("paint.bucket", json!({"x":10,"y":10,"tolerance":255,"color":"#ffffff"})),
        ("paint.gradient", json!({"from":[0,0],"to":[31,0],"colors":["#000000","#ffffff"]})),
        ("paint.dodge", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.burn", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.blur", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.sharpen", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.smudge", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.magicEraser", json!({"x":10,"y":10,"tolerance":255})),
        ("paint.backgroundEraser", json!({"points":[[8,8],[22,12]],"size":9,"tolerance":100})),
        ("paint.colorReplacement", json!({"points":[[8,8],[22,12]],"size":9,"tolerance":100,"mode":"luminosity","color":"#ffffff"})),
        ("paint.cloneStamp", json!({"points":[[8,8],[22,12]],"size":9,"source":[2,2]})),
        ("paint.historyBrush", json!({"points":[[8,8],[22,12]],"size":9})),
        ("paint.artHistoryBrush", json!({"points":[[8,8],[22,12]],"size":9,"tolerance":0})),
        ("paint.mixerBrush", json!({"points":[[8,8],[22,12]],"size":9,"color":"#ffffff"})),
    ];
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for (cmd, p) in &commands {
                let mut s = setup(mode, depth);
                subset(&mut s);
                let before = surface(&s);
                s.execute(cmd, p.clone()).unwrap_or_else(|e| panic!("{mode} {depth} {cmd}: {e}"));
                let after = surface(&s);
                unchanged(&before, &after);
                assert_ne!(before, after, "{mode} {depth} {cmd} must change selected samples");
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(surface(&s), before, "undo {cmd}");
                s.execute("edit.redo", json!({})).unwrap();
                assert_eq!(surface(&s), after, "redo {cmd}");
            }
        }
    }
}

#[test]
fn threshold_uses_each_selected_native_plane() {
    let mut s = setup("rgb", 16);
    s.edit("pixel", |doc, id| {
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[0.1, 0.95, 0.8, 0.7]);
        Ok(())
    })
    .unwrap();
    subset(&mut s);
    s.execute("image.adjustments.threshold", json!({"level":128})).unwrap();
    let px = surface(&s).pixel(4, 4);
    assert_eq!((px[0], px[2]), (0.0, 1.0));
    assert!((px[1] - 0.95).abs() < 0.0001 && (px[3] - 0.7).abs() < 0.0001);
}

#[test]
fn explicit_target_wins_and_invalid_sets_are_transactional() {
    let mut s = setup("rgb", 32);
    s.execute("channel.target", json!({"channel":"green"})).unwrap();
    let before = surface(&s);
    s.execute("image.adjustments.invert", json!({"target":{"colors":[0,2]}})).unwrap();
    unchanged(&before, &surface(&s));
    for colors in [json!([]), json!([-1, 2]), json!([0, 3]), json!([0, "red"]), json!(null), json!([u64::MAX]), json!(true)] {
        let prior = surface(&s);
        assert!(s.execute("image.adjustments.invert", json!({"target":{"colors":colors}})).is_err());
        assert_eq!(surface(&s), prior);
    }
}

#[test]
fn live_brush_and_eraser_match_commit_at_all_native_depths() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for erase in [false, true] {
                let mut s = setup(mode, depth);
                subset(&mut s);
                let before = surface(&s);
                let mut p = json!({"points":[[6,7,1],[23,15,0.7]],"size":9,"color":"#ffffff","erase":erase,"seed":42});
                let live = LiveStroke::begin(&s, &p).unwrap();
                p["seed"] = json!(live.seed);
                let st = s.active().unwrap();
                let shown = live.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
                unchanged(&before, &shown);
                s.offer_live_stroke(live).unwrap();
                let result = s.execute("paint.stroke", p).unwrap();
                assert_eq!(result["fromLive"], true, "reuse {mode} {depth} {erase}");
                assert!(
                    surface(&s) == shown,
                    "live pixels differ: {mode} {depth} {erase}; shown tiles {}, committed {}",
                    shown.tile_count(),
                    surface(&s).tile_count()
                );
            }
        }
    }
}

#[test]
fn live_mixer_captures_selected_components_and_matches_commit_and_replay() {
    use photocraft_engine::brush_cmds::mixer_live::LiveMixer;
    use photocraft_engine::paint::StrokePoint;

    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            let mut s = setup(mode, depth);
            subset(&mut s);
            let before = surface(&s);
            let params = json!({"points":[[6,7,1]],"size":9,"color":"#ffffff","seed":42,"sampleAllLayers":true});
            let mut live = LiveMixer::begin(&s, &params).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            let first = live.doc.layer(id).unwrap().surface().unwrap();
            unchanged(&before, first);
            assert_ne!(&before, first, "the initial dab paints {mode} {depth}");
            live.push(&[StrokePoint::new(23.0, 15.0, 0.7)]).unwrap();
            let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
            unchanged(&before, &shown);
            // The in-flight stroke keeps its original target even if the panel changes.
            s.execute("channel.target", json!({"channel":"composite"})).unwrap();
            let params = live.prepare_commit(&mut s).unwrap();
            assert_eq!(params["target"], json!({"colors":[0,2]}));
            assert_eq!(s.execute("paint.mixerBrush", params.clone()).unwrap()["fromLive"], true);
            assert_eq!(surface(&s), shown, "displayed pixels are committed {mode} {depth}");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(surface(&s), before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(surface(&s), shown);
            let mut replay = setup(mode, depth);
            replay.execute("paint.mixerBrush", params).unwrap();
            assert_eq!(surface(&replay), shown, "replay matches {mode} {depth}");
        }
    }
}

#[test]
fn unsupported_geometric_edits_and_camera_raw_fail_before_modification() {
    let mut s = setup("rgb", 8);
    subset(&mut s);
    for cmd in [
        "layer.translate",
        "select.float",
        "edit.transform",
        "edit.transform.rotate90Cw",
        "edit.contentAwareScale",
        "edit.puppetWarp",
        "edit.perspectiveWarp",
        "filter.cameraRaw",
    ] {
        let before = surface(&s);
        assert!(!s.is_enabled(cmd), "{cmd}");
        assert!(s.execute(cmd, json!({"dx":2,"dy":3})).is_err(), "{cmd}");
        assert_eq!(surface(&s), before);
    }
}

#[test]
fn native_float_payloads_survive_unselected_component_restoration() {
    // Exercise the low-level helper directly with raw HDR, signed zero and NaN payloads.
    let fmt = photocraft_color::PixelFormat::RGBA32F;
    let mut before = Surface::new(fmt);
    before.fill_rect(Rect::new(0, 0, 1, 1), &[2.0, 0.1, -1.0, 0.5]);
    let coord = photocraft_geom::TileCoord { tx: 0, ty: 0 };
    before.tile_mut(coord).bytes_mut()[4..8].copy_from_slice(&0x7fc0_1234u32.to_ne_bytes());
    let mut after = Surface::new(fmt);
    after.fill_rect(Rect::new(0, 0, 1, 1), &[0.3, 0.5, 0.7, 0.9]);
    let colors = color_channels::ColorChannels::from_indices([0, 2], 3).unwrap();
    color_channels::restrict_surface(&before, &mut after, colors).unwrap();
    assert_eq!(&before.tile(coord).unwrap().bytes()[4..8], &after.tile(coord).unwrap().bytes()[4..8]);
    assert_eq!(after.format().sample, SampleType::F32);
}

#[test]
fn live_retouching_matches_direct_replay_and_preserves_unselected_bytes() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for cmd in ["paint.dodge", "paint.burn", "paint.blur", "paint.sharpen", "paint.smudge", "paint.cloneStamp", "paint.historyBrush"] {
                let mut s = setup(mode, depth);
                subset(&mut s);
                let original = surface(&s);
                let p = json!({"points":[[6,7,1],[23,15,0.7]],"size":9,"source":[2,2]});
                let shown = if matches!(cmd, "paint.cloneStamp" | "paint.historyBrush") {
                    let live = photocraft_engine::retouch_cmds::LiveRetouch::begin(&s, cmd, &p).unwrap();
                    let st = s.active().unwrap();
                    let shown = live.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
                    s.offer_live_retouch(live).unwrap();
                    shown
                } else {
                    let live = photocraft_engine::retouch_cmds::LiveDab::begin(&s, cmd, &p).unwrap();
                    let st = s.active().unwrap();
                    let shown = live.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone();
                    s.offer_live_dab(live).unwrap();
                    shown
                };
                unchanged(&original, &shown);
                s.execute(cmd, p.clone()).unwrap();
                let committed = surface(&s);
                unchanged(&original, &committed);
                let mut replay = setup(mode, depth);
                subset(&mut replay);
                replay.execute(cmd, p).unwrap();
                assert!(committed == surface(&replay), "{mode} {depth} {cmd}: live reuse differs from replay");
            }
        }
    }
}

#[test]
fn copy_cut_paste_operate_on_the_whole_selected_set_in_one_history_step() {
    let mut s = setup("rgb", 16);
    subset(&mut s);
    let original = surface(&s);
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let inverted = surface(&s);
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    assert!(surface(&s) == original);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(surface(&s) == inverted);
    s.execute("edit.cut", json!({})).unwrap();
    unchanged(&inverted, &surface(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(surface(&s) == inverted);
}

#[test]
fn explicit_layer_restriction_protects_the_layer_actually_edited() {
    let mut s = setup("rgb", 32);
    let first = s.active().unwrap().active_layer.unwrap();
    let original = surface(&s);
    s.execute("layer.new.layer", json!({})).unwrap();
    subset(&mut s);
    s.execute("paint.stroke", json!({"layer":first.0,"target":{"colors":[0,2]},"points":[[8,8],[23,12]],"size":9,"color":"#ffffff"})).unwrap();
    let after = s.active().unwrap().doc.layer(first).unwrap().surface().unwrap();
    unchanged(&original, after);
    assert!(original != *after);
}

#[test]
fn background_eraser_on_an_opaque_background_edits_only_selected_channels() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":32,"height":24,"depth":depth})).unwrap();
        s.tools.background = [0.0, 0.0, 0.0, 1.0];
        subset(&mut s);
        let before = surface(&s);
        let id = s.active().unwrap().active_layer.unwrap();
        let p = json!({"points":[[12,12]],"size":12,"hardness":1,"tolerance":100});
        let live = photocraft_engine::eraser_cmds::live_background_eraser(&s, &p).unwrap();
        let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
        s.execute("paint.backgroundEraser", p).unwrap();
        let after = surface(&s);
        unchanged(&before, &after);
        assert!(before != after, "depth {depth}");
        assert!(shown == after, "preview depth {depth}");
        assert_eq!(before.format(), after.format());
        assert!(s.active().unwrap().doc.is_background(id));
    }
}

#[test]
fn lab_combined_tone_curve_changes_both_selected_components() {
    for command in ["image.adjustments.levels", "image.adjustments.curves"] {
        let mut s = setup("lab", 32);
        subset(&mut s);
        let before = surface(&s).pixel(4, 4);
        let p = if command.ends_with("levels") { json!({"gamma":1.8}) } else { json!({"points":[[0,255],[255,0]]}) };
        s.execute(command, p).unwrap();
        let after = surface(&s).pixel(4, 4);
        assert_ne!(before[0], after[0]);
        assert_ne!(before[2], after[2]);
        assert_eq!(before[1], after[1]);
        assert_eq!(before[3], after[3]);
    }
}

#[test]
fn lab16_neutral_chroma_survives_inversion_and_off_canvas_samples_are_adjusted() {
    let mut s = setup("lab", 16);
    s.edit("outside canvas", |doc, id| {
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(-4, -4, 4, 4), &[0.2, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
        Ok(())
    })
    .unwrap();
    subset(&mut s);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let px = surface(&s).pixel(-2, -2);
    assert!((px[0] - 0.8).abs() < 1e-4);
    assert!((px[2] - 128.0 / 255.0).abs() < 1e-4);
    assert!((px[1] - 128.0 / 255.0).abs() < 1e-4);
}

#[test]
fn explicit_copy_and_singleton_paste_use_the_requested_components() {
    let mut s = setup("rgb", 16);
    let before = surface(&s);
    s.execute("edit.copy", json!({"target":{"colors":[0,2]}})).unwrap();
    assert_eq!(s.clipboard.as_ref().unwrap().components, Some(vec![0, 2]));
    s.execute("channel.target", json!({"colors":[0,2]})).unwrap();
    s.execute("edit.copy", json!({"target":{"colors":[1]}})).unwrap();
    assert_eq!(s.clipboard.as_ref().unwrap().surface.format().mode, photocraft_color::ColorMode::Grayscale);
    let layers = s.active().unwrap().doc.layers.len();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({"target":{"colors":[0]}})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    let after = surface(&s).pixel(5, 5);
    let old = before.pixel(5, 5);
    assert_eq!(after[0], old[1]);
    assert_eq!(after[1], old[1]);
    assert_eq!(after[2], old[2]);
    assert_eq!(after[3], old[3]);
}

#[test]
fn paste_into_and_outside_obey_subset_and_pixel_selection() {
    for outside in [false, true] {
        let mut s = setup("rgb", 16);
        subset(&mut s);
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let before = surface(&s);
        let layers = s.active().unwrap().doc.layers.len();
        s.execute("select.rect", json!({"x":0,"y":0,"width":16,"height":24})).unwrap();
        let cmd = if outside { "edit.pasteSpecial.pasteOutside" } else { "edit.pasteSpecial.pasteInto" };
        s.execute(cmd, json!({"center":[16,12]})).unwrap();
        let after = surface(&s);
        unchanged(&before, &after);
        assert_eq!(s.active().unwrap().doc.layers.len(), layers);
        let (changed, kept) = if outside { (24, 8) } else { (8, 24) };
        assert_ne!(before.pixel(changed, 10)[0], after.pixel(changed, 10)[0]);
        assert_eq!(before.pixel(kept, 10), after.pixel(kept, 10));
    }
}

#[test]
fn feathered_pixel_selection_constrains_subset_adjust_filter_and_brush() {
    for depth in [8, 16, 32] {
        for (command, params) in [
            ("image.adjustments.invert", json!({})),
            ("filter.blur.gaussianBlur", json!({"radius":2})),
            ("paint.stroke", json!({"points":[[0,12],[31,12]],"size":32,"color":"#ffffff","seed":42})),
        ] {
            let mut s = setup("rgb", depth);
            s.execute("select.rect", json!({"x":10,"y":7,"width":12,"height":10})).unwrap();
            s.execute("select.modify.feather", json!({"radius":2})).unwrap();
            let selection = s.active().unwrap().doc.selection.clone().unwrap();
            let before = surface(&s);
            subset(&mut s);
            s.execute(command, params).unwrap();
            let after = surface(&s);
            unchanged(&before, &after);
            assert_ne!(before, after, "{command} @{depth}");
            let mut partial = false;
            for y in 0..24 {
                for x in 0..32 {
                    let amount = selection.pixel(x, y)[0];
                    if amount == 0.0 {
                        assert_eq!(before.pixel(x, y), after.pixel(x, y), "outside {command}");
                    }
                    if command == "image.adjustments.invert" && amount > 0.0 && amount < 1.0 {
                        partial = true;
                        for ch in [0, 2] {
                            let old = before.pixel(x, y)[ch];
                            let expected = old + (1.0 - 2.0 * old) * amount;
                            let tolerance = if depth == 8 {
                                2.0 / 255.0
                            } else if depth == 16 {
                                2.0 / 65535.0
                            } else {
                                1e-6
                            };
                            assert!((after.pixel(x, y)[ch] - expected).abs() < tolerance, "feather @{depth}");
                        }
                    }
                }
            }
            assert!(command != "image.adjustments.invert" || partial);
        }
    }
}

#[test]
fn supported_filter_families_and_last_filter_follow_current_set() {
    for (command, params) in [
        ("filter.blur.boxBlur", json!({"radius":2})),
        ("filter.other.highPass", json!({"radius":2})),
        ("filter.noise.median", json!({"radius":2})),
        ("filter.noise.addNoise", json!({"amount":30,"seed":42})),
        ("filter.stylize.emboss", json!({"height":2})),
        ("filter.distort.twirl", json!({"angle":60})),
        ("filter.pixelate.mosaic", json!({"cellSize":4})),
    ] {
        let mut s = setup("rgb", 16);
        subset(&mut s);
        let before = surface(&s);
        s.execute(command, params).unwrap();
        unchanged(&before, &surface(&s));
        assert_ne!(before, surface(&s), "{command}");
    }
    let mut s = setup("rgb", 32);
    subset(&mut s);
    s.execute("filter.noise.addNoise", json!({"amount":30,"seed":42})).unwrap();
    s.execute("channel.target", json!({"channel":"green"})).unwrap();
    let before = surface(&s);
    s.execute("filter.lastFilter", json!({})).unwrap();
    let after = surface(&s);
    for y in 0..24 {
        for x in 0..32 {
            for k in [0, 2, 3] {
                assert_eq!(before.pixel(x, y)[k].to_bits(), after.pixel(x, y)[k].to_bits(), "last filter component {k}");
            }
        }
    }
    assert_ne!(before, after);
}

#[test]
fn explicit_clear_and_singleton_cut_override_saved_channel_selection() {
    for command in ["edit.clear", "edit.cut"] {
        let mut s = setup("rgb", 32);
        s.execute("channel.new", json!({"fill":"white"})).unwrap();
        let before = surface(&s);
        let alpha = s.active().unwrap().doc.channels[0].surface.clone();
        s.execute(command, json!({"target":{"colors":[0]}})).unwrap();
        let after = surface(&s);
        assert_eq!(s.active().unwrap().doc.channels[0].surface, alpha);
        assert_ne!(before, after);
        for y in 0..24 {
            for x in 0..32 {
                for k in [1, 2, 3] {
                    assert_eq!(before.pixel(x, y)[k].to_bits(), after.pixel(x, y)[k].to_bits());
                }
            }
        }
        if command == "edit.cut" {
            assert_eq!(s.clipboard.as_ref().unwrap().surface.pixel(8, 8)[0], before.pixel(8, 8)[0]);
        }
        s.undo();
        assert_eq!(surface(&s), before);
    }
}

#[test]
fn singleton_component_clip_pastes_into_and_outside_without_luminosity_conversion() {
    for command in ["edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"] {
        let mut s = setup("rgb", 32);
        s.edit("flat", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[0.8, 0.4, 0.2, 0.65]);
            Ok(())
        })
        .unwrap();
        s.execute("edit.copy", json!({"target":{"colors":[0]}})).unwrap();
        s.execute("channel.target", json!({"channel":"blue"})).unwrap();
        s.execute("select.rect", json!({"x":0,"y":0,"width":16,"height":24})).unwrap();
        let before = surface(&s);
        s.execute(command, json!({"center":[16,12]})).unwrap();
        let (edited, kept) = if command.ends_with("pasteInto") { (4, 24) } else { (24, 4) };
        assert!((surface(&s).pixel(edited, 12)[2] - 0.8).abs() < 1e-6);
        assert_eq!(surface(&s).pixel(kept, 12), before.pixel(kept, 12));
        s.undo();
        assert_eq!(surface(&s), before);
    }
}

#[test]
fn lab_chroma_inversion_materializes_selected_default_pixels_and_preserves_holes() {
    for depth in [16, 32] {
        for inverse in [false, true] {
            let mut s = setup("lab", depth);
            s.edit("sparse opaque", |doc, id| {
                let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                *surf = Surface::with_default(surf.format(), &[0.2, 0.3, 0.4, 1.0]);
                let mut mask = Surface::new(photocraft_color::PixelFormat::GRAY8);
                if inverse {
                    mask.fill_rect(doc.bounds(), &[1.0]);
                }
                mask.fill_rect(Rect::new(8, 6, 20, 16), &[if inverse { 0.0 } else { 1.0 }]);
                doc.selection = Some(mask);
                Ok(())
            })
            .unwrap();
            let before = surface(&s);
            subset(&mut s);
            s.execute("image.adjustments.invert", json!({})).unwrap();
            let after = surface(&s);
            for (x, y) in [(0, 0), (12, 10), (31, 23)] {
                let selected = Rect::new(8, 6, 20, 16).contains(x, y) != inverse;
                let old = before.pixel(x, y);
                let new = after.pixel(x, y);
                for ch in [1, 3] {
                    assert_eq!(old[ch].to_bits(), new[ch].to_bits());
                }
                if selected {
                    assert!((new[0] - (1.0 - old[0])).abs() < 2.0 / 65535.0, "depth={depth} inverse={inverse} xy={x},{y} old={old:?} new={new:?}");
                    assert!((new[2] - (256.0 / 255.0 - old[2])).abs() < 2.0 / 65535.0);
                } else {
                    assert_eq!(old, new);
                }
            }
            s.undo();
            assert_eq!(surface(&s), before);
        }
    }
}

#[test]
fn bucket_and_gradient_edit_the_explicit_layer_and_restrict_that_same_layer() {
    for (command, mut params) in [
        ("paint.bucket", json!({"x":10,"y":10,"tolerance":255,"color":"#ffffff"})),
        ("paint.gradient", json!({"from":[0,0],"to":[31,0],"colors":["#ffffff","#000000"]})),
    ] {
        let mut s = setup("rgb", 16);
        let id = s.active().unwrap().active_layer.unwrap();
        let before = surface(&s);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color":"#336699"})).unwrap();
        let active = surface(&s);
        subset(&mut s);
        params["layer"] = json!(id.0);
        s.execute(command, params).unwrap();
        assert_eq!(surface(&s), active);
        let changed = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
        unchanged(&before, changed);
        assert_ne!(&before, changed);
        s.undo();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &before);
    }
}

#[test]
fn legacy_cmyk_channel_clips_keep_their_tone_when_pasted_into_a_set_or_single_channel() {
    for dest in [json!({"colors":[0,2]}), json!({"channel":{"color":2}})] {
        for depth in [8, 16, 32] {
            let mut s = setup("cmyk", depth);
            s.edit("inks", |doc, id| {
                doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[0.2, 0.4, 0.6, 0.1, 0.65]);
                Ok(())
            })
            .unwrap();
            s.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            assert!((s.clipboard.as_ref().unwrap().surface.pixel(4, 4)[0] - 0.8).abs() < 0.005);
            s.execute("channel.target", dest.clone()).unwrap();
            s.execute("edit.paste", json!({})).unwrap();
            assert!((surface(&s).pixel(4, 4)[2] - 0.2).abs() < 0.005, "depth={depth} target={dest}");
        }
    }
}

#[test]
fn cmyk_grayscale_clips_roundtrip_explicit_and_limited_paste_at_every_depth() {
    for depth in [8, 16, 32] {
        for explicit_copy in [false, true] {
            for command in ["edit.paste", "edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"] {
                let mut s = setup("cmyk", depth);
                s.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
                let before = surface(&s);
                s.execute("edit.copy", if explicit_copy { json!({"target":{"color":0}}) } else { json!({}) }).unwrap();
                s.edit("clear cyan", |doc, id| {
                    let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                    let mut raw = surf.read_region(Rect::new(0, 0, 32, 24));
                    for p in raw.as_chunks_mut::<5>().0 {
                        p[0] = 0.93;
                    }
                    surf.write_region(Rect::new(0, 0, 32, 24), &raw);
                    Ok(())
                })
                .unwrap();
                if command != "edit.paste" {
                    s.execute("select.rect", json!({"x":0,"y":0,"width":16,"height":24})).unwrap();
                }
                s.execute(command, json!({"target":{"color":0},"center":[16,12]})).unwrap();
                let after = surface(&s);
                let x = if command.ends_with("pasteOutside") { 20 } else { 4 };
                assert!((before.pixel(x, 8)[0] - after.pixel(x, 8)[0]).abs() < 2.0 / 255.0, "{depth} {explicit_copy} {command}");
            }
        }
    }
}

#[test]
fn last_filter_rejects_non_object_journal_params_without_panicking() {
    for saved in [json!(17), json!(true), json!([]), json!("bad")] {
        let mut s = setup("rgb", 16);
        s.journal.push(("filter.blur.gaussianBlur".into(), saved));
        let before = surface(&s);
        let p = json!({"target":{"colors":[0,2]},"layer":s.active().unwrap().active_layer.unwrap().0});
        assert!(s.execute("filter.lastFilter", p.clone()).is_err());
        let spec = photocraft_engine::command_specs().iter().find(|s| s.id == "filter.lastFilter").unwrap();
        assert!((spec.run)(&mut s, &p).is_err());
        assert_eq!(surface(&s), before);
    }
    let mut s = setup("rgb", 16);
    s.journal.push(("filter.blur.gaussianBlur".into(), json!(null)));
    s.execute("filter.lastFilter", json!({"target":{"color":0}})).unwrap();
}

#[test]
fn first_single_channel_retouch_preview_preserves_every_other_sample() {
    for cmd in ["paint.dodge", "paint.burn", "paint.sponge", "paint.blur", "paint.sharpen", "paint.smudge", "paint.colorReplacement"] {
        let mut s = setup("rgb", 32);
        s.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
        let before = surface(&s);
        let p = json!({"points":[[6,7,1],[23,15,0.7]],"size":9,"color":"#ffffff","tolerance":100});
        let live = if cmd == "paint.colorReplacement" {
            photocraft_engine::brush_cmds::live_color_replacement(&s, &p).unwrap()
        } else {
            photocraft_engine::retouch_cmds::LiveDab::begin(&s, cmd, &p).unwrap()
        };
        let shown = live.doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
        for y in 0..24 {
            for x in 0..32 {
                for k in [1, 2, 3] {
                    assert_eq!(before.pixel(x, y)[k].to_bits(), shown.pixel(x, y)[k].to_bits(), "{cmd} {x} {y} {k}");
                }
            }
        }
        if cmd != "paint.colorReplacement" {
            s.offer_live_dab(live).unwrap();
        }
        s.execute(cmd, p).unwrap();
    }
}

#[test]
fn last_filter_forwards_explicit_target_before_implicit_panel_selection() {
    let mut s = setup("rgb", 32);
    subset(&mut s);
    s.execute("filter.noise.addNoise", json!({"amount":20,"seed":42})).unwrap();
    let before = surface(&s);
    s.execute("filter.lastFilter", json!({"target":{"colors":[1]}})).unwrap();
    let after = surface(&s);
    assert_ne!(before, after);
    for y in 0..24 {
        for x in 0..32 {
            for k in [0, 2, 3] {
                assert_eq!(before.pixel(x, y)[k].to_bits(), after.pixel(x, y)[k].to_bits());
            }
        }
    }
}

#[test]
fn selected_channel_paste_keeps_source_document_placement_and_centers_across_documents() {
    for target in [None, Some(json!({"color":0})), Some(json!({"colors":[0,2]}))] {
        let mut s = setup("rgb", 16);
        let source = s.active().unwrap().doc.id;
        s.execute("channel.target", json!({"channel":"red"})).unwrap();
        s.execute("select.rect", json!({"x":2,"y":3,"width":4,"height":4})).unwrap();
        let params = target.as_ref().map_or(json!({}), |target| json!({"target":target}));
        s.execute("edit.copy", params.clone()).unwrap();
        assert_eq!(s.clipboard.as_ref().unwrap().source, Some(source));
        s.execute("select.deselect", json!({})).unwrap();
        assert_eq!(s.execute("edit.paste", params.clone()).unwrap()["offset"], json!([0, 0]), "same document: {target:?}");

        s.execute("file.new", json!({"width":40,"height":32,"depth":16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("channel.target", json!({"channel":"red"})).unwrap();
        assert_ne!(s.active().unwrap().doc.id, source);
        assert_eq!(s.execute("edit.paste", params).unwrap()["offset"], json!([16, 11]), "another document: {target:?}");
    }
}

#[test]
fn native_plane_writeback_rejects_incompatible_formats_without_editing() {
    let mut dest = surface(&setup("rgb", 16));
    let before = dest.clone();
    for format in [
        photocraft_color::PixelFormat::GRAY8,
        photocraft_color::PixelFormat::RGBA8,
        photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::U16, true),
    ] {
        let plane = Surface::new(format);
        assert!(color_channels::scatter(&mut dest, &plane, 0, Rect::new(0, 0, 32, 24)).is_err());
        assert_eq!(dest, before);
    }
}

#[test]
fn lab_single_channel_tone_parameters_round_trip_without_applying_lightness_twice() {
    use photocraft_engine::adjust_params;
    for (kind, params) in [("levels", json!({"lightness":{"gamma":2.0}})), ("curves", json!({"lightness":[[0,0],[128,200],[255,255]]}))] {
        let mut raw = setup("lab", 32);
        let mut canonical = setup("lab", 32);
        for session in [&mut raw, &mut canonical] {
            session.execute("channel.target", json!({"channel":{"color":0}})).unwrap();
        }
        let serialized = adjust_params::to_params(&adjust_params::from_params(kind, &params, None, photocraft_color::ColorMode::Lab).unwrap());
        let command = format!("image.adjustments.{kind}");
        raw.execute(&command, params).unwrap();
        canonical.execute(&command, serialized).unwrap();
        assert_eq!(surface(&raw), surface(&canonical));
    }
}

fn flat_rgb() -> Session {
    let mut s = setup("rgb", 32);
    s.edit("flat", |doc, id| {
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[0.25, 0.5, 0.25, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn noise_monochromatic_and_native_halftone_angles_keep_their_meaning_for_subsets() {
    let mut mono = flat_rgb();
    let mut color = flat_rgb();
    for s in [&mut mono, &mut color] {
        subset(s);
    }
    mono.execute("filter.noise.addNoise", json!({"amount":50,"seed":42,"monochromatic":true})).unwrap();
    color.execute("filter.noise.addNoise", json!({"amount":50,"seed":42,"monochromatic":false})).unwrap();
    assert_eq!(surface(&mono).pixel(4, 12)[0], surface(&mono).pixel(4, 12)[2]);
    assert_ne!(surface(&mono), surface(&color));
    assert_ne!(surface(&color).pixel(4, 12)[0], surface(&color).pixel(4, 12)[2]);
    let mut a = flat_rgb();
    let mut b = flat_rgb();
    for s in [&mut a, &mut b] {
        subset(s);
    }
    a.execute("filter.pixelate.colorHalftone", json!({"maxRadius":4,"channel3":0})).unwrap();
    b.execute("filter.pixelate.colorHalftone", json!({"maxRadius":4,"channel3":35})).unwrap();
    let a = surface(&a);
    let b = surface(&b);
    assert_ne!(a, b);
    for y in 0..24 {
        for x in 0..32 {
            assert_eq!(a.pixel(x, y)[0].to_bits(), b.pixel(x, y)[0].to_bits());
        }
    }
}

#[test]
fn blur_reads_opaque_black_pixels_outside_the_canvas() {
    let mut composite = flat_rgb();
    let mut partial = flat_rgb();
    for s in [&mut composite, &mut partial] {
        s.edit("opaque black border", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(32, 0, 48, 24), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    }
    subset(&mut partial);
    let before = surface(&partial);
    for s in [&mut composite, &mut partial] {
        s.execute("filter.blur.gaussianBlur", json!({"radius":3})).unwrap();
    }
    let expected = surface(&composite).pixel(31, 12);
    let actual = surface(&partial).pixel(31, 12);
    assert!((actual[0] - expected[0]).abs() < 1e-6, "{actual:?} != {expected:?}");
    assert!((actual[2] - expected[2]).abs() < 1e-6);
    unchanged(&before, &surface(&partial));
}

#[test]
fn small_filter_selections_preserve_shared_tiles_outside_the_selection() {
    let mut s = setup("rgb", 32);
    s.edit("larger flat image", |doc, id| {
        doc.size.width = 512;
        doc.size.height = 512;
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 512, 512), &[0.25, 0.5, 0.25, 1.0]);
        Ok(())
    })
    .unwrap();
    subset(&mut s);
    s.execute("select.rect", json!({"x":0,"y":0,"width":16,"height":16})).unwrap();
    let before = surface(&s);
    s.execute("filter.noise.addNoise", json!({"amount":50,"seed":42})).unwrap();
    let after = surface(&s);
    let distant = photocraft_geom::TileCoord { tx: 1, ty: 1 };
    assert_eq!(before.pixel(300, 300), after.pixel(300, 300));
    assert!(std::sync::Arc::ptr_eq(before.tile(distant).unwrap(), after.tile(distant).unwrap()));
}

#[test]
fn rgb_component_exposure_uses_the_document_transfer_at_all_depths() {
    for depth in [8, 16, 32] {
        for colors in [vec![0usize], vec![0, 2]] {
            for selection in [false, true] {
                let mut composite = setup("rgb", depth);
                let mut partial = setup("rgb", depth);
                for s in [&mut composite, &mut partial] {
                    s.edit("midgray", |doc, id| {
                        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[0.5, 0.5, 0.5, 1.0]);
                        Ok(())
                    })
                    .unwrap();
                    if selection {
                        s.execute("select.rect", json!({"x":4,"y":4,"width":20,"height":16})).unwrap();
                    }
                }
                let before = surface(&partial);
                partial.execute("channel.target", json!({"colors":colors})).unwrap();
                for s in [&mut composite, &mut partial] {
                    s.execute("image.adjustments.exposure", json!({"exposure":1})).unwrap();
                }
                let expected = surface(&composite);
                let actual = surface(&partial);
                for (x, y) in [(0, 0), (10, 10)] {
                    for k in 0..4 {
                        if colors.contains(&k) {
                            let tolerance = if depth == 8 {
                                1.1 / 255.0
                            } else if depth == 16 {
                                1.1 / 65535.0
                            } else {
                                1e-6
                            };
                            assert!((actual.pixel(x, y)[k] - expected.pixel(x, y)[k]).abs() < tolerance, "RGB{depth} {colors:?} {selection}");
                        } else {
                            assert_eq!(actual.pixel(x, y)[k].to_bits(), before.pixel(x, y)[k].to_bits());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn color_replacement_live_preview_forwards_explicit_component_and_pixel_targets() {
    for target in [json!({"colors":[1]}), json!("pixels")] {
        let mut s = flat_rgb();
        subset(&mut s);
        let params = json!({"points":[[8,8],[22,12]],"size":9,"tolerance":100,"mode":"luminosity","color":"#ffffff","seed":42,"target":target});
        let live = photocraft_engine::brush_cmds::live_color_replacement(&s, &params).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let preview = live.doc.layer(id).unwrap().surface().unwrap().clone();
        s.execute("paint.colorReplacement", params).unwrap();
        assert_eq!(surface(&s), preview);
    }
}

#[test]
fn symmetry_settings_remain_available_on_non_raster_layers_with_selected_channels() {
    let mut s = setup("rgb", 8);
    subset(&mut s);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    s.execute("paint.setSymmetry", json!({"mode":"vertical"})).unwrap();
    s.execute("paint.symmetryDisable", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("path.set", json!({"name":"work","path":{"subpaths":[{"closed":false,"knots":[[16,0],[16,24]]}]}})).unwrap();
    s.execute("paint.symmetryFromPath", json!({"name":"work"})).unwrap();
    s.execute("paint.symmetryDisable", json!({})).unwrap();
}

#[test]
fn remove_cannot_bypass_component_protection_by_creating_a_new_layer() {
    for colors in [json!([0]), json!([0, 2])] {
        let mut s = setup("rgb", 8);
        s.execute("channel.target", json!({"colors":colors})).unwrap();
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("paint.remove", json!({"points":[[8,8],[22,12]],"size":9,"newLayer":true})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn hdr_toning_cannot_flatten_a_document_when_colour_components_are_targeted() {
    let mut s = setup("rgb", 32);
    subset(&mut s);
    let before = s.active().unwrap().doc.clone();
    assert!(!s.is_enabled("image.adjustments.hdrToning"));
    assert!(s.execute("image.adjustments.hdrToning", json!({})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn sparse_neighbourhood_filters_keep_output_support_separate_from_sampling_extent() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":2048,"height":2048,"depth":32,"background":"transparent"})).unwrap();
    s.edit("small opaque patch", |doc, id| {
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 32), &[0.2, 0.4, 0.6, 1.0]);
        Ok(())
    })
    .unwrap();
    subset(&mut s);
    let before = surface(&s);
    s.execute("filter.other.highPass", json!({"radius":2})).unwrap();
    let after = surface(&s);
    assert!(after.tile_count() <= 4, "materialized {} tiles", after.tile_count());
    assert_eq!(after.pixel(1500, 1500), before.pixel(1500, 1500));
    unchanged(&before, &after);
    assert_ne!(before.pixel(16, 16), after.pixel(16, 16));
}

#[test]
fn singleton_copy_keeps_scalar_tone_for_alpha_and_mask_clipboard_consumers() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for mask in [false, true] {
                let mut s = setup(mode, depth);
                s.edit("uniform", |doc, id| {
                    let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                    let mut value = vec![0.3; surf.channels()];
                    *value.last_mut().unwrap() = 0.65;
                    surf.fill_rect(Rect::new(0, 0, 32, 24), &value);
                    Ok(())
                })
                .unwrap();
                s.execute("edit.copy", json!({"target":{"colors":[0]}})).unwrap();
                if mask {
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    s.execute("channel.target", json!({"channel":"mask"})).unwrap();
                } else {
                    s.execute("channel.new", json!({"fill":"white"})).unwrap();
                }
                s.execute("edit.paste", json!({})).unwrap();
                let st = s.active().unwrap();
                let dest = if mask { &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface } else { &st.doc.channels[0].surface };
                let expected = if mode == "cmyk" { 0.7 } else { 0.3 };
                assert!((dest.pixel(8, 8)[0] - expected).abs() < 0.005, "{mode}{depth} mask={mask}");
            }
        }
    }
}

#[test]
fn named_layers_resolve_identically_for_painting_and_component_restoration() {
    for (command, mut params) in [
        ("paint.stroke", json!({"points":[[8,8],[22,12]],"size":9,"color":"#ffffff","seed":42})),
        ("paint.blur", json!({"points":[[8,8],[22,12]],"size":9,"strength":80})),
        ("filter.blur.gaussianBlur", json!({"radius":2})),
        ("filter.blur.average", json!({})),
        ("image.adjustments.equalize", json!({})),
        ("image.adjustments.shadowsHighlights", json!({"shadowAmount":50})),
        ("filter.lensCorrection", json!({"distortion":20})),
        ("paint.magicEraser", json!({"x":10,"y":10,"tolerance":255})),
        ("paint.backgroundEraser", json!({"points":[[8,8],[22,12]],"size":9,"tolerance":100})),
        ("path.fill", json!({"color":"#ffffff"})),
        ("path.stroke", json!({"size":9,"color":"#ffffff"})),
    ] {
        let mut s = setup("rgb", 32);
        let id = s.active().unwrap().active_layer.unwrap();
        let name = s.active().unwrap().doc.layer(id).unwrap().name.clone();
        let before = surface(&s);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color":"#333333"})).unwrap();
        let active = surface(&s);
        s.execute("path.set", json!({"path":{"subpaths":[{"closed":true,"knots":[[2,2],[25,2],[25,20],[2,20]]}]}})).unwrap();
        subset(&mut s);
        params["layer"] = json!(name);
        s.execute(command, params).unwrap();
        assert!(surface(&s) == active, "{command} must not change the active layer");
        let changed = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
        unchanged(&before, changed);
        assert!(&before != changed, "{command} must change the named layer");
    }
}

#[test]
fn patch_and_content_aware_move_previews_protect_the_same_components_as_commit() {
    use photocraft_engine::retouch_cmds;
    for command in ["paint.patch", "paint.contentAwareMove"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":60})).unwrap();
        s.execute("paint.pencil", json!({"points":[[68,30],[74,30]],"size":6,"color":"#ff0000"})).unwrap();
        s.execute("select.lasso", json!({"points":[[60,20],[84,20],[84,40],[60,40]]})).unwrap();
        subset(&mut s);
        let id = s.active().unwrap().active_layer.unwrap();
        let before = surface(&s);
        let params = json!({"offset":[-40,0]});
        let preview = if command == "paint.patch" {
            retouch_cmds::patch_preview(&s, &params, 16384, 1).unwrap().0
        } else {
            retouch_cmds::content_aware_move_preview(&s, &params).unwrap().run(&photocraft_engine::jobs::JobCtx::new()).unwrap().0
        };
        unchanged(&before, preview.layer(id).unwrap().surface().unwrap());
        assert_eq!(surface(&s), before, "preview is read-only");
        s.execute(command, params).unwrap();
        assert_eq!(&surface(&s), preview.layer(id).unwrap().surface().unwrap());
    }
}

#[test]
fn lens_blur_keeps_original_transparency_depth_and_component_noise_streams() {
    let mut composite = flat_rgb();
    let mut partial = flat_rgb();
    for s in [&mut composite, &mut partial] {
        s.edit("focus alpha", |doc, id| {
            let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 16, 24), &[0.2, 0.2, 0.2, 128.0 / 255.0]);
            surf.fill_rect(Rect::new(16, 0, 32, 24), &[0.8, 0.2, 0.8, 128.0 / 255.0]);
            Ok(())
        })
        .unwrap();
    }
    subset(&mut partial);
    let before = surface(&partial);
    for s in [&mut composite, &mut partial] {
        s.execute("filter.blur.lensBlur", json!({"radius":8,"depthMap":"transparency","focalDistance":128})).unwrap();
    }
    let expected = surface(&composite).pixel(15, 12);
    let actual = surface(&partial).pixel(15, 12);
    for k in [0, 2] {
        assert!((expected[k] - actual[k]).abs() < 1e-5, "{expected:?} != {actual:?}");
    }
    unchanged(&before, &surface(&partial));
    let mut mono = flat_rgb();
    let mut color = flat_rgb();
    for s in [&mut mono, &mut color] {
        subset(s);
    }
    mono.execute("filter.blur.lensBlur", json!({"radius":1,"noise":30,"seed":42,"monochromatic":true})).unwrap();
    color.execute("filter.blur.lensBlur", json!({"radius":1,"noise":30,"seed":42,"monochromatic":false})).unwrap();
    assert_eq!(surface(&mono).pixel(4, 12)[0], surface(&mono).pixel(4, 12)[2]);
    assert_ne!(surface(&color).pixel(4, 12)[0], surface(&color).pixel(4, 12)[2]);
}

#[test]
fn coloured_clouds_and_fibers_keep_native_render_colours_on_selected_channels() {
    for command in ["filter.render.clouds", "filter.render.fibers"] {
        let mut composite = flat_rgb();
        let mut partial = flat_rgb();
        for s in [&mut composite, &mut partial] {
            s.tools.foreground = [1.0, 0.0, 0.0, 1.0];
            s.tools.background = [0.0, 0.0, 1.0, 1.0];
        }
        subset(&mut partial);
        let before = surface(&partial);
        for s in [&mut composite, &mut partial] {
            s.execute(command, json!({"seed":42})).unwrap();
        }
        let expected = surface(&composite);
        let actual = surface(&partial);
        unchanged(&before, &actual);
        for y in 0..24 {
            for x in 0..32 {
                for k in [0, 2] {
                    assert_eq!(actual.pixel(x, y)[k].to_bits(), expected.pixel(x, y)[k].to_bits(), "{command}");
                }
            }
        }
        assert_ne!(actual.pixel(8, 8)[0], actual.pixel(8, 8)[2]);
    }
}

#[test]
fn lighting_effects_keep_the_original_alpha_texture_for_colour_targets() {
    let mut composite = flat_rgb();
    let mut partial = flat_rgb();
    for s in [&mut composite, &mut partial] {
        s.edit("alpha relief", |doc, id| {
            let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(0, 0, 16, 24), &[0.5, 0.5, 0.5, 0.2]);
            surf.fill_rect(Rect::new(16, 0, 32, 24), &[0.5, 0.5, 0.5, 0.8]);
            Ok(())
        })
        .unwrap();
    }
    subset(&mut partial);
    let before = surface(&partial);
    for s in [&mut composite, &mut partial] {
        s.execute("filter.render.lightingEffects", json!({"texture":"alpha","height":100,"gloss":-100,"metallic":-100})).unwrap();
    }
    let expected = surface(&composite);
    let actual = surface(&partial);
    unchanged(&before, &actual);
    for y in 0..24 {
        for x in 0..32 {
            for k in [0, 2] {
                assert_eq!(actual.pixel(x, y)[k].to_bits(), expected.pixel(x, y)[k].to_bits());
            }
        }
    }
}

#[test]
fn mode_conversion_normalizes_a_now_complete_colour_set_to_composite() {
    let mut s = setup("cmyk", 8);
    s.execute("channel.target", json!({"colors":[0,1,2]})).unwrap();
    s.execute("image.mode.rgb", json!({})).unwrap();
    assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Composite);
    assert!(color_channels::selected(&s).is_none());
    assert!(s.active().unwrap().channel_view.selected_colors.is_empty());
    assert!(s.is_enabled("layer.translate"));
    assert!(s.is_enabled("filter.cameraRaw"));
}

#[test]
fn colour_only_edits_cannot_create_an_unrestricted_render_layer() {
    let mut s = setup("rgb", 8);
    subset(&mut s);
    let before = s.active().unwrap().doc.clone();
    for command in ["paint.remove", "filter.render.tree", "filter.render.flame", "filter.render.pictureFrame"] {
        assert!(s.execute(command, json!({"newLayer":true})).is_err(), "{command}");
        assert_eq!(&*s.active().unwrap().doc, &*before);
    }
}

#[test]
fn selections_with_a_selected_default_and_stored_holes_edit_the_remaining_components() {
    for depth in [8, 16, 32] {
        let mut s = setup("rgb", depth);
        let before = surface(&s);
        s.edit("selected default with a hole", |doc, _| {
            let mut mask = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
            mask.fill_rect(Rect::new(0, 0, 4, 4), &[0.0]);
            doc.selection = Some(mask);
            Ok(())
        })
        .unwrap();
        subset(&mut s);
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let after = surface(&s);
        unchanged(&before, &after);
        assert_eq!(before.pixel(1, 1), after.pixel(1, 1));
        for k in [0, 2] {
            assert!((after.pixel(10, 10)[k] - (1.0 - before.pixel(10, 10)[k])).abs() < 0.005, "depth {depth} component {k}");
        }
    }
}

#[test]
fn geometric_filters_on_colour_sets_preserve_background_identity_and_linked_masks() {
    for (command, params) in [
        ("filter.lensCorrection", json!({"distortion":20})),
        ("filter.liquify", json!({"strokes":[{"tool":"forwardWarp","size":12,"points":[[10,10],[15,13]]}]})),
    ] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":32,"height":24})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("coloured background and linked mask", |doc, _| {
            let layer = doc.layer_mut(id).unwrap();
            layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 12, 24), &[0.2, 0.4, 0.6, 1.0]);
            let mut mask = photocraft_doc::LayerMask::reveal_all();
            mask.surface.fill_rect(Rect::new(0, 0, 10, 24), &[0.3]);
            layer.mask = Some(mask);
            Ok(())
        })
        .unwrap();
        let original = s.active().unwrap().doc.layer(id).unwrap().clone();
        let before = surface(&s);
        subset(&mut s);
        s.execute(command, params).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        assert_eq!(layer.name, original.name);
        assert_eq!(layer.locks, original.locks);
        assert_eq!(layer.mask, original.mask);
        unchanged(&before, layer.surface().unwrap());
        assert!(layer.surface().unwrap() != &before, "{command}");
    }
}

#[test]
fn colour_adjustments_keep_the_original_cross_component_inputs() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for (cmd, params) in [
                ("image.adjustments.channelMixer", json!({"red":[0,100,0,0]})),
                ("image.adjustments.hueSaturation", json!({"saturation":-30})),
                ("image.adjustments.vibrance", json!({"vibrance":40})),
                ("image.adjustments.colorBalance", json!({"midtones":[20,-10,30]})),
                ("image.adjustments.photoFilter", json!({"color":"#ff0000","density":70})),
            ] {
                let mut control = setup(mode, depth);
                let before = surface(&control);
                control.execute(cmd, params.clone()).unwrap();
                let composite = surface(&control);
                for indices in [vec![0], vec![0, 2]] {
                    let mut s = setup(mode, depth);
                    s.execute("channel.target", json!({"colors":indices})).unwrap();
                    s.execute(cmd, params.clone()).unwrap();
                    let mut expected = composite.clone();
                    let colors = color_channels::ColorChannels::from_indices(indices, before.format().mode.color_channels()).unwrap();
                    color_channels::restrict_surface(&before, &mut expected, colors).unwrap();
                    assert!(surface(&s) == expected, "{mode} {depth} {cmd}");
                }
            }
        }
    }
}

#[test]
fn named_patch_and_content_aware_move_targets_do_not_edit_the_active_layer() {
    for command in ["paint.patch", "paint.contentAwareMove"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":60})).unwrap();
        s.execute("layer.new.layer", json!({"name":"other"})).unwrap();
        s.execute("edit.fill", json!({"color":"#ffffff"})).unwrap();
        s.execute("paint.pencil", json!({"points":[[68,30],[74,30]],"size":6,"color":"#ff0000"})).unwrap();
        s.execute("select.lasso", json!({"points":[[60,20],[84,20],[84,40],[60,40]]})).unwrap();
        subset(&mut s);
        let before = surface(&s);
        s.execute(command, json!({"offset":[-40,0],"layer":"Background"})).unwrap();
        assert!(surface(&s) == before, "{command} must leave the active layer unchanged");
    }
}

#[test]
fn colour_only_rendering_needs_an_existing_raster_destination() {
    let mut s = setup("rgb", 8);
    s.execute("select.deselectLayers", json!({})).unwrap();
    subset(&mut s);
    let before = s.active().unwrap().doc.clone();
    for cmd in ["filter.render.tree", "filter.render.flame", "filter.render.pictureFrame"] {
        assert!(!s.is_enabled(cmd));
        assert!(s.execute(cmd, json!({})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
}

#[test]
fn rgb_float_curves_keep_endpoint_extrapolation_in_selected_planes() {
    let params = json!({"points":[[0,0],[128,180],[255,255]]});
    for value in [-0.2, 1.5, 2.0] {
        for selected in [false, true] {
            for indices in [vec![0], vec![0, 2]] {
                let mut s = setup("rgb", 32);
                s.edit("HDR fixture", |doc, id| {
                    doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &[value, 0.4, value, 0.7]);
                    Ok(())
                })
                .unwrap();
                if selected {
                    s.execute("select.rect", json!({"x":0,"y":0,"width":16,"height":24})).unwrap();
                }
                let before = surface(&s);
                let mut control = Session::new();
                control.add_document((*s.active().unwrap().doc).clone(), None);
                control.select_layer(s.active().unwrap().active_layer.unwrap()).unwrap();
                control.execute("image.adjustments.curves", params.clone()).unwrap();
                let expected = surface(&control);
                s.execute("channel.target", json!({"colors":indices})).unwrap();
                s.execute("image.adjustments.curves", params.clone()).unwrap();
                let after = surface(&s);
                for (x, y) in [(10, 10), (25, 10)] {
                    for k in 0..4 {
                        if indices.contains(&k) {
                            assert!((after.pixel(x, y)[k] - expected.pixel(x, y)[k]).abs() < 1e-5, "value {value}, selection {selected}, component {k}");
                        } else {
                            assert_eq!(after.pixel(x, y)[k].to_bits(), before.pixel(x, y)[k].to_bits());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn clear_and_cut_in_mask_view_preserve_pixels_and_clear_only_selected_mask_samples() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for command in ["edit.clear", "edit.cut"] {
                let mut s = setup(mode, depth);
                s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                subset(&mut s);
                s.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
                s.execute("tools.setColors", json!({"background":"#000000"})).unwrap();
                s.edit("Feathered selection", |doc, _| {
                    let mut mask = Surface::new(photocraft_color::PixelFormat::GRAY8);
                    mask.fill_rect(Rect::new(4, 4, 12, 12), &[0.5]);
                    doc.selection = Some(mask);
                    Ok(())
                })
                .unwrap();
                let before = surface(&s);
                let layer = s.active().unwrap().active_layer.unwrap();
                let mask = |s: &Session| s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.clone();
                let mask_before = mask(&s);
                let steps = s.active().unwrap().history.past_len();
                s.execute(command, json!({})).unwrap();
                assert_eq!(surface(&s), before, "{command} {mode}@{depth}");
                let cleared = mask(&s);
                assert!((cleared.sample_channel(8, 8, 0) - (1.0 - 128.0 / 255.0)).abs() < 0.005);
                assert_eq!(cleared.sample_channel(20, 20, 0), 1.0);
                assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
                if command == "edit.cut" {
                    let clip = s.clipboard.as_ref().unwrap();
                    assert_eq!(clip.surface.format().mode, photocraft_doc::ColorMode::Grayscale);
                    assert_eq!(clip.surface.sample_channel(8, 8, 0), 1.0);
                    assert!(clip.components.is_none());
                }
                s.undo();
                assert_eq!(mask(&s), mask_before);
                assert_eq!(surface(&s), before);
                s.redo();
                assert_eq!(mask(&s), cleared);
            }
        }
    }
}

#[test]
fn refused_mask_cut_and_clear_leave_the_pixels_mask_and_clipboard_unchanged() {
    for command in ["edit.cut", "edit.clear"] {
        let mut s = setup("rgb", 16);
        let before = surface(&s);
        assert!(s.execute(command, json!({"target":"mask"})).is_err());
        assert_eq!(surface(&s), before);
        s.execute("edit.copy", json!({})).unwrap();
        let clipboard = s.clipboard.as_ref().unwrap().surface.clone();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        subset(&mut s);
        s.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
        s.edit("Lock layer", |doc, id| {
            doc.layer_mut(id.unwrap()).unwrap().locks.all = true;
            Ok(())
        })
        .unwrap();
        let doc = s.active().unwrap().doc.clone();
        let layer = s.active().unwrap().active_layer.unwrap();
        let steps = s.active().unwrap().history.past_len();
        assert!(s.execute(command, json!({})).is_err());
        assert_eq!(surface(&s), before);
        assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().mask, doc.layer(layer).unwrap().mask);
        assert_eq!(s.clipboard.as_ref().unwrap().surface, clipboard);
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }
}

#[test]
fn desaturate_honours_explicit_native_targets_over_the_panel_selection() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for target in [json!({"color":1}), json!({"colors":[1]}), json!("pixels")] {
                let mut s = setup(mode, depth);
                let before = surface(&s);
                let mut control = Session::new();
                let st = s.active().unwrap();
                control.add_document((*st.doc).clone(), None);
                control.select_layer(st.active_layer.unwrap()).unwrap();
                control.execute("image.adjustments.desaturate", json!({})).unwrap();
                let expected = surface(&control);
                subset(&mut s);
                s.execute("image.adjustments.desaturate", json!({"target":target})).unwrap();
                let actual = surface(&s);
                assert_ne!(actual, before, "{mode}@{depth} {target}");
                for y in 0..24 {
                    for x in 0..32 {
                        for k in 0..actual.channels() {
                            let want = if target == "pixels" || k == 1 { &expected } else { &before };
                            assert_eq!(
                                actual.sample_channel(x, y, k).to_bits(),
                                want.sample_channel(x, y, k).to_bits(),
                                "{mode}@{depth} {target} component {k}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn mask_view_routes_implicit_edits_and_tone_editors_to_the_mask_before_colour_sets() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for view in ["gray", "overlay"] {
                for colors in [vec![0], vec![0, 2]] {
                    let mut s = setup(mode, depth);
                    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                    s.execute("channel.target", json!({"colors":colors})).unwrap();
                    s.execute("view.layerMask", json!({"mode":view})).unwrap();
                    let before = surface(&s);
                    let st = s.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let descriptor = photocraft_engine::channel_cmds::isolated_curve_channel(st).unwrap();
                    assert_eq!(descriptor.gray, Some(photocraft_engine::channel_cmds::GraySurface::LayerMask));
                    assert!(descriptor.components.is_none());
                    s.execute("image.adjustments.invert", json!({})).unwrap();
                    assert_eq!(surface(&s), before);
                    let mask = |s: &Session| s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.clone();
                    assert_eq!(mask(&s).sample_channel(8, 8, 0), 0.0);
                    s.execute("paint.stroke", json!({"points":[[8,8]],"size":6,"hardness":1.0,"color":"#ffffff"})).unwrap();
                    assert!(mask(&s).sample_channel(8, 8, 0) > 0.9);
                    s.execute("filter.blur.gaussianBlur", json!({"radius":2})).unwrap();
                    s.execute("image.adjustments.desaturate", json!({})).unwrap();
                    assert_eq!(surface(&s), before);
                    let mask_before = mask(&s);
                    // An explicit native target can still override the viewed mask.
                    s.execute("image.adjustments.invert", json!({"target":{"colors":colors}})).unwrap();
                    assert_ne!(surface(&s), before);
                    assert_eq!(mask(&s), mask_before);
                    s.execute("channel.target", json!({"colors":colors})).unwrap();
                    assert!(photocraft_engine::mask_view_cmds::current(s.active().unwrap()).is_none(), "selecting colours leaves mask view");
                    s.execute("view.layerMask", json!({"mode":view})).unwrap();
                    s.execute("channel.target", json!({"channel":"composite"})).unwrap();
                    assert!(photocraft_engine::mask_view_cmds::current(s.active().unwrap()).is_none(), "selecting the composite leaves mask view");
                }
            }
        }
    }
}

#[test]
fn copy_and_cut_resolve_the_same_layer_for_composite_and_colour_targets() {
    for indices in [vec![], vec![0], vec![0, 2]] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":32})).unwrap();
        s.execute("layer.new.layer", json!({"name":"source"})).unwrap();
        s.execute("edit.fill", json!({"color":"#804020"})).unwrap();
        let source_id = s.active().unwrap().active_layer.unwrap();
        let source_before = surface(&s);
        s.execute("layer.new.layer", json!({"name":"active"})).unwrap();
        s.execute("edit.fill", json!({"color":"#00ff80"})).unwrap();
        let active_before = surface(&s);
        s.execute("tools.setColors", json!({"background":"#000000"})).unwrap();
        if !indices.is_empty() {
            s.execute("channel.target", json!({"colors":indices})).unwrap();
        }
        s.execute("edit.copy", json!({"layer":"source"})).unwrap();
        let copied = &s.clipboard.as_ref().unwrap().surface;
        let expected = if indices.is_empty() { &active_before } else { &source_before };
        assert_eq!(copied.pixel(4, 4)[0], expected.pixel(4, 4)[0], "Copy {indices:?}");
        assert_eq!(surface(&s), active_before);
        assert_eq!(s.active().unwrap().doc.layer(source_id).unwrap().surface().unwrap(), &source_before);
        s.execute("edit.cut", json!({"layer":"source"})).unwrap();
        let source_after = s.active().unwrap().doc.layer(source_id).unwrap().surface().unwrap();
        let clipboard = &s.clipboard.as_ref().unwrap().surface;
        if indices.is_empty() {
            assert!(source_after == &source_before);
            assert_eq!(surface(&s).pixel(4, 4)[3], 0.0);
            assert_eq!(clipboard.pixel(4, 4), active_before.pixel(4, 4));
        } else {
            assert!(surface(&s) == active_before);
            for k in 0..4 {
                let expected = if indices.contains(&k) { 0.0 } else { source_before.pixel(4, 4)[k] };
                assert_eq!(source_after.pixel(4, 4)[k], expected);
            }
            assert!((clipboard.pixel(4, 4)[0] - source_before.pixel(4, 4)[0]).abs() < 1e-6);
        }
    }
}

#[test]
fn named_red_eye_target_does_not_bypass_channel_protection_on_the_active_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":64,"depth":32})).unwrap();
    s.execute("layer.new.layer", json!({"name":"other"})).unwrap();
    s.execute("edit.fill", json!({"color":"#404040"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"eye"})).unwrap();
    s.edit("red pupil", |doc, id| {
        let mut values = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                let distance = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
                values.extend(if distance <= 1.2 {
                    [1.0; 4]
                } else if distance <= 8.0 {
                    [0.95, 0.1, 0.1, 1.0]
                } else {
                    [0.38, 0.28, 0.16, 1.0]
                });
            }
        }
        doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap().write_region(Rect::new(0, 0, 64, 64), &values);
        Ok(())
    })
    .unwrap();
    s.execute("channel.target", json!({"colors":[1,2]})).unwrap();
    let before = surface(&s);
    let error = s.execute("paint.redEye", json!({"x":32,"y":32,"layer":"other"})).unwrap_err();
    assert!(error.to_string().contains("no red-eye pixels"));
    assert!(surface(&s) == before);
}

#[test]
fn only_pixel_editing_paint_commands_follow_a_colour_set() {
    for id in ["paint.patch", "paint.contentAwareMove", "paint.stroke", "paint.magicEraser"] {
        assert!(photocraft_engine::channel_cmds::follows_target(id), "{id}");
    }
    for id in ["paint.mixerLoad", "paint.setSymmetry", "paint.symmetryFromPath", "paint.symmetryDisable"] {
        assert!(!photocraft_engine::channel_cmds::follows_target(id), "{id}");
    }
    // ⌥-click with the Mixer Brush samples a fill layer with a colour set targeted: it edits
    // nothing, so it is not refused with "Rasterize the layer".
    let mut s = setup("rgb", 8);
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#4080c0"})).unwrap();
    subset(&mut s);
    let loaded = s.execute("paint.mixerLoad", json!({"x": 4.0, "y": 4.0})).unwrap();
    assert_eq!(loaded["loaded"], json!(true));
}
