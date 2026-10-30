use super::*;
use photocraft_algo::segment::Rng;

/// A document whose background shows `px(x, y)` (RGB, 0–1), at the given depth.
fn session_with(w: u32, h: u32, depth: u32, px: impl Fn(u32, u32) -> [f32; 3]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        let mut data = Vec::with_capacity((w * h) as usize * n);
        for y in 0..h {
            for x in 0..w {
                let c = px(x, y);
                data.extend_from_slice(&c[..n.min(3)]);
                if n == 4 {
                    data.push(1.0);
                }
            }
        }
        bg.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn noise_field(w: u32, h: u32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..w * h).map(|_| rng.normal()).collect()
}

/// Warm textured left part (x < 60), cool right part.
fn two_regions(depth: u32) -> Session {
    let nz = noise_field(140, 100, 1);
    session_with(140, 100, depth, move |x, y| {
        let n = 0.04 * nz[(y * 140 + x) as usize];
        if x < 60 { [0.8 + n, 0.5 + n, 0.3] } else { [0.2, 0.35 + n, 0.6 + n] }.map(|v| v.clamp(0.0, 1.0))
    })
}

/// Textured disc (centre (100, 80), radius 45) on a noisy background.
fn disc(depth: u32) -> (Session, impl Fn(i32, i32) -> bool) {
    let nz = noise_field(200, 160, 2);
    let inside = |x: i32, y: i32| {
        let (dx, dy) = (x as f32 + 0.5 - 100.0, y as f32 + 0.5 - 80.0);
        dx * dx + dy * dy <= 45.0 * 45.0
    };
    let s = session_with(200, 160, depth, move |x, y| {
        let n = 0.05 * nz[(y * 200 + x) as usize];
        if inside(x as i32, y as i32) {
            let t = ((x + y) as f32 * 0.35).sin() * 0.5 + 0.5;
            [0.85 + 0.05 * t + n, 0.2 + 0.3 * t + n, 0.15 + n]
        } else {
            [0.25 + n, 0.45 + n, 0.7 + n]
        }
        .map(|v| v.clamp(0.0, 1.0))
    });
    (s, inside)
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn sel_iou(s: &Session, w: i32, h: i32, truth: impl Fn(i32, i32) -> bool) -> f32 {
    let (mut i, mut u) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (cov(s, x, y) >= 0.5, truth(x, y));
            i += (a && b) as i32;
            u += (a || b) as i32;
        }
    }
    i as f32 / u.max(1) as f32
}

#[test]
fn quick_selection_add_subtract_undo() {
    for depth in [8, 16] {
        let mut s = two_regions(depth);
        let r = s.execute("select.quick", json!({"points": [[20, 50], [35, 50]], "size": 8})).unwrap();
        assert_eq!(r["selected"], true);
        for y in (0..100).step_by(7) {
            for x in (0..140).step_by(3) {
                let c = cov(&s, x, y);
                if x < 57 {
                    assert!(c >= 0.5, "depth {depth}: ({x},{y}) not selected");
                } else if x > 62 {
                    assert!(c < 0.5, "depth {depth}: leaked to ({x},{y})");
                }
            }
        }
        // A second stroke on the right adds it.
        s.execute("select.quick", json!({"points": [[100, 50]], "size": 8})).unwrap();
        assert!(cov(&s, 130, 90) >= 0.5 && cov(&s, 10, 10) >= 0.5);
        // Subtract the left region again.
        s.execute("select.quick", json!({"points": [[20, 50]], "size": 8, "mode": "subtract"})).unwrap();
        assert!(cov(&s, 10, 10) < 0.5 && cov(&s, 130, 90) >= 0.5);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(cov(&s, 10, 10) >= 0.5);
        assert!(s.execute("select.quick", json!({"points": []})).is_err());
    }
}

