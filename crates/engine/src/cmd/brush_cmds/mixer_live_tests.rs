use super::mixer_live::LiveMixer;
use super::*;
use photocraft_color::PixelFormat;
use std::sync::Arc;

/// Build format-specific red/green sampling layers and an empty painting target.
fn fixture(fmt: PixelFormat) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":192,"height":96,"background":"#808080"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Source"})).unwrap();
    s.edit("fixture", |doc, id| {
        let (surface, _) = crate::channel_cmds::target_surface(doc, *id, &json!({}))?;
        *surface = Surface::new(fmt);
        surface.fill_rect(Rect::new(0, 0, 96, 96), &[1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::new(96, 0, 192, 96), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({"name":"Paint"})).unwrap();
    s.edit("depth", |doc, id| {
        *crate::channel_cmds::target_surface(doc, *id, &json!({}))?.0 = Surface::new(fmt);
        Ok(())
    })
    .unwrap();
    s.tools.brush.mixer.sample_all_layers = true;
    s
}

/// Snapshot the active raster target for exact pixel comparisons without borrowing the session.
fn target(s: &Session) -> Surface {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}

/// All-layer previews match replay and Undo exactly across supported depths without editing source layers.
#[test]
fn mixer_live_empty_layer_exact_commit_undo_source_preservation_and_chunking() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut s = fixture(fmt);
        assert!(s.tools.brush.mixer.sample_all_layers);
        let before = s.active().unwrap().doc.clone();
        let params = json!({"points":[[40,48]],"seed":47,"size":24,"wet":100,"mix":100,"cleanAfterStroke":false});
        let mut live = LiveMixer::begin(&s, &params).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        assert!(live.doc.layer(id).unwrap().surface().unwrap().rgba(40, 48)[3] > 0.0);
        assert_eq!(target(&s).rgba(40, 48)[3], 0.0, "preview must not edit the session");
        let mut points = vec![StrokePoint::new(40.0, 48.0, 1.0)];
        for x in [56.0, 72.0, 88.0, 104.0, 120.0, 136.0, 152.0] {
            let p = StrokePoint::new(x, 48.0, 1.0);
            live.push(&[p]).unwrap();
            points.push(p);
        }
        let preview = live.doc.layer(id).unwrap().surface().unwrap().clone();
        let mut full = params.clone();
        full["points"] = json!(points);
        s.execute("paint.mixerBrush", full).unwrap();
        assert_eq!(preview.read_region(before.bounds()), target(&s).read_region(before.bounds()), "live/replay tolerance is exactly zero");
        for old in &before.layers {
            if old.id != id {
                assert_eq!(
                    old.surface().unwrap().read_region(before.bounds()),
                    s.active().unwrap().doc.layer(old.id).unwrap().surface().unwrap().read_region(before.bounds())
                );
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(target(&s).read_region(before.bounds()), before.layer(id).unwrap().surface().unwrap().read_region(before.bounds()));
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(target(&s).read_region(before.bounds()), preview.read_region(before.bounds()));
    }
}

/// Current-layer mixing matches the frozen kernel; dropping a preview and cleaning the load paint nothing.
#[test]
fn mixer_live_current_layer_matches_reference_and_cancels() {
    let mut s = fixture(PixelFormat::RGBA32F);
    let p = json!({"points":[[30,30],[60,34],[100,60]],"seed":9,"size":16,"sampleAllLayers":false,"wet":65,"mix":70,"load":90});
    let mut expected = target(&s);
    let brush = resolve_brush(&s, &p, "paint.mixerBrush").unwrap();
    let mut state = photocraft_paint::mixer::MixerState::default();
    state.load(s.tools.foreground);
    photocraft_paint::mixer::apply_mixer_stroke(
        &mut expected,
        None,
        &Stroke { brush, points: parse_points(&p, "paint.mixerBrush").unwrap() },
        &MixerSettings { wet: 0.65, mix: 0.70, load: 0.90, sample_all_layers: false, ..Default::default() },
        &mut state,
        None,
        false,
    );
    let live = LiveMixer::begin(&s, &p).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let actual = live.doc.layer(id).unwrap().surface().unwrap().read_region(s.active().unwrap().doc.bounds());
    let expected = expected.read_region(s.active().unwrap().doc.bounds());
    let max = actual.iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(max <= 1e-6, "reference error {max}");
    drop(live);
    assert_eq!(target(&s).rgba(30, 30)[3], 0.0);
    s.execute("brush.mixer.clean", json!({})).unwrap();
    s.execute("paint.mixerBrush", json!({"points":[[30,30],[60,30]],"sampleAllLayers":false,"wet":0})).unwrap();
    assert_eq!(target(&s).rgba(30, 30)[3], 0.0, "clean brush must stay empty");
}

/// Sampling retains a spatial two-color load while leaving document pixels and history alone.
#[test]
fn mixer_sample_is_spatial_and_never_edits_document() {
    let mut s = fixture(PixelFormat::RGBA8);
    let before = s.active().unwrap().doc.clone();
    s.execute("brush.mixer.sample", json!({"points":[[96,48]],"size":40,"brush":{"hardness":1.0,"pressureSize":false}})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    let load = s.tools.mixer.spatial.as_ref().unwrap();
    assert_eq!(load.len(), 4096);
    assert!(load.iter().any(|p| p[0] > 0.9 && p[1] < 0.1 && p[3] > 0.9));
    assert!(load.iter().any(|p| p[1] > 0.9 && p[0] < 0.1 && p[3] > 0.9));
    s.execute("paint.mixerBrush", json!({"points":[[48,48]],"size":40,"wet":0,"mix":0,"brush":{"hardness":1.0,"pressureSize":false}})).unwrap();
    assert!(target(&s).rgba(40, 48)[0] > 0.9);
    assert!(target(&s).rgba(56, 48)[1] > 0.9);
    assert!(s.tools.mixer.spatial.is_some(), "sample persists across strokes");
}

/// Smoothing tails match replay and invalid input cannot mutate the last valid preview.
#[test]
fn mixer_live_smoothing_tail_and_invalid_input() {
    let mut s = fixture(PixelFormat::RGBA16);
    let p = json!({"points":[[20,40]],"size":12,"seed":42,"brush":{"smoothing":{"amount":0.6,"catchUpOnEnd":true}}});
    let mut live = LiveMixer::begin(&s, &p).unwrap();
    live.push(&[StrokePoint::new(60.0, 40.0, 0.6)]).unwrap();
    live.push(&[StrokePoint::new(100.0, 48.0, 1.0)]).unwrap();
    let before = live.doc.clone();
    assert!(live.push(&[StrokePoint::new(f64::NAN, 0.0, 1.0)]).is_err());
    assert!(std::sync::Arc::ptr_eq(&before, &live.doc));
    let mut commit = p.clone();
    commit["points"] = json!([[20, 40, 1], [60, 40, 0.6], [100, 48, 1]]);
    s.execute("paint.mixerBrush", commit).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    assert_eq!(
        live.doc.layer(id).unwrap().surface().unwrap().read_region(s.active().unwrap().doc.bounds()),
        target(&s).read_region(s.active().unwrap().doc.bounds())
    );
    assert!(s.execute("paint.mixerBrush", json!({"points":[[0,0]],"size":5001})).is_err());
    assert!(s.execute("brush.mixer.sample", json!({"points":[[0,0],[1,1]]})).is_err());
}

/// Composite sampling honors source masks and blending while current-layer mode ignores lower paint.
#[test]
fn mixer_sampling_obeys_masks_opacity_blend_and_current_layer_switch() {
    let mut s = fixture(PixelFormat::RGBA8);
    let source = s.active().unwrap().doc.layers.iter().find(|l| l.name == "Source").unwrap().id;
    s.edit("source composition", |doc, _| {
        let layer = doc.layer_mut(source).unwrap();
        layer.opacity = 0.4;
        layer.blend = photocraft_color::BlendMode::Multiply;
        layer.mask = Some(photocraft_doc::LayerMask::hide_all());
        Ok(())
    })
    .unwrap();
    let expected = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(39, 39, 40, 40)).px[0];
    s.execute("brush.mixer.sample", json!({"points":[[40,40]],"size":8})).unwrap();
    let actual = s.tools.mixer.reservoir.unwrap();
    for c in 0..3 {
        assert!((actual[c] - expected[c]).abs() < 1e-5);
    }
    s.execute("brush.mixer.clean", json!({})).unwrap();
    s.execute("paint.mixerBrush", json!({"points":[[40,40],[80,40]],"size":8,"wet":100,"mix":100,"sampleAllLayers":false})).unwrap();
    assert_eq!(target(&s).rgba(50, 40)[3], 0.0, "current empty layer provides no paint");
    s.execute("paint.mixerBrush", json!({"points":[[40,40],[80,40]],"size":8,"wet":100,"mix":100,"sampleAllLayers":true})).unwrap();
    assert!(target(&s).rgba(50, 40)[3] > 0.9, "all layers supplies the visible composite");
}

/// Later overlapping dabs pick up freshly deposited paint from the live composite.
#[test]
fn mixer_overlapping_dabs_resample_the_evolving_composite() {
    let mut s = fixture(PixelFormat::RGBA32F);
    s.execute("tools.setColors", json!({"foreground":"#0000ff"})).unwrap();
    let params = json!({"points":[[50,48]],"size":20,"seed":1,"wet":100,"mix":50,"cleanAfterStroke":false});
    let mut live = LiveMixer::begin(&s, &params).unwrap();
    live.push(&[StrokePoint::new(54.0, 48.0, 1.0), StrokePoint::new(50.0, 48.0, 1.0)]).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let pixel = live.doc.layer(id).unwrap().surface().unwrap().rgba(50, 48);
    assert!(pixel[2] > 0.5, "deposited blue feeds back into later pickups: {pixel:?}");
}

/// The shared live commit keeps shown pixels and one Undo step, refusing stale document snapshots.
#[test]
fn mixer_prepared_commit_uses_displayed_pixels_and_rejects_stale_documents() {
    let mut s = fixture(PixelFormat::RGBA8);
    let p = json!({"points":[[20,40]],"size":16,"target":"pixels","seed":55});
    let mut live = LiveMixer::begin(&s, &p).unwrap();
    live.push(&[StrokePoint::new(80.0, 40.0, 0.6)]).unwrap();
    let shown = live.doc.clone();
    let id = s.active().unwrap().active_layer.unwrap();
    let steps = s.active().unwrap().history.past_len();
    let commit = live.prepare_commit(&mut s).unwrap();
    assert!(s.live_commit.is_some());
    s.execute("paint.mixerBrush", commit.clone()).unwrap();
    assert!(s.live_commit.is_none());
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(target(&s).read_region(shown.bounds()), shown.layer(id).unwrap().surface().unwrap().read_region(shown.bounds()));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("paint.mixerBrush", commit).unwrap();
    assert_eq!(target(&s).read_region(shown.bounds()), shown.layer(id).unwrap().surface().unwrap().read_region(shown.bounds()));
    let stale = LiveMixer::begin(&s, &p).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(stale.prepare_commit(&mut s).is_err());
}

/// Load and clean affect tool state without dropping a floating selection or creating document history.
#[test]
fn mixer_load_commands_preserve_floating_selection_and_history() {
    let mut s = fixture(PixelFormat::RGBA8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.float", json!({"dx":8,"dy":0,"copy":true})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    for (command, params) in [("brush.mixer.sample", json!({"points":[[96,48]],"size":24})), ("brush.mixer.load", json!({})), ("brush.mixer.clean", json!({}))]
    {
        s.execute(command, params).unwrap();
        assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(crate::float_cmds::floating(s.active().unwrap()).is_some());
    }
}

/// Malformed reservoir input is rejected and automatic reload follows the selected setting.
#[test]
fn mixer_rejects_malformed_load_and_retains_or_reloads_paint() {
    let mut s = fixture(PixelFormat::RGBA8);
    let before = s.active().unwrap().doc.clone();
    for state in [json!({"spatial":[]}), json!({"spatial":[[1,0,0,1]]}), json!({"amount":"bad"}), json!({"reservoir":[1e300,0,0,1]})] {
        assert!(s.execute("paint.mixerBrush", json!({"points":[[40,40]],"mixerState":state})).is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    assert!(s.execute("brush.mixer.load", json!({"color":[1e300,0,0,1]})).is_err());
    s.execute("brush.mixer.load", json!({"color":"#ff0000"})).unwrap();
    s.execute("paint.mixerBrush", json!({"points":[[40,40],[80,40]],"size":10,"loadAfterStroke":false,"cleanAfterStroke":false,"load":10})).unwrap();
    assert!(s.tools.mixer.amount > 0.0 && s.tools.mixer.amount < 1.0);
    s.execute("paint.mixerBrush", json!({"points":[[40,40],[80,40]],"size":10,"loadAfterStroke":true,"load":10})).unwrap();
    assert_eq!(s.tools.mixer.amount, 1.0);
}

/// Large tips keep off-canvas layer pixels and commit the exact preview with reversible history.
#[test]
fn mixer_large_tip_retains_off_canvas_paint_and_commits_the_live_result() {
    let mut s = fixture(PixelFormat::RGBA8);
    let source = s.active().unwrap().doc.clone();
    let params = json!({"points":[[96,48]],"size":1024,"wet":0,"mix":0,"color":"#1122ee","brush":{"pressureSize":false}});
    let live = LiveMixer::begin(&s, &params).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
    assert!(shown.rgba(-100, 48)[3] > 0.99, "off-canvas layer pixels remain paintable");
    let params = live.prepare_commit(&mut s).unwrap();
    s.execute("paint.mixerBrush", params).unwrap();
    assert_eq!(target(&s), shown);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, *source);
}

/// Large pointer jumps must replay identically instead of turning a valid stroke into an input-limit error.
#[test]
fn mixer_long_segments_are_interpolated_and_replay_matches_live() {
    let mut s = fixture(PixelFormat::RGBA8);
    let p = json!({"points":[[20,48],[5020,48]],"seed":71,"size":2,"sampleAllLayers":false,"wet":0,"mix":0,"load":100,"color":"#ff0000"});
    let live = LiveMixer::begin(&s, &p).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let expected = live.doc.layer(id).unwrap().surface().unwrap().clone();
    s.execute("paint.mixerBrush", p).unwrap();
    for x in [20, 1020, 2068, 4090, 5019] {
        assert_eq!(target(&s).rgba(x, 48), expected.rgba(x, 48));
        assert!(target(&s).rgba(x, 48)[3] > 0.0);
    }
}

/// Authorization, validation, unknown commands and missing targets all discard the single-use offer.
#[test]
fn mixer_prepared_offer_is_dropped_on_every_early_dispatch_failure() {
    for failure in 0..4 {
        let mut s = fixture(PixelFormat::RGBA8);
        let live = LiveMixer::begin(&s, &json!({"points":[[20,20]],"size":4})).unwrap();
        let p = live.prepare_commit(&mut s).unwrap();
        let before = s.active().unwrap().doc.clone();
        let result = match failure {
            0 => {
                s.authorize = Some(|_, _| Err(EngineError::Other("denied".into())));
                s.execute("paint.mixerBrush", p)
            }
            1 => s.execute("paint.mixerBrush", json!("invalid params")),
            2 => s.execute("unknown.command", p),
            _ => {
                s.active_mut().unwrap().active_layer = None;
                s.execute("paint.mixerBrush", p)
            }
        };
        assert!(result.is_err());
        assert!(s.live_commit.is_none(), "failed dispatch retained the preview: {failure}");
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    }
}

/// A stroke beyond the live tile budget still renders on command replay and remains undoable.
#[test]
fn mixer_command_outgrows_the_preview_tile_budget_without_losing_the_stroke() {
    let mut s = fixture(PixelFormat::RGBA8);
    let points: Vec<_> = (0..257).map(|i| [128.0 + (i % 17) as f64 * 512.0, 128.0 + (i / 17) as f64 * 512.0]).collect();
    let p = json!({"points":points,"size":2,"brush":{"spacing":10.0,"smoothing":{"amount":0.0}},"seed":44,
        "sampleAllLayers":false,"wet":0,"mix":0,"load":100,"color":"#ff0000"});
    let before = s.active().unwrap().doc.clone();
    let err = LiveMixer::begin(&s, &p).err().expect("bounded preview");
    assert!(err.to_string().contains("256 tiles"), "{err}");
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    s.execute("paint.mixerBrush", p).unwrap();
    // Spacing deliberately leaves gaps; inspect the final tile's stroke region instead
    // of assuming the exact release pixel is a dab center.
    let end = Rect::new(600, 7776, 672, 7840);
    assert!(target(&s).read_region(end).as_chunks::<4>().0.iter().any(|px| px[3] > 0.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(target(&s).read_region(end).as_chunks::<4>().0.iter().all(|px| px[3] == 0.0));
}

/// An empty sample reports an error while retaining the previous paint load and document.
#[test]
fn mixer_empty_samples_preserve_the_previous_load_and_document() {
    let mut s = fixture(PixelFormat::RGBA8);
    s.execute("brush.mixer.sample", json!({"points": [[96,48]], "size": 40})).unwrap();
    let state = s.tools.mixer.clone();
    let doc = s.active().unwrap().doc.clone();
    for params in [json!({"points": [[20,20]], "size": 8, "sampleAllLayers": false}), json!({"points": [[-100,-100]], "size": 8, "sampleAllLayers": true})] {
        let error = s.execute("brush.mixer.sample", params).unwrap_err().to_string();
        assert!(error.contains("contains no paint"), "{error}");
        assert_eq!(s.tools.mixer, state);
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc));
    }
}

/// Rotating a hard round sampling grid must give zero coverage beyond the footprint, not panic.
#[test]
fn mixer_rotated_hard_tip_samples_safely_and_preserves_the_document() {
    for angle in [45.0, -45.0, 135.0] {
        let mut s = fixture(PixelFormat::RGBA8);
        s.execute("tools.setBrush", json!({"brush":{"size":40,"angle":angle,"hardness":1.0,"pressureSize":false}})).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.execute("brush.mixer.sample", json!({"points":[[48,48]]})).unwrap();
        let load = s.tools.mixer.spatial.as_ref().unwrap();
        assert_eq!(load.len(), 4096);
        assert!(load.iter().any(|c| c[3] == 0.0));
        assert!(load.iter().any(|c| c[0] > 0.9 && c[3] > 0.9));
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
}

/// The default stays current-layer-only; all-layer sampling is an explicit user choice.
#[test]
fn mixer_default_does_not_sample_other_layers() {
    assert!(!MixerSettings::default().sample_all_layers);
    assert!(!Session::new().tools.brush.mixer.sample_all_layers);
}

/// Large session tips and spatial reservoirs stay out of each journaled stroke.
#[test]
fn mixer_shared_live_commit_keeps_the_journal_compact() {
    let mut s = fixture(PixelFormat::RGBA8);
    s.execute("brush.mixer.sample", json!({"points":[[96,48]],"size":40})).unwrap();
    s.tools.brush.tip = TipShape::Sampled(GrayTile::from_fn(256, 256, |_, _| 1.0));
    for x in [40.0, 60.0, 80.0] {
        let p = json!({"points":[[x,48]],"size":12,"seed":7,"wet":0,"mix":0});
        let live = LiveMixer::begin(&s, &p).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
        let params = live.prepare_commit(&mut s).unwrap();
        assert!(s.live_commit.is_some());
        let result = s.execute("paint.mixerBrush", params).unwrap();
        assert_eq!(result["fromLive"], true);
        assert_eq!(target(&s), shown);
        assert!(s.live_commit.is_none());
        let params = &s.journal.last().unwrap().1;
        for field in ["brush", "mixerState", "color"] {
            assert!(params.get(field).is_none(), "{field}");
        }
        assert!(serde_json::to_vec(params).unwrap().len() < 1024);
    }
}

/// Preview limits never reject valid command sizes, including the maximum session size.
#[test]
fn mixer_large_brushes_replay_after_the_preview_budget() {
    for (size, pressure) in [(2000, 1.0), (5000, 1.0)] {
        let mut s = fixture(PixelFormat::RGBA8);
        let before = s.active().unwrap().doc.clone();
        let params = json!({"points":[[96,48,pressure]],"size":size,"wet":0,"mix":0,"sampleAllLayers":false,
            "color":"#ff0000","brush":{"pressureSize":true,"hardness":1.0}});
        assert!(LiveMixer::begin(&s, &params).is_err());
        s.execute("paint.mixerBrush", params).unwrap();
        assert!(target(&s).rgba(96, 48)[0] > 0.9);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(*s.active().unwrap().doc, *before);
    }
}

/// Long strokes past the live preview's sample and dab budgets still render on command replay
/// (Stroke Path with Simulate Pressure sends up to 200,000 samples), as main painted them.
#[test]
fn mixer_command_paints_strokes_beyond_the_preview_sample_and_dab_budgets() {
    // 70,000 samples a pixel apart (above 65,536), or 2,000 samples 170 px apart (about 340,000 dabs).
    for (count, step, budget) in [(70_000, 1, "65536"), (2_000, 170, "250000 dabs")] {
        let mut s = fixture(PixelFormat::RGBA8);
        // Zig-zagging along one row, one dab per pixel.
        let points: Vec<_> = (0..count).map(|i: i32| [f64::from(10 + (i * step % 340 - 170).abs()), 48.0]).collect();
        let p = json!({"points":points,"size":2,"brush":{"spacing":0.5,"hardness":1.0,"pressureSize":false,"smoothing":{"amount":0.0}},
            "seed":7,"sampleAllLayers":false,"wet":0,"mix":0,"load":100,"color":"#ff0000"});
        let err = LiveMixer::begin(&s, &p).err().expect("bounded preview");
        assert!(err.to_string().contains(budget), "{err}");
        let steps = s.active().unwrap().history.past_len();
        s.execute("paint.mixerBrush", p).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert!(target(&s).rgba(100, 48)[0] > 0.9);
    }
}
