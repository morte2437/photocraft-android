use std::sync::Arc;

use photocraft_geom::Rect;
use serde_json::json;

use super::*;
use crate::Session;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":12,"depth":depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("pattern", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::new(0, 0, 8, 12), &[0.2, 0.6, 0.8, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn rejected(s: &mut Session, id: &str, key: &str, value: Value) {
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    let journal = s.journal.clone();
    let active = s.active().unwrap().active_layer;
    let error = s.execute(id, json!({key:value})).unwrap_err();
    assert!(matches!(error, EngineError::BadParams { .. }), "{id}: {error}");
    let error = error.to_string();
    assert!(error.contains(id) && error.contains(key), "{error}");
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc), "{id}: {key}");
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.active().unwrap().active_layer, active);
    assert_eq!(s.journal, journal);
    assert!(s.jobs().is_empty());
}

#[test]
fn every_core_filter_rejects_unknown_keys_without_editing() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        for spec in crate::filters::specs().into_iter().filter(|s| s.id != "filter.lastFilter") {
            rejected(&mut s, spec.id, "radus", json!(3));
        }
    }
}

#[test]
fn core_filter_numbers_reject_wrong_types_and_documented_range_violations() {
    // Each numeric field in the core registry, independently of the validation table.
    let cases = [
        ("blur.gaussianBlur", "radius", 0.1, 1000.0),
        ("blur.boxBlur", "radius", 1.0, 2000.0),
        ("blur.motionBlur", "angle", -360.0, 360.0),
        ("blur.motionBlur", "distance", 1.0, 2000.0),
        ("blur.radialBlur", "amount", 1.0, 100.0),
        ("blur.radialBlur", "centerX", 0.0, 1.0),
        ("blur.radialBlur", "centerY", 0.0, 1.0),
        ("blur.surfaceBlur", "radius", 1.0, 100.0),
        ("blur.surfaceBlur", "threshold", 2.0, 255.0),
        ("sharpen.unsharpMask", "amount", 1.0, 500.0),
        ("sharpen.unsharpMask", "radius", 0.1, 1000.0),
        ("sharpen.unsharpMask", "threshold", 0.0, 255.0),
        ("sharpen.smartSharpen", "amount", 1.0, 500.0),
        ("sharpen.smartSharpen", "radius", 0.1, 64.0),
        ("sharpen.smartSharpen", "reduceNoise", 0.0, 100.0),
        ("noise.addNoise", "amount", 0.1, 400.0),
        ("noise.median", "radius", 1.0, 500.0),
        ("noise.dustAndScratches", "radius", 1.0, 500.0),
        ("noise.dustAndScratches", "threshold", 0.0, 255.0),
        ("pixelate.mosaic", "cellSize", 2.0, 200.0),
        ("stylize.emboss", "angle", -180.0, 180.0),
        ("stylize.emboss", "height", 1.0, 100.0),
        ("stylize.emboss", "amount", 1.0, 500.0),
        ("distort.twirl", "angle", -999.0, 999.0),
        ("distort.pinch", "amount", -100.0, 100.0),
        ("distort.spherize", "amount", -100.0, 100.0),
        ("distort.wave", "generators", 1.0, 999.0),
        ("distort.wave", "wavelengthMin", 1.0, 998.0),
        ("distort.wave", "wavelengthMax", 2.0, 999.0),
        ("distort.wave", "amplitudeMin", 1.0, 998.0),
        ("distort.wave", "amplitudeMax", 1.0, 999.0),
        ("distort.ripple", "amount", -999.0, 999.0),
        ("other.highPass", "radius", 0.1, 1000.0),
        ("other.minimum", "radius", 0.2, 500.0),
        ("other.maximum", "radius", 0.2, 500.0),
    ];
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        for (suffix, key, min, max) in cases {
            let id = format!("filter.{suffix}");
            for value in [json!("big"), json!(true), Value::Null, json!([]), json!({}), json!(min - 0.01), json!(max + 0.01)] {
                rejected(&mut s, &id, key, value);
            }
            // Large limits are validated without allocating a large blur kernel.
            for value in [min, max] {
                validate_params(&id, &json!({key:value})).unwrap();
            }
        }
        for key in ["horizontal", "vertical"] {
            rejected(&mut s, "filter.other.offset", key, json!("4"));
        }
    }
}

