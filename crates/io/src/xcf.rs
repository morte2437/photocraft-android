//! GIMP XCF (`.xcf`, `.xcf.gz`) read. Layout from the public specification
//! (https://developer.gimp.org/core/standards/xcf/, versions 0 to 26): a header, image
//! properties, then pointers to layer, channel and path structures; each drawable's pixels sit in
//! 64×64 tiles, uncompressed, RLE-coded or zlib-coded.
//!
//! Read: pixel layers at any position and size, groups (nested, pass-through), names, opacity,
//! visibility, offsets, layer masks (applied or disabled), locks, colour tags, blend modes (the
//! legacy and the 2.10 sets; those without a Photoshop equivalent open as Normal with a warning),
//! 8/16/32-bit integer and 16/32/64-bit float precisions (linear precisions get a linear-light
//! profile), the embedded ICC profile, resolution, guides, the comment, extra channels and the
//! saved selection; indexed images open as RGB. Text, vector and link layers open as their pixels;
//! non-destructive layer effects, paths and floating selections are reported, not applied.
//! Write: not supported (GIMP's own format; `.pcraft` keeps everything PhotoCraft holds).
//!
//! The reader is written from the specification and from files GIMP 3.2 writes; no GIMP code.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use photocraft_codecs::f16;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, LabelColor, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::ora::Warnings;
use crate::{ImportResult, IoError};

type Result<T> = std::result::Result<T, IoError>;

const MAGIC: &[u8] = b"gimp xcf ";
/// Canvas side limit (Photoshop's PSB limit).
pub(crate) const MAX_SIDE: u32 = 300_000;
/// Decoded pixels across every drawable are capped, as for OpenRaster and PDN.
pub(crate) const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Layers and groups in one document.
pub(crate) const MAX_LAYERS: usize = 8192;
/// Extra channels in one document.
const MAX_CHANNELS: usize = 1024;
/// Group nesting.
const MAX_DEPTH: usize = 64;
/// Total copied layer metadata (names and item paths). A single reused record with a large
/// name is bounded by MAX_STRING, but repeated layer offsets referencing it must not multiply
/// into unbounded retained metadata.
const MAX_METADATA_BYTES: usize = 32 << 20;
/// A string field (layer names, parasite names).
const MAX_STRING: usize = 1 << 20;
/// A property payload or a parasite that is read into memory.
const MAX_PAYLOAD: usize = 64 << 20;
/// The largest file a gzipped XCF may inflate to.
const MAX_INFLATED: usize = 1 << 30;
/// Highest version this reader was written against; newer files still open (unknown properties
/// are skipped by their length), with a warning.
const KNOWN_VERSION: u32 = 26;
const TILE: u32 = 64;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Xcf(message.into())
}

/// `true` for an XCF file (by its signature).
pub(crate) fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// `true` for a gzip stream named `.xcf.gz` / `.xcfgz` (GIMP writes and reads compressed XCF).
pub(crate) fn is_gzipped_xcf(name: &str, bytes: &[u8]) -> bool {
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    bytes.starts_with(&[0x1f, 0x8b]) && (lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz"))
}

/// Inflates a gzipped XCF, bounded.
pub(crate) fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut dec = flate2::read::GzDecoder::new(bytes).take(MAX_INFLATED as u64 + 1);
    dec.read_to_end(&mut out).map_err(|e| invalid(format!("gzip: {e}")))?;
    if out.len() > MAX_INFLATED {
        return Err(invalid("the compressed file inflates past the 1 GiB limit"));
    }
    Ok(out)
}

/// How samples are stored: bytes per sample, whether they are floats, and whether the values are
/// linear light (else gamma-encoded, i.e. the profile's own curve).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Precision {
    bytes: usize,
    float: bool,
    linear: bool,
}

impl Precision {
    const GAMMA8: Precision = Precision { bytes: 1, float: false, linear: false };

    fn from_header(version: u32, code: u32) -> Result<Precision> {
        let p = |bytes, float, linear| Ok(Precision { bytes, float, linear });
        match version {
            0..=3 => Ok(Self::GAMMA8),
            4 => match code {
                0 => p(1, false, false),
                1 => p(2, false, false),
                2 => p(4, false, true),
                3 => p(2, true, true),
                4 => p(4, true, true),
                c => Err(invalid(format!("unknown precision {c} in an XCF version 4 file"))),
            },
            5 | 6 => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                400 => p(2, true, true),
                450 => p(2, true, false),
                500 => p(4, true, true),
                550 => p(4, true, false),
                c => Err(invalid(format!("unknown precision {c} in an XCF version {version} file"))),
            },
            _ => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                500 => p(2, true, true),
                550 => p(2, true, false),
                600 => p(4, true, true),
                650 => p(4, true, false),
                700 => p(8, true, true),
                750 => p(8, true, false),
                c => Err(invalid(format!("unknown precision {c}"))),
            },
        }
    }

    /// The document depth the samples open at.
    fn depth(self) -> SampleType {
        match (self.float, self.bytes) {
            (false, 1) => SampleType::U8,
            (false, _) => SampleType::U16,
            (true, _) => SampleType::F32,
        }
    }

    /// One sample, big-endian, as a normalised float.
    fn sample(self, b: &[u8]) -> f32 {
        match (self.float, self.bytes, b) {
            (false, 1, [v]) => f32::from(*v) / 255.0,
            (false, 2, [a, b]) => f32::from(u16::from_be_bytes([*a, *b])) / 65535.0,
            (false, 4, [a, b, c, d]) => (u32::from_be_bytes([*a, *b, *c, *d]) as f64 / f64::from(u32::MAX)) as f32,
            (true, 2, [a, b]) => f16::from_bits(u16::from_be_bytes([*a, *b])).to_f32(),
            (true, 4, [a, b, c, d]) => f32::from_be_bytes([*a, *b, *c, *d]),
            (true, 8, [a, b, c, d, e, f, g, h]) => f64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) as f32,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Rle,
    Zlib,
}

