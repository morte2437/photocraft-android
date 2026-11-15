//! The black arrow pointer: a filled black arrow with a white edge, PhotoCraft's own drawing from
//! one polygon (tip at the top left, body down to the right, a short tail), as a signed distance
//! field (negative inside) in logical pixels from the tip with y pointing down. The hotspot is the
//! tip; the bitmap is cropped to the arrow so the tip sits one pixel in from its top left corner,
//! at (1, 1) at 100 %. Windows' stock arrow is white, so every platform gets this bitmap.

use std::sync::Arc;

use egui::CustomCursorImage;

use super::hand::capsule;

/// How far the polygon reaches from the tip, outline excluded.
pub(super) const REACH: f32 = 20.0;

/// The arrow's corners, clockwise from the tip.
const OUTLINE: [[f32; 2]; 7] = [[0.0, 0.0], [11.5, 11.8], [6.7, 11.8], [9.2, 17.4], [6.6, 18.6], [4.0, 12.8], [0.0, 16.5]];

/// The arrow's silhouette: negative inside it.
pub(super) fn shape(x: f32, y: f32) -> f32 {
    let mut distance = f32::MAX;
    let mut inside = false;
    for (&a, &b) in OUTLINE.iter().zip(OUTLINE.iter().cycle().skip(1)) {
        distance = distance.min(capsule(x, y, a, b, 0.0));
        // Even-odd rule: a ray towards +x crossing this edge flips the answer.
        if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
            inside = !inside;
        }
    }
    if inside { -distance } else { distance }
}

/// One bitmap pixel in straight RGBA: black `fill` over a white edge, `outline` being the coverage
/// of the whole arrow (edge included), each 0 to 1. No coverage is `[0; 4]`.
pub(super) fn pixel((fill, outline): (f32, f32)) -> [u8; 4] {
    let edge = outline * (1.0 - fill);
    let alpha = fill + edge;
    let white = if alpha > 0.0 { (255.0 * edge / alpha).round() as u8 } else { 0 };
    [white, white, white, (255.0 * alpha).round() as u8]
}

/// Crop `image` (the arrow centred on its hotspot) to the pixels it covers, the hotspot moving
/// with them. `None` when nothing is covered.
pub(super) fn crop(image: CustomCursorImage) -> Option<CustomCursorImage> {
    let [w, h] = image.size.map(usize::from);
    let covered = |col: usize, row: usize| image.rgba.get((row * w + col) * 4 + 3).is_some_and(|&a| a > 0);
    let cols: Vec<usize> = (0..w).filter(|&c| (0..h).any(|r| covered(c, r))).collect();
    let rows: Vec<usize> = (0..h).filter(|&r| (0..w).any(|c| covered(c, r))).collect();
    let (&c0, &c1) = (cols.first()?, cols.last()?);
    let (&r0, &r1) = (rows.first()?, rows.last()?);
    let mut rgba = Vec::with_capacity((c1 - c0 + 1) * (r1 - r0 + 1) * 4);
    for row in r0..=r1 {
        rgba.extend_from_slice(image.rgba.get((row * w + c0) * 4..(row * w + c1 + 1) * 4)?);
    }
    Some(CustomCursorImage {
        rgba: Arc::from(rgba),
        size: [u16::try_from(c1 - c0 + 1).ok()?, u16::try_from(r1 - r0 + 1).ok()?],
        hotspot: [image.hotspot[0].checked_sub(u16::try_from(c0).ok()?)?, image.hotspot[1].checked_sub(u16::try_from(r0).ok()?)?],
    })
}