#[test]
fn core_filter_choices_booleans_seeds_and_shared_fields_are_validated() {
    let mut s = session(8);
    for (suffix, key, choices) in [
        ("blur.radialBlur", "method", "spin|zoom"),
        ("blur.radialBlur", "quality", "draft|good|best"),
        ("noise.addNoise", "distribution", "uniform|gaussian"),
        ("distort.spherize", "mode", "normal|horizontalOnly|verticalOnly"),
        ("distort.wave", "type", "sine|triangle|square"),
        ("distort.wave", "undefinedAreas", "wrap|repeat"),
        ("distort.ripple", "size", "small|medium|large"),
        ("distort.polarCoordinates", "mode", "rectangularToPolar|polarToRectangular"),
        ("other.minimum", "preserve", "squareness|roundness"),
        ("other.maximum", "preserve", "squareness|roundness"),
        ("other.offset", "undefinedAreas", "wrap|repeat|transparent"),
    ] {
        let id = format!("filter.{suffix}");
        for value in [json!("typo"), json!(false), json!(1), Value::Null] {
            rejected(&mut s, &id, key, value);
        }
        for value in choices.split('|') {
            validate_params(&id, &json!({key:value})).unwrap();
        }
    }
    for value in [json!("true"), json!(0), Value::Null] {
        rejected(&mut s, "filter.noise.addNoise", "monochromatic", value);
    }
    for id in ["filter.noise.addNoise", "filter.distort.wave"] {
        for value in [json!(-1), json!(4_294_967_296_u64), json!(0.5), json!("12")] {
            rejected(&mut s, id, "seed", value);
        }
        // A valid u32 seed must not be saturated through an i32 helper.
        let result = s.execute(id, json!({"seed":u32::MAX})).unwrap();
        assert_eq!(result["filter"]["seed"], json!(u32::MAX));
    }
    for (key, value) in [("layer", json!("2")), ("target", json!("typo")), ("target", json!({"channel":-1})), ("coalesce", json!(false))] {
        rejected(&mut s, "filter.blur.gaussianBlur", key, value);
    }
}

#[test]
fn extended_filter_choices_match_their_specs_and_reject_unlisted_values() {
    let mut s = session(8);
    let specs = crate::filters_ext::specs();
    let ids = [
        "filter.pixelate.mezzotint",
        "filter.stylize.diffuse",
        "filter.stylize.extrude",
        "filter.stylize.tiles",
        "filter.stylize.traceContour",
        "filter.stylize.wind",
        "filter.distort.displace",
        "filter.distort.shear",
        "filter.distort.zigZag",
        "filter.render.lensFlare",
        "filter.render.lightingEffects",
        "filter.blur.lensBlur",
        "filter.other.hsbHsl",
        "filter.video.deInterlace",
    ];
    for id in ids {
        let doc = specs.iter().find(|spec| spec.id == id).unwrap().params;
        for &(key, allowed) in choices(id).unwrap() {
            // The table repeats the registry's params doc, so the two cannot drift apart.
            assert!(doc.contains(&format!("\"{key}\":\"{allowed}\"")), "{id}.{key}");
            for value in [json!("zzbogus"), json!(false), json!(1), Value::Null] {
                rejected(&mut s, id, key, value);
            }
            for value in allowed.split('|') {
                validate_params(id, &json!({key:value})).unwrap();
            }
        }
    }
    // Keys `prepare` injects, and undocumented ones, stay accepted as before.
    s.execute("filter.stylize.tiles", json!({"count":4,"foreground":[1,0,0,1]})).unwrap();
    s.execute("filter.stylize.wind", json!({"direction":"fromLeft"})).unwrap();
}

const STRICT_EXTENDED: [&str; 3] = ["filter.blur.smartBlur", "filter.blur.shapeBlur", "filter.noise.reduceNoise"];