/// Big-endian reads with bounds checks; every failure is an error, never a panic.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
    /// XCF 11 and later store pointers as 64-bit.
    wide: bool,
}

impl<'a> Cursor<'a> {
    fn at(&self, pos: usize) -> Cursor<'a> {
        Cursor { b: self.b, pos, wide: self.wide }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| invalid("offset overflow"))?;
        let s = self.b.get(self.pos..end).ok_or_else(|| invalid(format!("the file ends early (wanted {n} bytes at {})", self.pos)))?;
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn ptr(&mut self) -> Result<usize> {
        if self.wide {
            let s = self.take(8)?;
            usize::try_from(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])).map_err(|_| invalid("pointer does not fit in memory"))
        } else {
            self.u32().map(|v| v as usize)
        }
    }

    /// A STRING: `n+1` then `n` bytes and a NUL (0 for the empty string).
    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The same bounded STRING read without allocating or decoding an unused field.
    fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return Err(invalid(format!("a string of {n} bytes exceeds the limit")));
        }
        let s = self.take(n)?;
        Ok(s.strip_suffix(&[0]).unwrap_or(s))
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// One property record: its type and payload.
struct Prop<'a> {
    kind: u32,
    data: &'a [u8],
}

/// Reads a property list, measuring known payloads whose length words GIMP writes incorrectly.
/// Ends at PROP_END (type 0).
fn props<'a>(c: &mut Cursor<'a>) -> Result<Vec<Prop<'a>>> {
    let mut out = Vec::new();
    loop {
        let kind = c.u32()?;
        let len = c.u32()? as usize;
        if kind == 0 {
            return Ok(out);
        }
        // PROP_COLORMAP's length word was wrong in some releases (n+4 for 3n+4 bytes); its real
        // size follows from the count. PROP_USER_UNIT's length is wrong in every release.
        // PROP_LINK_LAYER's length can be four bytes short (#8857); read all its fields.
        let len = match kind {
            1 => {
                let n = c.at(c.pos).u32()? as usize;
                4usize.checked_add(n.checked_mul(3).ok_or_else(|| invalid("colormap overflow"))?).ok_or_else(|| invalid("colormap overflow"))?
            }
            24 => user_unit_len(c)?,
            48 => link_layer_len(c)?,
            _ => len,
        };
        if len > MAX_PAYLOAD {
            return Err(invalid(format!("property {kind} declares {len} bytes")));
        }
        let data = c.take(len)?;
        out.push(Prop { kind, data });
        if out.len() > 4096 {
            return Err(invalid("too many properties"));
        }
    }
}

/// A PROP_LINK_LAYER payload: flags, a path STRING, then the source width and height.
/// The dimensions are present for raster links too, where their values are unused.
fn link_layer_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(4)?;
    p.string_bytes()?;
    p.skip(8)?;
    Ok(p.pos - c.pos)
}

/// The real size of a PROP_USER_UNIT payload: a float, a count and three (XCF 21+) or five
/// strings. Measured without consuming.
fn user_unit_len(c: &Cursor<'_>) -> Result<usize> {
    let mut p = c.at(c.pos);
    p.skip(8)?;
    for _ in 0..3 {
        p.string()?;
    }
    // The two name forms of older files, when present: another string pair before the next
    // property. Decide by whether a plausible property header follows.
    let three = p.pos - c.pos;
    let mut q = p.at(p.pos);
    let looks_like_prop = |c: &mut Cursor<'_>| c.u32().map(|k| k <= 64).unwrap_or(false);
    if looks_like_prop(&mut q) {
        return Ok(three);
    }
    p.string()?;
    p.string()?;
    Ok(p.pos - c.pos)
}

