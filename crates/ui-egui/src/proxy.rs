//! Downsampled proxy documents for fast interactive previews (e.g. dragging an adjustment slider on
//! a 36 MP image). Built once per document revision; only adjustment parameters change while the
//! slider moves, so the proxy's pixels stay valid and each preview frame composites ~1/k² pixels.

use photocraft_doc::Document;

/// Target pixel count for previews (fast enough to composite every frame on CPU).
pub const PREVIEW_PIXELS: u64 = 2_500_000;

/// Downscale factor for a document, or 1 when it's already small.
pub fn factor(doc: &Document) -> u32 {
    let px = doc.size.area();
    if px <= PREVIEW_PIXELS * 2 {
        return 1;
    }
    ((px as f64 / PREVIEW_PIXELS as f64).sqrt().ceil() as u32).max(2)
}

/// Whether live previews may run on a reduced copy (Preferences › Performance › Low Resolution
/// Previews, on by default). Off, every live preview renders at full resolution.
pub fn reduced_previews(app: &crate::PhotocraftApp) -> bool {
    app.session.prefs().performance.low_resolution_previews
}

/// Downscale factor of a live preview of `doc`: [`factor`], or 1 when reduced previews are off.
pub fn preview_factor(doc: &Document, reduced: bool) -> u32 {
    if reduced { factor(doc) } else { 1 }
}

/// A live preview's factor `k` capped for a view at `zoom` (physical screen pixels per document
/// pixel): never coarser than the screen ([`crate::adjust_preview::proxy_factor`]), so a view at
/// 100 % shows full resolution instead of k-pixel blocks while a control moves (#5764).
pub fn view_factor(k: u32, zoom: f32) -> u32 {
    k.min(crate::adjust_preview::proxy_factor(zoom)).max(1)
}

pub use photocraft_compose::proxy::{downsample, proxy_document};

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::Size;

    #[test]
    fn factor_scales_with_size() {
        let small = Document::new("s", Size::new(1000, 1000), ColorMode::Rgb, SampleType::U8);
        assert_eq!(factor(&small), 1);
        let big = Document::new("b", Size::new(6016, 6016), ColorMode::Rgb, SampleType::U8);
        let k = factor(&big);
        assert!((3..=5).contains(&k), "{k}");
        assert!(big.size.area() / (k as u64 * k as u64) <= PREVIEW_PIXELS);
        assert_eq!(preview_factor(&big, true), k);
        assert_eq!(preview_factor(&big, false), 1);
        assert_eq!(preview_factor(&small, true), 1);
    }

    /// #5764: dragging a Curves point on a large document showed the canvas in k-pixel blocks at
    /// any zoom. The proxy may only be as coarse as the screen.
    #[test]
    fn view_factor_never_coarser_than_the_screen() {
        let big = Document::new("b", Size::new(6016, 6016), ColorMode::Rgb, SampleType::U8);
        let k = preview_factor(&big, true);
        assert!(k > 2, "{k}");
        assert_eq!(view_factor(k, 1.0), 1, "100 %: full resolution");
        assert_eq!(view_factor(k, 2.0), 1, "zoomed in: full resolution");
        assert_eq!(view_factor(k, 0.6), 1);
        assert_eq!(view_factor(k, 0.5), 2);
        assert_eq!(view_factor(k, 0.1), k, "far out: the document's proxy");
        assert_eq!(view_factor(1, 0.1), 1, "small documents stay full size");
        assert_eq!(view_factor(k, f32::NAN), 1);
        assert_eq!(view_factor(0, 0.1), 1);
    }

    #[test]
    fn proxy_keeps_structure_and_ids() {
        let doc = Document::with_background("d", Size::new(4000, 3000), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let p = proxy_document(&doc, 4);
        assert_eq!((p.size.width, p.size.height), (1000, 750));
        assert_eq!(p.layers[0].id, doc.layers[0].id);
        assert_eq!(p.layers[0].surface().unwrap().pixel(999, 749), vec![1.0; 4]);
    }
}
