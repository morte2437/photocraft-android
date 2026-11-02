//! Opt-in release comparison; full-canvas buffers belong on a CI runner.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Layer, LayerContent, Size};
use std::time::Instant;

fn reference(s: &mut Session, p: &Value) {
    let (canvas, px) = sample_pixels(s, true, None).unwrap();
    // Previous path: eyedropper colour came from the full composite already in memory.
    let [r, g, b, _] = px[1500 * canvas.width() as usize + 2000];
    let point = sel::RangeSample { color: [r, g, b], at: Some((2000.0, 1500.0)) };
    let query = ColorRangeSamples::with_samples(s, p, canvas, &[point], &[]).unwrap();
    let mut mask = query.coverage(&px, canvas.width() as usize, 1.0, [0.0, 0.0]);
    drop(px);
    scope_color_range(&mut mask, &s.active().unwrap().doc, canvas, SelectionMode::Replace);
    set_selection(s, "Color Range", canvas, mask, SelectionMode::Replace).unwrap();
}

#[test]
#[ignore = "24 MP release comparison: run on GitHub Actions"]
fn color_range_release_comparison() {
    let mut doc = Document::new("Color Range performance", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Surface::with_default(PixelFormat::RGBA8, &[0.65, 0.3, 0.2, 1.0]);
    pixels.fill_rect(Rect::new(3000, 2000, 3600, 2400), &[0.2, 0.3, 0.65, 1.0]);
    doc.layers.push(Layer::new("image", LayerContent::Raster(pixels)));
    let p = json!({"points": [[2000, 1500]], "fuzziness": 40, "localized": true, "range": 80});
    // Small, 1%, 4%, and full selections: include dense fallback cost in the comparison.
    for bounds in [Rect::new(2975, 1975, 3025, 2025), Rect::new(2700, 1800, 3300, 2200), Rect::new(2400, 1600, 3600, 2400), doc.bounds()] {
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(bounds, &[128.0 / 255.0]);
        doc.selection = Some(selection);
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        let mut expected = None;
        // One warmup pair, followed by three alternating measured pairs.
        for round in 0..4 {
            for old in if round % 2 == 0 { [true, false] } else { [false, true] } {
                let mut session = Session::new();
                session.add_document(doc.clone(), None);
                let now = Instant::now();
                if old {
                    reference(&mut session, &p);
                } else {
                    session.execute("select.colorRange", p.clone()).unwrap();
                }
                let elapsed = now.elapsed().as_secs_f64() * 1000.0;
                if round > 0 {
                    if old { old_times.push(elapsed) } else { new_times.push(elapsed) }
                }
                // Outside timing, compare every stored sample over the whole canvas.
                let actual = sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), doc.bounds());
                if let Some(ref expected) = expected {
                    assert_eq!(&actual, expected);
                } else {
                    expected = Some(actual);
                }
            }
        }
        old_times.sort_by(f64::total_cmp);
        new_times.sort_by(f64::total_cmp);
        let (old, new) = (old_times[1], new_times[1]);
        println!(
            "24 MP Color Range {}x{}: full_canvas={old:.3}ms cropped={new:.3}ms speedup={:.2}x rgba_bytes={} exact=true",
            bounds.width(),
            bounds.height(),
            old / new,
            bounds.size().area() * 16
        );
    }
}
