use super::*;

#[test]
fn crop_rejects_rectangles_that_would_wrap() {
    let mut s = session();
    // 2^32 + 100 wrapped to `x = 100` through the narrowing casts: a crop rectangle
    // somewhere unrelated to the numbers passed.
    let err = s.execute("image.crop", json!({"x": 4_294_967_396_i64, "y": 0, "width": 10, "height": 10})).unwrap_err();
    assert!(err.to_string().contains("32-bit"), "{err}");
    // In-range rectangles past the canvas still work (clamped by the crop itself).
    s.execute("image.crop", json!({"x": -10, "y": -10, "width": 1000, "height": 1000})).unwrap();
}

/// A crop whose translation to the origin can't be represented in i32 is rejected before
/// anything changes (#959): `-i32::MIN` overflowed the negation in `crop_doc`, `x + width`
/// past i32::MAX silently shortened the crop, and an origin one past i32::MIN moved the
/// layer pixels beyond i32::MAX (a length-mismatch panic while translating surfaces).
#[test]
fn crop_rejects_origins_that_cannot_move_to_zero() {
    const MIN: i64 = i32::MIN as i64;
    const MAX: i64 = i32::MAX as i64;
    let cases = [
        json!({"x": MIN, "y": 0, "width": 1, "height": 1}),
        json!({"x": 0, "y": MIN, "width": 1, "height": 1}),
        json!({"x": MIN, "y": MIN, "width": 1, "height": 1}),
        json!({"x": MIN, "y": 0, "width": 1, "height": 1, "deleteCroppedPixels": false}),
        json!({"x": MAX - 5, "y": 0, "width": 10, "height": 1}),
        json!({"x": 0, "y": MAX - 5, "width": 1, "height": 10}),
        json!({"x": MIN + 1, "y": 0, "width": 1, "height": 1, "deleteCroppedPixels": false}),
        json!({"x": 0, "y": MIN + 1, "width": 1, "height": 1, "deleteCroppedPixels": false}),
    ];
    for p in cases {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let (rev, steps) = (s.active().unwrap().revision, s.active().unwrap().history.entries().len());
        let err = s.execute("image.crop", p.clone()).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{p}: {err}");
        assert!(err.to_string().contains("32-bit"), "{p}: {err}");
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&st.doc, &before), "{p}: document changed");
        assert_eq!(st.doc.size, Size::new(40, 20), "{p}");
        assert_eq!(st.doc.layers[1].surface().unwrap().content_bounds(), Rect::new(10, 5, 20, 15), "{p}");
        assert_eq!((st.revision, st.history.entries().len()), (rev, steps), "{p}: history step recorded");
    }
    // Control: a legal negative origin past the canvas still crops (and extends) the canvas.
    let mut s = session();
    s.execute("image.crop", json!({"x": -10, "y": -5, "width": 60, "height": 30})).unwrap();
    assert_eq!(doc(&s).size, Size::new(60, 30));
    assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(20, 10), vec![1.0, 0.0, 0.0, 1.0]);
    // A far-off crop that deletes the (non-overlapping) pixels has nothing left to move.
    let mut s = session();
    s.execute("image.crop", json!({"x": MIN + 1, "y": 0, "width": 1, "height": 1})).unwrap();
    assert_eq!(doc(&s).size, Size::new(1, 1));
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 5, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
        l.mask = Some(photocraft_doc::LayerMask::reveal_all());
        l.effects.items.push(Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// Document-wide conversions must move every stored video frame, not just the displayed one (#4956).
#[test]
fn crop_moves_stored_video_frames() {
    let mut s = session();
    s.edit("video", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let frame = l.surface().unwrap().clone();
        let mut v = photocraft_doc::VideoData::new(vec![frame.clone(), frame.clone()], 24.0);
        v.original_frames = Some(vec![frame.clone(), frame]);
        l.video = Some(v);
        Ok(())
    })
    .unwrap();
    s.execute("image.crop", json!({"x": -10, "y": -5, "width": 60, "height": 30})).unwrap();
    let v = doc(&s).layers[1].video.as_ref().unwrap();
    let red = vec![1.0, 0.0, 0.0, 1.0];
    for frame in v.frames.iter().chain(v.original_frames.iter().flatten()) {
        assert_eq!(frame.pixel(20, 10), red);
        assert_eq!(frame.pixel(5, 5)[3], 0.0);
    }
}

/// Delete Cropped Pixels on a shrinking crop deletes the cropped area from every stored video
/// frame (and the original footage), not just the displayed one (#4956).
#[test]
fn delete_cropped_pixels_crops_stored_video_frames() {
    let mut s = session();
    s.edit("video", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let red = l.surface().unwrap().clone();
        let mut blue = red.clone();
        blue.fill_rect(Rect::new(0, 0, 40, 20), &[0.0, 0.0, 1.0, 1.0]);
        let mut v = photocraft_doc::VideoData::new(vec![red.clone(), blue.clone()], 24.0);
        v.original_frames = Some(vec![red, blue]);
        l.video = Some(v);
        Ok(())
    })
    .unwrap();
    s.execute("image.crop", json!({"x": 15, "y": 0, "width": 25, "height": 20, "deleteCroppedPixels": true})).unwrap();
    assert_eq!(doc(&s).size, Size::new(25, 20));
    let v = doc(&s).layers[1].video.as_ref().unwrap();
    let originals = v.original_frames.as_ref().unwrap();
    assert_eq!((v.frames.len(), originals.len()), (2, 2));
    for frames in [&v.frames, originals] {
        // The red square (10..20, 5..15) keeps only its right half; the full blue frame
        // keeps exactly the 25×20 crop.
        assert_eq!(frames[0].content_bounds(), Rect::new(0, 5, 5, 15));
        assert_eq!(frames[0].pixel(2, 10), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(frames[1].content_bounds(), Rect::new(0, 0, 25, 20));
        assert_eq!(frames[1].pixel(24, 19), vec![0.0, 0.0, 1.0, 1.0]);
    }
}

