use super::paths::{same_file, write_file};
use super::{Document, EngineError, Layer, LayerContent, Result, Session, Value, f64_param, flattened_checked, json, str_param};

/// Whether saving `doc` as a TIFF writes Photoshop layer data (anything beyond a lone
/// Background layer). The UI asks before such a save when the "ask before saving layered TIFF"
/// preference is on; see `photocraft_io::tiff_layers`.
pub fn tiff_would_write_layers(doc: &Document) -> bool {
    photocraft_io::tiff_layers::would_write_layers(doc)
}

/// What a headless save writes beyond the format: JPEG quality and format, TIFF layers, Targa
/// bits, DPX bits, DDS compression and mipmaps, PSD previews and the PSD merged image.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SaveOpts {
    /// Photoshop's 0–12 JPEG scale.
    pub quality: Option<f64>,
    /// JPEG Options › Format Options (`"jpegFormat"`); unset, Baseline ("Standard").
    pub jpeg_format: Option<photocraft_codecs::JpegFormat>,
    /// TIFF: keep the layers. Off unless a command's params ask (`"tiffLayers": true`).
    pub tiff_layers: bool,
    /// Targa: 24 or 32 bits per pixel (`"tgaBits"`); unset, 32 with an alpha channel or
    /// transparency, else 24.
    pub tga_bits: Option<photocraft_io::TgaBits>,
    /// DPX: 8, 10, 12 or 16 bits per sample (`"dpxBits"`); unset, 8 for an 8-bit document, else 16.
    pub dpx_bits: Option<photocraft_io::DpxBits>,
    /// TIFF: the image compression (`"tiffCompression": "none" | "lzw" | "zip"`); unset, none.
    pub tiff_compression: Option<photocraft_codecs::TiffCompression>,
    /// DDS: the compression and whether to write the mip chain (`"ddsCompression"`,
    /// `"ddsMipmaps"`); unset, uncompressed without mipmaps.
    pub dds: photocraft_io::DdsOptions,
    /// PSD/PSB: embed the thumbnail resource (Preferences › File Handling › Image Previews).
    pub image_previews: bool,
    /// PSD/PSB: write the real merged image (Preferences › File Handling › Maximize PSD and PSB
    /// File Compatibility).
    pub psd_maximize_compatibility: bool,
}

impl Default for SaveOpts {
    fn default() -> Self {
        SaveOpts {
            quality: None,
            jpeg_format: None,
            tiff_layers: false,
            tga_bits: None,
            dpx_bits: None,
            tiff_compression: None,
            dds: photocraft_io::DdsOptions::default(),
            image_previews: true,
            psd_maximize_compatibility: true,
        }
    }
}

impl SaveOpts {
    /// `quality`, `jpegFormat`, `tiffLayers` and `tgaBits` from command `cmd`'s params (a
    /// `jpegFormat` it doesn't name or a `tgaBits` other than 24 or 32 is an error); the image
    /// previews and the PSD merged image from the session's File Handling preferences (Ask When
    /// Saving has no dialog here, so it saves them, as Photoshop's Save As checkbox and its
    /// Maximize Compatibility checkbox both start on).
    pub(crate) fn from_params(s: &Session, p: &Value, cmd: &str) -> Result<Self> {
        let file_handling = &s.prefs().file_handling;
        Ok(SaveOpts {
            quality: f64_param(p, "quality"),
            jpeg_format: jpeg_format_param(p, cmd)?,
            tiff_layers: p.get("tiffLayers").and_then(Value::as_bool).unwrap_or(false),
            tga_bits: tga_bits_param(p, cmd)?,
            dpx_bits: dpx_bits_param(p, cmd)?,
            tiff_compression: tiff_compression_param(p, cmd)?,
            dds: dds_param(p, cmd)?.unwrap_or_default(),
            image_previews: file_handling.image_previews != crate::prefs::Ask::Never,
            psd_maximize_compatibility: file_handling.maximize_psd_compatibility != crate::prefs::Ask::Never,
        })
    }

    pub(crate) fn or_quality(mut self, q: f64) -> Self {
        self.quality = self.quality.or(Some(q));
        self
    }
}

impl From<Option<f64>> for SaveOpts {
    fn from(quality: Option<f64>) -> Self {
        SaveOpts { quality, ..Default::default() }
    }
}

/// The Targa bits per pixel in `p` (`"tgaBits": 24 | 32`; absent or null: the automatic choice).
/// Anything else is a bad parameter of command `cmd`. Shared by every save that takes it (the
/// engine's commands, the app's Save As and control channel, the headless server).
pub fn tga_bits_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::TgaBits>> {
    match p.get("tgaBits") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(photocraft_io::TgaBits::from_bits)
            .map(Some)
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("`tgaBits` must be 24 or 32, got {v}") }),
    }
}

/// The TIFF compression in `p` (`"tiffCompression": "none" | "lzw" | "zip"`, Photoshop's TIFF
/// Options; absent or null: none). Anything else is a bad parameter of command `cmd` (#5301).
pub fn tiff_compression_param(p: &Value, cmd: &str) -> Result<Option<photocraft_codecs::TiffCompression>> {
    use photocraft_codecs::TiffCompression;
    match p.get("tiffCompression") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_str() {
            Some("none") => Ok(Some(TiffCompression::None)),
            Some("lzw") => Ok(Some(TiffCompression::Lzw)),
            Some("zip") => Ok(Some(TiffCompression::Deflate)),
            _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("`tiffCompression` must be \"none\", \"lzw\" or \"zip\", got {v}") }),
        },
    }
}

