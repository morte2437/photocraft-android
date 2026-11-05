use super::*;

/// A textured RGB scene (deterministic blobs on a gradient).
fn scene(w: usize, h: usize, seed: u64) -> Vec<[f32; 4]> {
    let mut z = seed;
    let mut rnd = || {
        z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (z >> 33) as f64 / (1u64 << 31) as f64
    };
    let mut px: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let x = (i % w) as f32 / w as f32;
            [0.3 + 0.2 * x, 0.35, 0.4 - 0.1 * x, 1.0]
        })
        .collect();
    for _ in 0..(w * h / 700) {
        let (cx, cy, r) = (rnd() * w as f64, rnd() * h as f64, 3.0 + rnd() * 9.0);
        let col = [rnd() as f32, rnd() as f32, rnd() as f32, 1.0];
        for y in (cy - r).max(0.0) as usize..((cy + r) as usize + 1).min(h) {
            for x in (cx - r).max(0.0) as usize..((cx + r) as usize + 1).min(w) {
                if (x as f64 - cx).hypot(y as f64 - cy) < r {
                    px[y * w + x] = col;
                }
            }
        }
    }
    px
}

fn doc_from(name: &str, px: &[[f32; 4]], w: usize, h: usize, depth: SampleType) -> Document {
    let mut d = Document::new(name, Size::new(w as u32, h as u32), ColorMode::Rgb, depth);
    let fmt = d.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; w * h * n];
    for (q, o) in px.iter().zip(data.chunks_exact_mut(n)) {
        photocraft_raster::from_rgba_into(&fmt, *q, o);
    }
    let mut s = Surface::new(fmt);
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    d.layers.push(Layer::new("Background", LayerContent::Raster(s)));
    d
}

fn crop(px: &[[f32; 4]], sw: usize, x0: usize, y0: usize, w: usize, h: usize, gain: f32) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let q = px[(y0 + i / w) * sw + x0 + i % w];
            [q[0] * gain, q[1] * gain, q[2] * gain, 1.0]
        })
        .collect()
}

/// A fresh temporary folder for a test's files.
#[cfg(not(target_arch = "wasm32"))]
fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-photomerge-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn photomerge_open_documents_reposition_and_perspective() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (sw, sh) = (520, 220);
        let sc = scene(sw, sh, 5);
        let mut s = Session::new();
        // Three overlapping crops, the middle one 15 % brighter (gain compensation).
        for (k, (x0, g)) in [(0usize, 1.0f32), (130, 1.15), (260, 1.0)].into_iter().enumerate() {
            s.add_document(doc_from(&format!("img{k}"), &crop(&sc, sw, x0, 10, 220, 200, g), 220, 200, depth), None);
        }
        let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition"})).unwrap();
        assert_eq!(r["placed"].as_array().unwrap().len(), 3, "{r}");
        let (w, h) = (r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap());
        assert!((w as i64 - 480).abs() <= 2 && (h as i64 - 200).abs() <= 2, "{depth:?} {w}×{h}");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 3);
        assert!(d.layers.iter().all(|l| l.mask.is_some()));
        // Masks partition the panorama: every covered pixel is shown by exactly one layer.
        for &(x, y) in &[(20, 100), (200, 50), (380, 150), (470, 120)] {
            let shown: f32 = d.layers.iter().map(|l| l.mask.as_ref().unwrap().value(x, y) * l.surface().unwrap().rgba(x, y)[3]).sum();
            assert!((shown - 1.0).abs() < 0.01, "({x},{y}) {shown}");
        }
        // The composite matches the scene up to the global gain.
        let comp = photocraft_compose::flatten(d);
        let at = |x: i32, y: i32| comp.px[(y * comp.rect.width() as i32 + x) as usize];
        let gains = r["photometric"]["gains"].as_array().unwrap();
        assert!(gains[1][0].as_f64().unwrap() > gains[0][0].as_f64().unwrap() * 1.05, "{r}");
        let probe = at(160, 100);
        let truth = sc[110 * sw + 160];
        assert!((probe[1] - truth[1]).abs() < 0.08, "{probe:?} vs {truth:?}");
    }
    // Perspective + no blending: plain layers, no masks.
    let (sw, sh) = (520, 220);
    let sc = scene(sw, sh, 8);
    let mut s = Session::new();
    for (k, x0) in [0usize, 180].into_iter().enumerate() {
        s.add_document(doc_from(&format!("p{k}"), &crop(&sc, sw, x0, 10, 260, 200, 1.0), 260, 200, SampleType::U8), None);
    }
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "perspective", "blend": false})).unwrap();
    assert_eq!(r["layout"], "perspective");
    let d = &s.active().unwrap().doc;
    assert!(d.layers.iter().all(|l| l.mask.is_none()));
    assert!((r["width"].as_i64().unwrap() - 440).abs() <= 8, "{r}");
    assert!(s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "zigzag"})).is_err());
    let mut empty = Session::new();
    assert!(empty.execute("file.automate.photomerge", json!({})).is_err());
}