/// Resample on with only a new resolution keeps the print size, as Photoshop's Image Size
/// does (an action's `imageSize` with `Rslt` alone); without `keepPrintSize` only the
/// resolution changes.
#[test]
fn image_size_keep_print_size_resamples_to_the_resolution() {
    let mut s = session();
    s.execute("image.imageSize", json!({"resolution": 144, "resample": "bicubic"})).unwrap();
    assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(40, 20), 144.0));
    s.execute("image.imageSize", json!({"resolution": 288, "resample": "bicubic", "keepPrintSize": true})).unwrap();
    assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(80, 40), 288.0));
    // An explicit side still wins.
    s.execute("image.imageSize", json!({"width": 20, "resolution": 72, "resample": "bicubic", "keepPrintSize": true})).unwrap();
    assert_eq!(doc(&s).size, Size::new(20, 10));
}

#[test]
fn image_size_scales_everything() {
    for resample in ["bicubic", "bicubicSmoother", "bicubicSharper", "bicubicAutomatic", "automatic", "bilinear", "nearest", "lanczos", "preserveDetails"] {
        let mut s = session();
        s.execute("image.imageSize", json!({"width": 80, "resample": resample})).unwrap();
        let d = doc(&s);
        assert_eq!(d.size, Size::new(80, 40), "{resample}");
        let l = &d.layers[1];
        let b = l.surface().unwrap().content_bounds();
        assert!((b.x0 - 20).abs() <= 1 && (b.x1 - 40).abs() <= 1, "{resample} {b:?}");
        let Effect::DropShadow(sh) = &l.effects.items[0] else { panic!() };
        assert_eq!(sh.distance, 10.0);
        assert_eq!(l.mask.as_ref().unwrap().surface.default_pixel(), vec![1.0]);
    }
}

/// #6019: Image Size on an Indexed Color document makes no colours the colour table doesn't have.
#[test]
fn image_size_resamples_indexed_color_with_nearest_neighbor() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 1, "y": 0, "width": 1, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    for resample in ["bicubic", "automatic", "bilinear", "lanczos", "preserveDetails"] {
        s.execute("image.imageSize", json!({"width": 8, "resample": resample})).unwrap();
        let st = s.active().unwrap();
        let surf = st.doc.layers[0].surface().unwrap();
        for x in 0..8 {
            let px = surf.pixel(x, 0);
            assert!(px[..3] == [1.0, 0.0, 0.0] || px[..3] == [0.0, 0.0, 1.0], "{resample}: no blended colour at x {x}: {px:?}");
        }
        s.execute("history.undo", json!({})).unwrap();
    }
}

/// The canvas border stays opaque after Image Size (it used to fade into transparency, so a
/// Background or a 200 % export got a translucent frame), and a full selection stays full.
/// #1544: 64 x 48 to 300000 x 300000 (90 billion pixels) started allocating resampled bands
/// without a budget. It is refused before any allocation and the document is untouched; a
/// change of resolution only, and an ordinary resize, still work.
#[test]
fn image_size_refuses_a_raster_past_the_budget() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let before = (doc(&s).size, s.active().unwrap().history.past_len());
    let err = s.execute("image.imageSize", json!({"width": 300_000, "height": 300_000})).unwrap_err().to_string();
    assert!(err.contains("megapixels"), "{err}");
    assert_eq!((doc(&s).size, s.active().unwrap().history.past_len()), before);
    s.execute("image.imageSize", json!({"width": 300_000, "height": 300_000, "resample": "none", "resolution": 300})).unwrap();
    s.execute("image.imageSize", json!({"width": 128, "height": 96})).unwrap();
    assert_eq!(doc(&s).size, Size::new(128, 96));
}

#[test]
fn image_size_keeps_canvas_edges_opaque() {
    for depth in [8, 16, 32] {
        for (w, resample) in
            [(60, "bicubic"), (41, "lanczos"), (15, "bilinear"), (77, "preserveDetails"), (21, "bicubicSharper"), (47, "automatic"), (9, "bicubicAutomatic")]
        {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 30, "height": 20, "depth": depth, "background": "#336699"})).unwrap();
            s.execute("select.all", json!({})).unwrap();
            s.execute("image.imageSize", json!({"width": w, "resample": resample})).unwrap();
            let d = doc(&s);
            let flat = photocraft_compose::flatten(d);
            let (w, h) = (flat.rect.width() as usize, flat.rect.height() as usize);
            let mid = flat.px[(h / 2) * w + w / 2];
            assert!((mid[3] - 1.0).abs() < 1e-3, "{depth} {resample}: {mid:?}");
            for p in &flat.px {
                let off = p.iter().zip(mid).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                assert!(off < 0.01, "{depth} {resample} {w}: {p:?} vs {mid:?}");
            }
            let sel = d.selection.as_ref().unwrap();
            let (x1, y1) = (d.size.width as i32 - 1, d.size.height as i32 - 1);
            for (x, y) in [(0, 0), (x1, 0), (0, y1), (x1, y1)] {
                assert!(sel.pixel(x, y)[0] > 0.99, "{depth} {resample}: selection at ({x},{y})");
            }
        }
    }
}

#[test]
fn resolution_only_change() {
    let mut s = session();
    let r = s.execute("image.imageSize", json!({"resolution": 300, "resample": "none", "width": 999})).unwrap();
    assert_eq!(doc(&s).size, Size::new(40, 20));
    assert_eq!(doc(&s).resolution_dpi, 300.0);
    // #1815: the result is the size the document has, not the ignored request.
    assert_eq!(r, json!({"width": 40, "height": 20}));
}

