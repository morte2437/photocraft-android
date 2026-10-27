use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, a| {
        doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn new_clipboard_document_rejects_an_unrepresentable_origin() {
    let mut s = Session::new();
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_pixel(i32::MIN, 0, &[1.0, 0.0, 0.0, 1.0]);
    s.clipboard = Some(Clip { surface, bounds: Rect::from_xywh(i32::MIN, 0, 1, 1), layers: None, resolution: None, source: None });
    let result = new_from_clipboard(&mut s);
    assert!(result.unwrap_err().to_string().contains("clipboard origin"));
    assert!(s.execute("file.newFromClipboard", json!({})).is_err());
    assert!(s.documents().is_empty());
    assert_eq!(s.clipboard.as_ref().unwrap().surface.rgba(i32::MIN, 0), [1.0, 0.0, 0.0, 1.0]);
}

/// #8945: pixels copied in one document land in place when pasted back into it, and centred
/// in another, even where their old position would fit (Photoshop's Paste).
#[test]
fn paste_centres_pixels_from_another_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let r = s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(r["offset"], json!([0, 0]), "back into its own document: in place");
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "white"})).unwrap();
    let r = s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(r["offset"], json!([30, 25]), "into another document: centred");
    // An image from another app has no source document: centred too.
    s.clipboard = s.clipboard.take().map(|c| Clip { source: None, ..c });
    let r = s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(r["offset"], json!([30, 25]));
    // Paste in Place still keeps the position.
    let r = s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    assert_eq!(r["offset"], json!([0, 0]));
}

#[test]
fn paste_centers_clipboard_bounds_near_coordinate_limits() {
    for origin in [i32::MAX - 512, i32::MIN + 512] {
        let clip =
            Clip { surface: Surface::new(PixelFormat::RGBA8), bounds: Rect::from_xywh(origin, origin, 2, 2), layers: None, resolution: None, source: None };
        let (dx, dy) = paste_offset(&clip, DocId(1), Rect::new(0, 0, 20, 20), &json!({}), false);
        assert_eq!(i64::from(origin) + i64::from(dx), 9);
        assert_eq!(i64::from(origin) + i64::from(dy), 9);
        let clip = Clip { bounds: Rect::new(0, 0, 2, 2), ..clip };
        let (dx, dy) = paste_offset(&clip, DocId(1), Rect::from_xywh(origin, origin, 20, 20), &json!({}), false);
        assert_eq!(dx, origin + 9);
        assert_eq!(dy, origin + 9);
    }
}

#[test]
fn injected_clear_buffer_failure_leaves_the_document_unchanged() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
    let revision = st.revision;

    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.clear", json!({}));

    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for erasing pixels")));
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
    assert_eq!(st.revision, revision);
}

#[test]
fn injected_copy_buffer_failure_does_not_change_document_or_clipboard() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
    let revision = st.revision;

    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.copy", json!({}));

    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for copying pixels")));
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
    assert_eq!(st.revision, revision);
    assert!(s.clipboard.is_none());
}

#[test]
fn injected_fill_result_reservation_failure_keeps_pixels_and_history() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
    let revision = st.revision;
    let history_len = st.history.past_len();

    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.fill", json!({"color": "#00ff00"}));

    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for blending pixels")));
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), history_len);
}

#[test]
fn injected_paste_failure_does_not_change_document_or_clipboard() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 8, "height": 8})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let before_clip = s.clipboard.clone();
    let st = s.active().unwrap();
    let before_doc = (*st.doc).clone();
    let revision = st.revision;
    let history_len = st.history.past_len();

    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.paste", json!({}));

    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for pasting pixels")));
    let st = s.active().unwrap();
    assert_eq!(*st.doc, before_doc);
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), history_len);
    let after_clip = s.clipboard.as_ref().expect("clipboard remains available");
    let before_clip = before_clip.as_ref().expect("clipboard was populated");
    assert_eq!(after_clip.bounds, before_clip.bounds);
    assert_eq!(after_clip.surface, before_clip.surface);
}

#[test]
fn injected_transform_failure_keeps_pixels_and_history() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
    let revision = st.revision;
    let history_len = st.history.past_len();

    crate::allocation::fail_next_for_test();
    let result = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 20, 30]}));

    assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for transforming pixels")));
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), history_len);
}

fn active_bounds(s: &Session) -> Rect {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

/// New from Clipboard takes the copied document's resolution (Photoshop's Clipboard preset),
/// so a later Image Size by resolution scales from the right size.
#[test]
fn new_from_clipboard_keeps_the_source_resolution() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 60, "height": 40, "resolution": 300})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.newFromClipboard", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.resolution_dpi, 300.0);
}

/// #368: the clipboard as a document of its own, from New from Clipboard or a Paste with
/// nothing open, at every depth.
#[test]
fn new_from_clipboard_and_paste_with_no_document() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        assert!(!s.is_enabled("file.newFromClipboard") && !s.is_enabled("edit.paste"), "nothing copied yet");
        assert!(s.execute("file.newFromClipboard", json!({})).is_err());
        s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
        assert!(!s.active().unwrap().is_dirty(), "an untouched blank document is clean");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 15})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // With a document open: a second document, sized to the copy, its pixels at the origin.
        let r = s.execute("file.newFromClipboard", json!({})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(30), Some(15)), "{r}");
        assert_eq!(s.documents().len(), 2);
        assert!(s.active().unwrap().is_dirty(), "a document made from clipboard pixels needs saving (#4665)");
        let d = &s.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.layers.len()), (30, 15, 1));
        assert_eq!(d.pixel_format().sample, s.documents()[0].doc.pixel_format().sample, "{depth}-bit kept");
        assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
        let px = d.layers[0].surface().unwrap().rgba(0, 0);
        assert!(px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99, "{px:?}");
        // With nothing open, Paste makes the document too.
        while s.active().is_some() {
            s.execute("file.close", json!({})).unwrap();
        }
        assert!(s.is_enabled("edit.paste") && !s.is_enabled("edit.pasteSpecial.pasteInPlace"));
        s.execute("edit.paste", json!({})).unwrap();
        assert_eq!(s.documents().len(), 1);
        assert!(s.active().unwrap().is_dirty(), "Paste with no tab also needs a save prompt");
        assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
        assert!(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).is_ok(), "into the new document");
    }
}

#[test]
fn copy_paste_in_place_and_centred() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 30));
    assert!(s.active().unwrap().doc.selection.is_none());
    s.execute("edit.paste", json!({"center": [70, 70]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(60, 60, 80, 80));
    assert_eq!(s.active().unwrap().doc.layers.len(), 4);
}

/// A `w`×`h` document with an opaque `cw`×`ch` image from another app on the clipboard.
fn external_clip_session(w: i32, h: i32, cw: i32, ch: i32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.fill_rect(Rect::new(0, 0, cw, ch), &[1.0, 0.0, 0.0, 1.0]);
    s.clipboard = Some(Clip { surface, bounds: Rect::new(0, 0, cw, ch), layers: None, resolution: None, source: None });
    s
}

/// #6759: wherever the view is centred, a paste that fits the canvas stays wholly on it.
#[test]
fn paste_that_fits_stays_on_the_canvas() {
    for (w, h, center, want) in [
        (60, 100, [40.0, 45.0], Rect::new(0, 0, 60, 100)),
        (60, 100, [-500.0, 900.0], Rect::new(0, 0, 60, 100)),
        (100, 100, [10.0, 80.0], Rect::new(0, 0, 60, 100)),
        (100, 100, [95.0, 50.0], Rect::new(40, 0, 100, 100)),
        (100, 120, [50.0, 60.0], Rect::new(20, 10, 80, 110)),
    ] {
        let mut s = external_clip_session(w, h, 60, 100);
        s.execute("edit.paste", json!({ "center": center })).unwrap();
        assert_eq!(active_bounds(&s), want, "{w}x{h} canvas, view centre {center:?}");
    }
}

/// #6759: a paste larger than the canvas starts at its top-left corner, wherever the view is.
#[test]
fn paste_larger_than_the_canvas_starts_at_its_top_left() {
    for center in [[0.0, 0.0], [25.0, 20.0], [1000.0, 1000.0], [-1000.0, 30.0]] {
        let mut s = external_clip_session(50, 40, 60, 100);
        s.execute("edit.paste", json!({ "center": center })).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(0, 0, 60, 100), "view centre {center:?}");
    }
    // Wider but not taller: at the left edge, and wholly inside vertically.
    for (center, want) in
        [([500.0, 500.0], Rect::new(0, 100, 60, 200)), ([-500.0, -500.0], Rect::new(0, 0, 60, 100)), ([25.0, 120.0], Rect::new(0, 70, 60, 170))]
    {
        let mut s = external_clip_session(50, 200, 60, 100);
        s.execute("edit.paste", json!({ "center": center })).unwrap();
        assert_eq!(active_bounds(&s), want, "view centre {center:?}");
    }
}