#[test]
fn quick_selection_subtract_from_select_all_and_enhance() {
    let mut s = two_regions(8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.quick", json!({"points": [[100, 30], [110, 70]], "size": 10, "mode": "subtract"})).unwrap();
    assert!(cov(&s, 10, 50) >= 0.99);
    assert!(cov(&s, 120, 20) < 0.01 && cov(&s, 70, 90) < 0.01);
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.quick", json!({"points": [[20, 50]], "size": 8, "enhanceEdge": true})).unwrap();
    assert!(cov(&s, 10, 10) >= 0.5 && cov(&s, 100, 50) < 0.5);
}

#[test]
fn empty_result_keeps_the_selection_in_every_mode() {
    // Finding nothing must never wipe the selection, whatever the mode: Select Subject
    // and Object Selection run heuristics that often miss on busy photos, and a missed
    // Quick Selection dab must not destroy the work either.
    for m in ["add", "subtract", "replace"] {
        let mut s = two_regions(8);
        s.execute("select.all", json!({})).unwrap();
        let r = s.execute("select.quick", json!({"points": [[-5000, -5000]], "size": 8, "mode": m})).unwrap();
        assert_eq!(r["changed"], false, "mode {m}");
        assert_eq!(r["selected"], true, "mode {m}");
        assert!(cov(&s, 10, 50) >= 0.99, "mode {m}: Select All survived an empty result");
    }
    // Select Subject on a flat field finds nothing and keeps the selection too.
    let mut s = session_with(140, 100, 8, |_, _| [0.5, 0.5, 0.5]);
    s.execute("select.all", json!({})).unwrap();
    let r = s.execute("select.subject", json!({})).unwrap();
    assert_eq!(r["changed"], false);
    assert!(cov(&s, 10, 10) >= 0.99);
}

#[test]
fn object_selection_point_selects_the_object_under_it() {
    for depth in [8, 16, 32] {
        let (mut s, inside) = disc(depth);
        let r = s.execute("select.object", json!({"point": [100, 80]})).unwrap();
        assert_eq!(r["changed"], true, "depth {depth}");
        let v = sel_iou(&s, 200, 160, &inside);
        assert!(v > 0.9, "depth {depth}: IoU {v}");
    }
    // The background is no object: nothing changes.
    let (mut s, _) = disc(8);
    let r = s.execute("select.object", json!({"point": [3, 3]})).unwrap();
    assert_eq!(r["changed"], false);
    assert!(s.active().unwrap().doc.selection.is_none());
    assert!(s.execute("select.object", json!({"point": ["a", 1]})).is_err());
}

#[test]
fn object_selection_finds_a_large_foreground_at_each_depth_and_undoes() {
    for depth in [8, 16, 32] {
        let inside = |x: i32, y: i32| (10..190).contains(&x) && (8..152).contains(&y);
        let mut s = session_with(200, 160, depth, |x, y| if inside(x as i32, y as i32) { [0.8, 0.2, 0.1] } else { [0.1, 0.2, 0.8] });
        let r = s.execute("select.object", json!({"point": [100, 80]})).unwrap();
        assert_eq!(r["changed"], true, "depth {depth}");
        assert!(sel_iou(&s, 200, 160, inside) > 0.95, "depth {depth}");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        let r = s.execute("select.object", json!({"point": [1, 1]})).unwrap();
        assert_eq!(r["changed"], false, "background at depth {depth}");
        assert!(s.active().unwrap().doc.selection.is_none());
    }
}

#[test]
fn object_selection_point_honours_the_modes() {
    let (mut s, _) = disc(8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.object", json!({"point": [100, 80], "mode": "subtract"})).unwrap();
    assert!(cov(&s, 100, 80) < 0.5 && cov(&s, 3, 3) >= 0.5);
    s.execute("select.object", json!({"point": [100, 80], "mode": "add"})).unwrap();
    assert!(cov(&s, 100, 80) >= 0.5 && cov(&s, 3, 3) >= 0.5);
    s.execute("select.object", json!({"point": [100, 80], "mode": "intersect"})).unwrap();
    assert!(cov(&s, 100, 80) >= 0.5 && cov(&s, 3, 3) < 0.5);
}

#[test]
fn object_finder_cache_serves_the_whole_object_until_pixels_change() {
    let (mut s, _) = disc(8);
    let d = s.active().unwrap();
    let doc = std::sync::Arc::clone(&d.doc);
    let target = sampled_target(&doc, d.active_layer, false);
    assert!(s.object_finder.lookup(&doc, target, 100.0, 80.0).is_none());
    let r = std::sync::Arc::new(find_object(&doc, d.active_layer, false, 100.0, 80.0).unwrap());
    s.object_finder.insert(&doc, target, std::sync::Arc::clone(&r));
    // Any point on the object hits; the background and another target miss.
    assert!(std::sync::Arc::ptr_eq(&s.object_finder.lookup(&doc, target, 120.0, 90.0).unwrap(), &r));
    assert!(s.object_finder.lookup(&doc, target, 3.0, 3.0).is_none());
    assert!(s.object_finder.lookup(&doc, Some(LayerId(u64::MAX)), 100.0, 80.0).is_none());
    // The click reuses it, and a selection-only edit keeps it.
    s.execute("select.object", json!({"point": [110, 70]})).unwrap();
    assert_eq!(s.object_finder.len(), 1);
    s.execute("select.deselect", json!({})).unwrap();
    let d = s.active().unwrap();
    assert!(s.object_finder.lookup(&std::sync::Arc::clone(&d.doc), target, 100.0, 80.0).is_some());
    // A pixel edit drops it.
    s.execute("paint.pencil", json!({"points": [[5, 5], [30, 5]], "size": 4, "color": "#000000"})).unwrap();
    assert!(s.object_finder.is_empty());
}

#[test]
fn object_selection_disc() {
    for depth in [8, 16, 32] {
        let (mut s, inside) = disc(depth);
        s.execute("select.object", json!({"rect": [40, 20, 120, 120]})).unwrap();
        let v = sel_iou(&s, 200, 160, &inside);
        assert!(v > 0.95, "depth {depth}: IoU {v}");
    }
    let (mut s, _) = disc(8);
    assert!(s.execute("select.object", json!({"rect": [1, 2]})).is_err());
    // Negative width/height (drag up-left) work too.
    s.execute("select.object", json!({"rect": [160, 140, -120, -120]})).unwrap();
    assert!(cov(&s, 100, 80) >= 0.5);
}

#[test]
fn object_selection_on_an_empty_active_layer_says_so() {
    // #5246: an empty layer above the photo used to find nothing without a word.
    let (mut s, inside) = disc(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    for p in [json!({"rect": [40, 20, 120, 120]}), json!({"point": [100, 80]})] {
        let e = s.execute("select.object", p.clone()).unwrap_err().to_string();
        assert!(e.contains("active layer is empty") && e.contains("sampleAllLayers"), "{p}: {e}");
        assert!(s.active().unwrap().doc.selection.is_none());
    }
    // Sampling all layers finds the disc below.
    s.execute("select.object", json!({"rect": [40, 20, 120, 120], "sampleAllLayers": true})).unwrap();
    assert!(sel_iou(&s, 200, 160, &inside) > 0.95);
    // A layer with pixels elsewhere: the rectangle holds none of them.
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[5, 5], [10, 5]], "size": 2, "color": "#000000"})).unwrap();
    let e = s.execute("select.object", json!({"rect": [60, 60, 100, 80]})).unwrap_err().to_string();
    assert!(e.contains("no pixels in that area"), "{e}");
}

