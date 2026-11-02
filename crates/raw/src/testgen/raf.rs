use super::tiff::{TiffBuilder, Val};

// ---------------------------------------------------------------- Fujifilm RAF

/// How a synthetic RAF stores its samples (see `raf.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RafPacking {
    /// 16-bit containers; little-endian (`II`) or big-endian (`MM`) CFA TIFF.
    U16 { le: bool },
    /// 12-bit, little-endian bit stream, least significant bit first.
    Lsb12,
    /// 14-bit, most significant bit first in little-endian 32-bit words.
    Words14,
    /// A short `IS…` strip standing in for Fujifilm's compressed data.
    Compressed,
}

/// The X-Trans repeat at the data origin, as observed (row major, 0 = R, 1 = G, 2 = B).
pub const XTRANS: [u8; 36] = [
    1, 1, 0, 1, 1, 2, //
    1, 1, 2, 1, 1, 0, //
    2, 0, 1, 0, 2, 1, //
    1, 1, 2, 1, 1, 0, //
    1, 1, 0, 1, 1, 2, //
    0, 2, 1, 2, 0, 1,
];

/// A synthetic Fujifilm RAF.
#[derive(Debug, Clone)]
pub struct RafSpec {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u16>,
    pub bits: u32,
    pub packing: RafPacking,
    /// CFA colours at the data origin: 36 (X-Trans, 6×6) or 4 (Bayer, 2×2), row major.
    pub cfa: Vec<u8>,
    /// RawImageCropTopLeft / RawImageCroppedSize: top, left, height, width.
    pub crop: Option<[u16; 4]>,
    pub black: Vec<u32>,
    /// WB_GRBLevels: G, R, B.
    pub wb_grb: Option<[u32; 3]>,
    /// RawExposureBias as numerator / denominator.
    pub exposure_bias: Option<(i16, i16)>,
    /// EXIF orientation of the preview JPEG.
    pub orientation: u16,
    /// `false` writes an early-FinePix-style file without the CFA TIFF.
    pub cfa_tiff: bool,
}

impl RafSpec {
    pub fn new(width: usize, height: usize, data: Vec<u16>, cfa: Vec<u8>) -> Self {
        RafSpec {
            width,
            height,
            data,
            bits: 14,
            packing: RafPacking::U16 { le: true },
            cfa,
            crop: None,
            black: vec![1024],
            wb_grb: Some([302, 604, 453]),
            exposure_bias: None,
            orientation: 1,
            cfa_tiff: true,
        }
    }