fn be_u32(b: &[u8]) -> Option<u32> {
    b.get(..4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be_i32(b: &[u8]) -> Option<i32> {
    be_u32(b).map(|v| v as i32)
}

fn be_f32(b: &[u8]) -> Option<f32> {
    be_u32(b).map(f32::from_bits)
}

/// The parasites of a PROP_PARASITES payload as (name, data); a later duplicate wins.
fn parasites(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut c = Cursor { b: data, pos: 0, wide: false };
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // Map each name to its output slot so duplicate replacement is O(1). The parasite
    // records are file-controlled, so scanning `out` with iter_mut().find() per record is
    // O(n^2) and lets a crafted payload with many distinct names stall import. The map
    // preserves insertion order and last-value-wins behavior.
    let mut index: HashMap<String, usize> = HashMap::new();
    while c.pos < data.len() {
        let Ok(name) = c.string() else { break };
        let (Ok(_flags), Ok(len)) = (c.u32(), c.u32()) else { break };
        let Ok(payload) = c.take(len as usize) else { break };
        match index.get(&name) {
            Some(&i) => out[i].1 = payload.to_vec(),
            None => {
                index.insert(name.clone(), out.len());
                out.push((name, payload.to_vec()));
            }
        }
    }
    out
}

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel.
fn decode_tile(c: &Cursor<'_>, at: usize, end: usize, compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    let input = c.b.get(at..end.min(c.b.len())).ok_or_else(|| invalid("a tile pointer is outside the file"))?;
    match compression {
        Compression::None => input.get(..expected).map(<[u8]>::to_vec).ok_or_else(|| invalid("an uncompressed tile is cut short")),
        Compression::Zlib => {
            let mut out = Vec::with_capacity(expected);
            flate2::read::ZlibDecoder::new(input).take(expected as u64).read_to_end(&mut out).map_err(|e| invalid(format!("zlib tile: {e}")))?;
            if out.len() != expected {
                return Err(invalid("a zlib tile is cut short"));
            }
            Ok(out)
        }
        Compression::Rle => rle_decode(input, bpp, pixels),
    }
}

/// RLE tile data: one run-length stream per byte position of the pixel, each of `pixels` bytes.
fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; pixels * bpp];
    let mut pos = 0usize;
    let mut plane = Vec::with_capacity(pixels);
    for b in 0..bpp {
        plane.clear();
        while plane.len() < pixels {
            let op = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
            pos += 1;
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        false,
                    )
                }
                128 => {
                    let (p, q) = (input.get(pos).copied(), input.get(pos + 1).copied());
                    pos += 2;
                    (
                        usize::from(p.ok_or_else(|| invalid("an RLE tile is cut short"))?) * 256
                            + usize::from(q.ok_or_else(|| invalid("an RLE tile is cut short"))?),
                        true,
                    )
                }
                _ => (256 - usize::from(op), true),
            };
            if plane.len() + run > pixels {
                return Err(invalid("an RLE run crosses the end of its tile"));
            }
            if verbatim {
                let data = input.get(pos..pos + run).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += run;
                plane.extend_from_slice(data);
            } else {
                let v = *input.get(pos).ok_or_else(|| invalid("an RLE tile is cut short"))?;
                pos += 1;
                plane.resize(plane.len() + run, v);
            }
        }
        for (i, v) in plane.iter().enumerate() {
            out[i * bpp + b] = *v;
        }
    }
    Ok(out)
}

/// Image-wide facts the drawables need.
struct Image<'a> {
    c: Cursor<'a>,
    version: u32,
    precision: Precision,
    compression: Compression,
    base_type: u32,
    colormap: Vec<[u8; 3]>,
    budget: usize,
    /// Remaining allowance for copied layer metadata (names, item paths). Repeated layer
    /// offsets referencing one record with a large name would otherwise allocate an owned
    /// copy per reference, multiplying a small file into gigabytes of retained metadata.
    meta_budget: usize,
    warnings: Warnings,
    /// Layers GIMP composites with "Clip to backdrop" (its default for every mode but Normal).
    clipped_to_backdrop: usize,
}

/// Pixel data of a drawable: `channels` normalised samples per pixel, row-major.
struct Pixels {
    width: u32,
    height: u32,
    channels: usize,
    data: Vec<f32>,
}