#[test]
fn select_subject_centred_object() {
    let (mut s, inside) = disc(8);
    assert!(s.is_enabled("select.subject"));
    s.execute("select.subject", json!({})).unwrap();
    let v = sel_iou(&s, 200, 160, inside);
    assert!(v > 0.9, "IoU {v}");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
}

/// Select Subject looks at the selected layer only; with no layer selected it looks at the
/// whole image.
#[test]
fn select_subject_samples_the_selected_layer_else_the_composite() {
    let (mut s, inside) = disc(8);
    // A layer above holding only a small square in the corner: the disc is not on it.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[10, 10], [30, 10]], "size": 3, "color": "#000000"})).unwrap();
    s.execute("select.subject", json!({})).unwrap();
    assert!(cov(&s, 100, 80) < 0.5, "the disc on the layer below was selected");
    s.execute("select.deselect", json!({})).unwrap();
    // No layer selected: the whole image, so the disc is found.
    s.execute("select.deselectLayers", json!({})).unwrap();
    s.execute("select.subject", json!({})).unwrap();
    assert!(sel_iou(&s, 200, 160, &inside) > 0.9);
}

/// #2865: a repeat on unchanged pixels reuses the last mask (an identical selection, even
/// after Deselect); sampling something else or a pixel edit computes it again.
#[test]
fn select_subject_repeat_reuses_the_mask_until_pixels_change() {
    let (mut s, _) = disc(8);
    let sel = |s: &Session| {
        let mut v = Vec::new();
        s.active().unwrap().doc.selection.as_ref().unwrap().read_region_into(Rect::new(0, 0, 200, 160), &mut v);
        v
    };
    let cached = |s: &Session| s.subject_cache.region.clone().unwrap();
    s.execute("select.subject", json!({})).unwrap();
    let (first, mask) = (sel(&s), cached(&s));
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("select.subject", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&mask, &cached(&s)), "the repeat recomputed the mask");
    assert_eq!(sel(&s), first);
    // Another sampled target (the composite rather than the layer) is a miss.
    s.execute("select.subject", json!({"sampleAllLayers": true})).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&mask, &cached(&s)));
    // A pixel edit drops the entry; the next run recomputes it.
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        bg.write_region(Rect::new(0, 0, 1, 1), &vec![1.0; n]);
        Ok(())
    })
    .unwrap();
    assert!(s.subject_cache.region.is_none());
    s.execute("select.subject", json!({})).unwrap();
    assert_eq!(sel(&s), first);
}

