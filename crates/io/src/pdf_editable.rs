//! Experimental editable PDF import: a `craft-pdf` device turns painted paths into shape layers
//! and embedded-font text into text layers.
//!
//! Solid fills and strokes become [`ShapeLayer`]s in document pixels; embedded fonts whose glyphs
//! carry Unicode and a PostScript name become [`photocraft_doc::TextLayer`]s, with the font bytes
//! registered in the app's shared text engine. A page-sized axis-aligned clip that contains the
//! artboard is a no-op and is accepted. Images, other clipping paths, transparency groups,
//! patterns, soft masks, annotations and unsupported text mark the page unsupported, and the
//! caller falls back to the raster importer for that page with a warning. Nothing is ever half
//! imported.
//!
//! This is the app-owned `Device` route: Hayro's interpreter walks the page and reports each
//! paint with its real colour and geometry, so shape layers need no page raster underneath. That
//! is what makes moving or deleting a layer behave, and what avoids the black-path and
//! doubled-text failures of the old PDFium import.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use craft_pdf::interpret::color::Color as HayroColor;
use craft_pdf::interpret::font::{FontData, Glyph, OutlineFontData};
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, InterpreterWarning, Paint, PathDrawMode, SoftMask,
    TransformExt, interpret_page,
};
use craft_pdf::syntax::page::Page;
use kurbo::{Affine as KurboAffine, BezPath, Cap, Join, PathEl};
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::text::{CharStyle, TextRun};
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, StrokeAlign, Subpath};
use photocraft_doc::{Fill, Layer, LayerContent, TextLayer};
use photocraft_geom::{Affine, Point, Rect, TILE_SIZE};
use photocraft_raster::Interrupt;

/// Most layers one editable page may create; past it the page falls back to raster.
const MAX_LAYERS: usize = 20_000;
/// Most path knots one editable page may hold; past it the page falls back to raster.
const MAX_KNOTS: usize = 1_000_000;
/// Allocated tile bytes of all shape and text caches retained by one editable page.
const MAX_CACHE_BYTES: u64 = 256 << 20;
/// Bound text collection before building its layers and caches, including repeated overprinting.
const MAX_GLYPHS: usize = 100_000;
/// The strings in collected glyph records must also fit, not just their record count.
const MAX_GLYPH_BYTES: usize = 16 << 20;
/// Cap on fill/stroke glyph comparisons so hostile pages cannot make pairing quadratic forever.
const MAX_PAIR_CHECKS: usize = 1_000_000;
/// How often the watcher copies the interrupt into craft-pdf's cancellation flag. The Device's
/// callbacks are only reached per paint, so a page with one long path still needs this; 20 ms
/// keeps wakeups negligible while staying responsive.
const POLL: Duration = Duration::from_millis(20);
/// Largest gap (in em) inside one run before a space is inserted.
const WORD_GAP: f64 = 0.2;
/// Largest baseline offset (in em) for glyphs to share one line.
const LINE_TOLERANCE: f64 = 0.30;

/// Why an editable page could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditableError {
    /// The caller cancelled the import.
    Cancelled,
    /// The page uses something this first step cannot represent; rasterise it instead.
    Unsupported(&'static str),
}

/// Builds one page's shape layers in document coordinates inside `rect`.
///
/// `rect` is the page's artboard in document pixels; the page's displayed size comes from
/// `Page::render_dimensions()` (CropBox, rotation and UserUnit already applied), so a point maps
/// to `rect` by the per-axis scale `rect.size / render_dimensions`. Nothing here re-applies the
/// CropBox offset or rotation.
pub fn page(page: &Page<'_>, rect: Rect, ctl: &Interrupt) -> Result<Vec<Layer>, EditableError> {
    ctl.check().map_err(|_| EditableError::Cancelled)?;
    let (width_pt, height_pt) = page.render_dimensions();
    if !width_pt.is_finite() || !height_pt.is_finite() || width_pt <= 0.0 || height_pt <= 0.0 {
        return Err(EditableError::Unsupported("the page has no area"));
    }
    let sx = f64::from(rect.width()) / f64::from(width_pt);
    let sy = f64::from(rect.height()) / f64::from(height_pt);
    let doc = Affine::translate(f64::from(rect.x0), f64::from(rect.y0)).mul(&Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] });
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let warned = Arc::new(AtomicU8::new(0));
    let sink = Arc::clone(&warned);
    let settings = InterpreterSettings {
        cancelled: Some(Arc::clone(&cancelled)),
        warning_sink: Arc::new(move |warning| {
            let bit = match warning {
                InterpreterWarning::ImageDecodeFailure => 1,
                InterpreterWarning::UnsupportedFont => 2,
                InterpreterWarning::ContentTruncated => 4,
            };
            sink.fetch_or(bit, Ordering::Relaxed);
        }),
        ..InterpreterSettings::default()
    };
    let cache = InterpreterCache::new();
    let mut device = PathDevice {
        doc,
        clip: rect,
        fmt,
        dpi: 72.0 * sx,
        ctl: *ctl,
        layers: Vec::new(),
        glyphs: Vec::new(),
        glyph_bytes: 0,
        strokes: Vec::new(),
        fonts: Vec::new(),
        font_keys: HashMap::new(),
        seen_fonts: HashSet::new(),
        soft_mask: false,
        clips: Vec::new(),
        knots: 0,
        cached: 0,
        unsupported: None,
        cancelled: false,
    };
    std::thread::scope(|scope| {
        let flag = Arc::clone(&cancelled);
        let done = Arc::clone(&finished);
        let ctl = *ctl;
        scope.spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if ctl.cancelled() {
                    flag.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(POLL);
            }
        });
        // A hostile page must not take the process down. This only guards profiles that unwind;
        // a `panic = "abort"` build aborts here just as the raster path would.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let bbox = kurbo::Rect::new(0.0, 0.0, f64::from(width_pt), f64::from(height_pt));
            let mut context = Context::new(page.initial_transform(true).to_kurbo(), bbox, &cache, page.xref(), settings);
            interpret_page(page, &mut context, &mut device);
        }));
        if result.is_err() {
            device.unsupported = Some("the page could not be interpreted");
        }
        finished.store(true, Ordering::Relaxed);
    });
    if device.cancelled || ctl.cancelled() {
        return Err(EditableError::Cancelled);
    }
    let warned = warned.load(Ordering::Relaxed);
    if warned & 4 != 0 {
        return Err(EditableError::Unsupported("the page content was truncated"));
    }
    if warned & 2 != 0 {
        return Err(EditableError::Unsupported("a font could not be used"));
    }
    if warned & 1 != 0 {
        return Err(EditableError::Unsupported("an image could not be decoded"));
    }
    if let Some(reason) = device.unsupported {
        return Err(EditableError::Unsupported(reason));
    }
    pair_fake_bold(&mut device.glyphs, device.strokes, ctl)?;
    let families = register_fonts(&device.fonts)?;
    let texts = text_layers(&device.glyphs, &families, device.dpi, ctl)?;
    if device.layers.len().saturating_add(texts.len()) > MAX_LAYERS {
        return Err(EditableError::Unsupported("too many objects"));
    }
    let mut layers = merge_text_layers(device.layers, texts);
    render_text_caches(&mut layers, device.dpi, device.fmt, ctl, &mut device.cached)?;
    Ok(layers)
}

/// Removes stroked glyphs that duplicate a fill at the same place, marking that glyph fake bold.
fn pair_fake_bold(glyphs: &mut [GlyphRun], strokes: Vec<GlyphRun>, ctl: &Interrupt) -> Result<(), EditableError> {
    // Index fills by the exact attributes a stroke must match, so each stroke only compares
    // against candidates at its own paint order/font/text instead of the whole page.
    let mut index: HashMap<(usize, u128, u64), Vec<usize>> = HashMap::new();
    for (at, glyph) in glyphs.iter().enumerate() {
        if at.is_multiple_of(256) && ctl.check().is_err() {
            return Err(EditableError::Cancelled);
        }
        index.entry((glyph.order, glyph.font.key, text_hash(&glyph.text))).or_default().push(at);
    }
    let mut paired = vec![false; glyphs.len()];
    let mut checks = 0usize;
    for stroke in strokes {
        let mut found = None;
        if let Some(candidates) = index.get(&(stroke.order, stroke.font.key, text_hash(&stroke.text))) {
            for &at in candidates {
                if paired[at] {
                    continue;
                }
                if checks == MAX_PAIR_CHECKS {
                    return Err(EditableError::Unsupported("too much stroked text"));
                }
                if checks.is_multiple_of(256) && ctl.check().is_err() {
                    return Err(EditableError::Cancelled);
                }
                checks += 1;
                let glyph = &glyphs[at];
                let tolerance = 0.01 * glyph.size_px + 0.01;
                if glyph.text == stroke.text
                    && same_style(glyph, &stroke)
                    && (glyph.origin.x - stroke.origin.x).abs() <= tolerance
                    && (glyph.origin.y - stroke.origin.y).abs() <= tolerance
                {
                    found = Some(at);
                    break;
                }
            }
        }
        let Some(at) = found else {
            return Err(EditableError::Unsupported("stroked text"));
        };
        glyphs[at].fake_bold = true;
        paired[at] = true;
    }
    Ok(())
}