/// Type keeps its pixel size across a resolution change: resampled 2x at twice the ppi it is
/// twice as tall (not 4x), with Resample off it stays, and a later re-render doesn't jump.
#[test]
fn image_size_with_a_new_resolution_scales_type_with_the_pixels_only() {
    for (params, k) in [(json!({"width": 200, "height": 200, "resolution": 144}), 2), (json!({"resolution": 144, "resample": "none"}), 1)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi", "size": 12})).unwrap();
        let height = |s: &Session| doc(s).layers.last().unwrap().surface().unwrap().content_bounds().height();
        let h0 = height(&s);
        assert!(h0 > 4, "{h0}");
        s.execute("image.imageSize", params.clone()).unwrap();
        assert!(height(&s).abs_diff(h0 * k) <= 2, "{params}: {h0} -> {}", height(&s));
        let d = doc(&s);
        let LayerContent::Text(t) = &d.layers.last().unwrap().content else { panic!("type layer") };
        let mut t = t.clone();
        crate::type_cmds::refresh(d, &mut t);
        let after = t.cache.unwrap().content_bounds().height();
        assert!(after.abs_diff(h0 * k) <= 2, "{params}: re-rendered {h0} -> {after}");
    }
}

#[test]
fn image_size_reports_the_size_it_applied() {
    // #1815: every path reports the document's size afterwards, at every depth.
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
        let size = |s: &Session| json!({"width": doc(s).size.width, "height": doc(s).size.height});
        for p in [
            json!({"width": 999, "height": 777, "resample": "none", "resolution": 150}),
            json!({"width": 32}),
            json!({"height": 96, "resample": "nearest"}),
            json!({"width": 96, "height": 72, "resample": "none"}),
            json!({}),
        ] {
            let r = s.execute("image.imageSize", p.clone()).unwrap();
            assert_eq!(r, size(&s), "{depth}-bit {p}");
        }
        // 64×48 → (none) → 32×24 → 128×96 (aspect kept) → (none) → unchanged.
        assert_eq!(doc(&s).size, Size::new(128, 96), "{depth}-bit");
    }
}

/// #1114: nearest neighbor reduction keeps source values without averaging.
#[test]
fn nearest_neighbor_reduction_keeps_source_values() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            if (x + y) % 2 == 0 {
                s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
                s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
            }
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    let st = s.active().unwrap();
    let before = st.doc.layers[0].surface().unwrap().rgba(0, 0);
    assert!(before[0] < 0.01, "setup: (0,0) should be black, got {before:?}");
    s.execute("image.imageSize", json!({"width": 4, "height": 4, "resample": "nearest"})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    for (x, y) in [(0, 0), (1, 1), (2, 3)] {
        let v = surf.rgba(x, y)[0];
        assert!(v.is_finite() && !(0.01..=0.99).contains(&v), "nearest produced an averaged value {v} at ({x},{y})");
    }
}

#[test]
fn canvas_size_anchors_and_extends_background() {
    let mut s = session();
    s.execute("image.canvasSize", json!({"width": 60, "height": 30, "anchor": "topLeft", "extensionColor": "#00ff00"})).unwrap();
    let d = doc(&s);
    assert_eq!(d.size, Size::new(60, 30));
    assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(10, 5, 20, 15));
    let bg = d.layers[0].surface().unwrap();
    assert_eq!(bg.pixel(50, 25), vec![0.0, 1.0, 0.0, 1.0]);
    assert_eq!(bg.pixel(5, 5), vec![1.0, 1.0, 1.0, 1.0]);
    let mut s = session();
    s.execute("image.canvasSize", json!({"width": 20, "height": 10, "relative": true, "anchor": "center"})).unwrap();
    assert_eq!(doc(&s).layers[1].surface().unwrap().content_bounds(), Rect::new(20, 10, 30, 20));
}

#[test]
fn canvas_size_anchor_offsets_and_rejects_unlisted() {
    // 64x48 -> 100x80: dx spans 36 and dy spans 32, so each anchor factor lands on a known
    // offset. All nine documented values must anchor exactly where they do today (#2946).
    let expected = [
        ("topLeft", [0, 0]),
        ("top", [18, 0]),
        ("topRight", [36, 0]),
        ("left", [0, 16]),
        ("center", [18, 16]),
        ("right", [36, 16]),
        ("bottomLeft", [0, 32]),
        ("bottom", [18, 32]),
        ("bottomRight", [36, 32]),
    ];
    for (anchor, off) in expected {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
        let r = s.execute("image.canvasSize", json!({"width": 100, "height": 80, "anchor": anchor})).unwrap();
        assert_eq!(r["offset"], json!(off), "anchor {anchor}");
    }
    // An omitted anchor still defaults to center.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    assert_eq!(s.execute("image.canvasSize", json!({"width": 100, "height": 80})).unwrap()["offset"], json!([18, 16]));
    // A misspelling or unknown value is a BadParams error naming `anchor`, never an anchor
    // between two of the nine (the old independent-axis matching produced top-centre etc.).
    for anchor in ["topleft", "TopLeft", "TOPLEFT", "bottomleft", "bottomright", "middle", "", "NOPE"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
        let err = s.execute("image.canvasSize", json!({"width": 100, "height": 80, "anchor": anchor})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "anchor {anchor:?} must be rejected, got {err:?}");
        assert!(err.to_string().contains("anchor"), "the error for {anchor:?} must name `anchor`: {err}");
    }
}

#[test]
fn crop_to_selection_and_trim() {
    let mut s = session();
    assert!(s.execute("image.crop", json!({})).is_err(), "no selection, no rect");
    s.execute("select.rect", json!({"x": 5, "y": 2, "width": 20, "height": 10})).unwrap();
    s.execute("image.crop", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!(d.size, Size::new(20, 10));
    assert!(d.selection.is_none());
    assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(5, 3, 15, 10));
    // Trim transparent: hide the background, trim to the red square.
    let mut s = session();
    s.execute("layer.setProps", json!({"layer": doc(&s).layers[0].id.0, "visible": false})).unwrap();
    s.execute("image.trim", json!({"basedOn": "transparent"})).unwrap();
    // The red square (10×10) plus its drop shadow (distance 5, size 5).
    let size = doc(&s).size;
    assert!(size.width > 10 && size.width < 30 && size.height > 10 && size.height < 25, "{size:?}");
}