#[test]
fn photomerge_cylindrical_with_fill() {
    let (sw, sh) = (900, 260);
    let sc = scene(sw, sh, 21);
    let f = 260.0f64;
    let mut s = Session::new();
    let (tw, th) = (260usize, 200usize);
    for (k, pan) in [-0.45f64, 0.0, 0.45].into_iter().enumerate() {
        let px: Vec<[f32; 4]> = (0..tw * th)
            .map(|i| {
                let (dx, dy) = ((i % tw) as f64 - tw as f64 / 2.0, (i / tw) as f64 - th as f64 / 2.0);
                let (u, v) = (sw as f64 / 2.0 + f * (dx.atan2(f) + pan), sh as f64 / 2.0 + f * dy / dx.hypot(f));
                sc[(v.round().clamp(0.0, sh as f64 - 1.0) as usize) * sw + u.round().clamp(0.0, sw as f64 - 1.0) as usize]
            })
            .collect();
        s.add_document(doc_from(&format!("c{k}"), &px, tw, th, SampleType::U8), None);
    }
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "cylindrical", "contentAwareFill": true})).unwrap();
    assert_eq!(r["layout"], "cylindrical");
    assert_eq!(r["placed"].as_array().unwrap().len(), 3, "{r}");
    let d = &s.active().unwrap().doc;
    let top = d.layers.last().unwrap();
    assert_eq!(top.name, "Content-Aware Fill");
    // The fill layer is opaque everywhere (the bow-tie corners are filled).
    let b = d.bounds();
    assert!(top.surface().unwrap().rgba(1, 1)[3] > 0.99 && top.surface().unwrap().rgba(b.x1 - 2, b.y1 - 2)[3] > 0.99);
}

#[test]
fn hdr_merge_from_synthetic_exposures() {
    let (w, h) = (96usize, 64usize);
    let radiance: Vec<[f32; 3]> = (0..w * h)
        .map(|i| {
            let x = (i % w) as f32 / w as f32;
            let b = 0.003 * 2f32.powf(x * 11.0) * (1.0 + 0.2 * ((i / w) as f32 * 0.3).sin());
            [b, b * 0.9, b * 0.7]
        })
        .collect();
    let shoot = |dt: f32| -> Vec<[f32; 4]> {
        radiance
            .iter()
            .map(|r| {
                let f = |v: f32| photocraft_algo_srgb((v * dt).min(1.0));
                [f(r[0]), f(r[1]), f(r[2]), 1.0]
            })
            .collect()
    };
    let mut s = Session::new();
    for (k, dt) in [0.25f32, 1.0, 4.0].into_iter().enumerate() {
        let mut d = doc_from(&format!("e{k}"), &shoot(dt), w, h, SampleType::U8);
        d.metadata.exif = Some(std::sync::Arc::new(exif::build(&exif::CameraInfo {
            exposure_time: Some(dt as f64 / 100.0),
            f_number: Some(8.0),
            iso: Some(100.0),
            ..Default::default()
        })));
        s.add_document(d, None);
    }
    let r = s.execute("file.automate.mergeToHdrPro", json!({"useOpenDocuments": true})).unwrap();
    assert_eq!(r["exposureSource"], "exif");
    let ev: Vec<f64> = r["ev"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!((ev[1] - 2.0).abs() < 1e-6 && (ev[2] - 4.0).abs() < 1e-6, "{ev:?}");
    let d = &s.active().unwrap().doc;
    assert_eq!(d.depth, SampleType::F32);
    assert!(d.is_background(d.layers[0].id), "the merged layer is a real (locked) Background");
    let l = d.layers[0].surface().unwrap();
    // Linear output: ratios across the ramp follow the radiance.
    let (a, b) = (l.rgba(20, 30)[1] as f64, l.rgba(80, 30)[1] as f64);
    let expect = radiance[30 * w + 80][1] as f64 / radiance[30 * w + 20][1] as f64;
    assert!(((b / a) / expect).ln().abs() < 0.3, "{} vs {expect}", b / a);
    assert!(r["stops"].as_f64().unwrap() > 7.0, "{r}");
    let out = r["document"].as_u64().unwrap() as usize;
    s.close(out);
    // 8-bit tone-mapped output with explicit EVs.
    for method in ["localAdaptation", "exposureGamma", "highlightCompression", "equalizeHistogram"] {
        let r = s
            .execute("file.automate.mergeToHdrPro", json!({"paths": [], "useOpenDocuments": true, "exposures": [0, 2, 4], "mode": "8", "method": method}))
            .unwrap();
        assert_eq!(r["exposureSource"], "params");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.depth, SampleType::U8);
        let out = r["document"].as_u64().unwrap() as usize;
        s.close(out);
    }
    assert!(s.execute("file.automate.mergeToHdrPro", json!({"useOpenDocuments": true, "exposures": [0, 1]})).is_err());
}

