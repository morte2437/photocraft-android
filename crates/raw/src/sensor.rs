//! Undeveloped sensor data and the shared strip/tile reader.

use crate::color::ColorInfo;
use crate::error::{RawError, Result};
use crate::tiff::{Ifd, Tiff, tag};
use crate::{Limits, RawFormat, jxl, ljpeg, par};

/// A rectangle in sensor-data pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Rect { x, y, width, height }
    }

    /// The overlap of two rectangles (may be empty).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x.saturating_add(self.width)).min(o.x.saturating_add(o.width));
        let y1 = (self.y.saturating_add(self.height)).min(o.y.saturating_add(o.height));
        Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Colour of one CFA cell: 0 = red, 1 = green, 2 = blue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cfa {
    pub width: usize,
    pub height: usize,
    /// Row-major colours, `width * height` entries.
    pub colors: Vec<u8>,
    /// Data coordinates of the pattern's top-left cell.
    pub origin_x: usize,
    pub origin_y: usize,
}

impl Cfa {
    /// A 2×2 pattern from a string such as `"RGGB"`, anchored at (0, 0).
    pub fn bayer(pattern: &str) -> Option<Cfa> {
        let colors: Vec<u8> = pattern
            .chars()
            .map(|c| match c {
                'R' => Some(0),
                'G' => Some(1),
                'B' => Some(2),
                _ => None,
            })
            .collect::<Option<_>>()?;
        (colors.len() == 4).then_some(Cfa { width: 2, height: 2, colors, origin_x: 0, origin_y: 0 })
    }

    /// Colour at data coordinates.
    #[inline]
    pub fn color(&self, x: usize, y: usize) -> u8 {
        fn relative(v: usize, origin: usize, period: usize) -> Option<usize> {
            if period == 0 {
                return None;
            }
            let (v, origin) = (v % period, origin % period);
            Some(if v >= origin { v - origin } else { period - (origin - v) })
        }
        let Some((cx, cy)) = relative(x, self.origin_x, self.width).zip(relative(y, self.origin_y, self.height)) else {
            return 1;
        };
        cy.checked_mul(self.width).and_then(|i| i.checked_add(cx)).and_then(|i| self.colors.get(i)).copied().unwrap_or(1)
    }

    /// The 2×2 colours seen from data coordinates (`x0`, `y0`), row-major.
    pub fn phase(&self, x0: usize, y0: usize) -> [u8; 4] {
        [self.color(x0, y0), self.color(x0.saturating_add(1), y0), self.color(x0, y0.saturating_add(1)), self.color(x0.saturating_add(1), y0.saturating_add(1))]
    }

    /// `true` for a Bayer pattern: a 2×2 period holding one red, one blue and two
    /// diagonal greens.
    pub fn is_bayer(&self) -> bool {
        if self.width == 0 || self.height == 0 || self.width > 16 || self.height > 16 {
            return false;
        }
        let (ox, oy) = (self.origin_x % self.width, self.origin_y % self.height);
        let p = self.phase(ox, oy);
        let periodic = (0..self.height).all(|y| (0..self.width).all(|x| self.color(ox + x, oy + y) == p[(y % 2) * 2 + x % 2]));
        let greens_on_diagonal = (p[0] == 1 && p[3] == 1 && p[1] != 1 && p[2] != 1 && u16::from(p[1]) + u16::from(p[2]) == 2)
            || (p[1] == 1 && p[2] == 1 && p[0] != 1 && p[3] != 1 && u16::from(p[0]) + u16::from(p[3]) == 2);
        periodic && greens_on_diagonal
    }
}

/// Black levels (DNG BlackLevel / BlackLevelRepeatDim / BlackLevelDeltaH/V).
#[derive(Debug, Clone, PartialEq)]
pub struct BlackLevels {
    pub rows: usize,
    pub cols: usize,
    /// `rows * cols * samples` values, sample fastest.
    pub values: Vec<f32>,
    /// Per column of the active area.
    pub delta_h: Vec<f32>,
    /// Per row of the active area.
    pub delta_v: Vec<f32>,
}

impl BlackLevels {
    pub fn uniform(v: f32) -> Self {
        BlackLevels { rows: 1, cols: 1, values: vec![v], delta_h: Vec::new(), delta_v: Vec::new() }
    }

