//! Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`): the native layers, curves,
//! shapes, artboards, text, images and masks `craft-affinity` reads, mapped to PhotoCraft
//! layers (groups, shape layers, type layers, embedded smart objects, pixel layers, layer and
//! vector masks). What could not be mapped is listed in the warnings. A file whose native data
//! can't be read opens as its embedded preview, with a warning saying why.

use std::sync::Arc;

use craft_affinity::model::{self as af, Align, Blend, Kind, Pixels};
use craft_affinity::paint::{self as ap, Cap, GradientKind, Join};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::text::{CharStyle, ParagraphRun, ParagraphStyle, TextAlign, TextRun, TextShape};
use photocraft_doc::vector::{Knot, LineCap, LineJoin, Path, ShapeLayer, ShapeStroke, StrokeAlign, Subpath, VectorMask};
use photocraft_doc::{
    Artboard, ArtboardBackground, Document, Fill, GradientStyle, Group, Layer, LayerContent, LayerMask, Locks, SmartObject, SmartSource, TextLayer,
};
use photocraft_geom::{Affine, Point, Rect, Size};
use photocraft_raster::Surface;

use crate::{ImportResult, IoError};

pub use craft_affinity::is_affinity;

/// Extensions recognized as Affinity documents.
pub const EXTENSIONS: &[&str] = &["af", "afphoto", "afdesign", "afpub"];

/// Pixels an image or mask is resampled into, after cropping to the canvas.
const MAX_PIXELS: u64 = 64 << 20;
/// Canvas side in pixels.
const MAX_SIDE: f64 = 300_000.0;
/// Decoded pixel bytes allowed across the whole import: the caps above are per call, so a small
/// container of embedded images could still decode (and keep) a surface each until the process
/// dies. ORA and KRA budget their decoded pixels the same way.
const MAX_PIXEL_BYTES: u64 = 1 << 30;
/// Bytes the import's optional pixel caches (shapes, type layers) may hold together. They are
/// drawn again on the first edit, so running out costs nothing but speed; raster layers and
/// embedded images are lost for good when they don't fit. A separate pool keeps thousands of
/// cheap caches early in a file from using up [`MAX_PIXEL_BYTES`] and dropping images later.
const MAX_CACHE_BYTES: u64 = 512 << 20;

pub fn has_extension(name: &str) -> bool {
    name.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).is_some_and(|(_, ext)| EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

pub(crate) fn import(name: &str, bytes: &[u8], ctl: &photocraft_raster::Interrupt) -> Result<ImportResult, IoError> {
    match craft_affinity::read(bytes, craft_affinity::Limits::default()) {
        Ok(d) => {
            let mut b = Builder::new(name, &d)?;
            b.ctl = *ctl;
            b.document(&d)?;
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut warnings: Vec<String> = d.warnings.iter().map(|w| format!("Affinity: not imported or approximated: {w}.")).collect();
            warnings.extend(b.warnings.iter().map(|w| format!("Affinity: {w}.")));
            // Type layers draw from their model, within the import's raster bound: the sizes and
            // frames come from an untrusted file. They share the optional-cache budget with the
            // shapes, so a file with thousands of type layers can't allocate without end.
            let mut document = b.doc;
            let unrendered = crate::text_import::prepare_within(&mut document, None, &mut b.cache_budget, ctl).map_err(|_| IoError::Cancelled)?;
            if unrendered > 0 {
                warnings.push(format!("Affinity: {unrendered} type layers beyond the import's memory limit were left without pixels until they are edited."));
            }
            if b.uncached_shapes > 0 {
                warnings.push(format!("Affinity: {} shapes beyond the import's memory limit were left without a pixel cache.", b.uncached_shapes));
            }
            // Save never writes back over the Affinity file.
            Ok(ImportResult { document, warnings, source_read_only: true, preview_only: false })
        }
        Err(native) => preview(name, bytes, &native),
    }
}

/// The embedded preview, for files whose native document can't be read.
fn preview(name: &str, bytes: &[u8], native: &craft_affinity::Error) -> Result<ImportResult, IoError> {
    let p = craft_affinity::preview(bytes).map_err(|e| IoError::Unsupported(format!("{native}; its embedded preview can't be read either ({e})")))?;
    let opts = photocraft_codecs::DecodeOptions {
        limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 128 << 20 },
        ..Default::default()
    };
    let img = photocraft_codecs::decode_with(p.png, &opts)?;
    let mut r = crate::flat::image_to_document(name, &img)?;
    if let Some(layer) = r.document.layers.first_mut() {
        layer.name = "Affinity preview".into();
        layer.locks = Default::default();
    }
    r.source_read_only = true;
    r.preview_only = true;
    r.warnings.insert(
        0,
        format!(
            "The native Affinity document could not be read ({native}). Opened only its embedded {}×{} PNG preview, which may be smaller than the document: no layers, vectors, text or pages. Save a new copy; the Affinity source cannot be saved back. Export PSD or PNG from Affinity for full-resolution artwork.",
            p.width, p.height
        ),
    );
    Ok(r)
}

struct Builder<'a> {
    doc: Document,
    warnings: Vec<String>,
    /// Document pixels (in the canvas) per Affinity document pixel: always 1; spreads are offset.
    format: PixelFormat,
    canvas: Rect,
    /// Decoded pixel bytes left for embedded images and their resampled surfaces.
    pixel_budget: u64,
    /// Bytes left for the optional shape and type caches ([`MAX_CACHE_BYTES`]).
    cache_budget: u64,
    /// Shapes that went without a cache because `cache_budget` ran out.
    uncached_shapes: usize,
    /// Checked per layer: a cancelled import stops building.
    ctl: photocraft_raster::Interrupt<'a>,
}

