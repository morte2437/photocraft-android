use super::lj92::lj92_encode;
use super::tiff::{TiffBuilder, Val, srat, urat};

// ---------------------------------------------------------------- DNG

/// Encodes 8-bit samples (1 gray or 3 RGB per pixel, row-major) as a baseline JPEG at
/// quality 60, the way a lossy DNG segment is stored (an RGB segment is coded as YCbCr).
pub fn baseline_jpeg(samples: &[u8], width: usize, height: usize, per_pixel: usize) -> Vec<u8> {
    use image::ImageEncoder as _;
    let colour = if per_pixel == 3 { image::ExtendedColorType::Rgb8 } else { image::ExtendedColorType::L8 };
    let mut j = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut j, 60);
    enc.write_image(samples, width as u32, height as u32, colour).expect("jpeg segment");
    j
}

/// How the DNG stores its raw data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngStorage {
    /// One strip per `rows` rows, uncompressed at `bits` per sample (8, 12, 16…).
    Strips { rows: usize },
    /// Adobe DNG compression 34892: baseline JPEG tiles at tile size (edge tiles padded), one
    /// gray component for CFA and monochrome data, three (YCbCr-coded RGB) for LinearRaw. The
    /// spec's `bits` should be 8 (the DNG spec's depth for lossy JPEG; the reader rejects others);
    /// wider samples are written as their top 8 bits. Quality 60 leaves visible-but-mild artifacts.
    LossyJpegTiles { width: usize, height: usize },
    /// Adobe DNG compression 34892 in strips of this many rows: each strip is a baseline JPEG
    /// of the image width by the strip's own row count (the last strip is shorter).
    LossyJpegStrips { rows: usize },
    /// Adobe DNG compression 8: zlib streams with the TIFF predictor (1 = none, 2 = horizontal
    /// differencing) applied before compressing.
    DeflateStrips { rows: usize, predictor: u16 },
    /// Lossless-JPEG tiles of this size (2 components per JPEG row, as Adobe writes CFA tiles).
    Lj92Tiles { width: usize, height: usize },
    /// Lossless-JPEG strips of this many rows, 1 component.
    Lj92Strips { rows: usize },
    /// JPEG XL strips of this many rows (Compression 52546): one codestream per strip, taken from
    /// [`DngSpec::jxl_segments`] (the tests encode them; this crate has no JPEG XL encoder).
    JxlStrips { rows: usize },
    /// JPEG XL tiles of this size, one codestream per tile in row-major order from
    /// [`DngSpec::jxl_segments`].
    JxlTiles { width: usize, height: usize },
}

