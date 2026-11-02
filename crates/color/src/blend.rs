//! Photoshop and Paint.NET blend modes (scalar reference implementation).
//!
//! Formulas follow the public PDF / ISO 32000-2 and W3C Compositing definitions,
//! with Photoshop's variants where they differ (Hard Mix, Darker/Lighter Color,
//! Linear Burn/Dodge, Vivid/Linear/Pin Light, Subtract, Divide). This is the CPU reference
//! that the GPU kernels must match.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendMode {
    /// Groups only: children blend directly into the group's backdrop.
    PassThrough,
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Reflect,
    Glow,
    Negation,
    Xor,
    PaintNetColorBurn,
    PaintNetColorDodge,
}

impl BlendMode {
    /// Paint.NET modes without an exact Photoshop equivalent.
    pub const PAINT_NET_MODES: [BlendMode; 6] = [Self::Reflect, Self::Glow, Self::Negation, Self::Xor, Self::PaintNetColorBurn, Self::PaintNetColorDodge];

    /// All editable layer modes, with Paint.NET modes after Photoshop's menu order.
    pub fn layer_modes() -> impl Iterator<Item = Self> {
        Self::LAYER_MODES.into_iter().chain(Self::PAINT_NET_MODES)
    }

    pub fn has_psd_equivalent(self) -> bool {
        !Self::PAINT_NET_MODES.contains(&self)
    }
    /// Photoshop modes except PassThrough, in Photoshop menu order.
    pub const LAYER_MODES: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::PassThrough => "Pass Through",
            BlendMode::Normal => "Normal",
            BlendMode::Dissolve => "Dissolve",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::LinearBurn => "Linear Burn",
            BlendMode::DarkerColor => "Darker Color",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::LinearDodge => "Linear Dodge (Add)",
            BlendMode::LighterColor => "Lighter Color",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::HardLight => "Hard Light",
            BlendMode::VividLight => "Vivid Light",
            BlendMode::LinearLight => "Linear Light",
            BlendMode::PinLight => "Pin Light",
            BlendMode::HardMix => "Hard Mix",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
            BlendMode::Reflect => "Reflect",
            BlendMode::Glow => "Glow",
            BlendMode::Negation => "Negation",
            BlendMode::Xor => "XOR",
            BlendMode::PaintNetColorBurn => "Color Burn (Paint.NET)",
            BlendMode::PaintNetColorDodge => "Color Dodge (Paint.NET)",
        }
    }

    /// PSD 4-character blend key. Modes without an equivalent fall back to Normal;
    /// exporters must report that loss using [`Self::has_psd_equivalent`].
    pub fn psd_key(self) -> [u8; 4] {
        *match self {
            BlendMode::PassThrough => b"pass",
            BlendMode::Normal => b"norm",
            BlendMode::Dissolve => b"diss",
            BlendMode::Darken => b"dark",
            BlendMode::Multiply => b"mul ",
            BlendMode::ColorBurn => b"idiv",
            BlendMode::LinearBurn => b"lbrn",
            BlendMode::DarkerColor => b"dkCl",
            BlendMode::Lighten => b"lite",
            BlendMode::Screen => b"scrn",
            BlendMode::ColorDodge => b"div ",
            BlendMode::LinearDodge => b"lddg",
            BlendMode::LighterColor => b"lgCl",
            BlendMode::Overlay => b"over",
            BlendMode::SoftLight => b"sLit",
            BlendMode::HardLight => b"hLit",
            BlendMode::VividLight => b"vLit",
            BlendMode::LinearLight => b"lLit",
            BlendMode::PinLight => b"pLit",
            BlendMode::HardMix => b"hMix",
            BlendMode::Difference => b"diff",
            BlendMode::Exclusion => b"smud",
            BlendMode::Subtract => b"fsub",
            BlendMode::Divide => b"fdiv",
            BlendMode::Hue => b"hue ",
            BlendMode::Saturation => b"sat ",
            BlendMode::Color => b"colr",
            BlendMode::Luminosity => b"lum ",
            BlendMode::Reflect | BlendMode::Glow | BlendMode::Negation | BlendMode::Xor | BlendMode::PaintNetColorBurn | BlendMode::PaintNetColorDodge => {
                b"norm"
            }
        }
    }

    pub fn from_psd_key(key: [u8; 4]) -> Option<BlendMode> {
        std::iter::once(BlendMode::PassThrough).chain(Self::LAYER_MODES).find(|m| m.psd_key() == key)
    }

    pub fn is_separable(self) -> bool {
        !matches!(self, BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity | BlendMode::DarkerColor | BlendMode::LighterColor)
    }
}

/// Separable per-channel blend function `B(cb, cs)`.
#[inline]
pub fn blend_channel(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    channel(mode, cb, cs, 1.0)
}

/// [`blend_channel`] for 32-bit float documents, whose values may exceed 1: Linear Dodge (Add)
/// and Divide don't clip at 1 there (Photoshop's 32-bit Add is how light is added in HDR), only
/// at `f32::MAX`, so they stay finite. Every other mode is [`blend_channel`].
#[inline]
pub fn blend_channel_hdr(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    channel(mode, cb, cs, f32::MAX)
}