/// Paste Into centres on the selection even where that runs off the canvas.
#[test]
fn paste_into_is_not_clamped_to_the_canvas() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 90, "y": 90, "width": 10, "height": 10})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(75, 85, 115, 105));
}

/// The clamp never overflows or panics: NaN, infinite and `i32`-limit shifts saturate (Rule 9).
#[test]
fn paste_offset_clamp_survives_extreme_coordinates() {
    assert_eq!(clamp_shift(f64::NAN, 0, 10, 0, 100), 0);
    assert_eq!(clamp_shift(f64::INFINITY, 0, 10, 0, 100), 90);
    assert_eq!(clamp_shift(f64::NEG_INFINITY, i32::MIN, i32::MAX, 0, 1), i32::MAX);
    assert_eq!(clamp_shift(1e30, i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX), i32::MAX);
    let clip = Clip {
        surface: Surface::new(PixelFormat::RGBA8),
        bounds: Rect::new(i32::MAX - 2, i32::MAX - 2, i32::MAX, i32::MAX),
        layers: None,
        resolution: None,
        source: None,
    };
    let (dx, dy) = paste_offset(&clip, DocId(1), Rect::new(i32::MIN, i32::MIN, i32::MIN + 4, i32::MIN + 4), &json!({}), false);
    assert_eq!((dx, dy), (i32::MIN, i32::MIN), "an unrepresentable shift saturates instead of wrapping");
}

#[test]
fn paste_with_malformed_center_does_not_panic() {
    // A short/empty `center` array must not panic (Rule 9) — it falls back to the canvas centre.
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    for center in [json!([5]), json!([]), json!("nope"), json!([1, 2, 3])] {
        // Fresh clipboard each time is unnecessary; copy persists. Must return Ok, never panic.
        assert!(s.execute("edit.paste", json!({ "center": center })).is_ok(), "center={center}");
    }
}

#[test]
fn copy_paste_between_documents() {
    // Copy in one tab, paste into another: the clipboard is shared across documents.
    let mut s = session(); // doc A (100x100): red rect at (10,10)-(50,30)
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    // Open a second document (a new tab), which becomes active.
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(s.active_index().unwrap(), 1, "the new document is the active tab");
    let before = s.active().unwrap().doc.layers.len();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(s.active_index().unwrap(), 1, "paste stays in the second tab");
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), before + 1, "pasted layer added to the second document");
    let pasted = d.layers.last().unwrap().surface().unwrap();
    let cb = pasted.content_bounds();
    let (cx, cy) = (cb.x0 + cb.width() as i32 / 2, cb.y0 + cb.height() as i32 / 2);
    let mut px = [[0.0f32; 4]; 1];
    pasted.read_rgba_into(Rect::new(cx, cy, cx + 1, cy + 1), &mut px);
    assert!(px[0][0] > 0.9 && px[0][1] < 0.1 && px[0][2] < 0.1, "pasted content is red: {:?}", px[0]);
}

