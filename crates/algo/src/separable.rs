//! Separable blurs described as data, so another device (the GPU compositor's layer operators,
//! #5772) runs exactly the blur the CPU runs: the kernel weights or box widths come from here,
//! and [`crate::blur`] builds its own passes from the same description, so the two can't drift.
//!
//! [`OperatorRegion`] is the region rule that goes with it: which source pixels an output area
//! reads, with the edge pixels repeated at the extent as the CPU filters do.

use photocraft_geom::Rect;

use crate::{FilterParams, blur};

/// A blur made of passes along each axis, horizontal first, as the CPU runs it.
#[derive(Clone, Debug, PartialEq)]
pub enum Separable {
    /// One pass of this normalised kernel (odd length, centred) along each axis.
    Kernel(Vec<f32>),
    /// Running-mean box passes of these odd widths, in turn, along each axis.
    Boxes(Vec<usize>),
}

impl Separable {
    /// The blur `params` runs on the CPU, or `None` for a filter that isn't one (only Gaussian
    /// Blur so far). Small radii use the exact kernel, larger ones three box passes.
    pub fn of(params: &FilterParams) -> Option<Self> {
        match *params {
            FilterParams::GaussianBlur { radius } => Some(match blur::box_widths(params) {
                Some(boxes) => Separable::Boxes(boxes),
                None => Separable::Kernel(blur::gaussian_blur_kernel(radius)),
            }),
            _ => None,
        }
    }

    /// How far outside an output pixel the blur reads, along each axis.
    pub fn reach(&self) -> i32 {
        let reach = match self {
            Separable::Kernel(k) => k.len() / 2,
            Separable::Boxes(boxes) => boxes.iter().fold(0usize, |sum, w| sum.saturating_add(w / 2)),
        };
        i32::try_from(reach).unwrap_or(i32::MAX)
    }
}

/// The area a neighbourhood filter on a layer repeats its edge pixels at: the canvas, or past it
/// where the layer has pixels off the canvas, as Photoshop does.
pub fn edge_extent(canvas: Rect, content: Rect) -> Rect {
    canvas.union(&content)
}

/// Which source pixels an output area of a blur reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorRegion {
    /// How far the blur reads outside an output pixel ([`Separable::reach`]).
    pub reach: i32,
    /// Where the source's edge pixels repeat ([`edge_extent`]): nothing outside it is read.
    pub extent: Rect,
}

