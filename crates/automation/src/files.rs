//! File I/O shared by the MCP server and the CLI: open/save any supported
//! format, with `.pcraft` (ZIP or directory bundle) handled natively.

use std::path::Path;

use photocraft_doc::Document;
use photocraft_format::{PcraftWriter, SaveOptions};
use photocraft_io::{BandRenderer, ExportOptions};

use crate::AutomationError;

/// Result of opening a file.
pub struct Opened {
    pub document: Document,
    pub warnings: Vec<String>,
    pub source_read_only: bool,
}

/// Decode a document that was read through a filesystem capability.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Opened, AutomationError> {
    open_bytes_with_options(name, bytes, &photocraft_io::ImportOptions::default())
}

/// Decode a document that was read through a filesystem capability with custom import options.
pub fn open_bytes_with_options(name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions) -> Result<Opened, AutomationError> {
    if Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
        let document = photocraft_format::load_from_bytes(bytes)?;
        return Ok(Opened { warnings: photocraft_io::open_warnings(&document), document, source_read_only: false });
    }
    let r = photocraft_io::import_with_options(name, bytes, opts)?;
    Ok(Opened { document: r.document, warnings: r.warnings, source_read_only: r.source_read_only })
}

/// Open a document from disk. Directory bundles and `.pcraft` ZIPs load
/// natively; everything else goes through `photocraft-io` (PSD, PNG, …).
pub fn open(path: &Path) -> Result<Opened, AutomationError> {
    open_with_options(path, &photocraft_io::ImportOptions::default())
}

/// Open a document from disk with custom import options.
pub fn open_with_options(path: &Path, opts: &photocraft_io::ImportOptions) -> Result<Opened, AutomationError> {
    if path.is_dir() {
        let document = photocraft_format::load_path(path)?;
        return Ok(Opened { warnings: photocraft_io::open_warnings(&document), document, source_read_only: false });
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // A PSD/PSB is parsed from the open file (channel data read as it is decoded), so a CLI
    // convert or info of a 2 GB file does not hold the file in memory next to the document.
    #[cfg(any(unix, windows))]
    {
        let r = photocraft_io::import_file(&name, path, opts, &photocraft_raster::Interrupt::NONE).map_err(|e| match e {
            photocraft_io::IoError::Read(msg) => AutomationError::Io(msg),
            e => e.into(),
        })?;
        Ok(Opened { document: r.document, warnings: r.warnings, source_read_only: r.source_read_only })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let bytes = photocraft_format::read_file(path).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
        open_bytes_with_options(&name, &bytes, opts)
    }
}

/// Previews for a `.pcraft` bundle.
pub fn previews(doc: &Document) -> SaveOptions {
    SaveOptions { thumbnail: Some(photocraft_compose::thumbnail(doc, 256)), composite: Some(photocraft_compose::thumbnail(doc, 1024)), ..Default::default() }
}

/// [`previews`], plus the full-size composite when `opts` asks for it and it can be rendered
/// ([`ExportOptions::pcraft_full_composite`]). Fails when a scratch tile of `doc` could not be
/// read while they were rendered: the previews may then hold placeholder pixels, and the
/// bundle's own scratch checks only cover reads made after it starts.
fn native_options(doc: &Document, opts: &ExportOptions) -> Result<SaveOptions, AutomationError> {
    doc.scratch_health().map_err(AutomationError::Io)?;
    let generation = photocraft_raster::spill::read_error_generation();
    let mut out = previews(doc);
    if opts.pcraft_full_composite {
        out.full_composite = photocraft_io::pcraft_full_composite(doc);
        out.full_composite_icc = out.full_composite.as_ref().map(|_| photocraft_io::pcraft_full_composite_icc(doc));
    }
    photocraft_raster::spill::check_since(generation).map_err(AutomationError::Io)?;
    doc.scratch_health().map_err(AutomationError::Io)?;
    Ok(out)
}

/// Encode the name the saved native bundle will report on reopening. Save As updates
/// the session identity only *after* a successful write; without a separate snapshot
/// the manifest serializes the previous name instead.
fn named_for_output(doc: &Document, path: &str) -> Document {
    let mut named = doc.clone();
    if let Some(name) = path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()) {
        named.name = name.to_owned();
    }
    named
}