    /// Black level at active-area coordinates for sample `s` of `spp`.
    #[inline]
    pub fn at(&self, ax: usize, ay: usize, s: usize, spp: usize) -> f32 {
        let r = ay % self.rows.max(1);
        let c = ax % self.cols.max(1);
        let index = r.checked_mul(self.cols.max(1)).and_then(|i| i.checked_add(c)).and_then(|i| i.checked_mul(spp)).and_then(|i| i.checked_add(s));
        let base = index.and_then(|i| self.values.get(i)).or_else(|| self.values.first()).copied().unwrap_or(0.0);
        base + self.delta_h.get(ax).copied().unwrap_or(0.0) + self.delta_v.get(ay).copied().unwrap_or(0.0)
    }

    /// `true` when there is a single black value for everything.
    pub fn is_uniform(&self) -> bool {
        self.delta_h.iter().all(|v| *v == 0.0) && self.delta_v.iter().all(|v| *v == 0.0) && self.values.windows(2).all(|w| w[0] == w[1])
    }
}

/// Undeveloped sensor data with everything needed to develop it.
#[derive(Debug, Clone)]
pub struct Sensor {
    pub format: RawFormat,
    pub make: Option<String>,
    pub model: Option<String>,
    /// Data width and height in pixels.
    pub width: usize,
    pub height: usize,
    /// Samples per pixel: 1 for CFA data, 3 for LinearRaw (1 for monochrome LinearRaw).
    pub samples: usize,
    /// Row-major interleaved samples, `width * height * samples`.
    pub data: Vec<u16>,
    /// The CFA pattern (CFA data only).
    pub cfa: Option<Cfa>,
    /// Maps stored values to linear values (DNG LinearizationTable).
    pub linearization: Option<Vec<u16>>,
    pub black: BlackLevels,
    /// White (saturation) level per sample.
    pub white: [f32; 3],
    /// Sensor area holding image data (DNG ActiveArea); black-level deltas are indexed from it.
    pub active: Rect,
    /// The area to output (ActiveArea ∩ DefaultCrop).
    pub crop: Rect,
    pub color: ColorInfo,
    /// White-balance multipliers from the camera's own metadata (non-DNG), R G B.
    pub camera_wb: Option<[f64; 3]>,
    /// TIFF orientation (1–8).
    pub orientation: u16,
    /// Exposure compensation baked into the default rendering (DNG BaselineExposure), in EV.
    pub baseline_exposure: f64,
    /// DNG OpcodeList2 gain maps (lens shading), applied to the normalized data.
    pub gain_maps: Vec<crate::opcodes::GainMap>,
    /// The camera profile's default tone curve (like DNG ProfileToneCurve): increasing `[input,
    /// output]` pairs in linear ProPhoto, applied per channel after the colour conversion and
    /// exposure. Below the first point the curve is a line through 0; empty: no curve.
    pub tone_curve: Vec<[f32; 2]>,
    /// Camera Raw's adaptive black point, for profiles that copy its default rendering: this
    /// factor times the 0.1th percentile of the image's linear luminance is subtracted before the
    /// tone curve (see `tone.rs`). 0: no black point.
    pub black_point: f32,
    pub warnings: Vec<String>,
}

impl Sensor {
    /// `true` for monochrome data (one-sample LinearRaw: monochrome cameras, scanner DNGs), which
    /// develops to neutral grey with no white balance or colour calibration.
    pub fn is_monochrome(&self) -> bool {
        self.samples == 1 && self.cfa.is_none()
    }
}

/// Raw samples read from one IFD.
pub(crate) struct Plane {
    pub width: usize,
    pub height: usize,
    pub samples: usize,
    pub bits: u32,
    pub data: Vec<u16>,
    /// For floating-point samples, the factor they were multiplied by (their white level becomes
    /// 65535, `bits` is then 16): the file's black and white levels are scaled by it too.
    pub float_scale: Option<f32>,
}

pub(crate) struct Segment {
    pub x: usize,
    pub y: usize,
    /// Encoded width and height (tile size, or image width × rows per strip).
    pub w: usize,
    pub h: usize,
    pub offset: usize,
    pub len: usize,
}

/// How a lossless-JPEG tile's samples map onto the tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JpegLayout {
    /// The decoded sample sequence fills the tile row by row (DNG).
    Flat,
    /// A 4-component frame of half the tile's width and height: each JPEG
    /// pixel holds a 2×2 block, row-major (observed in Sony lossless ARW).
    Quads,
}

/// DNG 1.7 JPEG XL compression.
pub(crate) const JPEG_XL: u32 = 52546;

