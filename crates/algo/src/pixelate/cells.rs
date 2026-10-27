//! Reuse the jittered lattice in cell space. Raster work stays in the original
//! row/candidate order, including f32 ties and boundary-cell average additions.
use super::*;

#[derive(Clone, Copy)]
struct Site {
    point: (f32, f32),
    color: [f32; MAXC],
    radius: f32,
    sampled: bool,
}

struct Grid {
    x0: i32,
    y0: i32,
    width: usize,
    sites: Vec<Site>,
}

fn key(p: i32, origin: f32, cell: f32) -> i32 {
    ((p as f32 + 0.5 - origin) / cell).floor() as i32
}

impl Grid {
    fn new(src: &Image, area: Rect, ctx: &Ctx, cell: f32, seed: u32, dots: bool) -> Option<Self> {
        if src.data.len() != (src.rect.width() as usize).checked_mul(src.rect.height() as usize)?.checked_mul(src.ch)? {
            return None;
        }
        Self::new_with(area, ctx, src.ch, cell, seed, dots, |sx, sy, color| {
            for (c, value) in color.iter_mut().enumerate() {
                *value = src.get(sx, sy, c);
            }
            src.rect.contains(sx, sy) && ctx.bounds.contains(sx, sy)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with(area: Rect, ctx: &Ctx, n: usize, cell: f32, seed: u32, dots: bool, sample: impl Fn(i32, i32, &mut [f32]) -> bool) -> Option<Self> {
        if area.is_empty() || !cell.is_finite() || cell < 2.0 || n == 0 || n > MAXC {
            return None;
        }
        let origin = (ctx.bounds.x0 as f32, ctx.bounds.y0 as f32);
        let x0 = key(area.x0, origin.0, cell).checked_sub(1)?;
        let y0 = key(area.y0, origin.1, cell).checked_sub(1)?;
        let x1 = key(area.x1.checked_sub(1)?, origin.0, cell).checked_add(1)?;
        let y1 = key(area.y1.checked_sub(1)?, origin.1, cell).checked_add(1)?;
        let width = usize::try_from(x1.checked_sub(x0)?.checked_add(1)?).ok()?;
        let height = usize::try_from(y1.checked_sub(y0)?.checked_add(1)?).ok()?;
        let len = width.checked_mul(height)?;
        // Optional tile-local scratch, at most 16 MiB even for a direct kernel call.
        if len.checked_mul(std::mem::size_of::<Site>())? > 16 * 1024 * 1024 {
            return None;
        }
        let mut sites = Vec::new();
        sites.try_reserve_exact(len).ok()?;
        let cc = ncol(ctx, n);
        let sub = subtractive(ctx);
        for gy in y0..=y1 {
            for gx in x0..=x1 {
                let point = cell_point(gx, gy, cell, seed, origin);
                let (sx, sy) = (point.0.floor() as i32, point.1.floor() as i32);
                let mut color = [0.0; MAXC];
                let sampled = sample(sx, sy, color.get_mut(..n)?);
                let radius = if dots {
                    let j = (hash01(gx, gy, 17, seed) - 0.5) * 0.08;
                    for value in color.iter_mut().take(cc) {
                        *value = (*value + if sub { -j } else { j }).clamp(0.0, 1.0);
                    }
                    cell * (0.45 + 0.35 * hash01(gx, gy, 13, seed))
                } else {
                    0.0
                };
                sites.push(Site { point, color, radius, sampled });
            }
        }
        Some(Self { x0, y0, width, sites })
    }

    fn neighbours(&self, cx: i32, cy: i32) -> Option<[usize; 9]> {
        let x = usize::try_from(cx.checked_sub(1)?.checked_sub(self.x0)?).ok()?;
        let y = usize::try_from(cy.checked_sub(1)?.checked_sub(self.y0)?).ok()?;
        if x.checked_add(2)? >= self.width {
            return None;
        }
        let start = y.checked_mul(self.width)?.checked_add(x)?;
        let last = start.checked_add(2 * self.width)?.checked_add(2)?;
        self.sites.get(last)?;
        Some(std::array::from_fn(|i| start + i / 3 * self.width + i % 3))
    }
}

/// Contiguous x runs with the same base cell. Evaluate the old f32 mapping once
/// per column, rather than use an inverse mapping that could round differently.
fn runs(area: Rect, origin: f32, cell: f32) -> Vec<(i32, i32, i32)> {
    let mut runs: Vec<(i32, i32, i32)> = Vec::new();
    for x in area.x0..area.x1 {
        let cx = key(x, origin, cell);
        if let Some(last) = runs.last_mut()
            && last.0 == cx
        {
            last.2 = x + 1;
        } else {
            runs.push((cx, x, x + 1));
        }
    }
    runs
}

fn nearest(grid: &Grid, neighbours: &[usize; 9], x: i32, y: i32) -> Option<usize> {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut best = (f32::MAX, *neighbours.get(4)?);
    for &i in neighbours {
        let p = grid.sites.get(i)?.point;
        let d = (p.0 - fx).powi(2) + (p.1 - fy).powi(2);
        if d < best.0 {
            best = (d, i);
        }
    }
    Some(best.1)
}

/// Bounds on the *same f32 score* used by the scalar comparison. Along a
/// scanline, squared x distance has its maximum at an endpoint and its lower
/// bound at the clamped site centre. IEEE subtraction, square, addition and
/// sqrt are monotone on these nonnegative inputs, so these bounds also cover
/// rounding. No estimated bisectors or epsilon decide ownership.
fn score_range<const DOTS: bool>(site: &Site, x0: i32, x1: i32, y: i32) -> (f32, f32) {
    let left = x0 as f32 + 0.5;
    let right = (x1 - 1) as f32 + 0.5;
    let dy2 = (site.point.1 - (y as f32 + 0.5)).powi(2);
    let near = (site.point.0 - site.point.0.clamp(left, right)).powi(2) + dy2;
    let far = (site.point.0 - left).powi(2).max((site.point.0 - right).powi(2)) + dy2;
    if DOTS { (site.radius - far.sqrt(), site.radius - near.sqrt()) } else { (-far, -near) }
}

fn score<const DOTS: bool>(site: &Site, x: i32, y: i32) -> f32 {
    let d = (site.point.0 - (x as f32 + 0.5)).powi(2) + (site.point.1 - (y as f32 + 0.5)).powi(2);
    if DOTS { site.radius - d.sqrt() } else { -d }
}

/// Certify a whole run when one candidate's worst score beats every other's
/// best. Ambiguous runs split down to scalar pixels. Depth is bounded by the
/// output tile width (at most 11 splits for a 2048-pixel application tile).
fn certified_span<const DOTS: bool>(grid: &Grid, ids: &[usize; 9], x0: i32, y: i32, owners: &mut [u32], depths: &mut [f32]) -> Option<()> {
    let len = owners.len();
    if len == 0 {
        return Some(());
    }
    let x1 = x0.checked_add(i32::try_from(len).ok()?)?;
    let middle = x0 + (len / 2) as i32;
    let mut best = (if DOTS { -0.5 } else { -f32::MAX }, None);
    for &i in ids {
        let value = score::<DOTS>(grid.sites.get(i)?, middle, y);
        if value > best.0 {
            best = (value, Some(i));
        }
    }
    if let Some(i) = best.1 {
        let site = grid.sites.get(i)?;
        let lower = score_range::<DOTS>(site, x0, x1, y).0;
        let certified = (!DOTS || lower > -0.5)
            && ids.iter().filter(|&&j| j != i).all(|&j| grid.sites.get(j).is_some_and(|other| lower > score_range::<DOTS>(other, x0, x1, y).1));
        if len == 1 || certified {
            owners.fill(i as u32);
            if DOTS {
                if lower >= 0.5 {
                    depths.fill(0.5);
                } else {
                    for (offset, depth) in depths.iter_mut().enumerate() {
                        *depth = score::<true>(site, x0 + offset as i32, y);
                    }
                }
            }
            return Some(());
        }
    } else if len == 1 || ids.iter().all(|&i| grid.sites.get(i).is_some_and(|site| score_range::<DOTS>(site, x0, x1, y).1 <= -0.5)) {
        owners.fill(u32::MAX);
        return Some(());
    }
    let half = len / 2;
    let (left, right) = owners.split_at_mut(half);
    let (ld, rd) = depths.split_at_mut(half);
    certified_span::<DOTS>(grid, ids, x0, y, left, ld)?;
    certified_span::<DOTS>(grid, ids, x0 + half as i32, y, right, rd)
}

pub(super) fn crystallize(src: &Image, out: Rect, ctx: &Ctx, cell_size: f32, seed: u32) -> Option<Vec<f32>> {
    let cell = cell_size.max(2.0);
    let n = src.ch;
    let b = ctx.bounds;
    // Match the scalar averaging window, including floor/ceil at negative origins.
    // Decline extreme geometry before its i32 arithmetic; the scalar fallback owns it.
    let c0 = (
        (((i64::from(out.x0) - i64::from(b.x0)) as f32 / cell).floor() as i32).checked_sub(2)?,
        (((i64::from(out.y0) - i64::from(b.y0)) as f32 / cell).floor() as i32).checked_sub(2)?,
    );
    let c1 = (
        (((i64::from(out.x1) - i64::from(b.x0)) as f32 / cell).floor() as i32).checked_add(2)?,
        (((i64::from(out.y1) - i64::from(b.y0)) as f32 / cell).floor() as i32).checked_add(2)?,
    );
    let win = Rect::new(
        b.x0.checked_add((c0.0 as f32 * cell).floor() as i32)?,
        b.y0.checked_add((c0.1 as f32 * cell).floor() as i32)?,
        b.x0.checked_add((c1.0.checked_add(1)? as f32 * cell).ceil() as i32)?,
        b.y0.checked_add((c1.1.checked_add(1)? as f32 * cell).ceil() as i32)?,
    )
    .intersect(&src.rect)
    .intersect(&b);
    let mut grid = Grid::new(src, out.union(&win), ctx, cell, seed, false)?;
    let origin = (b.x0 as f32, b.y0 as f32);
    if grid.sites.iter().any(|s| !s.sampled) {
        let mut acc = Vec::new();
        let count = grid.sites.len().checked_mul(n.checked_add(1)?)?;
        acc.try_reserve_exact(count).ok()?;
        acc.resize(count, 0.0f32);
        let columns = runs(win, origin.0, cell);
        for y in win.y0..win.y1 {
            let cy = key(y, origin.1, cell);
            for &(cx, x0, x1) in &columns {
                let neighbours = grid.neighbours(cx, cy)?;
                // An interior cell's average is never used. Skip the entire run if
                // all nine possible winners have an on-image colour sample.
                if neighbours.iter().all(|&i| grid.sites.get(i).is_some_and(|s| s.sampled)) {
                    continue;
                }
                for x in x0..x1 {
                    let i = nearest(&grid, &neighbours, x, y)?;
                    if grid.sites.get(i)?.sampled {
                        continue;
                    }
                    let gx = grid.x0.checked_add((i % grid.width) as i32)?;
                    let gy = grid.y0.checked_add((i / grid.width) as i32)?;
                    if gx < c0.0 || gy < c0.1 || gx > c1.0 || gy > c1.1 {
                        continue;
                    }
                    let sum = acc.get_mut(i.checked_mul(n + 1)?..i.checked_add(1)?.checked_mul(n + 1)?)?;
                    let alpha = if ctx.alpha { src.get(x, y, n - 1) } else { 1.0 };
                    for (c, value) in sum.iter_mut().enumerate().take(n) {
                        let s = src.get(x, y, c);
                        *value += if ctx.alpha && c + 1 < n { s * alpha } else { s };
                    }
                    *sum.get_mut(n)? += 1.0;
                }
            }
        }
        for (site, sum) in grid.sites.iter_mut().zip(acc.chunks_exact(n + 1)) {
            let count = *sum.get(n)?;
            if !site.sampled && count > 0.0 {
                for (dst, value) in site.color.iter_mut().zip(sum).take(n) {
                    *dst = *value / count;
                }
                crate::fxutil::unpremul_px(site.color.get_mut(..n)?, ctx.alpha);
                site.sampled = true;
            }
        }
    }
    crystallize_grid(src, out, ctx, cell, &grid)
}

fn crystallize_grid(src: &Image, out: Rect, ctx: &Ctx, cell: f32, grid: &Grid) -> Option<Vec<f32>> {
    let n = src.ch;
    let b = ctx.bounds;
    let origin = (b.x0 as f32, b.y0 as f32);
    let mut result = if src.rect == out { src.data.clone() } else { src.crop(out) };
    let width = out.width() as usize;
    let stride = width.checked_mul(n)?;
    let columns = runs(out, origin.0, cell);
    let mut distances = vec![f32::MAX; width];
    let mut owners = vec![0u32; width];
    for (row, pixels) in result.chunks_exact_mut(stride).enumerate() {
        let y = out.y0.checked_add(i32::try_from(row).ok()?)?;
        let fy = y as f32 + 0.5;
        let cy = key(y, origin.1, cell);
        distances.fill(f32::MAX);
        for &(cx, x0, x1) in &columns {
            let neighbours = grid.neighbours(cx, cy)?;
            let start = usize::try_from(x0.checked_sub(out.x0)?).ok()?;
            let end = usize::try_from(x1.checked_sub(out.x0)?).ok()?;
            if cell >= 32.0 && out.width() <= 2048 {
                certified_span::<false>(grid, &neighbours, x0, y, owners.get_mut(start..end)?, distances.get_mut(start..end)?)?;
                continue;
            }
            owners.get_mut(start..end)?.fill(*neighbours.get(4)? as u32);
            for &i in &neighbours {
                let site = grid.sites.get(i)?;
                let dy2 = (site.point.1 - fy).powi(2);
                for (offset, (distance, owner)) in distances.get_mut(start..end)?.iter_mut().zip(owners.get_mut(start..end)?).enumerate() {
                    let fx = (x0 + offset as i32) as f32 + 0.5;
                    let d = (site.point.0 - fx).powi(2) + dy2;
                    if d < *distance {
                        *distance = d;
                        *owner = i as u32;
                    }
                }
            }
        }
        for (x, (pixel, &owner)) in pixels.chunks_exact_mut(n).zip(&owners).enumerate() {
            if !b.contains(out.x0 + x as i32, y) {
                continue;
            }
            let site = grid.sites.get(owner as usize)?;
            if site.sampled {
                pixel.copy_from_slice(site.color.get(..n)?);
            }
        }
    }
    Some(result)
}

/// A site may only compete in its own base column and its two neighbours.
/// Derive the inclusive pixel runs from the exact forward f32 cell mapping.
fn influence_columns(out: Rect, origin: f32, cell: f32, grid: &Grid) -> Option<Vec<(i32, i32)>> {
    let columns = runs(out, origin, cell);
    (0..grid.width)
        .map(|column| {
            let gx = grid.x0.checked_add(i32::try_from(column).ok()?)?;
            let lo = columns.partition_point(|&(cx, _, _)| cx < gx - 1);
            let hi = columns.partition_point(|&(cx, _, _)| cx <= gx + 1);
            if lo == hi {
                return Some((out.x0, out.x0));
            }
            Some((columns.get(lo)?.1, columns.get(hi - 1)?.2))
        })
        .collect()
}

pub(super) fn pointillize(src: &Image, out: Rect, ctx: &Ctx, cell_size: f32, seed: u32, background: [f32; 4]) -> Option<Vec<f32>> {
    let cell = cell_size.max(2.0);
    // This path uses integer pixel centres and conservative one-pixel support
    // margins. Beyond exact half-integer f32 coordinates use the scalar kernel.
    if cell > 4096.0 || [out.x0, out.y0, out.x1, out.y1].iter().any(|v| v.unsigned_abs() > 4_000_000) {
        return None;
    }
    let grid = Grid::new(src, out, ctx, cell, seed, true)?;
    pointillize_grid(src, out, ctx, cell, background, &grid)
}

fn pointillize_grid(src: &Image, out: Rect, ctx: &Ctx, cell: f32, background: [f32; 4], grid: &Grid) -> Option<Vec<f32>> {
    let origin = (ctx.bounds.x0 as f32, ctx.bounds.y0 as f32);
    let bg = native(ctx, background);
    let n = src.ch;
    let mut result = if src.rect == out { src.data.clone() } else { src.crop(out) };
    let columns = influence_columns(out, origin.0, cell, grid)?;
    let base_runs = runs(out, origin.0, cell);
    let width = out.width() as usize;
    let stride = width.checked_mul(n)?;
    let mut depths = vec![-0.5f32; width];
    let mut owners = vec![u32::MAX; width];
    for (row, pixels) in result.chunks_exact_mut(stride).enumerate() {
        let y = out.y0.checked_add(i32::try_from(row).ok()?)?;
        let fy = y as f32 + 0.5;
        let cy = key(y, origin.1, cell);
        depths.fill(-0.5);
        owners.fill(u32::MAX);
        if cell >= 32.0 && out.width() <= 2048 {
            for &(cx, x0, x1) in &base_runs {
                let ids = grid.neighbours(cx, cy)?;
                let start = usize::try_from(x0.checked_sub(out.x0)?).ok()?;
                let end = usize::try_from(x1.checked_sub(out.x0)?).ok()?;
                certified_span::<true>(grid, &ids, x0, y, owners.get_mut(start..end)?, depths.get_mut(start..end)?)?;
            }
        } else {
            for gy in cy.checked_sub(1)?..=cy.checked_add(1)? {
                let row = usize::try_from(gy.checked_sub(grid.y0)?).ok()?.checked_mul(grid.width)?;
                // Global gy/gx order is the original dy/dx tie order at every pixel.
                for (column, &(x0, x1)) in columns.iter().enumerate() {
                    if x0 >= x1 {
                        continue;
                    }
                    let i = row.checked_add(column)?;
                    let site = grid.sites.get(i)?;
                    let dy2 = (site.point.1 - fy).powi(2);
                    let reach = f64::from(site.radius) + 1.5;
                    let square = reach * reach - f64::from(dy2);
                    if square < 0.0 {
                        continue;
                    }
                    // The extra pixel covers subtraction/square/sqrt rounding for
                    // the bounded coordinates/radii above. The exact old depth
                    // comparison still decides every candidate that survives.
                    let reach_x = square.max(0.0).sqrt() + 1.0;
                    let lo = ((f64::from(site.point.0) - 0.5 - reach_x).floor() as i32).max(x0);
                    let hi = ((f64::from(site.point.0) - 0.5 + reach_x).ceil() as i32).min(x1);
                    if lo >= hi {
                        continue;
                    }
                    let start = usize::try_from(lo.checked_sub(out.x0)?).ok()?;
                    let end = usize::try_from(hi.checked_sub(out.x0)?).ok()?;
                    for (offset, (depth, owner)) in depths.get_mut(start..end)?.iter_mut().zip(owners.get_mut(start..end)?).enumerate() {
                        let fx = (lo + offset as i32) as f32 + 0.5;
                        let d = ((site.point.0 - fx).powi(2) + dy2).sqrt();
                        let candidate = site.radius - d;
                        if candidate > *depth {
                            *depth = candidate;
                            *owner = i as u32;
                        }
                    }
                }
            }
        }
        for ((pixel, &depth), &owner) in pixels.chunks_exact_mut(n).zip(&depths).zip(&owners) {
            let a_orig = if ctx.alpha { *pixel.get(n - 1)? } else { 1.0 };
            let mut canvas = bg;
            if ctx.alpha {
                *canvas.get_mut(n - 1)? = a_orig.max(bg.get(n - 1)?.min(1.0));
            }
            if owner != u32::MAX {
                let i = owner as usize;
                let k = (depth + 0.5).clamp(0.0, 1.0);
                let dot = grid.sites.get(i)?;
                for (c, value) in canvas.iter_mut().enumerate().take(n) {
                    *value += (*dot.color.get(c)? - *value) * k;
                }
            }
            pixel.copy_from_slice(canvas.get(..n)?);
        }
    }
    Some(result)
}

/// The filters sample cell centres, not a dense neighbourhood. On ordinary
/// tiles read those sparse samples directly; keep only the original output
/// pixels for alpha and selection mixing. Crystallize boundary tiles still
/// use the dense path, whose row-order f32 averages are part of its result.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tile(
    surface: &photocraft_raster::Surface,
    out: Rect,
    ctx: &Ctx,
    cell: f32,
    seed: u32,
    background: Option<[f32; 4]>,
    extent: Option<Rect>,
) -> Option<(Image, Vec<f32>)> {
    if !cell.is_finite()
        || !(2.0..=300.0).contains(&cell)
        || out.is_empty()
        || out.width() > 2048
        || out.height() > 2048
        || [out.x0, out.y0, out.x1, out.y1].iter().any(|v| v.unsigned_abs() > 4_000_000)
        || extent.is_some_and(|e| e.is_empty() || out.intersect(&e) != out)
    {
        return None;
    }
    let dots = background.is_some();
    let grid = Grid::new_with(out, ctx, surface.channels(), cell, seed, dots, |sx, sy, color| {
        let (x, y) = match extent {
            Some(e) => (sx.clamp(e.x0, e.x1 - 1), sy.clamp(e.y0, e.y1 - 1)),
            None => (sx, sy),
        };
        surface.read_pixel(x, y, color);
        ctx.bounds.contains(sx, sy)
    })?;
    if !dots && grid.sites.iter().any(|s| !s.sampled) {
        return None;
    }
    let original = Image::read(surface, out);
    let data = match background {
        Some(bg) => pointillize_grid(&original, out, ctx, cell, bg, &grid)?,
        None => crystallize_grid(&original, out, ctx, cell, &grid)?,
    };
    Some((original, data))
}

/// Same rounding and native byte order as Surface::write_region, before the
/// worker hands its result back. Bounds the allocation and retains float data
/// if an optional encode buffer cannot be reserved.
pub(crate) fn encode(format: photocraft_color::PixelFormat, samples: &[f32]) -> Option<Vec<u8>> {
    use photocraft_color::SampleType;
    let mut bytes = Vec::new();
    let len = samples.len().checked_mul(format.sample.bytes())?;
    bytes.try_reserve_exact(len).ok()?;
    bytes.resize(len, 0);
    match format.sample {
        SampleType::U8 => {
            for (dst, value) in bytes.iter_mut().zip(samples) {
                *dst = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        SampleType::U16 => {
            for (dst, value) in bytes.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                dst.copy_from_slice(&((value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_ne_bytes());
            }
        }
        SampleType::F32 => {
            for (dst, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
                dst.copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_raster::Surface;

    #[test]
    fn span_certificates_keep_discrete_winners_and_antialias_coverage() {
        for origin in [-16_777_210, -3_999_000, -37, 0, 3_999_000, 16_777_210] {
            let out = Rect::new(origin, -3, origin + 601, 4);
            let ctx = Ctx { bounds: out, mode: ColorMode::Rgb, alpha: true };
            let src = Image::new(out, 4);
            for cell in [32.0, 33.25, 100.0, 300.0, 4096.0] {
                for seed in 0..32 {
                    let grid = Grid::new(&src, out, &ctx, cell, seed, true).unwrap();
                    for y in out.y0..out.y1 {
                        let cy = key(y, ctx.bounds.y0 as f32, cell);
                        for (cx, x0, x1) in runs(out, ctx.bounds.x0 as f32, cell) {
                            let ids = grid.neighbours(cx, cy).unwrap();
                            for dots in [false, true] {
                                let mut owners = vec![u32::MAX; (x1 - x0) as usize];
                                let mut depths = vec![-0.5; owners.len()];
                                if dots {
                                    certified_span::<true>(&grid, &ids, x0, y, &mut owners, &mut depths).unwrap();
                                } else {
                                    certified_span::<false>(&grid, &ids, x0, y, &mut owners, &mut depths).unwrap();
                                }
                                for (offset, (&owner, &depth)) in owners.iter().zip(&depths).enumerate() {
                                    let x = x0 + offset as i32;
                                    let mut best = (if dots { -0.5 } else { -f32::MAX }, u32::MAX);
                                    for &i in &ids {
                                        let value = if dots { score::<true>(&grid.sites[i], x, y) } else { score::<false>(&grid.sites[i], x, y) };
                                        if value > best.0 {
                                            best = (value, i as u32);
                                        }
                                    }
                                    assert_eq!(owner, best.1, "x={x} y={y} cell={cell} seed={seed}");
                                    if dots && owner != u32::MAX {
                                        assert_eq!((depth + 0.5).clamp(0.0, 1.0).to_bits(), (best.0 + 0.5).clamp(0.0, 1.0).to_bits());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn worker_encoding_is_surface_write_region_at_every_depth() {
        let values = [f32::NAN, f32::NEG_INFINITY, -100.0, -0.0, f32::from_bits(1), 0.5 / 65535.0, 0.5 / 255.0, 0.5, 1.0, 1.1, f32::INFINITY];
        let area = Rect::new(-17, -9, 281, 2);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for alpha in [false, true] {
                    let format = PixelFormat::new(mode, sample, alpha);
                    let data: Vec<_> = (0..area.width() as usize * area.height() as usize * format.channels()).map(|i| values[i % values.len()]).collect();
                    let mut surface = Surface::new(format);
                    surface.write_region(area, &data);
                    assert_eq!(encode(format, &data).unwrap(), surface.to_interleaved(area));
                }
            }
        }
    }

    #[test]
    fn optional_grid_rejects_unbounded_or_incomplete_scratch() {
        let area = Rect::new(0, 0, 100_000, 100_000);
        let ctx = Ctx { bounds: area, mode: ColorMode::Rgb, alpha: true };
        let image = Image { rect: area, ch: 4, data: Vec::new() };
        assert!(Grid::new(&image, area, &ctx, 2.0, 0, true).is_none());
        assert!(Grid::new_with(area, &ctx, 4, 2.0, 0, true, |_, _, _| false).is_none());
        assert!(Grid::new_with(area, &ctx, 0, 2.0, 0, true, |_, _, _| false).is_none());
        assert!(Grid::new_with(area, &ctx, MAXC + 1, 2.0, 0, true, |_, _, _| false).is_none());
        assert!(Grid::new_with(area, &ctx, 4, f32::INFINITY, 0, true, |_, _, _| false).is_none());
    }
}
