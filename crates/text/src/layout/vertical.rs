use super::*;

/// How a character is set in vertical type (a simplification of Unicode's
/// `Vertical_Orientation`, UAX #50).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VClass {
    /// Rotated 90° clockwise (Latin, digits, most symbols).
    Rotate = 0,
    /// Upright (ideographs, kana, Hangul, full-width forms, CJK punctuation such as 、。),
    /// using a vertical alternate when the font has one.
    Upright = 1,
    /// Upright through its vertical alternate (brackets, the long-vowel mark, dashes); rotated
    /// when the font has none.
    AlternateOrRotate = 2,
}

/// The vertical-type class of a character.
pub fn vertical_class(c: char) -> VClass {
    let u = c as u32;
    let alt = matches!(
        u,
        0x2014..=0x2016
            | 0x2025
            | 0x2026
            | 0x2329
            | 0x232A
            | 0x3008..=0x3011
            | 0x3014..=0x301F
            | 0x3030
            | 0x30A0
            | 0x30FC
            | 0xFE59..=0xFE5E
            | 0xFF08
            | 0xFF09
            | 0xFF0D
            | 0xFF1A..=0xFF1E
            | 0xFF3B
            | 0xFF3D
            | 0xFF3F
            | 0xFF5B..=0xFF60
            | 0xFFE3
    );
    if alt {
        return VClass::AlternateOrRotate;
    }
    // Half-width katakana and Hangul rotate like other half-width text.
    if (0xFF61..=0xFFDC).contains(&u) {
        return VClass::Rotate;
    }
    let upright = crate::cjk::classify(c).is_some()
        || matches!(u, 0xA7 | 0xA9 | 0xAE | 0xB1 | 0xBC..=0xBE | 0xD7 | 0xF7 | 0x2E80..=0x2EFF | 0xFE10..=0xFE1F | 0xFE30..=0xFE4F | 0x1F000..=0x1FAFF);
    if upright { VClass::Upright } else { VClass::Rotate }
}

/// Placement data of one vertical-type glyph (before mapping to text space).
pub(super) struct VGlyph {
    pub(super) upright: bool,
    /// Advance along the column (px).
    pub(super) advance: f32,
    /// Upright: distance from the top of the glyph's cell down to its baseline (px).
    pub(super) origin: f32,
    /// Rotated: distance from the column centre line to the glyph's baseline (px).
    pub(super) centre: f32,
}

/// Vertical metrics of a font instance: `VORG`, `vhea`/`vmtx`, else the ideographic em box.
pub(super) struct VMetrics<'a> {
    font: Option<skrifa::FontRef<'a>>,
    coords: Vec<skrifa::instance::NormalizedCoord>,
    /// px per font unit.
    scale: f32,
    /// Em-box fallback origin (px below the cell top).
    em_origin: f32,
}

impl<'a> VMetrics<'a> {
    pub(super) fn new(fd: &'a FontData, coords: &[i16], size: f32) -> Self {
        use skrifa::MetadataProvider;
        let font = skrifa::FontRef::from_index(fd.data.as_ref(), fd.index).ok();
        let coords: Vec<skrifa::instance::NormalizedCoord> = coords.iter().map(|&c| skrifa::instance::NormalizedCoord::from_bits(c)).collect();
        let (scale, em_origin) = match &font {
            Some(f) => {
                let m = f.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::new(&coords));
                let upem = f32::from(m.units_per_em).max(1.0);
                let (asc, desc) = (m.ascent, -m.descent);
                // Ideographic em box: the em centred on the font's ascent/descent span.
                let r = if asc + desc > 0.0 && asc.is_finite() && desc.is_finite() { (asc / (asc + desc)).clamp(0.0, 1.0) } else { 0.88 };
                (size / upem, size * r)
            }
            None => (0.0, size * 0.88),
        };
        Self { font, coords, scale, em_origin }
    }

    pub(super) fn cmap(&self, c: char) -> Option<u32> {
        use skrifa::MetadataProvider;
        self.font.as_ref()?.charmap().map(c).map(|g| g.to_u32())
    }

    /// Distance from the top of an upright glyph's cell to its baseline (px).
    pub(super) fn origin(&self, gid: u32) -> f32 {
        self.vertical_origin(gid).filter(|v| v.is_finite()).unwrap_or(self.em_origin)
    }

    fn vertical_origin(&self, gid: u32) -> Option<f32> {
        use skrifa::MetadataProvider;
        use skrifa::raw::TableProvider;
        let f = self.font.as_ref()?;
        let g = skrifa::GlyphId::new(gid);
        if let Ok(vorg) = f.vorg() {
            return Some(f32::from(vorg.vertical_origin_y(g)) * self.scale);
        }
        let tsb = f.vmtx().ok()?.side_bearing(g)?;
        // Top of the outline (font units), from the outline itself (glyf and CFF alike).
        let outline = f.outline_glyphs().get(g)?;
        let mut pen = YMax(None);
        let settings = skrifa::outline::DrawSettings::unhinted(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::new(&self.coords));
        outline.draw(settings, &mut pen).ok()?;
        Some((pen.0? + f32::from(tsb)) * self.scale)
    }
}

/// Outline pen recording the largest y.
struct YMax(Option<f32>);

impl YMax {
    fn add(&mut self, y: f32) {
        self.0 = Some(self.0.map_or(y, |m| m.max(y)));
    }
}

impl skrifa::outline::OutlinePen for YMax {
    fn move_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn line_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn quad_to(&mut self, _cx0: f32, cy0: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(y);
    }
    fn curve_to(&mut self, _cx0: f32, cy0: f32, _cx1: f32, cy1: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(cy1);
        self.add(y);
    }
    fn close(&mut self) {}
}
