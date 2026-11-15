//! Format-neutral sensor data consumed by PhotoCraft's develop pipeline.

use crate::RawFormat;
use crate::color::ColorInfo;

/// A rectangle in sensor-data pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Rect { x, y, width, height }
    }

    /// The overlap of two rectangles (may be empty).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x.saturating_add(self.width)).min(o.x.saturating_add(o.width));
        let y1 = (self.y.saturating_add(self.height)).min(o.y.saturating_add(o.height));
        Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Colour of one CFA cell: 0 = red, 1 = green, 2 = blue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfa {
    pub width: usize,
    pub height: usize,
    /// Row-major colours, `width * height` entries.
    pub colors: Vec<u8>,
    /// Data coordinates of the pattern's top-left cell.
    pub origin_x: usize,
    pub origin_y: usize,
}

impl Cfa {
    /// A 2×2 pattern from a string such as `"RGGB"`, anchored at (0, 0).
    pub fn bayer(pattern: &str) -> Option<Cfa> {
        let colors: Vec<u8> = pattern
            .chars()
            .map(|c| match c {
                'R' => Some(0),
                'G' => Some(1),
                'B' => Some(2),
                _ => None,
            })
            .collect::<Option<_>>()?;
        (colors.len() == 4).then_some(Cfa { width: 2, height: 2, colors, origin_x: 0, origin_y: 0 })
    }

    /// Colour at data coordinates.
    #[inline]
    pub fn color(&self, x: usize, y: usize) -> u8 {
        fn relative(v: usize, origin: usize, period: usize) -> Option<usize> {
            if period == 0 {
                return None;
            }
            let (v, origin) = (v % period, origin % period);
            Some(if v >= origin { v - origin } else { period - (origin - v) })
        }
        let Some((cx, cy)) = relative(x, self.origin_x, self.width).zip(relative(y, self.origin_y, self.height)) else {
            return 1;
        };
        cy.checked_mul(self.width).and_then(|i| i.checked_add(cx)).and_then(|i| self.colors.get(i)).copied().unwrap_or(1)
    }

    /// The 2×2 colours seen from data coordinates (`x0`, `y0`), row-major.
    pub fn phase(&self, x0: usize, y0: usize) -> [u8; 4] {
        [self.color(x0, y0), self.color(x0.saturating_add(1), y0), self.color(x0, y0.saturating_add(1)), self.color(x0.saturating_add(1), y0.saturating_add(1))]
    }

    /// `true` for a Bayer pattern: a 2×2 period holding one red, one blue and two
    /// diagonal greens.
    pub fn is_bayer(&self) -> bool {
        if self.width == 0 || self.height == 0 || self.width > 16 || self.height > 16 {
            return false;
        }
        let (ox, oy) = (self.origin_x % self.width, self.origin_y % self.height);
        let p = self.phase(ox, oy);
        let periodic = (0..self.height).all(|y| (0..self.width).all(|x| self.color(ox + x, oy + y) == p[(y % 2) * 2 + x % 2]));
        let greens_on_diagonal = (p[0] == 1 && p[3] == 1 && p[1] != 1 && p[2] != 1 && u16::from(p[1]) + u16::from(p[2]) == 2)
            || (p[1] == 1 && p[2] == 1 && p[0] != 1 && p[3] != 1 && u16::from(p[0]) + u16::from(p[3]) == 2);
        periodic && greens_on_diagonal
    }
}

/// Black levels (DNG BlackLevel / BlackLevelRepeatDim / BlackLevelDeltaH/V).
#[derive(Debug, Clone, PartialEq)]
pub struct BlackLevels {
    pub rows: usize,
    pub cols: usize,
    /// `rows * cols * samples` values, sample fastest.
    pub values: Vec<f32>,
    /// Per column of the active area.
    pub delta_h: Vec<f32>,
    /// Per row of the active area.
    pub delta_v: Vec<f32>,
}

impl BlackLevels {
    pub fn uniform(v: f32) -> Self {
        BlackLevels { rows: 1, cols: 1, values: vec![v], delta_h: Vec::new(), delta_v: Vec::new() }
    }

