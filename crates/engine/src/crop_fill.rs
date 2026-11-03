//! The Crop tool's Fill: Content-Aware Fill (#3823). After a rotated crop or one that extends past
//! the canvas, the area the original image doesn't cover (the corners a rotation uncovers, the
//! strips an extension adds) is filled from its surroundings instead of holding the background
//! colour or transparency.
//!
//! Which layer: the Background when the document has one (the layer the crop filled with the
//! background colour), otherwise the active pixel layer (where the crop left transparency). Other
//! layers keep their transparency there, as after a plain crop. Each connected piece of the
//! uncovered area (each corner, each edge strip) is filled on its own, with Photoshop's sampling
//! window around that piece, and only pixels the image covers are sampled.
//!
//! Pixels an earlier crop with Delete Cropped Pixels off hid past the canvas edges are still the
//! layer's. Under a turned frame they count as covered, so they are kept (and sampled) rather than
//! filled over; a straight crop fills all of the area past the old canvas, them included, as
//! Photoshop does (#8060).

use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use rayon::prelude::*;

use crate::{EngineError, Result};

/// Seed for the patch search, so a crop fills the same way every time.
const SEED: u64 = 1;

/// The largest canvas (pixels) the uncovered area is worked out for: the Content-Aware window cap.
const MAX_PIXELS: u64 = 400_000_000;

/// Where the cropped canvas came from in the original document.
#[derive(Clone, Debug)]
pub(crate) struct Coverage {
    /// Cropped canvas → original document: `[a, b, c, d, e, f]` with
    /// `x = a·u + b·v + e`, `y = c·u + d·v + f`.
    pub map: [f64; 6],
    /// The original canvas size (px).
    pub width: f64,
    pub height: f64,
    /// Whether the map rotates (its resampled edge is antialiased against the background colour).
    pub rotated: bool,
    /// The filled layer's pixels before a turned crop (original document px), set by
    /// [`Coverage::holding`]: where they lie past the old canvas they are hidden image, not
    /// uncovered area. A straight crop has none: Photoshop fills over hidden pixels there.
    pub held: Option<Surface>,
}

impl Coverage {
    /// An unrotated crop to `r` (original document px).
    pub fn crop(r: Rect, width: u32, height: u32) -> Self {
        Coverage { map: [1.0, 0.0, 0.0, 1.0, f64::from(r.x0), f64::from(r.y0)], width: f64::from(width), height: f64::from(height), rotated: false, held: None }
    }

    /// Takes the pixels of the layer the fill will go to from `doc` before a turned crop (the
    /// surface is shared, not copied), so hidden pixels past the old canvas count as covered.
    pub fn holding(mut self, doc: &Document, active: Option<LayerId>) -> Self {
        self.held = target_layer(doc, active).ok().and_then(|id| doc.layer(id)).and_then(|l| l.surface()).cloned();
        self
    }

    /// Whether the cropped canvas's pixel (u, v) is the original image's: its centre maps inside
    /// the original canvas by at least half the pixel's projected size (exact for an unrotated
    /// crop); a rotated crop also refills the one-pixel antialiased edge, which the resampling
    /// blended with the background colour.
    fn covered(&self, u: i32, v: i32, margin: f64) -> bool {
        let [a, b, c, d, e, f] = self.map;
        let (cu, cv) = (f64::from(u) + 0.5, f64::from(v) + 0.5);
        let (x, y) = (a * cu + b * cv + e, c * cu + d * cv + f);
        (x >= margin && y >= margin && x <= self.width - margin && y <= self.height - margin) || self.holds(x, y)
    }

    /// Whether the layer held an image pixel at original document point (x, y) past the canvas
    /// (#8060): mostly opaque there and one pixel around it too (the turned edge is blended with
    /// the background colour, as at the canvas edge). Without an alpha channel
    /// hidden pixels can't be told from empty ones: nothing is held.
    fn holds(&self, x: f64, y: f64) -> bool {
        let Some(surf) = &self.held else { return false };
        let format = surf.format();
        if !(format.alpha && x.is_finite() && y.is_finite()) {
            return false;
        }
        let a = surf.channels().saturating_sub(1);
        let at = |x: f64, y: f64| {
            let (fx, fy) = (x.floor(), y.floor());
            let limit = f64::from(i32::MAX);
            fx.abs() < limit && fy.abs() < limit && surf.sample_channel(fx as i32, fy as i32, a) > 0.5
        };
        const ROUND: [(f64, f64); 5] = [(0.0, 0.0), (-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)];
        ROUND.iter().all(|&(dx, dy)| at(x + dx, y + dy))
    }

    fn margin(&self) -> f64 {
        let [a, b, ..] = self.map;
        0.5 * (a.abs() + b.abs()) + if self.rotated { 1.0 } else { 0.0 }
    }
}

/// Labels of the uncovered pixels: 0 covered, `UNLABELLED` uncovered before labelling, else the
/// component number (1-based).
const UNLABELLED: u32 = u32::MAX;

