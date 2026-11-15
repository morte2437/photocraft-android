//! #12879: the Pattern Stamp's Behind mode (`"mode": "behind"`). The pattern goes beneath the
//! layer's pixels: it shows where the layer is transparent and leaves existing pixels alone,
//! Aligned on and off, as the Brush's Behind does. The live stroke shows what the commit paints,
//! and every other mode paints what it painted before. Behind with Impressionist was not
//! measured, so an Impressionist stroke paints what it paints in Normal.

use photocraft_paint::StrokePoint;
use serde_json::json;

use super::*;

const W: i32 = 64;
const H: i32 = 48;
const RED: [f32; 4] = [0.8, 0.1, 0.1, 1.0];

/// A 64 × 48 document at `depth` with a new layer above the Background whose pixels are `f`.
fn session(depth: u64, f: impl Fn(i32, i32) -> [f32; 4]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "mode": "rgb"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let data: Vec<f32> =
            (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).map(move |x| (x, y))).flat_map(|(x, y)| photocraft_raster::from_rgba(&fmt, f(x, y))).collect();
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

/// Opaque red on the left half, transparent on the right.
fn half(x: i32, _: i32) -> [f32; 4] {
    if x < W / 2 { RED } else { [0.0; 4] }
}

/// A fully transparent layer.
fn empty(_: i32, _: i32) -> [f32; 4] {
    [0.0; 4]
}

/// The active layer's pixels, row by row.
fn pixels(doc: &photocraft_doc::Document, s: &Session) -> Vec<[f32; 4]> {
    let surf = doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| surf.rgba(x, y)).collect()
}

/// The active layer's pixels in the session's document.
fn layer(s: &Session) -> Vec<[f32; 4]> {
    pixels(&s.active().unwrap().doc, s)
}

/// The pixel at (x, y) of a row-by-row list.
fn at(px: &[[f32; 4]], x: i32, y: i32) -> [f32; 4] {
    px[(y * W + x) as usize]
}

