//! Paint helpers: bucket fill and the gradient tool.

use photocraft_color::blend::{self, BlendMode};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into};
use serde::{Deserialize, Serialize};

/// How a fill lays down its colour: blended with a mode, or Clear, which erases what it covers
/// (the Paint Bucket's Mode › Clear).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    Blend(BlendMode),
    Clear,
}

/// Gradient tool shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GradientShape {
    #[default]
    Linear,
    Radial,
    Angle,
    Reflected,
    Diamond,
}

/// Gradient tool parameter `t` for a drag from `from` to `to`.
pub fn tool_gradient_t(shape: GradientShape, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (x - from.0, y - from.1);
    let along = px * ux + py * uy;
    let across = -px * uy + py * ux;
    let t = match shape {
        GradientShape::Linear => along / len,
        GradientShape::Radial => (px * px + py * py).sqrt() / len,
        GradientShape::Reflected => along.abs() / len,
        GradientShape::Diamond => (along.abs() + across.abs()) / len,
        GradientShape::Angle => (across.atan2(along) / std::f32::consts::TAU).rem_euclid(1.0),
    };
    t.clamp(0.0, 1.0)
}

/// Samples RGBA stops (sorted by position) at `t`.
pub fn sample_stops(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    match stops {
        [] => [0.0; 4],
        [s] => s.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for w in stops.windows(2) {
                if t <= w[1].0 {
                    let k = if w[1].0 > w[0].0 { (t - w[0].0) / (w[1].0 - w[0].0) } else { 0.0 };
                    return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

/// Composites `color(i)` over `area` of the surface with per-pixel coverage. Where the coverage is
/// zero, `rim(x, y)` (when given) is the strength of a bucket's transparent-seed fringe there: the
/// pixel takes the fill at coverage `rim × (1 − α)`, its own transparency (#11354 review; see
/// [`bucket_fill_inks`]). Lock Transparent Pixels and Clear ignore it.
#[allow(clippy::too_many_arguments)]
fn composite_area(
    s: &mut Surface,
    area: Rect,
    mode: FillMode,
    coverage: impl Fn(i32, i32) -> f32 + Sync,
    color: impl Fn(i32, i32) -> [f32; 4] + Sync,
    lock_transparency: bool,
    inks: Option<[f32; 4]>,
    rim: Option<&(dyn Fn(i32, i32) -> f32 + Sync)>,
) {
    // Composite tile by tile (in parallel on native); tiles with zero coverage are left untouched,
    // so their copy-on-write storage (and GPU residency) survives.
    let fmt = s.format();
    let n = fmt.channels();
    let tiles: Vec<Rect> = area.tiles().map(|tc| tc.rect().intersect(&area)).filter(|r| !r.is_empty()).collect();
    let src: &Surface = s;
    let work = |r: &Rect| -> Option<(Rect, Vec<f32>)> {
        let w = r.width() as usize;
        let mut any = false;
        let mut region: Vec<f32> = Vec::new();
        let mut enc = [0.0f32; 8];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let mut k = coverage(x, y);
                let on_rim = match rim {
                    Some(rim) if k <= 0.0 && !lock_transparency && matches!(mode, FillMode::Blend(_)) => rim(x, y),
                    _ => 0.0,
                };
                if k <= 0.0 && on_rim <= 0.0 {
                    continue;
                }
                if !any {
                    region = src.read_region(*r);
                    any = true;
                }
                let i = ((y - r.y0) as usize * w + (x - r.x0) as usize) * n;
                let px = &mut region[i..i + n];
                let d = photocraft_raster::to_rgba(&fmt, px);
                if k <= 0.0 {
                    // A rim pixel: only its transparency takes the fill, so opaque art stays as it is.
                    k = on_rim * (1.0 - d[3].clamp(0.0, 1.0));
                    if k.is_nan() || k <= 0.0 {
                        continue;
                    }
                }
                let o = match mode {
                    FillMode::Blend(m) if lock_transparency => blend::composite_atop(m, d, color(x, y), k),
                    FillMode::Blend(m) => blend::composite(m, d, color(x, y), k),
                    // Locked transparency keeps the alpha, so Clear leaves the pixel as it is.
                    FillMode::Clear if lock_transparency => d,
                    FillMode::Clear => [d[0], d[1], d[2], d[3] * (1.0 - k.min(1.0))],
                };
                from_rgba_into(&fmt, o, &mut enc);
                // A colour picked as CMYK inks replaces fully covered pixels of a CMYK target as it is.
                if let Some(i) = inks
                    && fmt.mode == photocraft_color::ColorMode::Cmyk
                    && mode == FillMode::Blend(BlendMode::Normal)
                    && k >= 1.0
                {
                    enc[..4].copy_from_slice(&i);
                }
                px.copy_from_slice(&enc[..n]);
            }
        }
        any.then_some((*r, region))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(Rect, Vec<f32>)> = {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(Rect, Vec<f32>)> = tiles.iter().filter_map(work).collect();
    for (r, region) in done {
        s.write_region(r, &region);
    }
}

/// Gradient tool over `area` (the selection bounds or canvas).
#[allow(clippy::too_many_arguments)]
pub fn paint_gradient(
    s: &mut Surface,
    area: Rect,
    from: (f32, f32),
    to: (f32, f32),
    shape: GradientShape,
    stops: &[(f32, [f32; 4])],
    reverse: bool,
    opacity: f32,
    blend_mode: BlendMode,
    dither: bool,
    selection: Option<&Surface>,
    lock_transparency: bool,
) {
    composite_area(
        s,
        area,
        FillMode::Blend(blend_mode),
        |x, y| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0)),
        |x, y| {
            // Sampled one pixel up and left of the pixel centre, where the reference application samples (#12018).
            let t = tool_gradient_t(shape, from, to, x as f32 - 0.5, y as f32 - 0.5);
            let mut c = sample_stops(stops, if reverse { 1.0 - t } else { t });
            if dither {
                // One quantisation step of monochromatic noise breaks 8-bit banding without speckle.
                let n = (photocraft_color::dither_noise(x, y) - 0.5) / 255.0;
                for ch in c.iter_mut().take(3) {
                    *ch = (*ch + n).clamp(0.0, 1.0);
                }
            }
            c
        },
        lock_transparency,
        None,
        None,
    );
}

/// Paint bucket: fills the region similar to the seed pixel (on the layer's
/// own pixels) within `area`, respecting the selection.
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    color: [f32; 4],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> bool {
    bucket_fill_src(s, area, seed, tolerance, contiguous, anti_alias, opacity, selection, lock_transparency, |_, _| color)
}

/// Like [`bucket_fill`] but the fill colour comes from `src(x, y)` — e.g. a pattern sampled at the
/// canvas position — so the Paint Bucket can fill with a pattern as well as a solid colour.
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_src(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
) -> bool {
    bucket_fill_inks(s, area, seed, tolerance, contiguous, anti_alias, opacity, selection, lock_transparency, src, None, FillMode::Blend(BlendMode::Normal))
}

/// [`bucket_fill_src`] where `inks` (CMYK, 0..1) are what a fully covered pixel of a CMYK target
/// gets instead of `src`'s sRGB stand-in (a colour that was picked as CMYK), laid down with `mode`
/// (the options bar's Mode).
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_inks(
    s: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
) -> bool {
    if !area.contains(seed.0, seed.1) {
        return false;
    }
    let img = crate::selection::rgba8_image(s, area);
    let Some(region) = crate::selection::wand_region_bucket(&img, area, seed, tolerance, contiguous, anti_alias) else { return false };
    // Clicked on transparency with Anti-alias on: the wand's fringe gives the partly transparent
    // pixels around the flood no coverage (their alpha is measured against the seed's 0), but the
    // reference application fills each pixel 8-adjacent to the flood at coverage `1 − α`, so the
    // soft rim a cleared selection leaves rises partly (α 187 → 205, 56 → 211, 3 → 252; the
    // reference gives 204, 211, 253) and a black edge pixel of alpha 40 filled green becomes
    // (0, 248, 0, 221) (#11311, #12528, #11354 review). Pixels further out stay as they are; with
    // Anti-alias off nothing outside the flood changes.
    let at_seed = usize::try_from(seed.1 - area.y0)
        .ok()
        .and_then(|y| y.checked_mul(area.width() as usize))
        .and_then(|row| row.checked_add(usize::try_from(seed.0 - area.x0).ok()?));
    let empty_seed = anti_alias && at_seed.and_then(|i| img.get(i)).is_some_and(|p| p[3] == 0);
    drop(img);
    fill_region(s, &region, opacity, selection, lock_transparency, src, inks, mode, empty_seed);
    true
}

/// Paints a bucket region sampled before the edit, so its boundary can come from the visible
/// composite while only `s` receives the colour or pattern. Selection coverage still clips it.
pub fn bucket_fill_region_src(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
) {
    bucket_fill_region_inks(s, region, opacity, selection, lock_transparency, src, None, FillMode::Blend(BlendMode::Normal));
}

/// [`bucket_fill_region_src`] with the CMYK `inks` and the `mode` of [`bucket_fill_inks`].
#[allow(clippy::too_many_arguments)]
pub fn bucket_fill_region_inks(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
) {
    fill_region(s, region, opacity, selection, lock_transparency, src, inks, mode, false);
}

/// [`bucket_fill_region_inks`]; with `rim`, the pixels 8-adjacent to the region's fully covered
/// pixels that the region itself leaves uncovered take the fill at coverage `1 − α` (see
/// [`composite_area`]). Fully covered pixels and pixels further out are filled as without it.
#[allow(clippy::too_many_arguments)]
fn fill_region(
    s: &mut Surface,
    region: &crate::selection::Region,
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
    src: impl Fn(i32, i32) -> [f32; 4] + Sync,
    inks: Option<[f32; 4]>,
    mode: FillMode,
    rim: bool,
) {
    // Composite only over the filled region's box (with Anti-alias on it already holds the
    // one-pixel ring around the flood).
    let b = region.bbox;
    let strength = |x: i32, y: i32| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0));
    // The ring is only ever read for an unlocked Blend fill (see composite_area), and it is
    // empty when the region covers every pixel, so do not build it in those cases: on a large
    // canvas it is a bool per pixel plus a full scan of the mask.
    let use_rim = rim && !lock_transparency && matches!(mode, FillMode::Blend(_)) && region.mask.contains(&0);
    let ring = if use_rim { rim_ring(region) } else { Vec::new() };
    let on_ring = |x: i32, y: i32| {
        let at = usize::try_from(y - b.y0).ok().zip(usize::try_from(x - b.x0).ok()).filter(|_| b.contains(x, y));
        let hit = at.and_then(|(yy, xx)| yy.checked_mul(b.width() as usize)?.checked_add(xx)).and_then(|i| ring.get(i));
        if hit == Some(&true) { strength(x, y) } else { 0.0 }
    };
    let rim: Option<&(dyn Fn(i32, i32) -> f32 + Sync)> = if use_rim { Some(&on_ring) } else { None };
    composite_area(s, b, mode, |x, y| region.at(x, y) * strength(x, y), src, lock_transparency, inks, rim);
}