impl<'a> Image<'a> {
    /// Reads a hierarchy (`hptr`): the first level's tiles into `channels` floats per pixel,
    /// `source_channels` as stored (indexed images expand to RGB here).
    fn hierarchy(&mut self, hptr: usize, source_channels: usize, ctl: &Interrupt<'_>) -> Result<Pixels> {
        let mut c = self.c.at(hptr);
        let (w, h, bpp) = (c.u32()?, c.u32()?, c.u32()? as usize);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(invalid(format!("drawable size {w}x{h} is out of range")));
        }
        let expected_bpp = source_channels.checked_mul(self.precision.bytes).ok_or_else(|| invalid("bpp overflow"))?;
        if bpp != expected_bpp {
            return Err(invalid(format!("{bpp} bytes per pixel where {expected_bpp} were expected")));
        }
        let indexed = self.base_type == 2;
        let channels = if indexed { source_channels + 2 } else { source_channels };
        let count = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(channels)).ok_or_else(|| invalid("drawable overflow"))?;
        let bytes = count.checked_mul(4).ok_or_else(|| invalid("drawable overflow"))?;
        self.budget = self.budget.checked_sub(bytes).ok_or_else(|| invalid("the layers exceed the 1 GiB pixel limit"))?;
        let lptr = c.ptr()?;
        let mut l = self.c.at(lptr);
        let (lw, lh) = (l.u32()?, l.u32()?);
        if (lw, lh) != (w, h) {
            return Err(invalid("the level does not match its hierarchy"));
        }
        let across = w.div_ceil(TILE) as usize;
        let down = h.div_ceil(TILE) as usize;
        let mut tiles = Vec::with_capacity(across * down);
        for _ in 0..across * down {
            let p = l.ptr()?;
            if p == 0 {
                return Err(invalid("the tile list ends before every tile"));
            }
            tiles.push(p);
        }
        let mut data = vec![0.0f32; count];
        let stride = w as usize * channels;
        for (i, &at) in tiles.iter().enumerate() {
            if i % 64 == 0 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
            }
            let (tx, ty) = ((i % across) as u32, (i / across) as u32);
            let (tw, th) = ((w - tx * TILE).min(TILE), (h - ty * TILE).min(TILE));
            let end = tiles.get(i + 1).copied().filter(|&n| n > at).unwrap_or(self.c.b.len());
            let raw = decode_tile(&self.c, at, end, self.compression, bpp, (tw * th) as usize)?;
            for py in 0..th as usize {
                for px in 0..tw as usize {
                    let src = (py * tw as usize + px) * bpp;
                    let dst = (ty as usize * TILE as usize + py) * stride + (tx as usize * TILE as usize + px) * channels;
                    let pixel = &raw[src..src + bpp];
                    if indexed {
                        let idx = usize::from(pixel[0]);
                        let [r, g, b] = self.colormap.get(idx).copied().unwrap_or([pixel[0]; 3]);
                        data[dst] = f32::from(r) / 255.0;
                        data[dst + 1] = f32::from(g) / 255.0;
                        data[dst + 2] = f32::from(b) / 255.0;
                        if source_channels == 2 {
                            data[dst + 3] = f32::from(pixel[1]) / 255.0;
                        }
                    } else {
                        for ch in 0..channels {
                            data[dst + ch] = self.precision.sample(&pixel[ch * self.precision.bytes..(ch + 1) * self.precision.bytes]);
                        }
                    }
                }
            }
        }
        Ok(Pixels { width: w, height: h, channels, data })
    }

    /// A channel structure (a layer mask, the selection or an extra channel) as one-channel
    /// pixels, with its properties.
    fn channel(&mut self, ptr: usize, ctl: &Interrupt<'_>) -> Result<(String, Vec<Prop<'a>>, Pixels)> {
        let mut c = self.c.at(ptr);
        let (_w, _h) = (c.u32()?, c.u32()?);
        let name = c.string()?;
        let props = props(&mut c)?;
        let hptr = c.ptr()?;
        let saved = (self.base_type, self.precision);
        // Channels are always one plain sample per pixel, even in indexed images.
        self.base_type = 0;
        let px = self.hierarchy(hptr, 1, ctl);
        (self.base_type, self.precision) = saved;
        Ok((name, props, px?))
    }
}

/// A layer as read, before the tree is built.
struct Node {
    layer: Layer,
    path: Vec<u32>,
    group: bool,
    children: Vec<Node>,
}

/// GIMP's layer mode as Photoshop's, with a note when the two differ.
fn blend_mode(mode: u32) -> (BlendMode, Option<&'static str>) {
    use BlendMode as B;
    match mode {
        0 | 28 => (B::Normal, None),
        1 => (B::Dissolve, None),
        2 | 29 => (B::Normal, Some("Behind")),
        3 | 30 => (B::Multiply, None),
        4 | 31 => (B::Screen, None),
        5 | 19 => (B::SoftLight, Some("GIMP's legacy Soft light")),
        45 => (B::SoftLight, Some("GIMP's Soft light")),
        23 => (B::Overlay, None),
        6 | 32 => (B::Difference, None),
        7 | 33 => (B::LinearDodge, None),
        8 | 34 => (B::Subtract, None),
        9 | 35 => (B::Darken, None),
        10 | 36 => (B::Lighten, None),
        11 | 37 => (B::Hue, Some("HSV Hue")),
        12 | 38 => (B::Saturation, Some("HSV Saturation")),
        13 | 39 => (B::Color, Some("HSL Color")),
        14 | 40 => (B::Luminosity, Some("HSV Value")),
        15 | 41 => (B::Divide, None),
        16 | 42 => (B::ColorDodge, None),
        17 | 43 => (B::ColorBurn, None),
        18 | 44 => (B::HardLight, None),
        20 | 46 => (B::Normal, Some("Grain extract")),
        21 | 47 => (B::Normal, Some("Grain merge")),
        22 | 57 => (B::Normal, Some("Color erase")),
        24 => (B::Hue, Some("LCH Hue")),
        25 => (B::Saturation, Some("LCH Chroma")),
        26 => (B::Color, Some("LCH Color")),
        27 => (B::Luminosity, Some("LCH Lightness")),
        48 => (B::VividLight, None),
        49 => (B::PinLight, None),
        50 => (B::LinearLight, None),
        51 => (B::HardMix, None),
        52 => (B::Exclusion, None),
        53 => (B::LinearBurn, None),
        54 => (B::DarkerColor, Some("Luma darken only")),
        55 => (B::LighterColor, Some("Luma lighten only")),
        56 => (B::Luminosity, Some("Luminance")),
        58 => (B::Normal, Some("Erase")),
        59 => (B::Normal, Some("Merge")),
        60 => (B::Normal, Some("Split")),
        61 => (B::PassThrough, None),
        _ => (B::Normal, Some("an unknown mode")),
    }
}