/// Deduct `cost` bytes from a decoded-pixel budget; `false` (leaving the budget alone) when it
/// cannot cover them.
fn charge(budget: &mut u64, cost: u64) -> bool {
    match budget.checked_sub(cost) {
        Some(rest) => {
            *budget = rest;
            true
        }
        None => false,
    }
}

fn affine(a: af::Affine) -> Affine {
    Affine { m: a.0 }
}

fn point(p: af::Point) -> Point {
    Point::new(p.x, p.y)
}

impl Builder<'_> {
    fn new(name: &str, d: &af::Document) -> Result<Self, IoError> {
        // Spreads (Publisher pages) side by side, 64 px apart.
        let mut w: f64 = 0.0;
        let mut h: f64 = 0.0;
        for (i, s) in d.spreads.iter().enumerate() {
            w += (s.bounds.x1 - s.bounds.x0).max(1.0) + if i > 0 { 64.0 } else { 0.0 };
            h = h.max((s.bounds.y1 - s.bounds.y0).max(1.0));
        }
        if !(w.is_finite() && h.is_finite()) || w > MAX_SIDE || h > MAX_SIDE {
            return Err(IoError::Unsupported("the Affinity document is larger than 300 000 pixels on a side".into()));
        }
        let size = Size::new(w.ceil().max(1.0) as u32, h.ceil().max(1.0) as u32);
        let mut doc = Document::new(name, size, ColorMode::Rgb, SampleType::U8);
        doc.resolution_dpi = d.dpi as f32;
        let canvas = Rect::new(0, 0, size.width as i32, size.height as i32);
        Ok(Self {
            doc,
            warnings: Vec::new(),
            format: PixelFormat::RGBA8,
            canvas,
            pixel_budget: MAX_PIXEL_BYTES,
            cache_budget: MAX_CACHE_BYTES,
            uncached_shapes: 0,
            ctl: photocraft_raster::Interrupt::NONE,
        })
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn document(&mut self, d: &af::Document) -> Result<(), IoError> {
        let pages = d.spreads.len() > 1 || d.spreads.iter().any(|s| s.pages.len() > 1);
        let mut x = 0.0;
        let mut layers = Vec::new();
        for (i, spread) in d.spreads.iter().enumerate() {
            let to_doc = Affine::translate(x - spread.bounds.x0, -spread.bounds.y0);
            let mut children: Vec<Layer> = spread.nodes.iter().filter_map(|n| self.layer(n, to_doc, true)).collect();
            if pages {
                // Each page becomes an artboard holding the art drawn on it.
                let rects: Vec<af::Rect> = if spread.pages.is_empty() { vec![spread.bounds] } else { spread.pages.clone() };
                let page = rects.first().copied().unwrap_or(spread.bounds);
                let r = self.bbox(to_doc, page);
                let mut g = Layer::new(
                    format!("Page {}", i + 1),
                    LayerContent::Group(Group { children: std::mem::take(&mut children), expanded: true, artboard: None, frame_stroke: None }),
                );
                if let LayerContent::Group(gr) = &mut g.content {
                    gr.artboard = Some(Artboard { rect: r, background: ArtboardBackground::White, preset: String::new() });
                }
                if rects.len() > 1 {
                    self.warn("facing pages import as one artboard per spread");
                }
                layers.push(g);
            } else {
                layers.append(&mut children);
            }
            x += (spread.bounds.x1 - spread.bounds.x0).max(1.0) + 64.0;
        }
        if d.spreads.first().is_some_and(|s| !s.transparent) && !self.doc_has_artboards(&layers) {
            // An opaque page: a white background below the art.
            let mut bg = Surface::with_default(self.format, &[1.0, 1.0, 1.0, 1.0]);
            bg.prune();
            let mut l = Layer::new("Background", LayerContent::Raster(bg));
            l.locks.transparency = true;
            l.locks.position = true;
            layers.insert(0, l);
        }
        self.doc.layers = layers;
        Ok(())
    }

    fn doc_has_artboards(&self, layers: &[Layer]) -> bool {
        layers.iter().any(|l| matches!(&l.content, LayerContent::Group(g) if g.artboard.is_some()))
    }

    /// Integer bounding box of `r` mapped by `m`, clipped to the canvas.
    fn bbox(&self, m: Affine, r: af::Rect) -> Rect {
        let pts = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| m.apply(Point::new(x, y)));
        let fold = |f: fn(f64, f64) -> f64, init: f64, g: fn(&Point) -> f64| pts.iter().map(g).fold(init, f);
        let x0 = fold(f64::min, f64::INFINITY, |p| p.x).floor();
        let y0 = fold(f64::min, f64::INFINITY, |p| p.y).floor();
        let x1 = fold(f64::max, f64::NEG_INFINITY, |p| p.x).ceil();
        let y1 = fold(f64::max, f64::NEG_INFINITY, |p| p.y).ceil();
        let c = |v: f64, lo: i32, hi: i32| (v.clamp(f64::from(lo), f64::from(hi))) as i32;
        Rect::new(
            c(x0, self.canvas.x0, self.canvas.x1),
            c(y0, self.canvas.y0, self.canvas.y1),
            c(x1, self.canvas.x0, self.canvas.x1),
            c(y1, self.canvas.y0, self.canvas.y1),
        )
    }

    fn blend(&mut self, b: Blend) -> BlendMode {
        match b {
            Blend::Normal => BlendMode::Normal,
            Blend::PassThrough => BlendMode::PassThrough,
            Blend::Darken => BlendMode::Darken,
            Blend::DarkerColor => BlendMode::DarkerColor,
            Blend::Multiply => BlendMode::Multiply,
            Blend::ColorBurn => BlendMode::ColorBurn,
            Blend::Lighten => BlendMode::Lighten,
            Blend::LighterColor => BlendMode::LighterColor,
            Blend::Screen => BlendMode::Screen,
            Blend::ColorDodge => BlendMode::ColorDodge,
            Blend::Add => BlendMode::LinearDodge,
            Blend::Overlay => BlendMode::Overlay,
            Blend::SoftLight => BlendMode::SoftLight,
            Blend::HardLight => BlendMode::HardLight,
            Blend::VividLight => BlendMode::VividLight,
            Blend::PinLight => BlendMode::PinLight,
            Blend::LinearLight => BlendMode::LinearLight,
            Blend::HardMix => BlendMode::HardMix,
            Blend::Difference => BlendMode::Difference,
            Blend::Exclusion => BlendMode::Exclusion,
            Blend::Subtract => BlendMode::Subtract,
            Blend::Hue => BlendMode::Hue,
            Blend::Saturation => BlendMode::Saturation,
            Blend::Luminosity => BlendMode::Luminosity,
            Blend::Color => BlendMode::Color,
            Blend::Average | Blend::Negation | Blend::Reflect | Blend::Glow | Blend::Erase => {
                self.warn("the Average, Negation, Reflect, Glow and Erase blend modes import as Normal");
                BlendMode::Normal
            }
        }
    }

    fn base(&mut self, n: &af::Node, content: LayerContent) -> Layer {
        let mut l = Layer::new(if n.name.is_empty() { default_name(&n.kind) } else { n.name.clone() }, content);
        l.visible = n.visible;
        l.opacity = n.opacity as f32;
        l.blend = self.blend(n.blend);
        if n.locked {
            l.locks = Locks { all: true, ..Locks::default() };
        }
        l
    }

    fn group(&mut self, n: &af::Node, children: Vec<Layer>) -> Layer {
        // Pass-through and Normal (isolated) groups map one to one.
        self.base(n, LayerContent::Group(Group { children, expanded: false, artboard: None, frame_stroke: None }))
    }

    /// One Affinity node and its subtree; `to_doc` maps Affinity document pixels to the canvas.
    fn layer(&mut self, n: &af::Node, to_doc: Affine, top: bool) -> Option<Layer> {
        if self.ctl.cancelled() {
            return None;
        }
        let children: Vec<Layer> = n.children.iter().filter_map(|c| self.layer(c, to_doc, false)).collect();
        let mut l = match &n.kind {
            Kind::Layer | Kind::Group | Kind::Table(_) | Kind::MasterInstance { .. } => self.group(n, children),
            Kind::Unsupported if children.is_empty() => return None,
            Kind::Unsupported => self.group(n, children),
            Kind::Shape { path, fills, strokes, even_odd } => {
                let path = self.path(path, to_doc, *even_odd);
                let shape = self.shape(path.clone(), fills, strokes, to_doc);
                if children.is_empty() {
                    self.base(n, LayerContent::Shape(shape))
                } else {
                    // A shape clips its children: a group masked by the outline, the shape at its bottom.
                    let mut own = Layer::new(if n.name.is_empty() { "Shape".to_string() } else { n.name.clone() }, LayerContent::Shape(shape));
                    own.opacity = 1.0;
                    let mut kids = vec![own];
                    kids.extend(children);
                    let mut g = self.group(n, kids);
                    g.vector_mask = Some(VectorMask { path, enabled: true, linked: true, density: 1.0, feather: 0.0 });
                    if !strokes.is_empty() {
                        self.warn("strokes of shapes that clip other layers are clipped to the shape");
                    }
                    g
                }
            }
            Kind::Artboard { rect, background, .. } => {
                if n.class != craft_affinity::stream::Tag::of(b"ShpN") {
                    self.warn("a converted artboard outline (imported at its bounding box)");
                }
                let r = self.bbox(to_doc, *rect);
                let bg = match background.first() {
                    Some(ap::Paint::Solid(c)) => {
                        let c = color(*c);
                        if c.alpha <= 0.0 { ArtboardBackground::Transparent } else { ArtboardBackground::Custom(c) }
                    }
                    Some(_) => {
                        self.warn("artboard backgrounds other than a solid colour import as transparent");
                        ArtboardBackground::Transparent
                    }
                    None => ArtboardBackground::Transparent,
                };
                let mut g = self.group(n, children);
                if top {
                    if let LayerContent::Group(gr) = &mut g.content {
                        gr.artboard = Some(Artboard { rect: r, background: bg, preset: String::new() });
                    }
                } else {
                    // PhotoCraft artboards are top-level groups: a nested one clips by a vector mask.
                    let p = Path::new(vec![Subpath::polygon(&[
                        (f64::from(r.x0), f64::from(r.y0)),
                        (f64::from(r.x1), f64::from(r.y0)),
                        (f64::from(r.x1), f64::from(r.y1)),
                        (f64::from(r.x0), f64::from(r.y1)),
                    ])]);
                    g.vector_mask = Some(VectorMask { path: p, enabled: true, linked: true, density: 1.0, feather: 0.0 });
                    self.warn("artboards inside groups import as masked groups");
                }
                g
            }
            Kind::Text(t) => {
                let layer = self.text(t, to_doc);
                let l = self.base(n, LayerContent::Text(layer));
                if children.is_empty() {
                    l
                } else {
                    let mut kids = vec![l];
                    kids.extend(children);
                    self.group(n, kids)
                }
            }
            Kind::Image(img) => match self.image(img, to_doc) {
                Some(content) => self.base(n, content),
                None if children.is_empty() => return None,
                None => self.group(n, children),
            },
        };
        if let Some(mask) = &n.mask {
            l.vector_mask = Some(VectorMask { path: self.path(mask, to_doc, false), enabled: true, linked: true, density: 1.0, feather: 0.0 });
        }
        if let Some(mask) = &n.pixel_mask
            && let Some(surface) = self.mask(mask, to_doc)
        {
            l.mask = Some(LayerMask { surface, enabled: true, linked: true, density: 1.0, feather: 0.0 });
        }
        Some(l)
    }

    fn path(&self, p: &af::Path, to_doc: Affine, even_odd: bool) -> Path {
        let m = to_doc;
        let subpaths = p
            .subpaths
            .iter()
            .map(|s| {
                // Knots: each anchor with the control points of the segments arriving and leaving.
                let mut knots: Vec<Knot> = Vec::with_capacity(s.segments.len() + 1);
                let start = m.apply(point(s.start));
                knots.push(Knot::corner(start.x, start.y));
                for [c1, c2, e] in &s.segments {
                    if let Some(prev) = knots.last_mut() {
                        prev.out_ctrl = m.apply(point(*c1));
                    }
                    let e = m.apply(point(*e));
                    let mut k = Knot::corner(e.x, e.y);
                    k.in_ctrl = m.apply(point(*c2));
                    knots.push(k);
                }
                // A closed subpath that ends where it starts: the last knot is the first.
                if s.closed
                    && knots.len() > 1
                    && let (Some(f), Some(l)) = (knots.first().copied(), knots.last().copied())
                    && (f.anchor.x - l.anchor.x).abs() < 1e-9
                    && (f.anchor.y - l.anchor.y).abs() < 1e-9
                {
                    knots.pop();
                    if let Some(first) = knots.first_mut() {
                        first.in_ctrl = l.in_ctrl;
                    }
                }
                Subpath { closed: s.closed, knots, op: Default::default() }
            })
            .collect();
        let mut path = Path::new(subpaths);
        if even_odd {
            // One component filled even-odd, so inner subpaths cut holes as in Affinity.
            path.fill_rule = photocraft_doc::vector::FillRule::EvenOdd;
            for s in path.subpaths.iter_mut().skip(1) {
                s.op = photocraft_doc::vector::PathOp::Join;
            }
        }
        path
    }

    fn fill(&mut self, p: &ap::Paint, frame: Rect, to_doc: Affine) -> Option<Fill> {
        match p {
            ap::Paint::None => None,
            ap::Paint::Solid(c) => Some(Fill::Solid(color(*c))),
            ap::Paint::Gradient(g) => {
                let style = match g.kind {
                    GradientKind::Linear => GradientStyle::Linear,
                    GradientKind::Radial => GradientStyle::Radial,
                    GradientKind::Conical => GradientStyle::Angle,
                };
                let m = to_doc.mul(&affine(g.transform));
                let (o, e, v) = (m.apply(Point::new(0.0, 0.0)), m.apply(Point::new(1.0, 0.0)), m.apply(Point::new(0.0, 1.0)));
                let (u, w) = ((e.x - o.x).hypot(e.y - o.y), (v.x - o.x).hypot(v.y - o.y));
                if style == GradientStyle::Radial && u > 0.0 && (w / u - 1.0).abs() > 0.01 {
                    self.warn("elliptical gradients import as circular ones");
                }
                let (angle, scale, offset) =
                    photocraft_compose::gradient_fill::from_handles(style, [o.x as f32, o.y as f32], [e.x as f32, e.y as f32], frame, 0.0);
                let stops = g.stops.iter().map(|s| (s.offset as f32, Color { alpha: 1.0, ..color(s.color) })).collect();
                let opacity_stops = g.stops.iter().map(|s| (s.offset as f32, s.color.alpha() as f32)).collect();
                let midpoints = g.stops.iter().take(g.stops.len().saturating_sub(1)).map(|s| s.half_point() as f32).collect();
                Some(Fill::Gradient { stops, angle, scale, style, reverse: false, opacity_stops, midpoints, offset, dither: false, align: true })
            }
        }
    }

    fn shape(&mut self, path: Path, fills: &[ap::Paint], strokes: &[ap::Stroke], to_doc: Affine) -> ShapeLayer {
        let frame = path
            .control_bounds()
            .map(|(x0, y0, x1, y1)| Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32))
            .unwrap_or(self.canvas);
        if fills.len() > 1 || strokes.len() > 1 {
            self.warn("only one fill and one stroke per shape import");
        }
        let fill = fills.first().and_then(|f| self.fill(f, frame, to_doc));
        let stroke = strokes.first().and_then(|s| {
            let paint = self.fill(&s.paint, frame, to_doc)?;
            if s.behind {
                self.warn("strokes drawn behind their fill import in front of it");
            }
            Some(ShapeStroke {
                width: s.width as f32,
                paint,
                opacity: 1.0,
                align: match s.align {
                    ap::Align::Center => StrokeAlign::Center,
                    ap::Align::Inside => StrokeAlign::Inside,
                    ap::Align::Outside => StrokeAlign::Outside,
                },
                cap: match s.cap {
                    Cap::Butt => LineCap::Butt,
                    Cap::Round => LineCap::Round,
                    Cap::Square => LineCap::Square,
                },
                join: match s.join {
                    Join::Miter => LineJoin::Miter,
                    Join::Round => LineJoin::Round,
                    Join::Bevel => LineJoin::Bevel,
                },
                miter_limit: s.miter_limit as f32,
                // In multiples of the width, as Photoshop stores them.
                dashes: s.dash.as_ref().map(|(d, _)| d.iter().map(|v| (v / s.width.max(1e-9)) as f32).collect()).unwrap_or_default(),
                dash_offset: s.dash.as_ref().map_or(0.0, |(_, o)| (o / s.width.max(1e-9)) as f32),
            })
        });
        let mut sh = ShapeLayer { path, fill, stroke, ..ShapeLayer::default() };
        // The pixel cache is optional (the shape renders from its path without it), so charge the
        // area it would cover to the import's pixel budget and go without it when that is spent:
        // geometry across a 300000 px canvas would otherwise be rasterized for every shape.
        // Once not even one tile is left nothing can be charged, so skip compiling the shape
        // (flattening and stroking its path) just to throw the result away.
        let tile = crate::text_import::cache_bytes(Rect::new(0, 0, 1, 1), self.format.bytes_per_pixel());
        if self.cache_budget < tile {
            if !sh.path.subpaths.is_empty() {
                self.uncached_shapes += 1;
            }
            return sh;
        }
        // The compiled bounds are what `render` covers, an acute miter reaching past the stroke width included.
        #[cfg(test)]
        COMPILED_SHAPES.with(|n| n.set(n.get() + 1));
        let compiled = photocraft_vector::CompiledShape::new(&sh, photocraft_vector::DEFAULT_TOLERANCE, self.canvas);
        let area = compiled.bounds().map_or(Rect::default(), |b| b.intersect(&self.canvas));
        // Whole tiles: even a 2x2 px shape keeps one, which 50 000 small shapes add up to gigabytes.
        let cost = crate::text_import::cache_bytes(area, self.format.bytes_per_pixel());
        if charge(&mut self.cache_budget, cost) {
            sh.cache = Some(compiled.render(self.format, self.canvas));
        } else {
            self.uncached_shapes += 1;
        }
        sh
    }

    fn text(&mut self, t: &af::Text, to_doc: Affine) -> TextLayer {
        let origin = match t.frame {
            Some(f) => Point::new(f.x0, f.y0),
            None => point(t.anchor),
        };
        let m = to_doc.mul(&affine(t.transform)).mul(&Affine::translate(origin.x, origin.y));
        let k = m.determinant().abs().sqrt().max(1e-9);
        let transform = m.mul(&Affine::scale(1.0 / k));
        let dpi = self.doc.resolution_dpi.max(1.0) as f64;
        let mut text = String::new();
        let mut runs = Vec::new();
        for r in &t.runs {
            // Affinity separates paragraphs with U+2029 and lines with U+2028.
            let s = r.text.replace(['\u{2029}', '\u{2028}'], "\r");
            let style = CharStyle {
                font_family: r.family.clone(),
                postscript_name: (!r.postscript.is_empty()).then(|| r.postscript.clone()),
                weight: r.weight.clamp(100, 900) as u16,
                italic: r.italic,
                size_pt: (r.size * k * 72.0 / dpi) as f32,
                color: match &r.fill {
                    ap::Paint::Solid(c) => color(*c),
                    _ => Color::BLACK,
                },
                tracking: (r.tracking * 1000.0) as f32,
                leading_pt: r.leading.map(|l| (l * k * 72.0 / dpi) as f32),
                ..CharStyle::default()
            };
            text.push_str(&s);
            runs.push(TextRun { len: s.len(), style });
        }
        let align = match t.align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
            Align::Justify => TextAlign::JustifyLeft,
        };
        let mut layer = TextLayer {
            text: text.clone(),
            font_family: runs.first().map(|r| r.style.font_family.clone()).unwrap_or_default(),
            size_pt: runs.first().map_or(12.0, |r| r.style.size_pt),
            color: runs.first().map_or(Color::BLACK, |r| r.style.color),
            transform,
            runs,
            paragraphs: vec![ParagraphRun { len: text.len(), style: ParagraphStyle { align, ..ParagraphStyle::default() } }],
            ..TextLayer::default()
        };
        if let Some(f) = t.frame {
            layer.shape = TextShape::Box { x: 0.0, y: 0.0, width: ((f.x1 - f.x0) * k) as f32, height: ((f.y1 - f.y0) * k) as f32 };
        }
        layer
    }

    /// Straight RGBA8 pixels of an image (decoding an embedded file), with its size.
    fn pixels(&mut self, img: &af::Image) -> Option<(u32, u32, Vec<u8>)> {
        match &img.pixels {
            Pixels::Rgba8(v) => Some((img.width, img.height, v.clone())),
            Pixels::Cmyka8(v) => Some((img.width, img.height, Pixels::cmyka8_to_rgba8(v))),
            Pixels::Encoded(bytes) => {
                let opts = photocraft_codecs::DecodeOptions {
                    limits: photocraft_codecs::Limits { max_width: 65_535, max_height: 65_535, max_pixels: MAX_PIXELS, max_alloc: 512 << 20 },
                    ..Default::default()
                };
                match photocraft_codecs::decode_with(bytes, &opts) {
                    Ok(d) => {
                        let (w, h) = d.dimensions();
                        if !charge(&mut self.pixel_budget, u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4)) {
                            self.warn("the document's embedded images need more pixel memory than the import allows; the rest were left out");
                            return None;
                        }
                        Some((w, h, d.to_rgba8()))
                    }
                    Err(_) => {
                        self.warn("an embedded image that could not be decoded was left out");
                        None
                    }
                }
            }
        }
    }

    fn image(&mut self, img: &af::Image, to_doc: Affine) -> Option<LayerContent> {
        let (w, h, rgba) = self.pixels(img)?;
        // The pixel grid maps onto the Affinity image's own size, whatever the stored file's.
        let m = to_doc
            .mul(&affine(img.transform))
            .mul(&Affine { m: [f64::from(img.width) / f64::from(w.max(1)), 0.0, 0.0, f64::from(img.height) / f64::from(h.max(1)), 0.0, 0.0] });
        let surface = self.resample(w, h, &rgba, m, 4)?;
        Some(match &img.pixels {
            // A placed image keeps its file, as a smart object placed by the same transform.
            Pixels::Encoded(bytes) => LayerContent::Smart(SmartObject::new(
                SmartSource::Embedded { file_name: "Affinity image".into(), bytes: Arc::new(bytes.clone()) },
                m,
                Some(surface),
            )),
            Pixels::Rgba8(_) | Pixels::Cmyka8(_) => LayerContent::Raster(surface),
        })
    }

    fn mask(&mut self, img: &af::Image, to_doc: Affine) -> Option<Surface> {
        let (w, h, rgba) = self.pixels(img)?;
        let grey: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        let m = to_doc.mul(&affine(img.transform));
        self.resample(w, h, &grey, m, 1)
    }

    /// Bilinear resampling of `src` (`w`×`h`, `channels` 4 = straight RGBA8 or 1 = grey) mapped
    /// by `m` (source pixels to canvas pixels) into a surface; exact copy when `m` is an integer
    /// translation. Pixels outside the source are transparent (grey: 0, hidden).
    fn resample(&mut self, w: u32, h: u32, src: &[u8], m: Affine, channels: usize) -> Option<Surface> {
        let format = if channels == 1 { PixelFormat::GRAY8 } else { self.format };
        let mut out = if channels == 1 { Surface::with_default(format, &[0.0]) } else { Surface::new(format) };
        let r = self.bbox(m, af::Rect { x0: 0.0, y0: 0.0, x1: f64::from(w), y1: f64::from(h) });
        if r.width() == 0 || r.height() == 0 {
            return Some(out);
        }
        if u64::from(r.width()) * u64::from(r.height()) > MAX_PIXELS {
            self.warn("images larger than 64 megapixels on the canvas were left out");
            return None;
        }
        let [a, b, c, d, e, f] = m.m;
        let exact = a == 1.0 && d == 1.0 && b == 0.0 && c == 0.0 && e.fract() == 0.0 && f.fract() == 0.0;
        let inv = m.inverse()?;
        let (rw, rh) = (r.width() as usize, r.height() as usize);
        // The surface keeps whole tiles: a narrow image stores up to 256 times its pixels.
        if !charge(&mut self.pixel_budget, photocraft_raster::tile_storage_bytes(r, &format)) {
            self.warn("the document's images need more pixel memory than the import allows; the rest were left out");
            return None;
        }
        let mut buf = vec![0u8; rw * rh * channels];
        let at = |x: i64, y: i64, ch: usize| -> f64 {
            if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
                return 0.0;
            }
            f64::from(src.get((y as usize * w as usize + x as usize) * channels + ch).copied().unwrap_or(0))
        };
        for (row, out_row) in buf.chunks_exact_mut(rw * channels).enumerate() {
            for (col, px) in out_row.chunks_exact_mut(channels).enumerate() {
                let p = inv.apply(Point::new(f64::from(r.x0) + col as f64 + 0.5, f64::from(r.y0) + row as f64 + 0.5));
                if exact {
                    let (x, y) = (p.x.floor() as i64, p.y.floor() as i64);
                    for (ch, v) in px.iter_mut().enumerate() {
                        *v = at(x, y, ch) as u8;
                    }
                    continue;
                }
                let (fx, fy) = (p.x - 0.5, p.y - 0.5);
                let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
                let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
                let weights =
                    [((x0, y0), (1.0 - tx) * (1.0 - ty)), ((x0 + 1, y0), tx * (1.0 - ty)), ((x0, y0 + 1), (1.0 - tx) * ty), ((x0 + 1, y0 + 1), tx * ty)];
                if channels == 1 {
                    px[0] = weights.iter().map(|((x, y), wgt)| at(*x, *y, 0) * wgt).sum::<f64>().round().clamp(0.0, 255.0) as u8;
                } else {
                    // Interpolate premultiplied colour so transparent pixels don't bleed.
                    let alpha: f64 = weights.iter().map(|((x, y), wgt)| at(*x, *y, 3) * wgt).sum();
                    for (ch, v) in px.iter_mut().enumerate().take(3) {
                        let pm: f64 = weights.iter().map(|((x, y), wgt)| at(*x, *y, ch) * at(*x, *y, 3) / 255.0 * wgt).sum();
                        *v = if alpha > 0.0 { (pm * 255.0 / alpha).round().clamp(0.0, 255.0) as u8 } else { 0 };
                    }
                    px[3] = alpha.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        out.write_interleaved(r, &buf);
        out.prune();
        Some(out)
    }
}

fn default_name(k: &Kind) -> String {
    match k {
        Kind::Layer => "Layer",
        Kind::Group => "Group",
        Kind::Artboard { .. } => "Artboard",
        Kind::Shape { .. } => "Shape",
        Kind::Text(_) => "Text",
        Kind::Image(_) => "Image",
        Kind::Table(_) => "Table",
        Kind::MasterInstance { .. } => "Master",
        Kind::Unsupported => "Group",
    }
    .into()
}

fn color(c: ap::Color) -> Color {
    let f = |v: f64| v.clamp(0.0, 1.0) as f32;
    match c {
        ap::Color::Rgb { r, g, b, a } => Color::rgba(f(r), f(g), f(b), f(a)),
        ap::Color::Cmyk { c, m, y, k, a } => {
            let cmyk = Color { mode: ColorMode::Cmyk, c: [f(c), f(m), f(y), f(k)], alpha: f(a) };
            let [r, g, b] = cmyk.to_rgb();
            Color::rgba(r, g, b, f(a))
        }
        ap::Color::Gray { v, a } => Color::rgba(f(v), f(v), f(v), f(a)),
        ap::Color::Lab { l, a, b, alpha } => {
            let lab = Color { mode: ColorMode::Lab, c: [f(l / 100.0), f((a + 128.0) / 255.0), f((b + 128.0) / 255.0), 0.0], alpha: f(alpha) };
            let [r, g, bl] = lab.to_rgb();
            Color::rgba(r, g, bl, f(alpha))
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How many shapes this thread compiled for a pixel cache (for the exhausted-budget test).
    static COMPILED_SHAPES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transparent 64×64 page holding one image node per entry; the identity transform maps
    /// each image onto the page's top-left corner.
    fn affinity_document(images: Vec<(u32, u32, Pixels)>) -> af::Document {
        let nodes = images
            .into_iter()
            .enumerate()
            .map(|(i, (width, height, pixels))| af::Node {
                class: craft_affinity::synth::tag(b"ImgN"),
                name: format!("Image {i}"),
                visible: true,
                locked: false,
                opacity: 1.0,
                blend: Blend::Normal,
                kind: Kind::Image(af::Image { width, height, pixels, transform: af::Affine::IDENTITY }),
                mask: None,
                pixel_mask: None,
                effects: Vec::new(),
                children: Vec::new(),
            })
            .collect();
        af::Document {
            dpi: 72.0,
            spreads: vec![af::Spread { bounds: af::Rect { x0: 0.0, y0: 0.0, x1: 64.0, y1: 64.0 }, pages: Vec::new(), transparent: true, nodes }],
            saved_by: None,
            warnings: Vec::new(),
            masters: Vec::new(),
            cmyk_profile: None,
        }
    }

    #[test]
    fn the_pixel_budget_refuses_what_does_not_fit() {
        let mut budget = 100u64;
        assert!(charge(&mut budget, 60));
        assert!(!charge(&mut budget, 60), "a later image cannot overdraw the import's budget");
        assert_eq!(budget, 40, "a refused charge leaves the budget alone");
    }

    #[test]
    fn images_past_the_pixel_budget_are_left_out() {
        // Two 64×64 RGBA images each keep a whole 256×256 tile (256 KiB); a one-tile budget admits one and
        // the second is dropped with a warning instead of being staged anyway.
        let d = affinity_document(vec![(64, 64, Pixels::Rgba8(vec![0; 64 * 64 * 4])), (64, 64, Pixels::Rgba8(vec![0; 64 * 64 * 4]))]);
        let mut b = Builder::new("budget", &d).unwrap();
        b.pixel_budget = 256 * 256 * 4; // one whole tile per image
        b.document(&d).unwrap();
        assert_eq!(b.doc.layers.len(), 1, "the image past the budget must not import");
        assert!(b.warnings.iter().any(|w| w.contains("pixel memory")), "{:?}", b.warnings);
    }

    #[test]
    fn embedded_images_past_the_pixel_budget_are_not_decoded_into_it() {
        // An 8×8 embedded PNG costs 256 bytes decoded and 256 more resampled.
        let png = {
            let img = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgba, vec![0; 8 * 8 * 4]).unwrap();
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
        };
        let d = affinity_document(vec![(8, 8, Pixels::Encoded(png))]);
        let mut b = Builder::new("budget", &d).unwrap();
        b.pixel_budget = 8 * 8 * 4 - 1;
        b.document(&d).unwrap();
        assert!(b.doc.layers.is_empty(), "the embedded image must not import past the budget");
        assert!(b.warnings.iter().any(|w| w.contains("pixel memory")), "{:?}", b.warnings);
        // The same image imports when the budget covers the decode and the resample.
        let mut ok = Builder::new("room", &d).unwrap();
        ok.pixel_budget = 8 * 8 * 4 + 256 * 256 * 4;
        ok.document(&d).unwrap();
        assert_eq!(ok.doc.layers.len(), 1, "within the budget the image imports as a smart object");
    }

    fn node(kind: Kind) -> af::Node {
        af::Node {
            class: craft_affinity::synth::tag(b"ImgN"),
            name: "n".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: Blend::Normal,
            kind,
            mask: None,
            pixel_mask: None,
            effects: Vec::new(),
            children: Vec::new(),
        }
    }

    fn page(nodes: Vec<af::Node>) -> af::Document {
        af::Document {
            dpi: 72.0,
            spreads: vec![af::Spread { bounds: af::Rect { x0: 0.0, y0: 0.0, x1: 64.0, y1: 64.0 }, pages: Vec::new(), transparent: true, nodes }],
            saved_by: None,
            warnings: Vec::new(),
            masters: Vec::new(),
            cmyk_profile: None,
        }
    }

    /// A 2×2 px solid rectangle.
    fn tiny_shape(i: usize) -> af::Node {
        let p = |x: f64, y: f64| af::Point { x, y };
        let (x, y) = ((i % 30) as f64, (i / 30 % 30) as f64);
        let corners = [p(x, y), p(x + 2.0, y), p(x + 2.0, y + 2.0), p(x, y + 2.0)];
        let sub = af::SubPath { start: corners[0], segments: (1..=4).map(|k| [corners[k % 4]; 3]).collect(), closed: true };
        node(Kind::Shape {
            path: af::Path { subpaths: vec![sub] },
            fills: vec![ap::Paint::Solid(ap::Color::Rgb { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })],
            strokes: Vec::new(),
            even_odd: false,
        })
    }

    #[test]
    fn small_shapes_are_charged_a_whole_tile_each() {
        // Surfaces keep whole tiles, so 2×2 px shapes cost a tile each: 50 000 of them were
        // 7 GB. A budget of three tiles keeps the pixel cache of the first three only.
        let d = page((0..10).map(tiny_shape).collect());
        let mut b = Builder::new("shapes", &d).unwrap();
        b.cache_budget = 3 * 256 * 256 * 4;
        b.document(&d).unwrap();
        let cached = |l: &Layer| matches!(&l.content, LayerContent::Shape(s) if s.cache.is_some());
        assert_eq!(b.doc.layers.iter().filter(|l| cached(l)).count(), 3);
        assert_eq!(b.doc.layers.iter().filter(|l| matches!(l.content, LayerContent::Shape(_))).count(), 10, "every shape still imports");
        assert_eq!(b.uncached_shapes, 7);
        assert_eq!(b.cache_budget, 0);
        assert_eq!(b.pixel_budget, MAX_PIXEL_BYTES, "caches don't draw on the pool images need");
    }

    #[test]
    fn shapes_are_not_compiled_once_no_cache_tile_fits() {
        // Three tiles pay for the first three shapes; the other 27 must not even be compiled.
        let d = page((0..30).map(tiny_shape).collect());
        let mut b = Builder::new("shapes", &d).unwrap();
        b.cache_budget = 3 * 256 * 256 * 4;
        COMPILED_SHAPES.with(|n| n.set(0));
        b.document(&d).unwrap();
        assert_eq!(COMPILED_SHAPES.with(std::cell::Cell::get), 3);
        assert_eq!(b.uncached_shapes, 27, "the skipped-cache warning still counts them");
        assert_eq!(b.doc.layers.len(), 30, "every shape still imports");
    }

    #[test]
    fn an_acute_miter_is_charged_for_the_tiles_it_reaches() {
        // A thin spike at x = 250 with a 4 px miter stroke reaches x = 270, into the next 256 px
        // tile, though the stroke width alone would stay in the first.
        let p = |x: f64, y: f64| af::Point { x, y };
        let sub = af::SubPath {
            start: p(250.0, 100.0),
            segments: [p(200.0, 95.0), p(200.0, 105.0), p(250.0, 100.0)].iter().map(|c| [*c; 3]).collect(),
            closed: true,
        };
        let mut d = page(vec![node(Kind::Shape {
            path: af::Path { subpaths: vec![sub] },
            fills: Vec::new(),
            strokes: vec![ap::Stroke {
                paint: ap::Paint::Solid(ap::Color::Rgb { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                width: 4.0,
                cap: Cap::Butt,
                join: Join::Miter,
                miter_limit: 20.0,
                align: ap::Align::Center,
                dash: None,
                behind: false,
                profile: None,
            }],
            even_odd: false,
        })]);
        d.spreads[0].bounds.x1 = 600.0;
        d.spreads[0].bounds.y1 = 600.0;
        let mut b = Builder::new("miter", &d).unwrap();
        let tile = crate::text_import::cache_bytes(Rect::new(0, 0, 1, 1), b.format.bytes_per_pixel());
        b.cache_budget = tile;
        b.document(&d).unwrap();
        assert_eq!(b.uncached_shapes, 1, "two tiles do not fit a budget of one");
        assert_eq!(b.cache_budget, tile, "nothing was charged");
        let mut b2 = Builder::new("miter", &d).unwrap();
        b2.cache_budget = 2 * tile;
        b2.document(&d).unwrap();
        assert_eq!(b2.uncached_shapes, 0);
        assert_eq!(b2.cache_budget, 0, "charged for both tiles");
    }

    #[test]
    fn a_cancelled_import_stops_building_layers() {
        let d = page((0..10).map(tiny_shape).collect());
        let mut b = Builder::new("cancel", &d).unwrap();
        let cancel = || true;
        b.ctl = photocraft_raster::Interrupt::cancel_only(&cancel);
        b.document(&d).unwrap();
        assert!(b.doc.layers.is_empty(), "no layer is built once cancelled");
        let mut doc = Document::new("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let mut budget = u64::MAX;
        doc.layers.push(Layer::new("t", LayerContent::Text(TextLayer { text: "x".into(), ..TextLayer::default() })));
        assert!(crate::text_import::prepare_within(&mut doc, None, &mut budget, &b.ctl).is_err());
    }

    #[test]
    fn type_layers_share_the_cache_budget_and_keep_going_without_pixels() {
        // Each layer's ink is charged to the budget; one that doesn't fit keeps an empty cache and
        // is counted, instead of every layer of a 2000-layer file drawing its own raster.
        let mut doc = Document::new("t", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        for i in 0..20 {
            let text = "Budget".to_string();
            let mut t = TextLayer { text, font_family: "sans-serif".into(), size_pt: 24.0, ..TextLayer::default() };
            t.runs = vec![TextRun { len: t.text.len(), style: CharStyle { size_pt: 24.0, ..CharStyle::default() } }];
            doc.layers.push(Layer::new(format!("t{i}"), LayerContent::Text(t)));
        }
        // What one layer costs, then room for two and a half.
        let mut probe = u64::MAX;
        let mut one = doc.clone();
        one.layers.truncate(1);
        crate::text_import::prepare_within(&mut one, None, &mut probe, &photocraft_raster::Interrupt::NONE).unwrap();
        let cost = u64::MAX - probe;
        assert!(cost > 0);
        let mut budget = cost * 5 / 2;
        let skipped = crate::text_import::prepare_within(&mut doc, None, &mut budget, &photocraft_raster::Interrupt::NONE).unwrap();
        let drawn = doc.layers.iter().filter(|l| matches!(&l.content, LayerContent::Text(t) if t.cache.as_ref().is_some_and(|c| c.tile_count() > 0))).count();
        assert_eq!((drawn, skipped), (2, 18));
        assert!(drawn + skipped == 20, "drawn {drawn} skipped {skipped}");
        assert!(doc.layers.iter().all(|l| matches!(&l.content, LayerContent::Text(t) if t.cache.is_some())), "every layer has a cache, empty or not");
    }

    #[test]
    fn shape_caches_leave_room_for_images() {
        // 64 shapes overrun the cache pool; the image after them still imports.
        let mut nodes: Vec<af::Node> = (0..64).map(tiny_shape).collect();
        nodes.extend(affinity_document(vec![(64, 64, Pixels::Rgba8(vec![0; 64 * 64 * 4]))]).spreads.remove(0).nodes);
        let d = page(nodes);
        let mut b = Builder::new("mixed", &d).unwrap();
        b.cache_budget = 4 * 256 * 256 * 4;
        b.pixel_budget = 256 * 256 * 4; // one whole tile for the image
        b.document(&d).unwrap();
        assert_eq!(b.uncached_shapes, 60);
        assert!(b.doc.layers.iter().any(|l| matches!(l.content, LayerContent::Smart(_) | LayerContent::Raster(_))), "the image must import");
        assert!(!b.warnings.iter().any(|w| w.contains("pixel memory")), "{:?}", b.warnings);
    }
}
