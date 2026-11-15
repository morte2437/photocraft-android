//! #13809: Edit › Fade in Dissolve picks, per pixel, the pixel before the step or the step's
//! result, never a mix of the two; the opacity is the odds of the result.

use super::*;

const W: i32 = 400;
const H: i32 = 300;

fn bounds() -> Rect {
    Rect::new(0, 0, W, H)
}

/// The issue's test document: white Background plus an opaque layer `Paint` of four blocks and a
/// yellow square.
fn blocks_session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.edit("paint", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 200, 150), &[1.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(200, 0, 400, 150), &[0.0, 128.0 / 255.0, 1.0, 1.0]);
        surf.fill_rect(Rect::new(0, 150, 200, 300), &[64.0 / 255.0, 200.0 / 255.0, 64.0 / 255.0, 1.0]);
        surf.fill_rect(Rect::new(200, 150, 400, 300), &[128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
        surf.fill_rect(Rect::new(50, 50, 150, 150), &[1.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// The active layer's pixels over the canvas, one `channels`-long run per pixel.
fn layer_pixels(doc: &Document, s: &Session) -> (Vec<f32>, usize) {
    let surf = doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
    let mut out = vec![0.0; (W * H) as usize * surf.channels()];
    surf.read_region_into(bounds(), &mut out);
    (out, surf.channels())
}

fn active_pixels(s: &Session) -> (Vec<f32>, usize) {
    layer_pixels(&s.active().unwrap().doc, s)
}

/// How many pixels are exactly the step's result; panics on a pixel that is neither the base
/// nor the result.
fn count_picks(base: &[f32], top: &[f32], faded: &[f32], n: usize, what: &str) -> usize {
    let mut picked = 0;
    for (i, ((b, t), f)) in base.chunks_exact(n).zip(top.chunks_exact(n)).zip(faded.chunks_exact(n)).enumerate() {
        if f == t {
            picked += 1;
        } else {
            assert_eq!(f, b, "{what}: pixel ({}, {}) is a mix of {b:?} and {t:?}", i % W as usize, i / W as usize);
        }
    }
    picked
}

#[test]
fn dissolve_picks_the_base_or_the_inverted_pixel_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = blocks_session(depth);
        let (base, n) = active_pixels(&s);
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let (top, _) = active_pixels(&s);
        s.execute("edit.fade", json!({"opacity": 37, "mode": "dissolve"})).unwrap();
        let (faded, _) = active_pixels(&s);
        let picked = count_picks(&base, &top, &faded, n, &format!("depth {depth}"));
        let share = picked as f64 / f64::from(W * H);
        assert!((share - 0.37).abs() <= 0.03, "depth {depth}: {:.1} % of the pixels took the inverted value", share * 100.0);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Fade Invert"));
    }
}

#[test]
fn dissolve_at_100_is_normal_at_100_and_at_0_leaves_the_base() {
    for depth in [8, 16, 32] {
        let faded = |opacity: u32, mode: &str| {
            let mut s = blocks_session(depth);
            let (base, _) = active_pixels(&s);
            s.execute("image.adjustments.invert", json!({})).unwrap();
            let (top, _) = active_pixels(&s);
            s.execute("edit.fade", json!({"opacity": opacity, "mode": mode})).unwrap();
            (base, top, active_pixels(&s).0)
        };
        let (_, top, dissolve_full) = faded(100, "dissolve");
        assert!(dissolve_full == top, "depth {depth}: Dissolve 100 % is the step's result");
        assert!(dissolve_full == faded(100, "normal").2, "depth {depth}: Dissolve 100 % equals Normal 100 %");
        let (base, _, dissolve_none) = faded(0, "dissolve");
        assert!(dissolve_none == base, "depth {depth}: Dissolve 0 % leaves the base");
    }
}

/// A blurred red square on a transparent layer: each pixel takes the blurred value, alpha
/// included, or keeps the one before the blur, so the soft edge never becomes half as opaque.
#[test]
fn dissolve_picks_alpha_with_the_colour() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "T"})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(150, 100, 250, 200), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let (base, n) = active_pixels(&s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 10})).unwrap();
    let (top, _) = active_pixels(&s);
    s.execute("edit.fade", json!({"opacity": 50, "mode": "dissolve"})).unwrap();
    let (faded, _) = active_pixels(&s);
    count_picks(&base, &top, &faded, n, "blurred square");
    // Along y = 150, outside the square's left edge: alpha 0 or the blur's, both seen.
    let alpha = |px: &[f32], x: usize| px[(150 * W as usize + x) * n + n - 1];
    let row: Vec<f32> = (130..150).map(|x| alpha(&faded, x)).collect();
    assert!(row.iter().zip(130..150).all(|(&a, x)| a == 0.0 || a == alpha(&top, x)), "{row:?}");
    assert!(row.contains(&0.0) && row.iter().any(|&a| a > 0.0), "both picks along the edge: {row:?}");
}

/// The pick depends only on the pixel's position, so the dialog's preview is what OK applies
/// and the same params give the same pattern again.
#[test]
fn dissolve_preview_is_what_ok_applies() {
    let mut s = blocks_session(8);
    s.execute("image.adjustments.invert", json!({})).unwrap();
    let p = json!({"opacity": 37, "mode": "dissolve"});
    let preview = fade_preview(&s, &p).and_then(FadePreview::render).unwrap();
    let again = fade_preview(&s, &p).and_then(FadePreview::render).unwrap();
    assert!(layer_pixels(&preview, &s).0 == layer_pixels(&again, &s).0, "the same params give the same pattern");
    s.execute("edit.fade", p).unwrap();
    assert!(layer_pixels(&preview, &s).0 == active_pixels(&s).0, "the preview is what OK applies");
}