/// Inflates one zlib tile into at most `need` bytes. The stream must run to its end: a truncated
/// stream, or one that holds more than `need` bytes, is an error rather than a silent cut-off.
fn inflate_tile(src: &[u8], need: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; need];
    let mut d = flate2::Decompress::new(true);
    let status = d.decompress(src, &mut out, flate2::FlushDecompress::Finish).map_err(|_| RawError::malformed("deflate tile does not decompress"))?;
    if status != flate2::Status::StreamEnd {
        return Err(RawError::malformed("deflate tile is truncated or larger than its tile"));
    }
    out.truncate(usize::try_from(d.total_out()).unwrap_or(need).min(need));
    Ok(out)
}

/// Reads the (uncompressed, deflate, lossless-JPEG, lossy-JPEG or JPEG XL) image of a raw IFD.
pub(crate) fn read_plane(t: &Tiff, ifd: &Ifd, limits: &Limits, layout: JpegLayout) -> Result<Plane> {
    let width = t.tag_uint(ifd, tag::IMAGE_WIDTH).ok_or_else(|| RawError::malformed("raw image has no width"))? as usize;
    let height = t.tag_uint(ifd, tag::IMAGE_LENGTH).ok_or_else(|| RawError::malformed("raw image has no height"))? as usize;
    let samples = t.tag_uint(ifd, tag::SAMPLES_PER_PIXEL).unwrap_or(1) as usize;
    let bits = t.tag_uint(ifd, tag::BITS_PER_SAMPLE).unwrap_or(1);
    let compression = t.tag_uint(ifd, tag::COMPRESSION).unwrap_or(1);
    if !(1..=4).contains(&samples) {
        return Err(RawError::unsupported(format!("{samples} samples per pixel")));
    }
    // Floating-point data is read when it is JPEG XL-compressed, as converters write lossy
    // LinearRaw (DNG 1.7, #8125): 16, 24 or 32-bit floats, read as 16-bit samples.
    let float = t.tag_uint(ifd, tag::SAMPLE_FORMAT) == Some(3);
    if float && compression != JPEG_XL {
        return Err(RawError::unsupported("floating-point raw data that isn't JPEG XL-compressed"));
    }
    if float && !matches!(bits, 16 | 24 | 32) {
        return Err(RawError::unsupported(format!("{bits}-bit floating-point raw samples")));
    }
    let bits = if float { 16 } else { bits };
    // The floats' highest white level (default 1.0; DNG 1.7 WhiteLevel has one per sample plane)
    // becomes 65535, so no plane is clipped below its own white level.
    let float_scale = float.then(|| {
        let white = t.tag_floats(ifd, tag::WHITE_LEVEL).iter().map(|v| *v as f32).filter(|v| v.is_finite() && *v > 0.0).fold(0.0f32, f32::max);
        65535.0 / if white > 0.0 { white } else { 1.0 }
    });
    if !(1..=16).contains(&bits) {
        return Err(RawError::unsupported(format!("{bits}-bit raw samples")));
    }
    if samples > 1 && t.tag_uint(ifd, tag::PLANAR_CONFIGURATION) == Some(2) {
        return Err(RawError::unsupported("planar raw data"));
    }
    match compression {
        1 | 7 | 8 | 34892 | JPEG_XL => {}
        34713 => return Err(RawError::unsupported("Nikon compressed NEF")),
        32767 => return Err(RawError::unsupported("Sony compressed ARW")),
        65535 => return Err(RawError::unsupported("Pentax compressed PEF")),
        c => return Err(RawError::unsupported(format!("raw compression scheme {c}"))),
    }
    limits.check(width as u64, height as u64, samples as u64 * 2)?;
    let segments = segments(t, ifd, width, height)?;
    // The decoded tiles are all retained in `decoded` before assembly copies only the useful
    // pixels. Each tile is charged individually below, but file-controlled padding and repeated
    // source ranges can make the *cumulative* retained samples far exceed both the input and the
    // useful image. Reject that up front: the sum of padded tile samples (two bytes each) must
    // fit the allocation budget (checked per segment, #4959).
    retained_segment_bytes(&segments, samples, limits)?;
    let le = t.le;
    let tiled = t.tag_uint(ifd, tag::TILE_WIDTH).is_some();
    let decoded: Vec<Result<Vec<u16>>> = par::map(segments.len(), |i| {
        let s = &segments[i];
        let rows = s.h.min(height - s.y);
        let src = t.bytes(s.offset, s.len).ok_or_else(|| RawError::malformed("raw data lies outside the file"))?;
        // Every branch below sizes at least one buffer from the segment's own geometry, which
        // the file declares independently of the image: a tile can be far larger than the image
        // it belongs to, so the image check above does not bound those buffers. Charge the
        // segment itself before decoding it (#2925).
        limits.check(s.w as u64, s.h as u64, samples as u64 * 2)?;
        match compression {
            1 => unpack(src, s.w, rows, samples, bits, le),
            8 => {
                // Adobe DNG compression 8: zlib streams (RFC 1950). The output is capped at the
                // tile's size in whole bytes per sample, so a bomb cannot grow past the tile;
                // `unpack` then reads it bit-packed or in 16-bit containers by its length.
                let too_big = || RawError::malformed("deflate tile is too large");
                let need =
                    s.w.checked_mul(rows).and_then(|n| n.checked_mul(samples)).and_then(|n| n.checked_mul((bits as usize).div_ceil(8))).ok_or_else(too_big)?;
                let out = inflate_tile(src, need)?;
                let v = unpack(&out, s.w, rows, samples, bits, le)?;
                // The TIFF predictor runs after unpacking, per row, per sample plane.
                match t.tag_uint(ifd, tag::PREDICTOR).unwrap_or(1) {
                    1 => Ok(v),
                    2 => Ok(undelta(v, samples, s.w * samples, rows, bits)),
                    3 => Err(RawError::unsupported("floating-point predictor (compression 8, predictor 3)")),
                    p => Err(RawError::malformed(format!("unknown predictor {p}"))),
                }
            }
            // A JPEG XL segment codes only the rows inside the image (strips) or the whole tile.
            JPEG_XL => {
                let coded_rows = if tiled { s.h } else { rows };
                let mut v = jxl::decode(src, s.w, coded_rows, samples, bits, float_scale, limits)?;
                v.resize(s.w * s.h * samples, 0);
                Ok(v)
            }
            34892 => lossy_jpeg(src, s, rows, samples, bits, tiled, limits),
            // Compression 7 (lossless JPEG).
            _ => lossless_jpeg(src, s, rows, samples, layout, limits),
        }
    });
    // One segment holding the whole image row for row (a single strip): its buffer is the plane, no
    // second full-size buffer and no copy. The scatter below then has no segment left to place.
    let whole = matches!(segments.as_slice(), [s] if s.x == 0 && s.y == 0 && s.w == width && s.h >= height);
    let mut decoded = decoded.into_iter();
    let taken = if whole {
        let mut d = decoded.next().ok_or_else(|| RawError::malformed("raw segment missing"))??;
        if d.len() < width * height * samples {
            return Err(RawError::malformed("raw segment is smaller than the image"));
        }
        d.truncate(width * height * samples);
        Some(d)
    } else {
        None
    };
    let mut data = taken.unwrap_or_else(|| vec![0u16; width * height * samples]);
    for (s, d) in segments.iter().zip(decoded) {
        let d = d?;
        let rows = s.h.min(height - s.y);
        let cols = s.w.min(width - s.x);
        for r in 0..rows {
            let src = &d[r * s.w * samples..(r * s.w + cols) * samples];
            let at = ((s.y + r) * width + s.x) * samples;
            data[at..at + cols * samples].copy_from_slice(src);
        }
    }
    Ok(Plane { width, height, samples, bits, data, float_scale })
}

