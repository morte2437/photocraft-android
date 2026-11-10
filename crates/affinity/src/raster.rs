//! Pixel layers (`Rstr`) and placed images (`ImgN`). A placed image keeps its original file
//! (`DyBm.Bckg` → archive entry `c/N`), which is imported as is; other pixels are planar tiles of
//! 256 bytes × 256 rows per channel, each its own archive entry.

use std::collections::{HashMap, HashSet};

use crate::Error;
use crate::model::{Affine, Image, Pixels, Reader};
use crate::stream::{self, ObjId, Tag, Value};

/// Pixels decoded per layer (the content's bounding box, not the canvas).
const MAX_PIXELS: u64 = 64 << 20;
const TILE: usize = 256;
const TILE_BYTES: usize = TILE * TILE;

pub(crate) fn node_image(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(bitmap) = s.obj(id, b"Bitm") else {
        r.warn("pixel layers without pixels");
        return Ok(None);
    };
    if !s.is(bitmap, b"DyBm") {
        r.warn("pixel layers stored in an unknown way");
        return Ok(None);
    }
    let width = s.int(bitmap, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    // A placed image: the original file, byte for byte.
    let has_tiles = s.field(bitmap, b"Sta1").is_some();
    if let Some(name) = s.entry(bitmap, b"Bckg").filter(|n| !n.is_empty())
        && (s.is(id, b"ImgN") || !has_tiles)
    {
        match original(r, name) {
            Ok(bytes) => return Ok(Some(Image { width, height, pixels: Pixels::Encoded(bytes), transform: world })),
            Err(Error::Limit(e)) => return Err(Error::Limit(e)),
            Err(_) => r.warn("an embedded image file that could not be read (its cached pixels were used)"),
        }
    }
    // The content's bounding box, so a mostly empty canvas-sized layer stays small.
    let crop = s
        .field(id, b"BitR")
        .and_then(|v| if let Value::Ints(i) = v { Some(i.clone()) } else { None })
        .filter(|v| v.len() == 4 && v[0] > i64::from(i32::MIN) + 1)
        .map(|v| {
            (
                v[0].clamp(0, i64::from(width)) as u32,
                v[1].clamp(0, i64::from(height)) as u32,
                v[2].clamp(0, i64::from(width)) as u32,
                v[3].clamp(0, i64::from(height)) as u32,
            )
        })
        .unwrap_or((0, 0, width, height));
    if s.field(bitmap, b"Bckg").is_some() && has_tiles && !s.is(id, b"ImgN") {
        r.warn("edited placed images use their stored pixels");
    }
    decode(r, bitmap, crop, world)
}

/// A mask layer (`MRst`): one channel of coverage as grey pixels (white shows, black hides).
pub(crate) fn mask(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(bitmap) = s.obj(id, b"Bitm") else { return Ok(None) };
    let width = s.int(bitmap, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 || !matches!(s.enumeration(bitmap, b"Frmt"), Some((6 | 7, _))) {
        r.warn("pixel masks in an unknown format (imported without them)");
        return Ok(None);
    }
    decode(r, bitmap, (0, 0, width, height), world)
}

/// An embedded document or symbol instance (`EmbN`), as the picture of it Affinity caches.
pub(crate) fn embedded(r: &mut Reader, id: ObjId, world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let Some(cache) = s.obj(id, b"Bitm").and_then(|e| s.obj(e, b"Cach")) else { return Ok(None) };
    let Some(bbox) = s.obj(id, b"Bitm").and_then(|e| s.floats::<4>(e, b"ChBB")) else { return Ok(None) };
    let width = s.int(cache, b"BmpW").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let height = s.int(cache, b"BmpH").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let [x0, y0, x1, y1] = bbox;
    // The instance's origin is the centre of the embedded content (checked against the
    // thumbnails of documents with embedded documents and SVG files).
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let place = Affine([(x1 - x0) / f64::from(width), 0.0, 0.0, (y1 - y0) / f64::from(height), x0 - cx, y0 - cy]).then(world);
    decode(r, cache, (0, 0, width, height), place)
}

/// Decode the tiles of a bitmap inside `crop` (pixels), placed by `world` (bitmap pixels to document).
fn decode(r: &mut Reader, bitmap: ObjId, crop: (u32, u32, u32, u32), world: Affine) -> Result<Option<Image>, Error> {
    let s = r.s;
    let (x0, y0, x1, y1) = crop;
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        r.warn("pixel layers larger than 64 megapixels");
        return Ok(None);
    }
    let format = s.enumeration(bitmap, b"Frmt").map(|(f, _)| f);
    let (channels, bps) = match format {
        Some(0) => (4, 1),
        Some(1) => (4, 2),
        Some(6) => (1, 1),
        Some(7) => (1, 2),
        Some(4) => {
            r.warn("CMYK pixel layers (converted to RGB without the document's colour profile)");
            (5, 1)
        }
        _ => {
            r.warn("pixel layers in grey, Lab or 32-bit formats");
            return Ok(None);
        }
    };
    // A layer is materialized from its declared size, whatever the tiles hold: admit the output
    // and the planes it decodes through before either is allocated (#4960).
    let pixels = (w as usize).checked_mul(h as usize).ok_or(Error::Limit("pixel layer size"))?;
    let scratch = pixels.checked_mul(channels * bps).ok_or(Error::Limit("pixel layer size"))?;
    r.admit_pixels(pixels.checked_mul(4).ok_or(Error::Limit("pixel layer size"))?, scratch)?;
    let mut planes = Vec::with_capacity(channels);
    let mut cache = Cache::new();
    for c in 1..=channels {
        planes.push(plane(r, bitmap, c, bps, (x0, y0, w, h), &mut cache)?);
    }
    let rgba = interleave(&planes, channels, bps, pixels);
    let transform = Affine([1.0, 0.0, 0.0, 1.0, f64::from(x0), f64::from(y0)]).then(world);
    Ok(Some(Image { width: w, height: h, pixels: Pixels::Rgba8(rgba), transform }))
}

/// Quantize a unit-range sample to a byte: the one conversion every format goes through.
fn quant(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Interleave the decoded planes (`n` samples of `bps` bytes each) into RGBA8.
///
/// The samples used to be widened to `f64` one pixel at a time. The tables below hold the same
/// expressions evaluated once per input value, so the bytes are identical: an 8-bit sample
/// `x / 255 * 255` rounds back to `x`, a 16-bit one goes through a 64 Ki table, and CMYK's
/// `(1 - c)(1 - k)` through a table over the 256 x 256 `(c, k)` pairs.
pub(crate) fn interleave(planes: &[Vec<u8>], channels: usize, bps: usize, n: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; n * 4];
    let out = rgba.as_chunks_mut::<4>().0.iter_mut();
    let p = |c: usize| planes.get(c).map_or(&[][..], Vec::as_slice);
    match (channels, bps) {
        (1, 1) => {
            for (o, &v) in out.zip(p(0)) {
                *o = [v, v, v, 255];
            }
        }
        (4, 1) => {
            let (r, g, b, a) = (p(0), p(1), p(2), p(3));
            for ((((o, &r), &g), &b), &a) in out.zip(r).zip(g).zip(b).zip(a) {
                *o = [r, g, b, a];
            }
        }
        (5, 1) => {
            let lut = cmyk_table();
            let (c, m, y, k, a) = (p(0), p(1), p(2), p(3), p(4));
            for (((((o, &c), &m), &y), &k), &a) in out.zip(c).zip(m).zip(y).zip(k).zip(a) {
                let k = usize::from(k) << 8;
                *o = [lut[k | usize::from(c)], lut[k | usize::from(m)], lut[k | usize::from(y)], a];
            }
        }
        (_, _) => {
            let lut: Vec<u8> = (0..=u16::MAX).map(|v| quant(f64::from(v) / 65535.0)).collect();
            let s16 = |c: usize| p(c).as_chunks::<2>().0.iter().map(|b| lut[usize::from(u16::from_le_bytes(*b))]);
            if channels == 1 {
                for (o, g) in out.zip(s16(0)) {
                    *o = [g, g, g, 255];
                }
            } else {
                for ((((o, r), g), b), a) in out.zip(s16(0)).zip(s16(1)).zip(s16(2)).zip(s16(3)) {
                    *o = [r, g, b, a];
                }
            }
        }
    }
    rgba
}

/// `(1 - c)(1 - k)` quantized, indexed by `k << 8 | c`.
fn cmyk_table() -> Vec<u8> {
    let mut lut = Vec::with_capacity(256 * 256);
    for k in 0..=255u8 {
        for c in 0..=255u8 {
            lut.push(quant((1.0 - f64::from(c) / 255.0) * (1.0 - f64::from(k) / 255.0)));
        }
    }
    lut
}

/// The original image file: entry `c/N` is a stream whose root (`Blck`) holds it as `Data`.
fn original(r: &mut Reader, name: &str) -> Result<Vec<u8>, Error> {
    let bytes = r.archive.read(name)?;
    let s = stream::parse(&bytes)?;
    let root = s.object(s.root).ok_or(Error::Malformed("embedded image"))?;
    match root.get(Tag::of(b"Data")) {
        Some(Value::Blob(b)) if !b.is_empty() => Ok(b.clone()),
        _ => Err(Error::Malformed("embedded image without data")),
    }
}

fn tag(prefix: &[u8; 3], c: usize) -> [u8; 4] {
    [prefix[0], prefix[1], prefix[2], b'0' + c as u8]
}

/// Decoded tiles by archive entry name: a tile shared by several grid cells or channels is
/// extracted (and charged to the archive's budget) once.
type Cache<'s> = HashMap<&'s str, Vec<u8>>;

/// A tile to copy into a plane once its entry is decoded: pixel column and row of its corner.
struct Place<'s> {
    name: &'s str,
    at: (usize, usize),
}

