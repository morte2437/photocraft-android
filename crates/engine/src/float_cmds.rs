//! Floating selection: the selected pixels of a layer cut out and moved as a piece, without
//! touching the document until the piece is dropped.
//!
//! - `select.float {"dx","dy","copy"}` cuts the selected pixels of the active layer into a floating
//!   piece (the first time; `copy` leaves them and floats a duplicate) and moves it by whole
//!   pixels. Further calls move the same piece; nothing new is cut. The Move tool, the marquee and
//!   lasso ⌘-drags and the Move tool's arrow keys all move selected pixels this way.
//! - `select.drop` drops the piece into its layer and moves the selection with it, as one history
//!   step.
//! - Any other command drops the piece first (so it lands exactly where it was shown), except Undo,
//!   which puts it back where it was cut (nothing was changed yet), and `history.selectState`,
//!   which puts it back and then returns to the chosen state.
//!
//! While floating, the document is unchanged; [`displayed`] gives the document with the piece at
//! its offset, for the canvas. The selection stays where it was until the drop, when it becomes
//! the piece's outline ([`CutParts::outline`]); the canvas shows that outline from the start.

use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

/// A layer split by its selection: the layer with the selected pixels cut out, and the cut pixels
/// (cropped to the selection). Splitting touches only the selection's area, and placing the piece
/// ([`CutParts::moved`]) only the piece's: the cut-out layer's other tiles are shared.
#[derive(Clone, Debug)]
pub struct CutParts {
    pub layer: LayerId,
    rest: Surface,
    piece: Surface,
    piece_bounds: photocraft_geom::Rect,
    /// The selection the moved pixels take, unmoved: the alpha the piece took, the selection
    /// limited to the layer's transparency (Photoshop shrinks the marching ants to the piece as the
    /// drag starts, #7538). `None` keeps the selection: a mask has no transparency, and an empty
    /// piece no outline.
    outline: Option<Surface>,
    /// The pieces are the layer's mask (`"target":"mask"`), not its pixels.
    mask: bool,
}

impl CutParts {
    /// Split `layer` of `doc` by its selection. With `copy` the selected pixels stay in the layer
    /// too (the piece is a duplicate). Without, they leave a transparent hole, or on the Background,
    /// which can't hold transparency, one filled with `background` (Photoshop). Errors without a
    /// selection or pixels. With `mask` the layer's mask is split instead of its pixels; the hole it
    /// leaves takes the background colour (as on the Background layer).
    pub fn new(doc: &Document, layer: LayerId, copy: bool, background: [f32; 4], mask: bool) -> Result<Self> {
        let sel = doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
        let l = doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
        let surf = if mask {
            &l.mask.as_ref().ok_or_else(|| EngineError::Other("the layer has no mask".into()))?.surface
        } else {
            let LayerContent::Raster(surf) = &l.content else {
                return Err(EngineError::Other("the layer has no pixels to move".into()));
            };
            surf
        };
        let fmt = surf.format();
        let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
        // A mask has no alpha: it keeps its format, and the cut leaves background colour.
        let base = if mask || fmt == with_alpha { surf.clone() } else { surf.convert(with_alpha) };
        let mut rest = base.clone();
        let mut piece = Surface::new(with_alpha);
        let mut outline: Option<Surface> = None;
        let fill = !copy && (mask || doc.is_background(layer));
        // A mask's untouched pixels (its default, e.g. white) move too, so take the whole canvas.
        let b = sel.content_bounds().intersect(&if mask { doc.bounds() } else { base.content_bounds() });
        if !b.is_empty() {
            piece = cut_piece(&base, sel, b, with_alpha)?;
            // The moved pixels' selection is what they took with them: the alpha the piece
            // holds, which is the selection limited to the layer's own coverage (Photoshop's
            // marching ants hug the piece as the drag starts, #7538). A mask has no outline.
            if !mask {
                let n = with_alpha.channels();
                let cover: Vec<f32> = piece.try_read_region(b)?.chunks_exact(n).filter_map(|p| p.last().copied()).collect();
                let mut o = Surface::new(PixelFormat::new(ColorMode::Grayscale, sel.format().sample, false));
                o.try_write_region(b, &cover)?;
                o.prune();
                outline = Some(o);
            }
            if fill {
                let opaque_bg = [background[0], background[1], background[2], 1.0];
                crate::pixels::try_fill_surface(&mut rest, b, opaque_bg, Some(sel), false)?;
                if mask {
                    rest.prune();
                }
            } else if !copy && !mask {
                crate::pixels::try_lift_coverage(&mut rest, b, sel)?;
            }
        }
        let piece_bounds = piece.content_bounds();
        let outline = outline.filter(|o| !o.content_bounds().is_empty());
        Ok(Self { layer, rest, piece, piece_bounds, outline, mask })
    }