/// `B(cb, cs)` with Linear Dodge and Divide clipped at `hi` (1 at integer depths).
#[inline]
fn channel(mode: BlendMode, cb: f32, cs: f32, hi: f32) -> f32 {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough => cs,
        BlendMode::Darken => cb.min(cs),
        BlendMode::Multiply => cb * cs,
        BlendMode::ColorBurn => color_burn(cb, cs),
        BlendMode::LinearBurn => (cb + cs - 1.0).max(0.0),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::ColorDodge => color_dodge(cb, cs),
        BlendMode::LinearDodge => (cb + cs).min(hi),
        BlendMode::Overlay => hard_light(cs, cb),
        BlendMode::SoftLight => soft_light_ps(cb, cs),
        BlendMode::HardLight => hard_light(cb, cs),
        BlendMode::VividLight if cs <= 0.0 => 0.0,
        BlendMode::VividLight if cs >= 1.0 => 1.0,
        BlendMode::VividLight => {
            if cs <= 0.5 {
                color_burn(cb, 2.0 * cs)
            } else {
                color_dodge(cb, 2.0 * (cs - 0.5))
            }
        }
        BlendMode::LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
        BlendMode::PinLight => {
            if cs <= 0.5 {
                cb.min(2.0 * cs)
            } else {
                cb.max(2.0 * cs - 1.0)
            }
        }
        BlendMode::HardMix => {
            if vivid_light_hard_mix(cb, cs) >= 0.5 - 1e-6 {
                1.0
            } else {
                0.0
            }
        }
        BlendMode::Difference => (cb - cs).abs(),
        BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        BlendMode::Subtract => (cb - cs).max(0.0),
        BlendMode::Divide => {
            if cs <= 0.0 {
                if cb <= 0.0 { 0.0 } else { 1.0 }
            } else {
                (cb / cs).min(hi)
            }
        }
        BlendMode::Reflect => reflect(cb, cs),
        BlendMode::Glow => reflect(cs, cb),
        BlendMode::Negation => 1.0 - (1.0 - cb - cs).abs(),
        BlendMode::Xor => {
            // Paint.NET's XOR is defined on 8-bit channels even in a floating-point compositor.
            let b = (cb.clamp(0.0, 1.0) * 255.0).round() as u8;
            let s = (cs.clamp(0.0, 1.0) * 255.0).round() as u8;
            f32::from(b ^ s) / 255.0
        }
        BlendMode::PaintNetColorBurn => {
            if cs <= 0.0 {
                0.0
            } else {
                (1.0 - (1.0 - cb) / cs).max(0.0)
            }
        }
        BlendMode::PaintNetColorDodge => {
            if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        // Non-separable modes are handled by `blend_rgb`; fall back to source.
        BlendMode::DarkerColor | BlendMode::LighterColor | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
    }
}

fn reflect(cb: f32, cs: f32) -> f32 {
    if cs >= 1.0 { 1.0 } else { (cb * cb / (1.0 - cs)).min(1.0) }
}

#[inline]
fn vivid_light_hard_mix(cb: f32, cs: f32) -> f32 {
    const EDGE: f32 = 1e-4;
    if cs <= 0.5 {
        if cb >= 1.0 - EDGE {
            1.0
        } else if cs <= 0.0 {
            0.0
        } else {
            1.0 - ((1.0 - cb) / (2.0 * cs)).min(1.0)
        }
    } else if cb <= EDGE {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (2.0 * (1.0 - cs))).min(1.0)
    }
}

#[inline]
fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

#[inline]
fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

#[inline]
fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

/// Photoshop's Soft Light: the W3C/PDF curve. In the upper half (`cs > 0.5`) the backdrop moves
/// towards `D(cb)`, which is `sqrt(cb)` above a quarter and the cubic `((16·cb − 12)·cb + 4)·cb`
/// below it; with `sqrt` throughout, dark backdrops came out up to 20 levels too light (#8804,
/// measured on psd-tools `blend-modes/rgb-blend-modes.psd`).
#[inline]
fn soft_light_ps(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        2.0 * cb * cs + cb * cb * (1.0 - 2.0 * cs)
    } else {
        let b = cb.max(0.0);
        let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
        2.0 * cb * (1.0 - cs) + d * (2.0 * cs - 1.0)
    }
}

// ---- non-separable helpers (W3C Compositing Level 1) ----

