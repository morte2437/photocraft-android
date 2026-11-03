//! Opt-in Profile dithering preserves representation, alpha, wide paths and atomic refusal.

use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_engine::Session;
use photocraft_geom::{Rect, Size, TileCoord};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const GRAY: [f32; 4] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 83.0 / 255.0];

fn document(size: Size, surface: Surface, depth: SampleType) -> Document {
    let mut doc = Document::new("Dither", size, ColorMode::Rgb, depth);
    doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    doc.layers.push(Layer::new("Pixels", LayerContent::Raster(surface)));
    doc
}

fn session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

fn params(dither: Value) -> Value {
    json!({"profile": "linear-srgb", "intent": "relative", "bpc": false, "dither": dither})
}

fn converted(doc: Document, p: Value) -> Document {
    let mut s = session(doc);
    s.execute("edit.convertToProfile", p).unwrap();
    (*s.active().unwrap().doc).clone()
}

fn pixels(doc: &Document) -> &Surface {
    doc.layers[0].surface().unwrap()
}

/// A fractional constant distinguishes prequantization noise from ignored or post-byte noise.
#[test]
fn dither_rounds_fractional_colors_without_bias_or_alpha_noise() {
    let transform = Transform::new(Builtin::Srgb.profile(), Builtin::LinearSrgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    let mut fractional = [0.0; 3];
    transform.eval_fast(&GRAY[..3], &mut fractional);
    assert!((fractional[0] * 255.0 - 55.04443).abs() < 0.02, "the CMS fixture must have a fractional destination value");
    let before = document(Size::new(256, 256), Surface::with_default(PixelFormat::RGBA8, &GRAY), SampleType::U8);
    let ordinary = converted(before.clone(), params(json!(false)));
    assert_eq!((pixels(&ordinary).pixel(0, 0)[0] * 255.0).round() as u8, 55);
    let mut s = session(before.clone());
    s.execute("edit.convertToProfile", params(json!(true))).unwrap();
    let after = s.active().unwrap().doc.clone();
    assert!(pixels(&after).read_region(after.bounds()) != pixels(&ordinary).read_region(ordinary.bounds()), "dither must change fractional U8 colors");
    let tile = pixels(&after).tile(TileCoord::new(0, 0)).unwrap();
    let mut total = 0u64;
    let mut raised = 0;
    for px in tile.bytes().as_chunks::<4>().0 {
        assert!(px[0] == 55 || px[0] == 56, "dither quantizes neighboring destination values");
        assert_eq!(px[0], px[1]);
        assert_eq!(px[0], px[2]);
        assert_eq!(px[3], 83, "alpha is copied byte-for-byte");
        total += u64::from(px[0]);
        raised += usize::from(px[0] == 56);
    }
    assert!(raised > 0 && raised < 65536, "ignored dithering cannot pass this fixture");
    assert!((total as f64 / 65536.0 - 55.04443).abs() < 0.02);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(*s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, after);
}

/// Missing and redundant tiles must encode the same finite image, including outside padding.
#[test]
fn implicit_and_materialized_defaults_dither_identically() {
    let implicit = Surface::with_default(PixelFormat::RGBA8, &GRAY);
    let mut stored = implicit.clone();
    let tile = stored.solid_tile(&GRAY);
    stored.put_tiles([(TileCoord::new(0, 0), tile.clone()), (TileCoord::new(-2, 1), tile.clone()), (TileCoord::new(1_000_000, -1_000_000), tile)]);
    let size = Size::new(257, 259);
    let a = converted(document(size, implicit, SampleType::U8), params(json!(true)));
    let b = converted(document(size, stored, SampleType::U8), params(json!(true)));
    assert_eq!(pixels(&a).read_region(a.bounds()), pixels(&b).read_region(b.bounds()));
    assert_eq!(pixels(&a).default_bytes(), pixels(&b).default_bytes());
    for (x, y) in [(-512, 256), (256_000_000, -256_000_000), (300, 300), (256, 259)] {
        assert_eq!(pixels(&a).pixel(x, y), pixels(&b).pixel(x, y));
        assert_eq!(pixels(&b).pixel(x, y), pixels(&b).default_pixel());
    }
    assert!(pixels(&b).tile_count() <= 6, "no bounding box between distant tiles");
}

/// Shared source tiles need independent absolute-coordinate patterns; sparse content survives.
#[test]
fn shared_tiles_and_distant_nondefault_pixels_use_actual_coordinates() {
    let mut source = Surface::new(PixelFormat::RGBA8);
    let tile = source.solid_tile(&GRAY);
    source.put_tiles([(TileCoord::new(0, 0), tile.clone()), (TileCoord::new(1, 0), tile)]);
    for (x, y) in [(-512, 0), (256_000_003, 256_000_007)] {
        source.write_region(Rect::from_xywh(x, y, 1, 1), &GRAY);
    }
    let mut independent = source.clone();
    independent.put_tiles([(TileCoord::new(1, 0), independent.solid_tile(&GRAY))]);
    let size = Size::new(512, 256);
    let a = converted(document(size, source, SampleType::U8), params(json!(true)));
    let b = converted(document(size, independent, SampleType::U8), params(json!(true)));
    assert_eq!(pixels(&a).read_region(a.bounds()), pixels(&b).read_region(b.bounds()));
    assert_ne!(pixels(&a).tile(TileCoord::new(0, 0)).unwrap().bytes().to_vec(), pixels(&a).tile(TileCoord::new(1, 0)).unwrap().bytes().to_vec());
    for (x, y) in [(-512, 0), (256_000_003, 256_000_007)] {
        let px = pixels(&a).pixel(x, y);
        assert!([55.0, 56.0].contains(&(px[0] * 255.0).round()));
        assert_eq!((px[3] * 255.0).round() as u8, 83);
        assert_eq!(pixels(&a).pixel(x + 1, y), pixels(&a).default_pixel());
    }
    assert_eq!(pixels(&a).tile_count(), 4);
}

/// Defaults, wide documents (including U8 caches), same-profile and mismatched surfaces stay old.
#[test]
fn old_paths_and_integer_endpoints_remain_unchanged() {
    let base = document(Size::new(8, 6), Surface::with_default(PixelFormat::RGBA8, &GRAY), SampleType::U8);
    let ordinary = converted(base.clone(), params(json!(false)));
    for p in [json!({"profile": "linear-srgb", "intent": "relative", "bpc": false}), params(Value::Null)] {
        assert_eq!(converted(base.clone(), p), ordinary);
    }
    for depth in [SampleType::U16, SampleType::F32] {
        for sample in [depth, SampleType::U8] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let doc = document(Size::new(8, 6), Surface::with_default(format, &GRAY), depth);
            assert_eq!(converted(doc.clone(), params(json!(true))), converted(doc, params(json!(false))));
        }
    }
    for target in ["srgb", "linear-srgb"] {
        let mut doc = base.clone();
        if target != "srgb" {
            doc.layers[0].content = LayerContent::Raster(Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true), &[0.5, 0.3]));
        }
        let mut p = params(json!(false));
        p["profile"] = json!(target);
        let mut q = p.clone();
        q["dither"] = json!(true);
        assert_eq!(converted(doc.clone(), p), converted(doc, q));
    }
    let mut endpoints = Surface::new(PixelFormat::RGBA8);
    endpoints.write_region(Rect::from_xywh(0, 0, 2, 1), &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    let doc = document(Size::new(2, 1), endpoints, SampleType::U8);
    let a = converted(doc.clone(), params(json!(true)));
    let b = converted(doc, params(json!(false)));
    assert_eq!(pixels(&a).read_region(a.bounds()), pixels(&b).read_region(b.bounds()));
}

