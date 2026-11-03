//! Cutout (Filter Gallery › Artistic): the image as flat sheets of cut paper.
//!
//! What it does follows the reference application, measured on grey ramps, hue sweeps, squares of
//! known size and a portrait:
//!
//! 1. **Tone bands**: each pixel's band is the plain mean of its three channels, cut into Number of
//!    Levels equal steps (four levels: below 64, 128, 192 and the rest, on 0–255). Colours of
//!    different hue but the same mean share a band.
//! 2. **Regions**: 4-connected areas of one band. Regions smaller than the Edge Simplicity area
//!    merge, smallest first, into the neighbour they share most border with. The area depends on
//!    the slider alone, not on the image size. Each region is filled with the mean colour of its own
//!    pixels, so a region mixing hues comes out as their average.
//! 3. **Edges**: the cracks between regions are linked into chains from junction to junction
//!    (where three regions meet, or the canvas edge). Each crack is moved along its normal to where
//!    the pixels on either side say the edge is (an anti-aliased pixel is part one colour, part the
//!    other). A chain is fitted once, so two neighbouring shapes share one and the same edge.
//! 4. **Outlines**: straight-sided polygons. Sharp bends stay corners; the runs between are
//!    simplified (Douglas–Peucker) to a tolerance that grows with Edge Simplicity and shrinks with
//!    Edge Fidelity.
//! 5. **Fill**: each region is scan-converted with exact area coverage, so edges are anti-aliased
//!    and the coverages of shapes that share an edge sum to one: no seams.
//!
//! Tracing reads the whole image and runs once; filling runs per tile and gives the same pixels
//! whatever the tiling.
//!
//! Each stage is its own module: [`regions`] (1–2), [`edges`] (3), [`curves`] (4) and [`fill`]
//! (5); [`trace`] runs them in turn.

use photocraft_geom::Rect;

use crate::par;

mod curves;
mod edges;
mod fill;
mod regions;
mod util;

use curves::{bounds, fit};
use edges::follow_edges;
use fill::line_clipped;
use regions::{components, merge_regions};

type Px = [f32; 3];
type Pt = [f32; 2];

/// The tone band of a pixel: the mean of its three channels in `levels` equal steps.
fn band(p: Px, levels: usize) -> u8 {
    let v = (p[0] + p[1] + p[2]) / 3.0;
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    ((v * levels as f32) as usize).min(levels - 1) as u8
}

/// Region id of the area outside the image.
const OUT: u32 = u32::MAX;

/// The filter's three sliders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    /// Number of Levels (2–8): how many tone bands.
    pub levels: usize,
    /// Edge Simplicity (0–10): smaller shapes merge and outlines simplify more.
    pub simplicity: f32,
    /// Edge Fidelity (1–3): more corners are kept.
    pub fidelity: f32,
}

/// Linear interpolation in a per-step table over Edge Simplicity 0–10.
fn by_simplicity(table: &[f32; 11], simplicity: f32) -> f32 {
    let s = if simplicity.is_finite() { simplicity.clamp(0.0, 10.0) } else { 0.0 };
    let i = (s.floor() as usize).min(9);
    let t = s - i as f32;
    table[i] + (table[i + 1] - table[i]) * t
}

