//! Photomerge stacks the result layers in source order from the top: the first source is the
//! top layer and the last source the bottom one (#14092).

use super::*;

/// A deterministic textured RGB image: a gradient with scattered squares.
fn scene(w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut z = 7u64;
    let mut rnd = || {
        z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (z >> 33) as f64 / (1u64 << 31) as f64
    };
    let mut px: Vec<[f32; 4]> = (0..w * h).map(|i| [0.3 + 0.2 * (i % w) as f32 / w as f32, 0.35, 0.4, 1.0]).collect();
    for _ in 0..(w * h / 600) {
        let (cx, cy, r) = ((rnd() * w as f64) as usize, (rnd() * h as f64) as usize, 2 + (rnd() * 8.0) as usize);
        let col = [rnd() as f32, rnd() as f32, rnd() as f32, 1.0];
        for y in cy.saturating_sub(r)..(cy + r).min(h) {
            for x in cx.saturating_sub(r)..(cx + r).min(w) {
                px[y * w + x] = col;
            }
        }
    }
    px
}

/// A session with three 300x300 tiles cut at x 0, 200 and 400 from one 700x300 image, opened
/// in the order t1.tif, t2.tif, t3.tif.
fn three_tiles() -> Session {
    let (sw, sh, t) = (700usize, 300usize, 300usize);
    let sc = scene(sw, sh);
    let mut s = Session::new();
    for (k, x0) in [0usize, 200, 400].into_iter().enumerate() {
        let mut d = Document::new(format!("t{}.tif", k + 1), Size::new(t as u32, t as u32), ColorMode::Rgb, SampleType::U8);
        let fmt = d.pixel_format();
        let n = fmt.channels();
        let mut data = vec![0.0f32; t * t * n];
        for (i, o) in data.chunks_exact_mut(n).enumerate() {
            photocraft_raster::from_rgba_into(&fmt, sc[(i / t) * sw + x0 + i % t], o);
        }
        let mut surf = Surface::new(fmt);
        surf.write_region(Rect::new(0, 0, t as i32, t as i32), &data);
        d.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
        s.add_document(d, None);
    }
    s
}

/// Layer names from the top of the Layers panel down.
fn top_down(d: &Document) -> Vec<&str> {
    d.layers.iter().rev().map(|l| l.name.as_str()).collect()
}

#[test]
fn first_source_is_the_top_layer() {
    for blend in [true, false] {
        let mut s = three_tiles();
        let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "blend": blend})).unwrap();
        assert_eq!(r["placed"].as_array().unwrap().len(), 3, "{r}");
        let d = &s.active().unwrap().doc;
        assert_eq!(top_down(d), ["t1.tif", "t2.tif", "t3.tif"], "blend {blend}");
    }
}

#[test]
fn fill_layer_stays_above_the_sources() {
    let mut s = three_tiles();
    s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition", "contentAwareFill": true})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(top_down(d), ["Content-Aware Fill", "t1.tif", "t2.tif", "t3.tif"]);
}

#[test]
fn blended_composite_does_not_depend_on_the_stacking() {
    let mut s = three_tiles();
    s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition"})).unwrap();
    let d: Document = (*s.active().unwrap().doc).clone();
    let mut flipped = d.clone();
    flipped.layers.reverse();
    // The seam masks split the canvas, so each pixel shows one layer whatever the order.
    assert_eq!(photocraft_compose::flatten(&d).px, photocraft_compose::flatten(&flipped).px);
}
