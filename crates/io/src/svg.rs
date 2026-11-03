//! SVG import: a drawing opens as a document of shape layers, and places as a vector smart
//! object that is re-rasterised at its placement scale ([`rasterize`]).
//!
//! Parsing is `usvg`'s (the crate the UI's icons already go through): it resolves CSS, `use`,
//! units, nested transforms and turns text into outlines, handing over a tree of groups, paths,
//! images and text. Each path becomes a [`ShapeLayer`] (cubic Bézier knots, fill, stroke) and
//! each group a layer group with its opacity and blend mode. What the shape model cannot hold,
//! clip paths, masks, filters, pattern paints and embedded images, is rasterised by `resvg` into
//! a pixel layer so the picture still looks right, with a warning; gradient paints keep their
//! stops and direction in the layer's own gradient fill, with a warning that the exact geometry
//! is not kept.

use std::sync::{Arc, Mutex, OnceLock};

use photocraft_color::blend::BlendMode;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::effects::GradientStyle;
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, StrokeAlign, Subpath, VectorMask};
use photocraft_doc::{Document, Fill, Group, Layer, LayerContent, MAX_GROUP_DEPTH};
use photocraft_geom::{Point, Rect, Size};
use photocraft_raster::Surface;
use resvg::tiny_skia;
use resvg::usvg;

use crate::{ImportResult, IoError};

/// Largest canvas side an SVG opens at; a larger drawing is scaled down to fit.
pub const MAX_SIDE: u32 = 16_384;
/// A rasterised fallback layer or a smart-object render never exceeds this many pixels.
const MAX_RASTER_PIXELS: u64 = 24_000_000;
/// Largest decoded byte size (RGBA8) accepted for an embedded raster image before resvg decodes
/// it. resvg/tiny-skia allocate the full intrinsic image before any destination-size scaling,
/// so a highly compressed PNG or WebP could otherwise trigger gigabyte-scale allocation and
/// decompression that the destination pixmap's size check never sees.
const MAX_EMBEDDED_IMAGE_BYTES: u64 = MAX_RASTER_PIXELS * 4;

/// The IHDR width/height of a PNG, read directly from its header (no decoding): the signature
/// (8 bytes), the IHDR chunk's length and type (8 bytes), then big-endian width and height.
fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let sig = data.get(0..8)?;
    if sig != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    if data.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// Intrinsic dimensions of a raster image resvg can embed, from its header alone, for formats
/// where decoding allocates the full image before any destination scaling (PNG, WebP).
/// Returns `Some(Some((w, h)))` when the format is PNG/WebP and its header parses cleanly,
/// `Some(None)` when the format is PNG/WebP (by MIME or magic bytes under `"text/plain"`)
/// and its header is malformed, or `None` for other formats.
/// A JPEG's size from its frame header (the first SOF marker), without decoding anything.
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    loop {
        if *data.get(i)? != 0xFF {
            return None;
        }
        let marker = *data.get(i + 1)?;
        match marker {
            0xFF => i += 1,
            // SOF0..SOF15, but for DHT (C4), JPG (C8) and DAC (CC).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let h = u16::from_be_bytes([*data.get(i + 5)?, *data.get(i + 6)?]);
                let w = u16::from_be_bytes([*data.get(i + 7)?, *data.get(i + 8)?]);
                return Some((u32::from(w), u32::from(h)));
            }
            // Start of scan or end of image before any frame header.
            0xD8..=0xDA => return None,
            0x01 | 0xD0..=0xD7 => i += 2,
            _ => i += 2 + usize::from(u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?])),
        }
    }
}

fn embedded_image_dimensions(mime: &str, data: &[u8]) -> Option<Option<(u32, u32)>> {
    match mime {
        "image/png" => Some(png_dimensions(data)),
        "image/webp" | "image/gif" => Some(photocraft_codecs::probe_size(data).ok()),
        "image/jpeg" | "image/jpg" => Some(jpeg_dimensions(data)),
        "text/plain" => {
            if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
                Some(png_dimensions(data))
            } else if (data.get(0..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WEBP")) || data.starts_with(b"GIF8") {
                Some(photocraft_codecs::probe_size(data).ok())
            } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
                Some(jpeg_dimensions(data))
            } else {
                None
            }
        }
        _ => None,
    }
}
/// Aggregate memory for retained fallback surfaces in the document's pixel format.
const MAX_PIXEL_BYTES: u64 = 1 << 30;
/// Estimated peak during raster conversion (retained surfaces + RGBA8 pixmap + f32 staging + tiles).
const MAX_PEAK_PIXEL_BYTES: u64 = 3 << 29;
/// Past this many drawable elements the whole drawing opens as one pixel layer.
const MAX_NODES: usize = 2_000;
/// Nesting past which a subtree is rasterised instead of walked (bounded recursion).
const MAX_RECURSION: usize = 200;
/// Deepest element nesting accepted. The XML parser recurses once per level, so a file nested
/// a few thousand deep (70 KB of `<g>`) overflows a 2 MB thread stack, an abort no guard can
/// catch; real drawings stay under a hundred levels.
pub const MAX_XML_DEPTH: usize = 512;
/// Largest `.svgz` accepted once inflated (a 1.5 MB gzip can hold 1.5 GB of whitespace).
pub const MAX_INFLATED_BYTES: u64 = 128 << 20;

/// Does this look like an SVG document (plain XML; `.svgz` is recognised by its extension)?
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(4096)).unwrap_or(bytes);
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<svg") {
        return true;
    }
    (t.starts_with("<?xml") || t.starts_with("<!DOCTYPE") || t.starts_with("<!--")) && t.contains("<svg")
}

/// The fonts text runs are shaped with: the bundled craft fonts plus, on the desktop, the
/// system's. Loaded once per process.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        for f in photocraft_text::CRAFT_FONTS {
            db.load_font_data(f.bytes.to_vec());
        }
        #[cfg(not(target_arch = "wasm32"))]
        db.load_system_fonts();
        resolve_generic_families(&mut db, system_generic_family);
        Arc::new(db)
    })
    .clone()
}

/// Installed families that stand in for each generic one, best first: the metric-compatible
/// Liberation and Croscore fonts, then common Linux defaults, then the bundled craft fonts.
const SANS_SERIF: &[&str] = &["Liberation Sans", "Arimo", "Helvetica", "DejaVu Sans", "Noto Sans", photocraft_text::fonts::DEFAULT_FAMILY];
const SERIF: &[&str] = &["Liberation Serif", "Tinos", "Times", "DejaVu Serif", "Noto Serif"];
const MONOSPACE: &[&str] = &["Liberation Mono", "Cousine", "Courier", "DejaVu Sans Mono", "Noto Sans Mono", photocraft_text::fonts::MONO_FAMILY];

/// Points the generic families (`sans-serif`, `serif`, `monospace`, `cursive`, `fantasy`) at
/// fonts the database holds. fontdb names Windows fonts for them (Arial, Times New Roman,
/// Courier New…): on a system without those, every family of a usual
/// `font-family="Arial, Helvetica, sans-serif"` misses, and usvg drops the text although
/// hundreds of fonts are installed, where a browser sets it in a fallback font. A generic whose
/// font is installed is left alone. Otherwise the system's choice (`system`, Fontconfig on Linux)
/// wins when the database has it, then the first installed stand-in, then the sans-serif choice,
/// then any font at all.
fn resolve_generic_families(db: &mut usvg::fontdb::Database, system: impl Fn(&str) -> Option<String>) {
    use usvg::fontdb::Family;
    let has = |db: &usvg::fontdb::Database, name: &str| db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
    let Some(any) = db.faces().find_map(|f| f.families.first()).map(|(n, _)| n.clone()) else { return };
    let mut sans = None;
    for (generic, css, stand_ins) in [
        (Family::SansSerif, "sans-serif", SANS_SERIF),
        (Family::Serif, "serif", SERIF),
        (Family::Monospace, "monospace", MONOSPACE),
        (Family::Cursive, "cursive", &[][..]),
        (Family::Fantasy, "fantasy", &[][..]),
    ] {
        let current = db.family_name(&generic).to_string();
        let pick = if has(db, &current) {
            current
        } else {
            system(css)
                .filter(|n| has(db, n))
                .or_else(|| stand_ins.iter().find(|n| has(db, n)).map(|n| n.to_string()))
                .or_else(|| sans.clone())
                .unwrap_or_else(|| any.clone())
        };
        match generic {
            Family::SansSerif => db.set_sans_serif_family(pick.clone()),
            Family::Serif => db.set_serif_family(pick.clone()),
            Family::Monospace => db.set_monospace_family(pick.clone()),
            Family::Cursive => db.set_cursive_family(pick.clone()),
            _ => db.set_fantasy_family(pick.clone()),
        }
        if generic == Family::SansSerif {
            sans = Some(pick);
        }
    }
}

/// The family Fontconfig sets a generic family in (`fc-match sans-serif`), as the rest of the
/// desktop does; `None` without Fontconfig.
#[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
fn system_generic_family(generic: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match").args(["--format=%{family[0]}", generic]).output().ok()?;
    let name = String::from_utf8(out.stdout).ok()?;
    Some(name.trim().to_string()).filter(|n| out.status.success() && !n.is_empty())
}

#[cfg(not(all(target_os = "linux", not(target_arch = "wasm32"))))]
fn system_generic_family(_generic: &str) -> Option<String> {
    None
}

/// Most font file bytes [`in_memory`] keeps; past it a face stays on disk (slow, still right).
#[cfg(not(target_arch = "wasm32"))]
const MAX_CACHED_FONT_BYTES: usize = 256 << 20;