/// A hash of a glyph string for the fake-bold index; equality is still checked on the string.
fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Builds one point-text layer per line and style run, with the layer index it must be inserted
/// at to preserve paint order relative to paths.
fn text_layers(
    glyphs: &[GlyphRun],
    families: &std::collections::HashMap<u128, String>,
    dpi: f64,
    ctl: &Interrupt,
) -> Result<Vec<(usize, Layer)>, EditableError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < glyphs.len() {
        ctl.check().map_err(|_| EditableError::Cancelled)?;
        if out.len() >= MAX_LAYERS {
            return Err(EditableError::Unsupported("too many text objects"));
        }
        let first = &glyphs[i];
        let family = families.get(&first.font.key).cloned().unwrap_or_else(|| first.font.family.clone());
        let (dx, dy) = (first.angle.cos(), first.angle.sin());
        let (nx, ny) = (-dy, dx);
        let baseline = first.origin.x * nx + first.origin.y * ny;
        let start = i;
        let mut text = String::new();
        let mut positions = Vec::new();
        let mut last = first;
        while i < glyphs.len() {
            let glyph = &glyphs[i];
            if glyph.order != first.order || !same_style(glyph, first) {
                break;
            }
            let line = glyph.origin.x * nx + glyph.origin.y * ny;
            if (line - baseline).abs() > LINE_TOLERANCE * first.size_px {
                break;
            }
            if i > start {
                let along = (glyph.origin.x - first.origin.x) * dx + (glyph.origin.y - first.origin.y) * dy;
                let prev = (last.origin.x - first.origin.x) * dx + (last.origin.y - first.origin.y) * dy;
                let gap = along - (prev + last.advance);
                if gap > 3.0 * first.size_px {
                    break;
                }
                if gap > WORD_GAP * first.size_px && !text.ends_with(' ') {
                    text.push(' ');
                }
            }
            positions.push((text.len(), glyph.origin));
            text.push_str(&glyph.text);
            last = glyph;
            i += 1;
        }
        if !text.is_empty() && has_ink(&text) {
            let layer = text_layer(text, first, out.len(), &family);
            if let LayerContent::Text(text) = &layer.content {
                let shared = photocraft_text::shared();
                let mut engine = shared.lock().unwrap_or_else(|error| error.into_inner());
                let layout = engine.layout(text, dpi as f32);
                // PDF spacing is already in the glyph origins. A word-gap heuristic cannot
                // preserve TJ, character spacing, or explicitly positioned glyphs. Keep a run
                // editable only if the actual text layout reproduces every recorded origin.
                let clusters: std::collections::HashMap<_, _> = layout.clusters.iter().map(|c| (c.range.start, c)).collect();
                for (offset, expected) in positions {
                    let cluster = clusters.get(&offset);
                    let actual =
                        cluster.and_then(|c| layout.lines.get(c.line).map(|line| text.transform.apply(Point::new(f64::from(c.x), f64::from(line.baseline)))));
                    if actual.is_none_or(|at| (at.x - expected.x).abs() > 0.5 || (at.y - expected.y).abs() > 0.5) {
                        return Err(EditableError::Unsupported("text positioning"));
                    }
                }
            }
            out.push((first.order, layer));
        }
        if i == start {
            i += 1;
        }
    }
    Ok(out)
}

/// Whether `b` belongs to the same line and style run as `a`.
fn same_style(a: &GlyphRun, b: &GlyphRun) -> bool {
    let weight = |glyph: &GlyphRun| if glyph.fake_bold { 700 } else { glyph.font.weight };
    a.font.key == b.font.key
        && a.font.family == b.font.family
        && a.font.italic == b.font.italic
        && weight(a) == weight(b)
        && (a.size_pt - b.size_pt).abs() < 0.05
        && (a.angle - b.angle).abs() < 0.02
        && (a.color.c[0] - b.color.c[0]).abs() < 0.004
        && (a.color.c[1] - b.color.c[1]).abs() < 0.004
        && (a.color.c[2] - b.color.c[2]).abs() < 0.004
        && (a.color.alpha - b.color.alpha).abs() < 0.004
}

/// One line's point-text layer, with the family the shared engine registered.
fn text_layer(text: String, run: &GlyphRun, number: usize, family: &str) -> Layer {
    let weight = if run.fake_bold { 700 } else { run.font.weight };
    let font_style = match (weight >= 600, run.font.italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    };
    let style = CharStyle {
        font_family: family.to_string(),
        font_style: font_style.to_string(),
        postscript_name: Some(run.font.postscript.clone()),
        weight: u16::try_from(weight).unwrap_or(400),
        italic: run.font.italic,
        faux_bold: run.fake_bold,
        // PDF positions already contain kerning; applying font kerning again moves glyphs.
        kerning: photocraft_doc::text::Kerning::Off,
        size_pt: run.size_pt as f32,
        color: run.color,
        ..CharStyle::default()
    };
    let layer = TextLayer {
        text: text.clone(),
        font_family: family.to_string(),
        size_pt: run.size_pt as f32,
        color: run.color,
        transform: Affine::translate(run.origin.x, run.origin.y).mul(&Affine::rotate(run.angle)),
        runs: vec![TextRun { len: text.len(), style }],
        ..TextLayer::default()
    };
    Layer::new(format!("Text {}", number + 1), LayerContent::Text(layer))
}

/// Registers each embedded face with the shared engine under its content-keyed family and returns
/// the family each font key must use.
fn register_fonts(fonts: &[(u128, String, String, FontData)]) -> Result<std::collections::HashMap<u128, String>, EditableError> {
    let shared = photocraft_text::shared();
    let mut families = std::collections::HashMap::new();
    for (key, postscript, derived, data) in fonts {
        // Copy before taking the engine lock; parsing still happens under it, but the UI's
        // layout is not blocked while the bytes are cloned.
        let bytes = data.as_ref().as_ref().to_vec();
        let mut engine = shared.lock().unwrap_or_else(|error| error.into_inner());
        // Prefer an installed full family; a subset only holds the glyphs the page uses. The
        // Device has the font program's PostScript name, not the PDF `/BaseFont`; a content-keyed
        // alias keeps two documents' subsets of one name apart for the life of the shared engine,
        // and the same face imported twice reuses its registration.
        let family = if engine.has_family(derived) {
            derived.clone()
        } else {
            let alias = font_alias(postscript, *key);
            if engine.has_family(&alias) {
                alias
            } else {
                let names = engine.register_font_data_as(bytes, &alias);
                if names.is_empty() && !engine.has_family(&alias) {
                    return Err(EditableError::Unsupported("a font could not be registered"));
                }
                names.first().cloned().unwrap_or(alias)
            }
        };
        families.insert(*key, family);
    }
    Ok(families)
}

/// Renders the text caches so the page is complete before the engine refreshes it.
fn render_text_caches(layers: &mut [Layer], dpi: f64, fmt: PixelFormat, ctl: &Interrupt, cached: &mut u64) -> Result<(), EditableError> {
    let shared = photocraft_text::shared();
    for layer in layers {
        if ctl.check().is_err() {
            return Err(EditableError::Cancelled);
        }
        if let LayerContent::Text(text) = &mut layer.content
            && has_ink(&text.text)
        {
            // Re-lock per layer so a text-heavy import does not hold the UI's engine for the
            // whole cache-render loop.
            let mut engine = shared.lock().unwrap_or_else(|error| error.into_inner());
            let layout = engine.layout(text, dpi as f32);
            let area = photocraft_text::render::ink_rect(&layout, &text.transform);
            admit_cache(cached, area, fmt)?;
            let rendered = photocraft_text::render::rasterize(&layout, &text.transform, fmt, text.antialias);
            let rect = rendered.rect;
            text.cache = Some(rendered.surface);
            if rect.width() == 0 || rect.height() == 0 {
                return Err(EditableError::Unsupported("a text cache could not be rendered"));
            }
        }
    }
    Ok(())
}

/// Reserves complete tiles before rendering: even one painted pixel allocates a whole tile.
fn admit_cache(cached: &mut u64, area: Rect, fmt: PixelFormat) -> Result<(), EditableError> {
    let bytes = area.tile_count().saturating_mul(TILE_SIZE as u64).saturating_mul(TILE_SIZE as u64).saturating_mul(fmt.bytes_per_pixel() as u64);
    let total = cached.saturating_add(bytes);
    if total > MAX_CACHE_BYTES {
        return Err(EditableError::Unsupported("page cache budget"));
    }
    *cached = total;
    Ok(())
}

/// Puts text layers back at the path index where their glyphs were painted.
fn merge_text_layers(layers: Vec<Layer>, mut texts: Vec<(usize, Layer)>) -> Vec<Layer> {
    texts.sort_by_key(|(order, _)| *order);
    let mut out = Vec::with_capacity(layers.len() + texts.len());
    let mut texts = texts.into_iter().peekable();
    for (index, layer) in layers.into_iter().enumerate() {
        while texts.peek().is_some_and(|(order, _)| *order <= index) {
            if let Some((_, text)) = texts.next() {
                out.push(text);
            }
        }
        out.push(layer);
    }
    out.extend(texts.map(|(_, layer)| layer));
    out
}