/// #2901 (P10/P11): Paste, Paste in Place and Paste Into recomposite only where the pasted
/// layer draws, not the whole document; nothing outside that changes, and undo brings back
/// exactly the old pixels.
#[test]
fn paste_damages_only_the_pasted_layer() {
    let render = |s: &Session| photocraft_compose::render(&s.active().unwrap().doc, s.active().unwrap().doc.bounds());
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 50, "depth": depth, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // Half transparent, so an opaque copy pasted in place over it shows.
        s.execute("layer.setProps", json!({"opacity": 0.5})).unwrap();
        for (id, p) in [
            ("edit.paste", json!({"center": [40, 30]})),
            ("edit.pasteSpecial.pasteInPlace", json!({})),
            ("edit.pasteSpecial.pasteInto", json!({"center": [40, 30]})),
        ] {
            s.execute("select.rect", json!({"x": 36, "y": 27, "width": 6, "height": 4})).unwrap();
            let shown = render(&s);
            s.execute(id, p).unwrap();
            let st = s.active().unwrap();
            let area = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
            let damage = st.last_damage.unwrap_or_else(|| panic!("{depth}-bit {id}: bounded damage"));
            assert!(!damage.is_empty() && damage.intersect(&area) == damage, "{depth}-bit {id}: {damage:?} within {area:?}");
            let now = render(&s);
            assert_ne!(now, shown, "{depth}-bit {id}: the paste shows");
            let w = shown.rect.width() as usize;
            for (i, (a, b)) in shown.px.iter().zip(&now.px).enumerate() {
                if !damage.contains((i % w) as i32, (i / w) as i32) {
                    assert_eq!(a, b, "{depth}-bit {id}: pixel {i} outside the damage changed");
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(render(&s), shown, "{depth}-bit {id}: undo");
        }
    }
}

/// #1035: a `depth`-bit document with a blue-grey square copied from layer `src` (at
/// (5,5)-(15,15)) and an active layer `m` above it with a hide-all mask.
fn mask_target_session(depth: u32) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
    let src = LayerId(s.execute("layer.new.layer", json!({"name": "src"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let m = LayerId(s.execute("layer.new.layer", json!({"name": "m"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    (s, src, m)
}

/// The luminosity of `src`'s pixel at (`x`, `y`): what a paste into a mask writes there.
fn luminosity(s: &Session, src: LayerId, x: i32, y: i32) -> f32 {
    let p = s.active().unwrap().doc.layer(src).unwrap().surface().unwrap().rgba(x, y);
    photocraft_color::convert::rgb_to_gray([p[0], p[1], p[2]])
}

fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().value(x, y)
}

#[test]
fn paste_into_a_targeted_mask_writes_the_luminosity_in_one_step() {
    for depth in [8, 16, 32] {
        let (mut s, src, m) = mask_target_session(depth);
        let lum = luminosity(&s, src, 10, 10);
        assert!(lum > 0.05 && lum < 0.95, "{depth}-bit: a mid grey ({lum})");
        let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer_count(), layers, "{depth}-bit: no new layer");
        assert_eq!(st.history.past_len(), steps + 1, "{depth}-bit: one history step");
        assert_eq!(st.active_layer, Some(m));
        assert!((mask_at(&s, m, 10, 10) - lum).abs() < 2.0 / 255.0, "{depth}-bit: {} vs {lum}", mask_at(&s, m, 10, 10));
        assert_eq!(mask_at(&s, m, 20, 20), 0.0, "{depth}-bit: the mask outside the paste is unchanged");
        assert!(st.doc.layer(m).unwrap().surface().unwrap().content_bounds().is_empty(), "{depth}-bit: the pixels are untouched");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(mask_at(&s, m, 10, 10), 0.0, "{depth}-bit: undo restores the mask");
    }
}

#[test]
fn copy_with_the_mask_targeted_takes_the_mask_as_grayscale() {
    for depth in [8, 16, 32] {
        let (mut s, _, m) = mask_target_session(depth);
        s.edit("paint mask", |doc, _| {
            doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
            Ok(())
        })
        .unwrap();
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
        assert_eq!(r["bounds"], json!([0, 0, 40, 40]), "{depth}-bit: a mask copies whole, as in Photoshop");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{depth}-bit: copy adds no history step");
        let clip = s.clipboard.clone().unwrap();
        let px = clip.surface.rgba(10, 10);
        assert!((px[0] - 0.75).abs() < 2.0 / 255.0, "{depth}-bit: {} vs 0.75", px[0]);
        assert!(clip.surface.rgba(30, 30)[0] < 0.01, "{depth}-bit: the hidden area copies as black");
        assert!(px[3] > 0.99, "{depth}-bit: the copy is opaque");
        // Paste it into another layer's reveal-all mask: it replaces that mask (the request
        // in #2928): the square, and the black around it, land.
        let m2 = LayerId(s.execute("layer.new.layer", json!({"name": "m2"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
        assert!((mask_at(&s, m2, 10, 10) - 0.75).abs() < 2.0 / 255.0, "{depth}-bit: the copied mask lands");
        assert!(mask_at(&s, m2, 30, 30) < 0.01, "{depth}-bit: the copied black replaces the reveal-all");
    }
}

#[test]
fn an_untouched_mask_copies_the_whole_canvas_at_its_default() {
    let (mut s, _, _) = mask_target_session(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    assert_eq!(r["bounds"], json!([0, 0, 40, 40]), "a plain Reveal All mask still transfers");
    assert!(s.clipboard.clone().unwrap().surface.rgba(20, 20)[0] > 0.99);
}

#[test]
fn mask_copy_takes_only_the_selection() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("paint mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    let r = s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    assert_eq!(r["bounds"], json!([8, 8, 4, 4]));
    let clip = s.clipboard.clone().unwrap();
    assert!((clip.surface.rgba(9, 9)[0] - 0.75).abs() < 2.0 / 255.0);
    assert!(clip.surface.rgba(5, 5)[3] < 0.01, "nothing outside the selection");
}

#[test]
fn mask_copy_needs_a_mask_and_fails_gracefully() {
    let mut s = Session::new();
    assert!(s.execute("edit.copy", json!({"target": "mask"})).is_err(), "no document");
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    assert!(s.execute("edit.copy", json!({"target": "mask"})).is_err(), "the layer has no mask");
    assert!(s.clipboard.is_none(), "a refused copy leaves the clipboard alone");
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    assert!(s.execute("edit.copy", json!({"target": "mask"})).is_ok());
    // An adjustment layer has no pixels of its own: Copy takes the layer whole (#2344), and
    // its mask while that is targeted (#2928).
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    let a = s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    assert!(s.is_enabled("edit.copy"), "the layer itself can be copied");
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    assert!(s.is_enabled_with("edit.copy", &json!({"target": "mask"})), "the mask can be copied");
}

#[test]
fn a_copied_mask_travels_to_another_document() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("paint mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(6, 6, 16, 12), &[0.75]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.copy", json!({"target": "mask"})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    let n = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
    assert!((mask_at(&s, n, 10, 10) - 0.75).abs() < 2.0 / 255.0, "the mask arrived in the second document");
    assert!(mask_at(&s, n, 30, 30) < 0.01, "it replaced the reveal-all mask there");
}

#[test]
fn paste_follows_the_mask_view_and_is_placed_like_a_paste() {
    let (mut s, src, m) = mask_target_session(8);
    let lum = luminosity(&s, src, 10, 10);
    // ⌥-click the mask thumbnail: the canvas shows the mask, and pasting targets it.
    s.execute(crate::mask_view_cmds::ID, json!({"layer": m.0, "mode": "gray"})).unwrap();
    let r = s.execute("edit.paste", json!({"center": [30, 30]})).unwrap();
    assert_eq!(r["offset"], json!([20, 20]));
    assert!((mask_at(&s, m, 30, 30) - lum).abs() < 2.0 / 255.0);
    assert_eq!(mask_at(&s, m, 10, 10), 0.0);
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "Background, src, m");
}

#[test]
fn paste_into_and_outside_a_targeted_mask_keep_to_the_selection() {
    let (mut s, src, m) = mask_target_session(16);
    let lum = luminosity(&s, src, 10, 10);
    // The paste is centred on the selection, so lands in place: only the selection takes it.
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({"target": "mask"})).unwrap();
    assert!((mask_at(&s, m, 9, 9) - lum).abs() < 2.0 / 255.0);
    assert_eq!(mask_at(&s, m, 7, 7), 0.0, "pasted, but outside the selection");
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
    s.execute("edit.pasteSpecial.pasteOutside", json!({"target": "mask"})).unwrap();
    assert_eq!(mask_at(&s, m, 9, 9), 0.0, "Paste Outside leaves the selection alone");
    assert!((mask_at(&s, m, 7, 7) - lum).abs() < 2.0 / 255.0);
}

#[test]
fn paste_follows_a_targeted_alpha_channel() {
    let (mut s, src, _) = mask_target_session(8);
    let lum = luminosity(&s, src, 10, 10);
    s.execute("channel.new", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": {"channel": 0}})).unwrap();
    let c = &s.active().unwrap().doc.channels[0].surface;
    assert!((c.sample_channel(10, 10, 0) - lum).abs() < 2.0 / 255.0);
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
}

#[test]
fn paste_into_a_missing_mask_or_channel_is_an_error() {
    let (mut s, _, m) = mask_target_session(8);
    s.edit("drop mask", |doc, _| {
        doc.layer_mut(m).unwrap().mask = None;
        Ok(())
    })
    .unwrap();
    let layers = s.active().unwrap().doc.layer_count();
    for target in [json!("mask"), json!({"channel": 7}), json!("quickMask")] {
        for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
            assert!(s.execute(id, json!({ "target": target })).is_err(), "{id} {target}");
        }
    }
    assert_eq!(s.active().unwrap().doc.layer_count(), layers, "nothing pasted as a layer");
}

#[test]
fn paste_between_different_color_modes() {
    // Copy from an RGB document, paste into a Grayscale one: the clipboard converts to the
    // target document's format (a realistic between-tabs case that must not panic).
    let mut s = session(); // RGB, red rect
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 120, "height": 120, "mode": "gray"})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.mode, photocraft_doc::ColorMode::Grayscale);
    let pasted = d.layers.last().unwrap().surface().unwrap();
    assert_eq!(pasted.format().mode, photocraft_color::ColorMode::Grayscale, "pasted layer is grayscale");
    assert!(!pasted.content_bounds().is_empty(), "pasted content exists");
}

#[test]
fn transform_scale_grows_content() {
    let mut s = session(); // red rect (10,10)-(50,30): 40x20
    s.execute("edit.transform", json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
    let d = &s.active().unwrap().doc;
    let cb = d.layers.last().unwrap().surface().unwrap().content_bounds();
    // Scaled ~2x about the origin (≈ 80x40 at ≈ (20,20)); allow ~1px bilinear edge bleed.
    assert!((cb.x0 - 20).abs() <= 2 && (cb.y0 - 20).abs() <= 2, "origin ~ (20,20): {cb:?}");
    assert!((cb.width() as i32 - 80).abs() <= 3 && (cb.height() as i32 - 40).abs() <= 3, "size ~ 80x40: {cb:?}");
}

#[test]
fn cut_and_clear_on_the_background_fill_with_the_background_colour() {
    // The Background keeps its transparency lock: cutting fills with the background colour.
    let fresh = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.tools.background = [1.0, 0.0, 0.0, 1.0];
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        s
    };
    let px = |s: &Session, i: usize, x: i32, y: i32| s.active().unwrap().doc.layers[i].surface().unwrap().pixel(x, y);
    for cmd in ["edit.cut", "edit.clear", "layer.new.layerViaCut"] {
        let mut s = fresh();
        s.execute(cmd, json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert!(d.is_background(d.layers[0].id), "{cmd}: still the Background");
        assert_eq!(px(&s, 0, 8, 8), vec![1.0, 0.0, 0.0, 1.0], "{cmd}: cut area is the background colour");
        assert_eq!(px(&s, 0, 30, 20), vec![1.0, 1.0, 1.0, 1.0], "{cmd}: outside untouched");
    }
    // Layer via Cut still lifts the original pixels onto the new layer.
    let mut s = fresh();
    s.execute("layer.new.layerViaCut", json!({})).unwrap();
    assert_eq!(px(&s, 1, 8, 8), vec![1.0, 1.0, 1.0, 1.0]);
    // Once it's a normal layer, cutting makes transparency again.
    let mut s = fresh();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    s.execute("edit.clear", json!({})).unwrap();
    assert_eq!(px(&s, 0, 8, 8)[3], 0.0);
}

#[test]
fn layer_via_copy_of_a_background_keeps_colour_and_partial_selection() {
    // #668: lift reads only the selected area now; the result must be what converting the
    // whole layer gave: the colour, with the selection's coverage as alpha.
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "#336699"})).unwrap();
        s.edit("partial selection", |doc, _| {
            let mut m = photocraft_raster::Surface::new(PixelFormat::GRAY8);
            m.fill_rect(Rect::new(5, 5, 15, 15), &[1.0]);
            m.fill_rect(Rect::new(15, 5, 20, 15), &[0.5]);
            doc.selection = Some(m);
            Ok(())
        })
        .unwrap();
        let id = s.execute("layer.new.layerViaCopy", json!({})).unwrap()["layer"].as_u64().unwrap();
        let d = s.active().unwrap();
        let surf = d.doc.layer(LayerId(id)).unwrap().surface().unwrap();
        assert!(surf.format().alpha, "@{depth}");
        let full = surf.read_region(Rect::new(6, 6, 7, 7));
        let half = surf.read_region(Rect::new(16, 6, 17, 7));
        for (got, want) in full.iter().zip([0.2, 0.4, 0.6, 1.0]) {
            assert!((got - want).abs() < 0.01, "@{depth}: {full:?}");
        }
        assert!((half[3] - 0.5).abs() < 0.01 && (half[0] - 0.2).abs() < 0.01, "@{depth}: {half:?}");
        assert_eq!(surf.content_bounds(), Rect::new(5, 5, 20, 15), "@{depth}");
    }
}

#[test]
fn cut_clears_and_layer_via_copy_cut() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
    s.execute("edit.cut", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(20, 10, 50, 30));
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
    s.execute("layer.new.layerViaCut", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 20, 30));
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(20, 10, 50, 30));
    // Without a selection ⌘J duplicates the layer.
    let mut s = session();
    s.execute("layer.new.layerViaCopy", json!({})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), 3);
    assert!(d.layers[2].name.ends_with("copy"));
}

