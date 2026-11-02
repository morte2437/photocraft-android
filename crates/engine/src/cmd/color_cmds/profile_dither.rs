//! Opt-in 8-bit profile quantization. The ordinary CMS buffer paths stay unchanged.

use std::sync::Arc;

use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Tile;
use rayon::prelude::*;

use super::{
    ColorMode, Document, EngineError, Intent, PixelFormat, Profile, Result, SampleType, Surface, Transform, cms_err, convert_color, convert_document,
    convert_layer_colors, convert_surface, document_profile, space_mode,
};

const TILE_PIXELS: u128 = (TILE_SIZE * TILE_SIZE) as u128;
// Two per-pixel arrays and the decoded source default; no tile or plane staging.
const SCRATCH_BYTES: u128 = (3 * 16 * std::mem::size_of::<f32>()) as u128;

fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "edit.convertToProfile".into(), msg: message.into() }
}

/// Dither only layouts the CMS can transform directly, copying alpha without conversion.
fn eligible(s: &Surface, from: ColorMode, to: PixelFormat, t: &Transform) -> bool {
    let sf = s.format();
    sf.sample == SampleType::U8
        && sf.mode == from
        && t.inputs() == sf.mode.color_channels()
        && t.outputs() == to.mode.color_channels()
        && sf.alpha == to.alpha
        && sf.channels() <= 16
        && to.channels() <= 16
}

fn in_canvas_tiles(c: TileCoord, bounds: Rect) -> bool {
    if bounds.is_empty() {
        return false;
    }
    let a = TileCoord::containing(bounds.x0, bounds.y0);
    let b = TileCoord::containing(bounds.x1 - 1, bounds.y1 - 1);
    c.tx >= a.tx && c.tx <= b.tx && c.ty >= a.ty && c.ty <= b.ty
}

/// Noise needs real pixel coordinates, not the saturating rectangle of a malformed tile.
fn tile_origin(c: TileCoord) -> Result<(i32, i32)> {
    let x = i64::from(c.tx) * i64::from(TILE_SIZE);
    let y = i64::from(c.ty) * i64::from(TILE_SIZE);
    if x < i64::from(i32::MIN)
        || y < i64::from(i32::MIN)
        || x + i64::from(TILE_SIZE - 1) > i64::from(i32::MAX)
        || y + i64::from(TILE_SIZE - 1) > i64::from(i32::MAX)
    {
        return Err(bad("a stored tile has unaddressable coordinates; disable dithering to use the ordinary conversion"));
    }
    Ok((x as i32, y as i32))
}

/// A transparent default stays transparent: its converted default already stands for the absent
/// tiles, and dither noise under alpha 0 is invisible, so only stored tiles are dithered.
fn sparse(s: &Surface) -> bool {
    let sf = s.format();
    sf.alpha && s.default_pixel().get(sf.mode.color_channels()).is_some_and(|a| *a == 0.0)
}

/// The tiles the dithered output stores: every in-canvas tile (unless the surface is sparse)
/// plus every stored tile.
fn dither_tiles(s: &Surface, bounds: Rect) -> Vec<TileCoord> {
    if sparse(s) {
        return s.tiles().map(|(c, _)| *c).collect();
    }
    let outside = s.tiles().map(|(c, _)| *c).filter(|c| !in_canvas_tiles(*c, bounds));
    bounds.tiles().chain(outside).collect()
}

/// Price every additional output tile before allocating any; distant tiles add no bounding box.
fn output_bytes(s: &Surface, to: PixelFormat, bounds: Rect) -> Result<u128> {
    let mut count = if sparse(s) { 0 } else { u128::from(bounds.tile_count()) };
    for (c, _) in s.tiles() {
        tile_origin(*c)?;
        if sparse(s) || !in_canvas_tiles(*c, bounds) {
            count = count.checked_add(1).ok_or_else(|| bad("dither output size overflowed"))?;
        }
    }
    count
        .checked_mul(TILE_PIXELS)
        .and_then(|n| n.checked_mul(to.bytes_per_pixel() as u128))
        .and_then(|n| n.checked_add(to.bytes_per_pixel() as u128))
        .ok_or_else(|| bad("dither output size overflowed"))
}

