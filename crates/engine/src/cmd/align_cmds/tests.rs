use super::*;

/// Deterministic texture written into a layer at offset (dx, dy).
fn texture_layer(s: &mut Session, name: &str, dx: i32, dy: i32) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    paint_texture(s, id, dx, dy);
    id
}

/// Writes the deterministic texture into layer `id` at offset (dx, dy).
fn paint_texture(s: &mut Session, id: LayerId, dx: i32, dy: i32) {
    s.edit("tex", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let mut seed = 99u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        surf.fill_rect(Rect::new(dx, dy, dx + 240, dy + 180), &[0.4, 0.4, 0.4, 1.0]);
        for _ in 0..120 {
            let (x, y) = ((rnd() * 220.0) as i32, (rnd() * 160.0) as i32);
            let (sw, sh) = (4 + (rnd() * 14.0) as i32, 4 + (rnd() * 10.0) as i32);
            let v = rnd() as f32;
            surf.fill_rect(Rect::new(dx + x, dy + y, dx + x + sw, dy + y + sh), &[v, 1.0 - v, v * 0.5, 1.0]);
        }
        Ok(())
    })
    .unwrap();
}

fn select(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
}

#[test]
fn auto_align_repositions_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 240, "depth": depth, "background": "transparent"})).unwrap();
        let a = texture_layer(&mut s, "A", 20, 20);
        let b = texture_layer(&mut s, "B", 31, 13);
        assert!(!s.is_enabled("edit.autoAlignLayers"), "one layer selected");
        select(&mut s, &[a, b]);
        assert!(s.is_enabled("edit.autoAlignLayers"));
        let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
        assert_eq!(r["reference"], a.0);
        let bb = s.active().unwrap().doc.layer(b).unwrap().surface().unwrap().content_bounds();
        assert!((bb.x0 - 20).abs() <= 1 && (bb.y0 - 20).abs() <= 1, "depth {depth}: {bb:?} {r}");
        s.undo();
        let bb = s.active().unwrap().doc.layer(b).unwrap().surface().unwrap().content_bounds();
        assert_eq!((bb.x0, bb.y0), (31, 13));
    }
}

#[test]
fn auto_align_auto_projection_and_errors() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
    let a = texture_layer(&mut s, "A", 20, 20);
    let b = texture_layer(&mut s, "B", 26, 25);
    select(&mut s, &[a, b]);
    let r = s.execute("edit.autoAlignLayers", json!({})).unwrap();
    let m = r["aligned"][0]["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    assert!((m[2] + 6.0).abs() < 1.0 && (m[5] + 5.0).abs() < 1.0, "{r}");
    assert!(s.execute("edit.autoAlignLayers", json!({"projection": "sideways"})).is_err());
    assert!(s.execute("edit.autoAlignLayers", json!({"reference": u64::MAX})).is_err());
}

/// As in Photoshop, a locked layer stays where it is: it becomes the reference and the
/// others move onto it (#7080).
#[test]
fn auto_align_keeps_a_locked_layer_as_the_reference() {
    for lock in ["all", "position", "pixels"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
        let a = texture_layer(&mut s, "A", 20, 20);
        let b = texture_layer(&mut s, "B", 31, 13);
        s.edit("lock", |doc, _| {
            doc.layer_mut(b).unwrap().locks.set(lock, true);
            Ok(())
        })
        .unwrap();
        select(&mut s, &[a, b]);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
        assert_eq!(r["reference"], b.0, "{lock}: {r}");
        let doc = &s.active().unwrap().doc;
        let bb = doc.layer(b).unwrap().surface().unwrap().content_bounds();
        assert_eq!((bb.x0, bb.y0), (31, 13), "{lock}: the locked layer stays put");
        let ab = doc.layer(a).unwrap().surface().unwrap().content_bounds();
        assert!((ab.x0 - 31).abs() <= 1 && (ab.y0 - 13).abs() <= 1, "{lock}: {ab:?} {r}");
        // An explicit reference that would move the locked layer is refused.
        s.undo();
        assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": a.0})).is_err());
        // Two locked layers: nothing may move.
        s.edit("lock", |doc, _| {
            doc.layer_mut(a).unwrap().locks.set(lock, true);
            Ok(())
        })
        .unwrap();
        assert!(s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).is_err());
        let bb = s.active().unwrap().doc.layer(a).unwrap().surface().unwrap().content_bounds();
        assert_eq!((bb.x0, bb.y0), (20, 20));
    }
}