/// The JPEG format in `p` (`"jpegFormat": "baseline" | "optimized" | "progressive"`; absent or
/// null: Baseline). Anything else is a bad parameter of command `cmd`. Shared like
/// [`tga_bits_param`].
pub fn jpeg_format_param(p: &Value, cmd: &str) -> Result<Option<photocraft_codecs::JpegFormat>> {
    match p.get("jpegFormat") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(photocraft_codecs::JpegFormat::parse).map(Some).ok_or_else(|| EngineError::BadParams {
            cmd: cmd.into(),
            msg: format!("`jpegFormat` must be \"baseline\", \"optimized\" or \"progressive\", got {v}"),
        }),
    }
}

/// Photoshop's 0–12 JPEG quality as the encoder's 1–100 (0 → 1, 12 → 100, linear between; out of
/// range is clamped, and NaN or infinite is 100).
pub fn jpeg_quality(q: f64) -> u8 {
    if q.is_finite() { (q.clamp(0.0, 12.0) / 12.0 * 99.0 + 1.0).round() as u8 } else { 100 }
}

/// The DPX bits per sample in `p` (`"dpxBits": 8 | 10 | 12 | 16`; absent or null: the document's
/// depth). Anything else is a bad parameter of command `cmd`.
pub fn dpx_bits_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::DpxBits>> {
    match p.get("dpxBits") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(photocraft_io::DpxBits::from_bits)
            .map(Some)
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("`dpxBits` must be 8, 10, 12 or 16, got {v}") }),
    }
}

/// The DDS options in `p`: `"ddsCompression": "none" | "bc1" | "bc2" | "bc3"` (or the DXT names)
/// and `"ddsMipmaps": bool`. `None` when neither is given (Save As then asks); either alone
/// leaves the other at its default (uncompressed, no mipmaps). Anything else is a bad parameter
/// of command `cmd`.
pub fn dds_param(p: &Value, cmd: &str) -> Result<Option<photocraft_io::DdsOptions>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let compression = match p.get("ddsCompression") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .and_then(photocraft_io::DdsCompression::from_name)
                .ok_or_else(|| bad(format!("`ddsCompression` must be \"none\", \"bc1\", \"bc2\" or \"bc3\", got {v}")))?,
        ),
    };
    let mipmaps = match p.get("ddsMipmaps") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(format!("`ddsMipmaps` must be true or false, got {v}")))?),
    };
    if compression.is_none() && mipmaps.is_none() {
        return Ok(None);
    }
    Ok(Some(photocraft_io::DdsOptions { compression: compression.unwrap_or_default(), mipmaps: mipmaps.unwrap_or(false) }))
}

/// Encodes `doc` for `path`'s extension.
pub(crate) fn encode(doc: &Document, path: &str, save: impl Into<SaveOpts>) -> Result<(Vec<u8>, Vec<String>)> {
    crate::allocation::checkpoint("saving document")?;
    let save = save.into();
    let mut opts = photocraft_io::ExportOptions {
        tiff_layers: save.tiff_layers,
        tga_bits: save.tga_bits,
        image_previews: save.image_previews,
        psd_maximize_compatibility: save.psd_maximize_compatibility,
        // Linked files a PSD embeds are resolved against the document being worked on.
        document_path: crate::doc_path::current(),
        // An untrusted session's stored paths are document data: embed none of them.
        read_linked_files: !crate::jobs::authorization_installed(),
        ..Default::default()
    };
    // No quality chosen: write the file the way it was opened (a JPEG at its own quality, a WebP
    // lossy or lossless as it was), as Photoshop's Save does.
    photocraft_io::apply_file_encoding(doc, &mut opts.encode);
    opts.encode.dpx_bits = save.dpx_bits;
    if let Some(c) = save.tiff_compression {
        opts.encode.tiff_compression = c;
    }
    opts.encode.dds = save.dds;
    if let Some(q) = save.quality {
        let q = jpeg_quality(q);
        opts.encode.jpeg_quality = q;
        // A quality on a WebP save asks for the lossy encoder; the default WebP stays lossless.
        opts.encode.webp_quality = q;
        opts.encode.webp_lossless = false;
    }
    if let Some(f) = save.jpeg_format {
        opts.encode.jpeg_format = f;
    }
    photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

pub(crate) fn save_doc(doc: &Document, path: &str, save: impl Into<SaveOpts>) -> Result<Vec<String>> {
    let (bytes, warnings) = encode(doc, path, save)?;
    write_file(path, &bytes)?;
    Ok(warnings)
}

pub(super) fn save_a_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path", "file.saveACopy")?.to_string();
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // A copy over the document's own file replaces it, and a flat or lossy format can't be
    // undone, so it takes an explicit `overwrite` (#7437). The desktop dialog writes through the
    // shell, after the OS has confirmed the replace.
    if d.path.as_deref().is_some_and(|own| same_file(&path, own)) && p.get("overwrite").and_then(Value::as_bool) != Some(true) {
        return Err(EngineError::BadParams {
            cmd: "file.saveACopy".into(),
            msg: "`path` is the document's own file, so the copy would replace it: choose another path, or pass \"overwrite\": true".into(),
        });
    }
    let mut doc = (*d.doc).clone();
    if p.get("layers").and_then(Value::as_bool) == Some(false) {
        // "Layers" unchecked in Save a Copy: write the flattened image.
        let fmt = doc.pixel_format();
        let px = flattened_checked(&doc, fmt, "file.saveACopy")?;
        doc.layers = vec![Layer::new("Background", LayerContent::Raster(px))];
    }
    let warnings = save_doc(&doc, &path, SaveOpts::from_params(s, p, "file.saveACopy")?)?;
    Ok(json!({"path": path, "warnings": warnings}))
}