/// A content key for an embedded face, used to group its glyphs and to keep aliases unique.
fn font_content_key(data: &OutlineFontData) -> u128 {
    let hash = blake3::hash(data.data.as_ref().as_ref());
    let mut key = [0u8; 16];
    key.copy_from_slice(&hash.as_bytes()[..16]);
    u128::from_le_bytes(key)
}

/// Whether text has a visible character (not only spaces, controls or zero-width marks).
fn has_ink(text: &str) -> bool {
    text.chars().any(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FEFF}'))
}

/// The family alias an embedded face is registered under: its PostScript name plus a content
/// key, so subsets with the same name from different documents cannot replace each other.
fn font_alias(postscript: &str, key: u128) -> String {
    format!("{postscript} {key:016x}")
}

/// Collects filled and stroked paths and glyph runs; anything else marks the page unsupported.
struct PathDevice<'a> {
    /// Page points to document pixels: translation to the artboard plus the per-axis scale.
    doc: Affine,
    /// The page's artboard in document pixels; shape caches and strokes are clipped to it.
    clip: Rect,
    /// Document pixel format the shape caches are rendered in.
    fmt: PixelFormat,
    /// Document DPI, for text-layer caches.
    dpi: f64,
    ctl: Interrupt<'a>,
    layers: Vec<Layer>,
    /// Filled glyphs in content order.
    glyphs: Vec<GlyphRun>,
    glyph_bytes: usize,
    /// Stroked glyphs, paired with a fill at the same place to recognise fake bold.
    strokes: Vec<GlyphRun>,
    /// Embedded fonts met on the page: cache key, PostScript name, derived family and bytes.
    fonts: Vec<(u128, String, String, FontData)>,
    /// Hayro's cheap font identity → our content hash, so bytes are hashed once per face.
    font_keys: HashMap<u128, u128>,
    /// Content keys of fonts already in [`Self::fonts`], so glyph callbacks do not scan it.
    seen_fonts: HashSet<u128>,
    /// Whether a soft mask is active; the first paint under one falls back the page.
    soft_mask: bool,
    /// One entry per clip pushed by the interpreter: `true` for an accepted no-op clip.
    clips: Vec<bool>,
    knots: usize,
    /// Allocated tile bytes of caches retained so far.
    cached: u64,
    unsupported: Option<&'static str>,
    cancelled: bool,
}

/// One drawn glyph with the style the text layer needs.
struct GlyphRun {
    text: String,
    /// Baseline origin in document pixels.
    origin: Point,
    size_pt: f64,
    /// Em size in document pixels.
    size_px: f64,
    /// Baseline direction in radians (view space, y down).
    angle: f64,
    /// Advance in document pixels.
    advance: f64,
    font: FontInfo,
    /// How many layers had been drawn when this glyph was painted; text runs split at a path
    /// between glyphs so paint order is preserved.
    order: usize,
    color: Color,
    /// Drawn both filled and stroked: word-processor fake bold.
    fake_bold: bool,
}

/// The font style a run carries.
#[derive(Clone)]
struct FontInfo {
    /// Hayro's font cache key, to group glyphs of one font.
    key: u128,
    family: String,
    postscript: String,
    weight: u32,
    italic: bool,
}

impl PathDevice<'_> {
    /// Stops collecting at the first thing this step cannot represent.
    fn unsupported(&mut self, reason: &'static str) {
        if self.unsupported.is_none() {
            self.unsupported = Some(reason);
        }
    }

    /// Whether `clip` is an axis-aligned rectangle that contains the whole artboard, so it
    /// changes nothing in document space and can be ignored.
    fn no_op_clip(&self, clip: &ClipPath) -> bool {
        let Some((x0, y0, x1, y1)) = axis_aligned_rect(&clip.path) else { return false };
        let a = self.doc.apply(Point::new(x0, y0));
        let b = self.doc.apply(Point::new(x1, y1));
        let (dx0, dx1) = (a.x.min(b.x), a.x.max(b.x));
        let (dy0, dy1) = (a.y.min(b.y), a.y.max(b.y));
        let board = self.clip;
        dx0 <= f64::from(board.x0) + 0.5 && dy0 <= f64::from(board.y0) + 0.5 && dx1 >= f64::from(board.x1) - 0.5 && dy1 >= f64::from(board.y1) - 0.5
    }

    /// Adds one shape layer unless a page cap is reached. Renders its pixel cache within the
    /// page's cache budget so the layer is visible before the engine refreshes it.
    fn push_shape(&mut self, path: Path, fill: Option<Fill>, stroke: Option<ShapeStroke>) {
        if self.ctl.check().is_err() {
            self.cancelled = true;
            return;
        }
        if path.is_empty() {
            return;
        }
        if stroke.as_ref().is_some_and(|stroke| !stroke.dashes.is_empty()) && path.subpaths.iter().any(is_zero_length) {
            // A dashed zero-length subpath cannot be reproduced as a single point.
            self.unsupported("a dashed zero-length subpath");
            return;
        }
        if self.layers.len() >= MAX_LAYERS {
            self.unsupported("too many objects");
            return;
        }
        let mut shape = ShapeLayer { path, fill, stroke, ..ShapeLayer::default() };
        // Use the renderer's actual ink bounds, including miter tips, and reuse the compilation.
        // Control-point bounds expanded by one stroke width can clip acute joins.
        let compiled = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, self.clip);
        let area = compiled.bounds().unwrap_or(Rect::EMPTY).intersect(&self.clip);
        if area.width() > 0 && area.height() > 0 {
            if let Err(EditableError::Unsupported(reason)) = admit_cache(&mut self.cached, area, self.fmt) {
                self.unsupported(reason);
                return;
            }
            shape.cache = Some(compiled.render(self.fmt, area));
        }
        self.layers.push(Layer::new(format!("Path {}", self.layers.len() + 1), LayerContent::Shape(shape)));
    }
}

