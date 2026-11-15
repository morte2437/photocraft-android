//! Place in resumable steps (#10031): decode, flatten, resize and translate, each in pieces small
//! enough to run between two frames where there are no worker threads (the web build).
//!
//! Every piece computes what the one-shot code computes, so the placed layer is the same whether
//! a worker thread, a [`PlaceSteps`] driven to the end, or a frame-by-frame [`Session::poll_jobs`]
//! did the work. `fine` (a cooperative session) cuts the work into pieces; without it each stage
//! is one unit and native code keeps its parallel resize and flatten.

use photocraft_algo::resample::ResizeSteps;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_io::PngImport;
use photocraft_raster::{Surface, TranslateSteps};
use web_time::Instant;

use super::open::{PLACE_EMBEDDED, PlaceFile, PlaceInput, Prepared};
use super::paths::{file_name, read_file, stem};
use super::{
    Affine, Arc, Document, EngineError, LayerContent, Resample, Result, SmartObject, SmartSource, Value, check_flattenable, f64_param, flattened_checked,
    resize_surface, translate_surface,
};
use crate::jobs::{JobCtx, StepWork};

/// Pixels a flatten band covers at most: four tiles of a tile row (about 5 ms on one core).
const BAND_PIXELS: i64 = 4 * (TILE_SIZE as i64) * (TILE_SIZE as i64);

/// A place as a [`StepWork`]; its result is what [`super::open::apply_place`] inserts.
pub(crate) struct PlaceSteps {
    input: PlaceInput,
    /// The file's contents, once read (a path is read by the first step).
    bytes: Arc<Vec<u8>>,
    /// Cut the work into small pieces (for a session without threads).
    fine: bool,
    stage: Stage,
    /// Where the placed image lands, known once it is decoded.
    plan: Plan,
}

#[derive(Clone, Copy, Default)]
struct Plan {
    w: f64,
    h: f64,
    scale: f64,
    dx: f64,
    dy: f64,
}

enum Stage {
    /// Nothing done yet.
    Decode,
    /// A plain PNG, a few rows at a time.
    DecodePng(Box<PngImport>),
    Flatten(Box<Flatten>),
    Resize(Box<ResizeSteps>),
    /// The fitted pixels being moved to where they land, a few tiles at a time.
    Translate(Box<TranslateSteps>),
    /// The pixels in place (none for a frame's image, which renders from its source), to wrap in
    /// a layer.
    Finish(Option<Surface>),
    Spent,
}

impl PlaceSteps {
    /// `fine` cuts the work into pieces of a few milliseconds, as a session without worker
    /// threads needs; without it each stage is one unit of work.
    pub(crate) fn new(input: PlaceInput, fine: bool) -> Self {
        Self { input, bytes: Arc::new(Vec::new()), fine, stage: Stage::Decode, plan: Plan::default() }
    }

    /// An import error as the engine reports it: a cancel stays one and a refused memory budget
    /// keeps its typed fields.
    fn io_error(&self, e: photocraft_io::IoError) -> EngineError {
        match e {
            photocraft_io::IoError::Cancelled => EngineError::Cancelled,
            // Typed, as the one-shot place reports it: a refused memory budget keeps its fields.
            e => crate::error::io_open_error(&self.input.name, e),
        }
    }