/// The alpha index follows channel counts when converting to and from four-color CMYK.
#[test]
fn cmyk_channels_do_not_receive_alpha_or_drop_it() {
    for from_cmyk in [false, true] {
        let mut doc = document(Size::new(16, 16), Surface::with_default(PixelFormat::RGBA8, &GRAY), SampleType::U8);
        let mut p = params(json!(true));
        if from_cmyk {
            doc.mode = ColorMode::Cmyk;
            doc.icc_profile = Some(Builtin::CoatedCmyk.profile().to_bytes());
            doc.layers[0].content =
                LayerContent::Raster(Surface::with_default(PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true), &[0.2, 0.4, 0.6, 0.1, GRAY[3]]));
        } else {
            p["profile"] = json!("coated-cmyk");
        }
        let after = converted(doc, p);
        let surface = pixels(&after);
        for px in surface.tile(TileCoord::new(0, 0)).unwrap().bytes().chunks_exact(surface.channels()).take(16) {
            assert_eq!(*px.last().unwrap(), 83);
        }
        assert_eq!(surface.channels(), if from_cmyk { 4 } else { 5 });
    }
}

/// Sparse oversize, aggregate allocation, hostile coordinates and wrong flags preserve history.
#[test]
fn refusals_are_early_and_atomic() {
    let surface = Surface::with_default(PixelFormat::RGBA8, &GRAY);
    let mut aggregate = document(Size::new(32768, 16384), surface.clone(), SampleType::U8);
    for _ in 0..4 {
        aggregate.layers.push(Layer::new("Shared input", LayerContent::Raster(surface.clone())));
    }
    let mut hostile = surface.clone();
    hostile.put_tiles([(TileCoord::new(i32::MAX, 0), hostile.solid_tile(&GRAY))]);
    let mut cases = vec![
        (document(Size::new(1 << 20, 1 << 20), surface.clone(), SampleType::U8), json!(true)),
        (aggregate, json!(true)),
        (document(Size::new(1, 1), hostile, SampleType::U8), json!(true)),
    ];
    for value in [json!(1), json!("false"), json!([]), json!({})] {
        cases.push((document(Size::new(8, 6), surface.clone(), SampleType::U8), value));
    }
    for (doc, flag) in cases {
        let mut s = session(doc);
        s.execute("layer.new.layer", json!({"name": "Redo sentinel"})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let state = s.active().unwrap();
        let revision = state.revision;
        let selection = state.selected_layers.clone();
        let active = state.active_layer;
        let history = (state.history.past_len(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned));
        assert!(s.execute("edit.convertToProfile", params(flag)).is_err());
        let state = s.active().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!(state.revision, revision);
        assert_eq!(state.selected_layers, selection);
        assert_eq!(state.active_layer, active);
        assert_eq!((state.history.past_len(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned)), history);
        assert!(state.history.can_redo());
    }
}

/// Absent tiles of a transparent layer stay absent: only the stored tile is dithered.
#[test]
fn transparent_layers_stay_sparse() {
    let p = json!({"profile": "display-p3", "dither": true});
    let stored = |size: Size| {
        let mut surface = Surface::with_default(PixelFormat::RGBA8, &[0.0; 4]);
        let tile = surface.solid_tile(&GRAY);
        surface.put_tiles([(TileCoord::new(1, 1), tile)]);
        document(size, surface, SampleType::U8)
    };
    let large = converted(stored(Size::new(1024, 1024)), p.clone());
    assert_eq!(pixels(&large).tile_count(), 1, "a transparent layer must not become dense");
    let small = converted(stored(Size::new(512, 512)), p);
    let tile = |doc: &Document| pixels(doc).tile(TileCoord::new(1, 1)).unwrap().bytes().to_vec();
    assert_eq!(tile(&large), tile(&small), "the stored tile is dithered as before");
    assert_eq!(pixels(&large).pixel(700, 700)[3], 0.0);
}