/// The Background doesn't block Auto-Align: when a projection reprojects every layer
/// (Cylindrical, Spherical, or Auto choosing one), or another layer is the reference, it
/// becomes a normal layer ("Layer 0") and moves, as in Photoshop and as Free Transform does.
#[test]
fn auto_align_moves_the_background_as_a_normal_layer() {
    let setup = || {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 240, "background": "white"})).unwrap();
        let bg = s.active().unwrap().doc.background().unwrap().id;
        paint_texture(&mut s, bg, 20, 20);
        let b = texture_layer(&mut s, "B", 31, 13);
        select(&mut s, &[bg, b]);
        (s, bg, b)
    };
    for projection in ["cylindrical", "spherical", "auto"] {
        let (mut s, bg, b) = setup();
        let r = s.execute("edit.autoAlignLayers", json!({"projection": projection})).unwrap_or_else(|e| panic!("{projection}: {e}"));
        let doc = &s.active().unwrap().doc;
        let moved = r["aligned"].as_array().unwrap().iter().any(|a| a["layer"] == bg.0);
        assert_eq!(moved, projection != "auto" || r["layout"] != "perspective", "{projection}: {r}");
        assert_eq!(doc.is_background(bg), !moved, "{projection}: a moved Background is a normal layer: {r}");
        if moved {
            assert_eq!(doc.layer(bg).unwrap().name, "Layer 0", "{projection}");
        }
        assert!(doc.layer(b).is_some(), "{projection}");
        s.undo();
        assert!(s.active().unwrap().doc.is_background(bg), "{projection}: undo restores the Background");
    }
    // Planar, with the other layer as the reference: the Background moves onto it.
    let (mut s, bg, b) = setup();
    s.execute("edit.autoAlignLayers", json!({"projection": "reposition", "reference": b.0})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.is_background(bg));
    // Default reference: the Background (bottom) stays put and stays the Background.
    let (mut s, bg, b) = setup();
    let r = s.execute("edit.autoAlignLayers", json!({"projection": "reposition"})).unwrap();
    assert_eq!(r["reference"], bg.0, "{r}");
    let doc = &s.active().unwrap().doc;
    assert!(doc.is_background(bg));
    let bb = doc.layer(b).unwrap().surface().unwrap().content_bounds();
    assert!((bb.x0 - 20).abs() <= 1 && (bb.y0 - 20).abs() <= 1, "{bb:?} {r}");
}

/// The moving layer's mask is warped as a mask: one grey channel whose outside keeps its
/// default (reveal-all stays revealed, hide-all stays hidden), for planar and mesh warps.
#[test]
fn auto_align_warps_layer_masks_keeping_their_default() {
    for (projection, reveal) in [("reposition", true), ("reposition", false), ("cylindrical", true)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
        let a = texture_layer(&mut s, "A", 20, 20);
        let b = texture_layer(&mut s, "B", 31, 13);
        let (inside, outside) = if reveal { (0.0, 1.0) } else { (1.0, 0.0) };
        s.edit("mask", |doc, _| {
            let mut m = if reveal { LayerMask::reveal_all() } else { LayerMask::hide_all() };
            m.surface.fill_rect(Rect::new(100, 100, 110, 110), &[inside]);
            doc.layer_mut(b).unwrap().mask = Some(m);
            Ok(())
        })
        .unwrap();
        select(&mut s, &[a, b]);
        let r = s.execute("edit.autoAlignLayers", json!({"projection": projection})).unwrap();
        let doc = &s.active().unwrap().doc;
        let m = doc.layer(b).unwrap().mask.as_ref().unwrap();
        assert_eq!(m.surface.format().channels(), 1, "{projection}: {r}");
        assert_eq!(m.value(250, 200), outside, "{projection} reveal={reveal}: {r}");
        assert_eq!(m.value(5, 5), outside, "{projection} reveal={reveal}: {r}");
        if projection == "reposition" {
            // The square moved with the layer (by about (-11, +7)).
            assert!((m.value(94, 112) - inside).abs() < 0.01, "{r}");
        }
    }
}