/// #1096: Layer via Copy and Cut deselect, as Photoshop does, so a following Free Transform
/// moves the whole new layer; one undo brings back both the selection and the old layers.
#[test]
fn layer_via_copy_and_cut_deselect_in_one_step() {
    for cmd in ["layer.new.layerViaCopy", "layer.new.layerViaCut"] {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
        let (layers, steps) = (s.active().unwrap().doc.layers.len(), s.active().unwrap().history.past_len());
        s.execute(cmd, json!({})).unwrap();
        let st = s.active().unwrap();
        assert!(st.doc.selection.is_none(), "{cmd}: deselected");
        assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1), "{cmd}");
        assert!(s.undo());
        let st = s.active().unwrap();
        assert_eq!(st.doc.layers.len(), layers, "{cmd}: undo removes the layer");
        assert!(st.doc.selection.is_some(), "{cmd}: and restores the selection");
    }
}

/// #2777: with several layers selected and no pixel selection, Layer via Copy (Cmd+J) copies
/// every selected layer in one step, each copy above its original and the copies selected.
/// With a pixel selection it still copies the active layer's selected pixels only.
#[test]
fn layer_via_copy_duplicates_every_selected_layer() {
    let mut s = session();
    let a = s.active().unwrap().active_layer.unwrap();
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("layer.new.layerViaCopy", json!({})).unwrap();
    let st = s.active().unwrap();
    let ids: Vec<LayerId> = st.doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(ids.len(), 5, "background, a, a copy, b, b copy");
    assert_eq!((ids[1], ids[3]), (a, b), "each copy sits above its original");
    let mut selected = st.selected_layers();
    selected.sort_by_key(|l| l.0);
    assert_eq!(selected, vec![ids[2], ids[4]], "the copies become the selection");
    assert_eq!(st.history.past_len(), steps + 1, "one undo step");
    assert!(s.undo());
    s.execute("layer.select", json!({"layer": b.0})).unwrap();
    s.execute("layer.select", json!({"layer": a.0, "mode": "toggle"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layerViaCopy", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 4, "a pixel selection copies one layer's pixels");
}

fn active_content(s: &Session) -> LayerContent {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
}

/// #541, #2428: with no selection, Copy takes a smart object whole and Paste makes a smart
/// object again, with contents of its own; with a selection, the selected pixels paste as
/// pixels. Cut edits pixels, so it's refused; Layer via Copy and Duplicate Layer copy the layer.
#[test]
fn copy_a_smart_object_layer() {
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let LayerContent::Smart(original) = active_content(&s) else { panic!("smart object") };
    assert!(s.is_enabled("edit.copy"));
    assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 40, 20]), "no selection: the whole layer");
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let LayerContent::Smart(pasted) = active_content(&s) else { panic!("the paste is a smart object (#2428)") };
    assert_eq!(pasted.source, original.source, "the same contents");
    assert_ne!(pasted.contents_id, original.contents_id, "but not linked to the copied one");
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30));
    s.execute("edit.paste", json!({"center": [70, 70]})).unwrap();
    assert!(matches!(active_content(&s), LayerContent::Smart(_)));
    assert_eq!(active_bounds(&s), Rect::new(50, 60, 90, 80), "placed as pasted pixels are");
    s.undo();
    s.undo();
    assert!(matches!(active_content(&s), LayerContent::Smart(_)));
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 10, 10]), "only the selected part");
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    assert!(matches!(active_content(&s), LayerContent::Raster(_)), "a selection copies pixels");
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 20, 20));
    s.undo();
    let cut = s.execute("edit.cut", json!({})).unwrap_err().to_string();
    assert!(cut.contains("not directly editable"), "{cut}");
    assert!(matches!(active_content(&s), LayerContent::Smart(_)));
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layerViaCopy", json!({})).unwrap();
    assert!(matches!(active_content(&s), LayerContent::Smart(_)), "⌘J without a selection duplicates the smart object");
    s.execute("layer.duplicate", json!({})).unwrap();
    assert!(matches!(active_content(&s), LayerContent::Smart(_)));
    assert_eq!(s.active().unwrap().doc.layers.len(), 4);
}

/// #2478: with several layers selected and no pixel selection, Copy takes every one of them
/// and Paste recreates them all, in order, as one undo step; with a pixel selection Copy still
/// takes the active layer's selected pixels only.
#[test]
fn copy_and_paste_several_selected_layers() {
    let mut s = session();
    let a = s.active().unwrap().active_layer.unwrap();
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(b).unwrap().surface_mut().unwrap().fill_rect(Rect::new(60, 60, 80, 90), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    assert!(s.is_enabled("edit.copy"));
    assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 70, 80]), "both layers' pixels");
    let before = s.active().unwrap().doc.layers.len();
    let steps = s.active().unwrap().history.past_len();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), before + 2, "every selected layer is pasted");
    assert_eq!(st.history.past_len(), steps + 1, "one undo step");
    let n = st.doc.layers.len();
    let (pa, pb) = (st.doc.layers[n - 2].surface().unwrap(), st.doc.layers[n - 1].surface().unwrap());
    assert_eq!(pa.content_bounds(), Rect::new(10, 10, 50, 30), "the lower layer first");
    assert_eq!(pb.content_bounds(), Rect::new(60, 60, 80, 90));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), before);

    // A pixel selection takes the active layer's selected pixels, as a single layer.
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before + 1);
}