#[test]
fn crop_to_explicit_rect_can_extend_canvas() {
    let mut s = session();
    s.execute("image.crop", json!({"x": 10, "y": 5, "width": 10, "height": 10})).unwrap();
    assert_eq!(doc(&s).size, Size::new(10, 10));
    assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
    let mut s = session();
    s.execute("image.crop", json!({"x": -10, "y": 0, "width": 60, "height": 20})).unwrap();
    assert_eq!(doc(&s).size, Size::new(60, 20));
    assert_eq!(doc(&s).layers[1].surface().unwrap().pixel(20, 5), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn crop_extension_fills_the_background_with_the_background_colour() {
    let mut shrink = Session::new();
    shrink.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
    shrink.execute("image.crop", json!({"x": 2, "y": 0, "width": 12, "height": 16})).unwrap();
    let layer = &doc(&shrink).layers[0];
    assert_eq!(layer.name, "Background", "a shrink-only crop keeps the Background");
    assert!(layer.locks.transparency && layer.locks.position);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
    s.tools.background = [0.0, 1.0, 0.0, 1.0];
    s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
    let layer = &doc(&s).layers[0];
    assert_eq!(layer.name, "Background", "an extending crop keeps the Background (#3081)");
    assert!(layer.locks.transparency && layer.locks.position);
    assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 1.0, 0.0, 1.0], "the added border takes the background colour");
    assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0], "the original pixels stay");

    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    let layer = &doc(&s).layers[0];
    assert_eq!(layer.surface().unwrap().rgba(1, 8), [0.0, 0.0, 1.0, 1.0], "Fill paints the extended border");
    assert_eq!(layer.surface().unwrap().rgba(8, 8), [0.0, 0.0, 1.0, 1.0]);

    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let layer = &doc(&s).layers[0];
    assert_eq!(doc(&s).size, Size::new(16, 16));
    assert_eq!(layer.surface().unwrap().rgba(8, 8), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(0, 0, 16, 16), "undo removes the filled border");

    // A normal layer gets transparent pixels in the added area.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#ff0000"})).unwrap();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    s.execute("image.crop", json!({"x": -4, "y": 0, "width": 20, "height": 16})).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(1, 8), [0.0; 4]);
}

/// Delete Cropped Pixels deletes pixel-layer data only: type and smart objects keep what lies
/// outside the frame, so moving them later doesn't show them cut at the old edge.
#[test]
fn delete_cropped_pixels_keeps_type_and_smart_objects_whole() {
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("type.create", json!({"x": 10, "y": 15, "text": "Hi", "size": 12})).unwrap();
    let bounds = |s: &Session| -> Vec<Rect> { doc(s).layers[1..].iter().map(|l| l.surface().unwrap().content_bounds()).collect() };
    let before = bounds(&s);
    assert!(before.iter().all(|b| b.x0 < 15 && b.x1 > 15), "both cross the crop edge: {before:?}");
    s.execute("image.crop", json!({"x": 15, "y": 0, "width": 25, "height": 20})).unwrap();
    assert!(matches!(doc(&s).layers[1].content, LayerContent::Smart(_)) && matches!(doc(&s).layers[2].content, LayerContent::Text(_)));
    assert_eq!(bounds(&s), before.iter().map(|b| b.translate(-15, 0)).collect::<Vec<_>>());
}

/// A turned crop deleting pixels keeps type and smart objects whole too, also a smart object
/// whose source is unavailable (the rotation can't re-render it, so it keeps its cache): each
/// ends up exactly as the same turned crop with Delete Cropped Pixels off leaves it.
#[test]
fn turned_crop_deleting_pixels_keeps_type_and_smart_objects_whole() {
    let [deleted, kept] = [true, false].map(|delete| {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.edit("unlink", |doc, active| {
            if let LayerContent::Smart(sm) = &mut doc.layer_mut(active.unwrap()).unwrap().content {
                sm.source = photocraft_doc::SmartSource::Linked { path: "missing-photocraft-test-source.psd".into() };
            }
            Ok(())
        })
        .unwrap();
        s.execute("type.create", json!({"x": 10, "y": 15, "text": "Hi", "size": 12})).unwrap();
        s.execute("image.crop", json!({"x": 20, "y": 0, "width": 20, "height": 20, "angle": 2.0, "deleteCroppedPixels": delete})).unwrap();
        s
    });
    let (a, b) = (doc(&deleted), doc(&kept));
    assert!(matches!(a.layers[1].content, LayerContent::Smart(_)) && matches!(a.layers[2].content, LayerContent::Text(_)));
    for i in [1, 2] {
        let (sa, sb) = (a.layers[i].surface().unwrap(), b.layers[i].surface().unwrap());
        let r = sb.content_bounds();
        assert!(r.x0 < 0, "layer {i} reaches past the crop's left edge: {r:?}");
        assert_eq!(sa.content_bounds(), r, "layer {i}");
        assert_eq!(sa.read_region(r), sb.read_region(r), "layer {i}");
    }
}

/// The Background keeps the pixels a crop with Delete Cropped Pixels off hid: extending the
/// canvas over them again (Reveal All, Canvas Size) shows them rather than painting them over
/// with the background colour (#7086).
#[test]
fn extending_the_canvas_keeps_the_backgrounds_hidden_pixels() {
    for depth in [8, 16, 32] {
        let hidden = |s: &mut Session| {
            s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth, "background": "white"})).unwrap();
            s.edit("paint", |doc, _| {
                let bg = doc.background_mut().unwrap();
                bg.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 50, 100), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
            s.execute("image.crop", json!({"x": 50, "y": 0, "width": 50, "height": 100, "deleteCroppedPixels": false})).unwrap();
            s.tools.background = [0.0, 0.0, 0.0, 1.0];
        };
        let mut s = Session::new();
        hidden(&mut s);
        s.execute("image.revealAll", json!({})).unwrap();
        let surf = doc(&s).background().unwrap().surface().unwrap();
        assert_eq!(doc(&s).size, Size::new(100, 100), "{depth}-bit");
        assert_eq!(surf.rgba(10, 50), [1.0, 0.0, 0.0, 1.0], "{depth}-bit: Reveal All shows the hidden red half");
        assert_eq!(surf.rgba(80, 50), [1.0, 1.0, 1.0, 1.0], "{depth}-bit");

        // Canvas Size wider than the hidden pixels: they show, the rest takes the colour.
        let mut s = Session::new();
        hidden(&mut s);
        s.execute("image.canvasSize", json!({"width": 120, "height": 100, "anchor": "right"})).unwrap();
        let surf = doc(&s).background().unwrap().surface().unwrap();
        assert_eq!(surf.rgba(5, 50), [0.0, 0.0, 0.0, 1.0], "{depth}-bit: past the hidden pixels the background colour");
        assert_eq!(surf.rgba(30, 50), [1.0, 0.0, 0.0, 1.0], "{depth}-bit: Canvas Size shows the hidden red half");
        assert_eq!(surf.rgba(100, 50), [1.0, 1.0, 1.0, 1.0], "{depth}-bit");
    }
}

