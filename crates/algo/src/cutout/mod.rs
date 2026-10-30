//! Cutout (Filter Gallery › Artistic): the image as flat sheets of cut paper.
//!
//! The image is traced into shapes, and the shapes are filled back in:
//!
//! 1. **Smooth** within tones, edges kept: a bilateral filter (C. Tomasi and R. Manduchi,
//!    "Bilateral Filtering for Gray and Color Images", 1998) in Oklab, so a photo's noise doesn't
//!    decide pixel by pixel which of two colours a pixel gets.
//! 2. **Palette**: median cut (Heckbert 1982) then k-means over the smoothed colours, Number of
//!    Levels tones of the picture's own; a tone made of two clearly different hues splits in two,
//!    and colours only noise apart are merged.
//! 3. **Regions**: 4-connected areas of one palette colour. Regions smaller than the Edge
//!    Simplicity area, or thin slivers (the blends along an edge between two other colours), are
//!    merged smallest first into the neighbour they share most border with. Each region is filled
//!    with the mean colour of its own pixels.
//! 4. **Edges**: the cracks between regions are linked into chains from junction to junction
//!    (where three regions meet, or the canvas edge). Each crack is moved along its normal to where
//!    the pixels on either side say the edge is (an anti-aliased pixel is part one colour, part the
//!    other), so outlines are right to a fraction of a pixel. A chain is fitted once, so two
//!    neighbouring shapes share one and the same edge.
//! 5. **Curves**: turns sharper than Edge Fidelity allows are kept as corners (squared back where
//!    the pixels rounded them); the runs between are simplified to the Edge Simplicity tolerance
//!    (Douglas–Peucker) and drawn as Catmull-Rom curves.
//! 6. **Fill**: each region is scan-converted with exact area coverage, so edges are anti-aliased
//!    and the coverages of shapes that share an edge sum to one: no seams.
//!
//! Tracing reads the whole image and runs once; filling runs per tile and gives the same pixels
//! whatever the tiling.
//!
//! Each stage is its own module: [`colour`] (steps 1–2), [`regions`] (3), [`edges`] (4),
//! [`curves`] (5) and [`fill`] (6); [`trace`] runs them in turn.

use photocraft_geom::Rect;

use crate::par;

mod colour;
mod curves;
mod edges;
mod fill;
mod regions;
mod util;

use colour::{nearest, oklab, palette, smooth};
use curves::{bounds, fit};
use edges::follow_edges;
use fill::line_clipped;
use regions::{components, local_paper, merge_regions, paper_colours};

type Px = [f32; 3];
type Pt = [f32; 2];

/// Region id of the area outside the image.
const OUT: u32 = u32::MAX;

/// The filter's three sliders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    /// Number of Levels (2–8): the most colours the palette has.
    pub levels: usize,
    /// Edge Simplicity (0–10): smaller shapes merge and outlines simplify more.
    pub simplicity: f32,
    /// Edge Fidelity (1–3): more corners are kept.
    pub fidelity: f32,
}

impl Settings {
    /// Regions with fewer pixels merge into a neighbour.
    fn min_area(&self) -> u32 {
        (1.0 + self.simplicity.clamp(0.0, 10.0)).powi(2).round() as u32
    }
    /// Regions thinner than this (twice the area over the perimeter, in pixels) merge too.
    fn sliver(&self) -> f32 {
        0.6 + 0.1 * self.simplicity.clamp(0.0, 10.0)
    }
    /// Douglas–Peucker tolerance of the outlines, in pixels: looser with less Edge Fidelity
    /// (twice as loose at 1, half at 3).
    fn tolerance(&self) -> f32 {
        (0.5 + 0.12 * self.simplicity.clamp(0.0, 10.0)) * 2f32.powf(2.0 - self.fidelity.clamp(1.0, 3.0))
    }
    /// Below Edge Fidelity 2 the outline is rounded off before it is fitted: points averaged with
    /// this many neighbours either side (3 at fidelity 1).
    fn rounding(&self) -> usize {
        ((2.0 - self.fidelity.clamp(1.0, 3.0)) * 3.0).round().max(0.0) as usize
    }
    /// Turns sharper than this stay corners.
    fn corner_turn(&self) -> f32 {
        (100.0 - 22.5 * self.fidelity.clamp(1.0, 3.0)).to_radians()
    }
    /// Rounds of edge-preserving smoothing.
    fn smooth_rounds(&self) -> usize {
        2 + (self.simplicity.clamp(0.0, 10.0) / 4.0) as usize
    }
}