#[test]
fn auto_blend_panorama_masks_and_seamless_layer() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 60, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "left"})).unwrap();
        s.edit("l", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 130, 60), &[0.2, 0.3, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
        let left = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "right"})).unwrap();
        s.edit("r", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(70, 0, 200, 60), &[0.9, 0.6, 0.2, 1.0]);
            Ok(())
        })
        .unwrap();
        let right = s.active().unwrap().active_layer.unwrap();
        select(&mut s, &[left, right]);
        let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
        let doc = &s.active().unwrap().doc;
        let ml = doc.layer(left).unwrap().mask.as_ref().unwrap();
        let mr = doc.layer(right).unwrap().mask.as_ref().unwrap();
        assert_eq!((ml.value(10, 30), mr.value(10, 30)), (1.0, 0.0), "depth {depth}");
        assert_eq!((ml.value(190, 30), mr.value(190, 30)), (0.0, 1.0));
        let blended = LayerId(r["blended"].as_u64().unwrap());
        let px = doc.layer(blended).unwrap().surface().unwrap().pixel(5, 30);
        assert!((px[2] - 0.8).abs() < 0.02, "{px:?}");
        s.undo();
        assert!(s.active().unwrap().doc.layer(left).unwrap().mask.is_none());
    }
}

/// #5745: the Background can't take a mask, so Auto-Blend turns it into a normal layer first.
#[test]
fn auto_blend_turns_the_background_into_a_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 60, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.background().unwrap().id;
    s.execute("layer.new.layer", json!({"name": "right"})).unwrap();
    s.edit("r", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(70, 0, 200, 60), &[0.9, 0.6, 0.2, 1.0]);
        Ok(())
    })
    .unwrap();
    let right = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[bg, right]);
    s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.is_background(bg), "the Background became a layer to take its mask");
    assert!(doc.layer(bg).unwrap().mask.is_some());
    s.undo();
    assert!(s.active().unwrap().doc.is_background(bg), "undo restores the Background");
}

/// Seamless Tones and Colors evens out a 10% exposure step between two panorama layers.
#[test]
fn auto_blend_panorama_seamless_matches_exposure() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 60, "background": "transparent"})).unwrap();
    let mut ids = Vec::new();
    for (name, x0, x1, gain) in [("left", 0, 130, 1.0f32), ("right", 70, 200, 1.1)] {
        s.execute("layer.new.layer", json!({ "name": name })).unwrap();
        s.edit(name, |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x0, 0, x1, 60), &[0.2 * gain, 0.4 * gain, 0.5 * gain, 1.0]);
            Ok(())
        })
        .unwrap();
        ids.push(s.active().unwrap().active_layer.unwrap());
    }
    select(&mut s, &ids);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let blended = doc.layer(LayerId(r["blended"].as_u64().unwrap())).unwrap().surface().unwrap();
    let (a, b) = (blended.pixel(10, 30), blended.pixel(190, 30));
    // 0.5 vs 0.55 without gain compensation.
    assert!((a[2] - b[2]).abs() < 0.015, "{a:?} {b:?}");
}