/// A vertical Gaussian-blurred edge at x = 40 (warm left, cool right) with the left half selected.
fn blurred_edge_selected() -> Session {
    blurred_edge_selected_at(8)
}

fn blurred_edge_selected_at(depth: u32) -> Session {
    let mut s = session_with(80, 40, depth, |x, _| {
        let t = 1.0 / (1.0 + ((x as f32 + 0.5 - 40.0) / 1.8).exp());
        [0.9 * t + 0.1 * (1.0 - t), 0.8 * t + 0.2 * (1.0 - t), 0.2 * t + 0.6 * (1.0 - t)]
    });
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    s
}

fn soft_count(s: &photocraft_raster::Surface, y: i32) -> usize {
    (0..80)
        .filter(|x| {
            let v = s.sample_channel(*x, y, 0);
            v > 0.05 && v < 0.95
        })
        .count()
}

#[test]
fn refine_edge_outputs_and_undo() {
    // Selection output: the hard edge becomes a soft matte following the blur.
    let mut s = blurred_edge_selected();
    assert!(s.is_enabled("select.refineEdge"));
    s.execute("select.refineEdge", json!({"radius": 8})).unwrap();
    let sel = s.active().unwrap().doc.selection.clone().unwrap();
    assert!(soft_count(&sel, 20) >= 4, "selection edge not soft");
    assert!(sel.sample_channel(5, 20, 0) > 0.99 && sel.sample_channel(75, 20, 0) < 0.01);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(soft_count(s.active().unwrap().doc.selection.as_ref().unwrap(), 20), 0);

    // Layer mask output.
    let layers_before = s.active().unwrap().doc.layer_count();
    let r = s.execute("select.refineEdge", json!({"radius": 8, "output": "layerMask"})).unwrap();
    let d = s.active().unwrap();
    let l = d.doc.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
    let m = l.mask.as_ref().unwrap();
    assert!(soft_count(&m.surface, 20) >= 4);
    assert!(m.value(5, 20) > 0.99 && m.value(75, 20) < 0.01);
    assert!(d.doc.selection.is_none());
    assert_eq!(d.doc.layer_count(), layers_before);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layers[0].mask.is_none());
    assert!(s.active().unwrap().doc.selection.is_some());

    // Existing mask settings (density, feather, linked) are preserved
    s.edit("set mask", |doc, _| {
        doc.layers[0].mask = Some(photocraft_doc::LayerMask {
            surface: photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8),
            density: 0.75,
            feather: 3.5,
            linked: false,
            enabled: true,
        });
        Ok(())
    })
    .unwrap();
    let r = s.execute("select.refineEdge", json!({"radius": 8, "output": "layerMask"})).unwrap();
    let l = s.active().unwrap().doc.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
    let m = l.mask.as_ref().unwrap();
    assert_eq!(m.density, 0.75);
    assert_eq!(m.feather, 3.5);
    assert!(!m.linked);
    s.execute("edit.undo", json!({})).unwrap();
    s.edit("clear mask", |doc, _| {
        doc.layers[0].mask = None;
        Ok(())
    })
    .unwrap();

    // New layer: pixels with the refined alpha; the source is hidden.
    let r = s.execute("select.refineEdge", json!({"radius": 8, "feather": 1, "output": "newLayer"})).unwrap();
    let d = s.active().unwrap();
    let nid = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(d.active_layer, Some(nid));
    assert_eq!(d.doc.layer_count(), layers_before + 1);
    let nl = d.doc.layer(nid).unwrap();
    let surf = nl.surface().unwrap();
    assert!(surf.format().alpha);
    let a = |x: i32| surf.pixel(x, 20)[3];
    assert!(a(5) > 0.99 && a(75) == 0.0 && a(40) > 0.05 && a(40) < 0.95);
    assert!(!d.doc.layers[0].visible);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer_count(), layers_before);
    assert!(s.active().unwrap().doc.layers[0].visible);

    // New layer with mask + decontamination (selection output upgrades to it).
    let r = s.execute("select.refineEdge", json!({"radius": 8, "decontaminate": true, "amount": 100})).unwrap();
    assert_eq!(r["output"], "newLayerWithMask");
    let d = s.active().unwrap();
    let nl = d.doc.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert!(nl.mask.is_some());
    let px = nl.surface().unwrap().pixel(41, 20);
    let orig = d.doc.layers[0].surface().unwrap().pixel(41, 20);
    // The fringe colour moved towards the warm foreground (more red, less blue).
    assert!(px[0] > orig[0] + 0.05 && px[2] < orig[2] - 0.05, "{px:?} vs {orig:?}");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.execute("select.refineEdge", json!({"output": "bogus"})).is_err());
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled("select.refineEdge"));
}