/// Bytes of decoded samples the segments hold at once.
///
/// Each segment is charged against `limits` on its own just before it decodes, but every decoded
/// tile is retained until the scatter at the end of [`read_plane`], so what is alive at once is
/// their sum (plus the output plane the scatter fills), not the largest of them. A segment's geometry is declared independently of the image, so a
/// file can carry many segments that are each admissible and together are not (#4959). For a
/// well-formed file the segments tile the image, so their sum is about its own size, which
/// `limits.check` has already bounded.
fn retained_segment_bytes(segments: &[Segment], samples: usize, limits: &Limits) -> Result<u64> {
    let mut total: u64 = 0;
    for s in segments {
        let bytes = (s.w as u64)
            .checked_mul(s.h as u64)
            .and_then(|px| px.checked_mul(samples as u64 * 2))
            .ok_or_else(|| RawError::LimitExceeded("a raw segment is too large to decode".into()))?;
        total = total.saturating_add(bytes);
        if total > limits.max_alloc {
            return Err(RawError::LimitExceeded(format!("{total} bytes of decoded segments exceed {}", limits.max_alloc)));
        }
    }
    Ok(total)
}

pub(crate) fn segments(t: &Tiff, ifd: &Ifd, width: usize, height: usize) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    if let (Some(tw), Some(th)) = (t.tag_uint(ifd, tag::TILE_WIDTH), t.tag_uint(ifd, tag::TILE_LENGTH)) {
        let (tw, th) = (tw as usize, th as usize);
        if tw == 0 || th == 0 || tw > 1 << 16 || th > 1 << 16 {
            return Err(RawError::malformed("bad tile size"));
        }
        let across = width.div_ceil(tw);
        let down = height.div_ceil(th);
        let offsets = t.tag_uints(ifd, tag::TILE_OFFSETS);
        let counts = t.tag_uints(ifd, tag::TILE_BYTE_COUNTS);
        let n = across.checked_mul(down).ok_or_else(|| RawError::malformed("too many tiles"))?;
        if offsets.len() < n || counts.len() < n {
            return Err(RawError::malformed("missing tile offsets"));
        }
        for i in 0..n {
            out.push(Segment { x: (i % across) * tw, y: (i / across) * th, w: tw, h: th, offset: offsets[i] as usize, len: counts[i] as usize });
        }
    } else {
        let rps = match t.tag_uint(ifd, tag::ROWS_PER_STRIP) {
            Some(r) if r > 0 => (r as usize).min(height),
            _ => height,
        };
        let n = height.div_ceil(rps.max(1));
        let offsets = t.tag_uints(ifd, tag::STRIP_OFFSETS);
        let mut counts = t.tag_uints(ifd, tag::STRIP_BYTE_COUNTS);
        if offsets.len() < n {
            return Err(RawError::malformed("missing strip offsets"));
        }
        if counts.len() < n {
            if n == 1 {
                counts = vec![(t.data.len().saturating_sub(offsets[0] as usize)) as u32];
            } else {
                return Err(RawError::malformed("missing strip byte counts"));
            }
        }
        for i in 0..n {
            out.push(Segment { x: 0, y: i * rps, w: width, h: rps, offset: offsets[i] as usize, len: counts[i] as usize });
        }
    }
    Ok(out)
}