    /// Black level at active-area coordinates for sample `s` of `spp`.
    #[inline]
    pub fn at(&self, ax: usize, ay: usize, s: usize, spp: usize) -> f32 {
        let r = ay % self.rows.max(1);
        let c = ax % self.cols.max(1);
        let index = r.checked_mul(self.cols.max(1)).and_then(|i| i.checked_add(c)).and_then(|i| i.checked_mul(spp)).and_then(|i| i.checked_add(s));
        let base = index.and_then(|i| self.values.get(i)).or_else(|| self.values.first()).copied().unwrap_or(0.0);
        base + self.delta_h.get(ax).copied().unwrap_or(0.0) + self.delta_v.get(ay).copied().unwrap_or(0.0)
    }

    /// `true` when there is a single black value for everything.
    pub fn is_uniform(&self) -> bool {
        self.delta_h.iter().all(|v| *v == 0.0) && self.delta_v.iter().all(|v| *v == 0.0) && self.values.windows(2).all(|w| w[0] == w[1])
    }
}

/// Undeveloped sensor data with everything needed to develop it.
#[derive(Debug, Clone)]
pub struct Sensor {
    pub format: RawFormat,
    pub make: Option<String>,
    pub model: Option<String>,
    /// Data width and height in pixels.
    pub width: usize,
    pub height: usize,
    /// Samples per pixel: 1 for CFA data, 3 for LinearRaw (1 for monochrome LinearRaw).
    pub samples: usize,
    /// Row-major interleaved samples, `width * height * samples`.
    pub data: Vec<u16>,
    /// Floating-point sensor samples, kept without quantizing. When present, `data` is empty.
    pub float_data: Option<Vec<f32>>,
    /// The CFA pattern (CFA data only).
    pub cfa: Option<Cfa>,
    /// Maps stored values to linear values (DNG LinearizationTable).
    pub linearization: Option<Vec<u16>>,
    pub black: BlackLevels,
    /// White (saturation) level per sample.
    pub white: [f32; 3],
    /// Sensor area holding image data (DNG ActiveArea); black-level deltas are indexed from it.
    pub active: Rect,
    /// The area to output (ActiveArea ∩ DefaultCrop).
    pub crop: Rect,
    pub color: ColorInfo,
    /// White-balance multipliers from the camera's own metadata (non-DNG), R G B.
    pub camera_wb: Option<[f64; 3]>,
    /// TIFF orientation (1–8).
    pub orientation: u16,
    /// Exposure compensation baked into the default rendering (DNG BaselineExposure), in EV.
    pub baseline_exposure: f64,
    /// DNG OpcodeList2 gain maps (lens shading), applied to the normalized data.
    pub gain_maps: Vec<crate::opcodes::GainMap>,
    /// DNG MapPolynomial opcodes applied in the develop pipeline.
    pub polynomials: Vec<crate::opcodes::MapPolynomial>,
    /// The camera profile's default tone curve (like DNG ProfileToneCurve): increasing `[input,
    /// output]` pairs in linear ProPhoto, applied per channel after the colour conversion and
    /// exposure. Below the first point the curve is a line through 0; empty: no curve.
    pub tone_curve: Vec<[f32; 2]>,
    /// Camera Raw's adaptive black point, for profiles that copy its default rendering: this
    /// factor times the 0.1th percentile of the image's linear luminance is subtracted before the
    /// tone curve (see `tone.rs`). 0: no black point.
    pub black_point: f32,
    pub warnings: Vec<String>,
}

impl Sensor {
    pub fn sample_count(&self) -> usize {
        self.float_data.as_ref().map_or(self.data.len(), Vec::len)
    }

    #[inline]
    pub fn sample(&self, index: usize) -> f32 {
        self.float_data.as_ref().map_or_else(|| f32::from(self.data[index]), |samples| samples[index])
    }

    /// `true` for monochrome data (one-sample LinearRaw: monochrome cameras, scanner DNGs), which
    /// develops to neutral grey with no white balance or colour calibration.
    pub fn is_monochrome(&self) -> bool {
        self.samples == 1 && self.cfa.is_none()
    }
}