fn label(tag: u32) -> LabelColor {
    match tag {
        1 => LabelColor::Blue,
        2 => LabelColor::Green,
        3 => LabelColor::Yellow,
        4 => LabelColor::Orange,
        6 => LabelColor::Red,
        7 => LabelColor::Violet,
        8 => LabelColor::Gray,
        _ => LabelColor::None,
    }
}

/// Writes `px` (its channels matching `surface`'s) at `x, y` of the document.
fn write_pixels(surface: &mut Surface, px: &Pixels, x: i32, y: i32) -> Result<()> {
    let (w, h) = (px.width as i32, px.height as i32);
    let rect = Rect::new(x, y, x.checked_add(w).ok_or_else(|| invalid("offset overflow"))?, y.checked_add(h).ok_or_else(|| invalid("offset overflow"))?);
    if surface.channels() != px.channels || px.data.len() != (px.width as usize) * (px.height as usize) * px.channels {
        return Err(invalid("pixel data does not match the drawable"));
    }
    surface.write_region(rect, &px.data);
    surface.prune();
    Ok(())
}

/// Imports an XCF file (already inflated when it was gzipped).
pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    if !is_xcf(bytes) {
        return Err(invalid("not an XCF file"));
    }
    let mut c = Cursor { b: bytes, pos: MAGIC.len(), wide: false };
    let tag = c.take(5)?;
    let version = match tag {
        b"file\0" => 0,
        [b'v', a, b, d, 0] if a.is_ascii_digit() && b.is_ascii_digit() && d.is_ascii_digit() => {
            u32::from(a - b'0') * 100 + u32::from(b - b'0') * 10 + u32::from(d - b'0')
        }
        _ => return Err(invalid("unknown XCF version tag")),
    };
    c.wide = version >= 11;
    let mut warnings = Warnings::default();
    if version > KNOWN_VERSION {
        warnings.note(&format!("XCF version {version} is newer than this reader (version {KNOWN_VERSION}); unknown parts were skipped"));
    }
    let (w, h, base_type) = (c.u32()?, c.u32()?, c.u32()?);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(invalid(format!("canvas {w}x{h} must be 1 to {MAX_SIDE} pixels a side")));
    }
    if (w as usize).saturating_mul(h as usize).saturating_mul(16) > MAX_PIXEL_BYTES {
        return Err(invalid("the canvas exceeds the 1 GiB pixel limit"));
    }
    let precision = if version >= 4 { Precision::from_header(version, c.u32()?)? } else { Precision::GAMMA8 };
    let mode = match base_type {
        0 => ColorMode::Rgb,
        1 => ColorMode::Grayscale,
        2 => ColorMode::Rgb,
        t => return Err(invalid(format!("unknown image type {t}"))),
    };
    let depth = precision.depth();
    if base_type == 2 {
        warnings.note("the indexed image opens as RGB");
    }
    if !precision.float && precision.bytes == 4 {
        warnings.note("32-bit integer samples open as 16-bit");
    }

    // Image properties.
    let image_props = props(&mut c)?;
    let mut compression = Compression::None;
    let mut colormap = Vec::new();
    let mut resolution = None;
    let mut guides: Vec<(i32, u8)> = Vec::new();
    let mut icc = None;
    let mut comment = None;
    let mut path_count = 0usize;
    for p in &image_props {
        match p.kind {
            1 => {
                let n = be_u32(p.data).unwrap_or(0) as usize;
                colormap = p.data.get(4..).unwrap_or(&[]).as_chunks::<3>().0.iter().take(n.min(256)).copied().collect();
            }
            17 => {
                compression = match p.data.first() {
                    Some(0) => Compression::None,
                    Some(1) => Compression::Rle,
                    Some(2) => Compression::Zlib,
                    Some(k) => return Err(invalid(format!("unknown compression {k}"))),
                    None => return Err(invalid("empty compression property")),
                }
            }
            18 => {
                for g in p.data.as_chunks::<5>().0 {
                    guides.push((i32::from_be_bytes([g[0], g[1], g[2], g[3]]), g[4]));
                }
            }
            19 => {
                if let (Some(x), Some(y)) = (be_f32(p.data), p.data.get(4..).and_then(be_f32)) {
                    resolution = Some((x, y));
                }
            }
            21 => {
                for (name, data) in parasites(p.data) {
                    match name.as_str() {
                        "icc-profile" if !data.is_empty() => icc = Some(data),
                        "gimp-comment" => comment = Some(String::from_utf8_lossy(&data).trim_end_matches('\0').to_string()),
                        _ => {}
                    }
                }
            }
            23 => path_count += be_u32(p.data.get(4..).unwrap_or(&[])).unwrap_or(0) as usize,
            25 => path_count += be_u32(p.data.get(8..).unwrap_or(&[])).unwrap_or(0) as usize,
            _ => {}
        }
    }

    let mut layer_ptrs = Vec::new();
    loop {
        let p = c.ptr()?;
        if p == 0 {
            break;
        }
        layer_ptrs.push(p);
        if layer_ptrs.len() > MAX_LAYERS {
            return Err(invalid(format!("more than {MAX_LAYERS} layers")));
        }
    }
    let mut channel_ptrs = Vec::new();
    loop {
        let p = c.ptr()?;
        if p == 0 {
            break;
        }
        channel_ptrs.push(p);
        if channel_ptrs.len() > MAX_CHANNELS {
            return Err(invalid(format!("more than {MAX_CHANNELS} channels")));
        }
    }
    if version >= 18 {
        let mut n = 0;
        while c.ptr().ok().is_some_and(|p| p != 0) {
            n += 1;
            if n > MAX_CHANNELS {
                break;
            }
        }
        path_count += n;
    }

    let mut image = Image {
        c: c.at(0),
        version,
        precision,
        compression,
        base_type,
        colormap,
        budget: MAX_PIXEL_BYTES,
        meta_budget: MAX_METADATA_BYTES,
        warnings,
        clipped_to_backdrop: 0,
    };
    let fmt = PixelFormat::new(mode, depth, true);
    let mask_fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let total = layer_ptrs.len().max(1) as f32;

    // Layers, topmost first in the file.
    let mut nodes: Vec<Node> = Vec::new();
    let mut linear_compositing = false;
    for (i, &ptr) in layer_ptrs.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        ctl.progress(0.05 + 0.85 * i as f32 / total);
        if let Some((node, linear)) = image.layer(ptr, fmt, mask_fmt, ctl)? {
            linear_compositing |= linear;
            nodes.push(node);
        }
    }
    let mut roots: Vec<Node> = Vec::new();
    for (i, mut node) in nodes.into_iter().enumerate() {
        // Default path: the item's position at the top level, as a file without groups has it.
        if node.path.is_empty() {
            node.path = vec![i as u32];
        }
        if node.path.len() > MAX_DEPTH {
            image.warnings.layer(&node.layer.name, "is nested deeper than the reader allows and was moved to the top level");
            node.path.truncate(1);
        }
        place(&mut roots, node, &mut image.warnings);
    }
    let layers = finish(roots);

    // Channels: the selection and the extras.
    let mut channels = Vec::new();
    let mut selection = None;
    for &ptr in &channel_ptrs {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (name, cprops, px) = image.channel(ptr, ctl)?;
        let mut surface = Surface::new(mask_fmt);
        write_pixels(&mut surface, &px, 0, 0)?;
        if cprops.iter().any(|p| p.kind == 4) {
            selection = Some(surface);
        } else {
            channels.push(AlphaChannel::new(name, surface));
        }
    }

    let mut document = Document::new(name, Size::new(w, h), mode, depth);
    document.layers = layers;
    document.channels = channels;
    document.selection = selection;
    if let Some((x, y)) = resolution.filter(|(x, _)| *x > 0.0 && *x < 1.0e6) {
        document.resolution_dpi = x;
        if let Some(w) = crate::unequal_resolution_warning(f64::from(x), f64::from(y)) {
            image.warnings.note(&w);
        }
    }
    for (coord, orientation) in guides {
        if coord < 0 && version < 15 {
            continue;
        }
        match orientation {
            1 => document.guides.horizontal.push(f64::from(coord)),
            2 => document.guides.vertical.push(f64::from(coord)),
            _ => {}
        }
    }
    document.icc_profile = match (precision.linear, icc, mode) {
        (true, icc, ColorMode::Grayscale) => {
            if icc.is_some() {
                image.warnings.note("the linear-light image's own profile was replaced by linear gray");
            }
            Some(photocraft_cms::builtin::linear_gray().to_bytes())
        }
        (true, icc, _) => {
            if icc.is_some() {
                image.warnings.note("the linear-light image's own profile was replaced by linear sRGB");
            }
            Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes())
        }
        (false, Some(icc), _) => Some(Arc::new(icc)),
        (false, None, _) => None,
    };
    if let Some(comment) = comment.filter(|c| !c.is_empty()) {
        document.metadata.text.push(("Comment".into(), comment));
    }
    if path_count > 0 {
        image.warnings.note(&format!("{path_count} path(s) were not imported"));
    }
    // GIMP clips a layer in any mode but Normal to the layers beneath it, so it shows nothing where
    // they are transparent; PhotoCraft (like Photoshop) shows it there. Only tell when there is
    // transparency to show through: a bottom layer that is not opaque.
    if image.clipped_to_backdrop > 0 && document.layers.first().is_some_and(|l| !l.locks.transparency || l.is_group()) {
        image.warnings.note(&format!(
            "{} layer(s) use a mode GIMP clips to the layers beneath; where those are transparent GIMP shows nothing and PhotoCraft shows the layer",
            image.clipped_to_backdrop
        ));
    }
    // GIMP blends and composites in linear light by default (since 2.10); PhotoCraft, like
    // Photoshop, in the document's own encoding, so semi-transparent edges and most blend modes
    // render a little differently. Layers set to a perceptual space in GIMP match.
    if !precision.linear && linear_compositing {
        image.warnings.note("GIMP composites this image in linear light; PhotoCraft composites in gamma-encoded RGB, so blends and soft edges differ slightly");
    }
    if document.layers.is_empty() {
        image.warnings.note("the XCF file has no layers");
    }
    ctl.progress(1.0);
    Ok(ImportResult { document, warnings: image.warnings.finish(), source_read_only: true, preview_only: false })
}