/// A Background without an alpha channel (a Grayscale PSD import) has no hidden pixels to tell
/// from empty ones: the added area takes the colour, the kept area stays (#7086).
#[test]
fn an_alpha_less_background_extension_takes_the_colour() {
    let mut surf = Surface::new(photocraft_color::PixelFormat::GRAY8);
    surf.fill_rect(Rect::new(-50, 0, 100, 100), &[0.6]);
    let kept = surf.pixel(50, 50);
    fill_background_extension(&mut surf, Rect::new(-60, 0, 100, 100), Rect::new(0, 0, 100, 100), &[0.0]);
    assert_eq!(surf.pixel(-55, 50), vec![0.0], "past the old pixels the colour");
    assert_eq!(surf.pixel(-30, 50), vec![0.0], "no alpha: the added area takes the colour");
    assert_eq!(surf.pixel(50, 50), kept, "the kept area is unchanged");
}

/// A document (`mode`, `depth`) with a white Background and a layer holding a red pixel at
/// (12, 8) and a blue one at (30, 3).
fn marked(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    for (x, y, c) in [(12, 8, "#ff0000"), (30, 3, "#0000ff")] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"color": c})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn layer_rgba(s: &Session, layer: usize, x: i32, y: i32) -> [f32; 4] {
    doc(s).layers[layer].surface().unwrap().rgba(x, y)
}

/// #1792: a frame turned 90° clockwise crops what Image Rotation › Arbitrary 90° CCW and a
/// plain crop give, in one undo step that restores the document, at 8, 16 and 32 bits.
#[test]
fn crop_at_90_degrees_is_rotate_then_crop_in_one_step() {
    for depth in [8, 16, 32] {
        let mut s = marked("rgb", depth);
        let before = doc(&s).clone();
        let past = s.active().unwrap().history.past_len();
        // The 10 × 20 frame at (10, 0), turned 90° about (15, 10), covers x 5..25, y 5..15.
        let r = s.execute("image.crop", json!({"x": 10, "y": 0, "width": 10, "height": 20, "angle": 90})).unwrap();
        assert_eq!(r["angle"], json!(90.0), "{depth}");
        assert_eq!(doc(&s).size, Size::new(10, 20), "{depth}");
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{depth}: one undo step");
        // (12, 8) is 2.5 left of and 1.5 above the centre: in the upright frame, 1.5 left of
        // and 2.5 below its centre (5, 10).
        let red = layer_rgba(&s, 1, 3, 12);
        assert!(red[0] > 0.9 && red[1] < 0.1 && red[3] > 0.9, "{depth}: {red:?}");
        let mut reference = marked("rgb", depth);
        reference.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "ccw"})).unwrap();
        reference.execute("image.crop", json!({"x": 5, "y": 15, "width": 10, "height": 20})).unwrap();
        for l in 0..2 {
            for y in 0..20 {
                for x in 0..10 {
                    let (a, b) = (layer_rgba(&s, l, x, y), layer_rgba(&reference, l, x, y));
                    // A fully transparent pixel's colour is invisible, and the turned crop
                    // doesn't turn the pixels it deletes, which the warp spreads colour from.
                    let n = if b[3] > 1e-3 { 4 } else { 1 };
                    assert!(a.iter().rev().zip(b.iter().rev()).take(n).all(|(p, q)| (p - q).abs() < 1e-3), "{depth} layer {l} ({x}, {y}): {a:?} vs {b:?}");
                }
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
        let d = doc(&s);
        assert_eq!(d.size, before.size, "{depth}");
        assert_eq!(d.depth, before.depth);
        for y in 0..20 {
            for x in 0..40 {
                assert_eq!(layer_rgba(&s, 1, x, y), before.layers[1].surface().unwrap().rgba(x, y), "{depth} undo at ({x}, {y})");
            }
        }
    }
}

/// A slightly turned frame keeps its own size, the pixel under its centre and the Background's
/// colour; the turn's sign matters (a counter-clockwise frame is a negative angle).
#[test]
fn crop_at_a_small_angle_keeps_the_frame_size_in_every_mode() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 16), ("cmyk", 8), ("lab", 16)] {
        for angle in [5.0, -7.5, 365.0] {
            let mut s = marked(mode, depth);
            s.execute("image.crop", json!({"x": 2, "y": 1, "width": 21, "height": 15, "angle": angle})).unwrap();
            let d = doc(&s);
            assert_eq!(d.size, Size::new(21, 15), "{mode} {depth} {angle}");
            assert_eq!(d.depth.bytes() * 8, depth as usize, "{mode} {depth}");
            assert_eq!(format!("{:?}", d.mode).to_lowercase(), mode, "{mode} {depth}");
            assert_eq!(d.layers.len(), 2);
            // The frame's centre (12.5, 8.5) is the red pixel (the layer's only paint nearby);
            // it stays at the centre, its colour kept.
            let want = layer_rgba(&marked(mode, depth), 1, 12, 8);
            let red = layer_rgba(&s, 1, 10, 7);
            assert!(red[3] > 0.5 && (0..3).all(|c| (red[c] - want[c]).abs() < 0.2), "{mode} {depth} {angle}: {red:?} vs {want:?}");
            // The Background stays opaque white inside the canvas it came from.
            let bg = layer_rgba(&s, 0, 10, 7);
            assert!(bg.iter().all(|v| (v - 1.0).abs() < 0.02), "{mode} {depth} {angle}: {bg:?}");
        }
    }
}