fn photocraft_algo_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// A light scan with one dark tilted print on it.
fn scan_with_one_photo() -> Document {
    let (w, h) = (300usize, 220usize);
    let mut px = vec![[0.98f32, 0.98, 0.98, 1.0]; w * h];
    let (s, co) = 5.0f64.to_radians().sin_cos();
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f64 + 0.5 - 150.0, y as f64 + 0.5 - 110.0);
            let (u, v) = (co * dx + s * dy, -s * dx + co * dy);
            if u.abs() < 80.0 && v.abs() < 55.0 {
                px[y * w + x] = [0.2 + (u / 160.0) as f32 * 0.3, 0.25, 0.3, 1.0];
            }
        }
    }
    doc_from("scan", &px, w, h, SampleType::U8)
}

#[test]
fn crop_and_straighten_keeps_duotone_inks() {
    let mut s = Session::new();
    s.add_document(scan_with_one_photo(), None);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.duotone", json!({"type": "duotone", "inks": [{"name": "Black", "color": [0, 0, 0]}, {"name": "Orange", "color": [255, 128, 0]}]}))
        .unwrap();
    let src = s.documents()[0].doc.duotone.clone();
    assert!(src.is_some());
    let r = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap();
    assert_eq!(r["photos"].as_array().unwrap().len(), 1, "{r}");
    let d = &s.documents()[1].doc;
    assert_eq!(d.mode, ColorMode::Duotone);
    assert_eq!(d.duotone, src);
}