impl<'a> Device<'a> for PathDevice<'_> {
    /// A soft mask changes compositing in ways a plain layer cannot express, but it only matters
    /// if something is painted while it is active.
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        self.soft_mask = mask.is_some();
    }

    /// Only normal blending is editable.
    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        if blend_mode != BlendMode::Normal {
            self.unsupported("blend modes");
        }
    }

    /// Turns a filled/stroked path into a shape layer, rejecting paints and strokes it cannot keep.
    fn draw_path(&mut self, path: &BezPath, transform: KurboAffine, paint: &Paint<'a>, draw_mode: &PathDrawMode) {
        if self.unsupported.is_some() {
            return;
        }
        if self.soft_mask {
            self.unsupported("soft masks");
            return;
        }
        if self.ctl.check().is_err() {
            self.cancelled = true;
            return;
        }
        let color = match paint {
            Paint::Color(color) => to_color(color),
            Paint::Pattern(_) => {
                self.unsupported("patterns");
                return;
            }
        };
        let full = self.doc.mul(&knots_affine(&transform));
        let Some(path) = to_path(path, &full, &mut self.knots) else {
            self.unsupported("too many path points");
            return;
        };
        match draw_mode {
            PathDrawMode::Fill(rule) => {
                let mut path = path;
                path.fill_rule = match rule {
                    craft_pdf::interpret::FillRule::NonZero => FillRule::NonZero,
                    craft_pdf::interpret::FillRule::EvenOdd => FillRule::EvenOdd,
                };
                self.push_shape(path, Some(Fill::Solid(color)), None);
            }
            PathDrawMode::Stroke(stroke) => {
                if !uniform_transform(&transform) {
                    self.unsupported("non-uniform stroke transform");
                    return;
                }
                let Some(stroke) = to_stroke(stroke, color, full.determinant().abs().sqrt()) else {
                    self.unsupported("stroke properties");
                    return;
                };
                self.push_shape(path, None, Some(stroke));
            }
        }
    }

    /// Accepts an axis-aligned clip that contains the artboard; other clips fall back.
    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        if self.no_op_clip(clip_path) {
            self.clips.push(true);
            return;
        }
        self.clips.push(false);
        self.unsupported("clipping paths");
    }

    /// Transparency groups are not represented by the layer model.
    fn push_transparency_group(&mut self, _opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: BlendMode) {
        // A group needs its own blend/opacity/mask handling; until that exists the page is
        // rasterised rather than flattened into the wrong stacking order.
        let _ = (mask, blend_mode);
        self.unsupported("transparency groups");
    }

    /// Records a filled glyph as text, or its stroke copy for fake-bold pairing.
    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: KurboAffine, glyph_transform: KurboAffine, paint: &Paint<'a>, draw_mode: &GlyphDrawMode) {
        if self.unsupported.is_some() {
            return;
        }
        if matches!(draw_mode, GlyphDrawMode::Invisible) {
            return;
        }
        if self.soft_mask {
            self.unsupported("soft masks");
            return;
        }
        if self.ctl.check().is_err() {
            self.cancelled = true;
            return;
        }
        if matches!(draw_mode, GlyphDrawMode::Clip) {
            self.unsupported("text used as a clipping path");
            return;
        }
        if self.glyphs.len().saturating_add(self.strokes.len()) >= MAX_GLYPHS {
            self.unsupported("too many glyphs");
            return;
        }
        let text = match glyph.as_unicode() {
            Some(craft_pdf::interpret::hayro_cmap::BfString::Char(c)) => c.to_string(),
            Some(craft_pdf::interpret::hayro_cmap::BfString::String(s)) => s,
            None => {
                // No Unicode means the text layer could not be edited faithfully.
                self.unsupported("text without a Unicode mapping");
                return;
            }
        };
        if text.is_empty() || text.chars().all(|c| c.is_control()) {
            return;
        }
        let Glyph::Outline(outline) = glyph else {
            self.unsupported("Type 3 text");
            return;
        };
        let Some(data) = outline.font_data() else {
            self.unsupported("text without an embedded font");
            return;
        };
        if !data.embedded {
            self.unsupported("text in a substituted font");
            return;
        }
        let Some(postscript) = data.postscript_name.clone().filter(|name| !name.is_empty()) else {
            self.unsupported("text without a font name");
            return;
        };
        let color = match paint {
            Paint::Color(color) => to_color(color),
            Paint::Pattern(_) => {
                self.unsupported("text with a pattern paint");
                return;
            }
        };
        let t = transform * glyph_transform;
        let [a, b, c, d, e, f] = t.as_coeffs();
        // A glyph space of 1000 units per em, so the determinant's square root is 1/1000 of the
        // em size. Upright text has a positive a and a negative d (the view flips y).
        let size_pt = (a * d - b * c).abs().sqrt() * 1000.0;
        if !size_pt.is_finite() || size_pt <= 0.0 {
            self.unsupported("text without a usable size");
            return;
        }
        // Skewed, mirrored or sheared text cannot be a point text layer; leave the page to raster.
        // After the view's y flip an upright or rotated run has a ≈ -d and b ≈ c.
        let tolerance = 0.01 * (a * d - b * c).abs().sqrt();
        if (a + d).abs() > tolerance || (b - c).abs() > tolerance {
            self.unsupported("skewed or mirrored text");
            return;
        }
        if let GlyphDrawMode::Stroke(stroke) = draw_mode {
            // Only a thin, solid, same-colour outline can be approximated by faux bold.
            // Colour equality is checked against the fill in pair_fake_bold.
            if !stroke.line_width.is_finite()
                || stroke.line_width < 0.0
                || f64::from(stroke.line_width) * transform.determinant().abs().sqrt() > size_pt * 0.05
                || !stroke.dash_array.is_empty()
            {
                self.unsupported("stroked text geometry");
                return;
            }
        }
        let origin = self.doc.apply(Point::new(e, f));
        let size_px = size_pt * self.doc.determinant().abs().sqrt();
        let angle = b.atan2(a);
        if !angle.is_finite() || !size_px.is_finite() || size_px <= 0.0 {
            self.unsupported("text without a usable transform");
            return;
        }
        let advance = f64::from(outline.advance_width().unwrap_or(500.0)) / 1000.0 * size_px;
        let weight = font_weight(&data, &postscript);
        let italic = data.is_italic || name_has(&postscript, &["italic", "oblique"]);
        let identity = outline.font_cache_key();
        let key = *self.font_keys.entry(identity).or_insert_with(|| font_content_key(&data));
        let family = family_from_postscript(&postscript);
        if self.seen_fonts.insert(key) {
            self.fonts.push((key, postscript.clone(), family.clone(), data.data.clone()));
        }
        let bytes = std::mem::size_of::<GlyphRun>().saturating_add(text.len()).saturating_add(family.len()).saturating_add(postscript.len());
        self.glyph_bytes = self.glyph_bytes.saturating_add(bytes);
        if self.glyph_bytes > MAX_GLYPH_BYTES {
            self.unsupported("glyph memory budget");
            return;
        }
        let font = FontInfo { key, family, postscript, weight, italic };
        let glyph = GlyphRun { text, origin, size_pt, size_px, angle, advance, font, color, fake_bold: false, order: self.layers.len() };
        if matches!(draw_mode, GlyphDrawMode::Stroke(_)) {
            self.strokes.push(glyph);
        } else {
            self.glyphs.push(glyph);
        }
    }

    /// Images are not editable at this step.
    fn draw_image(&mut self, _image: Image<'a, '_>, _transform: KurboAffine) {
        self.unsupported("images");
    }

    /// Balances [`Self::push_clip_path`].
    fn pop_clip_path(&mut self) {
        let _ = self.clips.pop();
    }

    /// Balances [`Self::push_transparency_group`].
    fn pop_transparency_group(&mut self) {}

    /// Annotations are not part of the page's editable paint.
    fn begin_annotations(&mut self) {
        self.unsupported("annotations");
    }
}

/// The weight a font is given to a text layer, from its metadata and name.
fn font_weight(data: &OutlineFontData, postscript: &str) -> u32 {
    if name_has(postscript, &["bold", "semibold", "demibold", "extrabold", "ultrabold", "black", "heavy"]) {
        return data.weight.unwrap_or(700).max(700);
    }
    data.weight.unwrap_or(400)
}

/// Whether `name` contains any of the lowercase style words.
fn name_has(name: &str, words: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    words.iter().any(|word| lower.contains(word))
}

/// A family name from a `/BaseFont`/PostScript name, per the rules PdfCraft and pdfcer use:
/// drop a six-letter subset tag, a trailing style suffix and `MT`/`PS`, then space camel case.
/// The raw name stays in `CharStyle::postscript_name`, because some systems match that spelling.
fn family_from_postscript(name: &str) -> String {
    let mut s = name;
    if s.len() > 7 && s.as_bytes()[6] == b'+' && s[..6].bytes().all(|b| b.is_ascii_uppercase()) {
        s = &s[7..];
    }
    if let Some((head, tail)) = s.rsplit_once('-')
        && is_style_word(tail)
    {
        s = head;
    }
    loop {
        if let Some(rest) = s.strip_suffix("PSMT") {
            s = rest;
        } else if let Some(rest) = s.strip_suffix("MT") {
            s = rest;
        } else if let Some(rest) = s.strip_suffix("PS") {
            s = rest;
        } else {
            break;
        }
    }
    let spaced = space_camel(s);
    if spaced.is_empty() { name.to_string() } else { spaced }
}

/// Whether a suffix is a style word (after dropping `MT`/`PS` from it).
fn is_style_word(word: &str) -> bool {
    let mut w = word;
    loop {
        if let Some(rest) = w.strip_suffix("MT") {
            w = rest;
        } else if let Some(rest) = w.strip_suffix("PS") {
            w = rest;
        } else {
            break;
        }
    }
    matches!(
        w.to_ascii_lowercase().as_str(),
        "bold"
            | "italic"
            | "oblique"
            | "bolditalic"
            | "boldoblique"
            | "regular"
            | "medium"
            | "light"
            | "black"
            | "semibold"
            | "demibold"
            | "heavy"
            | "thin"
            | "book"
            | "roman"
            | "condensed"
            | "narrow"
            | "extended"
    )
}