/// One channel of level 0, cropped to (x0, y0, w, h) pixels; `bps` bytes per sample.
fn plane<'s>(
    r: &mut Reader<'s, '_, '_>,
    bitmap: ObjId,
    c: usize,
    bps: usize,
    (x0, y0, w, h): (u32, u32, u32, u32),
    cache: &mut Cache<'s>,
) -> Result<Vec<u8>, Error> {
    let s = r.s;
    let width = s.int(bitmap, b"BmpW").and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
    let height = s.int(bitmap, b"BmpH").and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
    let tw = s.int(bitmap, &tag(b"TWi", c)).and_then(|v| usize::try_from(v).ok()).unwrap_or_else(|| (width * bps).div_ceil(TILE));
    let th = s.int(bitmap, &tag(b"THi", c)).and_then(|v| usize::try_from(v).ok()).unwrap_or_else(|| height.div_ceil(TILE));
    // Borrow the file-controlled arrays: collecting the whole state array first costs its full
    // length on every occurrence of a shared layer, however small the declared grid is.
    let states: &[Value] = match s.field(bitmap, &tag(b"Sta", c)) {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    // Tile references in order. Null slots are skipped, but every slot walked (null or not) is
    // charged below, so a shared bitmap's long null prefix is not rescanned for free.
    let mut tiles: &[Value] = match s.field(bitmap, &tag(b"Idx", c)) {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    // The scan below runs for every occurrence of a shared bitmap, whatever it produces.
    r.charge_slots(states.len().min(tw.saturating_mul(th)))?;
    let row = (w as usize) * bps;
    let mut out = vec![0u8; row * h as usize];
    let (bx0, bx1) = (x0 as usize * bps, (x0 + w) as usize * bps);
    let (py0, py1) = (y0 as usize, (y0 + h) as usize);
    let crop = (bx0, py0, bx1, py1);
    // First walk the grid: flat cells are filled right away (cells never overlap, so the order
    // of the copies does not matter) and the tiles that show are collected. They are extracted
    // together below, on several threads, then copied. A walk that fails still extracts the
    // tiles before the failing cell, as a tile-by-tile walk would have.
    let mut places: Vec<Place<'s>> = Vec::new();
    let mut wanted: Vec<&'s str> = Vec::new();
    let mut queued: HashSet<&'s str> = HashSet::new();
    let mut failure = None;
    for (t, state) in states.iter().enumerate().take(tw.saturating_mul(th)) {
        let state = &if let Value::UInt(u) = state { u8::try_from(*u).unwrap_or(255) } else { 255 };
        let (ty, tx) = (t / tw.max(1), t % tw.max(1));
        let fill = match state {
            0 | 1 => None,
            2 => Some(0xFF),
            4 => {
                let Some(at) = tiles.iter().position(|v| matches!(v, Value::Obj(Some(_)))) else {
                    r.charge_slots(tiles.len())?;
                    failure = Some(Error::Malformed("missing pixel tile"));
                    break;
                };
                r.charge_slots(at + 1)?;
                let Some(Value::Obj(Some(blck))) = tiles.get(at) else {
                    failure = Some(Error::Malformed("missing pixel tile"));
                    break;
                };
                let blck = *blck;
                tiles = tiles.get(at + 1..).unwrap_or_default();
                let (ox, oy) = (tx * TILE, ty * TILE);
                if ox >= bx1 || ox + TILE <= bx0 || oy >= py1 || oy + TILE <= py0 {
                    continue;
                }
                let Some(name) = s.entry(blck, b"Data") else {
                    failure = Some(Error::Malformed("pixel tile without data"));
                    break;
                };
                if !cache.contains_key(name) && queued.insert(name) {
                    wanted.push(name);
                }
                places.push(Place { name, at: (ox, oy) });
                continue;
            }
            3 => {
                r.warn("32-bit pixel layers");
                None
            }
            5 => {
                r.warn("pixel layers drawn from an embedded image (left empty)");
                None
            }
            _ => {
                failure = Some(Error::Malformed("unknown pixel tile state"));
                break;
            }
        };
        if let Some(v) = fill {
            copy(&mut out, row, crop, (tx * TILE, ty * TILE), None, v);
        }
    }
    let decoded = r.archive.read_many(&wanted);
    // A tile that fails to extract is reported before the failure of the walk after it.
    for (name, mut data) in wanted.into_iter().zip(decoded?) {
        // Older files wrap a tile in a one-field stream: header, then a 64 KiB blob.
        if data.len() == TILE_BYTES + 22 && data.starts_with(b"\x00\xffKS") {
            data = data.get(21..21 + TILE_BYTES).ok_or(Error::Malformed("pixel tile"))?.to_vec();
        }
        if data.len() != TILE_BYTES {
            return Err(Error::Malformed("pixel tile size"));
        }
        cache.insert(name, data);
    }
    if let Some(e) = failure {
        return Err(e);
    }
    for place in &places {
        let tile = cache.get(place.name).ok_or(Error::Malformed("pixel tile"))?;
        copy(&mut out, row, crop, place.at, Some(tile), 0);
    }
    Ok(out)
}

/// Copy the overlap of a tile at byte column `ox`, row `oy` into the cropped plane. `tile` is
/// the tile's bytes, or `None` to fill the overlap with `fill`.
pub(crate) fn copy(out: &mut [u8], row: usize, (bx0, py0, bx1, py1): (usize, usize, usize, usize), (ox, oy): (usize, usize), tile: Option<&[u8]>, fill: u8) {
    let (xa, xb) = (ox.max(bx0), (ox + TILE).min(bx1));
    if xa >= xb {
        return;
    }
    for y in oy.max(py0)..(oy + TILE).min(py1) {
        let start = (y - py0) * row + (xa - bx0);
        let Some(dst) = out.get_mut(start..start + (xb - xa)) else { continue };
        match tile.and_then(|t| t.get((y - oy) * TILE + (xa - ox)..(y - oy) * TILE + (xb - ox))) {
            Some(src) => dst.copy_from_slice(src),
            None => dst.fill(if tile.is_some() { 0 } else { fill }),
        }
    }
}