#[test]
fn crop_and_straighten_disabled_for_indexed() {
    // Photoshop greys the command out for Indexed Color documents.
    let mut s = Session::new();
    s.add_document(scan_with_one_photo(), None);
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    assert_eq!(s.documents()[0].doc.mode, ColorMode::Indexed);
    assert!(!s.is_enabled("file.automate.cropAndStraightenPhotos"));
    assert!(s.execute("file.automate.cropAndStraightenPhotos", json!({})).is_err());
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn crop_and_straighten_only_processes_the_selection() {
    // Photoshop: with a selection, only the photos inside it are split off.
    let (w, h) = (480usize, 340usize);
    let mut px = vec![[0.98f32, 0.98, 0.98, 1.0]; w * h];
    // Upright photos: red 160x110 at (40, 60), blue 150x120 at (290, 180).
    for (x0, y0, pw, ph, c) in [(40, 60, 160, 110, [0.6f32, 0.1, 0.1, 1.0]), (290, 180, 150, 120, [0.1, 0.2, 0.6, 1.0])] {
        for y in y0..y0 + ph {
            px[y * w + x0..y * w + x0 + pw].fill(c);
        }
    }
    let mut s = Session::new();
    s.add_document(doc_from("scan", &px, w, h, SampleType::U8), None);
    s.execute("select.rect", json!({"x": 270, "y": 160, "width": 190, "height": 160})).unwrap();
    let r = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap();
    let found = r["photos"].as_array().unwrap();
    assert_eq!(found.len(), 1, "{r}");
    assert_eq!(s.documents().len(), 2);
    // The reported centre is in the scan's coordinates.
    let c = &found[0]["center"];
    assert!((c[0].as_f64().unwrap() - 365.0).abs() < 3.0 && (c[1].as_f64().unwrap() - 240.0).abs() < 3.0, "{c}");
    let d = &s.documents()[1].doc;
    assert!((d.size.width as i64 - 150).abs() <= 4 && (d.size.height as i64 - 120).abs() <= 4, "{:?}", d.size);
    let q = d.layers[0].surface().unwrap().rgba(d.size.width as i32 / 2, d.size.height as i32 / 2);
    assert!(q[2] > 0.5 && q[0] < 0.2, "{q:?}");
    // Without a selection both photos are found.
    s.set_active(0);
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap();
    assert_eq!(r["photos"].as_array().unwrap().len(), 2, "{r}");
}

#[test]
fn crop_and_straighten_skips_photos_between_selected_regions() {
    // Two selected rectangles: a photo in the gap between them lies inside
    // their bounding box but outside the selection, so it is not split off.
    let (w, h) = (480usize, 340usize);
    let mut px = vec![[0.75f32, 0.75, 0.75, 1.0]; w * h];
    // Red 160x110 at (40, 60) and blue 150x120 at (290, 180) are selected; green 130x80 at (280, 50) is not.
    for (x0, y0, pw, ph, c) in
        [(40, 60, 160, 110, [0.6f32, 0.1, 0.1, 1.0]), (290, 180, 150, 120, [0.1, 0.2, 0.6, 1.0]), (280, 50, 130, 80, [0.1, 0.6, 0.1, 1.0])]
    {
        for y in y0..y0 + ph {
            px[y * w + x0..y * w + x0 + pw].fill(c);
        }
    }
    let mut s = Session::new();
    s.add_document(doc_from("scan", &px, w, h, SampleType::U8), None);
    s.execute("select.rect", json!({"x": 20, "y": 40, "width": 200, "height": 150})).unwrap();
    s.execute("select.rect", json!({"x": 270, "y": 160, "width": 190, "height": 160, "mode": "add"})).unwrap();
    let r = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap();
    let found = r["photos"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{r}");
    for f in found {
        let c = &f["center"];
        let (cx, cy) = (c[0].as_f64().unwrap(), c[1].as_f64().unwrap());
        let near = |x: f64, y: f64| (cx - x).abs() < 3.0 && (cy - y).abs() < 3.0;
        assert!(near(120.0, 115.0) || near(365.0, 240.0), "{c}");
    }
}

#[test]
fn crop_and_straighten_refuses_a_selection_under_half_coverage() {
    // A faint (strongly feathered) selection has no pixel at or above 50%:
    // like Photoshop, nothing counts as selected, so no photo is split off
    // from the unmasked bounding box.
    let (w, h) = (480usize, 340usize);
    let mut px = vec![[0.98f32, 0.98, 0.98, 1.0]; w * h];
    for y in 180..300 {
        px[y * w + 290..y * w + 440].fill([0.1, 0.2, 0.6, 1.0]);
    }
    let mut s = Session::new();
    s.add_document(doc_from("scan", &px, w, h, SampleType::U8), None);
    let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
    m.fill_rect(Rect::new(270, 160, 460, 320), &[0.3]);
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).selection = Some(m);
    let e = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap_err();
    assert!(e.to_string().contains("50%"), "{e}");
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn crop_and_straighten_makes_documents() {
    let (w, h) = (500usize, 360usize);
    let mut px = vec![[0.98f32, 0.98, 0.98, 1.0]; w * h];
    let photos = [([140.0f64, 120.0f64], 160.0f64, 110.0f64, 6.0f64), ([360.0, 240.0], 150.0, 120.0, -9.0)];
    for (c, pw, ph, ang) in photos {
        let (s, co) = ang.to_radians().sin_cos();
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 + 0.5 - c[0], y as f64 + 0.5 - c[1]);
                let (u, v) = (co * dx + s * dy, -s * dx + co * dy);
                if u.abs() < pw / 2.0 && v.abs() < ph / 2.0 {
                    px[y * w + x] = [0.2 + (u / pw) as f32 * 0.3, 0.3, 0.6, 1.0];
                }
            }
        }
    }
    let mut s = Session::new();
    s.add_document(doc_from("scan", &px, w, h, SampleType::U16), None);
    assert!(s.is_enabled("file.automate.cropAndStraightenPhotos"));
    let r = s.execute("file.automate.cropAndStraightenPhotos", json!({})).unwrap();
    let found = r["photos"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{r}");
    assert_eq!(s.documents().len(), 3);
    assert!((found[0]["angle"].as_f64().unwrap() - 6.0).abs() < 1.0);
    let d = &s.documents()[1].doc;
    assert!((d.size.width as i64 - 158).abs() <= 4 && (d.size.height as i64 - 108).abs() <= 4, "{:?}", d.size);
    assert_eq!(d.depth, SampleType::U16);
    // Upright: the photo's horizontal gradient runs along x.
    let l = d.layers[0].surface().unwrap();
    let (a, b) = (l.rgba(10, d.size.height as i32 / 2)[0], l.rgba(d.size.width as i32 - 10, d.size.height as i32 / 2)[0]);
    assert!(b > a + 0.15, "{a} {b}");
    let (c, e) = (l.rgba(d.size.width as i32 / 2, 6)[0], l.rgba(d.size.width as i32 / 2, d.size.height as i32 - 6)[0]);
    assert!((c - e).abs() < 0.05);
}

