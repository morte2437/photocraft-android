//! Final-render coverage for #7989, extending the boundary fix in #8012.
use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, apply_liquify};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn bounds() -> Rect {
    // Cross the tile boundary at x=256, with a nonzero origin on both axes.
    Rect::new(192, -32, 320, 96)
}

fn source(mode: ColorMode, sample: SampleType) -> Surface {
    let format = PixelFormat::new(mode, sample, true);
    let mut pixels = Vec::with_capacity(128 * 128 * format.channels());
    for y in 0..128 {
        for x in 0..128 {
            let alpha = if (48..52).contains(&x) { 0.0 } else { ((x + 3 * y) % 5 + 1) as f32 / 5.0 };
            for channel in 0..mode.color_channels() {
                pixels.push(if alpha == 0.0 { 0.0 } else { ((x * 19 + y * 7 + channel * 43) % 251 + 2) as f32 / 255.0 });
            }
            pixels.push(alpha);
        }
    }
    let mut source = Surface::new(format);
    source.write_region(bounds(), &pixels);
    source
}

fn warp(points: &[[f64; 2]], size: f64, pressure: f64) -> LiquifyStroke {
    let mut stroke = LiquifyStroke::new(LiquifyTool::ForwardWarp, size);
    stroke.pressure = pressure;
    stroke.points = points.iter().map(|point| point.to_vec()).collect();
    stroke
}

fn freeze_left() -> LiquifyStroke {
    let mut stroke = LiquifyStroke::new(LiquifyTool::LassoMask, 1.0);
    stroke.amount = Some(1.0);
    stroke.points = vec![vec![192.0, -32.0], vec![256.0, -32.0], vec![256.0, 96.0], vec![192.0, 96.0]];
    stroke
}

fn check_frozen_output(prior_warp: bool) {
    for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let source = source(mode, sample);
            for cell in [2.5, 4.0] {
                let mut strokes = Vec::new();
                if prior_warp {
                    strokes.push(warp(&[[238.0, 20.0], [261.0, 28.0]], 64.0, 55.0));
                }
                strokes.push(freeze_left());
                let mut field = LiquifyField::from_strokes(bounds(), cell, &strokes);
                // #8012 establishes the bilinear mapping when the mask is applied.
                // This test checks later edits, not exact pre-mask cubic preservation.
                let before = apply_liquify(&source, &field);
                assert_eq!(before.format(), source.format());
                assert_eq!(before == source, !prior_warp, "the fixture must retain an earlier warp and keep a fresh mask-only operation unchanged");
                strokes.push(warp(&[[268.0, 32.0], [252.0, 32.0]], 48.0, 80.0));
                field.apply_stroke(&strokes[strokes.len() - 1]);
                let after = apply_liquify(&source, &field);
                assert_eq!(after.format(), source.format());

                let (mut protected, mut fractional_alpha, mut unfrozen_changes) = (0, 0, 0);
                let mut frozen_warped = 0;
                for y in bounds().y0..bounds().y1 {
                    for x in bounds().x0..bounds().x1 {
                        let pixel = before.pixel(x, y);
                        if field.freeze_at(f64::from(x) + 0.5, f64::from(y) + 0.5) == 1.0 {
                            protected += 1;
                            frozen_warped += usize::from(pixel != source.pixel(x, y));
                            let alpha = pixel[mode.color_channels()];
                            fractional_alpha += usize::from(alpha > 0.0 && alpha < 1.0);
                            // An unchanged F32 pixel can round on premultiply/unpremultiply.
                            // Integer-depth pixels must remain exactly equal.
                            let tolerance = if sample == SampleType::F32 { 4.0 * f32::EPSILON } else { 0.0 };
                            for (channel, (actual, expected)) in after.pixel(x, y).iter().zip(&pixel).enumerate() {
                                assert!(
                                    (actual - expected).abs() <= tolerance,
                                    "{mode:?}/{sample:?}, mesh {cell}, prior warp {prior_warp}: frozen channel {channel} changed at {x},{y}: {expected} -> {actual}"
                                );
                            }
                        } else if field.freeze_at(f64::from(x) + 0.5, f64::from(y) + 0.5) == 0.0 {
                            unfrozen_changes += usize::from(after.pixel(x, y) != pixel);
                        }
                    }
                }
                assert!(protected > 7_000 && fractional_alpha > 3_000, "the fixture must cover frozen pixels with fractional alpha");
                assert_eq!(frozen_warped > 0, prior_warp, "the fixture must retain the earlier warp inside the fully frozen region");
                assert!(unfrozen_changes > 0, "the adjacent unfrozen region must still move");
                let replay = LiquifyField::from_strokes(bounds(), cell, &strokes);
                assert!(field == replay, "stroke replay must reproduce the field exactly");
                assert!(apply_liquify(&source, &replay) == after, "stroke replay must reproduce the final pixels exactly");
            }
        }
    }
}

#[test]
fn fresh_frozen_output_preserves_color_and_alpha_across_formats() {
    check_frozen_output(false);
}

#[test]
fn existing_frozen_mapping_survives_adjacent_warps_across_formats() {
    check_frozen_output(true);
}