    /// The samples as stored.
    fn strip(&self) -> Vec<u8> {
        match self.packing {
            RafPacking::U16 { le } => self.data.iter().flat_map(|v| if le { v.to_le_bytes() } else { v.to_be_bytes() }).collect(),
            RafPacking::Lsb12 => {
                let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
                for &v in &self.data {
                    acc |= u64::from(v & 0xFFF) << n;
                    n += 12;
                    while n >= 8 {
                        out.push(acc as u8);
                        acc >>= 8;
                        n -= 8;
                    }
                }
                if n > 0 {
                    out.push(acc as u8);
                }
                out
            }
            RafPacking::Words14 => {
                let (mut logical, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
                for &v in &self.data {
                    acc = (acc << 14) | u64::from(v & 0x3FFF);
                    n += 14;
                    while n >= 8 {
                        logical.push((acc >> (n - 8)) as u8);
                        n -= 8;
                    }
                }
                if n > 0 {
                    logical.push((acc << (8 - n)) as u8);
                }
                logical.resize(logical.len().next_multiple_of(4), 0);
                logical.chunks(4).flat_map(|w| w.iter().rev().copied().collect::<Vec<u8>>()).collect()
            }
            RafPacking::Compressed => {
                let mut v = b"IS\x01\x00".to_vec();
                v.resize(self.width * self.height / 2, 0x5A);
                v
            }
        }
    }

    /// The CFA region: a TIFF whose IFD0 points (0xF000) at the Fuji raw IFD.
    fn cfa_region(&self) -> Vec<u8> {
        let le = !matches!(self.packing, RafPacking::U16 { le: false });
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let strip = self.strip();
        let strip_at = 2048u32;
        let entries: Vec<(u16, Vec<u32>)> = vec![
            (0xF001, vec![self.width as u32]),
            (0xF002, vec![self.height as u32]),
            (0xF003, vec![self.bits]),
            (0xF007, vec![strip_at]),
            (0xF008, vec![strip.len() as u32]),
            (0xF00A, self.black.clone()),
        ]
        .into_iter()
        .chain(self.wb_grb.map(|w| (0xF00E, w.to_vec())))
        .collect();
        let mut out = if le { b"II*\0".to_vec() } else { b"MM\0*".to_vec() };
        out.extend_from_slice(&u32b(8));
        // IFD0: one entry, 0xF000 (type IFD) → the Fuji IFD at 26.
        out.extend_from_slice(&u16b(1));
        out.extend_from_slice(&u16b(0xF000));
        out.extend_from_slice(&u16b(13));
        out.extend_from_slice(&u32b(1));
        out.extend_from_slice(&u32b(26));
        out.extend_from_slice(&u32b(0));
        let fuji_at = out.len();
        let mut extra_at = fuji_at + 2 + 12 * entries.len() + 4;
        let mut extra = Vec::new();
        out.extend_from_slice(&u16b(entries.len() as u16));
        for (tag, vals) in &entries {
            out.extend_from_slice(&u16b(*tag));
            out.extend_from_slice(&u16b(4));
            out.extend_from_slice(&u32b(vals.len() as u32));
            if vals.len() == 1 {
                out.extend_from_slice(&u32b(vals[0]));
            } else {
                out.extend_from_slice(&u32b(extra_at as u32));
                for v in vals {
                    extra.extend_from_slice(&u32b(*v));
                }
                extra_at += 4 * vals.len();
            }
        }
        out.extend_from_slice(&u32b(0));
        out.extend_from_slice(&extra);
        assert!(out.len() <= strip_at as usize);
        out.resize(strip_at as usize, 0);
        out.extend_from_slice(&strip);
        out
    }

    /// A tiny baseline-JPEG preview (APP1 EXIF with make, model and
    /// orientation; SOF0 32×24; EOI).
    fn preview(&self) -> Vec<u8> {
        let mut t = TiffBuilder::default();
        let ifd0 = t.ifd(vec![(271, Val::Ascii("FUJIFILM".into())), (272, Val::Ascii("X-Synthetic".into())), (274, Val::Short(vec![self.orientation]))]);
        t.chain = vec![ifd0];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&t.build());
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE1];
        j.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        j.extend_from_slice(&app1);
        j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 24, 0x00, 32, 1, 1, 0x11, 0, 0xFF, 0xD9]);
        j
    }

    /// The metadata directory.
    fn meta(&self) -> Vec<u8> {
        let mut recs: Vec<(u16, Vec<u8>)> = Vec::new();
        if let Some([top, left, h, w]) = self.crop {
            recs.push((0x0110, [top.to_be_bytes(), left.to_be_bytes()].concat()));
            recs.push((0x0111, [h.to_be_bytes(), w.to_be_bytes()].concat()));
        }
        if self.cfa.len() == 36 {
            recs.push((0x0130, vec![0x0C; 4]));
            recs.push((0x0131, self.cfa.iter().rev().copied().collect()));
        } else {
            // Bayer: reversed, colour in the low two bits, the second green as 3.
            let mut greens = 0;
            let codes: Vec<u8> = self
                .cfa
                .iter()
                .map(|&c| {
                    if c == 1 {
                        greens += 1;
                        if greens == 1 { 0x09 } else { 0x0B }
                    } else {
                        0x08 | c
                    }
                })
                .collect();
            recs.push((0x0130, codes.into_iter().rev().collect()));
        }
        if let Some((n, d)) = self.exposure_bias {
            recs.push((0x9650, [n.to_be_bytes(), d.to_be_bytes()].concat()));
        }
        let mut out = (recs.len() as u32).to_be_bytes().to_vec();
        for (tag, d) in recs {
            out.extend_from_slice(&tag.to_be_bytes());
            out.extend_from_slice(&(d.len() as u16).to_be_bytes());
            out.extend_from_slice(&d);
        }
        out
    }

    pub fn build(&self) -> Vec<u8> {
        let mut b = b"FUJIFILMCCD-RAW 0201FF000000".to_vec();
        b.extend_from_slice(b"X-Synthetic");
        b.resize(60, 0);
        b.extend_from_slice(b"0100");
        b.resize(160, 0);
        let preview = self.preview();
        let meta = self.meta();
        let region = if self.cfa_tiff { self.cfa_region() } else { self.strip() };
        let (p_at, m_at) = (b.len(), b.len() + preview.len());
        let c_at = m_at + meta.len();
        for (at, v) in [(84, p_at), (88, preview.len()), (92, m_at), (96, meta.len()), (100, c_at), (104, region.len())] {
            b[at..at + 4].copy_from_slice(&(v as u32).to_be_bytes());
        }
        b.extend_from_slice(&preview);
        b.extend_from_slice(&meta);
        b.extend_from_slice(&region);
        b
    }
}

/// Mosaics linear RGB through any CFA repeat (`cfa`: `cfa_w` colours per
/// row) anchored at (0, 0), like [`mosaic`].
pub fn mosaic_cfa(rgb: &[[f32; 3]], width: usize, cfa: &[u8], cfa_w: usize, black: u16, white: u16) -> Vec<u16> {
    let cfa_h = cfa.len() / cfa_w;
    rgb.iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = (i % width, i / width);
            let c = cfa[(y % cfa_h) * cfa_w + x % cfa_w] as usize;
            (f32::from(black) + p[c].clamp(0.0, 1.0) * f32::from(white - black)).round() as u16
        })
        .collect()
}

// ---------------------------------------------------------------- scenes

/// A smooth, colourful synthetic scene as linear RGB in 0..1.
pub fn scene(width: usize, height: usize) -> Vec<[f32; 3]> {
    (0..width * height)
        .map(|i| {
            let (x, y) = ((i % width) as f32 / width.max(1) as f32, (i / width) as f32 / height.max(1) as f32);
            [0.1 + 0.7 * x, 0.15 + 0.5 * (1.0 - y) * x + 0.2 * y, 0.6 - 0.4 * y + 0.2 * x]
        })
        .collect()
}

/// Mosaics linear RGB through a 2×2 CFA (`cfa` row-major colours) into
/// sensor values `black + v * (white - black)`.
pub fn mosaic(rgb: &[[f32; 3]], width: usize, cfa: [u8; 4], black: u16, white: u16) -> Vec<u16> {
    rgb.iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = (i % width, i / width);
            let c = cfa[(y & 1) * 2 + (x & 1)] as usize;
            let v = p[c].clamp(0.0, 1.0);
            (f32::from(black) + v * f32::from(white - black)).round() as u16
        })
        .collect()
}