/// Output To New Document (with or without a layer mask) opens a document like the source,
/// at every depth, and leaves the source document as it was.
#[test]
fn refine_edge_outputs_to_a_new_document() {
    for depth in [8, 16, 32] {
        let mut s = blurred_edge_selected_at(depth);
        s.edit("profile", |doc, _| {
            doc.resolution_dpi = 300.0;
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let r = s.execute("select.refineEdge", json!({"radius": 8, "output": "newDocument"})).unwrap();
        assert_eq!((r["output"].as_str(), r["document"].as_u64()), (Some("newDocument"), Some(1)), "depth {depth}");
        assert_eq!(s.active_index(), Some(1));
        let nd = &s.active().unwrap().doc;
        assert_eq!((nd.size, nd.mode, nd.depth, nd.resolution_dpi), (before.size, before.mode, before.depth, 300.0));
        assert_eq!(nd.layers.len(), 1);
        assert!(nd.selection.is_none() && nd.layers[0].mask.is_none());
        let surf = nd.layers[0].surface().unwrap();
        let a = |x: i32| surf.pixel(x, 20)[3];
        assert!(a(5) > 0.99 && a(75) == 0.0 && a(40) > 0.05 && a(40) < 0.95, "depth {depth}: {} {} {}", a(5), a(40), a(75));
        // The source is untouched: same layers, still visible, selection kept, no history step.
        let src = &s.documents()[0];
        assert_eq!(src.doc.layers, before.layers);
        assert_eq!(src.doc.selection, before.selection);
        assert_eq!(src.history.undo_label(), Some("profile"));

        // With a layer mask: full pixels, the refined matte as the mask.
        s.execute("file.close", json!({"document": 1})).unwrap();
        let r = s.execute("select.refineEdge", json!({"radius": 8, "output": "newDocumentWithMask"})).unwrap();
        assert_eq!(r["output"], "newDocumentWithMask");
        let nd = &s.active().unwrap().doc;
        let l = &nd.layers[0];
        assert_eq!(l.surface().unwrap().pixel(75, 20), before.layers[0].surface().unwrap().pixel(75, 20), "depth {depth}");
        let m = l.mask.as_ref().unwrap();
        assert!(soft_count(&m.surface, 20) >= 4, "depth {depth}");
        assert!(m.value(5, 20) > 0.99 && m.value(75, 20) < 0.01, "depth {depth}");
    }
}

#[test]
fn refine_edge_shift_and_contrast() {
    let mut s = blurred_edge_selected();
    // A hard selection moves 1 px at 100%: no band of several pixels.
    s.execute("select.refineEdge", json!({"shiftEdge": 100})).unwrap();
    assert!(cov(&s, 40, 20) > 0.99 && cov(&s, 41, 20) < 0.01);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("select.refineEdge", json!({"feather": 8, "contrast": 100})).unwrap();
    assert!(cov(&s, 38, 20) == 1.0 && cov(&s, 42, 20) == 0.0);
}

/// The Refine Edge Brush adds strands beyond the radius band, at every depth, and rejects
/// malformed strokes.
#[test]
fn refine_edge_brush_strokes() {
    for depth in [8, 16, 32] {
        // Warm subject (x < 40) with warm strands every 4th column from x = 60, on blue.
        let strand = |x: u32| x < 40 || ((60..84).contains(&x) && x.is_multiple_of(4));
        let mut s = session_with(120, 40, depth, |x, _| if strand(x) { [0.9, 0.7, 0.2] } else { [0.1, 0.3, 0.8] });
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
        let stroke = json!([{"points": [[70, 5], [70, 35]], "size": 30}]);
        s.execute("select.refineEdge", json!({"radius": 4, "refineStrokes": stroke})).unwrap();
        assert!(cov(&s, 64, 20) > 0.9 && cov(&s, 72, 20) > 0.9, "depth {depth}: strands {}", cov(&s, 64, 20));
        assert!(cov(&s, 66, 20) < 0.1 && cov(&s, 100, 20) < 0.01, "depth {depth}: background");
        s.execute("edit.undo", json!({})).unwrap();
        // Subtract over the same place paints the refinement out again.
        let both = json!([{"points": [[70, 5], [70, 35]], "size": 30}, {"points": [[70, 5], [70, 35]], "size": 40, "mode": "subtract"}]);
        s.execute("select.refineEdge", json!({"radius": 4, "refineStrokes": both})).unwrap();
        assert!(cov(&s, 64, 20) < 0.01, "depth {depth}");
        s.execute("edit.undo", json!({})).unwrap();
        for bad in [
            json!(7),
            json!([5]),
            json!([{"points": []}]),
            json!([{"points": [[1, "x"]]}]),
            json!([{"points": [[1, 2, 3]]}]),
            json!([{"points": [[1, 2]], "size": 0}]),
            json!([{"points": [[1, 2]], "size": 1e9}]),
            json!([{"points": [[1, 2]], "mode": "xor"}]),
            json!([{"points": [[0, 0], [1e6, 1e6], [0, 0], [1e6, 1e6]], "size": 5000}]),
        ] {
            assert!(s.execute("select.refineEdge", json!({"refineStrokes": bad})).is_err(), "{bad}");
        }
    }
}

#[test]
fn refine_edge_works_on_layer_mask_without_selection() {
    let mut s = blurred_edge_selected();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none());
    assert!(s.active().unwrap().doc.layers[0].mask.is_some());
    assert!(s.is_enabled("select.refineEdge"));

    let r = s.execute("select.refineEdge", json!({"radius": 8})).unwrap();
    assert_eq!(r["output"], "layerMask");
    let d = s.active().unwrap();
    let m = d.doc.layers[0].mask.as_ref().unwrap();
    assert!(soft_count(&m.surface, 20) >= 4);
}

#[test]
fn focus_area_command() {
    let nz = noise_field(160, 100, 3);
    let mut blurred = vec![0.0f32; nz.len()];
    for y in 0..100i32 {
        for x in 0..160i32 {
            let mut a = 0.0;
            for dy in -4..=4 {
                for dx in -4..=4 {
                    a += nz[((y + dy).clamp(0, 99) * 160 + (x + dx).clamp(0, 159)) as usize];
                }
            }
            blurred[(y * 160 + x) as usize] = a / 81.0;
        }
    }
    let mut s = session_with(160, 100, 8, move |x, y| {
        let i = (y * 160 + x) as usize;
        let v = 0.5 + 0.15 * if x < 80 { nz[i] } else { blurred[i] * 3.0 };
        [v, v, v].map(|c| c.clamp(0.0, 1.0))
    });
    s.execute("select.focusArea", json!({})).unwrap();
    assert!(cov(&s, 30, 50) >= 0.5 && cov(&s, 130, 50) < 0.5);
}
/// The selection as 8-bit coverage over `w × h`, to compare two selections exactly.
fn sel_bytes(s: &Session, w: i32, h: i32) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (cov(s, x, y) * 255.0).round() as u8)).collect()
}