/// #2478, #3588: several layers copied in one document paste into another, in that document's
/// bit depth and with their own pixels, in order, as one undo step.
#[test]
fn paste_several_layers_into_another_document() {
    for depth in [16, 32] {
        let mut s = session();
        let a = s.active().unwrap().active_layer.unwrap();
        let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.edit("paint", |doc, _| {
            doc.layer_mut(b).unwrap().surface_mut().unwrap().fill_rect(Rect::new(60, 60, 80, 90), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();

        s.execute("file.new", json!({"width": 120, "height": 120, "depth": depth})).unwrap();
        assert_eq!(s.active_index().unwrap(), 1, "the new document is the active tab");
        let (before, steps) = (s.active().unwrap().doc.layers.len(), s.active().unwrap().history.past_len());
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layers.len(), before + 2, "{depth}-bit: both layers arrive");
        assert_eq!(st.history.past_len(), steps + 1, "{depth}-bit: one undo step");
        let n = st.doc.layers.len();
        let (pa, pb) = (st.doc.layers[n - 2].surface().unwrap(), st.doc.layers[n - 1].surface().unwrap());
        assert_eq!(pa.format().sample, st.doc.depth, "{depth}-bit: converted to the target depth");
        assert_eq!(pb.format().sample, st.doc.depth, "{depth}-bit: converted to the target depth");
        assert_eq!(pa.content_bounds(), Rect::new(10, 10, 50, 30), "{depth}-bit: the lower layer first");
        assert_eq!(pb.content_bounds(), Rect::new(60, 60, 80, 90));
        let (mut red, mut blue) = ([[0.0f32; 4]; 1], [[0.0f32; 4]; 1]);
        pa.read_rgba_into(Rect::new(20, 20, 21, 21), &mut red);
        pb.read_rgba_into(Rect::new(70, 70, 71, 71), &mut blue);
        assert!(red[0][0] > 0.9 && red[0][1] < 0.1 && red[0][2] < 0.1, "{depth}-bit: red kept: {:?}", red[0]);
        assert!(blue[0][2] > 0.9 && blue[0][0] < 0.1 && blue[0][1] < 0.1, "{depth}-bit: blue kept: {:?}", blue[0]);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.layers.len(), before, "{depth}-bit: undo removes both");
        assert_eq!(s.documents().len(), 2, "the source document is untouched");
    }
}

/// #2478: the Background in a multi-layer copy pastes as an ordinary, unlocked layer.
#[test]
fn copying_the_background_with_another_layer_pastes_an_ordinary_layer() {
    let mut s = session();
    let a = s.active().unwrap().active_layer.unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": bg.0, "mode": "toggle"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), 4);
    let pasted = &st.doc.layers[st.doc.layers.len() - 2];
    assert!(!st.doc.is_background(pasted.id));
    assert_eq!(pasted.locks, Default::default());
    assert!(st.doc.is_background(st.doc.layers[0].id), "the real Background is untouched");
}

/// #2428: a smart object copied in one document pastes into another as a smart object, in
/// that document's bit depth.
#[test]
fn paste_a_smart_object_into_another_document() {
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 40, "depth": 16})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let LayerContent::Smart(sm) = active_content(&s) else { panic!("the paste is a smart object") };
    assert_eq!(sm.cache.map(|c| c.format().sample), Some(s.active().unwrap().doc.depth), "converted to 16 bits");
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30), "on the canvas: in place");
}

/// #3584: a PSD-embedded linked smart object (its bytes in the document's `lnk2` block, its
/// source a uuid) can still be opened after Copy + Paste into another document.
#[test]
fn paste_keeps_a_psd_embedded_smart_object_openable() {
    use std::sync::Arc;
    let mut s = session();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let outer = s.active().unwrap().active_layer.unwrap();
    s.edit("embedded linked source", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(outer).ok_or(EngineError::NoDocument)?.content else {
            return Err(EngineError::Other("not a smart object".into()));
        };
        sm.source = photocraft_doc::SmartSource::Linked { path: "uuid-1".into() };
        let linked = photocraft_io::linked::LinkedFile { uuid: "uuid-1".into(), file_name: "in.png".into(), bytes: vec![1, 2, 3, 4] };
        doc.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(photocraft_io::linked::encode_linked_file(&linked))));
        Ok(())
    })
    .unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let LayerContent::Smart(sm) = active_content(&s) else { panic!("the paste is a smart object") };
    assert!(
        crate::smart_cmds::source_bytes(&s.active().unwrap().doc.metadata, &sm.source).is_some(),
        "the pasted PSD-embedded linked source must resolve (#3584)"
    );
}

/// Type and shape layers copy their rendered pixels too; groups and adjustments have none.
#[test]
fn copy_reads_any_layer_that_shows_pixels() {
    let mut s = session();
    s.execute("type.create", json!({"text": "Hi", "size": 30, "x": 20, "y": 60})).unwrap();
    assert!(!s.is_enabled("edit.cut"));
    assert!(s.execute("edit.copy", json!({})).is_ok());
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert!(s.is_enabled("edit.copy"), "no selection: the group is copied whole (#2670)");
    s.execute("select.all", json!({})).unwrap();
    assert!(!s.is_enabled("edit.copy"));
}

/// #2670: with no selection, Copy takes a type layer, a shape layer or a group whole and
/// Paste recreates it, into another document too (with fresh ids, in its bit depth); with a
/// selection, Copy takes pixels.
#[test]
fn copy_paste_keeps_type_shape_and_group_layers() {
    let mut s = session();
    s.execute("type.create", json!({"text": "Hi", "size": 30, "x": 20, "y": 60})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let LayerContent::Text(t) = active_content(&s) else { panic!("the paste is a type layer") };
    assert_eq!(t.text, "Hi");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert!(matches!(active_content(&s), LayerContent::Raster(_)), "a selection copies pixels");
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let LayerContent::Group(src) = active_content(&s) else { panic!("a group") };
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 40, "depth": 16})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let LayerContent::Group(g) = active_content(&s) else { panic!("the paste is a group") };
    let [shape] = g.children.as_slice() else { panic!("one child") };
    assert!(matches!(shape.content, LayerContent::Shape(_)), "the shape stays a shape");
    assert_ne!(shape.id, src.children[0].id, "fresh ids");
    assert_eq!(shape.surface().map(|c| c.format().sample), Some(s.active().unwrap().doc.depth), "converted to 16 bits");
}

