//! Average changes colour, never coverage. Keep the existing RGB conversion and selection
//! weighting; Photoshop RGB8 observations are documented in docs/average-filter.md.
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface, from_rgba_into, to_rgba};

use crate::{Ctx, Image};

#[derive(Default)]
struct Mean {
    sum: [f64; 3],
    weight: f64,
}

impl Mean {
    fn add(&mut self, fmt: &PixelFormat, px: &[f32], coverage: f32) {
        let rgba = to_rgba(fmt, px);
        let w = f64::from(coverage);
        for (sum, value) in self.sum.iter_mut().zip(rgba) {
            *sum += f64::from(value) * w;
        }
        self.weight += w;
    }

    fn colour(&self) -> Option<[f32; 3]> {
        (self.weight > 0.0).then(|| self.sum.map(|s| (s / self.weight) as f32))
    }
}

fn mix(fmt: &PixelFormat, px: &mut [f32], colour: [f32; 3], coverage: f32) {
    if coverage <= 0.0 {
        return;
    }
    let mut rgba = to_rgba(fmt, px);
    for (value, mean) in rgba.iter_mut().zip(colour) {
        *value += (mean - *value) * coverage;
    }
    from_rgba_into(fmt, rgba, px);
}

// Rect's public coordinates can span more than i32::MAX. Reject these before raster helpers
// do their signed coordinate arithmetic, and cap a single operation's sample count.
fn valid_area(area: Rect, channels: usize) -> bool {
    area.width() <= i32::MAX as u32
        && area.height() <= i32::MAX as u32
        && u64::from(area.width()).saturating_mul(u64::from(area.height())).saturating_mul(channels as u64) <= 512 * 1024 * 1024
}

pub(crate) fn apply(surface: &Surface, area: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Option<Surface> {
    let fmt = surface.format();
    let n = fmt.channels();
    if !valid_area(area, n) || ctl.cancelled() {
        return None;
    }
    if area.is_empty() {
        return Some(surface.clone());
    }
    let mut raw = surface.try_read_region(area).ok()?;
    let width = area.width() as usize;
    let coverage = |i: usize| selection.map_or(1.0, |s| s.sample_channel(area.x0 + (i % width) as i32, area.y0 + (i / width) as i32, 0));
    let mut mean = Mean::default();
    for (i, px) in raw.chunks_exact(n).enumerate() {
        if i % width == 0 && ctl.cancelled() {
            return None;
        }
        mean.add(&fmt, px, coverage(i));
    }
    let Some(colour) = mean.colour() else { return Some(surface.clone()) };
    for (i, px) in raw.chunks_exact_mut(n).enumerate() {
        if i % width == 0 && ctl.cancelled() {
            return None;
        }
        mix(&fmt, px, colour, coverage(i));
    }
    let mut out = surface.clone();
    if ctl.cancelled() {
        return None;
    }
    out.try_write_region(area, &raw).ok()?;
    out.prune();
    ctl.progress(1.0);
    Some(out)
}

// The direct kernel API has no selection. Its mean is over the reference bounds, even when
// the requested output is only a tile; the surface dispatcher above also handles selections.
pub(crate) fn kernel(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    let fmt = PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha);
    let n = fmt.channels();
    if src.ch != n || !valid_area(src.rect, n) || !valid_area(out, n) {
        return Vec::new();
    }
    let read = |x: i32, y: i32| -> Option<&[f32]> {
        if !src.rect.contains(x, y) {
            return None;
        }
        let i = (i64::from(y) - i64::from(src.rect.y0)) as usize;
        let j = (i64::from(x) - i64::from(src.rect.x0)) as usize;
        let start = i.checked_mul(src.rect.width() as usize)?.checked_add(j)?.checked_mul(n)?;
        src.data.get(start..start.checked_add(n)?)
    };
    let bounds = ctx.bounds.intersect(&src.rect);
    let mut mean = Mean::default();
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            if let Some(px) = read(x, y) {
                mean.add(&fmt, px, 1.0);
            }
        }
    }
    let len = (out.width() as usize).checked_mul(out.height() as usize).and_then(|s| s.checked_mul(n));
    let Some(len) = len else { return Vec::new() };
    let mut result = Vec::new();
    if result.try_reserve_exact(len).is_err() {
        return result;
    }
    let colour = mean.colour();
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let mut px = [0.0; 5];
            let Some(pixel) = px.get_mut(..n) else { return Vec::new() };
            if let Some(source) = read(x, y) {
                pixel.copy_from_slice(source);
            }
            if let Some(colour) = colour {
                mix(&fmt, pixel, colour, 1.0);
            }
            result.extend_from_slice(pixel);
        }
    }
    result
}

#[cfg(test)]
mod tests;
