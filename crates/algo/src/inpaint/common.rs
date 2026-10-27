use super::*;

/// Below this many cells a pass runs on the calling thread (thread hand-off costs more than it
/// saves). Every pass gives the same result either way.
pub(super) const PAR_MIN: usize = 64 * 64;

/// [`PAR_MIN`] for the passes that match or vote a patch per cell.
pub(super) const PAR_MIN_PATCHES: usize = 16 * 16;

/// Largest level (in cells) whose source patches' moments [`Solver::prepare`] keeps.
pub(super) const SOURCE_MOMENTS_MAX: usize = 1 << 20;

/// SplitMix64: tiny deterministic RNG.
#[derive(Clone)]
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    /// A generator whose sequence is fixed by `seed`.
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    /// The next value of the SplitMix64 sequence.
    pub(crate) fn next(&mut self) -> u64 {
        crate::photo_util::splitmix64(&mut self.0)
    }
    /// Skip `n` numbers (the same state as after `n` calls of [`Rng::next`]).
    pub(crate) fn skip(&mut self, n: u64) {
        self.0 = self.0.wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    /// Uniform integer in `lo..=hi`.
    pub(crate) fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

/// How far a copied patch may be adjusted to its target, per channel: `gain · source + bias`, the
/// gain within `gain` and the bias within `±bias` (Image Melding's photometric transform). The
/// gain is the ratio of the patches' standard deviations, the bias what then matches their means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adapt {
    pub gain: (f32, f32),
    pub bias: f32,
}

impl Adapt {
    /// Copies are taken as they are.
    pub const NONE: Adapt = Adapt { gain: (1.0, 1.0), bias: 0.0 };

    pub fn is_none(&self) -> bool {
        self.gain == (1.0, 1.0) && self.bias == 0.0
    }
}

impl Default for Adapt {
    fn default() -> Self {
        Self::NONE
    }
}

/// What a pixel is to [`complete_masked`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Px {
    /// Not part of the image (a gap, or outside a transformed copy): ignored.
    Void,
    /// Known, part of the context the fill must agree with, but not copied from.
    Known,
    /// Known and may be copied from.
    Source,
    /// To be filled.
    Hole,
}

/// One pyramid level. Level 0 borrows the caller's buffers until it is solved.
pub(super) struct Level<'a> {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) img: Cow<'a, [f32]>,
    pub(super) px: Cow<'a, [Px]>,
}

impl Level<'_> {
    pub(super) const EMPTY: Level<'static> = Level { w: 0, h: 0, img: Cow::Borrowed(&[]), px: Cow::Borrowed(&[]) };
}

/// The half-open box `[x0, x1) × [y0, y1)` of a level's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Bounds {
    pub(super) x0: usize,
    pub(super) y0: usize,
    pub(super) x1: usize,
    pub(super) y1: usize,
}

impl Bounds {
    /// Grown by `r` on every side, within a `w × h` grid.
    pub(super) fn grow(self, r: usize, w: usize, h: usize) -> Bounds {
        Bounds { x0: self.x0.saturating_sub(r), y0: self.y0.saturating_sub(r), x1: self.x1.saturating_add(r).min(w), y1: self.y1.saturating_add(r).min(h) }
    }

    /// The same box on the next coarser level (whose cells are 2 × 2 of these).
    pub(super) fn half(self) -> Bounds {
        Bounds { x0: self.x0 / 2, y0: self.y0 / 2, x1: self.x1.div_ceil(2), y1: self.y1.div_ceil(2) }
    }

    pub(super) fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }

    pub(super) fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }

    pub(super) fn area(&self) -> usize {
        self.width() * self.height()
    }
}
