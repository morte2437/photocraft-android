//! JPEG XL-compressed DNG tiles and strips (Adobe DNG Specification 1.7, Compression 52546).
//!
//! Each strip or tile holds one JPEG XL codestream (bare or in the ISO/IEC 18181-2 container)
//! whose image is the segment's samples: one channel for CFA data, three for LinearRaw. Samsung
//! Expert RAW writes LinearRaw this way. Decoding is done by `jxl-oxide`, a pure-Rust decoder;
//! like heic-rs in `photocraft-heif`, every call into it runs under `catch_unwind` so a decoder
//! panic on a malformed file becomes [`RawError::Malformed`].
//!
//! The browser build (wasm32) leaves the decoder out: jxl-oxide added 0.88 MB to a wasm that
//! must stay under the per-file cap of static hosts (packaging/web/package.sh), for a format only
//! a few phones write. There, these segments are [`RawError::Unsupported`] with a clear message.

#[cfg(not(target_arch = "wasm32"))]
use jxl_oxide::{AllocTracker, JxlImage};

use crate::Limits;
use crate::error::{RawError, Result};

/// The browser build has no JPEG XL decoder (see the module docs).
#[cfg(target_arch = "wasm32")]
pub(crate) fn decode(_src: &[u8], _w: usize, _h: usize, _samples: usize, _bits: u32, _float_scale: Option<f32>, _limits: &Limits) -> Result<Vec<u16>> {
    Err(RawError::unsupported("JPEG XL-compressed DNG (DNG 1.7) can't be opened in the browser version of PhotoCraft; open it in the desktop app"))
}

/// Decodes one segment into `w * h * samples` interleaved samples at `bits` per sample (the
/// TIFF BitsPerSample, which DNG requires the codestream's bit depth to match). Floating-point
/// samples (SampleFormat 3) come back multiplied by `float_scale`, clipped to 0..=65535.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode(src: &[u8], w: usize, h: usize, samples: usize, bits: u32, float_scale: Option<f32>, limits: &Limits) -> Result<Vec<u16>> {
    // Floating-point samples are read into an f32 buffer first: four bytes a sample, not two.
    let sample_bytes = if float_scale.is_some() { 4 } else { 2 };
    limits.check(w as u64, h as u64, samples as u64 * sample_bytes)?;
    // jxl-oxide decodes into f32/i32 planes: allow a few times the output size, and never more
    // than the caller's per-buffer limit.
    let budget = (w as u64).saturating_mul(h as u64).saturating_mul(samples as u64).saturating_mul(16).min(limits.max_alloc);
    let budget = usize::try_from(budget).unwrap_or(usize::MAX);
    // jxl-oxide inflates Brotli-compressed boxes (`brob`: Exif, XMP) into storage the tracker
    // doesn't see, while it reads. A raw tile has no use for metadata, so refuse it unread.
    if has_compressed_box(src) {
        return Err(RawError::unsupported("JPEG XL raw tile with compressed metadata boxes"));
    }
    guarded(|| {
        let image =
            JxlImage::builder().alloc_tracker(AllocTracker::with_limit(budget)).read(src).map_err(|e| RawError::malformed(format!("JPEG XL raw data: {e}")))?;
        // Rendering samples spline geometry into an untracked vector with no bound on the sample
        // count, whatever their colour; sensor data never draws splines.
        if (0..image.num_loaded_frames()).filter_map(|i| image.frame(i)).any(|f| f.header().flags.splines()) {
            return Err(RawError::unsupported("JPEG XL raw tile with splines"));
        }
        let (jw, jh) = (image.width() as usize, image.height() as usize);
        if (jw, jh) != (w, h) {
            return Err(RawError::malformed(format!("JPEG XL raw tile is {jw}x{jh}, expected {w}x{h}")));
        }
        let render = image.render_frame(0).map_err(|e| RawError::malformed(format!("JPEG XL raw data: {e}")))?;
        let mut stream = render.stream_no_alpha();
        let channels = stream.channels() as usize;
        if channels != samples {
            return Err(RawError::unsupported(format!("JPEG XL raw tile with {channels} channels for {samples} samples per pixel")));
        }
        // Orientation is applied by the stream; DNG tiles are stored upright, so anything else
        // would not fit the tile grid.
        if (stream.width() as usize, stream.height() as usize) != (w, h) {
            return Err(RawError::unsupported("rotated JPEG XL raw tile"));
        }
        let n = w.checked_mul(h).and_then(|p| p.checked_mul(samples)).ok_or_else(|| RawError::malformed("JPEG XL raw tile is too large"))?;
        if let Some(scale) = float_scale {
            let mut f = vec![0f32; n];
            if stream.write_to_buffer(&mut f) != n {
                return Err(RawError::malformed("JPEG XL raw data is truncated"));
            }
            return Ok(f.into_iter().map(|v| if v.is_nan() { 0 } else { (v * scale).clamp(0.0, 65535.0).round() as u16 }).collect());
        }
        let mut out = vec![0u16; n];
        if stream.write_to_buffer(&mut out) != n {
            return Err(RawError::malformed("JPEG XL raw data is truncated"));
        }
        // 16-bit samples come back as stored; other depths are scaled to 0..=65535, so scale
        // them back to the TIFF's range (exact for lossless data: 65535 / (2^bits - 1) >= 1).
        if bits < 16 {
            let max = f64::from((1u32 << bits) - 1);
            for v in &mut out {
                *v = (f64::from(*v) * max / 65535.0).round() as u16;
            }
        }
        Ok(out)
    })
}