/// The #8783 review: a refine whose result would be dense over more than
/// [`matting::MAX_REFINE_AREA`] pixels is refused, not allocated: a thin Brush edit across a
/// huge canvas (its edited selection spans the canvas) and a selection of the whole canvas.
/// The selection stays as it was.
#[test]
fn refine_edge_refuses_a_selection_too_large_to_refine() {
    let mut s = Session::new();
    s.add_document(Document::new("huge", photocraft_doc::Size::new(100_000, 100_000), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8), None);
    s.execute("select.rect", json!({"x": 10, "y": 60, "width": 10, "height": 10})).unwrap();
    let small = s.active().and_then(|d| d.doc.selection.clone()).expect("selected");
    let diagonal = json!({"edits": [{"tool": "brush", "points": [[0, 0], [99_999, 99_999]], "size": 1}]});
    let t = std::time::Instant::now();
    let e = s.execute("select.refineEdge", diagonal).expect_err("too large to refine");
    assert!(e.to_string().contains("too large to refine"), "{e}");
    assert!(s.active().and_then(|d| d.doc.selection.as_ref()) == Some(&small), "the selection is unchanged");
    // The small selection itself still refines.
    s.execute("select.refineEdge", json!({"radius": 4})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let all = s.active().and_then(|d| d.doc.selection.clone()).expect("selected");
    assert!(s.execute("select.refineEdge", json!({"radius": 4})).is_err());
    assert!(s.active().and_then(|d| d.doc.selection.as_ref()) == Some(&all));
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
}

/// The #8783 review: Brush and Lasso edits and Refine Edge Brush strokes that would paint
/// too many tiles of a huge canvas are refused before anything is painted, and the selection
/// stays as it was.
#[test]
fn refine_edge_refuses_edits_that_cross_too_many_tiles() {
    let mut s = Session::new();
    s.add_document(Document::new("huge", photocraft_doc::Size::new(100_000, 100_000), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8), None);
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let before = s.active().and_then(|d| d.doc.selection.clone()).expect("selected");
    let zigzag: Vec<[f32; 2]> = (0..20).map(|i| [if i % 2 == 0 { 0.0 } else { 99_999.0 }, i as f32 * 5000.0]).collect();
    for p in [
        json!({"edits": [{"tool": "brush", "points": zigzag, "size": 1}]}),
        json!({"edits": [{"tool": "lasso", "points": zigzag}]}),
        json!({"refineStrokes": [{"points": zigzag, "size": 1}]}),
    ] {
        let e = s.execute("select.refineEdge", p.clone()).expect_err("refused");
        assert!(e.to_string().contains("too many tiles"), "{p}: {e}");
        assert!(s.active().and_then(|d| d.doc.selection.as_ref()) == Some(&before), "{p}: the selection is unchanged");
    }
}

/// Select and Mask's own tools (#7069, #8148): each edit changes the selection before the
/// edge is refined, at every depth, and the whole run is one history step.
#[test]
fn refine_edge_edits_apply_each_tool_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = two_regions(depth);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        let start = sel_bytes(&s, 140, 100);
        let run = |s: &mut Session, edits: Value| {
            s.execute("select.refineEdge", json!({"edits": edits})).unwrap();
            let after = sel_bytes(s, 140, 100);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(sel_bytes(s, 140, 100), start, "depth {depth}: one undo restores the selection");
            after
        };
        let at = |b: &[u8], x: usize, y: usize| b[y * 140 + x];
        // Quick Selection grows across the cool region; subtracting it again leaves the start.
        let quick = run(&mut s, json!([{"tool": "quick", "points": [[100, 50], [110, 50]], "size": 8}]));
        assert!(at(&quick, 120, 80) >= 128 && at(&quick, 5, 5) >= 128 && at(&quick, 40, 60) < 128, "depth {depth}: quick");
        let back =
            run(&mut s, json!([{"tool": "quick", "points": [[100, 50]], "size": 8}, {"tool": "quick", "points": [[100, 50]], "size": 8, "mode": "subtract"}]));
        assert!(at(&back, 120, 80) < 128 && at(&back, 5, 5) >= 128, "depth {depth}: quick subtract");
        // The Brush paints exactly where it goes.
        let brush = run(&mut s, json!([{"tool": "brush", "points": [[30, 80], [60, 80]], "size": 10}]));
        assert!(at(&brush, 45, 80) == 255 && at(&brush, 45, 90) == 0, "depth {depth}: brush");
        let erased = run(&mut s, json!([{"tool": "brush", "points": [[10, 10]], "size": 6, "mode": "subtract"}]));
        assert!(at(&erased, 10, 10) == 0 && at(&erased, 2, 2) == 255, "depth {depth}: brush subtract");
        // Lasso adds and subtracts polygons.
        let lasso = run(&mut s, json!([{"tool": "lasso", "points": [[50, 50], [90, 50], [90, 90], [50, 90]]}]));
        assert!(at(&lasso, 70, 70) == 255 && at(&lasso, 95, 70) == 0 && at(&lasso, 5, 5) == 255, "depth {depth}: lasso");
        let cut = run(&mut s, json!([{"tool": "lasso", "points": [[0, 0], [10, 0], [10, 10], [0, 10]], "mode": "subtract"}]));
        assert!(at(&cut, 5, 5) == 0 && at(&cut, 15, 15) == 255, "depth {depth}: lasso subtract");
        // Clear Selection, then Invert of nothing: everything.
        let all = run(&mut s, json!([{"tool": "clear"}, {"tool": "invert"}]));
        assert!(all.iter().all(|v| *v == 255), "depth {depth}: clear + invert");
        let inverted = run(&mut s, json!([{"tool": "invert"}]));
        assert!(at(&inverted, 5, 5) == 0 && at(&inverted, 100, 50) == 255, "depth {depth}: invert");
    }
}