/// A synthetic DNG.
#[derive(Debug, Clone)]
pub struct DngSpec {
    pub width: usize,
    pub height: usize,
    /// 1 (CFA, or monochrome LinearRaw with `monochrome`) or 3 (LinearRaw).
    pub samples: usize,
    /// With `samples` 1: a monochrome LinearRaw image (no CFA tags) instead of CFA data.
    pub monochrome: bool,
    /// Row-major samples (`width * height * samples`).
    pub data: Vec<u16>,
    pub bits: u16,
    pub cfa: [u8; 4],
    pub storage: DngStorage,
    pub big_endian: bool,
    pub black: Vec<u32>,
    pub black_repeat: (u16, u16),
    /// WhiteLevel; 0 leaves the tag out (the reader then defaults to the bit depth's maximum).
    pub white: u32,
    /// top, left, bottom, right
    pub active_area: Option<[u32; 4]>,
    /// origin (x, y), size (w, h), relative to the active area
    pub default_crop: Option<([u32; 2], [u32; 2])>,
    /// Double-precision origin override for malformed-metadata regression tests.
    pub default_crop_origin_double: Option<[f64; 2]>,
    /// A DefaultCropSize with no DefaultCropOrigin (`default_crop` must be `None`).
    pub default_crop_size_only: Option<[u32; 2]>,
    pub linearization: Option<Vec<u16>>,
    /// (illuminant code, ColorMatrix row-major)
    pub color_matrix1: Option<(u16, [f64; 9])>,
    pub color_matrix2: Option<(u16, [f64; 9])>,
    pub forward_matrix1: Option<[f64; 9]>,
    pub forward_matrix2: Option<[f64; 9]>,
    pub as_shot_neutral: Option<[f64; 3]>,
    pub baseline_exposure: Option<f64>,
    /// Raw OpcodeList2 bytes (see [`gain_map_opcode_list`]).
    pub opcode_list2: Option<Vec<u8>>,
    /// Encoded segments for [`DngStorage::JxlStrips`] / [`DngStorage::JxlTiles`].
    pub jxl_segments: Vec<Vec<u8>>,
    /// Pre-encoded segments for [`DngStorage::LossyJpegTiles`] / [`DngStorage::LossyJpegStrips`]
    /// (row-major), used instead of encoding `data` where present: for tiles that disagree with
    /// the IFD (hostile-input tests).
    pub lossy_segments: Vec<Vec<u8>>,
    /// Raw ProfileGainTableMap bytes, written to IFD0 (contents are not interpreted).
    pub profile_gain_table_map: Option<Vec<u8>>,
    pub orientation: u16,
    pub make: String,
    pub model: String,
    /// Extra IFD0 entries, written after the generated ones (e.g. malformed colour tags).
    pub ifd0_extra: Vec<(u16, Val)>,
    /// Extra raw-IFD entries, written after the generated ones (e.g. SampleFormat).
    pub raw_extra: Vec<(u16, Val)>,
}

impl DngSpec {
    /// A CFA DNG with default metadata around `data` (RGGB, 16-bit, one strip).
    pub fn cfa(width: usize, height: usize, data: Vec<u16>) -> Self {
        DngSpec {
            width,
            height,
            samples: 1,
            monochrome: false,
            data,
            bits: 16,
            cfa: [0, 1, 1, 2],
            storage: DngStorage::Strips { rows: height },
            big_endian: false,
            black: vec![0],
            black_repeat: (1, 1),
            white: 65535,
            active_area: None,
            default_crop: None,
            default_crop_origin_double: None,
            default_crop_size_only: None,
            linearization: None,
            color_matrix1: None,
            color_matrix2: None,
            forward_matrix1: None,
            forward_matrix2: None,
            as_shot_neutral: None,
            baseline_exposure: None,
            orientation: 1,
            opcode_list2: None,
            jxl_segments: Vec::new(),
            lossy_segments: Vec::new(),
            profile_gain_table_map: None,
            make: "Photocraft".into(),
            model: "Synthetic".into(),
            ifd0_extra: Vec::new(),
            raw_extra: Vec::new(),
        }
    }

    /// A monochrome (one-sample LinearRaw) DNG around `data`, as monochrome cameras and film
    /// scanners write them: no CFA tags and no colour matrices.
    pub fn monochrome(width: usize, height: usize, data: Vec<u16>) -> Self {
        DngSpec { monochrome: true, ..Self::cfa(width, height, data) }
    }

    fn is_cfa(&self) -> bool {
        self.samples == 1 && !self.monochrome
    }

    /// The 8-bit sample a lossy JPEG segment stores for `v`: the value itself at 8 bits, else
    /// its top 8 bits.
    fn sample8(&self, v: u16) -> u8 {
        (v >> self.bits.saturating_sub(8)) as u8
    }

