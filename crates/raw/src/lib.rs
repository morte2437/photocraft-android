//! `photocraft-raw`: a clean-room, pure-Rust camera raw decoder and developer.
//!
//! Implemented only from public specifications and papers: TIFF 6.0,
//! TIFF/EP (ISO 12234-2), the Adobe DNG Specification 1.7, ITU-T T.81
//! (lossless JPEG, process 14), the published structure of Canon's CR2
//! container, published descriptions of Sony's cRAW code, public maker-note
//! and RAF tag tables, Nikon Huffman tables recovered by black-box analysis of CC0
//! samples (see `nefc.rs`), observation of sample files, and the demosaicing
//! papers cited in [`Demosaic`]. No code from dcraw, LibRaw, rawspeed,
//! rawler, rawloader or darktable was used.
//!
//! * [`identify`] / [`is_raw`] recognise raw files from their bytes.
//! * [`decode`] reads the undeveloped [`Sensor`] data and metadata.
//! * [`develop`] turns it into 16-bit RGB in ProPhoto RGB (ROMM primaries,
//!   D50 white, gamma 1.8): linearization, black/white levels, white balance,
//!   highlight clipping, demosaicing, camera → XYZ → ProPhoto, exposure and
//!   orientation.
//! * [`embedded_preview`] finds the camera's full-size JPEG preview, for
//!   formats whose sensor data is not decoded yet; [`embedded_previews`] lists
//!   every preview.
//! * [`fit_look`] fits a per-file colour matrix and tone curve ([`CameraLook`])
//!   to the camera's own (decoded) JPEG, for files without colour calibration
//!   (method after LightCraft's `docs/camera-preview-colour.md`,
//!   storytold/lightcraft#165); pass it in [`DevelopOptions::camera_look`].
//! * Raws that carry no colour description (NEF) take a camera profile we
//!   measured from the camera's own output when there is one (`cameras.rs`:
//!   image area, levels, ForwardMatrix, tone curve; the Nikon D4 so far).
//!   Bodies whose full profile is not measured yet can still carry a measured
//!   image area (the D800E, the ILCE-7).
//!
//! Decoded today: DNG (uncompressed, deflate, lossless-JPEG, lossy-JPEG and JPEG XL, strips and
//! tiles, CFA and LinearRaw), CR2 (lossless JPEG with Canon slices, and sRAW / mRAW), uncompressed or
//! lossless-JPEG TIFF/EP raws (NEF, ARW, PEF… when not vendor-compressed),
//! Nikon compressed NEF (lossless and lossy), Sony compressed ARW (cRAW),
//! Panasonic RW2 (RawFormat 5 packed and RawFormat 3/4 vendor-compressed),
//! uncompressed Olympus ORF, uncompressed Fujifilm RAF (Bayer and
//! X-Trans), and Hasselblad 3FR/FFF (the X2D II 100C's uncompressed 16-bit CFA;
//! see [`crate::hasselblad`]). Everything else reports [`RawError::Unsupported`].
//!
//! The crate is standalone (no workspace dependencies), does no I/O, builds for
//! `wasm32-unknown-unknown` and never panics on hostile input: sizes are
//! checked against [`Limits`] before allocating.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod cameras;
mod color;
mod demosaic;
mod develop;
mod error;
mod look;
mod opcodes;
mod par;
mod saturation;
mod sensor;
mod shared;
mod tone;

#[cfg(feature = "testgen")]
pub mod testgen;

pub use crate::color::{Calibration, ColorInfo, Mat3};
pub use crate::demosaic::Demosaic;
pub use crate::develop::{DevelopOptions, Developed, RawInfo, WhiteBalance, develop_sensor};
pub use crate::error::RawError;
pub use crate::look::{CameraLook, LookRejected, PROXY_EDGE, Reference, ToneCurve, fit_look};
pub use crate::opcodes::{GainMap, MapPolynomial};
pub use crate::sensor::{BlackLevels, Cfa, Rect, Sensor};
pub use crate::tone::{Tone, retone};
pub use craft_raw::{
    DecodedPreview, PreviewColorSpace, embedded_jpeg_preview, embedded_jpeg_preview_covering, embedded_preview, embedded_preview_color_space,
    embedded_preview_covering,
};

/// Name of the output colour space of [`develop`].
pub const OUTPUT_SPACE: &str = "ProPhoto RGB (ROMM primaries, D50, gamma 1.8)";

/// The raw container's orientation, when recorded (CR3 metadata or TIFF IFD0).
/// Useful for camera JPEGs that carry no orientation of their own.
pub fn orientation(bytes: &[u8]) -> Option<u16> {
    craft_raw::orientation(bytes)
}

/// Camera raw container families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawFormat {
    Dng,
    /// Canon CR2.
    Cr2,
    /// Canon CR3 (ISO BMFF).
    Cr3,
    /// Nikon NEF / NRW.
    Nef,
    /// Sony ARW / SR2.
    Arw,
    /// Pentax PEF.
    Pef,
    /// Olympus ORF.
    Orf,
    /// Panasonic RW2.
    Rw2,
    /// Fujifilm RAF.
    Raf,
    /// Hasselblad 3FR / FFF.
    Fff,
    /// Another TIFF/EP raw.
    TiffEp,
}