/// `id`, or the same face loaded from memory when the database names a file for it. fontdb
/// re-reads a system font's whole file for every glyph usvg outlines (about a millisecond a
/// character with Times New Roman), so a short entity-expanded text could keep an import busy
/// for minutes. Each file is read once per process; the parse's own database copy then holds
/// every face of that file from memory.
#[cfg(not(target_arch = "wasm32"))]
fn in_memory(id: usvg::fontdb::ID, db: &mut Arc<usvg::fontdb::Database>) -> usvg::fontdb::ID {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, PoisonError};
    use usvg::fontdb::Source;
    type Files = (HashMap<PathBuf, Arc<Vec<u8>>>, usize);
    static FILES: OnceLock<Mutex<Files>> = OnceLock::new();
    static LOADING: Mutex<()> = Mutex::new(());
    let Some((Source::File(path), index)) = db.face_source(id) else { return id };
    let files = || FILES.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
    let fits = |files: &Files, len: usize| files.1.saturating_add(len) <= MAX_CACHED_FONT_BYTES;
    let cached = files().0.get(&path).cloned();
    // One miss at a time, so imports wanting the same file read it once: a second one waits
    // here, then takes the bytes the first one kept.
    let loading = cached.is_none().then(|| LOADING.lock().unwrap_or_else(PoisonError::into_inner));
    let cached = cached.or_else(|| files().0.get(&path).cloned());
    let bytes = match cached {
        Some(b) => b,
        None => {
            // The size first: a file past the budget isn't read at all, on this lookup or the next.
            let Ok(len) = std::fs::metadata(&path).map(|m| usize::try_from(m.len()).unwrap_or(usize::MAX)) else { return id };
            if !fits(&files(), len) {
                return id;
            }
            let Ok(b) = std::fs::read(&path) else { return id };
            let b = Arc::new(b);
            // The face has to load from the bytes before they're kept or the file's faces leave
            // the database: a file replaced or cut short since fontdb indexed it stays on disk.
            let mut probe = usvg::fontdb::Database::new();
            let ids = probe.load_font_source(Source::Binary(b.clone()));
            if !ids.iter().any(|&n| probe.face_source(n).is_some_and(|(_, i)| i == index)) {
                return id;
            }
            let mut files = files();
            if !fits(&files, b.len()) {
                return id;
            }
            files.1 = files.1.saturating_add(b.len());
            files.0.insert(path.clone(), b.clone());
            b
        }
    };
    drop(loading);
    let db = Arc::make_mut(db);
    let same_file: Vec<_> = db.faces().filter(|f| matches!(&f.source, Source::File(p) if *p == path)).map(|f| f.id).collect();
    // Last first: fontdb's slots are reused last-freed-first, so the file's faces come back in
    // their old places and order, and the fallback search (first face with the character)
    // still finds the face it finds on disk.
    for f in same_file.into_iter().rev() {
        db.remove_face(f);
    }
    let loaded = db.load_font_source(Source::Binary(bytes));
    loaded.into_iter().find(|&n| db.face_source(n).is_some_and(|(_, i)| i == index)).unwrap_or(id)
}

/// usvg's font choice, with the chosen faces read from memory ([`in_memory`]).
fn font_resolver() -> usvg::FontResolver<'static> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let select = usvg::FontResolver::default_font_selector();
        let fallback = usvg::FontResolver::default_fallback_selector();
        usvg::FontResolver {
            select_font: Box::new(move |font, db| select(font, db).map(|id| in_memory(id, db))),
            select_fallback: Box::new(move |c, used, db| fallback(c, used, db).map(|id| in_memory(id, db))),
        }
    }
    #[cfg(target_arch = "wasm32")]
    usvg::FontResolver::default()
}

/// Parse options for an untrusted drawing. `<image href>` resolves only `data:` URLs: a file path
/// or relative href is never read, so opening an SVG cannot pull local files (`/etc/...`,
/// `~/.ssh/...`) into the document. Embedded SVG images get no `<image>` resolution at all
/// (`usvg` already blanks it for sub-SVGs). An embedded SVG may nest `depth_left` levels: it is
/// parsed on top of the drawing's own recursion, so the two depths share [`MAX_XML_DEPTH`].
fn options(fontdb: Arc<usvg::fontdb::Database>, depth_left: usize) -> usvg::Options<'static> {
    let default_resolver = usvg::ImageHrefResolver::default_data_resolver();
    let resolve_data: usvg::ImageHrefDataResolverFn<'static> = Box::new(move |mime, data, opts| {
        // Reject embedded images whose decoded RGBA storage would exceed the budget before
        // resvg (which allocates the full intrinsic image before any destination scaling) ever
        // sees them. Formats without a cheap header probe fall through unchanged; their decode
        // path is bounded elsewhere or accepted as-is, matching the previous behavior.
        let probed = embedded_image_dimensions(mime, &data);
        if let Some(probed) = probed {
            let (w, h) = probed?;
            let decoded = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4);
            if decoded > MAX_EMBEDDED_IMAGE_BYTES {
                return None;
            }
        }
        // An embedded SVG (`image/svg+xml`, or `text/plain` that isn't one of the raster
        // formats probed above) is parsed by the default resolver with none of `parse_with`'s
        // checks, so they run here. It can itself be gzip-compressed (SVGZ-in-data-URL), and the
        // default resolver's sub-SVG loader would decompress again with no byte limit of its
        // own: hand it our bounded inflate's output instead, which no longer starts with the
        // gzip magic. Nesting deeper than `depth_left` drops the image rather than overflowing
        // the parser's stack.
        if mime == "image/svg+xml" || (mime == "text/plain" && probed.is_none()) {
            let (mime, xml) = match inflate(&data) {
                Ok(std::borrow::Cow::Borrowed(_)) => (mime, Arc::clone(&data)),
                Ok(std::borrow::Cow::Owned(xml)) if !xml.starts_with(&[0x1f, 0x8b]) => ("image/svg+xml", Arc::new(xml)),
                _ => return None,
            };
            if nesting_depth(&xml, depth_left) > depth_left {
                return None;
            }
            return default_resolver(mime, xml, opts);
        }
        default_resolver(mime, data, opts)
    });
    usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver { resolve_data, resolve_string: Box::new(|_, _| None) },
        fontdb,
        font_resolver: font_resolver(),
        ..usvg::Options::default()
    }
}

/// A parsed drawing.
#[derive(Debug)]
struct Parsed {
    tree: usvg::Tree,
    /// The drawing has text but no font at all was available to shape it. `usvg` then drops the
    /// text while parsing, before any node reaches the conversion, so this is the only place
    /// that knows why the text is missing.
    fontless_text: bool,
}

/// Parses a drawing, `.svgz` included. The XML is inflated and checked here, before the parser
/// sees it: deeper nesting than [`MAX_XML_DEPTH`] is an error, not a stack overflow. The font
/// database (and its one-off scan of the system fonts) is only loaded when the drawing may
/// contain text.
fn parse(bytes: &[u8]) -> Result<Parsed, IoError> {
    parse_with(bytes, fonts)
}

/// [`parse`] with the font database `fonts` supplies (the tests hand in an empty one).
fn parse_with(bytes: &[u8], fonts: fn() -> Arc<usvg::fontdb::Database>) -> Result<Parsed, IoError> {
    let xml = inflate(bytes)?;
    let depth = nesting_depth(&xml, MAX_XML_DEPTH);
    if depth > MAX_XML_DEPTH {
        return Err(IoError::Svg(format!("elements are nested more than {MAX_XML_DEPTH} levels deep")));
    }
    let has_text = xml.windows(4).any(|w| w == b"text");
    let fontdb = if has_text { fonts() } else { Arc::new(usvg::fontdb::Database::new()) };
    let fontless_text = has_text && fontdb.faces().next().is_none();
    let opt = options(fontdb, MAX_XML_DEPTH.saturating_sub(depth));
    // Parse the already-bounded-and-decoded text with from_str, not from_data: from_data
    // re-checks the gzip magic and would decompress a second time without our size limit if
    // the inflated content happened to start with a nested gzip stream.
    let text = std::str::from_utf8(&xml).map_err(|_| IoError::Svg("the SVG is not valid UTF-8".into()))?;
    let tree = usvg::Tree::from_str(text, &opt).map_err(|e| IoError::Svg(e.to_string()))?;
    Ok(Parsed { tree, fontless_text })
}

const MAX_BOUNDED_SVG_BYTES: u64 = 16 << 20;
const MAX_EMBEDDED_SVG_DEPTH: usize = 32;

struct ImageBudget {
    pixels: u64,
    xml_bytes: u64,
    svg_images: u64,
    /// The deepest chain of embedded SVG images (an SVG image inside an SVG image is 2). resvg
    /// draws each one through a scratch pixmap as large as the one it is drawn into and frees it
    /// afterwards, so this many such surfaces exist at once, however many siblings there are.
    svg_chain: u64,
    draw_nodes: u64,
    repeatable_raster: bool,
    repeatable_svg: bool,
    fontless_text: bool,
    /// An embedded image that can't be read fails the drawing instead of being left out. A
    /// stored display preview must draw completely, or it isn't used.
    strict: bool,
    error: Option<String>,
}

/// Leave out an embedded image that can't be read, or with a strict budget record `error`.
fn unreadable_image(budget: &Mutex<ImageBudget>, error: &str) -> Option<usvg::ImageKind> {
    if budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner).strict {
        image_error(budget, error);
    }
    None
}

/// Record the first budget or decoding failure seen while usvg resolves images. Resolver
/// callbacks cannot return errors, so the parse checks this slot afterwards and fails with it.
fn image_error(budget: &Mutex<ImageBudget>, error: impl Into<String>) {
    let mut budget = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if budget.error.is_none() {
        budget.error = Some(error.into());
    }
}