/// Fills the area of the cropped `doc` that `cov` says the original image doesn't cover (see the
/// module docs). Nothing uncovered: nothing changes. Errors leave `doc` to be rolled back by the
/// caller's history step.
pub(crate) fn fill_uncovered(doc: &mut Document, active: Option<LayerId>, cov: &Coverage) -> Result<()> {
    let canvas = doc.bounds();
    let (w, h) = (canvas.width(), canvas.height());
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(EngineError::Other(format!("the {w} x {h} crop is too large for Content-Aware fill")));
    }
    let (w, h) = (w as usize, h as usize);
    let len = w.checked_mul(h).ok_or_else(|| EngineError::Other("the crop is too large for Content-Aware fill".into()))?;
    if len == 0 {
        return Ok(());
    }
    let margin = cov.margin();
    let mut labels = crate::allocation::filled(len, 0u32, "Content-Aware crop fill")?;
    labels.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, l) in row.iter_mut().enumerate() {
            if !cov.covered(x as i32, y as i32, margin) {
                *l = UNLABELLED;
            }
        }
    });
    if !labels.contains(&UNLABELLED) {
        return Ok(());
    }
    if !labels.contains(&0) {
        return Err(EngineError::Other("Content-Aware fill needs some of the image inside the crop to sample from".into()));
    }
    let id = target_layer(doc, active)?;
    let pieces = label_components(&mut labels, w, h)?;
    let surf = doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("Content-Aware fill needs a pixel layer".into()))?;
    let label = |x: i32, y: i32| -> u32 {
        if x < 0 || y < 0 {
            return 0;
        }
        (y as usize).checked_mul(w).and_then(|i| i.checked_add(x as usize)).and_then(|i| labels.get(i)).copied().unwrap_or(0)
    };
    let n = surf.format().channels().max(1);
    let filled: Vec<(Rect, Vec<f32>)> = pieces
        .par_iter()
        .enumerate()
        .map(|(k, bounds)| -> Result<(Rect, Vec<f32>)> {
            let k = k as u32 + 1;
            let (window, px) = crate::fill_cmds::content_aware_native(surf, *bounds, canvas, true, SEED, |x, y| label(x, y) == k, |x, y| label(x, y) == 0)?;
            // The piece's bounds, its pixels from the fill and the rest as they were.
            let mut out = surf.read_region(*bounds);
            let ww = window.width() as usize;
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    if label(x, y) != k || !window.contains(x, y) {
                        continue;
                    }
                    let src = ((y - window.y0) as usize * ww + (x - window.x0) as usize) * n;
                    let dst = ((y - bounds.y0) as usize * bounds.width() as usize + (x - bounds.x0) as usize) * n;
                    if let (Some(s), Some(d)) = (px.get(src..src + n), out.get_mut(dst..dst + n)) {
                        d.copy_from_slice(s);
                    }
                }
            }
            Ok((*bounds, out))
        })
        .collect::<Result<_>>()?;
    let surf = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or_else(|| EngineError::Other("Content-Aware fill needs a pixel layer".into()))?;
    for (r, px) in filled {
        surf.write_region(r, &px);
    }
    Ok(())
}

/// The Background, else the active pixel layer.
fn target_layer(doc: &Document, active: Option<LayerId>) -> Result<LayerId> {
    if let Some(bg) = doc.background() {
        return Ok(bg.id);
    }
    active
        .filter(|id| doc.layer(*id).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_))))
        .ok_or_else(|| EngineError::Other("Content-Aware crop fill needs a Background or an active pixel layer".into()))
}

/// Numbers the 8-connected pieces of `UNLABELLED` pixels 1, 2, … and returns each one's bounds.
fn label_components(labels: &mut [u32], w: usize, h: usize) -> Result<Vec<Rect>> {
    let mut pieces: Vec<Rect> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..labels.len() {
        if labels.get(start) != Some(&UNLABELLED) {
            continue;
        }
        let k = u32::try_from(pieces.len() + 1).ok().filter(|k| *k < UNLABELLED).ok_or_else(|| EngineError::Other("too many uncovered areas".into()))?;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        if let Some(l) = labels.get_mut(start) {
            *l = k;
        }
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if (dx, dy) == (0, 0) || nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if let Some(l) = labels.get_mut(j)
                        && *l == UNLABELLED
                    {
                        *l = k;
                        stack.push(j);
                    }
                }
            }
        }
        pieces.push(Rect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1));
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_8_connected_with_their_bounds() {
        let (w, h) = (6, 4);
        let u = UNLABELLED;
        #[rustfmt::skip]
        let mut l = vec![
            u, 0, 0, 0, 0, u,
            0, u, 0, 0, 0, u,
            0, 0, 0, 0, 0, 0,
            0, 0, 0, u, u, 0,
        ];
        let pieces = label_components(&mut l, w, h).unwrap();
        assert_eq!(pieces, vec![Rect::new(0, 0, 2, 2), Rect::new(5, 0, 6, 2), Rect::new(3, 3, 5, 4)]);
        assert_eq!(l[7], 1);
        assert_eq!(l[11], 2);
        assert_eq!(l[22], 3);
    }

    #[test]
    fn unrotated_coverage_is_exact() {
        let c = Coverage::crop(Rect::new(-2, 0, 8, 4), 5, 4);
        let m = c.margin();
        assert!(!c.covered(1, 0, m), "x = -1");
        assert!(c.covered(2, 0, m), "x = 0");
        assert!(c.covered(6, 0, m), "x = 4");
        assert!(!c.covered(7, 0, m), "x = 5");
    }
}