/// A traced image: the shapes and their colours, ready to fill any tile.
pub(crate) struct Traced {
    /// Document position of the traced window's top-left pixel; outlines are relative to it.
    x0: i32,
    y0: i32,
    /// Fitted edges (polylines), each shared by the one or two regions it separates.
    chains: Vec<Vec<Pt>>,
    /// Per chain, its bounding box `[x0, y0, x1, y1]`.
    chain_boxes: Vec<[f32; 4]>,
    regions: Vec<Region>,
    /// Width of the traced window, each pixel's region, and its paper colour (the region's
    /// lightness with the hue of its own pixels nearby).
    width: usize,
    reg: Vec<u32>,
    field: Vec<Px>,
}

/// One shape: its flat colour, bounding box and outline.
struct Region {
    colour: Px,
    bbox: [f32; 4],
    /// The chains around the region, reversed where the region is on their right.
    edges: Vec<(u32, bool)>,
}

/// Traces `c` (sRGB colour of every pixel of `r`, row-major).
pub(crate) fn trace(c: &[Px], r: Rect, s: &Settings) -> Traced {
    let (w, h) = (r.width() as usize, r.height() as usize);
    let empty = Traced { x0: r.x0, y0: r.y0, chains: Vec::new(), chain_boxes: Vec::new(), regions: Vec::new(), width: 0, reg: Vec::new(), field: Vec::new() };
    if w == 0 || h == 0 || c.len() != w * h {
        return empty;
    }
    let mut lab = vec![[0.0f32; 3]; w * h];
    par::chunks_mut(&mut lab, w, true, |y, row| {
        for (o, p) in row.iter_mut().zip(&c[y * w..(y + 1) * w]) {
            *o = oklab(*p);
        }
    });
    let original = lab.clone();
    let lab = smooth(lab, w, h, s.smooth_rounds());
    let palette = palette(&lab, s.levels.clamp(2, 8));
    let mut labels = vec![0u8; w * h];
    par::chunks_mut(&mut labels, w, true, |y, row| {
        for (o, p) in row.iter_mut().zip(&lab[y * w..(y + 1) * w]) {
            *o = nearest(&palette, *p);
        }
    });
    let (comp, n) = components(&labels, w, h);
    let (reg, colours) = merge_regions(&comp, n, c, w, h, s);
    let (chains, sides) = follow_edges(&reg, &colours, c, w, h);
    let chains: Vec<Vec<Pt>> = par::map(chains.len(), true, |i| fit(&chains[i].0, chains[i].1, s));
    let chain_boxes: Vec<[f32; 4]> = chains.iter().map(|ch| bounds(ch)).collect();
    let paint = paper_colours(&reg, colours.len(), &original);
    let field = local_paper(&reg, &paint, &original, w, h, 3.0 + 1.2 * s.simplicity.clamp(0.0, 10.0));
    let mut regions: Vec<Region> =
        paint.into_iter().map(|colour| Region { colour, bbox: [f32::MAX, f32::MAX, f32::MIN, f32::MIN], edges: Vec::new() }).collect();
    for (k, &(left, right)) in sides.iter().enumerate() {
        for (side, rev) in [(left, false), (right, true)] {
            if let Some(g) = regions.get_mut(side as usize) {
                g.edges.push((k as u32, rev));
                let b = chain_boxes[k];
                g.bbox = [g.bbox[0].min(b[0]), g.bbox[1].min(b[1]), g.bbox[2].max(b[2]), g.bbox[3].max(b[3])];
            }
        }
    }
    Traced { x0: r.x0, y0: r.y0, chains, chain_boxes, regions, width: w, reg, field }
}