/// Decodes one compression-7 (lossless JPEG) segment to the segment's own samples.
///
/// The frame the entropy stream declares is capped by the budget, not only by the segment, and
/// the `Quads` scatter buffer comes out of the same budget. `max_alloc` counts bytes, a sample is two.
fn lossless_jpeg(src: &[u8], s: &Segment, rows: usize, samples: usize, layout: JpegLayout, limits: &Limits) -> Result<Vec<u16>> {
    let need = s.w * rows * samples;
    let scatter = if layout == JpegLayout::Quads && samples == 1 { s.w.saturating_mul(s.h).saturating_mul(2) as u64 } else { 0 };
    let budget = usize::try_from(limits.max_alloc.saturating_sub(scatter) / 2).unwrap_or(usize::MAX);
    let geom = s.w.saturating_mul(s.h).saturating_mul(samples).saturating_mul(4);
    let (f, mut v) = ljpeg::decode(src, geom.min(budget))?;
    if v.len() < need {
        return Err(RawError::malformed("lossless JPEG tile is smaller than its tile"));
    }
    if layout == JpegLayout::Quads && samples == 1 && f.components == 4 && f.width * 2 == s.w && f.height * 2 == s.h {
        let mut tile = vec![0u16; s.w * s.h];
        for (i, q) in v.as_chunks::<4>().0.iter().enumerate() {
            let at = 2 * (i / f.width) * s.w + 2 * (i % f.width);
            tile[at] = q[0];
            tile[at + 1] = q[1];
            tile[at + s.w] = q[2];
            tile[at + s.w + 1] = q[3];
        }
        return Ok(tile);
    }
    // The frame may declare up to four times the segment's samples; the scatter reads only the
    // segment's own rows, and the frame is retained until then, so keep just those (#4959).
    v.truncate(need);
    v.shrink_to_fit();
    Ok(v)
}