/// Deduct one image's `width × height` decoded pixels from the shared budget before it is
/// decoded. A zero-sized, overflowing or over-budget image records an error and returns `false`.
fn charge_image(budget: &Mutex<ImageBudget>, width: usize, height: usize) -> bool {
    let pixels = u64::try_from(width).ok().zip(u64::try_from(height).ok()).and_then(|(w, h)| w.checked_mul(h));
    let mut budget = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match pixels.filter(|pixels| *pixels != 0).and_then(|pixels| budget.pixels.checked_sub(pixels)) {
        Some(remaining) => {
            budget.pixels = remaining;
            true
        }
        None => {
            budget.error.get_or_insert_with(|| "embedded SVG raster images exceed the decoded pixel budget".into());
            false
        }
    }
}

/// The size declared by a GIF's first image descriptor, skipping extension blocks with checked
/// offsets; `None` when the stream is truncated or malformed.
///
/// A GIF's first frame can exceed its logical screen. resvg allocates the frame's RGBA buffer,
/// so inspect its descriptor too, without reading any LZW pixels.
fn gif_frame_size(bytes: &[u8]) -> Option<(usize, usize)> {
    let packed = *bytes.get(10)?;
    let table = if packed & 0x80 != 0 { 3usize.checked_mul(1usize.checked_shl(u32::from((packed & 7) + 1))?)? } else { 0 };
    let mut at = 13usize.checked_add(table)?;
    loop {
        match *bytes.get(at)? {
            0x2c => {
                let word = |offset: usize| -> Option<usize> {
                    let pair: [u8; 2] = bytes.get(at.checked_add(offset)?..at.checked_add(offset)?.checked_add(2)?)?.try_into().ok()?;
                    Some(usize::from(u16::from_le_bytes(pair)))
                };
                return Some((word(5)?, word(7)?));
            }
            0x21 => {
                at = at.checked_add(2)?;
                loop {
                    let length = usize::from(*bytes.get(at)?);
                    at = at.checked_add(1)?;
                    if length == 0 {
                        break;
                    }
                    bytes.get(at..at.checked_add(length)?)?;
                    at = at.checked_add(length)?;
                }
            }
            _ => return None,
        }
    }
}

/// Widen `dimensions` to the largest frame declared by the WebP RIFF `chunks` (VP8/VP8L
/// bitstreams and one level of ANMF animation frames); `None` when a chunk is malformed.
///
/// VP8X declares a container canvas, while a malformed lossy VP8 chunk may declare a larger
/// frame which the downstream decoder allocates before comparing it with that canvas.
fn webp_frame_sizes(chunks: &[u8], dimensions: &mut (usize, usize), nested: bool) -> Option<()> {
    let mut at = 0usize;
    while at < chunks.len() {
        let header = chunks.get(at..at.checked_add(8)?)?;
        let length = usize::try_from(u32::from_le_bytes(header.get(4..8)?.try_into().ok()?)).ok()?;
        let from = at.checked_add(8)?;
        let end = from.checked_add(length)?;
        let payload = chunks.get(from..end)?;
        match header.get(..4)? {
            b"VP8 " | b"VP8L" => {
                let mut probe = b"RIFF\0\0\0\0WEBP".to_vec();
                probe.extend_from_slice(header);
                probe.extend_from_slice(payload.get(..payload.len().min(10))?);
                let size = imagesize::blob_size(&probe).ok()?;
                dimensions.0 = dimensions.0.max(size.width);
                dimensions.1 = dimensions.1.max(size.height);
            }
            b"ANMF" if !nested => {
                let word = |at: usize| -> Option<usize> {
                    let bytes: [u8; 3] = payload.get(at..at.checked_add(3)?)?.try_into().ok()?;
                    usize::try_from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]).checked_add(1)?).ok()
                };
                dimensions.0 = dimensions.0.max(word(6)?);
                dimensions.1 = dimensions.1.max(word(9)?);
                webp_frame_sizes(payload.get(16..)?, dimensions, true)?;
            }
            b"ANMF" => return None,
            _ => {}
        }
        at = end.checked_add(length & 1)?;
        if at > chunks.len() {
            return None;
        }
    }
    Some(())
}

/// usvg's data-URL image resolver with budgets: nested SVG images are parsed with the same
/// budget up to [`MAX_EMBEDDED_SVG_DEPTH`] and within the `xml_left` levels of XML nesting the
/// drawing left them, and PNG/JPEG/GIF/WebP images are charged their largest declared frame
/// before decoding. An overrun records an error and resolves to no image so the enclosing parse
/// fails. An image that isn't one of those formats, is invalid, or has a MIME type usvg doesn't
/// know is left out and the rest still draws, as usvg's own resolver does (with a strict budget
/// it is an error too).
fn bounded_data_image(
    mime: &str,
    data: Arc<Vec<u8>>,
    options: &usvg::Options,
    budget: &Arc<Mutex<ImageBudget>>,
    depth: usize,
    xml_left: usize,
) -> Option<usvg::ImageKind> {
    if budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner).error.is_some() {
        return None;
    }
    let format = photocraft_codecs::detect(&data);
    if mime == "image/svg+xml" || (mime == "text/plain" && format.is_none()) {
        if depth >= MAX_EMBEDDED_SVG_DEPTH {
            image_error(budget, "embedded SVG images exceed the nesting limit");
            return None;
        }
        // A budget overrun inside the image is already recorded; any other failure (not an
        // SVG, nested deeper than the drawing left room for) only leaves the image out.
        let Ok(tree) = parse_image_bounded(&data, budget.clone(), depth + 1, xml_left) else {
            return unreadable_image(budget, "can't parse an embedded SVG image");
        };
        let mut counter = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        counter.svg_images = counter.svg_images.saturating_add(1);
        counter.svg_chain = counter.svg_chain.max(u64::try_from(depth + 1).unwrap_or(u64::MAX));
        Some(usvg::ImageKind::SVG(tree))
    } else {
        let supported = matches!(
            format,
            Some(photocraft_codecs::Format::Png | photocraft_codecs::Format::Jpeg | photocraft_codecs::Format::Gif | photocraft_codecs::Format::WebP)
        );
        let size = supported.then(|| imagesize::blob_size(&data).ok()).flatten();
        let Some(size) = size else { return unreadable_image(budget, "embedded SVG image has unsupported or invalid raster data") };
        let mut dimensions = (size.width, size.height);
        if format == Some(photocraft_codecs::Format::Gif) {
            let Some(frame) = gif_frame_size(&data) else { return unreadable_image(budget, "can't read embedded GIF frame dimensions") };
            dimensions = (dimensions.0.max(frame.0), dimensions.1.max(frame.1));
        }
        if format == Some(photocraft_codecs::Format::WebP) && data.get(12..).and_then(|chunks| webp_frame_sizes(chunks, &mut dimensions, false)).is_none() {
            return unreadable_image(budget, "can't read embedded WebP frame dimensions");
        }
        // usvg's resolver only wraps the bytes (resvg decodes them when drawing): an image under
        // a MIME type it doesn't know, or with no pixels, is left out uncharged, as usvg would.
        let Some(image) = (usvg::ImageHrefResolver::default_data_resolver())(mime, data, options) else {
            return unreadable_image(budget, "embedded SVG raster image has an unsupported MIME type");
        };
        if dimensions.0 == 0 || dimensions.1 == 0 {
            return unreadable_image(budget, "embedded SVG image has no pixels");
        }
        if !charge_image(budget, dimensions.0, dimensions.1) {
            return None;
        }
        Some(image)
    }
}

/// [`parse_image_bounded_with`] using the shared system font database.
fn parse_image_bounded(bytes: &[u8], budget: Arc<Mutex<ImageBudget>>, depth: usize, xml_left: usize) -> Result<usvg::Tree, IoError> {
    parse_image_bounded_with(bytes, budget, depth, xml_left, fonts)
}

/// Parse one (possibly gzip-compressed) SVG at nesting `depth` against `budget`: the byte,
/// inflated-size and XML-depth limits apply first, embedded images go through
/// [`bounded_data_image`], and the tree's drawing-node count and repeatable image resources are
/// added to the budget for the caller's work estimate. Fonts load only when text is present.
/// usvg parses an embedded SVG on top of the drawing's own recursion, so the drawing and the
/// images inside it share `xml_left` levels of XML nesting ([`MAX_XML_DEPTH`] for the drawing).
/// Byte and decode overruns are also recorded in `budget`, so they fail the whole drawing; a
/// nesting or syntax error only fails this SVG.
fn parse_image_bounded_with(
    bytes: &[u8],
    budget: Arc<Mutex<ImageBudget>>,
    depth: usize,
    xml_left: usize,
    fonts: fn() -> Arc<usvg::fontdb::Database>,
) -> Result<usvg::Tree, IoError> {
    let overrun = |error: IoError| {
        image_error(&budget, error.to_string());
        error
    };
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BOUNDED_SVG_BYTES {
        return Err(overrun(IoError::Svg("embedded SVG source exceeds the 16 MiB byte limit".into())));
    }
    let xml = inflate_with_limit(bytes, MAX_BOUNDED_SVG_BYTES).map_err(overrun)?;
    // Charged before anything can leave the image out, so skipped images still use up the budget.
    let xml_len = u64::try_from(xml.len()).unwrap_or(u64::MAX);
    let left = {
        let mut counter = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let left = counter.xml_bytes.checked_sub(xml_len);
        if let Some(left) = left {
            counter.xml_bytes = left;
        }
        left
    };
    if left.is_none() {
        return Err(overrun(IoError::Svg("embedded SVG XML exceeds the decoded byte budget".into())));
    }
    let nesting = nesting_depth(&xml, xml_left);
    if nesting > xml_left {
        return Err(IoError::Svg(format!("elements are nested more than {xml_left} levels deep")));
    }
    let has_text = xml.windows(4).any(|w| w == b"text");
    let fontdb = if has_text { fonts() } else { Arc::new(usvg::fontdb::Database::new()) };
    if has_text && fontdb.faces().next().is_none() {
        budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner).fontless_text = true;
    }
    // The resolver below replaces `options`' own, so its embedded-SVG depth argument is unused.
    let mut opt = options(fontdb, 0);
    let shared = budget.clone();
    let image_xml_left = xml_left.saturating_sub(nesting);
    opt.image_href_resolver.resolve_data = Box::new(move |mime, data, options| bounded_data_image(mime, data, options, &shared, depth, image_xml_left));
    // The bounded inflate above is the only decompression step. from_data would detect a
    // nested gzip payload and inflate it again without the source-byte limit.
    let text = std::str::from_utf8(&xml).map_err(|_| IoError::Svg("the SVG is not valid UTF-8".into()))?;
    let tree = usvg::Tree::from_str(text, &opt);
    if let Some(error) = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner).error.clone() {
        return Err(IoError::Svg(error));
    }
    let tree = tree.map_err(|e| IoError::Svg(e.to_string()))?;
    // Include nested SVG trees too. Paint servers can reuse one parsed raster in many
    // drawing nodes, so the work estimate must not assume every source is decoded once.
    let nodes = u64::try_from(count_nodes_with_limit(tree.root(), true, usize::MAX, false)).unwrap_or(u64::MAX);
    let mut counter = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    counter.draw_nodes = counter.draw_nodes.saturating_add(nodes);
    let (raster, svg) = repeatable_image_resources(&tree);
    counter.repeatable_raster |= raster;
    counter.repeatable_svg |= svg;
    Ok(tree)
}

