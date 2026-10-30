//! Patterns: tiles used by Pattern Overlay, pattern fill layers, pattern strokes, Edit › Fill
//! and brush textures. Pure data; rendering lives in `photocraft-compose`, the PSD mapping
//! (`Patt`/`Pat2`/`Pat3` global blocks) in `photocraft-io`.

use photocraft_color::SampleType;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// Largest pattern edge a file may declare (the PSD reader's limit; Define Pattern stops at 8000).
pub const MAX_EDGE: u32 = 30_000;

/// Most pixels a pattern may have to be painted (Define Pattern's 8000 × 8000 fits). Larger
/// patterns paint nothing.
pub const MAX_PAINT_PIXELS: u64 = 1 << 26;

/// Most bytes of working copy the CPU compositor holds for one pattern (`photocraft-compose`
/// checks the two agree at build time). See [`Pattern::working_bytes`]. A browser tab has far less
/// memory than a desktop process, and a failed allocation there aborts the tab, so the web build
/// holds half as much: an 8-bit pattern paints up to 33,554,432 px (about 5792²) there.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_WORKING_BYTES: u64 = 256 << 20;
/// See the desktop value above.
#[cfg(target_arch = "wasm32")]
pub const MAX_WORKING_BYTES: u64 = 128 << 20;

/// Rows of a pattern read at a time where a whole-tile `f32` copy would cost 16 bytes per pixel.
pub const BAND_ROWS: u32 = 256;

/// One pattern tile. Pixels cover `0..width × 0..height` of `surface`, in any pixel format
/// (depth and colour model are the pattern's own, as in Photoshop).
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// Unique id (a UUID string, as Photoshop's `Idnt`).
    pub id: String,
    /// Name as stored (Photoshop's built-ins use `$$$/Patterns/…=Display Name`; see
    /// [`Pattern::display_name`]).
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub surface: Surface,
}

/// 64-bit FNV-1a, used for deterministic pattern ids.
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

impl Pattern {
    /// A pattern with an id derived from its name and pixels (same content → same id, so
    /// defining a pattern is deterministic and replayable).
    pub fn new(name: impl Into<String>, surface: Surface, width: u32, height: u32) -> Pattern {
        let name = name.into();
        let mut h1 = fnv(0xcbf2_9ce4_8422_2325, name.as_bytes());
        h1 = fnv(h1, &width.to_le_bytes());
        h1 = fnv(h1, &height.to_le_bytes());
        // The samples row by row, a band at a time: the same sequence as one read of the whole
        // tile, without a full-size `f32` copy of a large pattern.
        let mut band = Vec::new();
        for top in (0..height).step_by(BAND_ROWS as usize) {
            let bottom = (top + BAND_ROWS).min(height);
            surface.read_region_into(Rect::new(0, top as i32, width as i32, bottom as i32), &mut band);
            for v in &band {
                h1 = fnv(h1, &v.to_bits().to_le_bytes());
            }
        }
        let h2 = fnv(h1 ^ 0x9e37_79b9_7f4a_7c15, format!("{:?}", surface.format()).as_bytes());
        let id = format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", (h1 >> 32) as u32, (h1 >> 16) as u16, h1 as u16, (h2 >> 48) as u16, h2 & 0xffff_ffff_ffff);
        Pattern { id, name, width, height, surface }
    }

    /// The name shown in UIs: Photoshop's localisation keys (`$$$/Patterns/…=Water`) show
    /// the text after `=`; trailing NULs are dropped.
    pub fn display_name(&self) -> &str {
        display_name(&self.name)
    }

    /// The tile's rectangle.
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// Whether the tile has pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Pixels in the tile (`width × height`, never overflowing).
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Bytes of working copy the compositor holds to paint this pattern. One that fits is expanded
    /// to premultiplied `f32` RGBA (16 per pixel). A pattern too large for that is held at its own
    /// depth instead, straight RGBA at 4 bytes per pixel for 8 bits and 8 for 16 bits: exactly its
    /// values, in a quarter or half the memory. A float one has no smaller exact form.
    pub fn working_bytes(&self) -> u64 {
        working_bytes_for(self.pixel_count(), self.surface.format().sample)
    }