/// Decodes one segment of a lossy DNG (Adobe DNG compression 34892): a complete baseline-DCT
/// JPEG stream (ITU-T T.81), 8 bits per sample, as DNG 1.4 defines it. A CFA segment is a
/// one-component (gray) JPEG; a 3-sample LinearRaw segment is a three-component one in YCbCr,
/// which the JPEG decoder turns back into the stored R, G, B. The decoded 8-bit values are the
/// stored sample values: the LinearizationTable, BlackLevel and WhiteLevel apply to them later,
/// as for every other compression, and WhiteLevel defaults to 255 for 8-bit data.
///
/// `rows` is the number of segment rows inside the image. A tile is always coded at the full
/// tile size (edge tiles are padded); a strip is coded at its row count (the last strip is
/// shorter). Either height is accepted, the width must be the segment's. The frame header is
/// checked before any pixel is decoded, and the decoder's own size cap is the segment, so a
/// hostile SOF cannot size a buffer larger than the tile the IFD declares (which `limits` has
/// already charged).
fn lossy_jpeg(src: &[u8], s: &Segment, rows: usize, samples: usize, bits: u32, tiled: bool, limits: &Limits) -> Result<Vec<u16>> {
    use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    if bits != 8 {
        return Err(RawError::unsupported(format!("lossy DNG with {bits}-bit samples (lossy JPEG tiles are 8-bit)")));
    }
    let out = match samples {
        1 => ColorSpace::Luma,
        3 => ColorSpace::RGB,
        n => return Err(RawError::unsupported(format!("lossy DNG with {n} samples per pixel (1 or 3 are supported)"))),
    };
    let options = DecoderOptions::default().jpeg_set_out_colorspace(out).set_max_width(s.w).set_max_height(s.h);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(src), options);
    dec.decode_headers().map_err(|e| RawError::malformed(format!("lossy JPEG tile header: {e:?}")))?;
    let f = dec.info().ok_or_else(|| RawError::malformed("lossy JPEG tile has no header"))?;
    let (fw, fh) = (usize::from(f.width), usize::from(f.height));
    let coded_rows = if tiled { s.h } else { rows };
    if fw != s.w || (fh != coded_rows && fh != rows) {
        return Err(RawError::malformed(format!("lossy JPEG tile is {fw}x{fh}, expected {}x{coded_rows}", s.w)));
    }
    if usize::from(f.components) != samples {
        return Err(RawError::malformed(format!("lossy JPEG tile has {} components, expected {samples}", f.components)));
    }
    let need = fw.checked_mul(fh).and_then(|n| n.checked_mul(samples)).ok_or_else(|| RawError::malformed("lossy JPEG tile is too large"))?;
    limits.check(fw as u64, fh as u64, samples as u64 * 2)?;
    let decoded = dec.decode().map_err(|e| RawError::malformed(format!("lossy JPEG tile: {e:?}")))?;
    if decoded.len() != need {
        return Err(RawError::malformed(format!("lossy JPEG tile holds {} samples, expected {need}", decoded.len())));
    }
    let mut v: Vec<u16> = decoded.into_iter().map(u16::from).collect();
    // A strip coded at its own row count still fills the segment's declared height.
    let full = s.w.checked_mul(s.h).and_then(|n| n.checked_mul(samples)).ok_or_else(|| RawError::malformed("lossy JPEG tile is too large"))?;
    v.resize(full, 0);
    Ok(v)
}

/// Reverses TIFF predictor 2 (horizontal differencing) on interleaved samples: each sample
/// gains its channel's predecessor in the row, wrapping at the sample's container (8 or 16 bits).
fn undelta(mut out: Vec<u16>, plane: usize, row_samples: usize, rows: usize, bits: u32) -> Vec<u16> {
    let mask: u16 = if bits <= 8 { 0xFF } else { 0xFFFF };
    for row in out.chunks_mut(row_samples.max(1)).take(rows) {
        for i in plane..row.len() {
            if let (Some(&prev), Some(&cur)) = (row.get(i - plane), row.get(i)) {
                row[i] = cur.wrapping_add(prev) & mask;
            }
        }
    }
    out
}

/// Unpacks uncompressed samples: 8-bit, 16-bit in the file's byte order, or
/// MSB-first bit-packed rows padded to a byte (TIFF 6.0). 9–15-bit data
/// stored in 16-bit containers (common in camera raws) is recognised by its
/// byte count.
fn unpack(src: &[u8], w: usize, rows: usize, samples: usize, bits: u32, le: bool) -> Result<Vec<u16>> {
    let n = w * rows * samples;
    let truncated = || RawError::malformed("uncompressed raw data is truncated");
    if bits == 16 || (bits > 8 && src.len() >= n * 2) {
        let b = src.get(..n * 2).ok_or_else(truncated)?;
        return Ok(b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect());
    }
    if bits == 8 {
        return Ok(src.get(..n).ok_or_else(truncated)?.iter().map(|&v| u16::from(v)).collect());
    }
    let row_bytes = (w * samples * bits as usize).div_ceil(8);
    if src.len() < row_bytes * rows {
        return Err(truncated());
    }
    // The common packings, a whole group of samples per fixed run of bytes (MSB first).
    let per_row = w * samples;
    match bits {
        12 | 14 | 10 => {
            let mut out = vec![0u16; n];
            for (orow, row) in out.chunks_exact_mut(per_row.max(1)).zip(src.chunks_exact(row_bytes.max(1))) {
                unpack_row_msb(row, orow, bits);
            }
            return Ok(out);
        }
        _ => {}
    }
    let mut out = Vec::with_capacity(n);
    for r in 0..rows {
        let row = &src[r * row_bytes..(r + 1) * row_bytes];
        let mut acc: u32 = 0;
        let mut have = 0u32;
        let mut it = row.iter();
        for _ in 0..w * samples {
            while have < bits {
                acc = (acc << 8) | u32::from(*it.next().unwrap_or(&0));
                have += 8;
            }
            have -= bits;
            out.push(((acc >> have) & ((1 << bits) - 1)) as u16);
        }
    }
    Ok(out)
}