/// Used paint/effect resources can decode an image once per use. Ordinary paths, gradients,
/// clipping paths and blur filters without image references do not multiply unrelated photos.
fn repeatable_image_resources(tree: &usvg::Tree) -> (bool, bool) {
    /// Mark `kinds` (raster, SVG) for images reachable from `group`, visiting each group once
    /// and turning conservative (both kinds) past the depth or step bound.
    fn visit(group: &usvg::Group, depth: usize, seen: &mut std::collections::HashSet<usize>, steps: &mut usize, kinds: &mut (bool, bool)) {
        if depth > MAX_RECURSION || *steps >= 1_000_000 {
            *kinds = (true, true); // Conservative when resource inspection itself reaches its bound.
            return;
        }
        if !seen.insert((group as *const usvg::Group).addr()) {
            return;
        }
        for node in group.children() {
            *steps = steps.saturating_add(1);
            match node {
                usvg::Node::Image(image) => match image.kind() {
                    usvg::ImageKind::SVG(_) => kinds.1 = true,
                    _ => kinds.0 = true,
                },
                usvg::Node::Group(child) => visit(child, depth + 1, seen, steps, kinds),
                _ => {}
            }
            node.subroots(|root| visit(root, depth + 1, seen, steps, kinds));
            if (kinds.0 && kinds.1) || *steps >= 1_000_000 {
                if *steps >= 1_000_000 {
                    *kinds = (true, true);
                }
                return;
            }
        }
    }
    let (mut seen, mut steps, mut kinds) = (std::collections::HashSet::new(), 0, (false, false));
    for root in tree.patterns().iter().map(|p| p.root()).chain(tree.masks().iter().map(|m| m.root())).chain(tree.clip_paths().iter().map(|c| c.root())) {
        visit(root, 0, &mut seen, &mut steps, &mut kinds);
    }
    for filter in tree.filters() {
        for primitive in filter.primitives() {
            if let usvg::filter::Kind::Image(image) = primitive.kind() {
                visit(image.root(), 0, &mut seen, &mut steps, &mut kinds);
            }
        }
    }
    kinds
}

struct ParsedWork {
    parsed: Parsed,
    svg_images: u64,
    svg_chain: u64,
    decoded: u64,
    draw_nodes: u64,
    repeatable_raster: bool,
    repeatable_svg: bool,
}

/// Parse untrusted SVG with at most `max_pixels` (capped at 24 MP) of decoded image pixels. The
/// scratch surfaces its embedded SVG images need at the drawing's own size count against the
/// same limit.
fn parse_bounded(bytes: &[u8], max_pixels: u64) -> Result<Parsed, IoError> {
    let (parsed, chain) = parse_bounded_inner(bytes, max_pixels)?;
    let area = canvas(&parsed.tree, &mut Vec::new()).size.area();
    check_scratch_surfaces(area, chain, max_pixels)?;
    Ok(parsed)
}

/// [`parse_bounded`] without the scratch-surface check, also returning the deepest chain of
/// embedded SVG images (see [`ImageBudget::svg_chain`]) so a render can check it at its own size.
fn parse_bounded_inner(bytes: &[u8], max_pixels: u64) -> Result<(Parsed, u64), IoError> {
    parse_bounded_inner_with(bytes, max_pixels, fonts)
}

/// [`parse_bounded_inner`] with an explicit font source, so tests can pin the available fonts.
fn parse_bounded_inner_with(bytes: &[u8], max_pixels: u64, fonts: fn() -> Arc<usvg::fontdb::Database>) -> Result<(Parsed, u64), IoError> {
    parse_bounded_work_with(bytes, max_pixels, false, fonts).map(|work| (work.parsed, work.svg_chain))
}

/// resvg draws an embedded SVG image through a scratch pixmap as large as the `area` it is drawn
/// into, one per level of a `chain` of nested images, all alive at once; past `max_pixels` (at
/// most 24 MP) of them is an error. Siblings reuse the space one after another, so a drawing
/// with any number of side-by-side SVG images needs one, like the drawing itself.
fn check_scratch_surfaces(area: u64, chain: u64, max_pixels: u64) -> Result<(), IoError> {
    if chain != 0 && area.checked_mul(chain).is_none_or(|pixels| pixels > max_pixels.min(MAX_RASTER_PIXELS)) {
        return Err(IoError::Svg("embedded SVG intermediate images exceed the SVG cache budget".into()));
    }
    Ok(())
}

/// The decoder's charged pixels, including raster images inside nested SVG images.
fn parse_bounded_work_with(bytes: &[u8], max_pixels: u64, strict: bool, fonts: fn() -> Arc<usvg::fontdb::Database>) -> Result<ParsedWork, IoError> {
    let budget = Arc::new(Mutex::new(ImageBudget {
        pixels: max_pixels.min(MAX_RASTER_PIXELS),
        xml_bytes: MAX_BOUNDED_SVG_BYTES,
        svg_images: 0,
        svg_chain: 0,
        draw_nodes: 0,
        repeatable_raster: false,
        repeatable_svg: false,
        fontless_text: false,
        strict,
        error: None,
    }));
    let tree = parse_image_bounded_with(bytes, budget.clone(), 0, MAX_XML_DEPTH, fonts)?;
    let counter = budget.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let decoded = max_pixels.min(MAX_RASTER_PIXELS).saturating_sub(counter.pixels);
    Ok(ParsedWork {
        parsed: Parsed { tree, fontless_text: counter.fontless_text },
        svg_images: counter.svg_images,
        svg_chain: counter.svg_chain,
        decoded,
        draw_nodes: counter.draw_nodes,
        repeatable_raster: counter.repeatable_raster,
        repeatable_svg: counter.repeatable_svg,
    })
}

/// The plain XML of `bytes`: itself, or the inflated content of a gzip `.svgz`, refused past
/// [`MAX_INFLATED_BYTES`] (a decompression bomb) or when the stream is corrupt.
fn inflate(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, IoError> {
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    inflate_with_limit(bytes, MAX_INFLATED_BYTES)
}

/// [`inflate`] with an explicit `limit`, which also applies to uncompressed input.
fn inflate_with_limit(bytes: &[u8], limit: u64) -> Result<std::borrow::Cow<'_, [u8]>, IoError> {
    use std::io::Read;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
            return Err(IoError::Svg(format!("the SVG exceeds the {limit} byte limit")));
        }
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    let mut out = Vec::new();
    let mut reader = flate2::bufread::MultiGzDecoder::new(bytes).take(limit.saturating_add(1));
    reader.read_to_end(&mut out).map_err(|e| IoError::Svg(format!("the compressed SVG couldn't be inflated: {e}")))?;
    if u64::try_from(out.len()).unwrap_or(u64::MAX) > limit {
        return Err(IoError::Svg(format!("the compressed SVG inflates past {} MB", limit >> 20)));
    }
    Ok(std::borrow::Cow::Owned(out))
}

