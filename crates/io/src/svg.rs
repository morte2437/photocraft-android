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

use std::sync::{Arc, OnceLock};

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
/// (`usvg` already blanks it for sub-SVGs).
fn options(fontdb: Arc<usvg::fontdb::Database>) -> usvg::Options<'static> {
    let default_resolver = usvg::ImageHrefResolver::default_data_resolver();
    let resolve_data: usvg::ImageHrefDataResolverFn<'static> = Box::new(move |mime, data, opts| {
        // Reject embedded images whose decoded RGBA storage would exceed the budget before
        // resvg (which allocates the full intrinsic image before any destination scaling) ever
        // sees them. Formats without a cheap header probe fall through unchanged; their decode
        // path is bounded elsewhere or accepted as-is, matching the previous behavior.
        if let Some(probed) = embedded_image_dimensions(mime, &data) {
            let (w, h) = probed?;
            let decoded = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4);
            if decoded > MAX_EMBEDDED_IMAGE_BYTES {
                return None;
            }
        }
        // An embedded `image/svg+xml` value can itself be gzip-compressed (SVGZ-in-data-URL).
        // The default resolver's sub-SVG loader checks for the gzip magic and decompresses
        // again with no byte limit of its own. Apply our bounded inflate first: if the data is
        // gzip-compressed, hand the default resolver the already-decompressed (and
        // size-checked) bytes, which no longer start with the gzip magic, so it parses them
        // directly instead of decompressing a second time.
        if (mime == "image/svg+xml" || mime == "text/plain") && data.starts_with(&[0x1f, 0x8b]) {
            return match inflate(&data) {
                Ok(xml) if !xml.starts_with(&[0x1f, 0x8b]) => default_resolver("image/svg+xml", std::sync::Arc::new(xml.into_owned()), opts),
                _ => None,
            };
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
    if nesting_depth(&xml, MAX_XML_DEPTH) > MAX_XML_DEPTH {
        return Err(IoError::Svg(format!("elements are nested more than {MAX_XML_DEPTH} levels deep")));
    }
    let has_text = xml.windows(4).any(|w| w == b"text");
    let fontdb = if has_text { fonts() } else { Arc::new(usvg::fontdb::Database::new()) };
    let fontless_text = has_text && fontdb.faces().next().is_none();
    let opt = options(fontdb);
    // Parse the already-bounded-and-decoded text with from_str, not from_data: from_data
    // re-checks the gzip magic and would decompress a second time without our size limit if
    // the inflated content happened to start with a nested gzip stream.
    let text = std::str::from_utf8(&xml).map_err(|_| IoError::Svg("the SVG is not valid UTF-8".into()))?;
    let tree = usvg::Tree::from_str(text, &opt).map_err(|e| IoError::Svg(e.to_string()))?;
    Ok(Parsed { tree, fontless_text })
}

/// The plain XML of `bytes`: itself, or the inflated content of a gzip `.svgz`, refused past
/// [`MAX_INFLATED_BYTES`] (a decompression bomb) or when the stream is corrupt.
fn inflate(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, IoError> {
    use std::io::Read;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(std::borrow::Cow::Borrowed(bytes));
    }
    let mut out = Vec::new();
    let mut reader = flate2::read::MultiGzDecoder::new(bytes).take(MAX_INFLATED_BYTES.saturating_add(1));
    reader.read_to_end(&mut out).map_err(|e| IoError::Svg(format!("the compressed SVG couldn't be inflated: {e}")))?;
    if u64::try_from(out.len()).unwrap_or(u64::MAX) > MAX_INFLATED_BYTES {
        return Err(IoError::Svg(format!("the compressed SVG inflates past {} MB", MAX_INFLATED_BYTES >> 20)));
    }
    Ok(std::borrow::Cow::Owned(out))
}

/// The deepest element nesting in `xml`, stopping as soon as it passes `limit` (the result is
/// then `limit + 1`). Comments, CDATA sections, processing instructions and the DOCTYPE add
/// nothing; quoted attribute values may hold `>` or `/>`. Malformed markup is only counted
/// loosely: the parser rejects it afterwards.
fn nesting_depth(xml: &[u8], limit: usize) -> usize {
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
    import_parsed(name, parse(bytes)?, max_group_depth)
}

fn import_parsed(name: &str, parsed: Parsed, max_group_depth: usize) -> Result<ImportResult, IoError> {
    import_parsed_with_budget(name, parsed, max_group_depth, MAX_PIXEL_BYTES)
}

fn import_parsed_with_budget(name: &str, parsed: Parsed, max_group_depth: usize, pixel_budget: u64) -> Result<ImportResult, IoError> {
    let Parsed { tree, fontless_text } = parsed;
    let mut warnings = Vec::new();
    if fontless_text {
        warnings.push("SVG: the text needs a font and none is installed, so it was dropped".to_string());
    }
    let cv = canvas(&tree, &mut warnings);
    let mut doc = Document::new(name, cv.size, ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let global = tiny_skia::Transform::from_scale(cv.scale, cv.scale);
    let max_group_depth = (max_group_depth != 0).then_some(max_group_depth.clamp(1, MAX_GROUP_DEPTH));
    let mut conv =
        Conv { fmt, canvas: doc.bounds(), global, max_group_depth, warnings, noted: Vec::new(), counts: [0; 4], pixel_budget, retained_pixel_bytes: 0 };
    if count_nodes(tree.root()) > MAX_NODES {
        conv.warnings.push(format!("SVG: more than {MAX_NODES} elements; opened as one pixel layer instead of shapes"));
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
    let cv = canvas(&tree, &mut Vec::new());
    let k = cv.scale * scale.clamp(1.0 / 64.0, 64.0);
    let side = |v: f32| ((v * k).ceil() as u32).max(1);
    let (w, h) = (side(tree.size().width()), side(tree.size().height()));
    let mut pixmap = pixmap(w, h).ok_or_else(|| IoError::Svg(format!("{w} × {h} px is too large to rasterise")))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    let rect = Rect::new(0, 0, w as i32, h as i32);
    let px = pixmap.pixels().iter().map(|p| rgba(p.demultiply())).collect();
    Ok(photocraft_compose::Buffer { rect, px })
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
fn count_nodes(root: &usvg::Group) -> usize {
    let mut n = 0;
    let mut stack: Vec<&usvg::Group> = vec![root];
    while let Some(g) = stack.pop() {
        for node in g.children() {
            n += 1;
            if let usvg::Node::Group(child) = node {
                stack.push(child);
            }
            if n > MAX_NODES {
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
                let mut l = Layer::new(self.name(g.id(), 1), LayerContent::Group(Group { children, expanded: false, artboard: None }));
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
                out.push(Layer::new(name, LayerContent::Group(Group { children, expanded: false, artboard: None })));
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
