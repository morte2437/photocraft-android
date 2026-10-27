//! Develop's per-pixel stages resolved for one image: every constant and table they use, so
//! another implementation of them (the GPU preview, `photocraft_gpu::CameraRawDevelop`) runs the
//! same maths. [`develop`](super::develop) builds its own constants with the same functions.

use super::{CameraRaw, Encoding, Wheel, curve_lut, hsv_to_rgb, is_linear, luma, parametric_lut, white_balance_gains, whites_blacks};

/// Entries of the tone curve tables (parametric and point curves).
pub const CURVE_LUT: usize = 1024;
/// Samples of a Whites or Blacks curve row: grey levels 0, 1/32, …, 1.
pub const WB_STEPS: usize = whites_blacks::STEPS;

/// White balance and exposure: channel gains in linear light, when the stage is on.
pub(crate) fn gains(p: &CameraRaw) -> Option<[f32; 3]> {
    if p.wb_neutral() {
        return None;
    }
    let g = white_balance_gains(p.temperature, p.tint);
    let norm = luma(g);
    Some(g.map(|v| v / norm * 2f32.powf(p.exposure)))
}

/// Contrast's strength, when Contrast is set.
fn contrast(p: &CameraRaw) -> Option<f32> {
    (p.contrast != 0.0).then_some(p.contrast / 100.0 * 0.6)
}

/// Whites and Blacks as two curve rows for the image's amounts.
#[derive(Clone, Debug, PartialEq)]
pub struct WhitesBlacks {
    /// The Whites curve over grey levels 0, 1/32, …, 1, when Whites is set.
    pub whites: Option<[f32; WB_STEPS]>,
    /// The Blacks curve, applied after Whites.
    pub blacks: Option<[f32; WB_STEPS]>,
    /// The clipping directions (Whites up, Blacks down) curve each channel; the others scale the
    /// pixel by the change of its luminance.
    pub per_channel: bool,
}

/// Color Grading's wheels resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Grade {
    /// Colour offsets of the shadows, midtones, highlights and global wheels.
    pub tints: [[f32; 3]; 4],
    /// Luminance of the same wheels (slider units).
    pub lums: [f32; 4],
    pub blend: f32,
    pub balance: f32,
}

impl Grade {
    pub(crate) fn new(p: &CameraRaw) -> Self {
        let tint = |wh: &Wheel| -> [f32; 3] {
            let c = hsv_to_rgb(wh.hue, 1.0, 1.0);
            let l = luma(c);
            [c[0] - l, c[1] - l, c[2] - l].map(|v| v * wh.sat / 100.0 * 0.3)
        };
        Grade {
            tints: [tint(&p.grade_shadows), tint(&p.grade_midtones), tint(&p.grade_highlights), tint(&p.grade_global)],
            lums: [p.grade_shadows.lum, p.grade_midtones.lum, p.grade_highlights.lum, p.grade_global.lum],
            blend: 0.15 + p.grade_blending.clamp(0.0, 100.0) / 100.0 * 0.35,
            balance: p.grade_balance.clamp(-100.0, 100.0) / 100.0 * 0.25,
        }
    }
}

/// The vignette's `vignette_style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VignetteStyle {
    HighlightPriority,
    ColorPriority,
    PaintOverlay,
}

/// The post-crop vignette's shape on a `w × h` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Vignette {
    /// −1 … 1.
    pub amount: f32,
    /// 0 … 1.
    pub highlights: f32,
    pub style: VignetteStyle,
    /// Frame centre (pixels).
    pub centre: [f32; 2],
    /// Half axes of the shape (pixels).
    pub axes: [f32; 2],
    /// Exponent of the superellipse.
    pub power: f32,
    /// The mask's smoothstep edges in normalised radius.
    pub edges: [f32; 2],
}

impl Vignette {
    pub(crate) fn new(w: usize, h: usize, p: &CameraRaw) -> Self {
        let mid = p.vignette_midpoint.clamp(0.0, 100.0) / 100.0;
        let feather = p.vignette_feather.clamp(0.0, 100.0) / 100.0;
        let round = p.vignette_roundness.clamp(-100.0, 100.0) / 100.0;
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        // Roundness: 0 follows the frame's aspect, +1 a circle, −1 a rounded rectangle.
        let (ax, ay) = if round >= 0.0 {
            let r = cx.min(cy) + (cx.max(cy) - cx.min(cy)) * (1.0 - round);
            (if cx >= cy { r } else { cx }, if cy > cx { r } else { cy })
        } else {
            (cx, cy)
        };
        let inner = 1.0 - mid * 0.8;
        Vignette {
            amount: p.vignette_amount / 100.0,
            highlights: p.vignette_highlights.clamp(0.0, 100.0) / 100.0,
            style: match p.vignette_style.as_str() {
                "paintOverlay" => VignetteStyle::PaintOverlay,
                "colorPriority" => VignetteStyle::ColorPriority,
                _ => VignetteStyle::HighlightPriority,
            },
            centre: [cx, cy],
            axes: [ax, ay],
            power: 2.0 + (-round).max(0.0) * 6.0,
            edges: [inner - feather * 0.5 * inner, inner + feather * 0.5 * (1.4 - inner) + 0.02],
        }
    }
}

/// Grain: value noise sampled in full-resolution coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Grain {
    /// Noise cell size (full-resolution pixels).
    pub cell: f32,
    /// Weight of the finer octave.
    pub roughness: f32,
    pub amount: f32,
    pub seed: u64,
    /// Full-resolution pixels per pixel (`1 / pixel_scale`).
    pub scale: f32,
}