    /// The selection the moved pixels take, where they were cut (`None`: the selection stays).
    pub fn outline(&self) -> Option<&Surface> {
        self.outline.as_ref()
    }

    /// `doc` with the piece moved by whole pixels (dx, dy) over the cut-out layer.
    pub fn moved(&self, doc: &Document, dx: i32, dy: i32) -> Result<Document> {
        let mut d = doc.clone();
        self.place(&mut d, dx, dy)?;
        Ok(d)
    }

    /// Put the piece, moved by (dx, dy), over the cut-out layer (or mask) of `doc`.
    fn place(&self, doc: &mut Document, dx: i32, dy: i32) -> Result<()> {
        let l = doc.layer_mut(self.layer).ok_or(EngineError::NoLayer(self.layer))?;
        let surf = if self.mask {
            l.mask.as_mut().map(|m| &mut m.surface).ok_or_else(|| EngineError::Other("the layer has no mask".into()))?
        } else {
            l.surface_mut().ok_or_else(|| EngineError::Other("the layer has no pixels to move".into()))?
        };
        if self.piece_bounds.is_empty() {
            *surf = self.rest.clone();
            return Ok(());
        }
        let mut out = self.rest.clone();
        let piece = self.piece.translated(dx, dy, self.piece_bounds);
        if self.mask {
            crate::fill_cmds::composite_over(&mut out, &piece, self.piece_bounds.translate(dx, dy), None)?;
        } else {
            crate::transform_cmds::composite_over(&mut out, &piece);
        }
        *surf = out;
        Ok(())
    }
}

/// Extract the selected pixels of `base` over `b` into a `with_alpha` surface, tile by tile in
/// parallel. Fully selected tiles on an alpha surface share `base`'s `Arc<Tile>` directly.
fn cut_piece(base: &Surface, sel: &Surface, b: photocraft_geom::Rect, with_alpha: PixelFormat) -> Result<Surface> {
    use photocraft_color::{SampleType, read_sample};
    use photocraft_geom::{TILE_SIZE, TileCoord};
    use photocraft_raster::Tile;
    use rayon::prelude::*;

    let mut piece = Surface::new(with_alpha);
    let src_fmt = base.format();
    let same_fmt = src_fmt == with_alpha;
    let n = with_alpha.channels();
    let a = n - 1;
    let dst_bpp = with_alpha.bytes_per_pixel();
    let src_bpp = src_fmt.bytes_per_pixel();
    let sb = with_alpha.sample.bytes();
    let alpha_off = dst_bpp - sb;
    let src_empty_default = same_fmt && base.default_bytes().iter().all(|&v| v == 0);
    let oom = |e| EngineError::Other(format!("not enough memory while floating selection ({e}); the document was not changed"));
    let mut coords = crate::allocation::capacity(b.tiles().count(), "floating selection")?;
    coords.extend(b.tiles());
    let tiles = coords
        .into_par_iter()
        .map(|tc| -> Result<Option<(TileCoord, Arc<Tile>)>> {
            let tr = tc.rect().intersect(&b);
            let mask = crate::fill_cmds::Mask::of(sel, tc);
            if mask.empty(tr) {
                return Ok(None);
            }
            let src_tile = base.tile(tc);
            if same_fmt && src_tile.is_none() && src_empty_default {
                return Ok(None);
            }
            if same_fmt
                && tr == tc.rect()
                && mask.full(tr)
                && let Some(t) = src_tile
            {
                return Ok(Some((tc, t.clone())));
            }
            let src_bytes = src_tile.map_or(base.default_bytes(), |t| t.bytes());
            let mut out = piece.try_default_tile().map_err(oom)?;
            let out_bytes = out.bytes_mut();
            let mut px = [0.0f32; 8];
            let mut has_content = false;
            for y in tr.y0..tr.y1 {
                let row = (y - tc.ty * TILE_SIZE) as usize * TILE_SIZE as usize;
                for x in tr.x0..tr.x1 {
                    let k = mask.at(x, y);
                    if k <= 0.0 {
                        continue;
                    }
                    let idx = row + (x - tc.tx * TILE_SIZE) as usize;
                    let dst_i = idx * dst_bpp;
                    let Some(dst_px) = out_bytes.get_mut(dst_i..dst_i + dst_bpp) else { continue };
                    let src_i = if src_bytes.len() == src_bpp { 0 } else { idx * src_bpp };
                    let Some(src_px) = src_bytes.get(src_i..src_i + src_bpp) else { continue };
                    if same_fmt && k >= 1.0 {
                        if src_px.iter().any(|&v| v != 0) {
                            dst_px.copy_from_slice(src_px);
                            has_content = true;
                        }
                        continue;
                    }
                    if same_fmt {
                        for (c, v) in px.iter_mut().enumerate().take(n) {
                            *v = read_sample(src_px, src_fmt.sample, c);
                        }
                        // The selection lifts its share of a pixel's coverage, up to all of it
                        // (Photoshop): a layer's own transparency, loaded as the selection, lifts
                        // it whole (#7655). An opaque pixel splits as k and 1 − k.
                        px[a] = px[a].min(k);
                    } else {
                        for (c, v) in px.iter_mut().enumerate().take(a) {
                            *v = read_sample(src_px, src_fmt.sample, c);
                        }
                        px[a] = k;
                    }
                    if px[a] <= 0.0 {
                        continue;
                    }
                    photocraft_raster::encode_pixel(&with_alpha, &px[..n], dst_px);
                    let non_zero = match with_alpha.sample {
                        SampleType::U8 => dst_px.get(alpha_off) != Some(&0) || dst_px.iter().any(|&v| v != 0),
                        SampleType::U16 => dst_px.get(alpha_off..dst_bpp) != Some(&[0, 0]) || dst_px.iter().any(|&v| v != 0),
                        SampleType::F32 => dst_px.iter().any(|&v| v != 0),
                    };
                    has_content |= non_zero;
                }
            }
            Ok(has_content.then(|| (tc, Arc::new(out))))
        })
        .collect::<Result<Vec<_>>>()?;
    piece.put_tiles(tiles.into_iter().flatten());
    Ok(piece)
}