#[test]
fn strict_extended_filters_reject_bad_fields_without_editing() {
    let numbers = [
        ("filter.blur.smartBlur", "radius", 0.1, 100.0),
        ("filter.blur.smartBlur", "threshold", 0.1, 100.0),
        ("filter.blur.shapeBlur", "radius", 5.0, 1000.0),
        ("filter.noise.reduceNoise", "strength", 0.0, 10.0),
        ("filter.noise.reduceNoise", "preserveDetails", 0.0, 100.0),
        ("filter.noise.reduceNoise", "reduceColorNoise", 0.0, 100.0),
        ("filter.noise.reduceNoise", "sharpenDetails", 0.0, 100.0),
    ];
    let specs = crate::filters_ext::specs();
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        for (id, key, min, max) in numbers {
            let doc = specs.iter().find(|spec| spec.id == id).unwrap().params;
            assert!(doc.contains(&format!("\"{key}\":{min}..{max}=")), "{id}.{key}");
            for value in [json!("3"), json!(true), Value::Null, json!([]), json!({}), json!(min - 0.01), json!(max + 0.01), json!(1e300)] {
                rejected(&mut s, id, key, value);
            }
            // Validate large endpoints without allocating large blur kernels.
            for value in [min, max] {
                validate_params(id, &json!({key:value})).unwrap();
            }
        }
        for id in STRICT_EXTENDED {
            for (key, value) in [("radus", json!(3)), ("layer", json!("2")), ("target", json!("typo")), ("coalesce", json!(false))] {
                rejected(&mut s, id, key, value);
            }
            for value in [json!([]), json!(false), json!(3), json!("params")] {
                assert!(matches!(validate_params(id, &value), Err(EngineError::BadParams { .. })));
            }
        }
        for value in [json!("true"), json!(1), Value::Null, json!([]), json!({})] {
            rejected(&mut s, "filter.noise.reduceNoise", "removeJpegArtifact", value);
        }
    }
    for value in [false, true] {
        validate_params("filter.noise.reduceNoise", &json!({"removeJpegArtifact":value})).unwrap();
    }
    for (id, key, allowed) in [
        ("filter.blur.smartBlur", "quality", "high|medium|low"),
        ("filter.blur.smartBlur", "mode", "normal|edgeOnly|overlayEdge"),
        ("filter.blur.shapeBlur", "shape", "circle|ring|square|diamond|triangle|hexagon|star|heart|cross"),
    ] {
        let doc = specs.iter().find(|spec| spec.id == id).unwrap().params;
        assert!(doc.contains(&format!("\"{key}\":\"{allowed}\"")), "{id}.{key}");
        let mut s = session(8);
        for value in [json!("typo"), json!(false), json!(1), Value::Null] {
            rejected(&mut s, id, key, value);
        }
        for value in allowed.split('|') {
            validate_params(id, &json!({key:value})).unwrap();
        }
    }
}

#[test]
fn strict_extended_filters_validate_before_floating_selection_commit_and_jobs() {
    let mut s = session(16);
    s.execute("select.rect", json!({"x":0,"y":0,"width":4,"height":4})).unwrap();
    s.execute("select.float", json!({"dx":8})).unwrap();
    for id in STRICT_EXTENDED {
        rejected(&mut s, id, "radus", json!(3));
        let doc = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        let journal = s.journal.clone();
        let params = json!({"radus":3});
        assert!(matches!(s.start(id, params.clone()), Err(EngineError::BadParams { .. })));
        let spec = crate::commands::find(id).unwrap();
        assert!(matches!((spec.run)(&mut s, &params), Err(EngineError::BadParams { .. })));
        assert!(crate::float_cmds::floating(s.active().unwrap()).is_some());
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc));
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(s.journal, journal);
        assert!(s.jobs().is_empty());
    }
}

#[test]
fn strict_extended_filters_keep_defaults_targets_history_and_last_filter() {
    for depth in [8, 16, 32] {
        for id in STRICT_EXTENDED {
            let mut s = session(depth);
            let layer = s.active().unwrap().active_layer.unwrap();
            for params in [Value::Null, json!({}), json!({"layer":layer.0,"target":"pixels","coalesce":"extended"})] {
                let before = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                s.execute(id, params).unwrap();
                assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                let after = s.active().unwrap().doc.clone();
                s.undo();
                assert_eq!(*s.active().unwrap().doc, *before);
                s.redo();
                assert_eq!(*s.active().unwrap().doc, *after);
            }
            s.execute("filter.lastFilter", json!({})).unwrap();
            // The session injects this target when editing an alpha channel.
            s.execute("channel.new", json!({"fill":"white"})).unwrap();
            s.execute(id, json!({})).unwrap();
        }
    }
}