/// Save or export `doc` to `path`, choosing the format from the extension
/// (or `format_override`, an extension such as `"png"`). `writer` makes
/// repeated `.pcraft` saves incremental. Returns warnings about lost data.
pub fn save(
    doc: &Document,
    path: &Path,
    format_override: Option<&str>,
    opts: &ExportOptions,
    writer: Option<&mut PcraftWriter>,
) -> Result<Vec<String>, AutomationError> {
    save_with_renderer(doc, path, format_override, opts, writer, None)
}

/// [`save`] with an optional custom [`BandRenderer`].
pub fn save_with_renderer(
    doc: &Document,
    path: &Path,
    format_override: Option<&str>,
    opts: &ExportOptions,
    writer: Option<&mut PcraftWriter>,
    mut renderer: Option<&mut dyn BandRenderer>,
) -> Result<Vec<String>, AutomationError> {
    let mut kept = Vec::new();
    let res = save_with_renderer_keeping(doc, path, format_override, opts, writer, renderer.as_mut().map(|r| &mut **r as &mut dyn BandRenderer), &mut kept);
    if res.is_err()
        && let Some(r) = renderer
    {
        // Callers without their own store still see the reason through the renderer.
        for warning in kept {
            r.push_diagnostic(warning);
        }
    }
    res
}

/// [`save_with_renderer`] that hands the export warnings of an encode whose write then failed
/// to the caller in `kept`, independent of what the renderer does with
/// [`BandRenderer::push_diagnostic`] (the default is a no-op).
pub fn save_with_renderer_keeping(
    doc: &Document,
    path: &Path,
    format_override: Option<&str>,
    opts: &ExportOptions,
    writer: Option<&mut PcraftWriter>,
    renderer: Option<&mut dyn BandRenderer>,
    kept: &mut Vec<String>,
) -> Result<Vec<String>, AutomationError> {
    let ext = format_override
        .map(|f| f.trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()))
        .ok_or_else(|| AutomationError::BadRequest(format!("cannot tell the format of `{}`; pass a format", path.display())))?;
    if ext == photocraft_format::EXTENSION {
        let named = named_for_output(doc, &path.to_string_lossy());
        let mut local = PcraftWriter::new();
        let w = writer.unwrap_or(&mut local);
        w.save_path(&named, path, &native_options(&named, opts)?)?;
        let mut warnings = Vec::new();
        if let Some(r) = renderer {
            warnings.push("native .pcraft format uses CPU compositor for previews".into());
            warnings.extend(r.drain_diagnostics());
        }
        return Ok(warnings);
    }
    let Some(renderer) = renderer else {
        let r = photocraft_io::export_with_renderer(doc, &ext, opts, &photocraft_raster::Interrupt::NONE, None)?;
        // Crash-safe: a failed write never destroys the previous file.
        photocraft_format::atomic_write(path, &r.bytes).map_err(|e| AutomationError::Io(e.to_string()))?;
        return Ok(r.warnings);
    };
    let r = photocraft_io::export_with_renderer(doc, &ext, opts, &photocraft_raster::Interrupt::NONE, Some(&mut *renderer))?;
    if let Err(e) = photocraft_format::atomic_write(path, &r.bytes) {
        // Encoding drained the renderer's diagnostics into `r.warnings`; hand them to the
        // caller so it can still report the adapter or fallback reason alongside the write error.
        kept.extend(r.warnings);
        return Err(AutomationError::Io(e.to_string()));
    }
    Ok(r.warnings)
}

/// Encode a document for a capability-scoped write. Unlike [`save`], this
/// never obtains ambient filesystem authority and emits `.pcraft` as a ZIP.
pub fn save_bytes(doc: &Document, name: &str, format_override: Option<&str>, opts: &ExportOptions) -> Result<(Vec<u8>, Vec<String>), AutomationError> {
    save_bytes_with_renderer(doc, name, format_override, opts, None)
}

