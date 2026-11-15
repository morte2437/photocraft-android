use super::tiff::{TiffBuilder, Val, urat};

// ---------------------------------------------------------------- Hasselblad 3FR

/// A synthetic Hasselblad 3FR, as the X2D II 100C writes it: IFD0 with Make,
/// Model and DNG-style AsShotNeutral, and the CFA image in a SubIFD —
/// `bits`-bit uncompressed samples with BlackLevel, WhiteLevel and DefaultCrop
/// tags and no CFA-pattern tags. `compression` and the optional tags exist for
/// the unsupported and hostile variants.
#[derive(Debug, Clone)]
pub struct Hasselblad3frSpec {
    pub width: usize,
    pub height: usize,
    /// Row-major sensor samples.
    pub data: Vec<u16>,
    pub bits: u16,
    pub compression: u16,
    /// The BlackLevel RATIONAL; `None` leaves the tag out.
    pub black: Option<(u32, u32)>,
    /// The WhiteLevel; `None` leaves the tag out.
    pub white: Option<u16>,
    /// AsShotNeutral; `None` leaves the tag out.
    pub as_shot_neutral: Option<[f64; 3]>,
    /// DefaultCropOrigin (x, y) and DefaultCropSize (w, h); `None` leaves both out.
    pub crop: Option<([u16; 2], [u16; 2])>,
    /// ActiveArea (top, left, bottom, right); `None` leaves the tag out.
    pub active_area: Option<[u32; 4]>,
    pub make: String,
    pub model: String,
}

impl Hasselblad3frSpec {
    /// An X2D II 100C-shaped file: 16-bit uncompressed, black 4096, white
    /// 64763, the sample file's as-shot neutral, no crop.
    pub fn new(width: usize, height: usize, data: Vec<u16>) -> Self {
        Hasselblad3frSpec {
            width,
            height,
            data,
            bits: 16,
            compression: 1,
            black: Some((4096, 1)),
            white: Some(64763),
            as_shot_neutral: Some([0.375, 1.0, 0.617]),
            crop: None,
            active_area: None,
            make: "Hasselblad".into(),
            model: "X2D II 100C".into(),
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut t = TiffBuilder::default();
        let len = self.data.len() * 2;
        let strip = t.blob(self.data.iter().flat_map(|&v| v.to_le_bytes()).collect());
        let mut raw = vec![
            (254, Val::Long(vec![0])),
            (256, Val::Long(vec![self.width as u32])),
            (257, Val::Long(vec![self.height as u32])),
            (258, Val::Short(vec![self.bits])),
            (259, Val::Short(vec![self.compression])),
            (262, Val::Short(vec![32803])),
            (273, Val::Blobs(vec![strip])),
            (277, Val::Short(vec![1])),
            (278, Val::Long(vec![self.height as u32])),
            (279, Val::Long(vec![len as u32])),
        ];
        if let Some((n, d)) = self.black {
            raw.push((50714, Val::Rational(vec![(n, d)])));
        }
        if let Some(w) = self.white {
            raw.push((50717, Val::Short(vec![w])));
        }
        if let Some((origin, size)) = &self.crop {
            raw.push((50719, Val::Short(origin.to_vec())));
            raw.push((50720, Val::Short(size.to_vec())));
        }
        if let Some(area) = self.active_area {
            raw.push((50829, Val::Long(area.to_vec())));
        }
        let raw_ifd = t.ifd(raw);
        let thumb = t.blob(vec![128; 12]);
        let mut ifd0 = vec![
            (254, Val::Long(vec![1])),
            (256, Val::Long(vec![2])),
            (257, Val::Long(vec![2])),
            (258, Val::Short(vec![8, 8, 8])),
            (259, Val::Short(vec![7])),
            (262, Val::Short(vec![6])),
            (271, Val::Ascii(self.make.clone())),
            (272, Val::Ascii(self.model.clone())),
            (273, Val::Blobs(vec![thumb])),
            (277, Val::Short(vec![3])),
            (278, Val::Long(vec![2])),
            (279, Val::Long(vec![12])),
            (330, Val::Ifds(vec![raw_ifd])),
            (50708, Val::Ascii(format!("{} {}", self.make, self.model))),
        ];
        if let Some(n) = self.as_shot_neutral {
            ifd0.push((50728, Val::Rational(n.iter().map(|&v| urat(v)).collect())));
        }
        let i0 = t.ifd(ifd0);
        t.chain = vec![i0];
        t.build()
    }
}