/// Puts `node` where its item path says, under the groups already placed; a path that leads
/// nowhere (at any depth) puts it at the top level, so later siblings keep their indices.
fn place(roots: &mut Vec<Node>, node: Node, warnings: &mut Warnings) {
    let depth = node.path.len().saturating_sub(1);
    let mut probe: &Vec<Node> = roots;
    let reachable = node.path.iter().take(depth).all(|&index| match probe.get(index as usize) {
        Some(n) if n.group => {
            probe = &n.children;
            true
        }
        _ => false,
    });
    if !reachable {
        warnings.layer(&node.layer.name, "has an item path that leads to no group; opened at the top level");
        let mut node = node;
        node.path = vec![];
        roots.push(node);
        return;
    }
    let mut level = roots;
    for &index in node.path.iter().take(depth) {
        level = &mut level[index as usize].children;
    }
    level.push(node);
}

/// Nodes top-first into layers bottom-first, groups recursively.
fn finish(nodes: Vec<Node>) -> Vec<Layer> {
    let mut out = Vec::with_capacity(nodes.len());
    for mut node in nodes.into_iter().rev() {
        if node.group {
            let children = finish(std::mem::take(&mut node.children));
            if let Some(c) = node.layer.children_mut() {
                *c = children;
            }
        }
        out.push(node.layer);
    }
    out
}