/// #5745: the top layer hangs off the canvas's right edge, so cropped to the canvas it lies
/// wholly over the bottom one. It used to get an all-black mask; now it shows on its side.
#[test]
fn auto_blend_panorama_of_a_layer_the_canvas_crops_to_all_overlap() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 60, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "left"})).unwrap();
    s.edit("l", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 60), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let left = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "right"})).unwrap();
    s.edit("r", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(120, 0, 320, 60), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let right = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[left, right]);
    s.execute("edit.autoBlendLayers", json!({"method": "panorama", "seamlessTones": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    let ml = doc.layer(left).unwrap().mask.as_ref().unwrap();
    let mr = doc.layer(right).unwrap().mask.as_ref().unwrap();
    assert_eq!((ml.value(10, 30), mr.value(10, 30)), (1.0, 0.0));
    assert_eq!((ml.value(195, 30), mr.value(195, 30)), (0.0, 1.0));
    for x in 0..200 {
        assert_eq!(ml.value(x, 30) + mr.value(x, 30), 1.0, "x {x} shows one layer");
    }
}

/// #5745: a layer Auto-Align left partly off the canvas is masked and blended over all of
/// its bounds, not cut off at the canvas edge.
#[test]
fn auto_blend_panorama_masks_past_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 60, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "left"})).unwrap();
    s.edit("l", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 150, 60), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let left = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "right"})).unwrap();
    s.edit("r", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(120, 0, 320, 60), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let right = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[left, right]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let ml = doc.layer(left).unwrap().mask.as_ref().unwrap();
    let mr = doc.layer(right).unwrap().mask.as_ref().unwrap();
    assert_eq!((ml.value(10, 30), mr.value(10, 30)), (1.0, 0.0));
    assert_eq!((ml.value(300, 30), mr.value(300, 30)), (0.0, 1.0), "off the canvas");
    let blended = doc.layer(LayerId(r["blended"].as_u64().unwrap())).unwrap().surface().unwrap();
    assert_eq!(blended.pixel(300, 30)[3], 1.0, "the merged layer covers the off-canvas part");
}

/// A sparse layer whose bounds are huge (a dot at each far corner) raises its own content
/// estimate, so the extent past the canvas is capped independently: it keeps to the canvas.
#[test]
fn auto_blend_sparse_huge_bounds_keep_to_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "a"})).unwrap();
    s.edit("a", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 150, 200), &[0.2, 0.4, 0.5, 1.0]);
        surf.fill_rect(Rect::new(100_000, 100_000, 100_001, 100_001), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "b"})).unwrap();
    s.edit("b", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 0, 200, 200), &[0.2, 0.4, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let b = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[a, b]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "panorama"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let bb = doc.layer(LayerId(r["blended"].as_u64().unwrap())).unwrap().surface().unwrap().content_bounds();
    assert!(bb.x1 <= 200 && bb.y1 <= 200, "{bb:?}");
}

/// The budget past the canvas counts every layer's buffers, not just the work-area pixels.
#[test]
fn auto_blend_budget_counts_layers_and_channels() {
    // Two RGBA layers: 4 bytes × (2 layers × (4 channels + luma, alpha, weight) + merged + depth).
    assert_eq!(blend_bytes_per_pixel(2, 4), 88);
    assert!(blend_bytes_per_pixel(5, 4) > blend_bytes_per_pixel(2, 4));
    // A 2^27-pixel area would need about 11 GiB for just two RGBA layers: it keeps to the canvas.
    assert!((1u64 << 27).saturating_mul(blend_bytes_per_pixel(2, 4)) > MAX_BLEND_BYTES);
    assert_eq!(blend_bytes_per_pixel(u64::MAX, 4), u64::MAX, "saturates instead of overflowing");
}

#[test]
fn auto_blend_stack_without_seamless() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "sharp"})).unwrap();
    s.edit("checker", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in (0..64).step_by(2) {
            for x in (0..64).step_by(2) {
                let v = if (x / 2 + y / 2) % 2 == 0 { 0.1 } else { 0.9 };
                surf.fill_rect(Rect::new(x, y, x + 2, y + 2), &[v, v, v, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "flat"})).unwrap();
    s.edit("flat", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 64), &[0.5, 0.5, 0.5, 1.0]);
        Ok(())
    })
    .unwrap();
    let flat = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[a, flat]);
    let r = s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
    assert!(r["blended"].is_null());
    let doc = &s.active().unwrap().doc;
    // The detailed layer is sharper everywhere.
    let shown = (0..64).filter(|x| doc.layer(a).unwrap().mask.as_ref().unwrap().value(*x, 32) > 0.5).count();
    assert_eq!(shown, 64);
    assert_eq!(doc.layer(flat).unwrap().mask.as_ref().unwrap().value(10, 10), 0.0);
    assert!(s.execute("edit.autoBlendLayers", json!({"method": "sideways"})).is_err());
}