#[test]
fn cross_document_group_paste_renders_shapes_on_destination_canvas() {
    for (copy_to_document, depth) in [(false, 8), (true, 16)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":100})).unwrap();
        let mut ids = Vec::new();
        for x in [10, 50] {
            s.execute("shape.create", json!({"kind":"rect","rect":[x,20,20,30],"fill":"#ff0000"})).unwrap();
            s.execute("layer.layerStyle.dropShadow", json!({"size":4,"distance":3})).unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        s.active_mut().unwrap().selected_layers = ids;
        s.execute("layer.groupLayers", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("file.new", json!({"width":800,"height":600,"depth":depth})).unwrap();
        if copy_to_document {
            s.execute("layer.copyToDocument", json!({"source":0,"document":1,"center":true})).unwrap();
        } else {
            s.execute("edit.paste", json!({"center":[400,300]})).unwrap();
        }
        let st = s.active().unwrap();
        let group = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert!(!crate::transform_cmds::transform_bounds(&st.doc, group).is_empty());
        assert_eq!(group.children().unwrap().len(), 2);
        for child in group.children().unwrap() {
            let LayerContent::Shape(shape) = &child.content else { panic!("shape") };
            let cache = shape.cache.as_ref().unwrap();
            assert_eq!(cache.format().sample, st.doc.depth);
            assert!(!cache.content_bounds().is_empty(), "destination cache must not be clipped to source canvas");
            assert!(cache.content_bounds().x0 > 100);
            let fresh = photocraft_vector::render_shape(shape, st.doc.pixel_format(), st.doc.bounds());
            assert_eq!(cache, &fresh);
        }
        let before = st.doc.clone();
        s.execute("layer.translate", json!({"dx":10,"dy":10})).unwrap();
        assert_ne!(s.active().unwrap().doc, before);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn paste_repairs_uncached_shapes_before_transform_and_effect_rendering() {
    let mut s = session();
    s.execute("shape.create", json!({"kind":"rect","rect":[10,10,40,20],"fill":"#ff0000"})).unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"size":4,"distance":3})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let layers = s.clipboard.as_mut().unwrap().layers.as_mut().unwrap();
    let LayerContent::Shape(sh) = &mut layers.layers[0].content else { panic!("shape") };
    sh.cache = None;
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(sh) = &layer.content else { panic!("shape") };
    assert!(sh.cache.is_some());
    assert!(!crate::transform_cmds::transform_bounds(&st.doc, layer).is_empty());
    let expected = photocraft_vector::render_shape(sh, st.doc.pixel_format(), st.doc.bounds());
    assert_eq!(sh.cache.as_ref().unwrap(), &expected);
    let rendered = photocraft_compose::render_layer_in(&st.doc, layer, st.doc.bounds());
    let mut fresh_doc = (*st.doc).clone();
    let snapshot = fresh_doc.clone();
    if let LayerContent::Shape(shape) = &mut fresh_doc.layer_mut(layer.id).unwrap().content {
        crate::vector_cmds::refresh_shape(&snapshot, shape);
    }
    let fresh = photocraft_compose::render_layer_in(&fresh_doc, fresh_doc.layer(layer.id).unwrap(), fresh_doc.bounds());
    assert_eq!(rendered.px, fresh.px, "pasted effects match a freshly rebuilt shape");
    assert!(s.undo());
    assert!(s.redo());
    let st = s.active().unwrap();
    assert!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().is_some());
}

/// #2344: fill and adjustment layers have no pixels of their own, so with no selection Copy
/// takes them whole and Paste recreates them, in the same or another document, in one step.
#[test]
fn copy_paste_keeps_fill_and_adjustment_layers() {
    for (depth, other) in [(8, 16), (16, 32), (32, 8)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 60, "height": 40, "depth": depth})).unwrap();
        s.execute("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff", "angle": 0})).unwrap();
        let LayerContent::Fill(fill) = active_content(&s) else { panic!("a fill layer") };
        let src = s.active().unwrap().active_layer.unwrap();
        assert!(s.is_enabled("edit.copy"), "{depth}-bit: a fill layer can be copied");
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([0, 0, 60, 40]), "{depth}-bit: what it shows");
        let clip = s.clipboard.clone().unwrap();
        assert!(clip.surface.rgba(59, 20)[0] > clip.surface.rgba(0, 20)[0], "{depth}-bit: the clip holds the gradient's pixels");
        let steps = s.active().unwrap().history.past_len();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{depth}-bit: one history step");
        assert_eq!(active_content(&s), LayerContent::Fill(fill.clone()), "{depth}-bit: the paste is the same fill layer");
        assert_ne!(s.active().unwrap().active_layer, Some(src), "{depth}-bit: a new layer");
        s.undo();
        assert_eq!(s.active().unwrap().active_layer, Some(src), "{depth}-bit: undo removes the paste");
        s.redo();
        assert!(matches!(active_content(&s), LayerContent::Fill(_)), "{depth}-bit: redo brings it back");

        s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
        let LayerContent::Adjustment(levels) = active_content(&s) else { panic!("an adjustment layer") };
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([0, 0, 60, 40]), "{depth}-bit: it applies to the canvas");
        s.execute("file.new", json!({"width": 30, "height": 30, "depth": other})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        assert_eq!(active_content(&s), LayerContent::Adjustment(levels), "{depth}-bit: the same Levels layer in the {other}-bit document");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "{depth}-bit: pasted above the Background");
    }
}

/// #2344: several adjustment layers, which show nothing alone, still copy and paste together.
#[test]
fn several_adjustment_layers_copy_together() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    let a = s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.select", json!({"layer": b, "mode": "toggle"})).unwrap();
    assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([0, 0, 40, 40]));
    s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), 5, "both are pasted");
    assert!(d.layers[3..].iter().all(|l| matches!(l.content, LayerContent::Adjustment(_))));
}

#[test]
fn merge_visible_keeps_hidden_layers() {
    let mut s = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    let hidden = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": hidden.0, "visible": false})).unwrap();
    s.execute("layer.mergeVisible", json!({})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[0].name, "Background");
    assert_eq!(d.layers[1].id, hidden);
    assert_eq!(d.layers[0].surface().unwrap().pixel(20, 20)[..3], [1.0, 0.0, 0.0]);
}

#[test]
fn auto_tone_stretches_a_low_contrast_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.edit("grey ramp", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        for x in 0..64 {
            let v = 0.3 + 0.4 * x as f32 / 63.0;
            surf.fill_rect(Rect::new(x, 0, x + 1, 64), &[v, v * 0.9, v * 0.8, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    for cmd in ["image.autoTone", "image.autoContrast", "image.autoColor"] {
        s.execute(cmd, json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        let (a, b) = (d.layers[0].surface().unwrap().pixel(0, 0), d.layers[0].surface().unwrap().pixel(63, 0));
        // Auto Contrast keeps colour (shared points): only the extreme channels reach 0 and 1.
        let (lo, hi) = if cmd == "image.autoContrast" { (a[2], b[0]) } else { (a[0], b[0]) };
        assert!(lo < 0.05 && hi > 0.95, "{cmd}: {a:?} {b:?}");
        s.undo();
    }
}

/// #352: ⌥⇧⌘T steps and repeats: each press adds a copy one transform further on, and each
/// is one undo step.
#[test]
fn transform_again_on_a_copy_steps_and_repeats() {
    let mut s = session();
    assert!(s.execute("edit.transform.againCopy", json!({})).is_err(), "nothing to repeat yet");
    assert_eq!(s.active().unwrap().doc.layers.len(), 2, "a refusal leaves no copy");
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    for (i, x) in [30, 40].into_iter().enumerate() {
        s.execute("edit.transform.againCopy", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 3 + i);
        assert_eq!(active_bounds(&s), Rect::new(x, 10, x + 40, 30), "the copy moved one step further");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1 + i, "one history step per press");
    }
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Transform Again"));
    // The original and the first copy stay where they were.
    let d = &s.active().unwrap().doc;
    let b = |i: usize| d.layers[i].surface().unwrap().content_bounds();
    assert_eq!((b(1), b(2)), (Rect::new(20, 10, 60, 30), Rect::new(30, 10, 70, 30)));
    // Undo removes the copy and its move together.
    s.undo();
    assert_eq!(s.active().unwrap().doc.layers.len(), 3);
    assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
    // A copy that can't move (position locked) is taken back.
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": id.0, "locks": {"position": true}})).unwrap();
    let layers = s.active().unwrap().doc.layers.len();
    assert!(s.execute("edit.transform.againCopy", json!({})).unwrap_err().to_string().contains("locked"));
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    assert!(!s.active().unwrap().history.can_redo(), "nothing to redo");
}

#[test]
fn toggle_last_state_and_transform_again() {
    let mut s = session();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
    s.execute("edit.transform.again", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
    s.execute("edit.toggleLastState", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(20, 10, 60, 30));
    s.execute("edit.toggleLastState", json!({})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
}

#[test]
fn guides_are_undoable() {
    let mut s = session();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 40})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 12.5})).unwrap();
    s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 60})).unwrap();
    let g = &s.active().unwrap().doc.guides;
    assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![60.0], vec![12.5]));
    s.execute("view.deleteGuide", json!({"orientation": "horizontal", "index": 0})).unwrap();
    s.undo();
    assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![12.5]);
    s.execute("view.clearGuides", json!({})).unwrap();
    assert!(s.active().unwrap().doc.guides.vertical.is_empty());
    assert!(s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 3, "position": 1})).is_err());
}

