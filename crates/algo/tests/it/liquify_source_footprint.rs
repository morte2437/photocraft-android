//! Liquify source reads must depend on the mapping, not the output tile phase.
use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, apply_liquify};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// Keeps the document size fixed while moving its edges relative to global tiles.
fn bounds(shift: i32) -> Rect {
    Rect::new(shift, 0, shift + 1024, 512)
}

/// Builds translated copies of RGB ramps and alternating columns so missing
/// interpolation taps change the sampled colors.
fn source(sample: SampleType, shift: i32) -> Surface {
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, false));
    let mut row = vec![0.0; 1024 * 3];
    for y in 0..512 {
        for (x, pixel) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            *pixel = [x as f32 / 1024.0, (x % 2) as f32, y as f32 / 512.0];
        }
        surface.write_region(Rect::new(shift, y, shift + 1024, y + 1), &row);
    }
    surface
}

/// Creates matching brush paths in document-relative coordinates, optionally
/// beside a frozen region that gives the displacement field a sharp boundary.
fn strokes(shift: i32, masked: bool) -> Vec<LiquifyStroke> {
    let offset = f64::from(shift);
    let mut strokes = Vec::new();
    if masked {
        let mut freeze = LiquifyStroke::new(LiquifyTool::LassoMask, 1.0);
        freeze.amount = Some(1.0);
        freeze.points = vec![vec![offset, 0.0], vec![offset + 508.0, 0.0], vec![offset + 508.0, 512.0], vec![offset, 512.0]];
        strokes.push(freeze);
    }
    let mut warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 512.0);
    warp.density = 100.0;
    warp.points = vec![vec![offset + 512.0, 256.5], vec![offset + 499.2, 256.5]];
    strokes.extend(std::iter::repeat_n(warp, 8));
    for stroke in &strokes {
        stroke.validate().expect("the fixture uses supported brush parameters");
    }
    strokes
}

/// Checks that a half-tile translation preserves editable output across sample
/// depths and mesh sizes when image content and brush paths move together.
fn check_tile_translation(masked: bool) {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let original = source(sample, 0);
        let translated = source(sample, 128);
        for cell in [2.0, 4.0] {
            let field = LiquifyField::from_strokes(bounds(0), cell, &strokes(0, masked));
            let shifted_field = LiquifyField::from_strokes(bounds(128), cell, &strokes(128, masked));
            if masked {
                assert!(field.d == shifted_field.d, "translation must preserve the nodal displacement values");
            }
            // The reported pixels lie entirely on the editable side of the mask.
            assert_eq!(field.freeze_at(509.5, 220.5), 0.0);
            assert_eq!(shifted_field.freeze_at(637.5, 220.5), 0.0);
            let output = apply_liquify(&original, &field);
            let shifted_output = apply_liquify(&translated, &shifted_field);
            assert_ne!(output.pixel(509, 220), original.pixel(509, 220), "the fixture must actually warp the inspected strip");
            let mut worst = 0.0f32;
            // Translation changes which output tile contains this strip, while
            // preserving the relative source image, brush path and document edges.
            for y in 220..290 {
                for x in 509..512 {
                    let a = output.pixel(x, y);
                    let b = shifted_output.pixel(x + 128, y);
                    for (a, b) in a.iter().zip(&b) {
                        worst = worst.max((a - b).abs());
                    }
                }
            }
            // Coordinate translation can perturb the last few F32 arithmetic bits.
            // This is far smaller than the clipped source-stencil error.
            let tolerance = if sample == SampleType::F32 { 2e-6 } else { 0.0 };
            assert!(worst <= tolerance, "{sample:?}, meshSize {cell}, masked={masked}: moving the document by half a tile changed output by {worst}");
        }
    }
}

#[test]
fn cubic_source_footprint_is_independent_of_output_tile_phase() {
    check_tile_translation(true);
}

#[test]
fn ordinary_unmasked_warp_is_independent_of_output_tile_phase() {
    check_tile_translation(false);
}

#[test]
fn source_reads_still_clamp_at_the_document_edge() {
    let mut warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 256.0);
    warp.density = 100.0;
    warp.points = vec![vec![1008.0, 256.5], vec![944.0, 256.5]];
    warp.validate().expect("the edge fixture uses supported brush parameters");
    let field = LiquifyField::from_strokes(bounds(0), 4.0, &[warp]);
    assert!(field.sample(1023.5, 256.5)[0] > 4.0, "the fixture must sample beyond the right document edge");
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let original = source(sample, 0);
        let output = apply_liquify(&original, &field);
        assert_eq!(output.pixel(1023, 256), original.pixel(1023, 256), "{sample:?}: an out-of-document source read must repeat the true document edge");
    }
}