/// Object Selection and Select Subject inside the workspace find what the standalone tools find.
#[test]
fn refine_edge_edits_object_and_subject() {
    for depth in [8, 16, 32] {
        let (mut s, inside) = disc(depth);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("select.refineEdge", json!({"edits": [{"tool": "object", "rect": [40, 20, 120, 120]}, {"tool": "lasso", "points": [[0, 0], [10, 0], [10, 10], [0, 10]], "mode": "subtract"}]})).unwrap();
        let v = sel_iou(&s, 200, 160, &inside);
        assert!(v > 0.95, "depth {depth}: object rect IoU {v}");
        s.execute("select.refineEdge", json!({"edits": [{"tool": "clear"}, {"tool": "object", "point": [3, 3]}, {"tool": "object", "point": [100, 80]}]}))
            .unwrap();
        let v = sel_iou(&s, 200, 160, &inside);
        assert!(v > 0.9, "depth {depth}: object point IoU {v} (the background point finds nothing)");
    }
    let (mut s, inside) = disc(8);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.refineEdge", json!({"edits": [{"tool": "subject"}]})).unwrap();
    let v = sel_iou(&s, 200, 160, &inside);
    assert!(v > 0.9, "subject replaces the selection: IoU {v}");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.refineEdge", json!({"edits": [{"tool": "subject", "mode": "add"}]})).unwrap();
    assert!(cov(&s, 5, 5) >= 0.5 && cov(&s, 100, 80) >= 0.5, "subject added to the selection");
}

