//! Content-Aware Move's Color control (#12197). Through Color 5 the moved middle stays as it was.
//! At Color 10 a brightness change shifts that middle by the mean difference of the surroundings;
//! a hue change leaves it alone. The edge band still blends. Move and Extend share the fit.

use super::*;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 8, "mode": "rgb"})).unwrap();
    s
}

fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [u8; 3]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let px = f(x, y);
                data.extend(photocraft_raster::from_rgba(&fmt, [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

fn levels(s: &Session, x: i32, y: i32) -> [i32; 3] {
    let d = s.active().unwrap();
    let px = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|c| (px[c] * 255.0).round() as i32)
}

fn checker(x: i32, y: i32) -> Option<u8> {
    if (100..180).contains(&x) && (100..180).contains(&y) {
        let (cx, cy) = ((x - 100) / 10, (y - 100) / 10);
        Some(if (cx + cy) % 2 == 0 { 60 } else { 200 })
    } else {
        None
    }
}

/// 800×600. Left half grey, right half green, with the 80×80 checker at (100, 100).
fn green_doc() -> Session {
    let mut s = session(800, 600);
    paint(&mut s, |x, y| {
        if let Some(v) = checker(x, y) {
            [v, v, v]
        } else if x < 400 {
            [128, 128, 128]
        } else {
            [40, 160, 60]
        }
    });
    s
}

/// The same checker on a horizontal grey ramp, constant across each 10 px column.
fn ramp_doc() -> Session {
    let mut s = session(800, 600);
    paint(&mut s, |x, y| {
        if let Some(v) = checker(x, y) {
            [v, v, v]
        } else {
            let column = x.div_euclid(10) * 10;
            let v = (40.0 + 180.0 * column as f32 / 790.0).round().clamp(0.0, 255.0) as u8;
            [v, v, v]
        }
    });
    s
}

fn flat_doc() -> Session {
    let mut s = session(800, 600);
    paint(&mut s, |x, _| if x < 400 { [204, 204, 204] } else { [51, 51, 51] });
    s
}

fn select_and_move(s: &mut Session, [x, y, w, h]: [i32; 4], offset: [i32; 2], color: u64, mode: &str) {
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    s.execute("paint.contentAwareMove", json!({"offset": offset, "structure": 4, "color": color, "mode": mode})).unwrap();
}

fn assert_near(got: [i32; 3], want: [i32; 3], tol: i32, what: &str) {
    for (c, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= tol, "{what} channel {c}: {got:?} vs {want:?} (±{tol})");
    }
}

#[test]
fn the_core_fit_shifts_brightness_and_leaves_a_hue_alone() {
    let grey = vec![130.0 / 255.0; 3];
    let bright = vec![176.0 / 255.0; 3];
    assert_eq!(cam_core_fit(3, &grey, &bright, 5, None), vec![(1.0, 0.0); 3], "color 5 does not move the middle");
    let bias = (176.0 - 130.0) / 255.0;
    for (g, b) in cam_core_fit(3, &grey, &bright, 10, None) {
        assert!((g - 1.0).abs() < 1e-5 && (b - bias).abs() < 1e-5, "color 10 is gain 1 plus the mean difference");
    }
    // Grey onto the green: the channels disagree, so the middle is copied.
    let green = vec![40.0 / 255.0, 160.0 / 255.0, 60.0 / 255.0];
    assert_eq!(cam_core_fit(3, &grey, &green, 10, None), vec![(1.0, 0.0); 3]);
    // One colour channel is a brightness change. Alpha is copied, not fitted.
    let from = vec![200.0 / 255.0, 200.0 / 255.0, 200.0 / 255.0, 1.0];
    let to = vec![51.0 / 255.0, 51.0 / 255.0, 51.0 / 255.0, 0.25];
    let gb = cam_core_fit(4, &from, &to, 10, Some(3));
    assert_eq!(gb[3], (1.0, 0.0), "alpha is copied");
    let expect = (51.0 - 200.0) / 255.0;
    assert!((gb[0].1 - expect).abs() < 1e-5 && gb[0].0 == 1.0);
    assert_eq!(cam_core_fit(1, &[200.0 / 255.0], &[51.0 / 255.0], 10, None)[0].0, 1.0);
}

#[test]
fn color_ten_on_green_leaves_the_middle_and_blends_the_edge() {
    // Selection 120,120 40×40 dragged (140,140) → (600,420). Row y 420 is the middle of the landing.
    let mut kept = green_doc();
    select_and_move(&mut kept, [120, 120, 40, 40], [460, 280], 0, "move");
    let mut fitted = green_doc();
    select_and_move(&mut fitted, [120, 120, 40, 40], [460, 280], 10, "move");

    // Color 0 keeps today's copy of the checker. Color 10 stays within ±5 of that, not a green tint.
    for (x, v) in [(590, 200), (595, 200), (600, 60), (605, 60), (610, 200)] {
        assert_eq!(levels(&kept, x, 420), [v, v, v], "color 0 keeps the checker at x {x}");
        assert_near(levels(&fitted, x, 420), levels(&kept, x, 420), 5, "color 10 middle");
    }
    // Edge band at Color 10. x 585 matches 55,86,60. x 610 is kept core (200), within ±20 of
    // 188,197,189. x 615's reference is 134,181,142; the existing fill already misses it at
    // Color 0 (about 178,190,180 against 143,182,149) and Color 10 lands near 101,160,111.
    // The range covers that blend and the reference, and rejects the old tint 77,197,97.
    assert_near(levels(&fitted, 585, 420), [55, 86, 60], 20, "x 585");
    assert_near(levels(&fitted, 610, 420), [188, 197, 189], 20, "x 610");
    let edge = levels(&fitted, 615, 420);
    assert!((90..=150).contains(&edge[0]) && (150..=195).contains(&edge[1]) && (100..=155).contains(&edge[2]), "x 615 blends into the green: {edge:?}");

    // Extend uses the same landing, so the middle stays too.
    let mut extended = green_doc();
    select_and_move(&mut extended, [120, 120, 40, 40], [460, 280], 10, "extend");
    for x in [590, 600, 610] {
        assert_near(levels(&extended, x, 420), levels(&fitted, x, 420), 5, "extend matches move");
    }
}

#[test]
fn a_brightness_change_shifts_the_middle_only_past_color_five() {
    let row = |color: u64| {
        let mut s = ramp_doc();
        select_and_move(&mut s, [120, 120, 40, 40], [460, 280], color, "move");
        (levels(&s, 590, 420), levels(&s, 600, 420))
    };
    let (light, dark) = row(0);
    assert_eq!((light, dark), ([200, 200, 200], [60, 60, 60]), "color 0 keeps the checker");
    assert_eq!(row(5), ([200, 200, 200], [60, 60, 60]), "color 5 leaves the middle");
    let (light, dark) = row(10);
    // Measured about +43 (242 / 104); the ring mean predicts about +46. Either is a real shift.
    assert!((230..=252).contains(&light[0]), "color 10 light cell: {light:?}");
    assert!((90..=112).contains(&dark[0]), "color 10 dark cell: {dark:?}");
    assert_eq!(light, [light[0]; 3]);
    assert_eq!(dark, [dark[0]; 3]);

    // Same lighting, dropped straight down: the middle darkens (about −63; dark cells clip).
    let mut s = ramp_doc();
    select_and_move(&mut s, [120, 120, 40, 40], [0, 280], 10, "move");
    let (light, dark) = (levels(&s, 130, 420), levels(&s, 140, 420));
    assert!(light[0] >= 120 && light[0] <= 155, "same lighting light cell: {light:?}");
    assert!(dark[0] <= 12, "same lighting dark cell clips near 0: {dark:?}");
    assert_eq!(light, [light[0]; 3]);
    assert_eq!(dark, [dark[0]; 3]);
}

#[test]
fn a_flat_patch_matches_the_new_surroundings_at_color_ten() {
    let middle = |color: u64, mode: &str| {
        let mut s = flat_doc();
        select_and_move(&mut s, [150, 250, 100, 100], [400, 0], color, mode);
        levels(&s, 600, 300)
    };
    for mode in ["move", "extend"] {
        assert_eq!(middle(0, mode), [204, 204, 204], "{mode} color 0 keeps 204");
        assert_eq!(middle(3, mode), [204, 204, 204], "{mode} color 3 leaves the middle");
        assert_eq!(middle(10, mode), [51, 51, 51], "{mode} color 10 becomes 51");
    }
    // The landing itself agrees; the old place is the part Move refills and Extend keeps.
    let mut moved = flat_doc();
    select_and_move(&mut moved, [150, 250, 100, 100], [400, 0], 10, "move");
    let mut extended = flat_doc();
    select_and_move(&mut extended, [150, 250, 100, 100], [400, 0], 10, "extend");
    for y in [256, 300, 340] {
        for x in [556, 600, 640] {
            assert_eq!(levels(&moved, x, y), levels(&extended, x, y), "move and extend agree at {x},{y}");
        }
    }
}