    fn pack(&self, rows: std::ops::Range<usize>) -> Vec<u8> {
        let n = self.width * self.samples;
        let vals = &self.data[rows.start * n..rows.end * n];
        match self.bits {
            8 => vals.iter().map(|&v| v as u8).collect(),
            16 => vals.iter().flat_map(|&v| if self.big_endian { v.to_be_bytes() } else { v.to_le_bytes() }).collect(),
            b => {
                // MSB-first bit packing, each row padded to a byte.
                let mut out = Vec::new();
                for row in vals.chunks(n) {
                    let mut acc: u64 = 0;
                    let mut have = 0u32;
                    for &v in row {
                        acc = (acc << b) | u64::from(v);
                        have += u32::from(b);
                        while have >= 8 {
                            have -= 8;
                            out.push((acc >> have) as u8);
                        }
                    }
                    if have > 0 {
                        out.push((acc << (8 - have)) as u8);
                    }
                }
                out
            }
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut t = TiffBuilder { big_endian: self.big_endian, ..Default::default() };
        let mut raw = self.raw_base_tags();
        self.push_storage_tags(&mut t, &mut raw);
        self.push_geometry_tags(&mut raw);
        if let Some(o) = &self.opcode_list2 {
            raw.push((51009, Val::Undefined(o.clone())));
        }
        if let Some(l) = &self.linearization {
            raw.push((50712, Val::Short(l.clone())));
        }
        raw.extend(self.raw_extra.iter().cloned());
        let raw_ifd = t.ifd(raw);
        // IFD0: a tiny RGB thumbnail plus the DNG metadata.
        let thumb = t.blob(vec![128; 3 * 4]);
        let mut ifd0 = self.ifd0_base_tags(thumb, raw_ifd);
        self.push_color_tags(&mut ifd0);
        ifd0.extend(self.ifd0_extra.iter().cloned());
        let i0 = t.ifd(ifd0);
        t.chain = vec![i0];
        t.build()
    }

    /// The raw IFD's tags that don't depend on how the image is stored.
    fn raw_base_tags(&self) -> Vec<(u16, Val)> {
        let mut raw: Vec<(u16, Val)> = vec![
            (254, Val::Long(vec![0])),
            (256, Val::Long(vec![self.width as u32])),
            (257, Val::Long(vec![self.height as u32])),
            (258, Val::Short(vec![self.bits; self.samples])),
            (262, Val::Short(vec![if self.is_cfa() { 32803 } else { 34892 }])),
            (277, Val::Short(vec![self.samples as u16])),
            (284, Val::Short(vec![1])),
            (50713, Val::Short(vec![self.black_repeat.0, self.black_repeat.1])),
            (50714, Val::Long(self.black.clone())),
        ];
        if self.white != 0 {
            raw.push((50717, Val::Long(vec![self.white; self.samples])));
        }
        if self.is_cfa() {
            raw.push((33421, Val::Short(vec![2, 2])));
            raw.push((33422, Val::Byte(self.cfa.to_vec())));
            raw.push((50710, Val::Byte(vec![0, 1, 2])));
            raw.push((50711, Val::Short(vec![1])));
        }
        raw
    }

    /// Encodes the image per `storage` into `t`'s blobs and adds the compression and
    /// strip or tile tags that point at them.
    fn push_storage_tags(&self, t: &mut TiffBuilder, raw: &mut Vec<(u16, Val)>) {
        match self.storage {
            DngStorage::DeflateStrips { rows, predictor } => {
                let (offs, lens) = self.strip_blobs(t, rows, |y, r| self.deflate_strip(y, r, predictor));
                raw.push((259, Val::Short(vec![8])));
                raw.push((317, Val::Short(vec![predictor])));
                push_strip_tags(raw, rows, offs, lens);
            }
            DngStorage::Strips { rows } => {
                let (offs, lens) = self.strip_blobs(t, rows, |y, r| self.pack(y..y + r));
                raw.push((259, Val::Short(vec![1])));
                push_strip_tags(raw, rows, offs, lens);
            }
            DngStorage::LossyJpegTiles { width: tw, height: th } => {
                let s = self.samples;
                let (offs, lens) = self.tile_blobs(t, tw, th, |tile_no, tile: Vec<u8>| {
                    self.lossy_segments.get(tile_no).cloned().unwrap_or_else(|| baseline_jpeg(&tile, tw, th, s))
                });
                raw.push((259, Val::Short(vec![34892])));
                push_tile_tags(raw, tw, th, offs, lens);
            }
            DngStorage::LossyJpegStrips { rows } => {
                let n = self.width * self.samples;
                let (offs, lens) = self.strip_blobs(t, rows, |y, r| {
                    let strip: Vec<u8> = self.data[y * n..(y + r) * n].iter().map(|&v| self.sample8(v)).collect();
                    let strip_no = y / rows;
                    self.lossy_segments.get(strip_no).cloned().unwrap_or_else(|| baseline_jpeg(&strip, self.width, r, self.samples))
                });
                raw.push((259, Val::Short(vec![34892])));
                push_strip_tags(raw, rows, offs, lens);
            }
            DngStorage::Lj92Strips { rows } => {
                let n = self.width * self.samples;
                let (offs, lens) =
                    self.strip_blobs(t, rows, |y, r| lj92_encode(&self.data[y * n..(y + r) * n], self.width, r, self.samples, self.bits as u8, 1, None));
                raw.push((259, Val::Short(vec![7])));
                push_strip_tags(raw, rows, offs, lens);
            }
            DngStorage::Lj92Tiles { width: tw, height: th } => {
                let s = self.samples;
                let (offs, lens) = self.tile_blobs(t, tw, th, |_, tile: Vec<u16>| {
                    // CFA tiles as Adobe writes them: half width, 2 components.
                    if s == 1 && tw % 2 == 0 {
                        lj92_encode(&tile, tw / 2, th, 2, self.bits as u8, 1, None)
                    } else {
                        lj92_encode(&tile, tw, th, s, self.bits as u8, 1, None)
                    }
                });
                raw.push((259, Val::Short(vec![7])));
                push_tile_tags(raw, tw, th, offs, lens);
            }
            DngStorage::JxlStrips { rows } => {
                let (offs, lens) = self.jxl_blobs(t);
                raw.push((259, Val::Short(vec![52546])));
                push_strip_tags(raw, rows, offs, lens);
            }
            DngStorage::JxlTiles { width: tw, height: th } => {
                let (offs, lens) = self.jxl_blobs(t);
                raw.push((259, Val::Short(vec![52546])));
                push_tile_tags(raw, tw, th, offs, lens);
            }
        }
    }

    /// One blob per strip of `rows` rows: `encode(first_row, rows_in_strip)` gives its bytes.
    /// Returns the blob handles and lengths.
    fn strip_blobs(&self, t: &mut TiffBuilder, rows: usize, mut encode: impl FnMut(usize, usize) -> Vec<u8>) -> (Vec<usize>, Vec<u32>) {
        let mut offs = Vec::new();
        let mut lens = Vec::new();
        for y in (0..self.height).step_by(rows) {
            let r = (y + rows).min(self.height) - y;
            let bytes = encode(y, r);
            lens.push(bytes.len() as u32);
            offs.push(t.blob(bytes));
        }
        (offs, lens)
    }

    /// One blob per `tw`×`th` tile, in row-major tile order: `encode(tile_number, samples)`.
    fn tile_blobs<T: Copy + Default + TileSample>(
        &self,
        t: &mut TiffBuilder,
        tw: usize,
        th: usize,
        mut encode: impl FnMut(usize, Vec<T>) -> Vec<u8>,
    ) -> (Vec<usize>, Vec<u32>) {
        let mut offs = Vec::new();
        let mut lens = Vec::new();
        for ty in (0..self.height).step_by(th) {
            for tx in (0..self.width).step_by(tw) {
                let bytes = encode(offs.len(), self.tile_samples(tx, ty, tw, th));
                lens.push(bytes.len() as u32);
                offs.push(t.blob(bytes));
            }
        }
        (offs, lens)
    }

    /// The samples of the tile at (`tx`, `ty`). Edge tiles are padded by repeating the last row
    /// / column.
    fn tile_samples<T: Copy + Default + TileSample>(&self, tx: usize, ty: usize, tw: usize, th: usize) -> Vec<T> {
        let s = self.samples;
        let mut tile = vec![T::default(); tw * th * s];
        for y in 0..th {
            for x in 0..tw {
                let sy = (ty + y).min(self.height - 1);
                let sx = (tx + x).min(self.width - 1);
                for c in 0..s {
                    tile[(y * tw + x) * s + c] = T::from_sample(self, self.data[(sy * self.width + sx) * s + c]);
                }
            }
        }
        tile
    }

    /// A deflated strip of `r` rows from row `y`, optionally with the horizontal-differencing
    /// predictor applied.
    fn deflate_strip(&self, y: usize, r: usize, predictor: u16) -> Vec<u8> {
        use std::io::Write;
        let n = self.width * self.samples;
        let mut v: Vec<u16> = self.data[y * n..(y + r) * n].to_vec();
        if predictor == 2 {
            // Horizontal differencing on the sample values, before byte packing
            // (the reader undoes predictor after unpacking, per row, per channel).
            let plane = if self.samples.is_multiple_of(3) { 3 } else { 1 };
            for row in v.chunks_mut(n) {
                for i in (plane..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - plane]);
                }
            }
        }
        // Pack the delta'd values with the file's byte order and bit depth.
        let bytes: Vec<u8> = match self.bits {
            8 => v.iter().map(|&x| x as u8).collect(),
            16 => v.iter().flat_map(|&x| if self.big_endian { x.to_be_bytes() } else { x.to_le_bytes() }).collect(),
            _ => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        };
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&bytes).expect("zlib");
        e.finish().expect("zlib")
    }

    /// The pre-encoded JPEG XL segments as blobs.
    fn jxl_blobs(&self, t: &mut TiffBuilder) -> (Vec<usize>, Vec<u32>) {
        let lens = self.jxl_segments.iter().map(|j| j.len() as u32).collect();
        let offs = self.jxl_segments.iter().map(|j| t.blob(j.clone())).collect();
        (offs, lens)
    }

    /// Active area and default crop tags.
    fn push_geometry_tags(&self, raw: &mut Vec<(u16, Val)>) {
        if let Some(a) = self.active_area {
            raw.push((50829, Val::Long(a.to_vec())));
        }
        if let Some((o, s)) = self.default_crop {
            let origin = self.default_crop_origin_double.map(|d| Val::Double(d.to_vec())).unwrap_or_else(|| Val::Long(o.to_vec()));
            raw.push((50719, origin));
            raw.push((50720, Val::Long(s.to_vec())));
        } else if let Some(origin) = self.default_crop_origin_double {
            raw.push((50719, Val::Double(origin.to_vec())));
        } else if let Some(size) = self.default_crop_size_only {
            raw.push((50720, Val::Long(size.to_vec())));
        }
    }

    /// IFD0's tags before the colour metadata: the thumbnail strip `thumb`, identity, orientation
    /// and the sub-IFD `raw_ifd`.
    fn ifd0_base_tags(&self, thumb: usize, raw_ifd: usize) -> Vec<(u16, Val)> {
        vec![
            (254, Val::Long(vec![1])),
            (256, Val::Long(vec![2])),
            (257, Val::Long(vec![2])),
            (258, Val::Short(vec![8, 8, 8])),
            (259, Val::Short(vec![1])),
            (262, Val::Short(vec![2])),
            (271, Val::Ascii(self.make.clone())),
            (272, Val::Ascii(self.model.clone())),
            (273, Val::Blobs(vec![thumb])),
            (274, Val::Short(vec![self.orientation])),
            (277, Val::Short(vec![3])),
            (278, Val::Long(vec![2])),
            (279, Val::Long(vec![12])),
            (330, Val::Ifds(vec![raw_ifd])),
            (50706, Val::Byte(vec![1, 4, 0, 0])),
            (50708, Val::Ascii(format!("{} {}", self.make, self.model))),
        ]
    }

    /// Colour matrices, white balance, exposure and the profile gain table.
    fn push_color_tags(&self, ifd0: &mut Vec<(u16, Val)>) {
        let m = |v: &[f64; 9]| Val::SRational(v.iter().map(|&x| srat(x)).collect());
        if let Some((ill, cm)) = &self.color_matrix1 {
            ifd0.push((50721, m(cm)));
            ifd0.push((50778, Val::Short(vec![*ill])));
        }
        if let Some((ill, cm)) = &self.color_matrix2 {
            ifd0.push((50722, m(cm)));
            ifd0.push((50779, Val::Short(vec![*ill])));
        }
        if let Some(f) = &self.forward_matrix1 {
            ifd0.push((50964, m(f)));
        }
        if let Some(f) = &self.forward_matrix2 {
            ifd0.push((50965, m(f)));
        }
        if let Some(n) = self.as_shot_neutral {
            ifd0.push((50728, Val::Rational(n.iter().map(|&v| urat(v)).collect())));
        }
        if let Some(b) = self.baseline_exposure {
            ifd0.push((50730, Val::SRational(vec![srat(b)])));
        }
        if let Some(m) = &self.profile_gain_table_map {
            ifd0.push((52525, Val::Undefined(m.clone())));
        }
    }
}