impl Grain {
    pub(crate) fn new(p: &CameraRaw) -> Self {
        Grain {
            cell: 1.0 + p.grain_size / 25.0,
            roughness: p.grain_roughness / 100.0,
            amount: p.grain_amount / 100.0 * 0.12,
            seed: u64::from(p.seed),
            scale: 1.0 / p.pixel_scale.clamp(0.01, 1.0),
        }
    }
}

/// Develop's per-pixel stages for one image and settings, in order. `None` stages are off.
#[derive(Clone, Debug, PartialEq)]
pub struct PerPixel {
    /// 1. White balance and exposure gains in linear light (sRGB-decoded values).
    pub gains: Option<[f32; 3]>,
    /// 3. Contrast's strength (Contrast without Texture or Clarity is per pixel).
    pub contrast: Option<f32>,
    /// 3b. Whites and Blacks, in the document's encoding.
    pub whites_blacks: Option<WhitesBlacks>,
    /// The document's transfer curve: 0 for sRGB's, else a power-law gamma.
    pub encoding: f32,
    /// 4. Vibrance and Saturation (slider / 100).
    pub presence: Option<[f32; 2]>,
    /// 5. The parametric tone curve, [`CURVE_LUT`] entries over 0 … 1.
    pub parametric: Option<Vec<f32>>,
    /// 5. Point curves: master, red, green, blue.
    pub point: [Option<Vec<f32>>; 4],
    /// 6. Color Mixer hue, saturation and luminance per band.
    pub hsl: Option<[[f32; 8]; 3]>,
    /// 7. Color Grading.
    pub grade: Option<Grade>,
    /// 9. Vignette.
    pub vignette: Option<Vignette>,
    /// 9. Grain.
    pub grain: Option<Grain>,
    /// Clip RGB to 0 … 1 at the end (all but 32-bit documents).
    pub clamp: bool,
}

impl CameraRaw {
    /// Whether developing needs a stage that reads neighbouring pixels: Dehaze, Shadows and
    /// Highlights, Texture, Clarity, sharpening or noise reduction.
    pub fn spatial(&self) -> bool {
        self.dehaze != 0.0
            || [self.highlights, self.shadows, self.texture, self.clarity].iter().any(|v| *v != 0.0)
            || self.sharpen_amount > 0.0
            || self.noise_luminance > 0.0
            || self.noise_color > 0.0
    }
}

/// The image's black and white points that Whites and Blacks adapt to, for `px` (`w × h`,
/// display-encoded) developed per pixel with `p`: what [`develop`](super::develop) measures
/// after white balance and Contrast. `None` when Whites and Blacks are 0 or `p` needs a
/// spatial stage.
pub fn white_black_points(px: &[[f32; 4]], w: usize, h: usize, p: &CameraRaw) -> Option<(f32, f32)> {
    if (p.whites == 0.0 && p.blacks == 0.0) || p.spatial() || px.len() != w.saturating_mul(h) {
        return None;
    }
    let enc = Encoding::of(p.encoding_gamma);
    let (g, k_con) = (gains(p), contrast(p));
    Some(super::luminance_points(px, w, h, enc, |q| {
        let q = g.map_or(q, |g| super::balance(q, g));
        k_con.map_or(q, |k| super::contrast(q, k))
    }))
}

/// Develop's stages for settings without spatial stages, on a `w × h` image; `points` are the
/// image's [`white_black_points`]. `None` when `p` needs a spatial stage or Calibration (these
/// stages don't run it), or Whites or Blacks is set without `points`. The points themselves
/// leave Calibration out, so a caller may cache them across Calibration changes.
pub fn per_pixel(p: &CameraRaw, w: usize, h: usize, float: bool, points: Option<(f32, f32)>) -> Option<PerPixel> {
    if p.spatial() || !p.calibration_neutral() {
        return None;
    }
    let identity = p.is_identity();
    let whites_blacks = if p.whites != 0.0 || p.blacks != 0.0 {
        let (bp, wp) = points?;
        let (xw, xb) = whites_blacks::amounts(p.whites.clamp(-100.0, 100.0), p.blacks.clamp(-100.0, 100.0), bp, wp);
        (xw != 0.0 || xb != 0.0).then(|| WhitesBlacks {
            whites: (xw != 0.0).then(|| whites_blacks::whites_row(xw)),
            blacks: (xb != 0.0).then(|| whites_blacks::blacks_row(xb)),
            per_channel: xw > 0.0 || xb < 0.0,
        })
    } else {
        None
    };
    let parametric_on = !(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0);
    let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, CURVE_LUT));
    Some(PerPixel {
        gains: gains(p),
        contrast: contrast(p),
        whites_blacks,
        encoding: Encoding::of(p.encoding_gamma).0,
        presence: (p.vibrance != 0.0 || p.saturation != 0.0).then_some([p.vibrance / 100.0, p.saturation / 100.0]),
        parametric: parametric_on.then(|| parametric_lut(p, CURVE_LUT)),
        point: [pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue)],
        hsl: (!p.hsl_neutral()).then_some([p.hsl_hue, p.hsl_sat, p.hsl_lum]),
        grade: (!p.grade_neutral()).then(|| Grade::new(p)),
        vignette: (p.vignette_amount != 0.0).then(|| Vignette::new(w, h, p)),
        grain: (p.grain_amount > 0.0).then(|| Grain::new(p)),
        clamp: !float && !identity,
    })
}