/// A smooth layer (soft blobs on a gradient) and the same picture a few pixels away: the
/// intensity registration finds the shift to a fraction of a pixel at every motion model.
fn smooth_layer(s: &mut Session, name: &str, dx: i32, dy: i32) -> LayerId {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.edit("smooth", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..150 {
            for x in 0..200 {
                let (u, v) = ((x - dx) as f32, (y - dy) as f32);
                let blob = (-((u - 90.0).powi(2) + (v - 70.0).powi(2)) / (2.0 * 25.0f32.powi(2))).exp();
                let blob2 = (-((u - 150.0).powi(2) + (v - 40.0).powi(2)) / (2.0 * 15.0f32.powi(2))).exp();
                let g = 0.2 + 0.5 * blob + 0.25 * blob2 + 0.001 * u;
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[g, g * 0.8, g * 0.6, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    s.active().unwrap().active_layer.unwrap()
}

#[test]
fn auto_align_registers_a_smooth_bracket_by_intensity() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 150, "depth": depth, "background": "transparent"})).unwrap();
        let a = smooth_layer(&mut s, "A", 0, 0);
        let b = smooth_layer(&mut s, "B", 4, 3);
        select(&mut s, &[a, b]);
        assert!(s.execute("edit.autoAlignLayers", json!({"registration": "sideways"})).is_err());
        assert!(s.execute("edit.autoAlignLayers", json!({"projection": "cylindrical", "registration": "intensity"})).is_err());
        for (params, model) in [
            (json!({"projection": "reposition", "registration": "intensity"}), "reposition"),
            (json!({"registration": "intensity"}), "collage"),
            (json!({"projection": "perspective", "registration": "intensity"}), "perspective"),
        ] {
            let r = s.execute("edit.autoAlignLayers", params).unwrap();
            assert_eq!(r["registration"], "intensity", "depth {depth}: {r}");
            assert_eq!(r["aligned"][0]["model"], model);
            let m = r["aligned"][0]["matrix"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
            assert!((m[2] + 4.0).abs() < 0.25 && (m[5] + 3.0).abs() < 0.25, "depth {depth} {model}: {r}");
            // B now shows A's picture.
            let doc = &s.active().unwrap().doc;
            let (pa, pb) = (doc.layer(a).unwrap().surface().unwrap().pixel(90, 70), doc.layer(b).unwrap().surface().unwrap().pixel(90, 70));
            assert!((pa[0] - pb[0]).abs() < 0.02, "depth {depth} {model}: {pa:?} vs {pb:?}");
            s.undo();
        }
    }
}

/// Layers with nothing in common are no fit: the intensity registration says so instead of
/// moving them by whatever pose its search box ends at.
#[test]
fn auto_align_rejects_unrelated_layers_instead_of_guessing_a_pose() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150, "depth": 8, "background": "transparent"})).unwrap();
    let a = smooth_layer(&mut s, "A", 0, 0);
    s.execute("layer.new.layer", json!({"name": "stripes"})).unwrap();
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..150 {
            for x in 0..200 {
                let g = if (x / 7 + y / 11) % 2 == 0 { 0.9 } else { 0.1 };
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[g, g, g, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    let b = s.active().unwrap().active_layer.unwrap();
    select(&mut s, &[a, b]);
    for registration in ["intensity", "auto"] {
        let err = s.execute("edit.autoAlignLayers", json!({"registration": registration})).unwrap_err().to_string();
        assert_eq!(err, NO_DETAIL, "{registration}");
    }
    // A layer that is mostly empty overlaps too little to be placed.
    let c = smooth_layer(&mut s, "C", 2, 1);
    s.edit("hole", |doc, _| {
        let surf = doc.layer_mut(c).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 200, 120), &[0.0, 0.0, 0.0, 0.0]);
        Ok(())
    })
    .unwrap();
    select(&mut s, &[a, c]);
    assert_eq!(s.execute("edit.autoAlignLayers", json!({"registration": "intensity"})).unwrap_err().to_string(), NO_DETAIL);
}

