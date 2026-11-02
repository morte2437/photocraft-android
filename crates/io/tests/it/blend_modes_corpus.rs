//! Blend modes against Photoshop's own merged image of psd-tools `blend-modes/rgb-blend-modes.psd`:
//! one 64 × 64 cell per mode, a duck over a rainbow backdrop.
#![cfg(feature = "corpus")]

use photocraft_color::BlendMode;
use photocraft_geom::Rect;
use photocraft_io::{import, merged_composite};
use photocraft_psd::PsdFile;

const CELL: i32 = 64;

/// The largest and mean difference in 8-bit levels between our flatten and Photoshop's merged image
/// over the cell of `mode`, on the pixels where that cell's duck is fully opaque (its anti-aliased
/// edge mixes in the backdrop, which says nothing about the mode).
fn cell_error(mode: BlendMode) -> (f32, f32) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd-tools/blend-modes/rgb-blend-modes.psd");
    let bytes = std::fs::read(path).expect("run cargo xtask corpus --psd-tools");
    let doc = import("rgb-blend-modes.psd", &bytes).expect("opens").document;
    let file = PsdFile::from_bytes(&bytes).expect("parses");
    let merged = merged_composite(&file).expect("merged image");
    let ours = photocraft_compose::flatten(&doc).px;
    let width = doc.size.width as i32;
    let duck = doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "rubber_duck_PNG1" && l.blend == mode).expect("the mode's duck");
    let surface = duck.surface().expect("duck pixels");
    let bounds = surface.content_bounds();
    let (x0, y0) = ((bounds.x0 / CELL) * CELL, (bounds.y0 / CELL) * CELL);
    let mut alpha = vec![[0.0f32; 4]; (CELL * CELL) as usize];
    surface.read_rgba_into(Rect::from_xywh(x0, y0, CELL as u32, CELL as u32), &mut alpha);
    let (mut max, mut sum, mut n) = (0.0f32, 0.0f32, 0u32);
    for y in 1..CELL - 1 {
        for x in 1..CELL - 1 {
            // Opaque, and so are its four neighbours (one pixel in from the edge).
            let opaque = |dx: i32, dy: i32| alpha[((y + dy) * CELL + x + dx) as usize][3] >= 1.0;
            if !(opaque(0, 0) && opaque(-1, 0) && opaque(1, 0) && opaque(0, -1) && opaque(0, 1)) {
                continue;
            }
            let i = ((y0 + y) * width + x0 + x) as usize;
            for c in 0..3 {
                let d = ((ours[i][c] * 255.0).round() - (merged[i][c] * 255.0).round()).abs();
                max = max.max(d);
                sum += d;
                n += 1;
            }
        }
    }
    assert!(n > 3000, "{mode:?}: only {n} opaque samples");
    (max, sum / n as f32)
}

/// #8804: Soft Light over the dark parts of the rainbow. With `sqrt(cb)` below a quarter the cell
/// was up to 20 levels too light; the W3C curve Photoshop uses is within 7.
#[test]
fn soft_light_matches_photoshop_over_dark_backdrops() {
    let (max, mean) = cell_error(BlendMode::SoftLight);
    eprintln!("Soft Light: max {max} mean {mean:.3}");
    assert!(max <= 7.0 && mean < 0.6, "max {max} mean {mean}");
}

/// The modes #8804 checked next to Soft Light stay where they were.
#[test]
fn separable_light_modes_match_photoshop() {
    for mode in [BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay, BlendMode::HardLight] {
        let (max, mean) = cell_error(mode);
        eprintln!("{mode:?}: max {max} mean {mean:.3}");
        assert!(max <= 3.0, "{mode:?}: max {max} mean {mean}");
    }
}