impl Settings {
    /// Regions with fewer pixels merge into a neighbour. Measured in the reference application
    /// with squares of 2–96 px: the smallest that survive are 6 px wide at Simplicity 0–2, 12 px
    /// at 4, 48 px at 6 and 96 px at 7, the same on a 256 px and a 600 px canvas.
    fn min_area(&self) -> u32 {
        const AREA: [f32; 11] = [25.0, 25.0, 25.0, 50.0, 100.0, 400.0, 1500.0, 5000.0, 10000.0, 16000.0, 30000.0];
        by_simplicity(&AREA, self.simplicity).round() as u32
    }
    /// Douglas–Peucker tolerance of the straight outlines, in pixels: coarser with more Edge
    /// Simplicity, finer with more Edge Fidelity.
    fn tolerance(&self) -> f32 {
        const TOL: [f32; 11] = [0.5, 0.5, 0.6, 0.8, 1.0, 1.5, 2.5, 4.0, 7.0, 12.0, 20.0];
        by_simplicity(&TOL, self.simplicity) * 2f32.powf(2.0 - self.fidelity.clamp(1.0, 3.0))
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
    let empty = Traced { x0: r.x0, y0: r.y0, chains: Vec::new(), chain_boxes: Vec::new(), regions: Vec::new() };
    if w == 0 || h == 0 || c.len() != w * h {
        return empty;
    }
    // A non-finite sample (32-bit documents can hold them) would make its region's mean colour,
    // and so its edges and fill, non-finite: count it as 0 from here on.
    let clean = |v: f32| if v.is_finite() { v } else { 0.0 };
    let owned: Vec<Px>;
    let c: &[Px] = if c.iter().flatten().all(|v| v.is_finite()) {
        c
    } else {
        owned = c.iter().map(|p| p.map(clean)).collect();
        &owned
    };
    let levels = s.levels.clamp(2, 8);
    let mut labels = vec![0u8; w * h];
    par::chunks_mut(&mut labels, w, true, |y, row| {
        for (o, p) in row.iter_mut().zip(&c[y * w..(y + 1) * w]) {
            *o = band(*p, levels);
        }
    });
    let (comp, n) = components(&labels, w, h);
    let (reg, colours) = merge_regions(&comp, n, c, w, h, s);
    let (chains, sides) = follow_edges(&reg, &colours, c, w, h);
    let chains: Vec<Vec<Pt>> = par::map(chains.len(), true, |i| fit(&chains[i].0, chains[i].1, s));
    let chain_boxes: Vec<[f32; 4]> = chains.iter().map(|ch| bounds(ch)).collect();
    let mut regions: Vec<Region> =
        colours.into_iter().map(|colour| Region { colour, bbox: [f32::MAX, f32::MAX, f32::MIN, f32::MIN], edges: Vec::new() }).collect();
    for (k, &(left, right)) in sides.iter().enumerate() {
        for (side, rev) in [(left, false), (right, true)] {
            if let Some(g) = regions.get_mut(side as usize) {
                g.edges.push((k as u32, rev));
                let b = chain_boxes[k];
                g.bbox = [g.bbox[0].min(b[0]), g.bbox[1].min(b[1]), g.bbox[2].max(b[2]), g.bbox[3].max(b[3])];
            }
        }
    }
    Traced { x0: r.x0, y0: r.y0, chains, chain_boxes, regions }
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
        for g in &self.regions {
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
                        let paint = g.colour;
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

    fn dist2(a: Px, b: Px) -> f32 {
        (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
    }

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
    fn edge_fidelity_changes_the_outlines_a_little() {
        // In the reference application Edge Fidelity nudges the outlines: on a portrait 1.9% of
        // pixels change from fidelity 1 to 2 and 1.2% from 2 to 3. Large smooth shapes here.
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
        for (name, a, b) in [("1 vs 2", &loose, &normal), ("2 vs 3", &normal, &tight)] {
            let f = changed(a, b);
            assert!(f > 0.002 && f < 0.06, "fidelity {name}: {f}");
        }
    }

    #[test]
    fn empty_images_stop_before_the_transpose_and_row_passes() {
        for r in [Rect::new(0, 0, 0, 4), Rect::new(0, 0, 4, 0), Rect::new(0, 0, 0, 0)] {
            let t = trace(&[], r, &settings(4, 2.0, 1.0));
            assert!(t.regions.is_empty());
            assert!(t.chains.is_empty());
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

    /// A NaN or infinite sample must not reach a region's colour, its edges or the fill.
    #[test]
    fn non_finite_samples_give_finite_covered_output() {
        let (w, h) = (24usize, 16usize);
        let mut c: Vec<Px> = (0..w * h).map(|i| if i % w < 12 { [0.1, 0.1, 0.1] } else { [0.9, 0.8, 0.7] }).collect();
        c[3 * w + 4] = [f32::NAN, 0.5, 0.5];
        c[8 * w + 15] = [f32::INFINITY, f32::NEG_INFINITY, 0.2];
        c[12 * w + 12] = [f32::NAN; 3];
        let r = Rect::new(0, 0, w as i32, h as i32);
        for simplicity in [0.0, 4.0] {
            let t = trace(&c, r, &settings(4, simplicity, 2.0));
            assert!(t.regions.iter().all(|g| g.colour.iter().all(|v| v.is_finite())), "region colours finite");
            assert!(t.chains.iter().flatten().flatten().all(|v| v.is_finite()), "outlines finite");
            let out = t.render(r);
            assert!(out.iter().flatten().all(|v| v.is_finite()), "pixels finite");
            let worst = coverage(&t, r).iter().map(|v| (v - 1.0).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-3, "coverage off by {worst}");
        }
    }

    #[test]
    fn noise_merges_into_neighbours_and_a_six_pixel_square_survives_at_simplicity_0() {
        let (w, h) = (32usize, 32usize);
        let c: Vec<Px> = (0..w * h)
            .map(|i| {
                // Noise inside one band (1/6 to 2/6 at six levels).
                let v = 0.2 + crate::photo_util::hash01((i % w) as i64, (i / w) as i64, 5) * 0.06;
                if (11..17).contains(&(i % w)) && (11..17).contains(&(i / w)) { [0.9, 0.9, 0.9] } else { [v, v, v] }
            })
            .collect();
        let r = Rect::new(0, 0, w as i32, h as i32);
        assert_eq!(trace(&c, r, &settings(6, 0.0, 2.0)).regions.len(), 2, "the speck stays at simplicity 0");
        assert_eq!(trace(&c, r, &settings(6, 10.0, 2.0)).regions.len(), 1, "the speck merges at simplicity 10");
    }
}