    /// One unit of work; `Some` when the place is prepared.
    fn advance(&mut self, ctx: &JobCtx, deadline: Instant) -> Result<Option<Prepared>> {
        match std::mem::replace(&mut self.stage, Stage::Spent) {
            Stage::Decode => {
                self.read(ctx)?;
                let input = &self.input;
                let bytes = &self.bytes;
                let png = if self.fine { PngImport::new(&input.name, bytes).map_err(|e| self.io_error(e))? } else { None };
                match png {
                    Some(png) => self.stage = Stage::DecodePng(Box::new(png)),
                    None => {
                        let imported = ctx
                            .stage(0.0, 0.6, "Decoding", |ctl| photocraft_io::import_with_options_and_interrupt(&input.name, bytes, &input.opts, ctl))
                            .map_err(|e| self.io_error(e))?;
                        self.decoded(imported, false, ctx)?;
                    }
                }
            }
            Stage::DecodePng(mut png) => {
                ctx.progress(0.55 * png.progress(), "Decoding");
                match png.step(&mut || Instant::now() < deadline).map_err(|e| self.io_error(e))? {
                    Some(imported) => self.decoded(imported, true, ctx)?,
                    None => self.stage = Stage::DecodePng(png),
                }
            }
            Stage::Flatten(mut flatten) => {
                ctx.progress(0.6 + 0.1 * flatten.progress(), "Flattening");
                match flatten.advance()? {
                    Some(px) => self.flattened(px, ctx),
                    None => self.stage = Stage::Flatten(flatten),
                }
            }
            Stage::Resize(mut resize) => {
                ctx.progress(0.7 + 0.2 * resize.progress(), "Resizing");
                match resize.step(&mut || Instant::now() < deadline) {
                    Some(px) => self.fitted(px),
                    None => self.stage = Stage::Resize(resize),
                }
            }
            Stage::Translate(mut shift) => {
                ctx.progress(0.9 + 0.05 * shift.progress(), "Placing");
                self.stage = match shift.step(&mut || Instant::now() < deadline) {
                    Some(px) => Stage::Finish(Some(px)),
                    None => Stage::Translate(shift),
                };
            }
            Stage::Finish(px) => return self.finish(px, ctx).map(Some),
            Stage::Spent => return Err(EngineError::Other("internal error: the place was already prepared".into())),
        }
        Ok(None)
    }

    /// Gets the file's bytes (reading a path) and refuses a place that cannot fit in memory,
    /// before anything is decoded.
    fn read(&mut self, ctx: &JobCtx) -> Result<()> {
        self.bytes = match &self.input.file {
            PlaceFile::Bytes(bytes) => Arc::clone(bytes),
            PlaceFile::Path(path) => {
                ctx.progress(0.0, "Reading");
                let bytes = read_file(path)?;
                ctx.check()?;
                Arc::new(bytes)
            }
        };
        let (name, fmt) = (&self.input.name, self.input.fmt);
        crate::memory_gate::ensure(&format!("place \u{201c}{}\u{201d}", file_name(name)), crate::memory_gate::place_need(&self.bytes, fmt.bytes_per_pixel()))?;
        // A path isn't read when the input is made, so whether it is a vector source is decided now.
        let vector = self.input.frame.is_some() || photocraft_io::is_svg_input(name, &self.bytes);
        self.input.doc = self.input.doc.take().filter(|_| vector);
        Ok(())
    }

    /// The file is decoded: where it lands, then on to flattening. `banded`: the document is a
    /// plain image, which flattens band by band.
    fn decoded(&mut self, imported: photocraft_io::ImportResult, banded: bool, ctx: &JobCtx) -> Result<()> {
        if imported.preview_only {
            return Err(EngineError::Other(format!(
                "{}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here",
                self.input.name
            )));
        }
        let src = imported.document;
        let (w, h) = (src.size.width as f64, src.size.height as f64);
        let (cw, ch) = self.input.canvas;
        let p = &self.input.params;
        let scale = match f64_param(p, "scale") {
            Some(k) => (k / 100.0).max(1e-4),
            None if self.input.frame.is_some() && self.input.fit => (cw / w).max(ch / h),
            None if self.input.fit && (w > cw || h > ch) => (cw / w).min(ch / h),
            None => 1.0,
        };
        let center = match p.get("center").and_then(Value::as_array) {
            Some(c) if c.len() == 2 => (c[0].as_f64().unwrap_or(cw / 2.0), c[1].as_f64().unwrap_or(ch / 2.0)),
            _ => (cw / 2.0, ch / 2.0),
        };
        // `scale` is the caller's: bound the resampled size before allocating it (a frame's image
        // renders through its smart object, which has its own budget). Resampling rounds the far
        // output edges outward; an unresampled placement keeps its size.
        if self.input.frame.is_none() {
            let (ow, oh) = if (scale - 1.0).abs() > 1e-9 { ((w * scale).ceil(), (h * scale).ceil()) } else { (w, h) };
            let scaled = ow * oh;
            if scaled.is_nan() || scaled > photocraft_compose::MAX_RENDER_PIXELS as f64 {
                return Err(EngineError::BadParams {
                    cmd: self.input.command().into(),
                    msg: format!("scaling {w}×{h} by {scale} makes an image too large to place"),
                });
            }
        }
        let (dx, dy) = ((center.0 - w * scale / 2.0).round(), (center.1 - h * scale / 2.0).round());
        self.plan = Plan { w, h, scale, dx, dy };
        // A frame's image renders through its smart object, which refuses a placement past the
        // memory budget; resampling it to the frame's size first would allocate that surface twice.
        if self.input.frame.is_some() {
            self.stage = Stage::Finish(None);
            return Ok(());
        }
        ctx.progress(0.6, "Flattening");
        self.stage = Stage::Flatten(Box::new(Flatten::new(src, banded && self.fine, self.input.fmt, self.input.command())?));
        Ok(())
    }