    /// Why this pattern paints nothing, for the person or agent who chose it; `None` when it
    /// paints. A pattern too large for the compositor's working copy renders as transparent, so
    /// every command that picks a pattern, and the import that brought it in, says so.
    pub fn paint_problem(&self) -> Option<String> {
        if self.is_paintable() {
            return None;
        }
        if self.is_empty() {
            return Some("it is empty".into());
        }
        let sample = self.surface.format().sample;
        Some(format!(
            "it is {} × {} px at {} bits per channel, and a pattern of that depth paints up to {} px in all",
            self.width,
            self.height,
            sample.bytes() * 8,
            max_paint_pixels(sample)
        ))
    }

    /// Whether the tile has pixels and is small enough to paint (see [`MAX_PAINT_PIXELS`] and
    /// [`MAX_WORKING_BYTES`]).
    pub fn is_paintable(&self) -> bool {
        !self.is_empty() && self.pixel_count() <= MAX_PAINT_PIXELS && self.working_bytes() <= MAX_WORKING_BYTES
    }
}

/// [`Pattern::working_bytes`] of a pattern of `pixels` pixels with `sample` samples.
pub fn working_bytes_for(pixels: u64, sample: SampleType) -> u64 {
    let expanded = pixels.saturating_mul(16);
    if expanded <= MAX_WORKING_BYTES {
        return expanded;
    }
    match sample {
        SampleType::U8 => pixels.saturating_mul(4),
        SampleType::U16 => pixels.saturating_mul(8),
        SampleType::F32 => expanded,
    }
}

/// Most pixels a pattern with `sample` samples may have and still paint: [`MAX_PAINT_PIXELS`] for
/// 8 bits, fewer where the working copy is larger per pixel.
pub fn max_paint_pixels(sample: SampleType) -> u64 {
    match sample {
        SampleType::U8 => MAX_PAINT_PIXELS,
        SampleType::U16 => MAX_PAINT_PIXELS.min(MAX_WORKING_BYTES / 8),
        SampleType::F32 => MAX_PAINT_PIXELS.min(MAX_WORKING_BYTES / 16),
    }
}

/// Display form of a stored pattern name (see [`Pattern::display_name`]).
pub fn display_name(name: &str) -> &str {
    let n = name.trim_end_matches('\0');
    match n.strip_prefix("$$$/").and_then(|r| r.split_once('=')) {
        Some((_, shown)) => shown,
        None => n,
    }
}