/// A piece floating `offset` whole pixels from where it was cut, over document `revision`.
#[derive(Clone, Debug)]
pub struct Floating {
    pub layer: LayerId,
    pub offset: (i32, i32),
    pub revision: u64,
    pub parts: Arc<CutParts>,
    /// The piece has been moved at least once, even if it is back where it was cut (nudged away
    /// and back): its selection is the moved pixels' then, as each of Photoshop's nudges leaves it.
    pub moved: bool,
}

/// The floating piece of `st`, if it is still valid (the document hasn't changed under it).
pub fn floating(st: &DocState) -> Option<&Floating> {
    st.floating.as_ref().filter(|f| f.revision == st.revision && st.doc.selection.is_some())
}

/// The document as shown while a piece floats `extra` further than its offset (a drag of it in
/// progress); `None` when nothing floats.
pub fn displayed(st: &DocState, extra: (i32, i32)) -> Option<Document> {
    let f = floating(st)?;
    f.parts.moved(&st.doc, f.offset.0 + extra.0, f.offset.1 + extra.1).ok()
}

fn int_param(p: &Value, key: &str) -> i32 {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(0, |v| v.round().clamp(-1e7, 1e7) as i32)
}

fn can_float(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document")?;
    if floating(st).is_some() {
        return Ok(());
    }
    if st.doc.selection.is_none() {
        return Err("there is no selection".into());
    }
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    if !matches!(l.content, LayerContent::Raster(_)) {
        return Err("the active layer has no pixels to move".into());
    }
    if locked_for_float(&st.doc, l) {
        return Err(format!("layer \"{}\" is locked", l.name));
    }
    Ok(())
}

/// Do `l`'s locks (its own, or a locked group's: `effective_locks`) stop its selected pixels
/// floating? The Background is position-locked, but its selected pixels move (Photoshop).
pub fn locked_for_float(doc: &Document, l: &photocraft_doc::Layer) -> bool {
    let locks = doc.effective_locks(l.id);
    locks.all || (locks.position && !doc.is_background(l.id))
}

