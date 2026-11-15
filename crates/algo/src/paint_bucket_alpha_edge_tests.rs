//! Paint Bucket edge blend with Anti-alias on (#12528), measured on the reference application.
//!
//! `paint::tests::bucket_on_transparency_blends_into_the_edge_at_its_transparency` already pins
//! the four edge values at Tolerance 32. These tests add the issue's two remaining measured
//! cases: Tolerance 0 gives the same edge as Tolerance 32, and an opaque seed's fringe on a grey
//! ramp keeps its values.

use photocraft_color::{PixelFormat, blend::BlendMode};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::paint::{FillMode, bucket_fill_inks};
use crate::selection::rgba8_image;

const GREEN: [u8; 4] = [0, 255, 0, 255];
/// Black edge alphas and the reference's result after a green fill from the transparent side.
const EDGE: [(u8, [u8; 4]); 4] = [(40, [0, 248, 0, 221]), (112, [0, 190, 0, 192]), (197, [0, 70, 0, 210]), (253, [0, 2, 0, 253])];

fn surface(img: &[[u8; 4]], area: Rect) -> Surface {
    let bytes: Vec<u8> = img.iter().flatten().copied().collect();
    Surface::from_interleaved(PixelFormat::RGBA8, area, &bytes)
}

/// Bucket-fills green at `seed` (Normal, Opacity 100, Contiguous) and returns the layer afterwards.
fn fill(img: &[[u8; 4]], area: Rect, seed: (i32, i32), tol: f32, anti_alias: bool) -> Vec<[u8; 4]> {
    let mut s = surface(img, area);
    let green = |_, _| [0.0, 1.0, 0.0, 1.0];
    assert!(bucket_fill_inks(&mut s, area, seed, tol, true, anti_alias, 1.0, None, false, green, None, FillMode::Blend(BlendMode::Normal)));
    rgba8_image(&s, area)
}

fn near(got: [u8; 4], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(g, w)| (*g as i32 - w as i32).abs() <= 2)
}

/// A transparent run (x 0..4), one black edge pixel of `alpha` (x 4), then opaque black (x 5..8).
fn edge_row(alpha: u8) -> Vec<[u8; 4]> {
    let mut img = vec![[0u8; 4]; 8];
    img[4] = [0, 0, 0, alpha];
    for p in &mut img[5..] {
        *p = [0, 0, 0, 255];
    }
    img
}

/// #12528: clicking transparency gives the same edge at Tolerance 0 as at Tolerance 32. With
/// Anti-alias on the black edge pixel takes the reference's value; with it off it stays. Either
/// way the transparent run fills solid green and the opaque black stays.
#[test]
fn transparent_seed_edge_is_the_same_at_tolerance_0_and_32() {
    let area = Rect::new(0, 0, 8, 1);
    for (alpha, reference) in EDGE {
        let img = edge_row(alpha);
        for tol in [0.0, 32.0] {
            for anti_alias in [true, false] {
                let after = fill(&img, area, (1, 0), tol, anti_alias);
                let label = format!("alpha {alpha}, tolerance {tol}, anti-alias {anti_alias}");
                if anti_alias {
                    assert!(near(after[4], reference), "{label}: edge {:?}, reference {reference:?}", after[4]);
                } else {
                    assert_eq!(after[4], img[4], "{label}: Anti-alias off leaves the edge");
                }
                for (x, p) in after.iter().enumerate().take(4) {
                    assert_eq!(*p, GREEN, "{label}: transparent pixel {x} fills");
                }
                for (x, p) in after.iter().enumerate().skip(5) {
                    assert_eq!(*p, img[x], "{label}: opaque pixel {x} stays");
                }
            }
        }
    }
}

/// #12528 opaque-seed comparison: on a grey ramp rising 1 per pixel, clicking 127 at Tolerance 32
/// with Anti-alias on fills 95..=159 pure green, blends 94 and 160 to about (1, 254, 1, 255), and
/// leaves 93 and 161 as they were.
#[test]
fn opaque_seed_fringe_on_a_ramp_keeps_its_values() {
    let img: Vec<[u8; 4]> = (0..=255u8).map(|v| [v, v, v, 255]).collect();
    let after = fill(&img, Rect::new(0, 0, 256, 1), (127, 0), 32.0, true);
    for (x, p) in after.iter().enumerate().take(160).skip(95) {
        assert_eq!(*p, GREEN, "ramp {x} fills");
    }
    for x in [94, 160] {
        assert_ne!(after[x], GREEN, "ramp {x} blends, not a full fill");
        assert!(near(after[x], [1, 254, 1, 255]), "ramp {x} blends: {:?}", after[x]);
    }
    for x in [93, 161] {
        assert_eq!(after[x], img[x], "ramp {x} stays");
    }
}