/// Finds a pattern by id, else by stored name, else by display name (case-insensitive).
pub fn find<'a>(patterns: &'a [Pattern], id: &str, name: &str) -> Option<&'a Pattern> {
    if !id.is_empty()
        && let Some(p) = patterns.iter().find(|p| p.id == id)
    {
        return Some(p);
    }
    if name.is_empty() {
        return None;
    }
    patterns.iter().find(|p| p.name == name).or_else(|| patterns.iter().find(|p| p.display_name().eq_ignore_ascii_case(display_name(name))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn ids_are_deterministic_and_lookup_falls_back_to_names() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        let a = Pattern::new("$$$/Patterns/Defaults/Water=Water", s.clone(), 4, 4);
        let b = Pattern::new("$$$/Patterns/Defaults/Water=Water", s.clone(), 4, 4);
        assert_eq!(a.id, b.id);
        assert_eq!(a.id.len(), 36);
        assert_ne!(Pattern::new("Other", s, 4, 4).id, a.id);
        assert_eq!(a.display_name(), "Water");
        let list = vec![a.clone()];
        assert!(find(&list, &a.id, "").is_some());
        assert!(find(&list, "nope", "water").is_some());
        assert!(find(&list, "nope", "").is_none());
    }

    /// The id is a hash of every sample in row-major order; reading a tall pattern a band of rows
    /// at a time must give the id one whole read gave, or saved documents would lose their
    /// pattern links.
    #[test]
    fn the_id_of_a_tall_pattern_matches_a_single_read_of_it() {
        let (w, h) = (7u32, BAND_ROWS * 2 + 37);
        let mut s = Surface::new(PixelFormat::RGBA8);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[(x % 5) as f32 / 4.0, (y % 7) as f32 / 6.0, ((x + y) % 3) as f32 / 2.0, 1.0]);
            }
        }
        let name = "tall";
        let mut h1 = fnv(0xcbf2_9ce4_8422_2325, name.as_bytes());
        h1 = fnv(h1, &w.to_le_bytes());
        h1 = fnv(h1, &h.to_le_bytes());
        for v in s.read_region(Rect::new(0, 0, w as i32, h as i32)) {
            h1 = fnv(h1, &v.to_bits().to_le_bytes());
        }
        let h2 = fnv(h1 ^ 0x9e37_79b9_7f4a_7c15, format!("{:?}", s.format()).as_bytes());
        let expected =
            format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", (h1 >> 32) as u32, (h1 >> 16) as u16, h1 as u16, (h2 >> 48) as u16, h2 & 0xffff_ffff_ffff);
        assert_eq!(Pattern::new(name, s, w, h).id, expected);
    }

    #[test]
    fn working_bytes_keep_a_pattern_exact_in_less_memory() {
        let sparse =
            |format: PixelFormat, width: u32, height: u32| Pattern { id: String::new(), name: String::new(), width, height, surface: Surface::new(format) };
        let float = PixelFormat::new(photocraft_color::ColorMode::Rgb, SampleType::F32, true);
        // What fits is expanded as before.
        assert_eq!(sparse(PixelFormat::RGBA8, 10, 10).working_bytes(), 1600);
        assert_eq!(sparse(float, 4096, 4096).working_bytes(), MAX_WORKING_BYTES);
        // Past that, a pattern is held at its own depth.
        assert_eq!(sparse(PixelFormat::RGBA8, 8000, 8000).working_bytes(), 256_000_000);
        assert_eq!(sparse(PixelFormat::RGBA16, 5000, 5000).working_bytes(), 200_000_000);
        assert!(sparse(PixelFormat::RGBA8, 8000, 8000).is_paintable());
        assert!(sparse(PixelFormat::RGBA16, 5792, 5792).is_paintable());
        assert!(!sparse(PixelFormat::RGBA16, 8000, 8000).is_paintable(), "512 MB at 16 bits");
        assert!(!sparse(float, 4097, 4096).is_paintable(), "float has no smaller exact form");
        assert!(!sparse(PixelFormat::RGBA8, 9000, 8000).is_paintable());
        assert!(!sparse(PixelFormat::RGBA8, 0, 8).is_paintable());
    }

    #[test]
    fn a_pattern_that_paints_nothing_says_why() {
        let sparse =
            |format: PixelFormat, width: u32, height: u32| Pattern { id: String::new(), name: String::new(), width, height, surface: Surface::new(format) };
        let float = PixelFormat::new(photocraft_color::ColorMode::Rgb, SampleType::F32, true);
        assert_eq!(sparse(PixelFormat::RGBA8, 8000, 8000).paint_problem(), None);
        assert_eq!(sparse(PixelFormat::RGBA8, 0, 4).paint_problem().as_deref(), Some("it is empty"));
        let reason = sparse(float, 5000, 5000).paint_problem().expect("float 5000 x 5000 paints nothing");
        assert!(reason.contains("5000 × 5000") && reason.contains("32 bits") && reason.contains("16777216"), "{reason}");
        let reason = sparse(PixelFormat::RGBA16, 8000, 8000).paint_problem().expect("16-bit 8000 x 8000 paints nothing");
        assert!(reason.contains("16 bits") && reason.contains("33554432"), "{reason}");
    }
}
