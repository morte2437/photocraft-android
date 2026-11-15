//! Format-neutral DNG correction data used by PhotoCraft's develop pipeline.

/// One GainMap opcode: a grid of gains, bilinearly interpolated, applied to
/// the pixels of an area (in active-area coordinates) at a row / column pitch.
#[derive(Debug, Clone, PartialEq)]
pub struct GainMap {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub plane: usize,
    pub planes: usize,
    pub row_pitch: usize,
    pub col_pitch: usize,
    pub points_v: usize,
    pub points_h: usize,
    /// Spacing and origin of the map points, in image-relative coordinates (0..1).
    pub spacing_v: f64,
    pub spacing_h: f64,
    pub origin_v: f64,
    pub origin_h: f64,
    pub map_planes: usize,
    /// `points_v × points_h × map_planes` gains.
    pub gains: Vec<f32>,
}

impl GainMap {
    /// Gains along the map's horizontal points for image-relative row `rv`
    /// and map plane `mp` (vertical interpolation done).
    pub fn row(&self, rv: f64, mp: usize) -> Vec<f32> {
        let mp = mp.min(self.map_planes.saturating_sub(1));
        let fv = ((rv - self.origin_v) / self.spacing_v).clamp(0.0, (self.points_v - 1) as f64);
        let i0 = (fv.floor() as usize).min(self.points_v - 1);
        let i1 = (i0 + 1).min(self.points_v - 1);
        let t = (fv - i0 as f64) as f32;
        (0..self.points_h)
            .map(|j| {
                let a = self.gains.get((i0 * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
                let b = self.gains.get((i1 * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
                a + (b - a) * t
            })
            .collect()
    }

    /// Interpolates a [`GainMap::row`] at image-relative column `rh`.
    #[inline]
    pub fn at(&self, row: &[f32], rh: f64) -> f32 {
        let fh = ((rh - self.origin_h) / self.spacing_h).clamp(0.0, (self.points_h - 1) as f64);
        let j0 = (fh.floor() as usize).min(self.points_h - 1);
        let j1 = (j0 + 1).min(self.points_h - 1);
        let t = (fh - j0 as f64) as f32;
        let (a, b) = (row.get(j0).copied().unwrap_or(1.0), row.get(j1).copied().unwrap_or(1.0));
        a + (b - a) * t
    }

    /// Bilinearly interpolates the gain at image-relative row `rv` and column `rh` for map
    /// plane `mp`, touching only the four surrounding grid points. This avoids building the
    /// whole `points_h`-wide row per image row, which made development work proportional to
    /// image height times horizontal map size even when each output row needs few values.
    #[inline]
    pub fn at_rv(&self, rv: f64, mp: usize, rh: f64) -> f32 {
        let mp = mp.min(self.map_planes.saturating_sub(1));
        let fv = ((rv - self.origin_v) / self.spacing_v).clamp(0.0, (self.points_v - 1) as f64);
        let i0 = (fv.floor() as usize).min(self.points_v - 1);
        let i1 = (i0 + 1).min(self.points_v - 1);
        let tv = (fv - i0 as f64) as f32;
        let fh = ((rh - self.origin_h) / self.spacing_h).clamp(0.0, (self.points_h - 1) as f64);
        let j0 = (fh.floor() as usize).min(self.points_h - 1);
        let j1 = (j0 + 1).min(self.points_h - 1);
        let th = (fh - j0 as f64) as f32;
        let g = |i: usize, j: usize| self.gains.get((i * self.points_h + j) * self.map_planes + mp).copied().unwrap_or(1.0);
        let top = g(i0, j0) + (g(i0, j1) - g(i0, j0)) * th;
        let bot = g(i1, j0) + (g(i1, j1) - g(i1, j0)) * th;
        top + (bot - top) * tv
    }

    /// `true` if the map covers active-area pixel (`ax`, `ay`).
    #[inline]
    pub fn covers(&self, ax: usize, ay: usize) -> bool {
        ay >= self.top
            && ay < self.bottom
            && ax >= self.left
            && ax < self.right
            && (ay - self.top).is_multiple_of(self.row_pitch)
            && (ax - self.left).is_multiple_of(self.col_pitch)
    }
}

/// One MapPolynomial opcode: `out = Σ cᵢ · inⁱ` on the linear reference values (0..1, after the
/// black and white levels) of an area, in active-area coordinates, and a plane range, at a row /
/// column pitch. Input and output are clipped to 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct MapPolynomial {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
    pub plane: usize,
    pub planes: usize,
    pub row_pitch: usize,
    pub col_pitch: usize,
    /// `c₀ … c_degree` (degree at most 8).
    pub coefficients: Vec<f64>,
}

impl MapPolynomial {
    /// The polynomial at `x`, both clipped to 0..1.
    #[inline]
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let y = self.coefficients.iter().rev().fold(0.0, |acc, c| acc * x + c);
        if y.is_finite() { y.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// `true` if the opcode covers active-area pixel (`ax`, `ay`) in sample `s`.
    #[inline]
    pub(crate) fn covers(&self, ax: usize, ay: usize, s: usize) -> bool {
        s >= self.plane
            && s - self.plane < self.planes
            && ay >= self.top
            && ay < self.bottom
            && ax >= self.left
            && ax < self.right
            && (ay - self.top).is_multiple_of(self.row_pitch)
            && (ax - self.left).is_multiple_of(self.col_pitch)
    }
}
