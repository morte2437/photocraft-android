//! A PNG import in resumable steps, for a caller that must not block for the whole file (the web
//! build has no threads): the same document [`crate::import_with_options_and_interrupt`] gives.
//!
//! A PNG whose rows come in final order is streamed: each decoded group of rows becomes tiles at
//! once and is dropped, as [`crate::import_with_options_and_interrupt`] does, so the image never
//! exists next to the layer built from it (a 100 MP RGBA8 file would need about 400 MB more, which
//! a browser tab does not have). Any other PNG (interlaced, animated, or stored in another
//! orientation) is decoded whole, turned upright and copied into tiles, each in bounded steps.

use std::sync::Arc;

use photocraft_codecs::{self as codecs, DecodeOptions, Format, Limits, OrientSteps, PngDecodeRows, PngExtras, PngFinish, SharedBytes, begin_png_finish};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{Interrupt, PruneCursor};

use crate::flat::{FlatBuilder, PngStream, apply_flat_format};
use crate::{ImportResult, IoError, Route};

/// The rows of a file being streamed into the tiles of its layer.
struct Streaming {
    rows: PngDecodeRows<SharedBytes>,
    stream: PngStream,
}

/// A streamed layer being scanned for tiles that hold only the default pixel.
struct Pruning {
    stream: PngStream,
    cursor: PruneCursor,
    extras: PngExtras,
}

enum Stage {
    /// The rows becoming tiles as they are decoded.
    Stream(Box<Streaming>),
    /// The streamed layer's prune scan.
    Prune(Box<Pruning>),
    /// The whole image, decoded a row at a time: a file whose rows can't be streamed.
    Rows(Box<PngDecodeRows<SharedBytes>>),
    /// The decoded image turned upright (an EXIF orientation 2-8), a band of rows at a time.
    Orient(Box<OrientSteps>),
    /// The decoded image becoming the document, a band of rows and a batch of tiles at a time.
    Build(Box<FlatBuilder>),
    Spent,
}

/// A plain PNG file being imported as a document, a few rows at a time.
pub struct PngImport {
    name: String,
    bytes: Arc<Vec<u8>>,
    stage: Stage,
}

impl PngImport {
    /// Starts importing `bytes` (named `name`) when the import would decode them as a plain PNG,
    /// which reads the header and refuses what the import refuses; `None` for any other file,
    /// which only [`crate::import_with_options_and_interrupt`] reads. The decoder shares `bytes`.
    pub fn new(name: &str, bytes: &Arc<Vec<u8>>) -> Result<Option<Self>, IoError> {
        // A native bundle suffix must never start the PNG streaming path, even when the bytes
        // are a PNG (for example, an invalid `.pcraft` file). The ordinary importer still uses
        // content detection and can open a PNG copied to a bare name.
        let extension = name.rsplit(['/', '\\']).next().and_then(|file| file.rsplit_once('.')).map(|(_, ext)| ext);
        if extension.is_some_and(|ext| photocraft_formats::from_extension(ext) == Some(photocraft_formats::Format::Pcraft)) {
            return Ok(None);
        }
        if crate::route(name, bytes) != Route::Flat || codecs::detect(bytes) != Some(Format::Png) {
            return Ok(None);
        }
        let rows = PngDecodeRows::new_streaming(SharedBytes(Arc::clone(bytes)), &Limits::default())?;
        let stage = if rows.streamable() {
            let stream = PngStream::for_steps(&rows.header())?;
            Stage::Stream(Box::new(Streaming { rows, stream }))
        } else {
            Stage::Rows(Box::new(Self::buffered(rows)?))
        };
        Ok(Some(Self { name: name.to_string(), bytes: Arc::clone(bytes), stage }))
    }

    /// A decoder that holds the whole image: its output is reserved up front.
    fn buffered(mut rows: PngDecodeRows<SharedBytes>) -> Result<PngDecodeRows<SharedBytes>, IoError> {
        rows.reserve_output()?;
        Ok(rows)
    }