#[test]
fn photomerge_rejects_insufficient_memory_budget() {
    let (sw, sh) = (520, 220);
    let sc = scene(sw, sh, 5);
    let mut s = Session::new();
    for (k, x0) in [0usize, 180].into_iter().enumerate() {
        s.add_document(doc_from(&format!("img{k}"), &crop(&sc, sw, x0, 10, 260, 200, 1.0), 260, 200, SampleType::U8), None);
    }
    assert_eq!(s.documents().len(), 2);
    // Budget of 1024 bytes is insufficient for the panorama.
    let err = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "memoryBudget": 1024})).unwrap_err();
    assert!(err.to_string().contains("not enough memory for file.automate.photomerge"), "{err}");
    assert!(err.to_string().contains("exceeds the"), "{err}");
    // Session state and documents must remain intact.
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active().unwrap().doc.name, "img1");
}

#[test]
fn photomerge_estimate_is_the_largest_phase_not_the_sum() {
    // Two 100×100 RGB8 images (4 bytes per pixel in the canvas) on a 150×100 canvas.
    let sizes = [(100, 100), (100, 100)];
    let (cw, ch, cc, bpp) = (150, 100, 3, 4);
    let band = panorama::band_bytes_bound(100, 100, cc, panorama::blend_levels(cw, ch).min(7), cw, ch);
    assert_eq!(band, 150 * 100 * 4 * 12);
    let (rois, accumulators) = (20_000 * 20, 15_000 * 21);
    let (peak, fixed) = estimate_photomerge_memory(&sizes, &sizes, cw, ch, cc, bpp, true, false);
    assert_eq!(fixed, rois + accumulators);
    // The bands phase (ROIs, accumulators and one band) is the largest here.
    let bands = rois + accumulators + band;
    let collapse = rois + accumulators + 15_000 * 12;
    let output = rois + 15_000 * 16 + 15_000 * 4 + 20_000 * 5;
    assert!(bands > collapse && bands > output);
    assert_eq!(peak, bands);
    // A sum of every buffer would be far more.
    assert!(peak < rois + accumulators + 15_000 * 16 + 20_000 * 5 + band);
    // Filling adds its own phase, which does not exceed the bands here.
    let (with_fill, _) = estimate_photomerge_memory(&sizes, &sizes, cw, ch, cc, bpp, true, true);
    assert_eq!(with_fill, (20_000 * 5 + 15_000 * 4 + 15_000 * 4 + (24 << 20)).max(peak));
    // Without blending only the sources and warped layers are held.
    assert_eq!(estimate_photomerge_memory(&sizes, &sizes, cw, ch, cc, bpp, false, false), (80_000 + 80_000, 0));
}