/// History steps taken so far.
fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// A hard stroke across both halves, with `extra` on top.
fn stroke(extra: Value) -> Value {
    let mut p = json!({"points": [[12, 24], [52, 24]], "size": 16, "hardness": 100, "pattern": "Checkerboard", "seed": 5});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// Runs [`stroke`] as `paint.patternStamp`.
fn stamp(s: &mut Session, extra: Value) -> Value {
    s.execute("paint.patternStamp", stroke(extra)).unwrap()
}

/// A half-filled layer, Aligned on and off, at every depth: the filled half is untouched and the
/// transparent half gets exactly the pattern a Normal stroke lays on transparency.
#[test]
fn behind_paints_only_where_the_layer_is_transparent() {
    for depth in [8, 16, 32] {
        for aligned in [true, false] {
            let mut s = session(depth, half);
            let before = layer(&s);
            let n = steps(&s);
            stamp(&mut s, json!({"mode": "behind", "aligned": aligned}));
            assert_eq!(steps(&s), n + 1, "one History step");
            // The same stroke in Normal on an empty layer: what the transparent half must show.
            let mut clear = session(depth, empty);
            stamp(&mut clear, json!({"aligned": aligned}));
            let (got, want) = (layer(&s), layer(&clear));
            let mut painted = 0;
            for y in 0..H {
                for x in 0..W {
                    let (g, tag) = (at(&got, x, y), format!("{depth}-bit aligned {aligned} ({x}, {y})"));
                    if x < W / 2 {
                        assert_eq!(g, at(&before, x, y), "{tag}: an existing pixel changed");
                    } else {
                        assert_eq!(g, at(&want, x, y), "{tag}: the pattern as Normal lays it on transparency");
                        painted += usize::from(g[3] > 0.99);
                    }
                }
            }
            assert!(painted > 200, "{depth}-bit aligned {aligned}: the transparent half was painted ({painted} px)");
            assert!(s.undo());
            assert_eq!(layer(&s), before);
        }
    }
}

/// Half-transparent pixels keep their colour on top of the pattern, as the Brush's Behind.
#[test]
fn behind_goes_beneath_partly_transparent_pixels() {
    let mut s = session(32, |_, _| [0.8, 0.1, 0.1, 0.5]);
    stamp(&mut s, json!({"mode": "Behind"}));
    let mut clear = session(32, empty);
    stamp(&mut clear, json!({}));
    let (got, pat) = (at(&layer(&s), 40, 24), at(&layer(&clear), 40, 24));
    assert!(pat[3] > 0.99, "{pat:?}");
    // Half red on top of the pattern: the layer's colour keeps its share.
    assert!((got[3] - 1.0).abs() < 1e-4, "{got:?}");
    for c in 0..3 {
        assert!((got[c] - (0.5 * RED[c] + 0.5 * pat[c])).abs() < 1e-3, "{got:?} over {pat:?}");
    }
    // Outside the stroke nothing moved.
    assert_eq!(at(&layer(&s), 40, 4), [0.8, 0.1, 0.1, 0.5]);
}

/// Opacity weighs what goes onto transparency; existing pixels still do not move.
#[test]
fn behind_follows_opacity_on_transparency() {
    let mut s = session(32, half);
    stamp(&mut s, json!({"mode": "behind", "opacity": 50}));
    let px = layer(&s);
    assert!((at(&px, 44, 24)[3] - 0.5).abs() < 1e-3, "{:?}", at(&px, 44, 24));
    assert_eq!(at(&px, 20, 24), RED);
}

/// Opaque pixels, locked transparency and the Background take nothing, live or committed.
#[test]
fn behind_paints_nothing_without_transparency_to_paint_into() {
    // A fully opaque layer.
    let mut s = session(8, |_, _| RED);
    let before = layer(&s);
    stamp(&mut s, json!({"mode": "behind"}));
    assert_eq!(layer(&s), before, "opaque pixels");
    // Transparency locked: the transparent half stays transparent.
    let mut s = session(8, half);
    s.edit("lock", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
        Ok(())
    })
    .unwrap();
    let before = layer(&s);
    stamp(&mut s, json!({"mode": "behind"}));
    assert_eq!(layer(&s), before, "locked transparency");
    let live = super::super::LiveRetouch::begin(&s, "paint.patternStamp", &stroke(json!({"mode": "behind"}))).unwrap().doc.clone();
    assert_eq!(pixels(&live, &s), before, "the live stroke too");
    // The Background has no transparency.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H})).unwrap();
    let before = layer(&s);
    stamp(&mut s, json!({"mode": "behind"}));
    assert_eq!(layer(&s), before, "the Background");
}

/// Behind with Impressionist was not measured: the stroke gives the pixels it gives in Normal,
/// committed and live, on a half-filled layer and with transparency locked.
#[test]
fn impressionist_paints_as_in_normal_when_behind_is_picked() {
    let all = [[12.0, 24.0], [30.0, 20.0], [40.0, 28.0], [52.0, 24.0]];
    let opts = |v: &[[f64; 2]], mode: &str| json!({"points": v, "size": 14, "hardness": 40, "opacity": 70, "pattern": "Bricks", "impressionist": true, "mode": mode, "seed": 11});
    for depth in [8, 16] {
        for lock in [false, true] {
            let run = |mode: &str| {
                let mut s = session(depth, half);
                if lock {
                    s.edit("lock", |doc, active| {
                        doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
                        Ok(())
                    })
                    .unwrap();
                }
                let before = layer(&s);
                let mut live = live_impressionist(&s, &opts(&all[..1], mode)).unwrap();
                live.push(&all[1..].iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect::<Vec<_>>()).unwrap();
                let shown = pixels(&live.doc, &s);
                s.execute("paint.patternStamp", opts(&all, mode)).unwrap();
                (before, shown, layer(&s))
            };
            let (before, live_behind, behind) = run("behind");
            let (_, live_normal, normal) = run("normal");
            let tag = format!("{depth}-bit lock {lock}");
            assert_eq!(behind, normal, "{tag}: the committed stroke");
            assert_eq!(live_behind, live_normal, "{tag}: the live stroke");
            assert_eq!(live_behind, behind, "{tag}: live is the commit");
            // As in Normal, the dabs go over the filled half.
            assert!((0..W / 2).any(|x| at(&behind, x, 24) != at(&before, x, 24)), "{tag}: painted over the pixels");
        }
    }
}