/// [`save_bytes`] with an optional custom [`BandRenderer`].
pub fn save_bytes_with_renderer(
    doc: &Document,
    name: &str,
    format_override: Option<&str>,
    opts: &ExportOptions,
    renderer: Option<&mut dyn BandRenderer>,
) -> Result<(Vec<u8>, Vec<String>), AutomationError> {
    let ext = format_override
        .map(|format| format.trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| Path::new(name).extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()))
        .ok_or_else(|| AutomationError::BadRequest(format!("cannot tell the format of `{name}`; pass a format")))?;
    if ext == photocraft_format::EXTENSION {
        let named = named_for_output(doc, name);
        // Encode before draining: if the options check or the encode fails, the queued
        // diagnostics stay with the renderer for the caller to report alongside the error.
        let bytes = photocraft_format::save_to_bytes(&named, &native_options(&named, opts)?)?;
        let mut warnings = Vec::new();
        if let Some(r) = renderer {
            warnings.push("native .pcraft format uses CPU compositor for previews".into());
            warnings.extend(r.drain_diagnostics());
        }
        return Ok((bytes, warnings));
    }
    let result = photocraft_io::export_with_renderer(doc, &ext, opts, &photocraft_raster::Interrupt::NONE, renderer)?;
    Ok((result.bytes, result.warnings))
}

/// Flattened document as PNG, scaled to fit `max_side` (0 = full size).
pub fn render_png(doc: &Document, max_side: u32) -> Result<Vec<u8>, AutomationError> {
    render_png_with_renderer(doc, max_side, None)
}

/// [`render_png`] with an optional custom [`BandRenderer`]. Preview fallback warnings are
/// queued on the renderer; use [`render_png_with_warnings`] to receive them directly.
pub fn render_png_with_renderer(doc: &Document, max_side: u32, mut renderer: Option<&mut dyn BandRenderer>) -> Result<Vec<u8>, AutomationError> {
    let (png, warnings) = render_png_with_warnings(doc, max_side, renderer.as_mut().map(|r| &mut **r as &mut dyn BandRenderer))?;
    if let Some(r) = renderer {
        for w in warnings {
            r.push_diagnostic(w);
        }
    }
    Ok(png)
}

/// [`render_png_with_renderer`] that returns the preview fallback warnings with the bytes.
pub fn render_png_with_warnings(doc: &Document, max_side: u32, mut renderer: Option<&mut dyn BandRenderer>) -> Result<(Vec<u8>, Vec<String>), AutomationError> {
    let side = if max_side == 0 { doc.size.width.max(doc.size.height) } else { max_side };
    let (target_w, target_h) = photocraft_compose::thumbnail_size(doc, side.max(1));
    let mut warnings = Vec::new();

    if let Some(r) = renderer.as_mut()
        && (target_w, target_h) == (doc.size.width, doc.size.height)
    {
        // 8-bit RGBA whatever the document's depth, like the CPU preview path: a 16-bit
        // document's preview must not be twice as large as the one it previews.
        let res = photocraft_io::export_preview_png_with_renderer(doc, Some(&mut **r));
        match res {
            Ok(res) => return Ok((res.bytes, res.warnings)),
            Err(photocraft_io::IoError::Cancelled) => return Err(AutomationError::Other("rendering was cancelled".into())),
            Err(e) => return Err(e.into()),
        }
    }

    if renderer.is_some() {
        warnings.push("scaled previews and thumbnails use CPU compositor; GPU rendering applies to full-size previews".into());
    }

    let img = photocraft_compose::thumbnail(doc, side.max(1));
    let image = photocraft_codecs::Image::from_u8(img.width, img.height, photocraft_codecs::ChannelLayout::Rgba, img.pixels)
        .map_err(|e| AutomationError::Other(e.to_string()))?;
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| AutomationError::Other(e.to_string()))?;
    Ok((png, warnings))
}