/// An edit gives what the standalone tool gives, then the edge is refined as usual, and the
/// result reaches every output.
#[test]
fn refine_edge_edits_match_the_tools_and_reach_the_outputs() {
    let mut tool = two_regions(8);
    tool.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    tool.execute("select.quick", json!({"points": [[100, 50], [110, 50]], "size": 8})).unwrap();
    let mut ws = two_regions(8);
    ws.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    ws.execute("select.refineEdge", json!({"edits": [{"tool": "quick", "points": [[100, 50], [110, 50]], "size": 8}]})).unwrap();
    assert_eq!(sel_bytes(&ws, 140, 100), sel_bytes(&tool, 140, 100));
    // Refinement runs on the edited selection: Shift Edge contracts the brushed bar too.
    let mut s = two_regions(8);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("select.refineEdge", json!({"shiftEdge": -100, "edits": [{"tool": "brush", "points": [[30, 80], [100, 80]], "size": 20}]})).unwrap();
    assert!(cov(&s, 60, 80) >= 0.5 && cov(&s, 60, 89) < 0.5 && cov(&s, 60, 70) < 0.5, "the hard 20 px bar shrank by one pixel at -100% Shift Edge");
    // Layer mask and new-layer outputs take the edited selection.
    let mut s = blurred_edge_selected();
    let r = s.execute("select.refineEdge", json!({"output": "layerMask", "edits": [{"tool": "invert"}]})).unwrap();
    assert_eq!(r["output"], "layerMask");
    let m = s.active().unwrap().doc.layers[0].mask.as_ref().unwrap();
    assert!(m.surface.sample_channel(10, 20, 0) < 0.01 && m.surface.sample_channel(70, 20, 0) > 0.99);
    let mut s = blurred_edge_selected();
    let r = s.execute("select.refineEdge", json!({"output": "newLayerWithMask", "edits": [{"tool": "invert"}]})).unwrap();
    assert_eq!(r["output"], "newLayerWithMask");
    let d = s.active().unwrap();
    let m = d.doc.layers.iter().find_map(|l| l.mask.as_ref()).unwrap();
    assert!(m.surface.sample_channel(10, 20, 0) < 0.01 && m.surface.sample_channel(70, 20, 0) > 0.99);
    // Clearing everything deselects.
    let mut s = blurred_edge_selected();
    let r = s.execute("select.refineEdge", json!({"edits": [{"tool": "clear"}]})).unwrap();
    assert_eq!(r["selected"], false);
    assert!(s.active().unwrap().doc.selection.is_none());
}

/// Bad edits are refused before anything changes.
#[test]
fn refine_edge_rejects_bad_edits_and_keeps_the_selection() {
    let mut s = blurred_edge_selected();
    let before = sel_bytes(&s, 80, 40);
    for bad in [json!(7), json!([{"tool": "nope"}]), json!([{"tool": "lasso", "points": [[0, 0]]}]), json!([{"tool": "brush", "points": [[0, 0]], "size": -1}])]
    {
        assert!(s.execute("select.refineEdge", json!({"edits": bad})).is_err(), "{bad}");
        assert_eq!(sel_bytes(&s, 80, 40), before, "{bad}");
    }
}