/// #2955: the rulers' zero point is set and reset by command, undoable, and bad params are
/// refused without touching the document.
#[test]
fn ruler_origin_is_set_reset_and_undoable() {
    let mut s = session();
    assert_eq!(s.active().unwrap().doc.ruler_origin, [0.0, 0.0]);
    assert_eq!(s.execute("view.setRulerOrigin", json!({"x": 40, "y": 12.5})).unwrap(), json!({"x": 40.0, "y": 12.5}));
    assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
    s.execute("view.resetRulerOrigin", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.ruler_origin, [0.0, 0.0]);
    s.undo();
    assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
    for bad in [json!({}), json!({"x": 1}), json!({"x": "a", "y": 2}), json!({"x": 1e300, "y": 0}), json!({"x": -2e7, "y": 0}), json!(null), json!([1, 2])] {
        assert!(s.execute("view.setRulerOrigin", bad.clone()).is_err(), "{bad} is refused");
    }
    assert_eq!(s.active().unwrap().doc.ruler_origin, [40.0, 12.5]);
    let mut empty = Session::new();
    assert!(empty.execute("view.setRulerOrigin", json!({"x": 1, "y": 2})).is_err());
    assert!(empty.execute("view.resetRulerOrigin", json!({})).is_err());
}

/// The last guide added in `orientation` for `position`.
fn new_guide(s: &mut Session, orientation: &str, position: Value) -> f64 {
    s.execute("view.newGuide", json!({"orientation": orientation, "position": position})).unwrap();
    let g = &s.active().unwrap().doc.guides;
    *if orientation == "vertical" { &g.vertical } else { &g.horizontal }.last().unwrap()
}

#[test]
fn guide_positions_take_photoshop_units() {
    let mut s = Session::new();
    // 4 × 2 inches at 100 ppi.
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    // Percent follows the guide's own axis: the width for a vertical guide, the height for a horizontal one.
    assert_eq!(new_guide(&mut s, "vertical", json!("50%")), 200.0);
    assert_eq!(new_guide(&mut s, "horizontal", json!("50%")), 100.0);
    for one_inch in ["1in", "1 in", "2.54cm", "25.4 mm", "72pt", "6pc", "100px", "100", "1,0 inch"] {
        assert!((new_guide(&mut s, "vertical", json!(one_inch)) - 100.0).abs() < 1e-3, "{one_inch}");
    }
    // Fractional and negative positions are kept; a plain number is pixels, as before.
    assert_eq!(new_guide(&mut s, "horizontal", json!("12.5px")), 12.5);
    assert_eq!(new_guide(&mut s, "vertical", json!("-10px")), -10.0);
    assert_eq!(new_guide(&mut s, "vertical", json!(40)), 40.0);
    // Points and picas follow Units & Rulers › Point Size.
    s.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
    assert!((new_guide(&mut s, "vertical", json!("72.27pt")) - 100.0).abs() < 1e-3);
    // A guide moved with a unit lands where a new one would.
    s.execute("view.moveGuide", json!({"orientation": "horizontal", "index": 0, "position": "25%"})).unwrap();
    assert_eq!(s.active().unwrap().doc.guides.horizontal[0], 50.0);
}

#[test]
fn new_guides_keep_their_own_colour_through_moves_deletes_and_undo() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    let color = |s: &Session, vertical: bool, p: f64| s.active().unwrap().doc.guides.color(vertical, p);
    // Without a colour a guide uses the preference colour, as ruler-drag guides always did.
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    assert_eq!(color(&s, true, 10.0), None);
    assert!(s.active().unwrap().doc.guides.colors.is_empty());
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 20, "color": "magenta"})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 20, "color": "#123456"})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 30, "color": "LightBlue"})).unwrap();
    assert_eq!(color(&s, true, 20.0), Some([0xff, 0x00, 0xff]));
    assert_eq!(color(&s, false, 20.0), Some([0x12, 0x34, 0x56]), "the same position across is a different guide");
    assert_eq!(color(&s, true, 30.0), Some([0x33, 0x66, 0xff]), "names are case-insensitive");
    // Moving takes the colour along.
    s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 1, "position": 25})).unwrap();
    assert_eq!((color(&s, true, 20.0), color(&s, true, 25.0)), (None, Some([0xff, 0x00, 0xff])));
    // Undo restores it where it was.
    s.undo();
    assert_eq!((color(&s, true, 20.0), color(&s, true, 25.0)), (Some([0xff, 0x00, 0xff]), None));
    // Deleting drops it; clearing drops them all.
    s.execute("view.deleteGuide", json!({"orientation": "vertical", "index": 2})).unwrap();
    assert_eq!(color(&s, true, 30.0), None);
    assert_eq!(s.active().unwrap().doc.guides.colors.len(), 2);
    s.execute("view.clearGuides", json!({})).unwrap();
    assert!(s.active().unwrap().doc.guides.colors.is_empty());
}

#[test]
fn moving_one_of_two_coloured_guides_at_one_position_leaves_the_other_its_colour() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    for _ in 0..2 {
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 50, "color": "green"})).unwrap();
    }
    s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 60})).unwrap();
    let g = &s.active().unwrap().doc.guides;
    assert_eq!((g.color(true, 50.0), g.color(true, 60.0)), (Some([0x33, 0xff, 0x33]), Some([0x33, 0xff, 0x33])));
}

#[test]
fn cyan_is_the_default_guide_colour_so_ruler_guides_read_as_cyan() {
    let default = crate::prefs::Preferences::default().guides_grid_and_slices.guide_color;
    let cyan = GUIDE_COLORS.iter().find(|(name, ..)| *name == "cyan").map(|(.., rgb)| *rgb);
    assert_eq!(cyan, crate::prefs::parse_hex(&default));
}

#[test]
fn bad_guide_colours_are_refused_without_a_guide() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    let undo_before = s.active().unwrap().history.can_undo();
    for bad in [json!("puce"), json!("#12345"), json!("#gggggg"), json!(""), json!(7), json!(null), json!(["cyan"])] {
        assert!(s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10, "color": bad.clone()})).is_err(), "{bad}");
    }
    let g = &s.active().unwrap().doc.guides;
    assert!(g.vertical.is_empty() && g.colors.is_empty());
    assert_eq!(s.active().unwrap().history.can_undo(), undo_before);
}

#[test]
fn editing_a_guide_changes_what_is_given_in_one_undoable_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 20, "color": "magenta"})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 30})).unwrap();
    let guides = |s: &Session| s.active().unwrap().doc.guides.clone();
    // Position only: the guide keeps its index and its own colour.
    s.execute("view.editGuide", json!({"orientation": "vertical", "index": 1, "position": "50%"})).unwrap();
    let g = guides(&s);
    assert_eq!(g.vertical, vec![10.0, 200.0, 30.0]);
    assert_eq!((g.color(true, 20.0), g.color(true, 200.0)), (None, Some([0xff, 0x00, 0xff])));
    // Colour only: an uncoloured guide gets one; the position stays.
    s.execute("view.editGuide", json!({"orientation": "vertical", "index": 0, "color": "#123456"})).unwrap();
    assert_eq!(guides(&s).color(true, 10.0), Some([0x12, 0x34, 0x56]));
    // Turning it: % is of the new orientation's extent, and the colour goes along.
    s.execute("view.editGuide", json!({"orientation": "vertical", "index": 1, "newOrientation": "horizontal", "position": "25%"})).unwrap();
    let g = guides(&s);
    assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![10.0, 30.0], vec![50.0]));
    assert_eq!((g.color(true, 200.0), g.color(false, 50.0)), (None, Some([0xff, 0x00, 0xff])));
    // One step each: undo restores the vertical guide with its colour.
    s.undo();
    let g = guides(&s);
    assert_eq!((g.vertical.clone(), g.horizontal.is_empty()), (vec![10.0, 200.0, 30.0], true));
    assert_eq!(g.color(true, 200.0), Some([0xff, 0x00, 0xff]));
}