/// Stochastic rounding before quantization preserves integer samples and neutral channels.
fn quantize(value: f32, threshold: f64) -> u8 {
    if !value.is_finite() {
        // Match the ordinary converter's saturating float cast, including NaN -> zero.
        return (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    let scaled = f64::from(value.clamp(0.0, 1.0) * 255.0);
    let lower = scaled.floor();
    (lower + f64::from(u8::from(scaled - lower > threshold))) as u8
}

/// Stream the finite canvas and stored deviations, retaining the undithered infinite default.
/// Tiles are independent (the noise depends on pixel coordinates only) and run in parallel.
fn surface(s: &Surface, to: PixelFormat, t: &Transform, bounds: Rect) -> Result<Surface> {
    let ds = to.channels();
    let mut converted_default = [0.0; 16];
    t.convert_f32(&s.default_pixel(), s.channels(), &mut converted_default[..ds], ds, true);
    let mut out = Surface::with_default(to, &converted_default[..ds]);
    let tiles = dither_tiles(s, bounds).par_iter().map(|c| dither_tile(s, &out, t, bounds, *c).map(|tile| (*c, Arc::new(tile)))).collect::<Result<Vec<_>>>()?;
    out.put_tiles(tiles);
    out.prune();
    Ok(out)
}

/// One output tile of [`surface`], starting from the converted default of `out`.
fn dither_tile(s: &Surface, out: &Surface, t: &Transform, bounds: Rect, c: TileCoord) -> Result<Tile> {
    let (ss, ds) = (s.channels(), out.channels());
    let (ni, no) = (t.inputs(), t.outputs());
    let default = s.default_bytes();
    let (x0, y0) = tile_origin(c)?;
    let source = s.tile(c).map(|tile| tile.bytes());
    let mut dest = out.try_default_tile().map_err(|e| EngineError::Other(format!("could not allocate dither output: {e}")))?;
    for (index, dp) in dest.bytes_mut().chunks_exact_mut(ds).enumerate() {
        let x = x0 + (index % TILE_SIZE as usize) as i32;
        let y = y0 + (index / TILE_SIZE as usize) as i32;
        let sp = match source {
            Some(bytes) => bytes.get(index * ss..index * ss + ss).ok_or_else(|| bad("a stored tile has invalid sample storage"))?,
            None => default,
        };
        // A redundant default tile outside the canvas must match the implicit plane.
        if !bounds.contains(x, y) && sp == default {
            continue;
        }
        let mut input = [0.0; 16];
        let mut output = [0.0; 16];
        for (d, s) in input.iter_mut().zip(sp).take(ni) {
            *d = f32::from(*s) / 255.0;
        }
        t.eval_fast(&input[..ni], &mut output[..no]);
        // The hash can return 1.0; recenter its 65536 bins strictly inside (0, 1).
        let threshold = (f64::from(photocraft_color::dither_noise(x, y)) * 65535.0 + 0.5) / 65536.0;
        for (d, value) in dp.iter_mut().zip(output).take(no) {
            *d = quantize(value, threshold);
        }
        for (d, s) in dp.iter_mut().skip(no).zip(sp.iter().skip(ni)) {
            *d = *s;
        }
    }
    Ok(dest)
}

/// Preserve ordinary conversion outside opt-in U8, admitting the whole dither edit first.
pub(super) fn convert(doc: &mut Document, dst: &Profile, intent: Intent, bpc: bool) -> Result<()> {
    let src = document_profile(doc);
    if doc.depth != SampleType::U8 || src.to_bytes() == dst.to_bytes() || doc.mode == ColorMode::Duotone {
        return convert_document(doc, dst, intent, bpc);
    }
    let to_mode = space_mode(dst.color_space).ok_or_else(|| EngineError::Other(format!("cannot convert to a {:?} profile", dst.color_space)))?;
    let from = doc.pixel_format().mode;
    let t = Transform::new(&src, dst, intent, bpc).map_err(cms_err)?;
    let format = PixelFormat::new(to_mode, doc.depth, true);
    let bounds = doc.bounds();
    if doc.size.width > i32::MAX as u32 || doc.size.height > i32::MAX as u32 {
        return Err(bad("the canvas has unaddressable coordinates; disable dithering to use the ordinary conversion"));
    }
    let mut bytes = SCRATCH_BYTES;
    let mut admission = Ok(());
    crate::image_cmds::for_each_surface(&mut doc.layers, false, &mut |s, _| {
        if admission.is_err() {
            return;
        }
        let to = PixelFormat { alpha: s.format().alpha, ..format };
        if eligible(s, from, to, &t) {
            admission = output_bytes(s, to, bounds).and_then(|additional| {
                bytes = bytes.checked_add(additional).ok_or_else(|| bad("dither output size overflowed"))?;
                Ok(())
            });
        }
    });
    admission?;
    // This is an output-raster cap, not process peak: the source and history remain retained.
    if bytes > u128::from(crate::image_cmds::MAX_RESAMPLE_BYTES) {
        return Err(bad("dither output exceeds the raster allocation budget; use a smaller canvas or disable dithering"));
    }
    let mut result = Ok(());
    crate::image_cmds::for_each_surface(&mut doc.layers, false, &mut |s, _| {
        if result.is_err() {
            return;
        }
        let to = PixelFormat { alpha: s.format().alpha, ..format };
        if eligible(s, from, to, &t) {
            result = surface(s, to, &t, bounds).map(|converted| *s = converted);
        } else {
            *s = convert_surface(s, from, to, &t);
        }
    });
    result?;
    for layer in &mut doc.layers {
        convert_layer_colors(layer, &mut |c| convert_color(c, from, to_mode, &t));
    }
    doc.mode = to_mode;
    doc.icc_profile = Some(dst.to_bytes());
    doc.color_table = None;
    doc.duotone = None;
    Ok(())
}