#[test]
fn photomerge_estimate_fits_six_large_photos_in_eight_gib() {
    let photos = [(6000, 4000); 6];
    let (peak, fixed) = estimate_photomerge_memory(&photos, &photos, 27_000, 4_112, 3, 4, true, false);
    assert!(peak < 8 << 30, "{} MiB", peak >> 20);
    assert!(fixed < peak);
}

/// A flat opaque RGBA8 surface covering `area`.
fn flat_surface(area: Rect, v: f32) -> Surface {
    let fmt = PixelFormat::RGBA8;
    let n = (area.width() * area.height()) as usize;
    let data: Vec<f32> = (0..n).flat_map(|_| [v, v, v, 1.0]).collect();
    let mut s = Surface::new(fmt);
    s.write_region(area, &data);
    s
}

#[test]
fn seam_blend_rejects_a_band_larger_than_its_budget() {
    let (a, b) = (Rect::new(0, 0, 64, 64), Rect::new(32, 0, 96, 64));
    let warped = vec![flat_surface(a, 0.3), flat_surface(b, 0.6)];
    let opt = BlendOptions { seamless: true, gains: true, geometry: None, budget: Some(1000) };
    let err = seam_blend(warped, Rect::new(0, 0, 96, 64), &[0, 1], opt, None).err().expect("a 1000 byte budget can't hold a band");
    assert!(err.to_string().contains("not enough memory to allocate"), "{err}");
    // Without a budget the same blend succeeds.
    let warped = vec![flat_surface(a, 0.3), flat_surface(b, 0.6)];
    let opt = BlendOptions { seamless: true, gains: true, geometry: None, budget: None };
    assert!(seam_blend(warped, Rect::new(0, 0, 96, 64), &[0, 1], opt, None).is_ok());
}

#[test]
fn photomerge_succeeds_with_a_large_enough_memory_budget() {
    let (sw, sh) = (520, 220);
    let sc = scene(sw, sh, 5);
    let mut s = Session::new();
    for (k, x0) in [0usize, 180].into_iter().enumerate() {
        s.add_document(doc_from(&format!("img{k}"), &crop(&sc, sw, x0, 10, 260, 200, 1.0), 260, 200, SampleType::U8), None);
    }
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "memoryBudget": 1u64 << 30})).unwrap();
    assert_eq!(r["layers"], 2, "{r}");
    assert_eq!(s.documents().len(), 3);
}

#[test]
fn photomerge_memory_budget_precedence_and_defaults() {
    // The param wins over the environment and the machine.
    assert_eq!(budget_from(Some(123), Some("64"), Some(64 << 30)), 123);
    // Then the environment variable (MiB).
    assert_eq!(budget_from(None, Some(" 64 "), Some(64 << 30)), 64 << 20);
    // An unparsable variable falls through to the machine.
    assert_eq!(budget_from(None, Some("lots"), Some(8 << 30)), 4 << 30);
    // Half of RAM with no upper ceiling.
    assert_eq!(budget_from(None, None, Some(64 << 30)), 32 << 30);
    // At least 1 GiB, and 4 GiB of RAM assumed when unknown.
    assert_eq!(budget_from(None, None, Some(1 << 30)), 1 << 30);
    assert_eq!(budget_from(None, None, None), 2 << 30);
    // Huge values saturate instead of panicking or truncating.
    assert_eq!(budget_from(Some(u64::MAX), None, None), usize::try_from(u64::MAX).unwrap_or(usize::MAX));
    let _ = budget_from(None, Some("18446744073709551615"), None);
    let _ = budget_from(None, None, Some(u64::MAX));
}

#[test]
fn photomerge_rejects_oversized_sparse_source_before_flattening() {
    let mut s = Session::new();
    s.add_document(Document::new("sparse", Size::new(100_000, 100_000), ColorMode::Rgb, SampleType::U8), None);
    let err = load_sources(&s, &json!({"useOpenDocuments": true}), "file.automate.photomerge", None).err().expect("oversized canvas must be refused");
    assert!(err.to_string().contains("too large to flatten"), "{err}");
    assert_eq!(s.documents().len(), 1);
    assert_eq!(s.active().unwrap().doc.name, "sparse");
}

