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
fn embedded_image_dimensions(mime: &str, data: &[u8]) -> Option<Option<(u32, u32)>> {
    match mime {
        "image/png" => Some(png_dimensions(data)),
        "image/webp" => Some(photocraft_codecs::probe_size(data).ok()),
        "text/plain" => {
            if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
                Some(png_dimensions(data))
            } else if data.get(0..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
                Some(photocraft_codecs::probe_size(data).ok())
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
        sh.cache = Some(photocraft_vector::render_shape(&sh, self.fmt, self.canvas));
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
mod tests {
    use super::*;

    #[test]
    fn sniffing_accepts_svg_and_rejects_other_xml() {
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(is_svg("\u{feff}<?xml version=\"1.0\"?>\n<!-- hi -->\n<svg/>".as_bytes()));
        assert!(is_svg(b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x\"><svg/>"));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"));
        assert!(!is_svg(b"\x89PNG\r\n"));
        assert!(!is_svg(b""));
    }

    #[test]
    fn quadratics_become_cubics_and_closing_merges_the_start_knot() {
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.quad_to(5.0, 10.0, 10.0, 0.0);
        pb.line_to(0.0, 0.0);
        pb.close();
        pb.move_to(20.0, 20.0);
        pb.line_to(30.0, 20.0);
        let path = pb.finish().unwrap();
        let sp = knots(&path);
        assert_eq!(sp.len(), 2);
        assert!(sp[0].closed && !sp[1].closed);
        // The explicit line back to the start was merged into the first knot.
        assert_eq!(sp[0].knots.len(), 2);
        let (a, b) = (&sp[0].knots[0], &sp[0].knots[1]);
        assert!((a.out_ctrl.x - 10.0 / 3.0).abs() < 1e-9 && (a.out_ctrl.y - 20.0 / 3.0).abs() < 1e-9);
        assert!((b.in_ctrl.x - 20.0 / 3.0).abs() < 1e-9 && (b.in_ctrl.y - 20.0 / 3.0).abs() < 1e-9);
        assert_eq!(b.anchor, Point::new(10.0, 0.0));
        assert_eq!(sp[1].knots.len(), 2);
    }

    #[test]
    fn text_without_any_font_is_reported_not_silently_dropped() {
        // No font at all (a bare CI image): usvg drops the <text> while parsing, so nothing
        // reaches the conversion; the reader still has to say why the text is missing.
        fn no_fonts() -> Arc<usvg::fontdb::Database> {
            Arc::new(usvg::fontdb::Database::new())
        }
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\">II</text></svg>";
        let parsed = parse_with(svg, no_fonts).unwrap();
        assert!(parsed.fontless_text);
        let r = import_parsed("t", parsed, MAX_GROUP_DEPTH).unwrap();
        assert!(r.document.layers.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("font")), "{:?}", r.warnings);
        // A drawing without text never loads or misses fonts.
        let parsed =
            parse_with(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\"/></svg>", no_fonts).unwrap();
        assert!(!parsed.fontless_text);
        let r = import_parsed("t", parsed, MAX_GROUP_DEPTH).unwrap();
        assert!(r.warnings.iter().all(|w| !w.contains("font")), "{:?}", r.warnings);
    }

    /// #4806: text placed by a `transform` (Illustrator writes `matrix(1 0 0 1 x y)` on every text
    /// element), on the text or on a group around it, lands where resvg draws it. usvg leaves the
    /// glyph outlines' absolute transforms at identity, so reading them dropped the transform: the
    /// text went off the canvas or kept its untransformed place and size.
    ///
    /// Shaped with the bundled Inter alone, never a system font: on a machine without fonts (a
    /// bare CI image or VM) the test would otherwise have nothing to check, and the drawing and
    /// its reference come from the same parse, so both use that face.
    #[test]
    fn transformed_text_lands_where_resvg_draws_it() {
        fn inter() -> Arc<usvg::fontdb::Database> {
            let mut db = usvg::fontdb::Database::new();
            db.load_font_data(photocraft_text::fonts::INTER_REGULAR.to_vec());
            Arc::new(db)
        }
        assert!(inter().faces().next().is_some(), "the bundled Inter loads");
        let font = r#"font-size="24" font-family="Inter" fill="black""#;
        let cases = [
            format!(r#"<text transform="translate(10 40)" {font}>IHI</text>"#),
            format!(r#"<text transform="matrix(1 0 0 1 10 40)" {font}>IHI</text>"#),
            format!(r#"<text transform="translate(10 0)" x="20" y="40" {font}>IHI</text>"#),
            format!(r#"<text transform="scale(2)" x="5" y="25" {font}>IHI</text>"#),
            format!(r#"<g transform="translate(10 40)"><text {font}>IHI</text></g>"#),
            format!(r#"<g transform="translate(60 0)"><g transform="rotate(90)"><text x="5" y="20" {font}>IHI</text></g></g>"#),
        ];
        for c in cases {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60">{c}</svg>"#);
            let parsed = parse_with(svg.as_bytes(), inter).unwrap();
            assert!(!parsed.fontless_text, "{c}");
            let mut reference = tiny_skia::Pixmap::new(120, 60).unwrap();
            resvg::render(&parsed.tree, tiny_skia::Transform::default(), &mut reference.as_mut());
            let r = import_parsed("t", parsed, MAX_GROUP_DEPTH).unwrap();
            let [layer] = r.document.layers.as_slice() else { panic!("one text layer, got {}: {c}", r.document.layers.len()) };
            assert!(matches!(layer.content, LayerContent::Group(_)), "text stays outline shapes: {c}");
            let imported = photocraft_compose::flatten(&r.document);
            let (mut both, mut either) = (0usize, 0usize);
            let (mut a_box, mut b_box) = ([i32::MAX, i32::MAX, i32::MIN, i32::MIN], [i32::MAX, i32::MAX, i32::MIN, i32::MIN]);
            let grow = |bx: &mut [i32; 4], x: i32, y: i32| *bx = [bx[0].min(x), bx[1].min(y), bx[2].max(x), bx[3].max(y)];
            for (i, px) in reference.pixels().iter().enumerate() {
                let (x, y) = ((i % 120) as i32, (i / 120) as i32);
                let (a, b) = (imported.get(x, y)[3] > 0.5, px.alpha() > 127);
                if a {
                    grow(&mut a_box, x, y);
                }
                if b {
                    grow(&mut b_box, x, y);
                }
                both += usize::from(a && b);
                either += usize::from(a || b);
            }
            assert!(either > 50, "resvg draws the text on the canvas: {c}");
            // Same place and size to the pixel; the outlines are filled by a different rasteriser,
            // so the edges of thin stems may differ by half a pixel.
            assert!(a_box.iter().zip(b_box).all(|(a, b)| (a - b).abs() <= 1), "imported {a_box:?}, resvg {b_box:?}: {c}");
            assert!(both * 10 >= either * 7, "imported glyphs cover what resvg draws ({both} of {either} px): {c}");
        }
    }

    /// #4266: a group clipped by one path (the PhotoCraft icon's rounded square) stays editable
    /// shapes with the clip as its vector mask, and looks as resvg draws the drawing; a clip of
    /// two shapes is still rasterised, with the warning.
    #[test]
    fn a_single_path_clip_becomes_the_groups_vector_mask() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
            <defs><clipPath id="c"><rect x="4" y="4" width="40" height="40" rx="10"/></clipPath></defs>
            <g transform="translate(8 6)" clip-path="url(#c)">
              <rect width="64" height="64" fill="#2f7bf5"/>
              <circle cx="40" cy="40" r="20" fill="#efe9dc"/>
            </g></svg>"##;
        let r = import_svg("t", svg).unwrap();
        assert!(r.warnings.iter().all(|w| !w.contains("clip-path")), "{:?}", r.warnings);
        let [group] = r.document.layers.as_slice() else { panic!("one group: {:?}", r.document.layers.len()) };
        let LayerContent::Group(g) = &group.content else { panic!("a group") };
        assert_eq!(g.children.len(), 2);
        assert!(g.children.iter().all(|c| matches!(c.content, LayerContent::Shape(_))), "editable shapes");
        let mask = group.vector_mask.as_ref().expect("the clip as a vector mask");
        let xs: Vec<f64> = mask.path.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor.x)).collect();
        let (lo, hi) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
        assert!((lo - 12.0).abs() < 1e-3 && (hi - 52.0).abs() < 1e-3, "in the group's space: {lo}..{hi}");
        // Pixel for pixel what resvg draws, but for anti-aliasing along the edges.
        let ours = photocraft_compose::flatten(&r.document).to_rgba8().pixels;
        let tree = parse(svg).unwrap().tree;
        let mut pm = tiny_skia::Pixmap::new(64, 64).unwrap();
        resvg::render(&tree, tiny_skia::Transform::default(), &mut pm.as_mut());
        let theirs: Vec<u8> = pm
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let off = ours.chunks(4).zip(theirs.chunks(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 48)).count();
        assert!(off < 64, "{off} pixels differ");
        // The report's file: PhotoCraft's own icon opens without the clip-path warning.
        let icon = import_svg("photocraft.svg", include_bytes!("../../../assets/app-icon/photocraft.svg")).unwrap();
        assert!(icon.warnings.iter().all(|w| !w.contains("clip-path")), "{:?}", icon.warnings);
        assert!(icon.document.layers.iter().any(|l| l.vector_mask.is_some()), "its rounded square is a vector mask");
        // Two shapes in the clip: rasterised, and said so.
        let two = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
            <defs><clipPath id="c"><rect width="20" height="20"/><rect x="30" width="20" height="20"/></clipPath></defs>
            <g clip-path="url(#c)"><rect width="64" height="64"/><rect width="9" height="9" fill="red"/></g></svg>"##;
        let r = import_svg("t", two).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("clip-path")), "{:?}", r.warnings);
    }

    /// A database with the bundled Inter alone, generics resolved without Fontconfig: none of
    /// fontdb's Windows defaults (Arial, Times New Roman, Courier New) is in it.
    fn inter_only() -> Arc<usvg::fontdb::Database> {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec());
        resolve_generic_families(&mut db, |_| None);
        Arc::new(db)
    }

    /// Text asking for fonts that aren't installed (`Arial, Helvetica, sans-serif` on a Linux
    /// without them) is set in an installed font, as a browser does, instead of being dropped.
    #[test]
    fn text_in_missing_fonts_falls_back_to_an_installed_one() {
        for family in ["Arial, Helvetica, sans-serif", "Times New Roman", "Courier New, monospace", "cursive", "fantasy"] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\" font-family=\"{family}\">II</text></svg>"
            );
            let parsed = parse_with(svg.as_bytes(), inter_only).unwrap();
            assert!(!parsed.fontless_text);
            let r = import_parsed("t", parsed, MAX_GROUP_DEPTH).unwrap();
            assert_eq!(r.document.layers.first().map(|l| l.name.as_str()), Some("II"), "{family}: {:?}", r.warnings);
        }
    }

    #[test]
    fn generic_families_point_at_installed_fonts() {
        use usvg::fontdb::Family;
        let db = inter_only();
        for g in [Family::SansSerif, Family::Serif, Family::Monospace, Family::Cursive, Family::Fantasy] {
            assert_eq!(db.family_name(&g), "Inter", "{g:?}");
        }
        // The system's choice wins when installed; a generic whose font is installed stays.
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec());
        db.load_font_data(include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec());
        db.set_cursive_family("Inter");
        resolve_generic_families(&mut db, |g| Some(if g == "serif" { "JetBrains Mono" } else { "Not Installed" }.to_string()));
        assert_eq!(db.family_name(&Family::Serif), "JetBrains Mono");
        assert_eq!(db.family_name(&Family::Monospace), "JetBrains Mono", "the bundled mono stand-in");
        assert_eq!(db.family_name(&Family::SansSerif), "Inter");
        assert_eq!(db.family_name(&Family::Cursive), "Inter");
        // An empty database is left alone.
        let mut empty = usvg::fontdb::Database::new();
        resolve_generic_families(&mut empty, |_| Some("Inter".into()));
        assert_eq!(empty.family_name(&Family::SansSerif), "Arial");
    }

    #[test]
    fn rasterized_groups_stop_at_the_aggregate_pixel_budget() {
        let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">");
        svg.push_str("<defs><mask id=\"m\"><rect width=\"256\" height=\"256\" fill=\"white\"/></mask></defs>");
        for i in 0..8 {
            svg.push_str(&format!("<g id=\"g{i}\" mask=\"url(#m)\"><rect width=\"256\" height=\"256\" fill=\"#00ff00\"/></g>"));
        }
        svg.push_str("</svg>");
        // Enough for one retained 256×256 RGBA8 surface, not eight of them.
        let budget = 256u64 * 256 * 4;
        let result = import_parsed_with_budget("budget", parse(svg.as_bytes()).unwrap(), MAX_GROUP_DEPTH, budget).unwrap();
        assert_eq!(result.document.layers.len(), 1, "one U8 surface fills the retained-byte allowance");
        assert_eq!(result.document.layers[0].name, "g0");
        assert!(result.warnings.iter().any(|warning| warning.contains("pixel memory")), "{:?}", result.warnings);
    }

    #[test]
    fn oversized_raster_does_not_consume_budget_for_later_layers() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="5100" height="4800">
            <defs>
              <mask id="big" maskUnits="userSpaceOnUse" x="0" y="0" width="5100" height="4800">
                <rect width="5100" height="4800" fill="white"/>
              </mask>
              <mask id="small-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="32" height="32">
                <rect width="32" height="32" fill="white"/>
              </mask>
            </defs>
            <g id="oversized" mask="url(#big)"><rect width="5100" height="4800" fill="red"/></g>
            <g id="small" mask="url(#small-mask)"><rect width="32" height="32" fill="blue"/></g>
        </svg>"##;
        let result = import_parsed_with_budget("budget", parse(svg).unwrap(), MAX_GROUP_DEPTH, 64 * 64 * 4).unwrap();
        assert_eq!(result.document.layers.len(), 1, "the oversized group must not debit the budget");
        assert_eq!(result.document.layers[0].name, "small");
        assert!(result.warnings.iter().any(|w| w.contains("too large to rasterise")), "{:?}", result.warnings);
        assert!(!result.warnings.iter().any(|w| w.contains("pixel memory budget")), "{:?}", result.warnings);
    }

    #[test]
    fn skipped_expensive_raster_does_not_hide_a_smaller_one() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256">
            <defs>
              <mask id="big"><rect width="256" height="256" fill="white"/></mask>
              <mask id="little"><rect width="16" height="16" fill="white"/></mask>
            </defs>
            <g id="large" mask="url(#big)"><rect width="256" height="256" fill="red"/></g>
            <g id="small" mask="url(#little)"><rect width="16" height="16" fill="blue"/></g>
        </svg>"##;
        let result = import_parsed_with_budget("budget", parse(svg).unwrap(), MAX_GROUP_DEPTH, 32 * 32 * 4).unwrap();
        assert_eq!(result.document.layers.len(), 1);
        assert_eq!(result.document.layers[0].name, "small");
        assert!(result.warnings.iter().any(|w| w.contains("some rasterised elements were omitted")), "{:?}", result.warnings);
    }

    #[test]
    fn huge_drawings_scale_to_the_side_limit() {
        let tree = parse(b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100000 50000\"/>").unwrap().tree;
        let mut w = Vec::new();
        let cv = canvas(&tree, &mut w);
        assert_eq!((cv.size.width, cv.size.height), (MAX_SIDE, MAX_SIDE / 2));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn resolve_data_rejects_oversized_and_malformed_embedded_png_and_webp() {
        let opt = options(Arc::new(usvg::fontdb::Database::new()));
        let resolve = &opt.image_href_resolver.resolve_data;
        let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255; 12]).unwrap();
        let valid_png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        assert!(resolve("image/png", Arc::new(valid_png.clone()), &opt).is_some());
        assert!(resolve("text/plain", Arc::new(valid_png.clone()), &opt).is_some());

        let mut huge_png = valid_png;
        huge_png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
        huge_png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
        assert!(resolve("image/png", Arc::new(huge_png.clone()), &opt).is_none(), "oversized PNG rejected before decode");
        assert!(resolve("text/plain", Arc::new(huge_png), &opt).is_none(), "oversized mime-less PNG rejected before decode");

        // Malformed WebP header under image/webp or text/plain must be rejected at the probe step.
        let bad_webp = b"RIFF\x10\x00\x00\x00WEBPVP8 ".to_vec();
        assert!(resolve("image/webp", Arc::new(bad_webp.clone()), &opt).is_none());
        assert!(resolve("text/plain", Arc::new(bad_webp), &opt).is_none());
    }

    #[test]
    fn resolve_data_rejects_nested_gzip_and_handles_text_plain_svgz() {
        use std::io::Write;
        fn gz(bytes: &[u8]) -> Vec<u8> {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(bytes).unwrap();
            e.finish().unwrap()
        }
        let opt = options(Arc::new(usvg::fontdb::Database::new()));
        let resolve = &opt.image_href_resolver.resolve_data;
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><rect width=\"4\" height=\"4\"/></svg>";
        let once = gz(svg);
        assert!(resolve("image/svg+xml", Arc::new(once.clone()), &opt).is_some());
        assert!(resolve("text/plain", Arc::new(once.clone()), &opt).is_some());

        // Doubly gzip-compressed payload must be rejected rather than decompressed a second time by usvg.
        let twice = gz(&once);
        assert!(resolve("image/svg+xml", Arc::new(twice.clone()), &opt).is_none());
        assert!(resolve("text/plain", Arc::new(twice), &opt).is_none());
    }
}
