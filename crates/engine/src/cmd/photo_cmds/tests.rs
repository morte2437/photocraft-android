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
