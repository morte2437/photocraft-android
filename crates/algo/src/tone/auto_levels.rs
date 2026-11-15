//! Numeric analysis for PhotoCraft's ordinary Auto Tone/Contrast/Color policy.
//! Display-bin sampling and clipping are independent of document/layer models.
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};

const MAX_ROW_BYTES: usize = 64 << 20;
const MAX_SAMPLES: u64 = 16_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tone,
    Contrast,
    Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    InvalidGeometry,
    WorkspaceLimit,
    Allocation,
}
impl std::fmt::Display for Error {
    /// Expose bounded native automatic-analysis errors as actionable messages for calling hosts.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "native automatic tone analysis cancelled",
            Self::InvalidGeometry => "invalid native automatic tone analysis geometry",
            Self::WorkspaceLimit => "native automatic tone analysis exceeds bounded workspace",
            Self::Allocation => "native automatic tone analysis allocation unavailable",
        })
    }
}
impl std::error::Error for Error {}

/// Sample native display bins on a bounded grid, skipping transparent pixels and checking cancellation per
/// row/block.
fn histograms(surface: &Surface, area: Rect, ctl: &Interrupt<'_>) -> Result<[[u64; 256]; 3], Error> {
    ctl.check().map_err(|_| Error::Cancelled)?;
    let width = usize::try_from(i64::from(area.x1) - i64::from(area.x0)).map_err(|_| Error::InvalidGeometry)?;
    let height = usize::try_from(i64::from(area.y1) - i64::from(area.y0)).map_err(|_| Error::InvalidGeometry)?;
    if [area.x0, area.y0].iter().any(|n| n.checked_sub(TILE_SIZE).is_none()) || [area.x1, area.y1].iter().any(|n| n.checked_add(TILE_SIZE).is_none()) {
        return Err(Error::InvalidGeometry);
    }
    let mut histograms = [[0; 256]; 3];
    if width == 0 || height == 0 {
        return Ok(histograms);
    }
    let pixels = (width as u64).checked_mul(height as u64).ok_or(Error::WorkspaceLimit)?;
    let step = usize::try_from((pixels / 4_000_000).max(1)).map_err(|_| Error::WorkspaceLimit)?.isqrt().max(1);
    let samples = (width.div_ceil(step) as u64).checked_mul(height.div_ceil(step) as u64).ok_or(Error::WorkspaceLimit)?;
    if width.checked_mul(4).is_none_or(|bytes| bytes > MAX_ROW_BYTES) || samples > MAX_SAMPLES {
        return Err(Error::WorkspaceLimit);
    }
    let mut row = Vec::new();
    row.try_reserve_exact(width).map_err(|_| Error::Allocation)?;
    row.resize(width, [0u8; 4]);
    for y in (area.y0..area.y1).step_by(step) {
        ctl.check().map_err(|_| Error::Cancelled)?;
        surface.read_rgba8_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for (index, pixel) in row.iter().step_by(step).enumerate() {
            if index.is_multiple_of(4096) {
                ctl.check().map_err(|_| Error::Cancelled)?;
            }
            if pixel.get(3) == Some(&0) {
                continue;
            }
            for (channel, &sample) in histograms.iter_mut().zip(pixel) {
                let count = channel.get_mut(usize::from(sample)).ok_or(Error::InvalidGeometry)?;
                *count = count.saturating_add(1);
            }
        }
    }
    ctl.check().map_err(|_| Error::Cancelled)?;
    Ok(histograms)
}

/// Find the native 0.1-percent histogram endpoints; empty or flat ranges use identity.
fn clip_points(histogram: &[u64; 256]) -> (f32, f32) {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return (0., 1.);
    }
    let limit = (total as f64 * 0.001) as u64;
    let (mut accumulated, mut low) = (0u64, 0usize);
    while low < 255 && accumulated + histogram.get(low).copied().unwrap_or(0) <= limit {
        accumulated += histogram.get(low).copied().unwrap_or(0);
        low += 1;
    }
    let (mut accumulated, mut high) = (0u64, 255usize);
    while high > 0 && accumulated + histogram.get(high).copied().unwrap_or(0) <= limit {
        accumulated += histogram.get(high).copied().unwrap_or(0);
        high -= 1;
    }
    if low >= high { (0., 1.) } else { (low as f32 / 255., high as f32 / 255.) }
}
/// Select the central occupied display bin for ordinary native Auto Color neutralization.
fn median(histogram: &[u64; 256]) -> f32 {
    let total: u64 = histogram.iter().sum();
    let mut accumulated = 0;
    for (index, value) in histogram.iter().enumerate() {
        accumulated += value;
        if accumulated * 2 >= total {
            return index as f32 / 255.;
        }
    }
    0.5
}