#[test]
fn photomerge_source_load_charges_the_document_and_both_flatten_surfaces() {
    let cmd = "file.automate.photomerge";
    let px = scene(260, 200, 3);
    let one = doc_from("a", &px, 260, 200, SampleType::U8);
    let bpp = one.pixel_format().bytes_per_pixel();
    let output = surface_bytes(260, 200, bpp);
    let cost = SourceLoad::new(None).needed(&one, bpp, 0);
    assert_eq!(document_pixels(&one), output, "a covered 260×200 surface is two 256 px tiles");
    assert_eq!(cost, 3 * output, "the document, the kept surface and the flatten's working surface");

    let mut load = SourceLoad::new(Some(cost));
    assert!(load.admit(cmd, &one, bpp, 0).is_ok(), "the first source fits exactly");
    assert_eq!(load.retained, output);
    let err = load.admit(cmd, &one, bpp, 0).expect_err("the second source exceeds the budget");
    assert!(err.to_string().contains("not enough memory for file.automate.photomerge"), "{err}");
    assert!(err.to_string().contains("260×200"), "{err}");
    assert!(err.to_string().contains("exceeds the"), "{err}");
    assert_eq!(load.retained, output, "a refused source is not counted as retained");

    // An encoded file's bytes are held during the import as well.
    assert!(SourceLoad::new(Some(cost + 99)).admit(cmd, &one, bpp, 100).is_err());
    assert!(SourceLoad::new(Some(cost + 100)).admit(cmd, &one, bpp, 100).is_ok());

    // Every layer of the decoded document is charged, not just the two output surfaces.
    let mut layered = one.clone();
    layered.layers.extend(doc_from("b", &px, 260, 200, SampleType::U8).layers);
    assert_eq!(document_pixels(&layered), 2 * output);
    assert!(SourceLoad::new(Some(cost + output - 1)).admit(cmd, &layered, bpp, 0).is_err());
    assert!(SourceLoad::new(Some(cost + output)).admit(cmd, &layered, bpp, 0).is_ok());

    // Hostile sizes saturate and fail a finite budget instead of wrapping under it.
    assert!(SourceLoad::new(Some(1 << 40)).admit(cmd, &one, usize::MAX, usize::MAX).is_err());
    // Without a budget (Merge to HDR Pro) nothing is assessed.
    assert!(SourceLoad::new(None).admit(cmd, &one, usize::MAX, usize::MAX).is_ok());
}

#[test]
fn photomerge_rejects_over_budget_sources_before_flattening() {
    let (sw, sh) = (520, 220);
    let sc = scene(sw, sh, 9);
    let mut s = Session::new();
    for (k, x0) in [0usize, 180].into_iter().enumerate() {
        s.add_document(doc_from(&format!("img{k}"), &crop(&sc, sw, x0, 10, 260, 200, 1.0), 260, 200, SampleType::U8), None);
    }
    // Enough for the first source, but not for a second one after its own surface is kept.
    let one = &s.documents()[0].doc;
    let bpp = one.pixel_format().bytes_per_pixel();
    let cost = SourceLoad::new(None).needed(one, bpp, 0);
    let budget = cost + surface_bytes(260, 200, bpp) / 2;
    let err = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "memoryBudget": budget as u64})).unwrap_err();
    assert!(err.to_string().contains("not enough memory for file.automate.photomerge"), "{err}");
    assert!(err.to_string().contains("exceeds the"), "{err}");
    // Refused during loading, before alignment, warping or a new document.
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active().unwrap().doc.name, "img1");
}