/// Over `region.bbox` (row-major): the pixels the region leaves uncovered that touch one of its
/// fully covered pixels, diagonals included.
fn rim_ring(region: &crate::selection::Region) -> Vec<bool> {
    let (w, h) = (region.bbox.width() as usize, region.bbox.height() as usize);
    let full = |x: usize, y: usize| y.checked_mul(w).and_then(|r| r.checked_add(x)).and_then(|i| region.mask.get(i)) == Some(&255);
    let n = w.saturating_mul(h).min(region.mask.len());
    (0..n)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            region.mask.get(i) == Some(&0)
                && (y.saturating_sub(1)..=(y + 1).min(h - 1)).any(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|nx| (nx, ny) != (x, y) && full(nx, ny)))
        })
        .collect()
}

#[cfg(test)]
#[path = "paint_gradient_sample_tests.rs"]
mod paint_gradient_sample_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn gradient_t_shapes() {
        let (f, t) = ((0.0, 0.0), (10.0, 0.0));
        assert_eq!(tool_gradient_t(GradientShape::Linear, f, t, 5.0, 3.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Linear, f, t, -5.0, 0.0), 0.0);
        assert_eq!(tool_gradient_t(GradientShape::Radial, f, t, 0.0, 5.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Reflected, f, t, -5.0, 0.0), 0.5);
        assert_eq!(tool_gradient_t(GradientShape::Diamond, f, t, 2.5, 2.5), 0.5);
        assert!((tool_gradient_t(GradientShape::Angle, f, t, 0.0, 5.0) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn gradient_paints_ramp_within_selection() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let a = Rect::new(0, 0, 11, 2);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 11, 1), &[1.0]);
        paint_gradient(
            &mut s,
            a,
            (-0.5, 0.0),
            (9.5, 0.0),
            GradientShape::Linear,
            &[(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])],
            false,
            1.0,
            BlendMode::Normal,
            false,
            Some(&sel),
            false,
        );
        assert!(s.pixel(0, 0)[0] < 0.01 && s.pixel(10, 0)[0] > 0.99);
        assert!((s.pixel(5, 0)[0] - 0.5).abs() < 0.01);
        assert_eq!(s.pixel(5, 1)[3], 0.0, "outside the selection");
    }

    #[test]
    fn locked_gradient_blends_source_atop_on_semi_transparent_pixels() {
        // Locked half-transparent white, flat black gradient at 50%: colour halfway, alpha kept (#5783).
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for mode in [BlendMode::Normal, BlendMode::Multiply] {
                let mut s = Surface::new(fmt);
                let a = Rect::new(0, 0, 4, 1);
                s.fill_rect(a, &[1.0, 1.0, 1.0, 0.5]);
                let black = [0.0, 0.0, 0.0, 1.0];
                paint_gradient(&mut s, a, (0.0, 0.0), (4.0, 0.0), GradientShape::Linear, &[(0.0, black), (1.0, black)], false, 0.5, mode, false, None, true);
                let p = s.rgba(2, 0);
                assert!((p[0] - 0.5).abs() < 0.01 && (p[3] - 0.5).abs() < 0.01, "{fmt:?} {mode:?} {p:?}");
            }
        }
    }

    #[test]
    fn bucket_fills_contiguous_region() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let a = Rect::new(0, 0, 10, 10);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(5, 0, 6, 10), &[0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill(&mut s, a, (1, 1), 10.0, true, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
        assert_eq!(s.pixel(2, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.pixel(8, 2), vec![1.0, 1.0, 1.0, 1.0]);
        assert!(!bucket_fill(&mut s, a, (50, 1), 10.0, true, false, [1.0, 0.0, 0.0, 1.0], 1.0, None, false));
    }

    #[test]
    fn bucket_region_uses_document_coordinates_and_multiplies_coverage() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let region = crate::selection::Region { bbox: Rect::new(-3, 4, 0, 5), mask: vec![255, 128, 255] };
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(-3, 4, -1, 5), &[0.5]);
        bucket_fill_region_src(&mut s, &region, 0.5, Some(&selection), false, |_, _| [1.0, 0.0, 0.0, 1.0]);
        assert!((s.rgba(-3, 4)[3] - 0.25).abs() < 0.005);
        assert!((s.rgba(-2, 4)[3] - 0.125).abs() < 0.005);
        assert_eq!(s.rgba(-1, 4), [0.0; 4], "outside the selection");
        assert_eq!(s.rgba(-4, 4), [0.0; 4], "outside the region");
    }

    /// #11311: flooding transparency with anti-aliasing fills the empty part of the region's edge
    /// pixels at the fill's opacity, so the edge is no semi-transparent ring; opaque pixels the
    /// anti-aliasing reaches blend exactly as before.
    #[test]
    fn bucket_on_transparency_fills_the_edge_pixels_in_full() {
        let a = Rect::new(0, 0, 8, 1);
        let white = |_, _| [1.0, 1.0, 1.0, 1.0];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            for (opaque, opacity) in [([1.0, 1.0, 1.0, 1.0], 1.0), ([1.0, 1.0, 1.0, 1.0], 0.5), ([1.0, 0.0, 0.0, 1.0], 1.0)] {
                let mut s = Surface::new(fmt);
                s.fill_rect(Rect::new(4, 0, 8, 1), &opaque);
                // The same region filled without the rule: what the bucket used to do.
                let mut before = s.clone();
                let img = crate::selection::rgba8_image(&before, a);
                let region = crate::selection::wand_region(&img, a, (1, 0), 0.0, true, true).unwrap();
                bucket_fill_region_inks(&mut before, &region, opacity, None, false, white, None, FillMode::Blend(BlendMode::Normal));

                assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, opacity, None, false, white, None, FillMode::Blend(BlendMode::Normal)));
                for x in 0..4 {
                    let p = s.rgba(x, 0);
                    assert!(p[..3].iter().all(|c| (c - 1.0).abs() < 1e-3) && (p[3] - opacity).abs() < 3e-3, "{fmt:?} {opaque:?} at {x}: {p:?}");
                }
                for x in 4..8 {
                    assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} {opaque:?} at {x}: opaque pixels blend as before");
                }
                assert!(s.rgba(4, 0)[3] > 0.999);
            }
        }
    }

    /// #11354 review: on a CMYK target, the edge pixels a transparent-seed bucket makes
    /// opaque take the picked inks like the fully covered interior — their colour is the
    /// fill alone. A partially transparent destination the fill touches keeps the blended
    /// colour it already had, so its inks stay the conversion of that blend.
    #[test]
    fn bucket_on_transparency_writes_the_picked_cmyk_inks_at_the_edge() {
        let a = Rect::new(0, 0, 8, 1);
        let inks = [1.0f32, 0.0, 0.0, 0.0];
        let rgb = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| c.clamp(0.0, 1.0));
        let src = |_, _| [rgb[0], rgb[1], rgb[2], 1.0];
        let raw = |s: &Surface, x: i32| -> Vec<f32> { s.read_region(Rect::new(x, 0, x + 1, 1)).into_iter().take(4).collect() };
        let is_inks = |got: &[f32]| got.iter().zip(inks).all(|(a, b)| (a - b).abs() < 0.02);

        // Transparent run against opaque white: x=3 is the anti-aliased edge (coverage < 1).
        let mut s = Surface::new(PixelFormat::CMYKA8);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        assert!(is_inks(&raw(&s, 1)), "fully covered: {:?}", raw(&s, 1));
        assert!(is_inks(&raw(&s, 3)), "the made-opaque edge takes the picked inks: {:?}", raw(&s, 3));

        // The edge pixel holds white at half alpha instead: the fill raises its alpha,
        // but the colour is a blend, so the picked inks do not replace it.
        let mut s = Surface::new(PixelFormat::CMYKA8);
        s.fill_rect(Rect::new(3, 0, 4, 1), &[0.0, 0.0, 0.0, 0.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[0.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(bucket_fill_inks(&mut s, a, (1, 0), 0.0, true, true, 1.0, None, false, src, Some(inks), FillMode::Blend(BlendMode::Normal)));
        let edge = s.rgba(3, 0);
        assert!(edge[3] > 0.5, "the fill raised the partial pixel's alpha: {edge:?}");
        assert!(!is_inks(&raw(&s, 3)), "a blended destination keeps its own encoding: {:?}", raw(&s, 3));
    }

    /// #11354 review: the soft rim a Clear leaves rises only partly. The reference application
    /// takes rim alphas 187, 56 and 3 to 204, 211 and 253; the rule here, `α + (1 − α)²`, gives
    /// 205, 211 and 252, each within one level of it (fit in PR #11354's review round). The rim pixel
    /// sits on the bucket's anti-aliased fringe; the hole beside it fills in full and the opaque
    /// art behind it is untouched by the rule.
    #[test]
    fn bucket_on_transparency_raises_the_cleared_rim_partly() {
        let a = Rect::new(0, 0, 8, 1);
        let white = |_, _| [1.0, 1.0, 1.0, 1.0];
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        // (rim alpha after the Clear, the reference application's result, ours)
        for (rim, reference, ours) in [(187, 204, 205), (56, 211, 211), (3, 253, 252)] {
            for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
                let mut s = Surface::new(fmt);
                // The cleared hole is [0, 0, 0, 0] (Clear zeroes its RGB), the rim straight white.
                s.fill_rect(Rect::new(4, 0, 5, 1), &[1.0, 1.0, 1.0, rim as f32 / 255.0]);
                s.fill_rect(Rect::new(5, 0, 8, 1), &[1.0, 1.0, 1.0, 1.0]);
                let before = s.clone();
                assert!(bucket_fill_inks(&mut s, a, (1, 0), 32.0, true, true, 1.0, None, false, white, None, FillMode::Blend(BlendMode::Normal)));
                let p = s.rgba(4, 0);
                assert_eq!(level(p[3]), ours, "{fmt:?}: rim {rim} -> {p:?}");
                assert!((ours - reference).abs() <= 1, "rim {rim}: ours {ours}, reference {reference}");
                assert!(p[..3].iter().all(|c| (c - 1.0).abs() < 1e-3), "{fmt:?}: the rim stays white: {p:?}");
                for x in 0..4 {
                    assert_eq!(level(s.rgba(x, 0)[3]), 255, "{fmt:?} rim {rim}: the hole and its edge fill in full at {x}");
                }
                for x in 5..8 {
                    assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} rim {rim}: opaque art at {x}");
                }
            }
        }
    }

    /// The rule is for a click on transparency only: from an opaque seed the fill is exactly the
    /// wand region's own fill, fringe included.
    #[test]
    fn bucket_on_opaque_pixels_keeps_neighbouring_transparency() {
        let a = Rect::new(0, 0, 8, 1);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 1), &[1.0, 1.0, 1.0, 0.5]);
        s.fill_rect(Rect::new(4, 0, 8, 1), &[1.0, 1.0, 1.0, 1.0]);
        let mut plain = s.clone();
        let img = crate::selection::rgba8_image(&plain, a);
        let region = crate::selection::wand_region(&img, a, (6, 0), 0.0, true, true).unwrap();
        bucket_fill_region_inks(&mut plain, &region, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal));
        assert!(bucket_fill_inks(&mut s, a, (6, 0), 0.0, true, true, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal)));
        for x in 0..8 {
            assert_eq!(s.rgba(x, 0), plain.rgba(x, 0), "at {x}");
        }
        assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "outside the region");
    }

    /// #12528 (reference application, transparent seed, Tolerance 32, Anti-alias on): a black
    /// edge pixel bordering the filled area takes the green fill at coverage `1 − α`, so its
    /// colour as well as its alpha follow the reference within two levels; with Anti-alias off it
    /// stays as it was.
    #[test]
    fn bucket_on_transparency_blends_into_the_edge_at_its_transparency() {
        let a = Rect::new(0, 0, 8, 1);
        let green = |_, _| [0.0, 1.0, 0.0, 1.0];
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        // (edge alpha, the reference application's result)
        for (alpha, reference) in [(40, [0, 248, 0, 221]), (112, [0, 190, 0, 192]), (197, [0, 70, 0, 210]), (253, [0, 2, 0, 253])] {
            for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
                for anti_alias in [true, false] {
                    let mut s = Surface::new(fmt);
                    s.fill_rect(Rect::new(4, 0, 5, 1), &[0.0, 0.0, 0.0, alpha as f32 / 255.0]);
                    s.fill_rect(Rect::new(5, 0, 8, 1), &[0.0, 0.0, 0.0, 1.0]);
                    let before = s.clone();
                    assert!(bucket_fill_inks(&mut s, a, (1, 0), 32.0, true, anti_alias, 1.0, None, false, green, None, FillMode::Blend(BlendMode::Normal)));
                    let got = s.rgba(4, 0).map(level);
                    if anti_alias {
                        assert!(got.iter().zip(reference).all(|(g, r)| (g - r).abs() <= 2), "{fmt:?} alpha {alpha}: {got:?}, reference {reference:?}");
                    } else {
                        assert_eq!(s.rgba(4, 0), before.rgba(4, 0), "{fmt:?} alpha {alpha}: Anti-alias off leaves the edge");
                    }
                    for x in 5..8 {
                        assert_eq!(s.rgba(x, 0), before.rgba(x, 0), "{fmt:?} alpha {alpha}: opaque art at {x}");
                    }
                }
            }
        }
    }

    /// #11354 review (measured in the reference application with a 4 px feathered rim): only the
    /// one-pixel ring 8-adjacent to the flood rises, diagonals included; pixels further out keep
    /// their alpha exactly.
    #[test]
    fn bucket_on_transparency_raises_only_the_ring_next_to_the_flood() {
        let a = Rect::new(0, 0, 7, 7);
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(a, &[1.0, 1.0, 1.0, 100.0 / 255.0]);
        s.fill_rect(Rect::new(3, 3, 4, 4), &[0.0; 4]);
        let before = s.clone();
        assert!(bucket_fill_inks(&mut s, a, (3, 3), 32.0, true, true, 1.0, None, false, |_, _| [1.0; 4], None, FillMode::Blend(BlendMode::Normal)));
        for y in 0..7 {
            for x in 0..7 {
                let (p, was) = (s.rgba(x, y), before.rgba(x, y));
                match ((x - 3).abs(), (y - 3).abs()) {
                    (0, 0) => assert!(p[3] > 0.999, "the flood at ({x},{y}): {p:?}"),
                    (dx, dy) if dx <= 1 && dy <= 1 => assert!(p[3] > was[3] + 0.1, "the ring at ({x},{y}) rises: {p:?}"),
                    _ => assert_eq!(p, was, "beyond the ring at ({x},{y})"),
                }
            }
        }
    }

    /// The Paint Bucket's Mode: Multiply darkens what's there, Clear erases it (half at 50 %).
    #[test]
    fn bucket_mode_blends_or_clears() {
        let a = Rect::new(0, 0, 4, 1);
        let grey = [0.5, 0.5, 0.5, 1.0];
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(a, &grey);
            let red = |_, _| [1.0, 0.0, 0.0, 1.0];
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 1.0, None, false, red, None, FillMode::Blend(BlendMode::Multiply)));
            let px = s.rgba(1, 0);
            assert!((px[0] - 0.5).abs() < 0.01 && px[1] < 0.01 && (px[3] - 1.0).abs() < 1e-3, "{fmt:?} multiply: {px:?}");
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 0.5, None, false, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} clear at 50 %");
            // Locked transparency keeps the alpha, so Clear does nothing.
            assert!(bucket_fill_inks(&mut s, a, (0, 0), 0.0, true, false, 1.0, None, true, red, None, FillMode::Clear));
            assert!((s.rgba(1, 0)[3] - 0.5).abs() < 0.01, "{fmt:?} locked");
        }
    }
}