/// `TimesNewRoman` → `Times New Roman`; acronym runs and hyphens stay as they are.
fn space_camel(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in chars.iter().copied().enumerate() {
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let next = chars.get(i + 1).copied();
        let boundary = ch.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
            || ch.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_uppercase()) && next.is_some_and(|n| n.is_ascii_lowercase());
        if boundary && !out.is_empty() && !out.ends_with(' ') && !out.ends_with('-') {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// The bounds of a path that is one axis-aligned rectangle, else `None`.
fn axis_aligned_rect(path: &BezPath) -> Option<(f64, f64, f64, f64)> {
    let elements = path.elements();
    if !(5..=6).contains(&elements.len()) {
        return None;
    }
    let mut points = Vec::with_capacity(4);
    for element in elements {
        match *element {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => points.push((p.x, p.y)),
            PathEl::ClosePath => {}
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => return None,
        }
    }
    if points.len() != 4 {
        // An explicitly closed rectangle uses four LineTo points plus the duplicate start.
        if points.len() == 5 && (points[0].0 - points[4].0).abs() < 1.0e-6 && (points[0].1 - points[4].1).abs() < 1.0e-6 {
            points.pop();
        } else {
            return None;
        }
    }
    let x0 = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let x1 = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let y0 = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let y1 = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let on_edge = |v: f64, lo: f64, hi: f64| (v - lo).abs() < 1.0e-6 || (v - hi).abs() < 1.0e-6;
    if points.iter().any(|(x, y)| !on_edge(*x, x0, x1) || !on_edge(*y, y0, y1)) {
        return None;
    }
    // A bow-tie traversal also visits the four corners, so require its shoelace area to match
    // the bounding box (a real rectangle covers it exactly).
    let mut twice_area = 0.0;
    for i in 0..4 {
        let (ax, ay) = points[i];
        let (bx, by) = points[(i + 1) % 4];
        twice_area += ax * by - bx * ay;
    }
    let box_area = (x1 - x0) * (y1 - y0);
    if (twice_area.abs() * 0.5 - box_area).abs() > box_area * 1.0e-3 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

/// Whether every knot of a subpath shares the first anchor (a zero-length subpath).
fn is_zero_length(subpath: &Subpath) -> bool {
    subpath.knots.first().is_some_and(|first| subpath.knots.iter().all(|knot| knot.anchor.x == first.anchor.x && knot.anchor.y == first.anchor.y))
}

/// Whether a path transform keeps circles circular: both axes the same length and perpendicular.
fn uniform_transform(transform: &KurboAffine) -> bool {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let x = (a * a + b * b).sqrt();
    let y = (c * c + d * d).sqrt();
    if !x.is_finite() || !y.is_finite() || x <= 0.0 || y <= 0.0 {
        return false;
    }
    let same = (x - y).abs() <= 0.01 * x.max(y);
    let square = (a * c + b * d).abs() <= 0.01 * x * y;
    same && square
}

/// A PDF colour as a straight-alpha PhotoCraft colour.
fn to_color(color: &HayroColor) -> Color {
    let rgba = color.to_rgba();
    // `AlphaColor` components are straight (not premultiplied), despite the raster path being
    // premultiplied: `Color::to_rgba8` reads them as straight too.
    let [r, g, b, a] = rgba.components();
    Color::rgba(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0))
}

/// A kurbo affine as PhotoCraft's affine (same `[a, b, c, d, e, f]` layout).
fn knots_affine(transform: &KurboAffine) -> Affine {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    Affine { m: [a, b, c, d, e, f] }
}

/// A Hayro stroke as PhotoCraft's stroke: width in pixels, dashes in multiples of the width.
fn to_stroke(stroke: &craft_pdf::interpret::StrokeProps, color: Color, scale: f64) -> Option<ShapeStroke> {
    if !stroke.line_width.is_finite() || stroke.line_width < 0.0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    // A zero line width is the thinnest line the device can draw (ISO 32000-2 §8.4.3.2); the
    // raster path draws it one document pixel wide, so do the same and use that as the dash base.
    let width = if stroke.line_width > 0.0 { stroke.line_width * scale as f32 } else { 1.0 };
    if !width.is_finite() || width <= 0.0 {
        return None;
    }
    let mut dashes: Vec<f32> = stroke.dash_array.iter().map(|dash| dash * scale as f32 / width).collect();
    // An odd PDF dash array repeats, so [3] means 3 on, 3 off rather than a single entry.
    if dashes.len() % 2 == 1 {
        let copy = dashes.clone();
        dashes.extend(copy);
    }
    Some(ShapeStroke {
        width,
        paint: Fill::Solid(color),
        opacity: 1.0,
        align: StrokeAlign::Center,
        cap: match stroke.line_cap {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        },
        join: match stroke.line_join {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        },
        miter_limit: stroke.miter_limit,
        dashes,
        dash_offset: stroke.dash_offset * scale as f32 / width,
    })
}

/// Converts a kurbo path to PhotoCraft's cubic knot model. `None` once `knots` passes the cap.
fn to_path(bez: &BezPath, transform: &Affine, knots: &mut usize) -> Option<Path> {
    let mut subpaths = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut restarted = false;
    let mut last = Point::new(0.0, 0.0);
    for element in bez.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                if let Some(subpath) = current.take()
                    && (!restarted || subpath.knots.len() > 1)
                {
                    subpaths.push(subpath);
                }
                restarted = false;
                current = Some(Subpath { closed: false, knots: vec![Knot::corner(p.x, p.y)], op: PathOp::Combine });
                last = Point::new(p.x, p.y);
            }
            PathEl::LineTo(p) => {
                if let Some(subpath) = current.as_mut() {
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = knot.anchor;
                    }
                    subpath.knots.push(Knot::corner(p.x, p.y));
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::QuadTo(control, p) => {
                if let Some(subpath) = current.as_mut() {
                    let start = subpath.knots.last().map_or(last, |knot| knot.anchor);
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = Point::new(start.x + 2.0 / 3.0 * (control.x - start.x), start.y + 2.0 / 3.0 * (control.y - start.y));
                    }
                    subpath.knots.push(Knot {
                        anchor: Point::new(p.x, p.y),
                        in_ctrl: Point::new(p.x + 2.0 / 3.0 * (control.x - p.x), p.y + 2.0 / 3.0 * (control.y - p.y)),
                        out_ctrl: Point::new(p.x, p.y),
                        smooth: false,
                    });
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(subpath) = current.as_mut() {
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = Point::new(c1.x, c1.y);
                    }
                    subpath.knots.push(Knot { anchor: Point::new(p.x, p.y), in_ctrl: Point::new(c2.x, c2.y), out_ctrl: Point::new(p.x, p.y), smooth: false });
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::ClosePath => {
                if let Some(mut subpath) = current.take() {
                    let restart = subpath.knots.first().map(|knot| knot.anchor);
                    subpath.closed = true;
                    if subpath.knots.len() > 1 {
                        let first = subpath.knots[0].anchor;
                        let last_anchor = subpath.knots.last().map_or(first, |knot| knot.anchor);
                        if (first.x - last_anchor.x).abs() < 1e-9 && (first.y - last_anchor.y).abs() < 1e-9 {
                            // A straight closing segment can merge into the first knot; a curve
                            // that returns to its start must keep both knots to preserve the loop.
                            let straight = subpath.knots.last().is_some_and(|knot| {
                                knot.in_ctrl.x == knot.anchor.x
                                    && knot.in_ctrl.y == knot.anchor.y
                                    && knot.out_ctrl.x == knot.anchor.x
                                    && knot.out_ctrl.y == knot.anchor.y
                            });
                            if straight && let Some(knot) = subpath.knots.pop() {
                                subpath.knots[0].in_ctrl = knot.in_ctrl;
                            }
                        } else {
                            subpath.knots[0].in_ctrl = first;
                        }
                    }
                    // A single-point closed subpath still paints a dot with a round cap.
                    if !restarted || subpath.knots.len() > 1 {
                        subpaths.push(subpath);
                    }
                    // A close starts a new subpath at the closed subpath's start point; a
                    // following line/curve continues from there (PDF 32000-1 §8.5.2.1).
                    if let Some(start) = restart {
                        current = Some(Subpath { closed: false, knots: vec![Knot::corner(start.x, start.y)], op: PathOp::Combine });
                        last = start;
                        restarted = true;
                    } else {
                        restarted = false;
                    }
                }
            }
        }
    }
    if let Some(subpath) = current
        && (!restarted || subpath.knots.len() > 1)
    {
        subpaths.push(subpath);
    }
    *knots = knots.saturating_add(subpaths.iter().map(|subpath| subpath.knots.len()).sum::<usize>());
    if *knots > MAX_KNOTS {
        return None;
    }
    for subpath in &mut subpaths {
        for knot in &mut subpath.knots {
            *knot = knot.transform(transform);
        }
    }
    Some(Path::new(subpaths))
}

#[cfg(test)]
mod tests {
    use lopdf::{Document as PdfDocument, Object, Stream, dictionary};

    use super::*;
    use crate::{ImportOptions, IoError, import_with_options_and_interrupt};

    /// The embedded test font.
    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

    /// A one-page 200 x 100 pt PDF whose content stream is `content`.
    fn pdf(content: &str) -> Vec<u8> {
        pdf_with_resources(content, dictionary! {})
    }

    /// [`pdf`] with page resources (for example an ExtGState).
    fn pdf_with_resources(content: &str, resources: lopdf::Dictionary) -> Vec<u8> {
        let mut pdf = PdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(200.0), Object::Real(100.0)]),
            "Contents" => content_id,
            "Resources" => resources,
        });
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Object::Array(vec![Object::Reference(page_id)]),
                "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).expect("writing the fixture");
        bytes
    }

    /// A one-page 200 x 100 pt PDF whose content stream is `content` and whose page uses an
    /// embedded TrueType font as `/F1`.
    fn text_pdf(content: &str) -> Vec<u8> {
        text_pdf_named(content, "Inter-Regular")
    }

    /// [`text_pdf`] with the font named `name` (the same Inter bytes, so tests can exercise the
    /// name-based style rules).
    fn text_pdf_named(content: &str, name: &str) -> Vec<u8> {
        text_pdf_bytes(content, name, INTER)
    }

    /// [`text_pdf`] with the font named `name` and the given font bytes.
    fn text_pdf_bytes(content: &str, name: &str, font_bytes: &[u8]) -> Vec<u8> {
        let mut pdf = PdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let font_file = pdf.add_object(Stream::new(dictionary! { "Length1" => font_bytes.len() as i64 }, font_bytes.to_vec()));
        let descriptor = pdf.add_object(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => Object::Name(name.as_bytes().to_vec()),
            "Flags" => 32,
            "FontBBox" => Object::Array(vec![Object::Integer(-100), Object::Integer(-200), Object::Integer(1000), Object::Integer(900)]),
            "ItalicAngle" => 0,
            "Ascent" => 900,
            "Descent" => -200,
            "CapHeight" => 700,
            "StemV" => 80,
            "FontFile2" => font_file,
        });
        let font = pdf.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "TrueType",
            "BaseFont" => Object::Name(name.as_bytes().to_vec()),
            "FirstChar" => 32,
            "LastChar" => 126,
            "Widths" => Object::Array((32..=126u8).map(|c| {
                let Ok(face) = ttf_parser::Face::parse(font_bytes, 0) else { return Object::Integer(600); };
                let width = face.glyph_index(char::from(c)).and_then(|id| face.glyph_hor_advance(id)).unwrap_or(0);
                Object::Real(f32::from(width) * 1000.0 / f32::from(face.units_per_em()))
            }).collect()),
            "FontDescriptor" => descriptor,
            "Encoding" => "WinAnsiEncoding",
        });
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(200.0), Object::Real(100.0)]),
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Object::Array(vec![Object::Reference(page_id)]),
                "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).expect("writing the text fixture");
        bytes
    }

    /// The artboard group's children.
    fn children(document: &photocraft_doc::Document) -> &[Layer] {
        match &document.layers.first().expect("a page group").content {
            LayerContent::Group(group) => &group.children,
            _ => panic!("the page is not a group"),
        }
    }

    /// The first text layer of `document`.
    fn text_layer(document: &photocraft_doc::Document) -> &TextLayer {
        children(document)
            .iter()
            .find_map(|layer| match &layer.content {
                LayerContent::Text(text) => Some(text),
                _ => None,
            })
            .expect("a text layer")
    }

    /// A solid fill or stroke colour.
    fn solid(fill: &Fill) -> Color {
        match fill {
            Fill::Solid(color) => *color,
            _ => panic!("not a solid paint"),
        }
    }

    /// Path bounds in document pixels.
    fn bounds(path: &Path) -> (f64, f64, f64, f64) {
        let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for knot in path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
            b.0 = b.0.min(knot.anchor.x);
            b.1 = b.1.min(knot.anchor.y);
            b.2 = b.2.max(knot.anchor.x);
            b.3 = b.3.max(knot.anchor.y);
        }
        b
    }

    #[test]
    fn painted_paths_become_shape_layers_with_their_colours_and_dashes() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f 0 0 1 RG 3 w [9 4] 0 d 100 30 m 180 30 l S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("paths.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable import");
        let children = children(&imported.document);
        assert_eq!(children.len(), 2, "one fill and one stroke layer");
        let LayerContent::Shape(fill) = &children[0].content else { panic!("the first child is not a shape") };
        assert_eq!(solid(fill.fill.as_ref().expect("a fill")), Color::rgb(1.0, 0.0, 0.0));
        assert!(fill.stroke.is_none());
        let (x0, y0, x1, y1) = bounds(&fill.path);
        assert!((x0 - 20.0).abs() < 0.01 && (x1 - 80.0).abs() < 0.01, "x bounds {x0}..{x1}");
        assert!((y0 - 40.0).abs() < 0.01 && (y1 - 80.0).abs() < 0.01, "PDF y up becomes document y down: {y0}..{y1}");

        let LayerContent::Shape(stroke) = &children[1].content else { panic!("the second child is not a shape") };
        assert!(stroke.fill.is_none());
        let stroke = stroke.stroke.as_ref().expect("a stroke");
        assert_eq!(solid(&stroke.paint), Color::rgb(0.0, 0.0, 1.0));
        assert!((stroke.width - 3.0).abs() < 0.01, "stroke width {}", stroke.width);
        assert_eq!(stroke.dashes.len(), 2);
        assert!((stroke.dashes[0] - 3.0).abs() < 0.01 && (stroke.dashes[1] - 4.0 / 3.0).abs() < 0.01, "dashes {:?}", stroke.dashes);
    }

    #[test]
    fn the_editable_composite_matches_the_raster_page() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f 0 0 1 RG 3 w [9 4] 0 d 100 30 m 180 30 l S");
        let raster = import_with_options_and_interrupt("paths.pdf", &bytes, &ImportOptions { pdf_dpi: 72.0, ..ImportOptions::default() }, &Interrupt::NONE)
            .expect("a raster import");
        let editable = import_with_options_and_interrupt(
            "paths.pdf",
            &bytes,
            &ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() },
            &Interrupt::NONE,
        )
        .expect("an editable import");
        let fmt = editable.document.pixel_format();
        let a = photocraft_compose::flatten_to_surface(&editable.document, fmt, None);
        let b = photocraft_compose::flatten_to_surface(&raster.document, fmt, None);
        let rect = editable.document.bounds();
        let av = a.read_region(rect);
        let bv = b.read_region(rect);
        assert_eq!(av.len(), bv.len());
        let mut sum = 0.0f64;
        let mut max = 0.0f32;
        for (x, y) in av.iter().zip(&bv) {
            let d = (x - y).abs();
            sum += f64::from(d);
            max = max.max(d);
        }
        let mean = sum / av.len() as f64;
        assert!(mean < 0.01, "mean pixel difference {mean:.4} (max {max:.3})");
    }

    #[test]
    fn text_falls_back_to_the_raster_page_with_a_warning() {
        let bytes = pdf("BT /F1 24 Tf 10 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("a fallback import");
        assert!(
            imported.warnings.iter().any(|warning| warning.contains("text without an embedded font") && warning.contains("rasterised")),
            "{:?}",
            imported.warnings
        );
        assert!(matches!(children(&imported.document).first().map(|layer| &layer.content), Some(LayerContent::Raster(_))));
    }

    #[test]
    fn embedded_text_becomes_a_text_layer_with_its_style() {
        let bytes = text_pdf("BT /F1 24 Tf 1 0 0 rg 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable text import");
        let text = text_layer(&imported.document);
        assert_eq!(text.text, "Hi");
        assert!(text.font_family.starts_with("Inter"), "family {}", text.font_family);
        assert!((f64::from(text.size_pt) - 24.0).abs() < 0.1, "size {}", text.size_pt);
        assert_eq!(text.color, Color::rgb(1.0, 0.0, 0.0));
        let origin = text.transform.apply(Point::new(0.0, 0.0));
        assert!((origin.x - 20.0).abs() < 0.5 && (origin.y - 50.0).abs() < 0.5, "origin {origin:?}");
        assert!(text.cache.is_some(), "the text cache is rendered on import");
        let style = &text.runs[0].style;
        assert_eq!(style.postscript_name.as_deref(), Some("Inter-Regular"));
        assert_eq!(style.weight, 400);
        assert!(!style.italic);
        assert!(imported.warnings.iter().any(|warning| warning.contains("fonts embedded in the PDF")), "{:?}", imported.warnings);
    }

    #[test]
    fn imported_text_relayouts_with_the_shared_engine() {
        let bytes = text_pdf("BT /F1 24 Tf 1 0 0 rg 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable text import");
        let mut text = text_layer(&imported.document).clone();
        text.cache = None;
        let shared = photocraft_text::shared();
        let mut engine = shared.lock().unwrap_or_else(|error| error.into_inner());
        assert!(engine.has_family(&text.font_family), "the app engine must know the imported family");
        let rect = engine.render_layer(&mut text, 72.0, PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        assert!(rect.width() > 0 && rect.height() > 0, "the layer re-lays out");
        assert!(text.cache.is_some());
    }

    #[test]
    fn rotated_text_stays_editable() {
        let bytes = text_pdf("BT /F1 24 Tf 0 1 -1 0 100 20 Tm (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("rotated.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(!imported.warnings.iter().any(|warning| warning.contains("skewed or mirrored")), "{:?}", imported.warnings);
        assert_eq!(text_layer(&imported.document).text, "Hi");
    }

    #[test]
    fn font_aliases_include_the_content_key() {
        assert_ne!(font_alias("Inter-Regular", 1), font_alias("Inter-Regular", 2));
        assert!(font_alias("Inter-Regular", 0x12ab).contains("Inter-Regular"));
    }

    #[test]
    fn two_documents_with_one_font_name_keep_distinct_aliases() {
        // Rename the bundled Inter family to one that is not installed, so the embedded face
        // takes the alias path; same PostScript name and family, different bytes (a trailing byte
        // changes the content key), as two different subsets of one font would.
        let mut base = INTER.to_vec();
        for at in 0..base.len().saturating_sub(5) {
            if &base[at..at + 5] == b"Inter" {
                base[at..at + 5].copy_from_slice(b"Xntr");
            }
        }
        let mut other = base.clone();
        other.push(0);
        let a = text_pdf_bytes("BT /F1 24 Tf 20 50 Td (Hi) Tj ET", "Xntr-Regular", &base);
        let b = text_pdf_bytes("BT /F1 24 Tf 20 50 Td (Hi) Tj ET", "Xntr-Regular", &other);
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let first = import_with_options_and_interrupt("a.pdf", &a, &options, &Interrupt::NONE).expect("first import");
        let second = import_with_options_and_interrupt("b.pdf", &b, &options, &Interrupt::NONE).expect("second import");
        let fa = text_layer(&first.document).font_family.clone();
        let fb = text_layer(&second.document).font_family.clone();
        assert_ne!(fa, fb, "the two embedded faces must not share an alias");
        let shared = photocraft_text::shared();
        let mut engine = shared.lock().unwrap_or_else(|error| error.into_inner());
        assert!(engine.has_family(&fa) && engine.has_family(&fb));
    }

    #[test]
    fn mirrored_text_falls_back() {
        let bytes = text_pdf("BT /F1 24 Tf -1 0 0 1 100 50 Tm (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("mirror.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(
            imported.warnings.iter().any(|warning| warning.contains("skewed or mirrored text") && warning.contains("rasterised")),
            "{:?}",
            imported.warnings
        );
    }

    #[test]
    fn font_names_set_bold_and_italic() {
        for (name, weight, italic) in [("Inter-Bold", 700, false), ("Inter-Italic", 400, true), ("Inter-BoldItalic", 700, true)] {
            let bytes = text_pdf_named("BT /F1 24 Tf 20 50 Td (Hi) Tj ET", name);
            let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
            let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable text import");
            let style = &text_layer(&imported.document).runs[0].style;
            assert_eq!(style.weight, weight, "{name}");
            assert_eq!(style.italic, italic, "{name}");
            assert!(style.font_family.starts_with("Inter"), "{name}: {}", style.font_family);
        }
    }

    #[test]
    fn a_page_sized_clip_does_not_force_a_raster_page() {
        let bytes = pdf("q 0 0 200 100 re W n 1 0 0 rg 20 20 60 40 re f Q");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("clip.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(!imported.warnings.iter().any(|warning| warning.contains("clipping")), "{:?}", imported.warnings);
        assert!(children(&imported.document).iter().any(|layer| matches!(layer.content, LayerContent::Shape(_))));
    }

    #[test]
    fn a_zero_width_stroke_is_a_one_pixel_hairline() {
        let bytes = pdf("0 0 1 RG 0 w 20 30 m 180 30 l S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("hairline.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        let LayerContent::Shape(shape) = &children(&imported.document)[0].content else { panic!("not a shape") };
        let stroke = shape.stroke.as_ref().expect("a stroke");
        assert!((stroke.width - 1.0).abs() < 0.01, "width {}", stroke.width);
    }

    #[test]
    fn a_non_uniform_stroke_transform_falls_back() {
        let bytes = pdf("q 2 0 0 1 0 0 cm 0 0 1 RG 4 w 20 30 m 80 30 l S Q");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("skew.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(
            imported.warnings.iter().any(|warning| warning.contains("non-uniform stroke transform") && warning.contains("rasterised")),
            "{:?}",
            imported.warnings
        );
        assert!(matches!(children(&imported.document).first().map(|layer| &layer.content), Some(LayerContent::Raster(_))));
    }

    #[test]
    fn a_uniform_stroke_transform_scales_the_width() {
        let bytes = pdf("q 2 0 0 2 0 0 cm 0 0 1 RG 4 w 20 30 m 80 30 l S Q");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("scaled.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        let LayerContent::Shape(shape) = &children(&imported.document)[0].content else { panic!("not a shape") };
        let stroke = shape.stroke.as_ref().expect("a stroke");
        assert!((stroke.width - 8.0).abs() < 0.01, "width {}", stroke.width);
    }

    #[test]
    fn extgstate_fill_and_stroke_opacity_reach_the_layers() {
        let resources = dictionary! {
            "ExtGState" => dictionary! {
                "GS0" => dictionary! { "ca" => 0.5, "CA" => 0.5 },
            },
        };
        let bytes = pdf_with_resources("/GS0 gs 0.2 0.4 0.6 rg 20 20 60 40 re f 0 0 1 RG 4 w 20 80 m 180 80 l S", resources);
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("opacity.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        let layers = children(&imported.document);
        let LayerContent::Shape(fill) = &layers[0].content else { panic!("not a shape") };
        let fill = solid(fill.fill.as_ref().expect("a fill"));
        assert!((fill.alpha - 0.5).abs() < 0.01);
        assert!((fill.c[0] - 0.2).abs() < 0.01 && (fill.c[1] - 0.4).abs() < 0.01 && (fill.c[2] - 0.6).abs() < 0.01, "{fill:?}");
        let LayerContent::Shape(stroke) = &layers[1].content else { panic!("not a shape") };
        assert!((solid(&stroke.stroke.as_ref().expect("a stroke").paint).alpha - 0.5).abs() < 0.01);
    }

    #[test]
    fn odd_and_zero_dash_arrays_are_normalised() {
        let mut stroke = craft_pdf::interpret::StrokeProps {
            line_width: 2.0,
            line_cap: Cap::Butt,
            line_join: Join::Miter,
            miter_limit: 10.0,
            dash_array: Default::default(),
            dash_offset: 0.0,
        };
        stroke.dash_array.push(3.0);
        let out = to_stroke(&stroke, Color::BLACK, 1.0).expect("a stroke");
        assert_eq!(out.dashes, vec![1.5, 1.5], "an odd array repeats");
        stroke.dash_array.clear();
        stroke.dash_array.push(0.0);
        stroke.dash_array.push(4.0);
        let out = to_stroke(&stroke, Color::BLACK, 1.0).expect("a stroke");
        assert_eq!(out.dashes, vec![0.0, 2.0], "zero-length dashes stay");
    }

    #[test]
    fn fill_then_stroke_text_is_fake_bold_not_a_second_run() {
        let bytes = text_pdf("BT /F1 24 Tf 2 Tr 1 0 0 rg 1 0 0 RG 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable text import");
        let texts: Vec<_> = children(&imported.document)
            .iter()
            .filter_map(|layer| match &layer.content {
                LayerContent::Text(text) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 1, "one run, not a duplicate stroke run");
        assert_eq!(texts[0].text, "Hi");
        assert_eq!(texts[0].runs[0].style.weight, 700);
        assert_eq!(texts[0].color, Color::rgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn font_family_rules_follow_the_base_font() {
        assert_eq!(family_from_postscript("ABCDEF+TimesNewRomanPSMT"), "Times New Roman");
        assert_eq!(family_from_postscript("Arial-BoldMT"), "Arial");
        assert_eq!(family_from_postscript("Helvetica-Oblique"), "Helvetica");
        assert_eq!(family_from_postscript("MS-Gothic"), "MS-Gothic");
        assert_eq!(family_from_postscript("SimSun"), "Sim Sun");
        assert_eq!(family_from_postscript("Inter-Regular"), "Inter");
        assert_eq!(family_from_postscript("Roboto-Black"), "Roboto");
    }

    #[test]
    fn the_editable_text_composite_matches_the_raster_page() {
        let bytes = text_pdf("BT /F1 24 Tf 1 0 0 rg 20 50 Td (Hi) Tj ET");
        let raster = import_with_options_and_interrupt("text.pdf", &bytes, &ImportOptions { pdf_dpi: 72.0, ..ImportOptions::default() }, &Interrupt::NONE)
            .expect("a raster import");
        let editable = import_with_options_and_interrupt(
            "text.pdf",
            &bytes,
            &ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() },
            &Interrupt::NONE,
        )
        .expect("an editable import");
        let fmt = editable.document.pixel_format();
        let a = photocraft_compose::flatten_to_surface(&editable.document, fmt, None);
        let b = photocraft_compose::flatten_to_surface(&raster.document, fmt, None);
        let rect = editable.document.bounds();
        let av = a.read_region(rect);
        let bv = b.read_region(rect);
        let mut sum = 0.0f64;
        let mut max = 0.0f32;
        for (x, y) in av.iter().zip(&bv) {
            let d = (x - y).abs();
            sum += f64::from(d);
            max = max.max(d);
        }
        let mean = sum / av.len() as f64;
        assert!(mean < 0.02, "mean pixel difference {mean:.4} (max {max:.3})");
        // The cache must actually contain text: a blank layer can still pass a mean comparison.
        let channels = fmt.channels();
        let width = rect.width() as usize;
        let mut red = 0;
        for y in 30..60usize {
            for x in 20..45usize {
                let at = (y * width + x) * channels;
                if av[at] > 0.4 && av[at + 1] < 0.4 && av[at + 2] < 0.4 {
                    red += 1;
                }
            }
        }
        assert!(red > 0, "the editable text cache is blank");
    }

    #[test]
    fn text_keeps_paint_order_on_both_sides_of_a_path() {
        let bytes = text_pdf("BT /F1 24 Tf 20 50 Td (A) Tj ET 0 0 1 rg 0 0 200 100 re f BT /F1 24 Tf 20 20 Td (B) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("order.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        let layers = children(&imported.document);
        assert_eq!(layers.len(), 3);
        let text = |layer: &Layer| match &layer.content {
            LayerContent::Text(text) => text.text.clone(),
            _ => String::new(),
        };
        assert_eq!(text(&layers[0]), "A");
        assert!(matches!(layers[1].content, LayerContent::Shape(_)), "the path stays between the runs");
        assert_eq!(text(&layers[2]), "B");
    }

    #[test]
    fn fake_bold_does_not_pair_across_a_path() {
        let bytes = text_pdf("BT /F1 24 Tf 20 50 Td (Hi) Tj ET 0 0 1 rg 0 0 200 100 re f BT /F1 24 Tf 1 Tr 1 w 0 0 1 RG 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("pair.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(imported.warnings.iter().any(|warning| warning.contains("stroked text") && warning.contains("rasterised")), "{:?}", imported.warnings);
    }

    #[test]
    fn a_curve_returning_to_its_start_keeps_both_knots() {
        let mut bez = BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.curve_to((10.0, 0.0), (10.0, 10.0), (0.0, 0.0));
        bez.close_path();
        let mut knots = 0;
        let path = to_path(&bez, &Affine::IDENTITY, &mut knots).expect("a path");
        assert_eq!(path.subpaths.len(), 1);
        assert!(path.subpaths[0].knots.len() >= 2);
    }

    #[test]
    fn an_explicitly_closed_rectangle_is_a_no_op_clip() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((200.0, 0.0));
        path.line_to((200.0, 100.0));
        path.line_to((0.0, 100.0));
        path.line_to((0.0, 0.0));
        path.close_path();
        assert_eq!(axis_aligned_rect(&path), Some((0.0, 0.0, 200.0, 100.0)));
    }

    #[test]
    fn a_dashed_zero_length_line_falls_back() {
        let bytes = pdf("[1 1] 0 d 10 10 m 20 20 l 30 30 m h S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("dash.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        assert!(
            imported.warnings.iter().any(|warning| warning.contains("dashed zero-length subpath") && warning.contains("rasterised")),
            "{:?}",
            imported.warnings
        );
    }

    #[test]
    fn a_solid_zero_length_line_stays_editable() {
        let bytes = pdf("10 10 m 20 20 l 30 30 m h S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("solid.pdf", &bytes, &options, &Interrupt::NONE).expect("import");
        let layers = children(&imported.document);
        assert!(layers.iter().any(|layer| matches!(layer.content, LayerContent::Shape(_))), "{:?}", imported.warnings);
    }

    #[test]
    fn a_closed_single_point_subpath_is_retained() {
        let mut bez = BezPath::new();
        bez.move_to((10.0, 10.0));
        bez.close_path();
        let mut knots = 0;
        let path = to_path(&bez, &Affine::IDENTITY, &mut knots).expect("a path");
        assert_eq!(path.subpaths.len(), 1);
        assert_eq!(path.subpaths[0].knots.len(), 1);
        assert!(path.subpaths[0].closed);
    }

    #[test]
    fn a_bow_tie_path_is_not_a_no_op_clip() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 10.0));
        path.line_to((10.0, 0.0));
        path.line_to((0.0, 10.0));
        path.close_path();
        assert!(axis_aligned_rect(&path).is_none());
    }

    #[test]
    fn a_line_after_close_starts_a_new_subpath() {
        let mut bez = BezPath::new();
        bez.move_to((0.0, 0.0));
        bez.line_to((10.0, 0.0));
        bez.line_to((10.0, 10.0));
        bez.close_path();
        bez.line_to((20.0, 0.0));
        let mut knots = 0;
        let path = to_path(&bez, &Affine::IDENTITY, &mut knots).expect("a path");
        assert_eq!(path.subpaths.len(), 2);
        assert_eq!(path.subpaths[1].knots[0].anchor, Point::new(0.0, 0.0));
        assert_eq!(path.subpaths[1].knots.last().expect("a knot").anchor, Point::new(20.0, 0.0));
    }

    #[test]
    fn cancelling_between_objects_stops_the_editable_page() {
        let mut content = String::from("1 0 0 rg\n");
        for i in 0..20 {
            content.push_str(&format!("{} 20 20 20 re f\n", i * 8));
        }
        let bytes = pdf(&content);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 3;
        let ctl = Interrupt::cancel_only(&cancel);
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        assert!(matches!(import_with_options_and_interrupt("objects.pdf", &bytes, &options, &ctl), Err(IoError::Cancelled)));
    }

    #[test]
    fn a_cancelled_editable_import_returns_cancelled() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f");
        let cancel = || true;
        let ctl = Interrupt::cancel_only(&cancel);
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        assert!(matches!(import_with_options_and_interrupt("paths.pdf", &bytes, &options, &ctl), Err(IoError::Cancelled)));
    }
    #[test]
    fn positioned_pdf_text_is_not_silently_reflowed() {
        let bytes = text_pdf("BT /F1 24 Tf 20 50 Td [(H) -900 (i)] TJ ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("spacing.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        assert!(children(&imported.document).iter().all(|l| matches!(l.content, LayerContent::Raster(_))), "unrepresentable spacing must rasterise");
        assert!(imported.warnings.iter().any(|w| w.contains("rasterised")));
    }

    #[test]
    fn colored_text_outlines_are_not_fake_bold() {
        let bytes = text_pdf("BT /F1 24 Tf 1 0 0 rg 0 0 1 RG 1 w 2 Tr 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("outline.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        assert!(children(&imported.document).iter().all(|l| matches!(l.content, LayerContent::Raster(_))), "a blue outline cannot become red bold text");
    }

    #[test]
    fn a_shape_cache_keeps_the_whole_miter() {
        let bytes = pdf("10 w 10 M 40 20 m 50 70 l 60 20 l S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("miter.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        let layer = children(&imported.document).first().unwrap();
        let LayerContent::Shape(shape) = &layer.content else { panic!("expected an editable stroke") };
        let board = Rect::from_xywh(0, 0, 200, 100);
        let expected = photocraft_vector::render_shape(shape, imported.document.pixel_format(), board);
        assert!(shape.cache.as_ref().unwrap().read_region(board) == expected.read_region(board), "bounds must include the pointed join");
    }

    #[test]
    fn cache_budget_charges_full_tiles_and_preserves_the_limit() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let tile_bytes = TILE_SIZE as u64 * TILE_SIZE as u64 * fmt.bytes_per_pixel() as u64;
        let mut used = MAX_CACHE_BYTES - tile_bytes;
        admit_cache(&mut used, Rect::from_xywh(0, 0, 1, 1), fmt).unwrap();
        assert_eq!(used, MAX_CACHE_BYTES);
        assert_eq!(admit_cache(&mut used, Rect::from_xywh(0, 0, 1, 1), fmt), Err(EditableError::Unsupported("page cache budget")));
        assert_eq!(used, MAX_CACHE_BYTES);
        let mut used = 0;
        admit_cache(&mut used, Rect::from_xywh(-1, -1, 2, 2), fmt).unwrap();
        assert_eq!(used, 4 * tile_bytes, "a tiny rectangle may straddle four tiles");
    }

    #[test]
    fn text_cache_respects_the_budget_already_used_by_shapes() {
        let bytes = text_pdf("BT /F1 24 Tf 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text-budget.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        let mut text = text_layer(&imported.document).clone();
        text.cache = None;
        let mut layers = vec![Layer::new("Text", LayerContent::Text(text))];
        let mut cached = MAX_CACHE_BYTES;
        assert_eq!(
            render_text_caches(&mut layers, 72.0, imported.document.pixel_format(), &Interrupt::NONE, &mut cached),
            Err(EditableError::Unsupported("page cache budget"))
        );
        let LayerContent::Text(text) = &layers[0].content else { panic!("text layer") };
        assert!(text.cache.is_none(), "reject before allocating the text cache");
    }

    #[test]
    fn multiple_text_caches_share_one_allowance() {
        let bytes = text_pdf("BT /F1 24 Tf 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text-budget.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        let mut text = text_layer(&imported.document).clone();
        text.cache = None;
        let fmt = imported.document.pixel_format();
        let layout = photocraft_text::shared().lock().unwrap().layout(&text, 72.0);
        let area = photocraft_text::render::ink_rect(&layout, &text.transform);
        let mut one_cache = 0;
        admit_cache(&mut one_cache, area, fmt).unwrap();
        assert!(one_cache > 0);
        let mut layers = vec![Layer::new("First", LayerContent::Text(text.clone())), Layer::new("Second", LayerContent::Text(text))];
        let mut cached = MAX_CACHE_BYTES - one_cache;
        assert_eq!(render_text_caches(&mut layers, 72.0, fmt, &Interrupt::NONE, &mut cached), Err(EditableError::Unsupported("page cache budget")));
        assert_eq!(cached, MAX_CACHE_BYTES);
        assert!(matches!(&layers[0].content, LayerContent::Text(t) if t.cache.is_some()));
        assert!(matches!(&layers[1].content, LayerContent::Text(t) if t.cache.is_none()));
        let mut cached = 0;
        render_text_caches(&mut layers, 72.0, fmt, &Interrupt::NONE, &mut cached).unwrap();
        assert_eq!(cached, one_cache * 2);
        assert!(layers.iter().all(|l| matches!(&l.content, LayerContent::Text(t) if t.cache.is_some())));
    }
    #[test]
    fn thick_text_outlines_are_not_fake_bold() {
        let bytes = text_pdf("BT /F1 24 Tf 1 0 0 rg 1 0 0 RG 5 w 2 Tr 20 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("thick-outline.pdf", &bytes, &options, &Interrupt::NONE).unwrap();
        assert!(children(&imported.document).iter().all(|l| matches!(l.content, LayerContent::Raster(_))));
    }
}