/// One MSB-first bit-packed row of 10, 12 or 14-bit samples (`row` holds at least
/// `out.len() * bits / 8` bytes, rounded up; missing bytes read as 0, as [`unpack`] does).
fn unpack_row_msb(row: &[u8], out: &mut [u16], bits: u32) {
    let mask = (1u64 << bits) - 1;
    // Samples per group and bytes per group: 4 × 10 = 40, 2 × 12 = 24, 4 × 14 = 56 bits.
    let (g, gb) = match bits {
        10 => (4, 5),
        12 => (2, 3),
        _ => (4, 7),
    };
    let groups = out.len() / g;
    for (o, b) in out[..groups * g].chunks_exact_mut(g).zip(row.chunks_exact(gb)) {
        let mut acc = 0u64;
        for &x in b {
            acc = (acc << 8) | u64::from(x);
        }
        for (k, v) in o.iter_mut().enumerate() {
            *v = ((acc >> ((g - 1 - k) as u32 * bits)) & mask) as u16;
        }
    }
    // The tail (fewer than a group): the general reader.
    let mut acc: u32 = 0;
    let mut have = 0u32;
    let mut it = row.iter().skip(groups * gb);
    for v in &mut out[groups * g..] {
        while have < bits {
            acc = (acc << 8) | u32::from(*it.next().unwrap_or(&0));
            have += 8;
        }
        have -= bits;
        *v = ((acc >> have) & ((1 << bits) - 1)) as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grouped 10/12/14-bit readers equal the plain bit reader on every row width.
    #[test]
    fn grouped_unpack_matches_the_bit_reader() {
        let plain = |src: &[u8], w: usize, rows: usize, bits: u32| {
            let row_bytes = (w * bits as usize).div_ceil(8);
            let mut out = Vec::new();
            for r in 0..rows {
                let row = &src[r * row_bytes..(r + 1) * row_bytes];
                let (mut acc, mut have, mut it) = (0u32, 0u32, row.iter());
                for _ in 0..w {
                    while have < bits {
                        acc = (acc << 8) | u32::from(*it.next().unwrap_or(&0));
                        have += 8;
                    }
                    have -= bits;
                    out.push(((acc >> have) & ((1 << bits) - 1)) as u16);
                }
            }
            out
        };
        let mut s = 7u64;
        for bits in [10u32, 12, 14] {
            for samples in [1usize, 3] {
                // Widths whose packed rows are shorter than 16-bit containers (else `unpack` reads containers).
                for w in [4usize, 5, 7, 8, 13, 64, 67] {
                    let rows = 3;
                    let src: Vec<u8> = (0..(w * samples * bits as usize).div_ceil(8) * rows)
                        .map(|_| {
                            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                            (s >> 40) as u8
                        })
                        .collect();
                    assert_eq!(
                        unpack(&src, w, rows, samples, bits, true).unwrap(),
                        plain(&src, w * samples, rows, bits),
                        "{bits} bits, {samples} samples, {w} wide"
                    );
                }
            }
        }
    }

    /// #4959: a decoded segment is retained until the scatter, so the segments are charged
    /// together. Each of these is far inside the budget on its own; their sum is not.
    #[test]
    fn segments_are_charged_together_rather_than_one_at_a_time() {
        let limits = Limits { max_alloc: 8 << 20, ..Limits::default() };
        // 1024 x 512 samples, two bytes each: one mebibyte a tile.
        let tile = |i: usize| Segment { x: 0, y: i * 512, w: 1024, h: 512, offset: 0, len: 0 };
        let eight: Vec<Segment> = (0..8).map(tile).collect();
        assert_eq!(retained_segment_bytes(&eight, 1, &limits).unwrap(), 8 << 20, "the budget is spent exactly");

        let nine: Vec<Segment> = (0..9).map(tile).collect();
        assert!(matches!(retained_segment_bytes(&nine, 1, &limits), Err(RawError::LimitExceeded(_))), "one tile over");
        assert!(retained_segment_bytes(&eight, 3, &limits).is_err(), "more samples a pixel cost proportionally more");

        // A segment whose own geometry overflows is refused rather than wrapping to a small sum.
        let huge = [Segment { x: 0, y: 0, w: usize::MAX, h: usize::MAX, offset: 0, len: 0 }];
        assert!(matches!(retained_segment_bytes(&huge, 1, &limits), Err(RawError::LimitExceeded(_))));

        // A normal tiled frame is unaffected: 6000 x 4000 in 512-pixel tiles, padding included.
        let normal: Vec<Segment> = (0..12 * 8).map(|i| Segment { x: (i % 12) * 512, y: (i / 12) * 512, w: 512, h: 512, offset: 0, len: 0 }).collect();
        assert!(retained_segment_bytes(&normal, 1, &Limits::default()).is_ok());
    }

    /// #4959 end to end: the sum is charged in `read_plane`, not only by the helper. An 18x18
    /// CFA in four padded 16x16 lossless-JPEG tiles: the image (648 B) and each tile (512 B) fit
    /// a 1500 B budget, the four retained tiles (2048 B) do not.
    #[test]
    fn decode_charges_the_retained_tiles_together() {
        use crate::testgen::{DngSpec, DngStorage, mosaic, scene};
        let mut d = DngSpec::cfa(18, 18, mosaic(&scene(18, 18), 18, [0, 1, 1, 2], 64, 4000));
        d.bits = 12;
        d.white = 4000;
        d.black = vec![64];
        d.storage = DngStorage::Lj92Tiles { width: 16, height: 16 };
        let b = d.build();
        let tight = Limits { max_alloc: 1500, ..Limits::default() };
        assert!(matches!(crate::decode(&b, &tight), Err(RawError::LimitExceeded(_))));
        let exact = Limits { max_alloc: 2048, ..Limits::default() };
        assert!(crate::decode(&b, &exact).is_ok(), "the padded tiles exactly at the budget still decode");
    }

    /// #4959: a lossless-JPEG frame may declare up to four times its segment's samples. The
    /// segment is charged for its own size only, so it must keep only that until the scatter.
    #[test]
    fn lossless_jpeg_keeps_only_the_segment() {
        let frame: Vec<u16> = (0..64 * 64).map(|i| (i % 4096) as u16).collect();
        let src = crate::testgen::lj92_encode(&frame, 64, 64, 1, 12, 1, None);
        let s = Segment { x: 0, y: 0, w: 32, h: 32, offset: 0, len: src.len() };
        let v = lossless_jpeg(&src, &s, 32, 1, JpegLayout::Flat, &Limits::default()).unwrap();
        assert_eq!(v.len(), 32 * 32, "only the segment's samples are retained");
        assert!(v.capacity() < 64 * 64, "the frame's spare capacity is released");
        assert_eq!(v[..32], frame[..32], "the first row is the frame's first row");
        // An edge strip keeps only the rows inside the image.
        let v = lossless_jpeg(&src, &s, 20, 1, JpegLayout::Flat, &Limits::default()).unwrap();
        assert_eq!(v.len(), 32 * 20);
    }

    #[test]
    fn bayer_patterns() {
        for p in ["RGGB", "BGGR", "GRBG", "GBRG"] {
            assert!(Cfa::bayer(p).unwrap().is_bayer(), "{p}");
        }
        for p in ["RGBG", "RRGB", "GGRB"] {
            assert!(!Cfa::bayer(p).unwrap().is_bayer(), "{p}");
        }
        let mut c = Cfa::bayer("RGGB").unwrap();
        c.origin_x = 1;
        assert_eq!(c.phase(0, 0), [1, 0, 2, 1]); // GRBG
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn inflate_tile_requires_stream_end() {
        let data: Vec<u8> = (0..64u8).collect();
        let z = zlib(&data);
        assert_eq!(inflate_tile(&z, 64).unwrap(), data);
        // Shorter than the tile, but a complete stream: still accepted.
        assert_eq!(inflate_tile(&z, 100).unwrap(), data);
        // Truncated stream.
        assert!(inflate_tile(&z[..z.len() - 6], 64).is_err());
        // More decoded output than the tile holds.
        assert!(inflate_tile(&z, 32).is_err());
    }

    #[test]
    fn unpack_12_bit() {
        // Two 12-bit samples 0xABC, 0x123 packed MSB first.
        assert_eq!(unpack(&[0xAB, 0xC1, 0x23], 2, 1, 1, 12, true).unwrap(), vec![0xABC, 0x123]);
        // 12-bit data in 16-bit containers.
        assert_eq!(unpack(&[0xBC, 0x0A, 0x23, 0x01], 2, 1, 1, 12, true).unwrap(), vec![0xABC, 0x123]);
        assert!(unpack(&[0xAB], 2, 1, 1, 12, true).is_err());
    }
}