#[test]
fn photomerge_rejects_a_panorama_over_budget_after_loading() {
    let (sw, sh) = (520, 220);
    let sc = scene(sw, sh, 5);
    let mut s = Session::new();
    for (k, x0) in [0usize, 180].into_iter().enumerate() {
        s.add_document(doc_from(&format!("img{k}"), &crop(&sc, sw, x0, 10, 260, 200, 1.0), 260, 200, SampleType::U8), None);
    }
    // Enough to load both sources, but below the warp/blend estimate, so the post-alignment
    // check refuses the job rather than the loader.
    let one = &s.documents()[0].doc;
    let bpp = one.pixel_format().bytes_per_pixel();
    let budget = SourceLoad::new(None).needed(one, bpp, 0) + surface_bytes(260, 200, bpp);
    let err = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "memoryBudget": budget as u64})).unwrap_err();
    assert!(err.to_string().contains("estimated memory"), "{err}");
    assert!(err.to_string().contains("exceeds budget"), "{err}");
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active().unwrap().doc.name, "img1");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn photomerge_refuses_an_oversized_file_before_reading_or_decoding_it() {
    let cmd = "file.automate.photomerge";
    let dir = temp_dir("oversized-file");
    let path = dir.join("big.png");
    // Not a valid image: reaching the decoder would report a format error instead.
    std::fs::write(&path, vec![0xAB; 4096]).unwrap();
    let err = load_sources(&Session::new(), &json!({"paths": [path.to_string_lossy()]}), cmd, Some(1024))
        .err()
        .expect("a file larger than the budget must be refused");
    assert!(err.to_string().contains("was not read"), "{err}");
    assert!(err.to_string().contains("only 1 KiB is left"), "{err}");
    assert!(!err.to_string().contains("unrecognized"), "{err}");
    assert!(!err.to_string().contains("image format"), "{err}");
}

/// A source that cannot be opened is a file error under a budget too, not a memory error.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn photomerge_reports_a_missing_file_as_a_file_error_under_a_budget() {
    let cmd = "file.automate.photomerge";
    let missing = temp_dir("missing-file").join("does-not-exist.png");
    let err = load_sources(&Session::new(), &json!({"paths": [missing.to_string_lossy()]}), cmd, Some(1 << 30)).err().expect("a missing file must fail");
    let message = err.to_string();
    assert!(!message.contains("not enough memory"), "{message}");
    assert!(message.contains("does-not-exist.png"), "{message}");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn photomerge_file_source_is_charged_beside_the_sources_already_loaded() {
    let cmd = "file.automate.photomerge";
    let px = scene(260, 200, 4);
    let open = doc_from("open", &px, 260, 200, SampleType::U8);
    let bpp = open.pixel_format().bytes_per_pixel();
    let cost = SourceLoad::new(None).needed(&open, bpp, 0);
    let png = photocraft_io::export(&open, "source.png", &Default::default()).unwrap().bytes;
    let dir = temp_dir("file-budget");
    let path = dir.join("source.png");
    std::fs::write(&path, &png).unwrap();
    // Exactly enough for the file's encoded bytes, its document and its flatten surfaces, plus
    // two bytes: alone it loads, but not beside an open source that keeps its surface.
    let budget = cost + png.len() + 2;

    let alone = Session::new();
    let (loaded, _, _) = load_sources(&alone, &json!({"paths": [path.to_string_lossy()]}), cmd, Some(budget)).expect("the file alone fits");
    assert_eq!(loaded.len(), 1);

    let mut s = Session::new();
    s.add_document(open, None);
    let err = load_sources(&s, &json!({"useOpenDocuments": true, "paths": [path.to_string_lossy()]}), cmd, Some(budget))
        .err()
        .expect("the retained open source leaves no room for the file's own pixels");
    let message = err.to_string();
    assert!(message.contains("exceeds the") || message.contains("limit exceeded"), "{message}");
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn merge_to_hdr_pro_accepts_sources_without_the_photomerge_budget() {
    let (sw, sh) = (120, 90);
    let sc = scene(sw, sh, 11);
    let mut s = Session::new();
    for k in 0..2 {
        s.add_document(doc_from(&format!("exp{k}"), &crop(&sc, sw, 0, 0, sw, sh, 1.0 + k as f32), sw, sh, SampleType::U8), None);
    }
    // A 1-byte Photomerge budget refuses the sources; Merge to HDR Pro passes no budget and
    // still loads them, so its behaviour is unchanged.
    assert!(load_sources(&s, &json!({"useOpenDocuments": true}), "file.automate.photomerge", Some(1)).is_err());
    let (srcs, fmt, _) = load_sources(&s, &json!({"useOpenDocuments": true}), "file.automate.mergeToHdrPro", None).unwrap();
    assert_eq!(srcs.len(), 2);
    assert_eq!(fmt, PixelFormat::RGBA8);
}