/// How a tile's samples are stored: raw 16-bit values, or the 8-bit samples of a lossy JPEG.
trait TileSample {
    fn from_sample(spec: &DngSpec, v: u16) -> Self;
}

impl TileSample for u16 {
    fn from_sample(_: &DngSpec, v: u16) -> Self {
        v
    }
}

impl TileSample for u8 {
    fn from_sample(spec: &DngSpec, v: u16) -> Self {
        spec.sample8(v)
    }
}

/// Strip layout tags for the blobs `offs` (lengths `lens`).
fn push_strip_tags(raw: &mut Vec<(u16, Val)>, rows: usize, offs: Vec<usize>, lens: Vec<u32>) {
    raw.push((278, Val::Long(vec![rows as u32])));
    raw.push((273, Val::Blobs(offs)));
    raw.push((279, Val::Long(lens)));
}

/// Tile layout tags for the blobs `offs` (lengths `lens`).
fn push_tile_tags(raw: &mut Vec<(u16, Val)>, tw: usize, th: usize, offs: Vec<usize>, lens: Vec<u32>) {
    raw.push((322, Val::Long(vec![tw as u32])));
    raw.push((323, Val::Long(vec![th as u32])));
    raw.push((324, Val::Blobs(offs)));
    raw.push((325, Val::Long(lens)));
}