#[inline]
pub fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// ClipColor: moves `c` towards its grey ([`lum`]) until every channel is within `0..=top`,
/// keeping the luminosity. `top` is the brightest in-gamut value: 1, or `f32::MAX` in 32-bit
/// documents (values above 1 are valid light there, so only negatives are pulled in).
///
/// The scale factors `l / (l − min)` and `(top − l) / (max − l)` lie in 0..1 only while the
/// luminosity itself is in range. Outside it no colour of that luminosity is in gamut: the plain
/// formula turned colours inside out (a grey of 2 as the Luminosity of (0.5, 0.25, 0.125) gave
/// (1.0, 2.32, 2.99)) and, for greys, divided by the rounding noise of `max − l`, so results were
/// arbitrary and differed between the CPU and the GPU. Such colours become their grey instead
/// (the formula's value as the luminosity reaches the bound).
fn clip_color(c: [f32; 3], top: f32) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        let d = l - n;
        for v in &mut out {
            *v = if l > 0.0 && d > 0.0 { l + (*v - l) * l / d } else { l };
        }
    }
    if x > top {
        let d = x - l;
        for v in &mut out {
            *v = if l < top && d > 0.0 { l + (*v - l) * (top - l) / d } else { l };
        }
    }
    out
}

/// `c` with its luminosity ([`lum`]) moved to `l`, clipped back into gamut towards grey (the
/// Luminosity blend mode's SetLum + ClipColor).
pub fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    set_lum_within(c, l, 1.0)
}

/// [`set_lum`] clipping at `top` instead of 1 (see [`blend_rgb_within`]).
fn set_lum_within(c: [f32; 3], l: f32, top: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d], top)
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap_or(std::cmp::Ordering::Equal));
    let (imin, imid, imax) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    if c[imax] > c[imin] {
        out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
        out[imax] = s;
    }
    out[imin] = 0.0;
    out
}

/// Blend a whole RGB triple: `B(Cb, Cs)` including non-separable modes.
pub fn blend_rgb(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    blend_rgb_within(mode, cb, cs, 1.0)
}

/// [`blend_rgb`] with Hue / Saturation / Color / Luminosity clipped at `top` instead of 1:
/// `f32::MAX` keeps the values above 1 of 32-bit documents (their gamut has no upper bound).
pub fn blend_rgb_within(mode: BlendMode, cb: [f32; 3], cs: [f32; 3], top: f32) -> [f32; 3] {
    match mode {
        BlendMode::Hue => set_lum_within(set_sat(cs, sat(cb)), lum(cb), top),
        BlendMode::Saturation => set_lum_within(set_sat(cb, sat(cs)), lum(cb), top),
        BlendMode::Color => set_lum_within(cs, lum(cb), top),
        BlendMode::Luminosity => set_lum_within(cb, lum(cs), top),
        BlendMode::DarkerColor => {
            if lum(cs) < lum(cb) {
                cs
            } else {
                cb
            }
        }
        BlendMode::LighterColor => {
            if lum(cs) > lum(cb) {
                cs
            } else {
                cb
            }
        }
        m => [blend_channel(m, cb[0], cs[0]), blend_channel(m, cb[1], cs[1]), blend_channel(m, cb[2], cs[2])],
    }
}

/// [`blend_rgb`] for 32-bit float documents: separable modes use [`blend_channel_hdr`], and the
/// non-separable ones keep values above 1 ([`blend_rgb_within`] at `f32::MAX`).
pub fn blend_rgb_hdr(mode: BlendMode, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    if mode.is_separable() { std::array::from_fn(|i| blend_channel_hdr(mode, cb[i], cs[i])) } else { blend_rgb_within(mode, cb, cs, f32::MAX) }
}

/// Composite a straight-alpha source over a straight-alpha backdrop with blend mode and opacity.
///
/// Uses the W3C/PDF general formula:
/// `αo = αs + αb(1−αs)`,
/// `αo·Co = (1−αs)·αb·Cb + (1−αb)·αs·Cs + αs·αb·B(Cb,Cs)`.
/// Dissolve must be resolved by the caller (per-pixel coverage decision) before calling.
pub fn composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let ab = backdrop[3];
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let cb = [backdrop[0], backdrop[1], backdrop[2]];
    let cs = [source[0], source[1], source[2]];
    let b = blend_rgb(mode, cb, cs);
    let ao = as_ + ab * (1.0 - as_);
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0f32; 4];
    for i in 0..3 {
        let premul = (1.0 - as_) * ab * cb[i] + (1.0 - ab) * as_ * cs[i] + as_ * ab * b[i];
        out[i] = premul / ao;
    }
    out[3] = ao;
    out
}

/// [`composite`] with the backdrop's alpha kept (Lock Transparent Pixels, Fill's Preserve
/// Transparency): "source atop", `Co = (1−αs)·Cb + αs·B(Cb,Cs)`, `αo = αb`. Unlike source-over
/// followed by restoring the alpha, a half-transparent backdrop takes the new colour by `αs` alone.
pub fn composite_atop(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4], opacity: f32) -> [f32; 4] {
    let as_ = source[3] * opacity;
    if as_ <= 0.0 {
        return backdrop;
    }
    let b = blend_rgb(mode, [backdrop[0], backdrop[1], backdrop[2]], [source[0], source[1], source[2]]);
    let mut out = backdrop;
    for i in 0..3 {
        out[i] = (1.0 - as_) * backdrop[i] + as_ * b[i];
    }
    out
}

#[cfg(test)]
mod tests;