    /// The image is flattened: on to resizing it to fit, if it needs that.
    fn flattened(&mut self, px: Surface, ctx: &JobCtx) {
        let scale = self.plan.scale;
        if (scale - 1.0).abs() <= 1e-9 {
            self.fitted(px);
        } else if self.fine {
            self.stage = Stage::Resize(Box::new(ResizeSteps::new(px, scale, scale, Resample::Bicubic, None)));
        } else {
            ctx.progress(0.7, "Resizing");
            self.fitted(resize_surface(&px, scale, scale, Resample::Bicubic));
        }
    }

    /// The pixels are fitted: on to moving them to where they land. A cooperative session does
    /// that a few target tiles at a time (an offset off the tile grid rebuilds nearly every tile
    /// the image touches); otherwise it is one [`translate_surface`].
    fn fitted(&mut self, px: Surface) {
        let (dx, dy) = (self.plan.dx as i32, self.plan.dy as i32);
        self.stage = if self.fine {
            let content = px.content_bounds();
            Stage::Translate(Box::new(TranslateSteps::new(px, dx, dy, content)))
        } else {
            Stage::Finish(Some(translate_surface(&px, dx, dy)))
        };
    }

    /// Wraps the pixels, already in place, in the layer's content.
    fn finish(&mut self, px: Option<Surface>, ctx: &JobCtx) -> Result<Prepared> {
        let Plan { w, h, scale, dx, dy } = self.plan;
        let input = &self.input;
        let (cw, ch) = input.canvas;
        let placement = input.frame.as_ref().map_or(Affine { m: [scale, 0.0, 0.0, scale, dx, dy] }, |(_, f)| {
            f.transform.mul(&Affine { m: [scale / cw, 0.0, 0.0, scale / ch, dx / cw, dy / ch] })
        });
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| placement.apply(photocraft_geom::Point::new(x, y)));
        let bounds = [
            corners.iter().map(|q| q.x).fold(f64::INFINITY, f64::min),
            corners.iter().map(|q| q.y).fold(f64::INFINITY, f64::min),
            corners.iter().map(|q| q.x).fold(f64::NEG_INFINITY, f64::max),
            corners.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max),
        ];
        let label = if input.linked.is_some() { "Place Linked" } else { PLACE_EMBEDDED };
        // A frame's image is always a smart object.
        let smart = input.smart || input.frame.is_some();
        let content = if smart {
            let source = match &input.linked {
                Some(path) => SmartSource::Linked { path: path.clone() },
                None => SmartSource::Embedded { file_name: file_name(&input.name), bytes: Arc::clone(&self.bytes) },
            };
            let mut so = SmartObject::new(source, placement, px);
            // A vector source (SVG) or a frame's image renders at its placement scale instead of
            // as a resampled raster.
            if let Some(doc) = &input.doc {
                ctx.progress(0.9, "Rendering");
                ctx.check()?;
                if let Some(sharp) = crate::smart_cmds::render(doc, &so)? {
                    so.cache = Some(sharp);
                }
            }
            if so.cache.is_none() {
                return Err(EngineError::Other(format!("{}: the placed image could not be rendered", input.name)));
            }
            LayerContent::Smart(so)
        } else {
            let px = px.ok_or_else(|| EngineError::Other(format!("{}: a frame's image must be a smart object", input.name)))?;
            LayerContent::Raster(px)
        };
        Ok(Prepared { content, layer_name: stem(&input.name), label, scale, bounds, smart, frame: input.frame.as_ref().map(|(id, _)| *id) })
    }
}