impl OperatorRegion {
    /// The source pixels output `out` reads: `out` grown by the reach, inside the extent. Pixels
    /// it needs beyond the extent repeat the edge of this area, so they need no source of their
    /// own. Empty when `out` is empty or doesn't come within reach of the extent.
    pub fn input(&self, out: Rect) -> Rect {
        if out.is_empty() {
            return Rect::EMPTY;
        }
        out.inflate(self.reach.max(0)).intersect(&self.extent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Halo;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_raster::{Interrupt, Surface};

    const RADII: [f32; 11] = [0.01, 0.05, 0.1, 0.3, 0.8, 2.0, 4.0, 4.5, 12.0, 40.0, 250.0];

    /// The CPU blur's own choice: the exact kernel up to radius 4, boxes beyond; and its halo
    /// (how far the CPU grows an output area) covers the reach.
    #[test]
    fn the_description_is_the_cpus_blur() {
        for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let blur = Separable::of(&params).unwrap();
            match &blur {
                Separable::Kernel(k) => {
                    assert!(radius <= 4.0, "radius {radius}");
                    assert_eq!(k, &blur::gaussian_blur_kernel(radius));
                    assert_eq!(k.len() % 2, 1);
                    assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                }
                Separable::Boxes(b) => {
                    assert!(radius > 4.0, "radius {radius}");
                    assert_eq!(Some(b.clone()), blur::box_widths(&params));
                    assert!(b.iter().all(|w| w % 2 == 1));
                }
            }
            let Halo::Radius(halo) = params.halo() else { panic!("a Gaussian has a radius halo") };
            assert!(blur.reach() <= halo, "radius {radius}: reach {} past the halo {halo}", blur.reach());
        }
        assert_eq!(Separable::of(&FilterParams::BoxBlur { radius: 9.0 }), None, "not a Gaussian (yet)");
        assert_eq!(Separable::Boxes(vec![usize::MAX, 3]).reach(), i32::MAX, "saturates");
    }

    /// One opaque pixel in a transparent layer, blurred on the CPU, spreads no further than the
    /// reach (beyond it the box passes' running sums leave only float residue, far below a
    /// 16-bit level), and the exact kernel spreads exactly that far.
    #[test]
    fn an_impulse_spreads_no_further_than_the_reach() {
        for radius in RADII {
            let params = FilterParams::GaussianBlur { radius };
            let reach = Separable::of(&params).unwrap().reach();
            let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
            let size = 2 * reach + 41;
            let canvas = Rect::new(0, 0, size, size);
            let mut s = Surface::new(fmt);
            let dot = Rect::new(size / 2, size / 2, size / 2 + 1, size / 2 + 1);
            s.fill_rect(dot, &[1.0, 1.0, 1.0, 1.0]);
            let out = crate::apply_in_with(&s, &params, canvas, canvas, None, canvas, &Interrupt::NONE).unwrap();
            let mut px = Vec::new();
            out.read_region_into(canvas, &mut px);
            let touched = px
                .as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                // The box passes' running sums leave float residue (~1e-8) after the impulse.
                .filter(|(_, p)| p[3] > 1e-6)
                .fold(Rect::EMPTY, |r, (i, _)| {
                    let (x, y) = ((i as i32) % size, (i as i32) / size);
                    r.union(&Rect::new(x, y, x + 1, y + 1))
                });
            // Nothing past the reach; the exact kernel reaches all of it. (A single pixel's box
            // tails are smaller than the running sums' residue, so they aren't checked for that.)
            assert!(dot.inflate(reach).contains_rect(&touched), "radius {radius}: {touched:?} past the reach {reach}");
            if let Some(Separable::Kernel(k)) = Separable::of(&params) {
                // Calibrated subpixel kernels have tails below the visibility threshold.
                let center = k.len() / 2;
                let visible = k.iter().enumerate().filter(|(_, w)| **w * k[center] > 1e-6).map(|(i, _)| i.abs_diff(center)).max().unwrap_or(0);
                assert_eq!(touched, dot.inflate(visible as i32), "radius {radius}");
            }
        }
    }

    /// Pixels outside [`OperatorRegion::input`] don't change an output area's result, and the
    /// edge repeats at the extent: a layer's area next to the extent's edge blurs the same
    /// whatever lies beyond it.
    #[test]
    fn output_depends_only_on_the_input_region() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let extent = Rect::new(0, 0, 300, 200);
        let noise = |seed: u32| {
            let mut s = Surface::new(fmt);
            for y in 0..200 {
                for x in 0..300 {
                    let h = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ seed;
                    let v = (h % 1000) as f32 / 999.0;
                    s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, (h % 7) as f32 / 6.0, (h % 5) as f32 / 4.0]);
                }
            }
            s
        };
        let a = noise(1);
        for radius in [0.8, 4.0, 12.0] {
            let params = FilterParams::GaussianBlur { radius };
            let region = OperatorRegion { reach: Separable::of(&params).unwrap().reach(), extent };
            for out in [Rect::new(100, 80, 140, 120), Rect::new(0, 0, 30, 25), Rect::new(270, 170, 300, 200)] {
                let input = region.input(out);
                assert!(extent.contains_rect(&input) && input.contains_rect(&out));
                // Everything outside the input region replaced by other pixels.
                let mut b = noise(2);
                let mut inside = Vec::new();
                a.read_region_into(input, &mut inside);
                b.write_region(input, &inside);
                let blur = |s: &Surface| {
                    let r = crate::apply_in_with(s, &params, out, extent, None, extent, &Interrupt::NONE).unwrap();
                    let mut px = Vec::new();
                    r.read_region_into(out, &mut px);
                    px
                };
                assert_eq!(blur(&a), blur(&b), "radius {radius}, {out:?}");
            }
        }
        assert_eq!(OperatorRegion { reach: 5, extent }.input(Rect::EMPTY), Rect::EMPTY);
        assert_eq!(OperatorRegion { reach: 5, extent }.input(Rect::new(400, 400, 410, 410)), Rect::EMPTY, "out of reach of the extent");
        assert_eq!(OperatorRegion { reach: -3, extent }.input(Rect::new(10, 10, 20, 20)), Rect::new(10, 10, 20, 20));
        assert_eq!(edge_extent(extent, Rect::new(-10, 50, 20, 60)), Rect::new(-10, 0, 300, 200));
    }
}