/// A turned crop that deletes the cropped pixels turns only what lands in the frame; inside the
/// new canvas every layer, mask and channel is exactly what turning everything gives (the
/// frame spans several tiles, at several depths and angles).
#[test]
fn turned_crop_deleting_pixels_matches_turning_the_whole_canvas() {
    use photocraft_color::PixelFormat;
    let pixels = || (0..500).flat_map(|y| (0..700).map(move |x| (x, y)));
    for depth in [8, 16, 32] {
        for angle in [3.0, -7.5, 33.0, 181.0] {
            let mut sessions = [true, false].map(|delete| {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 700, "height": 500, "depth": depth, "background": "white"})).unwrap();
                s.execute("layer.new.layer", json!({})).unwrap();
                s.edit("paint", |doc, active| {
                    let wave = |x: i32, y: i32, k: i32| ((x * 7 + y * 13 + k * 31) % 97) as f32 / 96.0;
                    let all = Rect::new(0, 0, 700, 500);
                    let rgba: Vec<f32> = pixels().flat_map(|(x, y)| [wave(x, y, 0), wave(x, y, 1), wave(x, y, 2), wave(y, x, 3)]).collect();
                    let gray: Vec<f32> = pixels().map(|(x, y)| wave(y, x, 4)).collect();
                    let l = doc.layer_mut(active.unwrap()).unwrap();
                    l.surface_mut().unwrap().write_region(all, &rgba);
                    let mut mask = photocraft_doc::LayerMask::reveal_all();
                    mask.surface.write_region(all, &gray);
                    l.mask = Some(mask);
                    let bg = doc.background_mut().unwrap().surface_mut().unwrap();
                    bg.write_region(all, &pixels().flat_map(|(x, y)| [wave(x, y, 5), wave(x, y, 6), wave(x, y, 7), 1.0]).collect::<Vec<_>>());
                    let fmt = PixelFormat::new(ColorMode::Grayscale, doc.pixel_format().sample, false);
                    let mut ch = Surface::new(fmt);
                    ch.write_region(all, &gray);
                    doc.channels.push(photocraft_doc::AlphaChannel::new("Alpha 1", ch));
                    Ok(())
                })
                .unwrap();
                let p = json!({"x": 150, "y": 120, "width": 330, "height": 290, "angle": angle, "deleteCroppedPixels": delete});
                s.execute("image.crop", p).unwrap();
                s
            });
            let [deleted, kept] = &mut sessions;
            let (a, b) = (doc(deleted), doc(kept));
            assert_eq!(a.size, b.size, "{depth} {angle}");
            let canvas = a.bounds();
            for (la, lb) in a.layers.iter().zip(&b.layers) {
                let (sa, sb) = (la.surface().unwrap(), lb.surface().unwrap());
                assert!(sa.to_interleaved(canvas) == sb.to_interleaved(canvas), "{depth} {angle}: layer {}", la.name);
                if let (Some(ma), Some(mb)) = (&la.mask, &lb.mask) {
                    assert!(ma.surface.to_interleaved(canvas) == mb.surface.to_interleaved(canvas), "{depth} {angle}: mask of {}", la.name);
                }
            }
            assert!(a.channels[0].surface.to_interleaved(canvas) == b.channels[0].surface.to_interleaved(canvas), "{depth} {angle}: alpha channel");
            // Deleting keeps nothing past the canvas.
            let layer = a.layers[1].surface().unwrap();
            assert!(canvas.contains_rect(&layer.content_bounds()), "{depth} {angle}: {:?}", layer.content_bounds());
        }
    }
}

/// Angle 0 (or a whole turn) is the plain crop: no resampling, the same document.
#[test]
fn crop_at_angle_zero_is_the_plain_crop() {
    let mut plain = marked("rgb", 16);
    plain.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10})).unwrap();
    for angle in [json!(0), json!(0.0), json!(360), json!(-720), json!(null)] {
        let mut s = marked("rgb", 16);
        let r = s.execute("image.crop", json!({"x": 3, "y": 2, "width": 30, "height": 10, "angle": angle})).unwrap();
        assert!(r.get("angle").is_none(), "{angle}: {r}");
        assert_eq!(doc(&s).size, Size::new(30, 10));
        for l in 0..2 {
            for y in 0..10 {
                for x in 0..30 {
                    assert_eq!(layer_rgba(&s, l, x, y), layer_rgba(&plain, l, x, y), "{angle} layer {l} ({x}, {y})");
                }
            }
        }
    }
}

/// Bad angles fail cleanly, before anything changes.
#[test]
fn crop_angle_rejects_bad_values_without_changing_the_document() {
    let bad = [
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "NaN"}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": "12"}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": [1]}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": true}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": 1e308}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": -3601}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "angle": f64::MAX}),
        // An angle needs the frame: a selection crop has none.
        json!({"angle": 10}),
        json!({"x": 0, "y": 0, "width": 0, "height": 10, "angle": 10}),
        json!({"x": 0, "y": 0, "width": 300_001, "height": 10, "angle": 10}),
        json!({"x": i32::MAX - 5, "y": 0, "width": 10, "height": 10, "angle": 10}),
    ];
    for p in bad {
        let mut s = marked("rgb", 8);
        s.execute("select.all", json!({})).unwrap();
        let before = doc(&s).clone();
        let past = s.active().unwrap().history.past_len();
        assert!(s.execute("image.crop", p.clone()).is_err(), "{p}");
        assert_eq!(doc(&s).size, before.size, "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "{p}");
    }
    // Far-off but representable frames turn and crop without panicking.
    for (x, y) in [(-1_000_000, 0), (0, 1_000_000), (i32::MIN / 4, i32::MAX / 4)] {
        let mut s = marked("rgb", 8);
        s.execute("image.crop", json!({"x": x, "y": y, "width": 4, "height": 3, "angle": 33.3})).unwrap();
        assert_eq!(doc(&s).size, Size::new(4, 3));
    }
}