impl StepWork<Prepared> for PlaceSteps {
    /// Runs units of work until `deadline` (at least one), checking for a cancel between them.
    fn step(&mut self, ctx: &JobCtx, deadline: Instant) -> Result<Option<Prepared>> {
        loop {
            ctx.check()?;
            if let Some(prepared) = self.advance(ctx, deadline)? {
                return Ok(Some(prepared));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

/// The decoded document being flattened to the surface it is resized from.
pub(super) struct Flatten {
    doc: Document,
    fmt: photocraft_color::PixelFormat,
    /// The place command, for the size error.
    command: &'static str,
    /// Flatten band by band; otherwise in one go.
    banded: bool,
    bounds: Rect,
    /// The next band's top left corner.
    at: (i32, i32),
    /// Width of a band, a whole number of tiles.
    cols: i32,
    out: Option<Surface>,
}

impl Flatten {
    /// `command` is the place command (`file.placeEmbedded` or `file.placeLinked`), named in the
    /// error a canvas past the compositor's limit gives.
    pub(super) fn new(doc: Document, banded: bool, fmt: photocraft_color::PixelFormat, command: &'static str) -> Result<Self> {
        check_flattenable(&doc, command)?;
        let bounds = doc.bounds();
        // Bands hold whole tiles, so each tile is written by one band and can move as it is. A
        // document that does not start on the tile grid (none does) flattens in one go.
        let banded = banded && bounds.x0.rem_euclid(TILE_SIZE) == 0 && bounds.y0.rem_euclid(TILE_SIZE) == 0;
        let cols = i32::try_from(BAND_PIXELS / i64::from(TILE_SIZE) / i64::from(TILE_SIZE)).unwrap_or(1).max(1) * TILE_SIZE;
        Ok(Self { doc, fmt, command, banded, bounds, at: (bounds.x0, bounds.y0), cols, out: None })
    }

    /// How much of the flatten is done, `0.0..=1.0`.
    fn progress(&self) -> f32 {
        let rows = i64::from(self.bounds.height()).max(1);
        (i64::from(self.at.1.saturating_sub(self.bounds.y0)) as f32 / rows as f32).clamp(0.0, 1.0)
    }

    /// Flattens the next band (the whole document if not banded); `Some` with the surface once
    /// the last one is in.
    pub(super) fn advance(&mut self) -> Result<Option<Surface>> {
        if !self.banded {
            return flattened_checked(&self.doc, self.fmt, self.command).map(Some);
        }
        let (x, y) = self.at;
        // A band is part of one tile row, at most `cols` wide.
        let y1 = (y.div_euclid(TILE_SIZE).saturating_add(1).saturating_mul(TILE_SIZE)).min(self.bounds.y1);
        let x1 = x.saturating_add(self.cols).min(self.bounds.x1);
        let band = photocraft_compose::composite_to_surface(&self.doc, Rect::new(x, y, x1, y1), self.fmt, None);
        // The band's tiles move into the result as they are, with no copy of their pixels.
        let out = self.out.get_or_insert_with(|| Surface::with_default(self.fmt, &band.default_pixel()));
        out.put_tiles(band.tiles().map(|(c, t)| (*c, Arc::clone(t))));
        self.at = if x1 >= self.bounds.x1 { (self.bounds.x0, y1) } else { (x1, y) };
        Ok(if self.at.1 >= self.bounds.y1 { Some(self.out.take().unwrap_or_else(|| Surface::new(self.fmt))) } else { None })
    }
}