/// The ISO/IEC 18181-2 container's signature box.
#[cfg(not(target_arch = "wasm32"))]
const CONTAINER_SIGNATURE: &[u8] = &[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Whether `src` is a JPEG XL container holding a Brotli-compressed (`brob`) box. A malformed box
/// size ends the walk: the decoder refuses that file itself.
#[cfg(not(target_arch = "wasm32"))]
fn has_compressed_box(src: &[u8]) -> bool {
    if !src.starts_with(CONTAINER_SIGNATURE) {
        return false;
    }
    let mut at = 0usize;
    while let Some(head) = src.get(at..at.saturating_add(8)) {
        if &head[4..] == b"brob" {
            return true;
        }
        let size = match u32::from_be_bytes([head[0], head[1], head[2], head[3]]) {
            0 => return false, // the last box, running to the end
            1 => match src.get(at + 8..at.saturating_add(16)).and_then(|b| <[u8; 8]>::try_from(b).ok()) {
                Some(b) => u64::from_be_bytes(b),
                None => return false,
            },
            n => u64::from(n),
        };
        match usize::try_from(size).ok().filter(|&s| s >= 8).and_then(|s| at.checked_add(s)) {
            Some(next) => at = next,
            None => return false,
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(RawError::malformed("the JPEG XL decoder failed on this file")))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{CONTAINER_SIGNATURE, decode, has_compressed_box};
    use crate::Limits;

    fn container(boxes: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = CONTAINER_SIGNATURE.to_vec();
        for (kind, body) in boxes {
            out.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
            out.extend_from_slice(*kind);
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn a_compressed_metadata_box_is_refused_before_decoding() {
        let brob = container(&[(b"ftyp", b"jxl \0\0\0\0jxl "), (b"brob", b"Exif\x8b\x02\x80"), (b"jxlc", &[0xFF, 0x0A])]);
        assert!(has_compressed_box(&brob));
        assert!(decode(&brob, 1, 1, 1, 16, None, &Limits::default()).is_err());
        assert!(!has_compressed_box(&container(&[(b"ftyp", b"jxl \0\0\0\0jxl "), (b"jxlc", &[0xFF, 0x0A])])));
        // A bare codestream is not a container.
        assert!(!has_compressed_box(&[0xFF, 0x0A, 0, 0, 0, 0, b'b', b'r', b'o', b'b']));
    }
}
