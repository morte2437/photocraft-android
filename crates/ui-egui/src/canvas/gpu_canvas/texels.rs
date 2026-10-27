use super::*;

/// Straight-alpha f32 RGBA (sRGB-encoded) → premultiplied RGBA8, split across threads on native.
pub fn premultiply_rgba8(px: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    texels_into(FORMAT, px, &mut out);
    out
}

/// Straight-alpha f32 RGBA (sRGB-encoded) → premultiplied RGBA16F (little-endian), split across
/// threads on native. Colour values above 1.0 are kept (32-bit documents); negatives and NaN
/// become 0.
pub fn premultiply_rgba16f(px: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    texels_into(FORMAT_HIGH, px, &mut out);
    out
}

/// Bytes per texel of a canvas texture format.
pub(super) fn texel_bytes(format: wgpu::TextureFormat) -> u64 {
    if format == FORMAT_HIGH { 8 } else { 4 }
}

/// Straight-alpha f32 RGBA → premultiplied texels of `format`.
pub(super) fn texels(format: wgpu::TextureFormat, px: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    texels_into(format, px, &mut out);
    out
}

/// [`texels`] into `out` (resized; reusing it across bands saves page-faulting a fresh buffer).
pub(super) fn texels_into(format: wgpu::TextureFormat, px: &[[f32; 4]], out: &mut Vec<u8>) {
    fn rgba8(src: &[[f32; 4]], dst: &mut [u8]) {
        for (o, p) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src) {
            let a = p[3].clamp(0.0, 1.0);
            o[0] = (p[0].clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
            o[1] = (p[1].clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
            o[2] = (p[2].clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
            o[3] = (a * 255.0 + 0.5) as u8;
        }
    }
    fn rgba16f(src: &[[f32; 4]], dst: &mut [u8]) {
        for (o, p) in dst.as_chunks_mut::<8>().0.iter_mut().zip(src) {
            let a = if p[3].is_nan() { 0.0 } else { p[3].clamp(0.0, 1.0) };
            let [r, g, b, a] = [p[0].max(0.0) * a, p[1].max(0.0) * a, p[2].max(0.0) * a, a].map(|v| f32_to_f16(v).to_le_bytes());
            *o = [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]];
        }
    }
    let bpp = texel_bytes(format) as usize;
    let convert = if format == FORMAT_HIGH { rgba16f } else { rgba8 };
    out.clear();
    out.resize(px.len() * bpp, 0);
    #[cfg(not(target_arch = "wasm32"))]
    if px.len() > 1 << 18 {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
        let chunk = px.len().div_ceil(threads);
        std::thread::scope(|s| {
            for (src, dst) in px.chunks(chunk).zip(out.chunks_mut(chunk * bpp)) {
                s.spawn(move || convert(src, dst));
            }
        });
        return;
    }
    convert(px, out);
}

/// IEEE 754 binary32 → binary16, round to nearest even; overflow saturates to the largest
/// finite half (65504) so HDR values never become infinite, NaN becomes 0. Branch-light: the
/// rounding is done with integer adds on the f32 bits (and an f32 add for subnormals).
pub(super) fn f32_to_f16(v: f32) -> u16 {
    // 2^-14: the smallest normal half; below it the result is subnormal (or zero).
    const MIN_NORMAL: u32 = 113 << 23;
    // 2^16: halves overflow from here (65520 and up round to infinity; saturate instead).
    const OVERFLOW: u32 = 143 << 23;
    // Adding 0.5 (as f32) shifts a subnormal half's bits to the bottom of the mantissa.
    const DENORM_MAGIC: u32 = 126 << 23;
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let a = bits & 0x7fff_ffff;
    if a > 0x7f80_0000 {
        return 0;
    }
    let h = if a >= OVERFLOW {
        0x7bff
    } else if a < MIN_NORMAL {
        (f32::from_bits(a) + f32::from_bits(DENORM_MAGIC)).to_bits().wrapping_sub(DENORM_MAGIC) as u16
    } else {
        // Rebias the exponent and round the 13 dropped mantissa bits to nearest even.
        let odd = (a >> 13) & 1;
        (((a.wrapping_add(0xc800_0fff).wrapping_add(odd)) >> 13).min(0x7bff)) as u16
    };
    sign | h
}

/// IEEE 754 binary16 → binary32.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let man = f32::from(h & 0x3ff);
    sign * match exp {
        0 => man * 2f32.powi(-24),
        0x1f => {
            if man == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

/// One stored texel of `format` as f32 RGBA.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn texel_to_f32(format: wgpu::TextureFormat, b: &[u8]) -> [f32; 4] {
    let mut o = [0.0f32; 4];
    for (i, v) in o.iter_mut().enumerate() {
        *v = if format == FORMAT_HIGH {
            f16_to_f32(u16::from_le_bytes([b.get(i * 2).copied().unwrap_or(0), b.get(i * 2 + 1).copied().unwrap_or(0)]))
        } else {
            f32::from(b.get(i).copied().unwrap_or(0)) / 255.0
        };
    }
    o
}

// ---------------------------------------------------------------------------------------------
// GPU resources

/// The GPU memory a refresh's layer pages need against what they may hold (bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerFit {
    /// The document refreshed.
    pub doc: u64,
    pub needed: u64,
    pub budget: u64,
}

impl LayerFit {
    pub fn fits(&self) -> bool {
        self.needed <= self.budget
    }
}