#[test]
fn crop_refuses_a_canvas_side_past_the_new_document_limit() {
    // #960: an unbounded explicit crop left a canvas that a later Trim or Duplicate Merged
    // could only abort on allocating.
    for (w, h) in [(100_000_000, 100_000_000), (300_001, 10), (10, 300_001)] {
        let mut s = session();
        let (size, past) = (doc(&s).size, s.active().unwrap().history.past_len());
        let err = s.execute("image.crop", json!({"x": 0, "y": 0, "width": w, "height": h})).unwrap_err();
        assert!(err.to_string().contains("300000 px limit"), "{w} x {h}: {err}");
        assert_eq!((doc(&s).size, s.active().unwrap().history.past_len()), (size, past), "{w} x {h}: nothing changed");
    }
    // At the limit it still extends the canvas (the tiles stay lazy).
    let mut s = session();
    s.execute("image.crop", json!({"x": 0, "y": 0, "width": 300_000, "height": 2})).unwrap();
    assert_eq!(doc(&s).size, Size::new(300_000, 2));
}

/// #2443: the Crop tool's W x H x Resolution crops, resamples the cropped area to exactly the
/// target size and sets the resolution, in one undo step, at 8 and 16 bits.
#[test]
fn crop_to_a_target_size_and_resolution_is_one_step() {
    for depth in [8, 16] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 100, 200, 225), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let past = s.active().unwrap().history.past_len();
        // 4 x 5 in at 300 ppi from a 100 x 125 px frame.
        let r = s
            .execute("image.crop", json!({"x": 100, "y": 100, "width": 100, "height": 125, "targetWidth": 1200, "targetHeight": 1500, "resolution": 300}))
            .unwrap();
        assert_eq!((r["documentWidth"].as_u64(), r["documentHeight"].as_u64()), (Some(1200), Some(1500)), "{depth}: {r}");
        assert_eq!(r["resolution"].as_f64(), Some(300.0), "{depth}");
        let d = doc(&s);
        assert_eq!(d.size, Size::new(1200, 1500), "{depth}");
        assert_eq!(d.resolution_dpi, 300.0, "{depth}");
        assert_eq!(d.layers[0].surface().unwrap().format().sample, if depth == 8 { SampleType::U8 } else { SampleType::U16 });
        // The red frame filled the crop; it fills the result.
        let red = d.layers[1].surface().unwrap();
        for (x, y) in [(0, 0), (600, 750), (1199, 1499)] {
            assert_eq!(red.pixel(x, y), vec![1.0, 0.0, 0.0, 1.0], "{depth}: ({x}, {y})");
        }
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{depth}: one history step");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(400, 300), 72.0), "{depth}: undo restores the original");
    }
}

#[test]
fn crop_target_resamples_after_a_turned_crop_and_resolution_alone_keeps_the_pixels() {
    let mut s = session();
    let past = s.active().unwrap().history.past_len();
    s.execute("image.crop", json!({"x": 10, "y": 0, "width": 10, "height": 20, "angle": 90, "targetWidth": 30, "targetHeight": 60, "resample": "nearest"}))
        .unwrap();
    assert_eq!(doc(&s).size, Size::new(30, 60));
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "rotate, crop and resample: one step");
    let mut s = session();
    s.execute("image.crop", json!({"x": 2, "y": 3, "width": 30, "height": 10, "resolution": 150})).unwrap();
    assert_eq!((doc(&s).size, doc(&s).resolution_dpi), (Size::new(30, 10), 150.0));
}

/// #7722: a W x H crop of an Indexed Color document resamples like Image Size (#6019), with
/// Nearest Neighbor, so no pixel gets a blend that isn't in the colour table.
#[test]
fn crop_target_resamples_indexed_color_with_nearest_neighbor() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 1, "y": 0, "width": 1, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    for resample in [None, Some("bicubic"), Some("automatic"), Some("bilinear"), Some("lanczos")] {
        let mut p = json!({"x": 0, "y": 0, "width": 2, "height": 1, "targetWidth": 8, "targetHeight": 1});
        if let Some(r) = resample {
            p["resample"] = json!(r);
        }
        s.execute("image.crop", p).unwrap();
        let surf = doc(&s).layers[0].surface().unwrap();
        for x in 0..8 {
            let px = surf.pixel(x, 0);
            assert!(px[..3] == [1.0, 0.0, 0.0] || px[..3] == [0.0, 0.0, 1.0], "{resample:?}: no blended colour at x {x}: {px:?}");
        }
        s.execute("history.undo", json!({})).unwrap();
    }
}

#[test]
fn crop_target_accepts_whole_number_floats() {
    // `json.dumps(100.0)` and many other JSON writers send whole numbers as floats.
    let mut s = session();
    s.execute("image.crop", json!({"x": 0, "y": 0, "width": 20, "height": 10, "targetWidth": 40.0, "targetHeight": 20.0})).unwrap();
    assert_eq!(doc(&s).size, Size::new(40, 20));
}