impl Traced {
    /// The cut paper over `r` (document coordinates): sRGB colour per pixel, row-major.
    pub(crate) fn render(&self, r: Rect) -> Vec<Px> {
        let (w, h) = (r.width() as usize, r.height() as usize);
        let mut col = vec![[0.0f32; 3]; w * h];
        if w == 0 || h == 0 {
            return col;
        }
        let mut weight = vec![0.0f32; w * h];
        let stride = w + 2;
        let mut acc = vec![0.0f32; stride * h];
        let (ox, oy) = ((self.x0 - r.x0) as f32, (self.y0 - r.y0) as f32);
        let (wf, hf) = (w as f32, h as f32);
        let (dx, dy) = ((r.x0 - self.x0) as i64, (r.y0 - self.y0) as i64);
        let th = self.reg.len() / self.width.max(1);
        for (gi, g) in self.regions.iter().enumerate() {
            let b = [g.bbox[0] + ox, g.bbox[1] + oy, g.bbox[2] + ox, g.bbox[3] + oy];
            if b[2] <= 0.0 || b[0] >= wf || b[3] <= 0.0 || b[1] >= hf {
                continue;
            }
            for &(k, rev) in &g.edges {
                let (Some(pts), Some(cb)) = (self.chains.get(k as usize), self.chain_boxes.get(k as usize)) else { continue };
                // Chains left of the tile still count (they wind the rows they cross); chains to
                // its right or above or below it don't.
                if cb[0] + ox >= wf || cb[3] + oy <= 0.0 || cb[1] + oy >= hf {
                    continue;
                }
                for seg in pts.windows(2) {
                    let (a, z) = if rev { (seg[1], seg[0]) } else { (seg[0], seg[1]) };
                    line_clipped(&mut acc, stride, h, [a[0] + ox, a[1] + oy], [z[0] + ox, z[1] + oy]);
                }
            }
            let y0 = b[1].floor().max(0.0) as usize;
            let y1 = (b[3].ceil().max(0.0) as usize).min(h);
            let xe = ((b[2].ceil().max(0.0) as usize) + 1).min(w);
            for y in y0..y1 {
                let row = &mut acc[y * stride..(y + 1) * stride];
                let mut sum = 0.0f32;
                for (x, v) in row.iter().enumerate().take(xe) {
                    sum += v;
                    let cov = sum.abs().min(1.0);
                    if cov > 0.0 {
                        let i = y * w + x;
                        // Inside its own pixels a shape takes the local colour; where it reaches
                        // into a neighbour's edge pixel, its own flat colour.
                        let (tx, ty) = (x as i64 + dx, y as i64 + dy);
                        let t = (tx >= 0 && ty >= 0 && (tx as usize) < self.width && (ty as usize) < th).then(|| ty as usize * self.width + tx as usize);
                        let paint = match t {
                            Some(t) if self.reg[t] as usize == gi => self.field[t],
                            _ => g.colour,
                        };
                        col[i] = [col[i][0] + paint[0] * cov, col[i][1] + paint[1] * cov, col[i][2] + paint[2] * cov];
                        weight[i] += cov;
                    }
                }
                row.fill(0.0);
            }
        }
        for (p, &wt) in col.iter_mut().zip(&weight) {
            if wt > 1e-6 {
                *p = [p[0] / wt, p[1] / wt, p[2] / wt];
            }
        }
        col
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use colour::dist2;

    fn settings(levels: usize, simplicity: f32, fidelity: f32) -> Settings {
        Settings { levels, simplicity, fidelity }
    }

    /// Total coverage of every pixel: each pixel is covered exactly once by the shapes.
    fn coverage(t: &Traced, r: Rect) -> Vec<f32> {
        let (w, h) = (r.width() as usize, r.height() as usize);
        let mut weight = vec![0.0f32; w * h];
        let stride = w + 2;
        let mut acc = vec![0.0f32; stride * h];
        for g in &t.regions {
            for &(k, rev) in &g.edges {
                for seg in t.chains[k as usize].windows(2) {
                    let (a, z) = if rev { (seg[1], seg[0]) } else { (seg[0], seg[1]) };
                    line_clipped(&mut acc, stride, h, a, z);
                }
            }
            for y in 0..h {
                let mut sum = 0.0;
                for x in 0..w {
                    sum += acc[y * stride + x];
                    weight[y * w + x] += sum.abs().min(1.0);
                }
                acc[y * stride..(y + 1) * stride].fill(0.0);
            }
        }
        weight
    }

    #[test]
    fn shapes_cover_every_pixel_once_with_no_seams() {
        // Blobs of four colours touching each other and the canvas edge, with T-junctions,
        // saddles and holes.
        let (w, h) = (48usize, 36usize);
        let pal = [[0.1, 0.1, 0.1], [0.9, 0.2, 0.2], [0.2, 0.7, 0.3], [0.95, 0.95, 0.9]];
        let c: Vec<Px> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let k = ((x / 7.0).sin() + (y / 5.0).cos() + (x * y / 90.0).sin()).abs() * 1.6;
                pal[(k as usize).min(3)]
            })
            .collect();
        let r = Rect::new(0, 0, w as i32, h as i32);
        for fidelity in [1.0, 3.0] {
            let t = trace(&c, r, &settings(4, 2.0, fidelity));
            assert!(t.regions.len() > 4);
            let cov = coverage(&t, r);
            let worst = cov.iter().map(|v| (v - 1.0).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-3, "coverage off by {worst}");
        }
    }

    #[test]
    fn edge_fidelity_changes_the_outlines() {
        // It used to set only which bends count as corners: on smooth shapes 0.5% of pixels
        // changed from fidelity 1 to 2 and 2.8% from 2 to 3 (now about 9% and 5%).
        // Large smooth shapes, as in a photo: few corners, long curved outlines.
        let (w, h) = (160usize, 120usize);
        let pal = [[0.1, 0.1, 0.1], [0.9, 0.3, 0.2], [0.3, 0.6, 0.9]];
        let c: Vec<Px> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let k = ((x / 17.0).sin() * 1.3 + (y / 13.0).cos() + (x * y / 900.0).sin()).abs();
                pal[(k as usize).min(2)]
            })
            .collect();
        let r = Rect::new(0, 0, w as i32, h as i32);
        let out = |f: f32| trace(&c, r, &settings(3, 2.0, f)).render(r);
        let changed = |a: &[Px], b: &[Px]| a.iter().zip(b).filter(|(p, q)| dist2(**p, **q) > 0.01).count() as f32 / a.len() as f32;
        let (loose, normal, tight) = (out(1.0), out(2.0), out(3.0));
        assert!(changed(&loose, &normal) > 0.04, "fidelity 1 vs 2: {}", changed(&loose, &normal));
        assert!(changed(&normal, &tight) > 0.04, "fidelity 2 vs 3: {}", changed(&normal, &tight));
    }

    #[test]
    fn empty_images_stop_before_the_transpose_and_row_passes() {
        for r in [Rect::new(0, 0, 0, 4), Rect::new(0, 0, 4, 0), Rect::new(0, 0, 0, 0)] {
            let t = trace(&[], r, &settings(4, 2.0, 1.0));
            assert!(t.regions.is_empty());
            assert!(t.chains.is_empty());
            assert!(t.field.is_empty());
        }
    }

    #[test]
    fn filling_does_not_depend_on_the_tiling() {
        let (w, h) = (40usize, 30usize);
        let c: Vec<Px> = (0..w * h).map(|i| if ((i % w) / 6 + (i / w) / 5) % 3 == 0 { [0.8, 0.3, 0.1] } else { [0.1, 0.3, 0.7] }).collect();
        let r = Rect::new(10, 20, 10 + w as i32, 20 + h as i32);
        let t = trace(&c, r, &settings(3, 1.0, 2.0));
        let whole = t.render(r);
        for tile in [Rect::new(10, 20, 23, 31), Rect::new(23, 31, 50, 50), Rect::new(37, 20, 50, 33)] {
            let part = t.render(tile);
            for y in tile.y0..tile.y1 {
                for x in tile.x0..tile.x1 {
                    let a = whole[((y - r.y0) as usize) * w + (x - r.x0) as usize];
                    let b = part[((y - tile.y0) as usize) * tile.width() as usize + (x - tile.x0) as usize];
                    assert!(dist2(a, b) < 1e-8, "({x}, {y}): {a:?} vs {b:?}");
                }
            }
        }
    }

    #[test]
    fn noise_merges_into_neighbours_and_noise_apart_colours_are_one() {
        let (w, h) = (32usize, 32usize);
        let c: Vec<Px> = (0..w * h)
            .map(|i| {
                let v = 0.3 + crate::photo_util::hash01((i % w) as i64, (i / w) as i64, 5) * 0.06;
                if (11..14).contains(&(i % w)) && (11..14).contains(&(i / w)) { [0.9, 0.9, 0.9] } else { [v, v, v] }
            })
            .collect();
        let r = Rect::new(0, 0, w as i32, h as i32);
        assert_eq!(trace(&c, r, &settings(6, 0.0, 2.0)).regions.len(), 2, "the speck stays at simplicity 0");
        assert_eq!(trace(&c, r, &settings(6, 10.0, 2.0)).regions.len(), 1, "the speck merges at simplicity 10");
    }
}