/// The live aligned stroke, fed point by point, shows what the commit paints.
#[test]
fn the_live_behind_stroke_matches_the_commit() {
    let all = [[12.0, 24.0], [30.0, 20.0], [40.0, 28.0], [52.0, 24.0]];
    let pts = |v: &[[f64; 2]]| v.iter().map(|q| StrokePoint::new(q[0], q[1], 1.0)).collect::<Vec<_>>();
    for depth in [8, 16] {
        for aligned in [true, false] {
            let mut s = session(depth, |x, y| if x < W / 2 { RED } else { [0.2, 0.4, 0.9, 0.25 * ((x + y) % 4) as f32] });
            let before = layer(&s);
            let opts = |v: &[[f64; 2]]| {
                json!({"points": v, "size": 14, "hardness": 40, "opacity": 70, "pattern": "Checkerboard", "scale": 150, "angle": 20,
                       "aligned": aligned, "mode": "behind", "seed": 11})
            };
            let shown = {
                let mut live = super::super::LiveRetouch::begin(&s, "paint.patternStamp", &opts(&all[..1])).unwrap();
                live.push(&pts(&all[1..3])).unwrap();
                live.push(&pts(&all[3..])).unwrap();
                live.doc.clone()
            };
            s.execute("paint.patternStamp", opts(&all)).unwrap();
            let tag = format!("{depth}-bit aligned {aligned}");
            let committed = layer(&s);
            assert_eq!(pixels(&shown, &s), committed, "{tag}");
            assert_ne!(committed, before, "{tag}: the stroke painted");
            assert!((0..H).all(|y| (0..W / 2).all(|x| at(&committed, x, y) == at(&before, x, y))), "{tag}: opaque pixels kept");
        }
    }
}

/// Without `"mode": "behind"` the stroke is the blend-mode stroke it was: over the pixels.
#[test]
fn other_modes_still_paint_over_the_pixels() {
    let run = |extra: Value| {
        let mut s = session(8, half);
        stamp(&mut s, extra);
        layer(&s)
    };
    let (plain, normal, multiply, behind) = (run(json!({})), run(json!({"mode": "normal"})), run(json!({"mode": "multiply"})), run(json!({"mode": "behind"})));
    assert_eq!(plain, normal, "Normal is the default");
    // Normal replaces the red with the opaque pattern; Multiply darkens it; Behind keeps it.
    let want = if (20i32.div_euclid(8) + 24i32.div_euclid(8)).rem_euclid(2) == 0 { 0.8 } else { 1.0 };
    let p = at(&plain, 20, 24);
    assert!((0..3).all(|c| (p[c] - want).abs() < 0.01) && p[3] > 0.99, "{p:?}");
    assert_ne!(at(&multiply, 20, 24), at(&plain, 20, 24));
    assert_eq!(at(&behind, 20, 24), at(&layer(&session(8, half)), 20, 24));
    // On transparency all three lay the same pattern.
    assert_eq!(at(&plain, 44, 24), at(&behind, 44, 24));
    // An unknown mode is still refused, and paints nothing.
    let mut s = session(8, half);
    let (before, n) = (layer(&s), steps(&s));
    assert!(s.execute("paint.patternStamp", stroke(json!({"mode": "beneath"}))).is_err());
    assert_eq!((layer(&s), steps(&s)), (before, n));
}

/// Inside a selection only: the selection weighs Behind as it weighs the other modes.
#[test]
fn behind_respects_the_selection() {
    let mut s = session(8, empty);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": H})).unwrap();
    stamp(&mut s, json!({"mode": "behind"}));
    let px = layer(&s);
    assert!(at(&px, 36, 24)[3] > 0.99, "{:?}", at(&px, 36, 24));
    assert_eq!(at(&px, 46, 24), [0.0; 4], "outside the selection");
}