impl<'a> Image<'a> {
    /// Reads a layer structure, and whether GIMP composites it in linear light. `None` for a
    /// floating selection, which is reported instead.
    fn layer(&mut self, ptr: usize, fmt: PixelFormat, mask_fmt: PixelFormat, ctl: &Interrupt<'_>) -> Result<Option<(Node, bool)>> {
        let mut c = self.c.at(ptr);
        let (lw, lh, kind) = (c.u32()?, c.u32()?, c.u32()?);
        let name = c.string()?;
        // Charge the owned name against the document-wide metadata budget: repeated layer
        // offsets referencing one large name must not retain unbounded copies.
        self.meta_budget = self.meta_budget.checked_sub(name.len()).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
        let lprops = props(&mut c)?;
        let hptr = c.ptr()?;
        let mptr = c.ptr()?;
        let mut effects = 0usize;
        if self.version >= 20 {
            while c.ptr()? != 0 {
                effects += 1;
                if effects > 1024 {
                    return Err(invalid("too many layer effects"));
                }
            }
        }
        let (type_rgb, alpha) = match kind {
            0 => (true, false),
            1 => (true, true),
            2 | 4 => (false, false),
            3 | 5 => (false, true),
            k => return Err(invalid(format!("unknown layer type {k} on '{name}'"))),
        };
        if type_rgb != (self.base_type != 1) && self.base_type != 2 {
            return Err(invalid(format!("layer '{name}' does not match the image's colour model")));
        }

        let mut layer_mode = 0u32;
        let mut opacity = 1.0f32;
        // PROP_FLOAT_OPACITY wins over PROP_OPACITY whatever their order in the file.
        let mut float_opacity = None;
        let mut visible = true;
        let (mut x, mut y) = (0i32, 0i32);
        let mut apply_mask = true;
        let mut group = false;
        let mut expanded = true;
        let mut path = Vec::new();
        let mut text = false;
        let mut vector = false;
        let mut link = false;
        let mut floating = false;
        let mut clipped = false;
        let mut composite_mode = 0i32;
        let mut composite_space = 0i32;
        let mut blend_space = 0i32;
        let mut locks = photocraft_doc::Locks::default();
        let mut tag = LabelColor::None;
        for p in &lprops {
            let v = be_u32(p.data).unwrap_or(0);
            match p.kind {
                5 => floating = true,
                6 => opacity = (v.min(255) as f32) / 255.0,
                33 => float_opacity = be_f32(p.data).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)).or(float_opacity),
                7 => layer_mode = v,
                8 => visible = v != 0,
                10 => locks.transparency = v != 0,
                11 => apply_mask = v != 0,
                15 => {
                    x = be_i32(p.data).unwrap_or(0);
                    y = p.data.get(4..).and_then(be_i32).unwrap_or(0);
                }
                26 => text = true,
                28 => locks.pixels = v != 0,
                29 => group = true,
                30 => {
                    path = p.data.as_chunks::<4>().0.iter().map(|s| u32::from_be_bytes(*s)).collect();
                    self.meta_budget =
                        self.meta_budget.checked_sub(path.len().saturating_mul(4)).ok_or_else(|| invalid("layer metadata exceeds the budget"))?;
                }
                31 => expanded = v & 1 != 0,
                32 => locks.position = v != 0,
                34 => tag = label(v),
                35 => composite_mode = be_i32(p.data).unwrap_or(0),
                36 => composite_space = be_i32(p.data).unwrap_or(0),
                37 => blend_space = be_i32(p.data).unwrap_or(0),
                47 => vector = true,
                48 => link = true,
                21 => {
                    for (pname, _) in parasites(p.data) {
                        match pname.as_str() {
                            "gimphoto-clipped" => clipped = true,
                            "gimp-text-layer" => text = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if floating {
            self.warnings.layer(&name, "is a floating selection and was not imported (anchor it in GIMP first)");
            return Ok(None);
        }

        let (blend, note) = blend_mode(layer_mode);
        if let Some(note) = note {
            self.warnings.layer(
                &name,
                &format!(
                    "uses GIMP's {note} mode, which opens as {}",
                    if blend == BlendMode::Normal { "Normal".to_string() } else { format!("Photoshop's {blend:?}") }
                ),
            );
        }
        match composite_mode.unsigned_abs() {
            0 | 1 => {}
            2 => self.clipped_to_backdrop += 1,
            3 => self.warnings.layer(&name, "composites with Clip to layer, which is not reproduced"),
            4 => self.warnings.layer(&name, "composites with Intersection, which is not reproduced"),
            _ => {}
        }
        if text {
            self.warnings.layer(&name, "is a text layer and opens as pixels");
        }
        if vector {
            self.warnings.layer(&name, "is a vector layer and opens as pixels");
        }
        if link {
            self.warnings.layer(&name, "is a link layer and opens as pixels");
        }
        if effects > 0 {
            self.warnings.layer(&name, &format!("has {effects} layer effect(s) (non-destructive filters) that are not applied"));
        }

        let mut layer = if group {
            let mut g = Layer::group(name.clone(), Vec::new());
            if let LayerContent::Group(gr) = &mut g.content {
                gr.expanded = expanded;
            }
            g.blend = if blend == BlendMode::PassThrough { BlendMode::PassThrough } else { blend };
            g
        } else {
            let mut l = Layer::raster(name.clone(), fmt);
            l.blend = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
            l
        };
        layer.opacity = float_opacity.unwrap_or(opacity);
        layer.visible = visible;
        layer.locks = locks;
        layer.label = tag;
        layer.clipped = clipped;

        if !group && hptr != 0 {
            let source_channels = match (self.base_type, alpha) {
                (2, false) => 1,
                (2, true) => 2,
                (1, false) => 1,
                (1, true) => 2,
                (_, false) => 3,
                (_, true) => 4,
            };
            let mut px = self.hierarchy(hptr, source_channels, ctl)?;
            if (px.width, px.height) != (lw, lh) {
                return Err(invalid(format!("layer '{name}' pixels are {}x{} where {lw}x{lh} were declared", px.width, px.height)));
            }
            if !alpha {
                // An opaque layer: add the alpha channel the document's pixel format carries.
                let n = px.channels;
                let mut data = Vec::with_capacity(px.data.len() / n * (n + 1));
                for p in px.data.chunks_exact(n) {
                    data.extend_from_slice(p);
                    data.push(1.0);
                }
                px = Pixels { width: px.width, height: px.height, channels: n + 1, data };
            }
            let mut surface = Surface::new(fmt);
            write_pixels(&mut surface, &px, x, y)?;
            layer.content = LayerContent::Raster(surface);
            if !alpha {
                layer.locks.transparency = true;
            }
        }
        if mptr != 0 {
            let (_mname, _mprops, px) = self.channel(mptr, ctl)?;
            let mut surface = Surface::new(mask_fmt);
            write_pixels(&mut surface, &px, x, y)?;
            layer.mask = Some(LayerMask { surface, enabled: apply_mask, linked: true, density: 1.0, feather: 0.0 });
        }
        let linear = composite_space.unsigned_abs() == 1 || blend_space.unsigned_abs() == 1;
        Ok(Some((Node { layer, path, group, children: Vec::new() }, linear)))
    }
}