#[test]
fn bad_guide_edits_are_refused_without_a_history_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10, "color": "green"})).unwrap();
    let before = s.active().unwrap().doc.guides.clone();
    for bad in [
        json!({}),
        json!({"orientation": "vertical"}),
        json!({"orientation": "vertical", "index": 1}),
        json!({"orientation": "horizontal", "index": 0}),
        json!({"orientation": "vertical", "index": -1}),
        json!({"orientation": "vertical", "index": "0"}),
        json!({"orientation": "vertical", "index": u64::MAX}),
        json!({"orientation": "diagonal", "index": 0}),
        json!({"orientation": "vertical", "index": 0, "newOrientation": 3}),
        json!({"orientation": "vertical", "index": 0, "position": "far"}),
        json!({"orientation": "vertical", "index": 0, "position": 1e12}),
        json!({"orientation": "vertical", "index": 0, "color": "puce"}),
    ] {
        assert!(s.execute("view.editGuide", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.active().unwrap().doc.guides, before);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("New Guide"));
}

#[test]
fn bad_guide_positions_are_refused_without_a_guide() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 100})).unwrap();
    let undo_before = s.active().unwrap().history.can_undo();
    for bad in [
        json!("abc"),
        json!("50 furlongs"),
        json!(""),
        json!("%"),
        json!("NaN"),
        json!("inf"),
        json!("1e400"),
        json!("2000000px"),
        json!(1e300),
        json!(true),
        json!([1]),
    ] {
        assert!(s.execute("view.newGuide", json!({"orientation": "vertical", "position": bad.clone()})).is_err(), "{bad}");
    }
    assert!(s.execute("view.newGuide", json!({"orientation": "vertical"})).is_err(), "no position");
    let g = &s.active().unwrap().doc.guides;
    assert!(g.vertical.is_empty() && g.horizontal.is_empty());
    assert_eq!(s.active().unwrap().history.can_undo(), undo_before, "a refused guide records no history step");
}

/// Guides keep their exact place (#2918): 1 cm at 300 ppi through New Guide and .pcraft, and
/// a 1/32 px PSD position past 2^19 px, where f32 has no 1/32 steps left.
#[test]
fn guides_keep_sub_pixel_positions_through_pcraft_and_psd() {
    let (cm, far) = (300.0 / 2.54, 600_000.031_25);
    let mut s = session();
    s.execute("view.newGuide", json!({"orientation": "horizontal", "position": cm})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": far})).unwrap();
    fn wide(v: impl Into<f64>) -> f64 {
        v.into()
    }
    let exact = |g: &photocraft_doc::Guides| (wide(g.horizontal[0]), wide(g.vertical[0]));
    let doc = &s.active().unwrap().doc;
    assert_eq!(exact(&doc.guides), (cm, far));
    let pcraft = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    assert_eq!(exact(&photocraft_format::load_from_bytes(&pcraft).unwrap().guides), (cm, far));
    let psd = photocraft_io::export(doc, "psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("g.psd", &psd).unwrap().document.guides;
    assert_eq!(wide(back.vertical[0]), far, "1032 stores 1/32 px");
}

/// `lift` as it was before #4516: the whole area through one `f32` buffer. The oracle for
/// the tiled version.
fn lift_reference(src: &Surface, sel: Option<&Surface>, canvas: Rect) -> Clip {
    let fmt = src.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let area = match sel {
        Some(m) => m.content_bounds().intersect(&canvas),
        None => src.content_bounds().intersect(&canvas),
    };
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Clip { surface: out, bounds: Rect::EMPTY, layers: None, resolution: None, source: None };
    }
    let px = lift_region(src, sel, area, with_alpha.channels()).unwrap();
    out.try_write_region(area, &px).unwrap();
    out.prune();
    let bounds = out.content_bounds();
    Clip { surface: out, bounds, layers: None, resolution: None, source: None }
}

fn noise(fmt: PixelFormat, r: Rect, seed: u32) -> Surface {
    let mut s = Surface::new(fmt);
    let n = r.width() as usize * r.height() as usize * fmt.channels();
    let data: Vec<f32> = (0..n as u32)
        .map(|i| {
            let mut h = seed.wrapping_mul(0x9e37_79b9) ^ i.wrapping_mul(0x85eb_ca6b);
            h ^= h >> 13;
            h = h.wrapping_mul(0xc2b2_ae35);
            (h >> 16) as f32 / 65535.0
        })
        .collect();
    s.write_region(r, &data);
    s
}

fn assert_same_clip(a: &Clip, b: &Clip, what: &str) {
    assert_eq!(a.bounds, b.bounds, "{what}: bounds");
    assert_eq!(a.surface.format(), b.surface.format(), "{what}: format");
    let r = a.bounds.union(&b.bounds);
    if !r.is_empty() {
        let (pa, pb) = (a.surface.read_region(r), b.surface.read_region(r));
        assert!(pa.iter().zip(&pb).all(|(x, y)| x.to_bits() == y.to_bits()), "{what}: pixels differ");
    }
}

/// The tiled `lift` gives exactly the old result (#4516): with and without alpha, at every
/// depth, with no selection, Select All, a partial feathered selection and a selection off
/// the tile grid, and for content reaching past the canvas.
#[test]
fn tiled_lift_matches_the_whole_area_lift() {
    use photocraft_color::{ColorMode, SampleType};
    let canvas = Rect::new(0, 0, 700, 600);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for alpha in [true, false] {
            let fmt = PixelFormat::new(ColorMode::Rgb, sample, alpha);
            let src = noise(fmt, Rect::new(-40, -30, 760, 640), 7);
            let mut all = Surface::new(PixelFormat::GRAY8);
            all.fill_rect(canvas, &[1.0]);
            let mut partial = Surface::new(PixelFormat::GRAY8);
            partial.fill_rect(Rect::new(100, 50, 600, 550), &[1.0]);
            partial.write_region(Rect::new(90, 40, 610, 50), &vec![0.5; 520 * 10]);
            let off_grid = noise(PixelFormat::GRAY8, Rect::new(13, 270, 333, 290), 9);
            for (name, sel) in [("no selection", None), ("select all", Some(&all)), ("partial", Some(&partial)), ("off grid", Some(&off_grid))] {
                let what = format!("{sample:?} alpha {alpha} {name}");
                let new = lift(&src, sel, canvas).unwrap();
                assert_same_clip(&new, &lift_reference(&src, sel, canvas), &what);
            }
        }
    }
}

/// Copy shares the layer's whole tiles instead of copying them.
#[test]
fn copy_shares_whole_tiles_with_the_layer() {
    let src = noise(PixelFormat::RGBA8, Rect::new(0, 0, 1024, 768), 3);
    let canvas = Rect::new(0, 0, 1024, 768);
    let mut all = Surface::new(PixelFormat::GRAY8);
    all.fill_rect(canvas, &[1.0]);
    for sel in [None, Some(&all)] {
        let clip = lift(&src, sel, canvas).unwrap();
        let shared = src.tiles().filter(|(c, t)| clip.surface.tile(**c).is_some_and(|o| std::sync::Arc::ptr_eq(o, t))).count();
        assert_eq!(shared, src.tile_count(), "every tile is shared");
    }
}