/// Two layers, each sharp on one half: the merged layer takes each half from the sharp layer.
#[test]
fn auto_blend_stack_fuses_the_sharp_halves() {
    let checker = |x: i32, y: i32| if (x / 3 + y / 3) % 2 == 0 { 0.2 } else { 0.8 };
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 96, "height": 64, "depth": depth, "background": "transparent"})).unwrap();
        let mut ids = Vec::new();
        for (name, sharp_left) in [("left", true), ("right", false)] {
            s.execute("layer.new.layer", json!({"name": name})).unwrap();
            s.edit(name, |doc, active| {
                let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for y in 0..64 {
                    for x in 0..96 {
                        // The sharp half is the checker, the other half its local mean (flat gray).
                        let v = if (x < 48) == sharp_left { checker(x, y) } else { 0.5 };
                        surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        select(&mut s, &ids);
        for halo in [0.0, 2.0] {
            let r = s.execute("edit.autoBlendLayers", json!({"method": "stack", "haloControl": halo})).unwrap();
            let doc = &s.active().unwrap().doc;
            let blended = doc.layer(LayerId(r["blended"].as_u64().unwrap())).unwrap().surface().unwrap();
            for (x, y) in [(10, 20), (20, 40), (76, 20), (85, 40)] {
                let px = blended.pixel(x, y);
                assert!((px[0] - checker(x, y)).abs() < 0.08, "depth {depth} halo {halo} ({x},{y}): {px:?}");
                assert!((px[3] - 1.0).abs() < 1e-3, "{px:?}");
            }
            // The masks still pick per pixel.
            assert_eq!(doc.layer(ids[0]).unwrap().mask.as_ref().unwrap().value(10, 20), 1.0);
            assert_eq!(doc.layer(ids[1]).unwrap().mask.as_ref().unwrap().value(85, 40), 1.0);
            s.undo();
            assert!(s.active().unwrap().doc.layer(ids[0]).unwrap().mask.is_none());
        }
        for bad in [json!(-1), json!(9), json!("hard")] {
            assert!(s.execute("edit.autoBlendLayers", json!({"method": "stack", "haloControl": bad})).is_err(), "{bad}");
        }
    }
}

/// Three layers, each sharp in one third: the Depth layer says which, black to white from
/// the bottom layer to the top, hidden, and undone with the rest.
#[test]
fn auto_blend_stack_depth_map() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 96, "height": 48, "depth": depth, "background": "transparent"})).unwrap();
        let mut ids = Vec::new();
        for k in 0..3 {
            s.execute("layer.new.layer", json!({"name": format!("L{k}")})).unwrap();
            s.edit("third", |doc, active| {
                let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for y in 0..48 {
                    for x in 0..96 {
                        let v = if x / 32 == k && ((x / 2) + (y / 2)) % 2 == 0 {
                            0.2
                        } else if x / 32 == k {
                            0.8
                        } else {
                            0.5
                        };
                        surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        select(&mut s, &ids);
        assert!(s.execute("edit.autoBlendLayers", json!({"method": "panorama", "depthMap": true})).is_err());
        for seamless in [false, true] {
            let r = s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": seamless, "depthMap": true})).unwrap();
            let doc = &s.active().unwrap().doc;
            let d = doc.layer(LayerId(r["depth"].as_u64().unwrap())).unwrap();
            assert!(!d.visible && d.name.starts_with("Depth"), "depth {depth}: {} visible {} {r}", d.name, d.visible);
            let surf = d.surface().unwrap();
            for (x, want) in [(8, 0.0), (48, 0.5), (88, 1.0)] {
                let px = surf.pixel(x, 24);
                assert!((px[0] - want).abs() < 0.2 && (px[3] - 1.0).abs() < 1e-3, "depth {depth} seamless {seamless} x {x}: {px:?}");
            }
            assert_eq!(r["blended"].is_null(), !seamless);
            if seamless {
                // The merged layer sits above the depth layer and is the active one.
                assert_eq!(s.active().unwrap().active_layer, Some(LayerId(r["blended"].as_u64().unwrap())));
            }
            s.undo();
            assert!(s.active().unwrap().doc.layer(LayerId(r["depth"].as_u64().unwrap())).is_none());
        }
    }
}