#[test]
fn crop_target_rejects_bad_params_without_changing_the_document() {
    let rect = json!({"x": 0, "y": 0, "width": 10, "height": 10});
    let bad_params = [
        json!({"targetWidth": 100}),
        json!({"targetHeight": 100}),
        json!({"targetWidth": 0, "targetHeight": 10}),
        json!({"targetWidth": -5, "targetHeight": 10}),
        json!({"targetWidth": 10.5, "targetHeight": 10}),
        json!({"targetWidth": -1.0, "targetHeight": 10}),
        json!({"targetWidth": 0.0, "targetHeight": 10}),
        json!({"targetWidth": 1e39, "targetHeight": 10}),
        json!({"targetWidth": 300_001.0, "targetHeight": 10}),
        json!({"targetWidth": "10", "targetHeight": 10}),
        json!({"targetWidth": 300_001, "targetHeight": 10}),
        json!({"targetWidth": 4_294_967_306_u64, "targetHeight": 10}),
        json!({"targetWidth": 300_000, "targetHeight": 300_000}),
        json!({"targetWidth": 10, "targetHeight": 10, "resample": "none"}),
        json!({"targetWidth": 10, "targetHeight": 10, "resample": 3}),
        json!({"resolution": 0}),
        json!({"resolution": -72}),
        json!({"resolution": 1e9}),
        json!({"resolution": "300"}),
    ];
    for extra in bad_params {
        let mut s = session();
        let (size, dpi, past) = (doc(&s).size, doc(&s).resolution_dpi, s.active().unwrap().history.past_len());
        let mut p = rect.clone();
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        assert!(s.execute("image.crop", p.clone()).is_err(), "{p}");
        assert_eq!((doc(&s).size, doc(&s).resolution_dpi, s.active().unwrap().history.past_len()), (size, dpi, past), "{p}: nothing changed");
    }
}

#[test]
fn trim_based_on_top_left_colour() {
    let mut s = session();
    s.edit("no fx", |doc, _| {
        doc.layers[1].effects.items.clear();
        Ok(())
    })
    .unwrap();
    s.execute("image.trim", json!({"basedOn": "topLeft"})).unwrap();
    assert_eq!(doc(&s).size, Size::new(10, 10));
}

#[test]
fn thirty_two_bit_holds_linear_light() {
    // Photoshop 27.11: an 8-bit sRGB grey 128 becomes 0.2159 in 32 bits (and 32 → 0.0144),
    // and comes back unchanged; alpha keeps its value.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 1, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 2, 1), &[128.0 / 255.0, 32.0 / 255.0, 1.0, 0.5]);
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.bits32", json!({})).unwrap();
    let px = doc(&s).layers[1].surface().unwrap().pixel(0, 0);
    for (got, want) in px.iter().zip([0.2159, 0.0144, 1.0, 128.0 / 255.0]) {
        assert!((got - want).abs() < 2e-4, "{px:?}");
    }
    s.execute("image.mode.bits8", json!({})).unwrap();
    let px = doc(&s).layers[1].surface().unwrap().pixel(0, 0);
    for (got, want) in px.iter().zip([128.0 / 255.0, 32.0 / 255.0, 1.0, 128.0 / 255.0]) {
        assert!((got - want).abs() < 0.6 / 255.0, "{px:?}");
    }
}

#[test]
fn grayscale_conversion_matches_photoshop() {
    // Photoshop 27.11, default Color Settings (working gray Dot Gain 20 %): Image > Mode >
    // Grayscale turns sRGB 64 / 128 / 192 grey and pure red into 46 / 105 / 177 / 107.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 1, "background": "white"})).unwrap();
    s.edit("paint", |doc, _| {
        let l = doc.layers.first_mut().unwrap();
        let surf = l.surface_mut().unwrap();
        for (x, c) in [[64.0, 64.0, 64.0], [128.0, 128.0, 128.0], [192.0, 192.0, 192.0], [255.0, 0.0, 0.0]].iter().enumerate() {
            surf.fill_rect(Rect::new(x as i32, 0, x as i32 + 1, 1), &[c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let surf = doc(&s).layers[0].surface().unwrap();
    for (x, want) in [46.0, 105.0, 177.0, 107.0].iter().enumerate() {
        let got = surf.pixel(x as i32, 0)[0] * 255.0;
        assert!((got - want).abs() <= 1.5, "pixel {x}: {got} vs {want}");
    }
}

#[test]
fn mode_and_depth_conversions() {
    let mut s = session();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(doc(&s).mode, ColorMode::Grayscale);
    assert_eq!(doc(&s).layers[1].surface().unwrap().format().mode, ColorMode::Grayscale);
    s.execute("image.mode.bits16", json!({})).unwrap();
    assert_eq!(doc(&s).depth, SampleType::U16);
    assert_eq!(doc(&s).layers[1].surface().unwrap().format().sample, SampleType::U16);
    assert_eq!(doc(&s).layers[1].mask.as_ref().unwrap().surface.format().sample, SampleType::U16);
    s.execute("image.mode.cmyk", json!({})).unwrap();
    s.execute("image.mode.bits32", json!({})).unwrap();
    s.execute("image.mode.lab", json!({})).unwrap();
    s.execute("image.mode.rgb", json!({})).unwrap();
    s.execute("image.mode.bits8", json!({})).unwrap();
    let d = doc(&s);
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).depth, SampleType::F32);
    let revision = s.active().unwrap().revision;
    s.execute("image.mode.bits32", json!({})).unwrap();
    assert_eq!(s.active().unwrap().revision, revision, "an unchanged bit depth must preserve the history branch");
    assert!(s.redo(), "selecting the current depth must not discard the undone conversion");
    assert_eq!(doc(&s).depth, SampleType::U8);
}

/// Duplicate › Merged Layers Only gives one locked Background when the source has one and a
/// plain "Layer 1" (keeping transparency) when it doesn't, as Photoshop does.
#[test]
fn merged_duplicate_is_a_background_only_when_the_source_has_one() {
    let mut s = session();
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let d = doc(&s);
    assert!(d.is_background(d.layers[0].id), "source has a Background: the merged copy is one");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    let d = doc(&s);
    assert!(d.background().is_none(), "transparent source: no Background in the copy");
    assert_eq!(d.layers[0].name, "Layer 1");
}

#[test]
fn duplicate_document() {
    let mut s = session();
    let r = s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap();
    assert_eq!(r["document"], json!(1));
    assert_eq!(s.documents().len(), 2);
    assert_eq!(doc(&s).layers.len(), 1);
    assert!(doc(&s).name.ends_with("copy"));
}