/// Returns three native Levels rows `[in_black, in_white, gamma]` in RGB order.
/// Uses the existing 256 display bins, ordinary alpha exclusion, sampled-grid
/// policy, 0.1% end clipping, and Auto Color median neutralization unchanged.
pub fn analyze(surface: &Surface, area: Rect, kind: Kind, ctl: &Interrupt<'_>) -> Result<[[f32; 3]; 3], Error> {
    let histogram = histograms(surface, area, ctl)?;
    let mut rows = [[0., 1., 1.]; 3];
    if kind == Kind::Contrast {
        let mut combined = [0u64; 256];
        for channel in &histogram {
            for (sum, value) in combined.iter_mut().zip(channel) {
                *sum += value;
            }
        }
        let (black, white) = clip_points(&combined);
        rows.fill([black, white, 1.]);
    } else {
        for (row, channel) in rows.iter_mut().zip(&histogram) {
            let (black, white) = clip_points(channel);
            *row = [black, white, 1.];
        }
        if kind == Kind::Color {
            let medians = histogram.map(|channel| median(&channel));
            let target = medians.iter().sum::<f32>() / 3.;
            for (row, median) in rows.iter_mut().zip(medians) {
                let [black, white, gamma] = row;
                let t = ((median - *black) / (*white - *black).max(1e-3)).clamp(0.02, 0.98);
                let want = ((target - *black) / (*white - *black).max(1e-3)).clamp(0.02, 0.98);
                *gamma = (t.ln() / want.ln()).clamp(0.1, 9.99);
            }
        }
    }
    ctl.check().map_err(|_| Error::Cancelled)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    #[test]
    /// Pin native per-channel, combined-contrast and color-neutralization policies against known histogram samples.
    fn known_native_bins_keep_tone_contrast_and_color_policy_at_every_depth() {
        for sample in SampleType::ALL {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
            surface.write_region(Rect::new(0, 0, 3, 1), &[0.2, 0.3, 0.4, 1., 0.4, 0.5, 0.6, 1., 0.6, 0.7, 0.8, 1.]);
            let tone = analyze(&surface, Rect::new(0, 0, 3, 1), Kind::Tone, &Interrupt::NONE).unwrap();
            let contrast = analyze(&surface, Rect::new(0, 0, 3, 1), Kind::Contrast, &Interrupt::NONE).unwrap();
            assert_eq!(tone[0], [51. / 255., 153. / 255., 1.]);
            assert_eq!(contrast, [[51. / 255., 204. / 255., 1.]; 3]);
            let color = analyze(&surface, Rect::new(0, 0, 3, 1), Kind::Color, &Interrupt::NONE).unwrap();
            assert!(color[0][2] > 1. && color[2][2] < 1.);
        }
    }
    #[test]
    /// Verify transparent/empty inputs and hostile geometry remain bounded and cancellation returns a controlled
    /// error.
    fn transparent_empty_hostile_and_cancelled_analysis_are_bounded() {
        let surface = Surface::new(PixelFormat::RGBA8);
        assert_eq!(analyze(&surface, Rect::new(0, 0, 2, 2), Kind::Tone, &Interrupt::NONE).unwrap(), [[0., 1., 1.]; 3]);
        assert!(matches!(analyze(&surface, Rect::new(2, 0, 1, 1), Kind::Tone, &Interrupt::NONE), Err(Error::InvalidGeometry)));
        assert!(matches!(analyze(&surface, Rect::new(i32::MIN, 0, i32::MAX, 1), Kind::Tone, &Interrupt::NONE), Err(Error::InvalidGeometry)));
        // One pixel near the signed-coordinate edge must fail before native
        // tile bounds can overflow; this does not require a large allocation.
        assert!(matches!(analyze(&surface, Rect::new(i32::MAX - 1, 0, i32::MAX, 1), Kind::Tone, &Interrupt::NONE), Err(Error::InvalidGeometry)));
        assert!(matches!(analyze(&surface, Rect::new(0, 0, 20_000_000, 1), Kind::Tone, &Interrupt::NONE), Err(Error::WorkspaceLimit)));
        assert!(matches!(analyze(&surface, Rect::new(0, 0, 1, 1), Kind::Tone, &Interrupt::cancel_only(&|| true)), Err(Error::Cancelled)));
    }
}