/// The deepest element nesting in `xml`, stopping as soon as it passes `limit` (the result is
/// then `limit + 1`). Comments, CDATA sections, processing instructions and the DOCTYPE add
/// nothing; quoted attribute values may hold `>` or `/>`. Malformed markup is only counted
/// loosely: the parser rejects it afterwards.
pub(crate) fn nesting_depth(xml: &[u8], limit: usize) -> usize {
    let after = |from: usize, pat: &[u8]| -> usize {
        xml.get(from..).and_then(|s| s.windows(pat.len()).position(|w| w == pat)).map_or(xml.len(), |p| from.saturating_add(p).saturating_add(pat.len()))
    };
    // The byte after a tag's closing `>` (outside quotes) and whether the tag ended in `/>`.
    let tag_end = |from: usize| -> (usize, bool) {
        let (mut quote, mut last) = (None, b' ');
        for (k, &b) in xml.iter().enumerate().skip(from) {
            match quote {
                Some(q) if b == q => quote = None,
                Some(_) => {}
                None if b == b'"' || b == b'\'' => quote = Some(b),
                None if b == b'>' => return (k.saturating_add(1), last == b'/'),
                None => {
                    if !b.is_ascii_whitespace() {
                        last = b;
                    }
                }
            }
        }
        (xml.len(), false)
    };
    let (mut depth, mut deepest, mut i) = (0usize, 0usize, 0usize);
    while let Some(p) = xml.get(i..).and_then(|s| s.iter().position(|&b| b == b'<')) {
        let at = i.saturating_add(p);
        let rest = xml.get(at..).unwrap_or_default();
        i = if rest.starts_with(b"<!--") {
            after(at.saturating_add(4), b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            after(at.saturating_add(9), b"]]>")
        } else if rest.starts_with(b"<?") {
            after(at.saturating_add(2), b"?>")
        } else if rest.starts_with(b"<!") {
            tag_end(at.saturating_add(2)).0
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            tag_end(at.saturating_add(2)).0
        } else {
            let (end, self_closing) = tag_end(at.saturating_add(1));
            if !self_closing {
                depth = depth.saturating_add(1);
                deepest = deepest.max(depth);
                if deepest > limit {
                    return deepest;
                }
            }
            end
        };
    }
    deepest
}

/// The document canvas for a drawing: its size in px, scaled down to [`MAX_SIDE`] when needed.
struct Canvas {
    size: Size,
    /// Drawing units → canvas pixels (1 unless the drawing had to shrink).
    scale: f32,
}

fn canvas(tree: &usvg::Tree, warnings: &mut Vec<String>) -> Canvas {
    let (w, h) = (tree.size().width().max(1.0), tree.size().height().max(1.0));
    let limit = MAX_SIDE as f32;
    let scale = (limit / w).min(limit / h).min(1.0);
    if scale < 1.0 {
        warnings.push(format!("SVG: the {w:.0} × {h:.0} px drawing was scaled down to fit {MAX_SIDE} px"));
    }
    let side = |v: f32| ((v * scale).round() as u32).clamp(1, MAX_SIDE);
    Canvas { size: Size::new(side(w), side(h)), scale }
}

/// Opens an SVG as a document of shape layers (see the module docs for what is rasterised).
pub fn import_svg(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_svg_with_group_depth(name, bytes, MAX_GROUP_DEPTH)
}

/// Opens an SVG while rasterising any subtrees deeper than `max_group_depth` into single pixel
/// layers. The document's hard nesting limit remains [`MAX_GROUP_DEPTH`].
pub fn import_svg_with_group_depth(name: &str, bytes: &[u8], max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_svg_inner(name, bytes, max_group_depth, None, false)
}

/// Opens an SVG with a conservative budget for eagerly rendered layer caches. A canvas above
/// the budget (or the rasterisation limit) is rejected before any layer cache is allocated.
/// When the canvas fits but its elements' combined caches would exceed the budget, the whole
/// drawing becomes one pixel layer, with an explicit editability warning.
pub fn import_svg_bounded(name: &str, bytes: &[u8], max_cached_pixels: u64) -> Result<ImportResult, IoError> {
    import_svg_inner(name, bytes, MAX_GROUP_DEPTH, Some(max_cached_pixels), false)
}

/// Import a drawing with one aggregate pixel-work allowance and return its conservative charge.
/// This includes decoded raster pixels (including repeated paints), eager layer-cache surfaces
/// and embedded-SVG intermediate surfaces. Image decoding and each canvas/cache component keep
/// their independent 24 MP caps; the aggregate allowance can pay for several components.
/// Raster-bearing patterns, masks and image-filter resources conservatively reserve repaint
/// work across drawing nodes; ordinary vector paths, artboard clips and blur alone do not.
/// [`import_svg_bounded`] retains its existing independent image/cache limits. An embedded image
/// it can't read is left out, as on placement.
pub fn import_svg_bounded_with_work(name: &str, bytes: &[u8], max_work: u64) -> Result<(ImportResult, u64), IoError> {
    import_svg_bounded_with_work_mode(name, bytes, max_work, false)
}

/// [`import_svg_bounded_with_work`] for a stored display preview: an embedded image it can't
/// read fails it, so only a preview that draws completely stands in for its native source.
pub fn import_svg_preview_with_work(name: &str, bytes: &[u8], max_work: u64) -> Result<(ImportResult, u64), IoError> {
    import_svg_bounded_with_work_mode(name, bytes, max_work, true)
}

fn import_svg_bounded_with_work_mode(name: &str, bytes: &[u8], max_work: u64, strict: bool) -> Result<(ImportResult, u64), IoError> {
    if max_work == 0 {
        return Err(IoError::Svg("SVG preview validation work budget is exhausted".into()));
    }
    let ParsedWork { parsed, svg_images, decoded, draw_nodes, repeatable_raster, repeatable_svg, .. } =
        parse_bounded_work_with(bytes, max_work.min(MAX_RASTER_PIXELS), strict, fonts)?;
    let area = canvas(&parsed.tree, &mut Vec::new()).size.area();
    // This preview check charges every embedded SVG image a canvas-sized surface of work.
    if svg_images != 0 && area.checked_mul(svg_images.saturating_add(1)).is_none_or(|pixels| pixels > max_work.min(MAX_RASTER_PIXELS)) {
        return Err(IoError::Svg("embedded SVG intermediate images exceed the SVG cache budget".into()));
    }
    let uses = draw_nodes.max(1);
    let intermediate_surfaces = svg_images.saturating_mul(if repeatable_svg { uses } else { 1 }).saturating_add(1);
    // Only raster-bearing repeatable resources can repaint an already-parsed image. Plain
    // vector paths alongside a photo do not decode that photo again for each path.
    let decoded_work = decoded
        .checked_mul(if repeatable_raster { uses } else { 1 })
        .filter(|pixels| *pixels <= max_work)
        .ok_or_else(|| IoError::Svg("SVG image decoding exceeds the validation work budget".into()))?;
    let cache_limit = (max_work.saturating_sub(decoded_work) / intermediate_surfaces).min(MAX_RASTER_PIXELS);
    if area > cache_limit {
        return Err(IoError::Svg("SVG preview decoding and rendering exceed the validation work budget".into()));
    }
    let nodes = count_nodes(parsed.tree.root(), true);
    let layers = area.checked_mul(u64::try_from(nodes.max(1)).unwrap_or(u64::MAX));
    // import_parsed uses this same bound to select one whole-drawing fallback before it
    // renders any layer caches. A tiny canvas cannot hide a huge decoded source image.
    let rendered = if nodes > MAX_NODES || layers.is_none_or(|pixels| pixels > cache_limit) { area } else { layers.unwrap_or(area) };
    let work = rendered
        .checked_mul(intermediate_surfaces)
        .and_then(|pixels| decoded_work.checked_add(pixels))
        .filter(|pixels| *pixels <= max_work)
        .ok_or_else(|| IoError::Svg("SVG preview validation work budget exceeded".into()))?;
    import_parsed(name, parsed, MAX_GROUP_DEPTH, Some(cache_limit), false).map(|imported| (imported, work))
}

/// The same cache bound without a lossy aggregate-budget fallback. Smart-object operations
/// use this variant so editing contents or changing Stack Mode fails before excessive caches
/// are rendered, rather than silently changing the source's layer structure.
pub fn import_svg_bounded_layers(name: &str, bytes: &[u8], max_cached_pixels: u64) -> Result<ImportResult, IoError> {
    import_svg_inner(name, bytes, MAX_GROUP_DEPTH, Some(max_cached_pixels), true)
}

/// Opens untrusted SVG (smart-object contents) as [`import_svg`] does, as editable layers under
/// its retained-pixel budget, but with the bounded parse: embedded images may decode to at most
/// `max_decoded_pixels` (capped at 24 MP) in all, nested SVG images get the same budget, and
/// compressed data inflates to at most 16 MiB.
pub fn import_svg_bounded_images(name: &str, bytes: &[u8], max_decoded_pixels: u64) -> Result<ImportResult, IoError> {
    import_parsed(name, parse_bounded(bytes, max_decoded_pixels)?, MAX_GROUP_DEPTH, None, false)
}

/// The pixel bounds [`import_svg_bounded`] gives the drawing, found without rendering any layer
/// cache. It applies the same bounded parse (decoded-image and nested-SVG budgets) and the same
/// canvas limit, so a canvas of exactly `max_cached_pixels` (at most 24 MP) fits and one more
/// pixel does not. A smart object's warp frame needs only these bounds, so an SVG whose editable
/// layer caches exceed the budget (rejected by [`import_svg_bounded_layers`]) still has one.
pub fn bounded_canvas_bounds(bytes: &[u8], max_cached_pixels: u64) -> Result<Rect, IoError> {
    let parsed = parse_bounded(bytes, max_cached_pixels)?;
    let cv = canvas(&parsed.tree, &mut Vec::new());
    let limit = max_cached_pixels.min(MAX_RASTER_PIXELS);
    if cv.size.area() > limit {
        return Err(IoError::Svg(format!("{} × {} px exceeds the SVG layer cache budget of {limit} pixels", cv.size.width, cv.size.height)));
    }
    Ok(Rect::from_size(cv.size))
}

/// Parse (bounded when `max_cached_pixels` is set) and convert to layers; see [`import_parsed_with_limits`].
fn import_svg_inner(
    name: &str,
    bytes: &[u8],
    max_group_depth: usize,
    max_cached_pixels: Option<u64>,
    reject_cache_overflow: bool,
) -> Result<ImportResult, IoError> {
    let parsed = match max_cached_pixels {
        Some(limit) => parse_bounded(bytes, limit)?,
        None => parse(bytes)?,
    };
    import_parsed(name, parsed, max_group_depth, max_cached_pixels, reject_cache_overflow)
}

/// [`import_parsed_with_limits`] with the default retained-pixel byte budget.
fn import_parsed(
    name: &str,
    parsed: Parsed,
    max_group_depth: usize,
    max_cached_pixels: Option<u64>,
    reject_cache_overflow: bool,
) -> Result<ImportResult, IoError> {
    import_parsed_with_limits(name, parsed, max_group_depth, max_cached_pixels, reject_cache_overflow, MAX_PIXEL_BYTES)
}

#[cfg(test)]
fn import_parsed_with_budget(name: &str, parsed: Parsed, max_group_depth: usize, pixel_budget: u64) -> Result<ImportResult, IoError> {
    import_parsed_with_limits(name, parsed, max_group_depth, None, false, pixel_budget)
}

/// Convert a parsed tree into a document. With `max_cached_pixels`, a canvas over the budget is
/// an error and caches over it either fail (`reject_cache_overflow`) or collapse the drawing into
/// one pixel layer with a warning; more than [`MAX_NODES`] elements always collapse.
/// `pixel_budget` bounds the bytes of retained layer pixels.
fn import_parsed_with_limits(
    name: &str,
    parsed: Parsed,
    max_group_depth: usize,
    max_cached_pixels: Option<u64>,
    reject_cache_overflow: bool,
    pixel_budget: u64,
) -> Result<ImportResult, IoError> {
    let Parsed { tree, fontless_text } = parsed;
    let mut warnings = Vec::new();
    if fontless_text {
        warnings.push("SVG: the text needs a font and none is installed, so it was dropped".to_string());
    }
    let cv = canvas(&tree, &mut warnings);
    let nodes = count_nodes(tree.root(), max_cached_pixels.is_some());
    let over_cache_budget = if let Some(budget) = max_cached_pixels {
        let area = cv.size.area();
        if area > budget.min(MAX_RASTER_PIXELS) {
            return Err(IoError::Svg(format!(
                "{} × {} px exceeds the SVG layer cache budget of {} pixels",
                cv.size.width,
                cv.size.height,
                budget.min(MAX_RASTER_PIXELS)
            )));
        }
        let count = u64::try_from(nodes.min(MAX_NODES)).unwrap_or(u64::MAX);
        area.checked_mul(count).is_none_or(|pixels| pixels > budget)
    } else {
        false
    };
    if over_cache_budget && reject_cache_overflow {
        return Err(IoError::Svg("the source's editable layer caches exceed the SVG cache budget".into()));
    }
    let mut doc = Document::new(name, cv.size, ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let global = tiny_skia::Transform::from_scale(cv.scale, cv.scale);
    let max_group_depth = (max_group_depth != 0).then_some(max_group_depth.clamp(1, MAX_GROUP_DEPTH));
    let mut conv =
        Conv { fmt, canvas: doc.bounds(), global, max_group_depth, warnings, noted: Vec::new(), counts: [0; 4], pixel_budget, retained_pixel_bytes: 0 };
    if nodes > MAX_NODES || over_cache_budget {
        if over_cache_budget {
            conv.warnings.push("SVG: combined layer caches exceed the clipboard cache budget; opened as one pixel layer instead of editable shapes".into());
        } else {
            conv.warnings.push(format!("SVG: more than {MAX_NODES} elements; opened as one pixel layer instead of shapes"));
        }
        let (w, h) = (cv.size.width, cv.size.height);
        if let Some(mut pixmap) = pixmap(w, h) {
            resvg::render(&tree, global, &mut pixmap.as_mut());
            doc.layers.push(Layer::new("Background", LayerContent::Raster(surface(&pixmap, doc.bounds(), fmt))));
        }
    } else {
        doc.layers = conv.convert_children(tree.root().children(), 0, 0, tree.root().abs_transform());
    }
    if doc.layers.is_empty() {
        conv.warnings.push("SVG: nothing to draw (an empty drawing)".to_string());
    }
    Ok(ImportResult { document: doc, warnings: conv.warnings, source_read_only: false, preview_only: false })
}

/// The whole drawing rendered at `scale` (1 = its own size), as a straight-alpha RGBA buffer at
/// the origin. Smart objects use it to stay sharp at any placement.
pub fn rasterize(bytes: &[u8], scale: f32) -> Result<photocraft_compose::Buffer, IoError> {
    let tree = parse(bytes)?.tree;
    rasterize_tree(&tree, scale)
}

/// Render untrusted embedded SVG with intrinsic raster-image and aggregate decode budgets.
pub fn rasterize_bounded(bytes: &[u8], scale: f32, max_decoded_pixels: u64) -> Result<photocraft_compose::Buffer, IoError> {
    let (parsed, chain) = parse_bounded_inner(bytes, max_decoded_pixels)?;
    let tree = parsed.tree;
    if chain != 0 {
        let cv = canvas(&tree, &mut Vec::new());
        let k = cv.scale * scale.clamp(1.0 / 64.0, 64.0);
        let width = ((tree.size().width() * k).ceil() as u32).max(1);
        let height = ((tree.size().height() * k).ceil() as u32).max(1);
        check_scratch_surfaces(u64::from(width) * u64::from(height), chain, max_decoded_pixels)
            .map_err(|_| IoError::Svg("embedded SVG intermediate images exceed the rendered pixel budget".into()))?;
    }
    rasterize_tree(&tree, scale)
}

/// Render the whole tree at `scale` (clamped to 1/64–64) times its canvas size into a straight-alpha
/// buffer at the origin; a result over the 24 MP rasterisation limit is an error.
fn rasterize_tree(tree: &usvg::Tree, scale: f32) -> Result<photocraft_compose::Buffer, IoError> {
    let cv = canvas(tree, &mut Vec::new());
    let pixmap = render(tree, cv.scale * scale.clamp(1.0 / 64.0, 64.0))?;
    let rect = Rect::new(0, 0, pixmap.width() as i32, pixmap.height() as i32);
    let px = pixmap.pixels().iter().map(|p| rgba(p.demultiply())).collect();
    Ok(photocraft_compose::Buffer { rect, px })
}

/// The whole drawing at one pixel per SVG unit, scaled down to fit [`MAX_SIDE`] a side and
/// [`MAX_RASTER_PIXELS`] in all: its width, height and straight-alpha RGBA8 pixels. Paste uses it
/// for a drawing copied as SVG markup.
pub fn rasterize_rgba8(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), IoError> {
    let tree = parse(bytes)?.tree;
    let cv = canvas(&tree, &mut Vec::new());
    let (w, h) = (f64::from(tree.size().width() * cv.scale), f64::from(tree.size().height() * cv.scale));
    // The largest k ≤ 1 with (w·k + 2)(h·k + 2) ≤ the limit: `render` rounds each side up, and
    // f32 rounding can add one more pixel.
    let max = MAX_RASTER_PIXELS as f64;
    let fit = if (w + 2.0) * (h + 2.0) <= max {
        1.0
    } else {
        let (a, b) = (w * h, 2.0 * (w + h));
        (-b + (b * b - 4.0 * a * (4.0 - max)).sqrt()) / (2.0 * a)
    };
    let pixmap = render(&tree, cv.scale * fit as f32)?;
    let px = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok((pixmap.width(), pixmap.height(), px))
}

/// `tree` drawn at `k` pixels per SVG unit on a transparent pixmap whose sides round up.
fn render(tree: &usvg::Tree, k: f32) -> Result<tiny_skia::Pixmap, IoError> {
    let side = |v: f32| ((v * k).ceil() as u32).max(1);
    let (w, h) = (side(tree.size().width()), side(tree.size().height()));
    let mut pixmap = pixmap(w, h).ok_or_else(|| IoError::Svg(format!("{w} × {h} px is too large to rasterise")))?;
    resvg::render(tree, tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    Ok(pixmap)
}

/// A transparent pixmap, or `None` past [`MAX_RASTER_PIXELS`].
fn pixmap(w: u32, h: u32) -> Option<tiny_skia::Pixmap> {
    if u64::from(w) * u64::from(h) > MAX_RASTER_PIXELS {
        return None;
    }
    tiny_skia::Pixmap::new(w, h)
}

fn rgba(c: tiny_skia::ColorU8) -> [f32; 4] {
    [f32::from(c.red()) / 255.0, f32::from(c.green()) / 255.0, f32::from(c.blue()) / 255.0, f32::from(c.alpha()) / 255.0]
}

/// A pixmap as a surface covering `rect` in `fmt`.
fn surface(pixmap: &tiny_skia::Pixmap, rect: Rect, fmt: PixelFormat) -> Surface {
    let n = fmt.channels();
    let mut data = vec![0.0f32; pixmap.pixels().len() * n];
    for (p, out) in pixmap.pixels().iter().zip(data.chunks_exact_mut(n)) {
        photocraft_raster::from_rgba_into(&fmt, rgba(p.demultiply()), out);
    }
    let mut s = Surface::new(fmt);
    if !rect.is_empty() {
        s.write_region(rect, &data);
    }
    s.prune();
    s
}

/// Drawable elements in the tree (iterative: the nesting can be anything).
fn count_nodes(root: &usvg::Group, include_text: bool) -> usize {
    count_nodes_with_limit(root, include_text, MAX_NODES, true)
}

/// Count structural cache nodes or drawing nodes, stopping only at the caller's limit.
fn count_nodes_with_limit(root: &usvg::Group, include_text: bool, limit: usize, count_groups: bool) -> usize {
    let mut n: usize = 0;
    let mut stack: Vec<&usvg::Group> = vec![root];
    while let Some(g) = stack.pop() {
        for node in g.children() {
            if count_groups || !matches!(node, usvg::Node::Group(_)) {
                n = n.saturating_add(1);
            }
            if let usvg::Node::Group(child) = node {
                stack.push(child);
            }
            if include_text && let usvg::Node::Text(text) = node {
                stack.push(text.flattened());
            }
            if n > limit {
                return n;
            }
        }
    }
    n
}

/// Converts a parsed tree into layers.
struct Conv {
    fmt: PixelFormat,
    canvas: Rect,
    /// Drawing units → canvas pixels.
    global: tiny_skia::Transform,
    max_group_depth: Option<usize>,
    warnings: Vec<String>,
    /// Features already warned about (one warning each).
    noted: Vec<&'static str>,
    /// Unnamed shapes, groups, images and texts so far, for "Shape 3"-style names.
    counts: [usize; 4],
    /// Remaining budget for retained rasterised surfaces, in actual sample bytes.
    pixel_budget: u64,
    /// Successfully retained raster surface memory; rejected nodes are never counted.
    retained_pixel_bytes: u64,
}

impl Conv {
    fn note(&mut self, what: &'static str, msg: String) {
        if !self.noted.contains(&what) {
            self.noted.push(what);
            self.warnings.push(msg);
        }
    }

    /// Check both retained and transient memory before rendering; charge only retained output.
    fn can_retain_pixels(&mut self, pixels: u64) -> bool {
        let bytes = pixels.saturating_mul(self.fmt.bytes_per_pixel() as u64);
        if bytes > self.pixel_budget {
            self.note("pixel-budget", "SVG: some rasterised elements were omitted because they exceeded the remaining pixel memory budget".into());
            return false;
        }
        // surface() converts the RGBA8 pixmap into f32 scratch space before writing
        // sample-format tiles. All three buffers briefly coexist with retained layers.
        let temporary = pixels.saturating_mul(4 + self.fmt.channels() as u64 * 4 + self.fmt.bytes_per_pixel() as u64);
        if self.retained_pixel_bytes.saturating_add(temporary) > MAX_PEAK_PIXEL_BYTES {
            self.note("pixel-peak", "SVG: some rasterised elements were omitted to bound temporary conversion memory".into());
            return false;
        }
        true
    }

    /// The element's `id`, or a numbered name by kind (0 shape, 1 group, 2 image, 3 text).
    fn name(&mut self, id: &str, kind: usize) -> String {
        if !id.is_empty() {
            return id.to_string();
        }
        let label = ["Shape", "Group", "Image", "Text"].get(kind).copied().unwrap_or("Layer");
        let n = self.counts.get_mut(kind).map(|c| {
            *c += 1;
            *c
        });
        format!("{label} {}", n.unwrap_or(0))
    }

    /// `nodes` in paint order (first = bottom), as layers in the same order. `parent` is the
    /// absolute transform of the group holding them (what resvg needs to render one of them
    /// alone, since a node's own data is in its parent's space).
    ///
    /// Transforms are composed from `parent` down rather than read from `abs_transform()`: the
    /// glyph outlines usvg builds for a text element (`Text::flattened`) keep identity absolute
    /// transforms, so text placed by its own or an ancestor's `transform` (Illustrator writes
    /// `matrix(1 0 0 1 x y)` on every text element) would lose it (#4806). Elsewhere the two agree.
    fn convert_children(&mut self, nodes: &[usvg::Node], depth: usize, rec: usize, parent: tiny_skia::Transform) -> Vec<Layer> {
        let mut out = Vec::new();
        for node in nodes {
            self.convert_node(node, &mut out, depth, rec, parent);
        }
        out
    }

    fn convert_node(&mut self, node: &usvg::Node, out: &mut Vec<Layer>, depth: usize, rec: usize, parent: tiny_skia::Transform) {
        match node {
            usvg::Node::Group(g) => {
                // A clip-path of one plain path becomes the group's vector mask (#4266); others
                // are rasterised with the group.
                let clip = g.clip_path().and_then(|cp| self.clip_mask(parent.pre_concat(g.transform()), cp));
                let dropped: [(bool, &'static str); 3] =
                    [(!g.filters().is_empty(), "filter"), (g.clip_path().is_some() && clip.is_none(), "clip-path"), (g.mask().is_some(), "mask")];
                let too_deep = self.max_group_depth.is_some_and(|limit| depth >= limit) || depth >= MAX_GROUP_DEPTH || rec >= MAX_RECURSION;
                if dropped.iter().any(|d| d.0) || too_deep {
                    for (yes, what) in dropped {
                        if yes {
                            self.note(what, format!("SVG: a group with a {what} was rasterised, so it is not editable as shapes"));
                        }
                    }
                    if too_deep {
                        self.note("depth", "SVG: groups nested too deeply were rasterised".to_string());
                    }
                    let name = self.name(g.id(), 1);
                    out.extend(self.raster_layer(node, name, parent));
                    return;
                }
                // A group that only carries a transform (baked into its paths by usvg) or that
                // usvg inserted for its own bookkeeping adds nothing to the layer tree.
                let plain = clip.is_none() && g.id().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal;
                let abs = parent.pre_concat(g.transform());
                let children = self.convert_children(g.children(), if plain { depth } else { depth + 1 }, rec + 1, abs);
                if children.is_empty() {
                    return;
                }
                if plain {
                    out.extend(children);
                    return;
                }
                let only_id = g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal;
                // A lone child takes the group's name (and its clip, when it has no vector mask of
                // its own).
                if only_id && children.len() == 1 && (clip.is_none() || children.iter().all(|c| c.vector_mask.is_none())) {
                    if let Some(mut child) = children.into_iter().next() {
                        if !g.id().is_empty() {
                            child.name = g.id().to_string();
                        }
                        child.vector_mask = clip.or(child.vector_mask);
                        out.push(child);
                    }
                    return;
                }
                let mut l = Layer::new(self.name(g.id(), 1), LayerContent::Group(Group { children, expanded: false, artboard: None, frame_stroke: None }));
                l.opacity = g.opacity().get();
                l.blend = blend(g.blend_mode());
                l.vector_mask = clip;
                out.push(l);
            }
            usvg::Node::Path(p) => out.extend(self.shape_layer(node, p, parent)),
            usvg::Node::Image(i) => {
                self.note("image", "SVG: embedded images are pixel layers".to_string());
                let name = self.name(i.id(), 2);
                out.extend(self.raster_layer(node, name, parent));
            }
            usvg::Node::Text(t) => {
                // The outlines are in the text element's own space.
                let abs = t.abs_transform().pre_concat(t.flattened().transform());
                let children = self.convert_children(t.flattened().children(), depth + 1, rec + 1, abs);
                if children.is_empty() {
                    self.note("text", "SVG: text needs a font the drawing can use; none matched, so it was dropped".to_string());
                    return;
                }
                let name = if t.id().is_empty() {
                    let content: String = t.chunks().iter().map(|c| c.text()).collect::<Vec<_>>().join("");
                    let content = content.trim();
                    if content.is_empty() { self.name("", 3) } else { content.chars().take(31).collect() }
                } else {
                    t.id().to_string()
                };
                out.push(Layer::new(name, LayerContent::Group(Group { children, expanded: false, artboard: None, frame_stroke: None })));
            }
        }
    }

    /// The vector mask for a group clipped by `cp`, when the clip is one visible path with no
    /// clip of its own: what resvg draws for the clip (the group's absolute transform `abs`, then
    /// the clip's, then the path), in canvas pixels, with the clip rule as its fill rule.
    fn clip_mask(&self, abs: tiny_skia::Transform, cp: &usvg::ClipPath) -> Option<VectorMask> {
        if cp.clip_path().is_some() {
            return None;
        }
        let [usvg::Node::Path(p)] = cp.root().children() else { return None };
        if !p.is_visible() {
            return None;
        }
        let ts = abs.pre_concat(cp.transform()).post_concat(self.global);
        let subpaths = knots(&(*p.data()).clone().transform(ts)?);
        if subpaths.is_empty() {
            return None;
        }
        let fill_rule = match p.fill().map(|f| f.rule()) {
            Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };
        Some(VectorMask::new(Path { subpaths, fill_rule, inverted: false }))
    }

    /// A path as a shape layer, or a pixel layer when its paint is a pattern.
    fn shape_layer(&mut self, node: &usvg::Node, p: &usvg::Path, parent: tiny_skia::Transform) -> Option<Layer> {
        if !p.is_visible() {
            return None;
        }
        let is_pattern = |paint: &usvg::Paint| matches!(paint, usvg::Paint::Pattern(_));
        if p.fill().is_some_and(|f| is_pattern(f.paint())) || p.stroke().is_some_and(|s| is_pattern(s.paint())) {
            self.note("pattern", "SVG: pattern paints were rasterised, so those shapes are not editable".to_string());
            let name = self.name(p.id(), 0);
            return self.raster_layer(node, name, parent);
        }
        // A path has no transform of its own: its data is in its parent's space.
        let ts = parent.post_concat(self.global);
        let data = (*p.data()).clone().transform(ts)?;
        let subpaths = knots(&data);
        if subpaths.is_empty() {
            return None;
        }
        let fill = p.fill().map(|f| self.paint(f.paint(), f.opacity().get(), ts));
        let stroke = p.stroke().map(|s| {
            // Stroke widths are in drawing units; under a non-uniform scale they take the
            // geometric mean, the one width the shape model has.
            let (kx, ky) = ((ts.sx * ts.sx + ts.ky * ts.ky).sqrt(), (ts.kx * ts.kx + ts.sy * ts.sy).sqrt());
            if (kx - ky).abs() > 0.01 * kx.max(ky) {
                self.note("stroke-scale", "SVG: strokes under a non-uniform scale keep one averaged width".to_string());
            }
            let k = (ts.sx * ts.sy - ts.kx * ts.ky).abs().sqrt().max(1e-6);
            let width = (s.width().get() * k).max(0.01);
            ShapeStroke {
                width,
                paint: self.paint(s.paint(), 1.0, ts),
                opacity: s.opacity().get(),
                align: StrokeAlign::Center,
                cap: match s.linecap() {
                    usvg::LineCap::Butt => LineCap::Butt,
                    usvg::LineCap::Round => LineCap::Round,
                    usvg::LineCap::Square => LineCap::Square,
                },
                join: match s.linejoin() {
                    usvg::LineJoin::Round => LineJoin::Round,
                    usvg::LineJoin::Bevel => LineJoin::Bevel,
                    usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                },
                miter_limit: s.miterlimit().get(),
                dashes: s.dasharray().map(|d| d.iter().map(|v| v * k / width).collect()).unwrap_or_default(),
                dash_offset: s.dashoffset() * k / width,
            }
        });
        let fill_rule = match p.fill().map(|f| f.rule()) {
            Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };
        let mut sh = ShapeLayer { path: Path { subpaths, fill_rule, inverted: false }, fill, stroke, ..Default::default() };
        // The pixel cache is optional, so it is charged like any other retained raster and left
        // out (the shape renders from its path) once the budget is spent: compact geometry can
        // cover the whole canvas in every one of thousands of shapes.
        let b = data.bounds();
        let reach = sh.stroke.as_ref().map_or(0.0, |s| s.width);
        let area = Rect::new(
            (b.left() - reach).floor() as i32,
            (b.top() - reach).floor() as i32,
            (b.right() + reach).ceil() as i32,
            (b.bottom() + reach).ceil() as i32,
        )
        .intersect(&self.canvas);
        let pixels = u64::from(area.width()) * u64::from(area.height());
        if self.can_retain_pixels(pixels) {
            sh.cache = Some(photocraft_vector::render_shape(&sh, self.fmt, self.canvas));
            let bytes = pixels * self.fmt.bytes_per_pixel() as u64;
            self.pixel_budget -= bytes;
            self.retained_pixel_bytes += bytes;
        }
        Some(Layer::new(self.name(p.id(), 0), LayerContent::Shape(sh)))
    }

    /// A paint as a layer fill. `ts` maps the paint's space onto the canvas.
    fn paint(&mut self, paint: &usvg::Paint, opacity: f32, ts: tiny_skia::Transform) -> Fill {
        let stops = |g: &usvg::BaseGradient| -> Vec<(f32, Color)> {
            g.stops().iter().map(|s| (s.offset().get(), color(s.color(), s.opacity().get() * opacity))).collect()
        };
        let gradient = |stops: Vec<(f32, Color)>, angle: f32, style: GradientStyle| Fill::Gradient {
            stops,
            angle,
            scale: 1.0,
            style,
            reverse: false,
            opacity_stops: Vec::new(),
            midpoints: Vec::new(),
            offset: (0.0, 0.0),
            dither: false,
            align: true,
        };
        match paint {
            usvg::Paint::Color(c) => Fill::Solid(color(*c, opacity)),
            usvg::Paint::LinearGradient(g) => {
                self.note("gradient", "SVG: gradient paints keep their stops and direction, not their exact geometry".to_string());
                let gt = g.transform().post_concat(ts);
                let mut a = tiny_skia::Point::from_xy(g.x1(), g.y1());
                let mut b = tiny_skia::Point::from_xy(g.x2(), g.y2());
                gt.map_point(&mut a);
                gt.map_point(&mut b);
                // Photoshop's angle: 0° left → right, counter-clockwise positive (y points down).
                let angle = (-(b.y - a.y)).atan2(b.x - a.x).to_degrees();
                gradient(stops(g), if angle.is_finite() { angle } else { 0.0 }, GradientStyle::Linear)
            }
            usvg::Paint::RadialGradient(g) => {
                self.note("gradient", "SVG: gradient paints keep their stops and direction, not their exact geometry".to_string());
                gradient(stops(g), 0.0, GradientStyle::Radial)
            }
            // Pattern paints are rasterised before we get here.
            usvg::Paint::Pattern(_) => Fill::Solid(Color::rgba(0.5, 0.5, 0.5, opacity)),
        }
    }

    /// The node rendered by resvg into a pixel layer over its bounding box, clipped to the canvas.
    /// `parent` is the absolute transform of the node's parent group.
    fn raster_layer(&mut self, node: &usvg::Node, name: String, parent: tiny_skia::Transform) -> Option<Layer> {
        // Where the node lands, from its box in its parent's space (as usvg computes its absolute
        // box, but through `parent`, which is right inside text too).
        let bb = match node {
            usvg::Node::Group(g) => g.layer_bounding_box().transform(parent.pre_concat(g.transform()))?,
            _ => node.bounding_box().to_non_zero_rect()?.transform(parent)?,
        };
        // What resvg shifts the node by before drawing it.
        let shift = node.abs_layer_bounding_box()?;
        let k = self.global.sx;
        let x0 = ((bb.x() * k).floor() as i32).max(self.canvas.x0);
        let y0 = ((bb.y() * k).floor() as i32).max(self.canvas.y0);
        let x1 = ((bb.right() * k).ceil() as i32).min(self.canvas.x1);
        let y1 = ((bb.bottom() * k).ceil() as i32).min(self.canvas.y1);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let pixels = u64::from(w) * u64::from(h);
        // Preflight the pixmap limit first, without charging the shared allowance.
        if pixels > MAX_RASTER_PIXELS {
            self.note("raster-size", "SVG: an element too large to rasterise was dropped".to_string());
            return None;
        }
        if !self.can_retain_pixels(pixels) {
            return None;
        }
        let Some(mut pixmap) = pixmap(w, h) else {
            self.note("raster-alloc", "SVG: a rasterised element could not be allocated".to_string());
            return None;
        };
        // resvg renders the node in its parent's space after shifting its absolute bounding box
        // to the origin: undo that shift, apply the parent's transform, then scale to the canvas
        // and shift so the pixmap starts at (x0, y0).
        let canvas = tiny_skia::Transform::from_row(k, 0.0, 0.0, k, -(x0 as f32), -(y0 as f32));
        let ts = canvas.pre_concat(parent).pre_translate(shift.x(), shift.y());
        resvg::render_node(node, ts, &mut pixmap.as_mut())?;
        let layer = Layer::new(name, LayerContent::Raster(surface(&pixmap, Rect::new(x0, y0, x1, y1), self.fmt)));
        let bytes = pixels * self.fmt.bytes_per_pixel() as u64;
        self.pixel_budget -= bytes;
        self.retained_pixel_bytes += bytes;
        Some(layer)
    }
}

fn color(c: usvg::Color, alpha: f32) -> Color {
    Color::rgba(f32::from(c.red) / 255.0, f32::from(c.green) / 255.0, f32::from(c.blue) / 255.0, alpha.clamp(0.0, 1.0))
}

fn blend(b: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match b {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

fn pt(p: tiny_skia::Point) -> Point {
    Point::new(f64::from(p.x), f64::from(p.y))
}

/// A flattened-transform path as subpaths of knots: lines keep retracted handles, quadratics
/// are raised to cubics, and a closing segment that returns to the start merges into it. Every
/// subpath after the first joins its component, so the fill rule counts winding across the whole
/// path as SVG does (a second subpath drawn inside the first is a hole under even-odd).
fn knots(path: &tiny_skia::Path) -> Vec<Subpath> {
    use tiny_skia::PathSegment as S;
    let mut out: Vec<Subpath> = Vec::new();
    let mut cur: Option<Subpath> = None;
    fn flush(cur: &mut Option<Subpath>, out: &mut Vec<Subpath>, closed: bool) {
        let Some(mut sp) = cur.take() else { return };
        sp.closed = closed;
        if closed && sp.knots.len() > 1 {
            let (first, last) = (sp.knots.first().map(|k| k.anchor), sp.knots.last().map(|k| (k.anchor, k.in_ctrl)));
            if let (Some(a), Some((b, in_ctrl))) = (first, last)
                && (a.x - b.x).abs() < 1e-6
                && (a.y - b.y).abs() < 1e-6
            {
                sp.knots.pop();
                if let Some(k) = sp.knots.first_mut() {
                    k.in_ctrl = in_ctrl;
                }
            }
        }
        if !sp.knots.is_empty() {
            if !out.is_empty() {
                sp.op = PathOp::Join;
            }
            out.push(sp);
        }
    }
    fn curve(sp: &mut Subpath, c1: Point, c2: Point, p: Point) {
        if let Some(last) = sp.knots.last_mut() {
            last.out_ctrl = c1;
        }
        sp.knots.push(Knot { anchor: p, in_ctrl: c2, out_ctrl: p, smooth: false });
    }
    for seg in path.segments() {
        match seg {
            S::MoveTo(p) => {
                flush(&mut cur, &mut out, false);
                cur = Some(Subpath { knots: vec![Knot::corner(f64::from(p.x), f64::from(p.y))], ..Default::default() });
            }
            S::LineTo(p) => {
                if let Some(sp) = cur.as_mut() {
                    sp.knots.push(Knot::corner(f64::from(p.x), f64::from(p.y)));
                }
            }
            S::QuadTo(c, p) => {
                if let Some(sp) = cur.as_mut() {
                    let (c, p) = (pt(c), pt(p));
                    let p0 = sp.knots.last().map_or(p, |k| k.anchor);
                    let c1 = Point::new(p0.x + 2.0 / 3.0 * (c.x - p0.x), p0.y + 2.0 / 3.0 * (c.y - p0.y));
                    let c2 = Point::new(p.x + 2.0 / 3.0 * (c.x - p.x), p.y + 2.0 / 3.0 * (c.y - p.y));
                    curve(sp, c1, c2, p);
                }
            }
            S::CubicTo(c1, c2, p) => {
                if let Some(sp) = cur.as_mut() {
                    curve(sp, pt(c1), pt(c2), pt(p));
                }
            }
            S::Close => flush(&mut cur, &mut out, true),
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

#[cfg(test)]
mod tests;