/// An OpcodeList2 with one GainMap over active-area rectangle `area` (top,
/// left, bottom, right), every `pitch` rows and columns, from a 2×2 grid of
/// gains spanning the image (relative coordinates 0..1).
pub fn gain_map_opcode_list(area: [u32; 4], pitch: u32, gains: [[f32; 2]; 2]) -> Vec<u8> {
    let mut p = Vec::new();
    for v in [area[0], area[1], area[2], area[3], 0, 1, pitch, pitch, 2, 2] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    for v in [1.0f64, 1.0, 0.0, 0.0] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    p.extend_from_slice(&1u32.to_be_bytes());
    for g in gains.iter().flatten() {
        p.extend_from_slice(&g.to_bits().to_be_bytes());
    }
    let mut b = 1u32.to_be_bytes().to_vec();
    for v in [9u32, 0x0103_0000, 1, p.len() as u32] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.extend_from_slice(&p);
    b
}

/// An opcode list of MapPolynomial opcodes (ID 8), one per `(plane, coefficients)`, each over
/// `area` (top, left, bottom, right) at pitch 1.
pub fn map_polynomial_opcode_list(area: [u32; 4], planes: &[(u32, Vec<f64>)]) -> Vec<u8> {
    let mut b = (planes.len() as u32).to_be_bytes().to_vec();
    for (plane, coefficients) in planes {
        let mut p = Vec::new();
        let degree = coefficients.len().saturating_sub(1) as u32;
        for v in [area[0], area[1], area[2], area[3], *plane, 1, 1, 1, degree] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        for c in coefficients {
            p.extend_from_slice(&c.to_be_bytes());
        }
        for v in [8u32, 0x0103_0000, 0, p.len() as u32] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&p);
    }
    b
}