impl RawFormat {
    pub fn name(self) -> &'static str {
        match self {
            RawFormat::Dng => "DNG",
            RawFormat::Cr2 => "CR2",
            RawFormat::Cr3 => "CR3",
            RawFormat::Nef => "NEF",
            RawFormat::Arw => "ARW",
            RawFormat::Pef => "PEF",
            RawFormat::Orf => "ORF",
            RawFormat::Rw2 => "RW2",
            RawFormat::Raf => "RAF",
            RawFormat::Fff => "3FR/FFF",
            RawFormat::TiffEp => "TIFF/EP raw",
        }
    }
}

/// File extensions of the raw formats this crate recognises.
pub const EXTENSIONS: &[&str] = &["dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "pef", "orf", "rw2", "raf", "3fr", "fff"];

/// Decompression-bomb guards, checked on header values before allocating
/// (the same shape and defaults as `photocraft-codecs`' limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    /// Maximum `width * height`.
    pub max_pixels: u64,
    /// Maximum bytes of one buffer.
    pub max_alloc: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_width: 1 << 18, max_height: 1 << 18, max_pixels: 1 << 28, max_alloc: 2 << 30 }
    }
}

impl Limits {
    /// Checks a `width × height` buffer of `bytes_per_pixel` bytes per pixel.
    pub fn check(&self, width: u64, height: u64, bytes_per_pixel: u64) -> Result<(), RawError> {
        if width == 0 || height == 0 {
            return Err(RawError::Malformed(format!("zero-sized image {width}x{height}")));
        }
        if width > u64::from(self.max_width) || height > u64::from(self.max_height) {
            return Err(RawError::LimitExceeded(format!("dimensions {width}x{height} exceed {}x{}", self.max_width, self.max_height)));
        }
        let pixels = width.saturating_mul(height);
        if pixels > self.max_pixels {
            return Err(RawError::LimitExceeded(format!("{pixels} pixels exceed {}", self.max_pixels)));
        }
        let bytes = pixels.saturating_mul(bytes_per_pixel);
        if bytes > self.max_alloc || usize::try_from(bytes).is_err() {
            return Err(RawError::LimitExceeded(format!("{bytes} bytes exceed {}", self.max_alloc)));
        }
        Ok(())
    }
}

/// Recognises a camera raw file from its bytes (cheap: header and IFDs only).
pub fn identify(bytes: &[u8]) -> Option<RawFormat> {
    craft_raw::probe(bytes).map(crate::shared::format)
}

/// `true` if `bytes` look like a camera raw file.
pub fn is_raw(bytes: &[u8]) -> bool {
    craft_raw::probe(bytes).is_some()
}

/// Reads the undeveloped sensor data and metadata.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Sensor, RawError> {
    let max_samples = usize::try_from(limits.max_pixels.saturating_mul(3).min(limits.max_alloc / 4)).unwrap_or(usize::MAX);
    let mut shared_limits = craft_raw::Limits::default();
    shared_limits.max_samples = max_samples;
    let info = craft_raw::probe_info_with(bytes, &shared_limits).map_err(shared_error)?;
    if info.width > limits.max_width as usize || info.height > limits.max_height as usize {
        return Err(RawError::LimitExceeded(format!("dimensions {}×{} exceed {}×{}", info.width, info.height, limits.max_width, limits.max_height)));
    }
    let image = craft_raw::decode_with(bytes, &shared_limits).map_err(shared_error)?;
    Ok(crate::shared::adapt(image))
}

/// Decode a DNG JPEG XL preview under the app's existing allocation limits.
pub fn embedded_jxl_preview(bytes: &[u8], limits: &Limits) -> Option<DecodedPreview> {
    let mut shared = craft_raw::Limits::default();
    shared.max_samples = usize::try_from(limits.max_pixels.saturating_mul(3).min(limits.max_alloc / 2)).unwrap_or(usize::MAX);
    craft_raw::embedded_jxl_preview(bytes, &shared)
}

fn shared_error(error: craft_raw::RawError) -> RawError {
    match error {
        craft_raw::RawError::NotRaw => RawError::NotRaw,
        craft_raw::RawError::Unsupported(message) => RawError::Unsupported(message),
        craft_raw::RawError::Corrupt(message) => RawError::Malformed(message),
        craft_raw::RawError::Limit(message) => RawError::LimitExceeded(message.to_string()),
        craft_raw::RawError::Tiff(error) => RawError::Malformed(error.to_string()),
    }
}

/// Decodes and develops a raw file.
pub fn develop(bytes: &[u8], opts: &DevelopOptions) -> Result<Developed, RawError> {
    let sensor = decode(bytes, &opts.limits)?;
    develop_sensor(&sensor, opts)
}