    /// Whether the rows are streamed into the layer's tiles as they are decoded, so the decoded
    /// image never exists whole. False for a file whose rows can't come in final order, and from
    /// the moment a late `eXIf` sends a streamed one back to a whole decode.
    pub fn streams(&self) -> bool {
        matches!(self.stage, Stage::Stream(_) | Stage::Prune(_))
    }

    /// How much of the import is done, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        match &self.stage {
            Stage::Stream(s) => s.rows.progress(),
            Stage::Rows(rows) => rows.progress(),
            Stage::Orient(steps) => steps.progress(),
            Stage::Prune(_) | Stage::Build(_) | Stage::Spent => 1.0,
        }
    }

    /// Works while `more()` returns true (at least one bounded piece per call): rows (into tiles
    /// as they come, or into the whole image), the turn upright, the copy into tiles, then the
    /// prune scan. `Some` once the document is built, after which the import is spent.
    pub fn step(&mut self, more: &mut dyn FnMut() -> bool) -> Result<Option<ImportResult>, IoError> {
        match std::mem::replace(&mut self.stage, Stage::Spent) {
            Stage::Stream(mut s) => {
                let result = {
                    let Streaming { rows, stream } = &mut *s;
                    let mut sink = |y0: u32, data: &[u8]| stream.write_rows(y0, data, &Interrupt::NONE);
                    rows.step_stream::<IoError>(TILE_SIZE as u32, more, &mut sink)?
                };
                self.stage = match result {
                    None => Stage::Stream(s),
                    // An `eXIf` after the image data turns the image: decode it whole instead, as
                    // the one-shot import does, with the streamed layer released first.
                    Some(extras) if extras.orientation != 1 => {
                        drop(s);
                        let rows = PngDecodeRows::new_streaming(SharedBytes(Arc::clone(&self.bytes)), &Limits::default())?;
                        Stage::Rows(Box::new(Self::buffered(rows)?))
                    }
                    Some(extras) => {
                        let stream = s.stream;
                        let cursor = stream.prune_cursor();
                        Stage::Prune(Box::new(Pruning { stream, cursor, extras }))
                    }
                };
                Ok(None)
            }
            Stage::Prune(mut p) => loop {
                if p.stream.prune_step(&mut p.cursor) {
                    let Pruning { stream, extras, .. } = *p;
                    let mut r = stream.into_document(&self.name, extras)?;
                    apply_flat_format(&mut r, &self.bytes, Some(Format::Png), None)?;
                    return Ok(Some(r));
                }
                if !more() {
                    self.stage = Stage::Prune(p);
                    return Ok(None);
                }
            },
            Stage::Rows(mut rows) => {
                self.stage = match rows.step(more)? {
                    Some(img) => {
                        // The resolution, upright orientation and final limit checks `decode` does.
                        match begin_png_finish(img, &DecodeOptions::default())? {
                            PngFinish::Done(img) => Stage::Build(Box::new(FlatBuilder::new(&self.name, *img)?)),
                            PngFinish::Orient(steps) => Stage::Orient(steps),
                        }
                    }
                    None => Stage::Rows(rows),
                };
                Ok(None)
            }
            Stage::Orient(mut steps) => {
                self.stage = match steps.step(more)? {
                    Some(img) => Stage::Build(Box::new(FlatBuilder::new(&self.name, img)?)),
                    None => Stage::Orient(steps),
                };
                Ok(None)
            }
            Stage::Build(mut build) => loop {
                if !build.copy_done() {
                    build.copy_step()?;
                    if !more() {
                        self.stage = Stage::Build(build);
                        return Ok(None);
                    }
                    continue;
                }
                if !build.prune_done() {
                    build.prune_step();
                    if !more() {
                        self.stage = Stage::Build(build);
                        return Ok(None);
                    }
                    continue;
                }
                return build.finish(self.bytes.as_slice(), Some(Format::Png)).map(Some);
            },
            Stage::Spent => Err(IoError::Unsupported("the PNG was already imported".into())),
        }
    }
}