#[test]
fn stored_extended_filter_records_still_decode_tolerantly() {
    use photocraft_algo::{BlurQuality, BlurShape, FilterParams, SmartBlurMode};
    let smart = crate::filters::params_for("filter.blur.smartBlur", &json!({"radius":-2,"threshold":900,"quality":"old","mode":false,"extra":true})).unwrap();
    assert!(matches!(smart, FilterParams::SmartBlur { radius: 0.1, threshold: 100.0, quality: BlurQuality::High, mode: SmartBlurMode::Normal }));
    let shape = crate::filters::params_for("filter.blur.shapeBlur", &json!({"radius":2000,"shape":"old"})).unwrap();
    assert!(matches!(shape, FilterParams::ShapeBlur { radius: 1000.0, shape: BlurShape::Circle }));
    let noise = crate::filters::params_for("filter.noise.reduceNoise", &json!({"strength":-1,"preserveDetails":101,"removeJpegArtifact":"yes"})).unwrap();
    assert!(matches!(noise, FilterParams::ReduceNoise { strength: 0.0, preserve_details: 100.0, remove_jpeg_artifact: false, .. }));
}

#[test]
fn invalid_filters_leave_floating_selection_and_background_jobs_alone() {
    let mut s = session(16);
    s.execute("select.rect", json!({"x":0,"y":0,"width":4,"height":4})).unwrap();
    s.execute("select.float", json!({"dx":8})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    for params in [json!({"radius":"big"}), json!({"radus":3}), json!({"radius":5000}), json!({"radius":-4})] {
        assert!(s.start("filter.blur.gaussianBlur", params.clone()).is_err());
        let spec = crate::commands::find("filter.blur.gaussianBlur").unwrap();
        assert!((spec.run)(&mut s, &params).is_err());
        assert!(crate::float_cmds::floating(s.active().unwrap()).is_some());
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc));
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert!(s.jobs().is_empty());
    }
}

#[test]
fn valid_filters_keep_defaults_explicit_targets_and_undo_redo() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let layer = s.active().unwrap().active_layer.unwrap();
        let pixels = |s: &Session| s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 16, 12));
        let before = pixels(&s);
        for params in [Value::Null, json!({})] {
            let result = s.execute("filter.blur.gaussianBlur", params).unwrap();
            assert_eq!(result["filter"]["radius"], json!(1.0));
            assert_ne!(pixels(&s), before);
            s.undo();
            assert_eq!(pixels(&s), before);
        }
        s.execute("layer.new.layer", json!({})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        let result = s.execute("filter.blur.gaussianBlur", json!({"radius":3,"layer":layer.0,"target":"pixels","coalesce":"blur"})).unwrap();
        assert_eq!(result["filter"]["radius"], json!(3.0));
        let after = pixels(&s);
        assert_ne!(after, before);
        assert_eq!(s.active().unwrap().doc.layer(other).unwrap().surface().unwrap().tile_count(), 0);
        s.undo();
        assert_eq!(pixels(&s), before);
        s.redo();
        assert_eq!(pixels(&s), after);
    }
}

#[test]
fn injected_channel_targets_and_extended_filter_colours_keep_working() {
    let mut s = session(8);
    s.execute("channel.new", json!({"fill":"white"})).unwrap();
    let pixels = s.active().unwrap().doc.layers.clone();
    s.execute("filter.noise.addNoise", json!({"amount":40,"seed":2})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers, pixels);
    assert!(s.active().unwrap().doc.channels[0].surface.read_region(Rect::new(0, 0, 16, 12)).iter().any(|&v| v < 0.99));
    s.execute("channel.target", json!({"channel":"composite"})).unwrap();
    s.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    s.tools.background = [1.0, 0.0, 0.0, 1.0];
    let params = crate::filters_ext::prepare(&s, "filter.render.fibers", &json!({}));
    assert_eq!(params["foreground"], json!(s.tools.foreground));
    assert_eq!(params["background"], json!(s.tools.background));
    s.execute("filter.render.fibers", params).unwrap();
}