fn float(s: &mut Session, p: &Value) -> Result<Value> {
    let (dx, dy) = (int_param(p, "dx"), int_param(p, "dy"));
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let background = s.tools.background;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if floating(st).is_none() {
        let layer = st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let parts = CutParts::new(&st.doc, layer, copy, background, crate::commands::is_mask_target(p))?;
        st.floating = Some(Floating { layer, offset: (0, 0), revision: st.revision, parts: Arc::new(parts), moved: false });
    }
    let f = st.floating.as_mut().ok_or(EngineError::NoDocument)?;
    f.offset = (f.offset.0.saturating_add(dx), f.offset.1.saturating_add(dy));
    f.moved |= (dx, dy) != (0, 0);
    Ok(json!({"layer": f.layer.0, "offset": [f.offset.0, f.offset.1]}))
}

/// Drop the floating piece into its layer and move the selection with it, shrunk to the piece's
/// outline (one history step).
pub fn drop_floating(s: &mut Session) -> Result<Value> {
    let Some(st) = s.active_mut() else { return Ok(json!({"dropped": false})) };
    let Some(f) = floating(st).cloned() else {
        st.floating = None;
        return Ok(json!({"dropped": false}));
    };
    st.floating = None;
    let (dx, dy) = f.offset;
    if (dx, dy) == (0, 0) {
        // Back where it was cut: the pixels are as they were. A piece that never moved (a click)
        // changes nothing; one nudged away and back keeps the moved pixels' selection (#7538).
        if let Some(outline) = f.parts.outline().filter(|_| f.moved) {
            let outline = outline.clone();
            s.edit("Move", |doc, _| {
                doc.selection = Some(outline);
                Ok(())
            })?;
        }
        return Ok(json!({"dropped": true, "offset": [0, 0]}));
    }
    s.edit("Move", |doc, _| {
        f.parts.place(doc, dx, dy)?;
        let sel = f.parts.outline().or(doc.selection.as_ref());
        doc.selection = sel.map(|sel| photocraft_algo::resample::translate_surface(sel, dx, dy));
        Ok(())
    })?;
    Ok(json!({"dropped": true, "offset": [dx, dy]}))
}

/// Commands that leave a floating piece floating: its own, and ones that don't touch documents.
fn keeps_floating(id: &str) -> bool {
    matches!(id, "select.float" | "select.drop") || ["view.", "window.", "help.", "prefs."].iter().any(|p| id.starts_with(p))
}

/// Before any command (`Session::dispatch`): Undo puts a floating piece back (and is used up:
/// returns its result); every other command but the floating ones drops it first.
pub(crate) fn before_command(s: &mut Session, id: &str) -> Result<Option<Value>> {
    let Some(st) = s.active_mut() else { return Ok(None) };
    if floating(st).is_none() {
        st.floating = None;
        return Ok(None);
    }
    match id {
        // Edit › Undo and the History panel's Step Backward step back over the move itself.
        "edit.undo" | "history.stepBackward" => {
            st.floating = None;
            Ok(Some(json!({"floating": "returned"})))
        }
        // Redo / Step Forward with nothing to step to leave the piece floating (and are refused).
        "edit.redo" | "history.stepForward" if !st.history.can_redo() => Ok(None),
        // Returning to a History state, or stepping forward to one, puts the piece back too (it
        // never reached the document), and then moves the history: dropping it first would add
        // a step and discard the redo states the step is heading for.
        "history.selectState" | "edit.redo" | "history.stepForward" => {
            st.floating = None;
            Ok(None)
        }
        _ if keeps_floating(id) => Ok(None),
        _ => drop_floating(s).map(|_| None),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "select.float",
            label: "Float Selection",
            menu: &[],
            shortcut: None,
            params: r##"{"dx":px=0,"dy":px=0,"copy":bool=false,"target":"pixels"|"mask"="pixels" (mask: float the selected part of the layer's mask, the hole takes the background colour)} → {layer, offset} (cuts the selected pixels of the active layer into a floating piece the first time, or with copy leaves them and floats a duplicate; on the Background the hole takes the background colour; then moves it by whole pixels; dropped by select.drop or any other command, put back by edit.undo)"##,
            enabled: can_float,
            journal: true,
            run: float,
        },
        CommandSpec {
            id: "select.drop",
            label: "Drop Floating Selection",
            menu: &[],
            shortcut: None,
            params: r##"{} → {dropped, offset} (drops the floating piece into its layer and moves the selection with it, shrunk to the moved pixels: the selection limited to where the layer has pixels, as in Photoshop; a mask or an empty piece keeps the selection's shape; one history step)"##,
            enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document".into()),
            journal: true,
            run: |s, _| drop_floating(s),
        },
    ]
}

#[cfg(test)]
mod tests;
