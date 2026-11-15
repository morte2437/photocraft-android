//! Generic native coverage operations. No file-format identities or external resources.
//!
//! Hole filling is boundary-connected grayscale reconstruction: each sample is raised to
//! min(path to boundary) max(sample along path). This standard minimax definition is also
//! described by Barnes/Lehman/Mulla, Computers & Geosciences62 (2014),117–127:
//! <https://doi.org/10.1016/j.cageo.2013.04.024>. Independent implementation; no source code copied.
use crate::{Ctx, image::Image};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub const MAX_PIXELS: usize = 64 * 1024 * 1024;
pub const MAX_WORKSPACE_BYTES: usize = 1_500_000_000;
const MAX_EDGE: i64 = 65_536;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation<'a> {
    SetAlpha(f32),
    AlphaCurve(&'a [f32]),
    /// Extract native coverage onto stored neutral axes; opaque output, unchanged model.
    AlphaToColor,
    ClampAlpha {
        low: f32,
        high: f32,
    },
    /// Native rule: alpha strictly greater than value becomes1; equality becomes0.
    ThresholdAlpha(f32),
    FillHoles {
        connectivity: u8,
        alpha_only: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    InvalidGeometry,
    InvalidParameter,
    AlphaRequired,
    NonFiniteSample,
    WorkspaceLimit,
    Allocation,
}
impl std::fmt::Display for Error {
    /// Describe checked coverage failures without exposing implementation-specific error values to callers.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "native coverage operation cancelled",
            Self::InvalidGeometry => "native coverage bounds are invalid",
            Self::InvalidParameter => "invalid native coverage parameter",
            Self::AlphaRequired => "native coverage operation requires an alpha-capable layer",
            Self::NonFiniteSample => "native coverage operation requires finite samples",
            Self::WorkspaceLimit => "native coverage operation exceeds its bounded workspace",
            Self::Allocation => "native coverage workspace could not be allocated",
        })
    }
}
impl std::error::Error for Error {}
#[derive(Clone, Copy)]
struct Shape {
    width: usize,
    height: usize,
    pixels: usize,
}
/// Validate positive native dimensions and channel count, then bound pixel and reconstruction workspace arithmetic.
fn shape(rect: Rect, channels: usize, operation: Operation<'_>) -> Result<Shape, Error> {
    let width = i64::from(rect.x1) - i64::from(rect.x0);
    let height = i64::from(rect.y1) - i64::from(rect.y0);
    if width <= 0 || height <= 0 || width > MAX_EDGE || height > MAX_EDGE || channels == 0 || channels > 5 {
        return Err(Error::InvalidGeometry);
    }
    let width = usize::try_from(width).map_err(|_| Error::InvalidGeometry)?;
    let height = usize::try_from(height).map_err(|_| Error::InvalidGeometry)?;
    let pixels = width.checked_mul(height).ok_or(Error::WorkspaceLimit)?;
    // Only reconstruction visits/allocates the whole domain. Point operations admit
    // their clipped output tiles in preflight, even on a larger sparse owner.
    if matches!(operation, Operation::FillHoles { .. }) && pixels > MAX_PIXELS {
        return Err(Error::WorkspaceLimit);
    }
    // Plane/queue workspace is checked separately from padded native output tiles.
    let per_pixel = if matches!(operation, Operation::FillHoles { .. }) { std::mem::size_of::<Node>() + 5 } else { 0 };
    if pixels.checked_mul(per_pixel).is_none_or(|bytes| bytes > MAX_WORKSPACE_BYTES) {
        return Err(Error::WorkspaceLimit);
    }
    Ok(Shape { width, height, pixels })
}
/// Admit padded output tiles, cloned sparse headers and selection/topology scratch before allocating any result.
fn preflight(surface: &Surface, area: Rect, shape: Shape, operation: Operation<'_>, selection: Option<&Surface>) -> Result<(), Error> {
    let ts = i64::from(photocraft_geom::TILE_SIZE);
    let tile_count = |lo: i32, hi: i32| -> Result<usize, Error> {
        if hi <= lo {
            return Ok(0);
        }
        usize::try_from((i64::from(hi) - 1).div_euclid(ts) - i64::from(lo).div_euclid(ts) + 1).map_err(|_| Error::WorkspaceLimit)
    };
    let tile_bytes =
        usize::try_from(ts).ok().and_then(|n| n.checked_mul(n)).and_then(|n| n.checked_mul(surface.format().bytes_per_pixel())).ok_or(Error::WorkspaceLimit)?;
    let output = tile_count(area.x0, area.x1)?
        .checked_mul(tile_count(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul(tile_bytes.checked_add(128)?))
        .ok_or(Error::WorkspaceLimit)?;
    // Cloning the sparse Surface also duplicates its map nodes, even outside publication.
    let cloned_headers = surface.tile_count().checked_mul(128).ok_or(Error::WorkspaceLimit)?;
    let workspace = if matches!(operation, Operation::FillHoles { .. }) {
        shape.pixels.checked_mul(std::mem::size_of::<Node>() + 5).ok_or(Error::WorkspaceLimit)?
    } else {
        0
    };
    // Up to three interleaved f32 64x64 buffers coexist during selection publication.
    let scratch_channels =
        surface.channels().checked_mul(2).and_then(|n| n.checked_add(selection.map_or(0, Surface::channels))).ok_or(Error::WorkspaceLimit)?;
    let scratch = 64usize.checked_mul(64).and_then(|n| n.checked_mul(scratch_channels)).and_then(|n| n.checked_mul(4)).ok_or(Error::WorkspaceLimit)?;
    if output.checked_add(cloned_headers).and_then(|n| n.checked_add(workspace)).and_then(|n| n.checked_add(scratch)).is_none_or(|n| n > MAX_WORKSPACE_BYTES) {
        return Err(Error::WorkspaceLimit);
    }
    Ok(())
}
/// Reserve fallibly and initialize bounded chunks, checking cancellation between chunks.
fn zeroes<T: Clone>(len: usize, value: T, ctl: &Interrupt<'_>) -> Result<Vec<T>, Error> {
    let mut result = Vec::new();
    result.try_reserve_exact(len).map_err(|_| Error::Allocation)?;
    while result.len() < len {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let end = result.len().saturating_add(2048).min(len);
        result.resize(end, value.clone());
    }
    Ok(result)
}
/// Enforce finite coverage controls, exact curve shape and the alpha support required by each operation.
fn validate(operation: Operation<'_>, alpha: bool) -> Result<(), Error> {
    match operation {
        Operation::AlphaCurve(values) if values.len() != 256 || values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) => {
            Err(Error::InvalidParameter)
        }
        Operation::SetAlpha(value) if !value.is_finite() || !(0.0..=1.0).contains(&value) => Err(Error::InvalidParameter),
        Operation::ClampAlpha { low, high } if !low.is_finite() || !high.is_finite() || !(0.0..=1.0).contains(&low) || !(low..=1.0).contains(&high) => {
            Err(Error::InvalidParameter)
        }
        Operation::ThresholdAlpha(value) if !value.is_finite() || !(0.0..=1.0).contains(&value) => Err(Error::InvalidParameter),
        Operation::SetAlpha(_) | Operation::AlphaCurve(_) | Operation::AlphaToColor | Operation::ClampAlpha { .. } | Operation::ThresholdAlpha(_) if !alpha => {
            Err(Error::AlphaRequired)
        }
        Operation::FillHoles { connectivity, .. } if connectivity != 4 && connectivity != 8 => Err(Error::InvalidParameter),
        Operation::FillHoles { alpha_only: true, .. } if !alpha => Err(Error::AlphaRequired),
        _ => Ok(()),
    }
}
/// Evaluate native absolute alpha, strict threshold, clamp or piecewise-linear transfer rules on a finite sample.
fn alpha_value(operation: Operation<'_>, current: f32) -> Result<f32, Error> {
    match operation {
        Operation::SetAlpha(value) => Ok(value),
        Operation::AlphaCurve(values) if current.is_finite() => {
            let pos = current.clamp(0., 1.) * 255.;
            let index = (pos.floor() as usize).min(255);
            let t = pos - index as f32;
            let a = *values.get(index).ok_or(Error::InvalidParameter)?;
            let b = *values.get(index.saturating_add(1).min(255)).ok_or(Error::InvalidParameter)?;
            Ok(a + (b - a) * t)
        }
        Operation::ClampAlpha { low, high } if current.is_finite() => Ok(current.clamp(low, high)),
        Operation::ThresholdAlpha(value) if current.is_finite() => Ok(if current > value { 1. } else { 0. }),
        _ => Err(Error::NonFiniteSample),
    }
}
/// Place normalized coverage on the current model's neutral axes and make the pixel opaque, without an RGB conversion.
fn alpha_to_color(pixel: &mut [f32], mode: ColorMode) -> Result<(), Error> {
    let (alpha, color) = pixel.split_last_mut().ok_or(Error::AlphaRequired)?;
    if !alpha.is_finite() || color.iter().any(|value| !value.is_finite()) {
        return Err(Error::NonFiniteSample);
    }
    let value = alpha.clamp(0., 1.);
    color.fill(value);
    match mode {
        ColorMode::Cmyk => {
            color.fill(0.);
            *color.last_mut().ok_or(Error::InvalidGeometry)? = 1. - value;
        }
        ColorMode::Lab => {
            // Existing native Lab normalization is [L/100,(a+128)/255,(b+128)/255]
            // (raster::from_rgba_into). This is channel encoding, not an sRGB conversion.
            for channel in color.iter_mut().skip(1) {
                *channel = 128. / 255.;
            }
        }
        _ => {}
    }
    *alpha = 1.;
    Ok(())
}
/// Visit clipped 64-by-64 regions and stop immediately when a region callback fails.
fn tiles(rect: Rect, mut action: impl FnMut(Rect) -> Result<(), Error>) -> Result<(), Error> {
    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = y.saturating_add(64).min(rect.y1);
        let mut x = rect.x0;
        while x < rect.x1 {
            let x1 = x.saturating_add(64).min(rect.x1);
            action(Rect::new(x, y, x1, y1))?;
            x = x1;
        }
        y = y1;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
struct Node {
    level: f32,
    index: usize,
}
impl PartialEq for Node {
    /// Compare queue nodes by both pixel index and the total ordering of their spill level.
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.level.total_cmp(&other.level) == Ordering::Equal
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    /// Delegate to the deterministic total order used by the reconstruction heap.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    /// Reverse level and index ordering so BinaryHeap removes the lowest deterministic spill candidate first.
    fn cmp(&self, other: &Self) -> Ordering {
        other.level.total_cmp(&self.level).then_with(|| other.index.cmp(&self.index))
    }
}
/// Visit only in-bounds four- or eight-connected neighbors using checked index arithmetic.
fn neighbors(index: usize, shape: Shape, connectivity: u8, mut visit: impl FnMut(usize) -> Result<(), Error>) -> Result<(), Error> {
    let x = index % shape.width;
    let y = index / shape.width;
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if (dx == 0 && dy == 0) || (connectivity == 4 && dx != 0 && dy != 0) {
                continue;
            }
            let nx = i64::try_from(x).map_err(|_| Error::InvalidGeometry)? + dx;
            let ny = i64::try_from(y).map_err(|_| Error::InvalidGeometry)? + dy;
            if nx < 0 || ny < 0 {
                continue;
            }
            let nx = usize::try_from(nx).map_err(|_| Error::InvalidGeometry)?;
            let ny = usize::try_from(ny).map_err(|_| Error::InvalidGeometry)?;
            if nx >= shape.width || ny >= shape.height {
                continue;
            }
            let next = ny.checked_mul(shape.width).and_then(|v| v.checked_add(nx)).ok_or(Error::InvalidGeometry)?;
            visit(next)?;
        }
    }
    Ok(())
}
/// Raise enclosed samples by boundary-connected minimax reconstruction with finite-input, allocation and cancellation
/// checks.
fn fill(plane: &mut [f32], shape: Shape, connectivity: u8, ctl: &Interrupt<'_>) -> Result<(), Error> {
    if plane.len() != shape.pixels {
        return Err(Error::InvalidGeometry);
    }
    for chunk in plane.chunks(2048) {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        if chunk.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFiniteSample);
        }
    }
    let mut visited = zeroes(shape.pixels, false, ctl)?;
    let mut queue = BinaryHeap::new();
    queue.try_reserve_exact(shape.pixels).map_err(|_| Error::Allocation)?;
    for index in 0..shape.pixels {
        if index.is_multiple_of(2048) && ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let x = index % shape.width;
        let y = index / shape.width;
        if x == 0 || y == 0 || x + 1 == shape.width || y + 1 == shape.height {
            *visited.get_mut(index).ok_or(Error::InvalidGeometry)? = true;
            queue.push(Node { level: *plane.get(index).ok_or(Error::InvalidGeometry)?, index });
        }
    }
    let mut processed = 0usize;
    while let Some(node) = queue.pop() {
        if processed.is_multiple_of(2048) && ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        processed = processed.saturating_add(1);
        neighbors(node.index, shape, connectivity, |index| {
            let seen = visited.get_mut(index).ok_or(Error::InvalidGeometry)?;
            if *seen {
                return Ok(());
            }
            *seen = true;
            let value = plane.get_mut(index).ok_or(Error::InvalidGeometry)?;
            *value = value.max(node.level);
            queue.push(Node { level: *value, index });
            Ok(())
        })?;
    }
    Ok(())
}
/// Extract one finite native channel into a checked whole-owner plane using bounded tile reads.
fn plane_from(surface: &Surface, domain: Rect, shape: Shape, channel: usize, ctl: &Interrupt<'_>) -> Result<Vec<f32>, Error> {
    let mut plane = zeroes(shape.pixels, 0.0, ctl)?;
    let channels = surface.channels();
    tiles(domain, |tile| {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let data = surface.try_read_region(tile).map_err(|_| Error::Allocation)?;
        let width = usize::try_from(tile.width()).map_err(|_| Error::InvalidGeometry)?;
        for (pixel, samples) in data.chunks_exact(channels).enumerate() {
            let value = *samples.get(channel).ok_or(Error::InvalidGeometry)?;
            if !value.is_finite() {
                return Err(Error::NonFiniteSample);
            }
            let x = usize::try_from(i64::from(tile.x0) - i64::from(domain.x0))
                .map_err(|_| Error::InvalidGeometry)?
                .checked_add(pixel % width)
                .ok_or(Error::InvalidGeometry)?;
            let y = usize::try_from(i64::from(tile.y0) - i64::from(domain.y0))
                .map_err(|_| Error::InvalidGeometry)?
                .checked_add(pixel / width)
                .ok_or(Error::InvalidGeometry)?;
            let index = y.checked_mul(shape.width).and_then(|v| v.checked_add(x)).ok_or(Error::InvalidGeometry)?;
            *plane.get_mut(index).ok_or(Error::InvalidGeometry)? = value;
        }
        Ok(())
    })?;
    Ok(plane)
}

/// Whole-owner checked native operation. `domain` defines topology; `area` and `selection`
/// limit publication only. Failure/cancellation never changes the source surface.
pub fn apply_with(
    surface: &Surface,
    operation: Operation<'_>,
    area: Rect,
    domain: Rect,
    selection: Option<&Surface>,
    ctl: &Interrupt<'_>,
) -> Result<Surface, Error> {
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    validate(operation, surface.format().alpha)?;
    if area.x1 < area.x0 || area.y1 < area.y0 {
        return Err(Error::InvalidGeometry);
    }
    let shape = shape(domain, surface.channels(), operation)?;
    let area = area.intersect(&domain);
    preflight(surface, area, shape, operation, selection)?;
    let mut output = surface.clone();
    if area.is_empty() {
        return Ok(output);
    }
    let channels = surface.channels();
    match operation {
        Operation::SetAlpha(_) | Operation::AlphaCurve(_) | Operation::ClampAlpha { .. } | Operation::ThresholdAlpha(_) => tiles(area, |tile| {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let mut data = surface.try_read_region(tile).map_err(|_| Error::Allocation)?;
            for pixel in data.chunks_exact_mut(channels) {
                if pixel.iter().any(|value| !value.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                let alpha = pixel.last_mut().ok_or(Error::AlphaRequired)?;
                *alpha = alpha_value(operation, *alpha)?;
            }
            output.try_write_region(tile, &data).map_err(|_| Error::Allocation)
        })?,
        Operation::AlphaToColor => tiles(area, |tile| {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let mut data = surface.try_read_region(tile).map_err(|_| Error::Allocation)?;
            for pixel in data.chunks_exact_mut(channels) {
                alpha_to_color(pixel, surface.format().mode)?;
            }
            output.try_write_region(tile, &data).map_err(|_| Error::Allocation)
        })?,
        Operation::FillHoles { connectivity, alpha_only } => {
            let first = if alpha_only { channels.checked_sub(1).ok_or(Error::AlphaRequired)? } else { 0 };
            let count = channels - first;
            for channel in first..channels {
                let mut plane = plane_from(surface, domain, shape, channel, ctl)?;
                fill(&mut plane, shape, connectivity, ctl)?;
                tiles(area, |tile| {
                    if ctl.cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let mut data = output.try_read_region(tile).map_err(|_| Error::Allocation)?;
                    let width = usize::try_from(tile.width()).map_err(|_| Error::InvalidGeometry)?;
                    for (pixel, samples) in data.chunks_exact_mut(channels).enumerate() {
                        let x = usize::try_from(i64::from(tile.x0) - i64::from(domain.x0))
                            .map_err(|_| Error::InvalidGeometry)?
                            .checked_add(pixel % width)
                            .ok_or(Error::InvalidGeometry)?;
                        let y = usize::try_from(i64::from(tile.y0) - i64::from(domain.y0))
                            .map_err(|_| Error::InvalidGeometry)?
                            .checked_add(pixel / width)
                            .ok_or(Error::InvalidGeometry)?;
                        let index = y.checked_mul(shape.width).and_then(|v| v.checked_add(x)).ok_or(Error::InvalidGeometry)?;
                        *samples.get_mut(channel).ok_or(Error::InvalidGeometry)? = *plane.get(index).ok_or(Error::InvalidGeometry)?;
                    }
                    output.try_write_region(tile, &data).map_err(|_| Error::Allocation)
                })?;
                ctl.progress((channel - first + 1) as f32 / count as f32);
            }
        }
    }
    if let Some(selection) = selection {
        tiles(area, |tile| {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let selected = selection.try_read_region(tile).map_err(|_| Error::Allocation)?;
            if selected.chunks_exact(selection.channels()).any(|pixel| pixel.first().is_none_or(|v| !v.is_finite())) {
                return Err(Error::NonFiniteSample);
            }
            let mut filtered = output.try_read_region(tile).map_err(|_| Error::Allocation)?;
            let original = Image { rect: tile, ch: channels, data: surface.try_read_region(tile).map_err(|_| Error::Allocation)? };
            super::mix_selection(&mut filtered, tile, selection, &original, surface.format().alpha);
            if filtered.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            output.try_write_region(tile, &filtered).map_err(|_| Error::Allocation)
        })?;
    }
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    ctl.progress(1.0);
    Ok(output)
}

/// Legacy infallible raw-kernel adapter. Valid image regions retain identity on a rejected
/// operation rather than returning a short buffer. Production Surface callers use checked
/// apply_with directly, so workspace/parameter errors are never reported as successful edits.
/// As with other raw kernels, malformed image geometry is outside this low-level contract;
/// it yields no samples, never an input-derived unbounded allocation or panic.
pub(crate) fn run_image(source: &Image, out: Rect, ctx: &Ctx, operation: Operation<'_>) -> Vec<f32> {
    let run = || -> Result<Vec<f32>, Error> {
        if PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha).channels() != source.ch {
            return Err(Error::InvalidGeometry);
        }
        validate(operation, ctx.alpha)?;
        let geometry = shape(source.rect, source.ch, operation)?;
        let mut result = identity_region(source, out)?;
        let workspace = if matches!(operation, Operation::FillHoles { .. }) {
            geometry.pixels.checked_mul(std::mem::size_of::<Node>() + 5).ok_or(Error::WorkspaceLimit)?
        } else {
            0
        };
        if result.len().checked_mul(4).and_then(|n| n.checked_add(workspace)).is_none_or(|n| n > MAX_WORKSPACE_BYTES) {
            return Err(Error::WorkspaceLimit);
        }
        match operation {
            Operation::SetAlpha(_) | Operation::AlphaCurve(_) | Operation::ClampAlpha { .. } | Operation::ThresholdAlpha(_) => {
                for pixel in result.chunks_exact_mut(source.ch) {
                    let a = pixel.last_mut().ok_or(Error::AlphaRequired)?;
                    *a = alpha_value(operation, *a)?;
                }
            }
            Operation::AlphaToColor => {
                for pixel in result.chunks_exact_mut(source.ch) {
                    alpha_to_color(pixel, ctx.mode)?;
                }
            }
            Operation::FillHoles { connectivity, alpha_only } => {
                let first = if alpha_only { source.ch.checked_sub(1).ok_or(Error::AlphaRequired)? } else { 0 };
                for channel in first..source.ch {
                    let mut plane = zeroes(geometry.pixels, 0., &Interrupt::NONE)?;
                    for (target, pixel) in plane.iter_mut().zip(source.data.chunks_exact(source.ch)) {
                        *target = *pixel.get(channel).ok_or(Error::InvalidGeometry)?;
                    }
                    fill(&mut plane, geometry, connectivity, &Interrupt::NONE)?;
                    let width = usize::try_from(i64::from(out.x1) - i64::from(out.x0)).map_err(|_| Error::InvalidGeometry)?;
                    let x = usize::try_from(i64::from(out.x0) - i64::from(source.rect.x0)).map_err(|_| Error::InvalidGeometry)?;
                    let y = usize::try_from(i64::from(out.y0) - i64::from(source.rect.y0)).map_err(|_| Error::InvalidGeometry)?;
                    for (i, pixel) in result.chunks_exact_mut(source.ch).enumerate() {
                        let index = y
                            .checked_add(i / width)
                            .and_then(|n| n.checked_mul(geometry.width))
                            .and_then(|n| n.checked_add(x))
                            .and_then(|n| n.checked_add(i % width))
                            .ok_or(Error::InvalidGeometry)?;
                        *pixel.get_mut(channel).ok_or(Error::InvalidGeometry)? = *plane.get(index).ok_or(Error::InvalidGeometry)?;
                    }
                }
            }
        }
        Ok(result)
    };
    run().unwrap_or_else(|_| identity_region(source, out).unwrap_or_default())
}

/// Copy a valid contained raw-image region for the infallible adapter's rejected-operation fallback.
fn identity_region(source: &Image, out: Rect) -> Result<Vec<f32>, Error> {
    let input = shape(source.rect, source.ch, Operation::SetAlpha(1.))?;
    let output = shape(out, source.ch, Operation::SetAlpha(1.))?;
    if out.intersect(&source.rect) != out || input.pixels.checked_mul(source.ch) != Some(source.data.len()) {
        return Err(Error::InvalidGeometry);
    }
    let len = output.pixels.checked_mul(source.ch).filter(|n| n.checked_mul(4).is_some_and(|n| n <= MAX_WORKSPACE_BYTES)).ok_or(Error::WorkspaceLimit)?;
    let mut result = Vec::new();
    result.try_reserve_exact(len).map_err(|_| Error::Allocation)?;
    let row_len = output.width.checked_mul(source.ch).ok_or(Error::InvalidGeometry)?;
    let x = usize::try_from(i64::from(out.x0) - i64::from(source.rect.x0)).map_err(|_| Error::InvalidGeometry)?;
    let y = usize::try_from(i64::from(out.y0) - i64::from(source.rect.y0)).map_err(|_| Error::InvalidGeometry)?;
    for row in 0..output.height {
        let start = y
            .checked_add(row)
            .and_then(|n| n.checked_mul(input.width))
            .and_then(|n| n.checked_add(x))
            .and_then(|n| n.checked_mul(source.ch))
            .ok_or(Error::InvalidGeometry)?;
        let end = start.checked_add(row_len).ok_or(Error::InvalidGeometry)?;
        result.extend_from_slice(source.data.get(start..end).ok_or(Error::InvalidGeometry)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    #[test]
    /// Allow small point edits on large owners while keeping reconstruction, geometry and output workspace limits.
    fn small_point_operations_in_large_domain_keep_reconstruction_and_output_limits() {
        let area = Rect::new(11, 13, 12, 14);
        let s = surface(ColorMode::Rgb, SampleType::F32, true, area, &[0.2, 0.4, 0.6, 0.5]);
        let domain = Rect::new(0, 0, 12_000, 8_000);
        let curve: Vec<_> = (0..256).map(|i| i as f32 / 255.).collect();
        for op in [
            Operation::SetAlpha(0.25),
            Operation::AlphaCurve(&curve),
            Operation::AlphaToColor,
            Operation::ClampAlpha { low: 0., high: 0.3 },
            Operation::ThresholdAlpha(0.4),
        ] {
            assert_eq!(apply_with(&s, op, area, domain, None, &Interrupt::NONE).unwrap(), apply(&s, op, area));
            assert_eq!(apply_with(&s, op, domain, domain, None, &Interrupt::NONE), Err(Error::WorkspaceLimit));
        }
        let fill = Operation::FillHoles { connectivity: 4, alpha_only: true };
        assert_eq!(apply_with(&s, fill, area, domain, None, &Interrupt::NONE), Err(Error::WorkspaceLimit));
        assert_eq!(apply_with(&s, Operation::SetAlpha(0.5), area, Rect::new(0, 0, 0, 1), None, &Interrupt::NONE), Err(Error::InvalidGeometry));
    }
    /// Encode explicit test samples at the requested native mode, depth and alpha layout.
    fn surface(mode: ColorMode, depth: SampleType, alpha: bool, rect: Rect, data: &[f32]) -> Surface {
        let mut s = Surface::new(PixelFormat::new(mode, depth, alpha));
        s.try_write_region(rect, data).unwrap();
        s
    }
    /// Run a checked operation over the complete fixture domain with no selection or cancellation.
    fn apply(s: &Surface, op: Operation, r: Rect) -> Surface {
        apply_with(s, op, r, r, None, &Interrupt::NONE).unwrap()
    }
    #[test]
    /// Compare heap-based reconstruction against a separately iterated grayscale relaxation oracle over varied inputs.
    fn grayscale_reconstruction_matches_independent_iterative_oracle() {
        let r = Rect::new(-3, 2, 4, 8);
        let (w, h) = (7usize, 6usize);
        for seed in 0..12u32 {
            let mut state = seed + 1;
            let input: Vec<f32> = (0..w * h)
                .map(|_| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    (state % 17) as f32 / 16.
                })
                .collect();
            let mut oracle = vec![f32::INFINITY; input.len()];
            for y in 0..h {
                for x in 0..w {
                    if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                        oracle[y * w + x] = input[y * w + x];
                    }
                }
            }
            loop {
                let previous = oracle.clone();
                for y in 1..h - 1 {
                    for x in 1..w - 1 {
                        let i = y * w + x;
                        oracle[i] = input[i].max(previous[i - 1].min(previous[i + 1]).min(previous[i - w]).min(previous[i + w]));
                    }
                }
                if oracle == previous {
                    break;
                }
            }
            let s = surface(ColorMode::Grayscale, SampleType::F32, false, r, &input);
            let got = apply(&s, Operation::FillHoles { connectivity: 4, alpha_only: false }, r);
            assert_eq!(got.read_region(r), oracle);
        }
    }
    #[test]
    /// Pin the different diagonal escape rules and the exact enclosing spill level with small hand-authored planes.
    fn connectivity_distinguishes_diagonal_escape_and_preserves_spill_levels() {
        let r = Rect::new(0, 0, 3, 3);
        let input = [0., 1., 1., 1., 0.2, 1., 1., 1., 1.];
        let s = surface(ColorMode::Grayscale, SampleType::F32, false, r, &input);
        assert_eq!(apply(&s, Operation::FillHoles { connectivity: 4, alpha_only: false }, r).pixel(1, 1), vec![1.]);
        assert_eq!(apply(&s, Operation::FillHoles { connectivity: 8, alpha_only: false }, r).pixel(1, 1), vec![0.2]);
        let s = surface(ColorMode::Grayscale, SampleType::F32, false, r, &[0.7, 0.7, 0.7, 0.7, 0.1, 0.7, 0.7, 0.7, 0.7]);
        assert_eq!(apply(&s, Operation::FillHoles { connectivity: 4, alpha_only: false }, r).pixel(1, 1), vec![0.7]);
    }
    #[test]
    /// Require absolute alpha replacement to leave all stored color channels unchanged across native models and depths.
    fn every_native_model_depth_keeps_color_when_replacing_absolute_alpha() {
        let r = Rect::new(0, 0, 2, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let fmt = PixelFormat::new(mode, depth, true);
                let ch = fmt.channels();
                let mut data = vec![0.37; ch * 2];
                data[ch - 1] = 0.;
                data[ch * 2 - 1] = 1.;
                if depth == SampleType::F32 {
                    data[0] = -2.;
                    data[ch] = 7.;
                }
                let s = surface(mode, depth, true, r, &data);
                let before = s.read_region(r);
                let got = apply(&s, Operation::SetAlpha(0.5), r).read_region(r);
                for i in 0..ch * 2 {
                    if i % ch == ch - 1 {
                        assert!((got[i] - 0.5).abs() < 0.003)
                    } else {
                        assert_eq!(got[i], before[i]);
                    }
                }
                let ring = Rect::new(0, 0, 3, 3);
                let mut values = vec![0.4; ch * 9];
                for pixel in values.chunks_exact_mut(ch) {
                    pixel[ch - 1] = 1.;
                    if depth == SampleType::F32 {
                        pixel[0] = -2.;
                    }
                }
                values[4 * ch + ch - 1] = 0.;
                let s = surface(mode, depth, true, ring, &values);
                let before = s.read_region(ring);
                let filled = apply(&s, Operation::FillHoles { connectivity: 4, alpha_only: true }, ring).read_region(ring);
                for (old, new) in before.chunks_exact(ch).zip(filled.chunks_exact(ch)) {
                    assert_eq!(&old[..ch - 1], &new[..ch - 1]);
                    assert_eq!(new[ch - 1], 1.);
                }
            }
        }
    }
    #[test]
    /// Check native alpha bounds and strict greater-than thresholding against already quantized source samples.
    fn clamp_and_strict_threshold_preserve_native_color_and_quantized_boundary_rules() {
        let r = Rect::new(0, 0, 5, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let channels = PixelFormat::new(mode, depth, true).channels();
                let data: Vec<f32> = (0..5)
                    .flat_map(|i| {
                        let mut pixel = vec![if depth == SampleType::F32 { -2. } else { 0.3 }; channels];
                        pixel[channels - 1] = i as f32 / 4.;
                        pixel
                    })
                    .collect();
                let s = surface(mode, depth, true, r, &data);
                let before = s.read_region(r);
                for operation in [Operation::ClampAlpha { low: 0.25, high: 0.75 }, Operation::ThresholdAlpha(0.5)] {
                    let got = apply(&s, operation, r).read_region(r);
                    for (original, changed) in before.chunks_exact(channels).zip(got.chunks_exact(channels)) {
                        assert_eq!(&changed[..channels - 1], &original[..channels - 1]);
                        let expected = alpha_value(operation, original[channels - 1]).unwrap();
                        assert!((changed[channels - 1] - expected).abs() < 0.003);
                    }
                }
            }
        }
        for operation in [Operation::ClampAlpha { low: 0.8, high: 0.2 }, Operation::ClampAlpha { low: f32::NAN, high: 1. }, Operation::ThresholdAlpha(2.)] {
            let s = surface(ColorMode::Rgb, SampleType::F32, true, r, &[0.; 20]);
            assert!(apply_with(&s, operation, r, r, None, &Interrupt::NONE).is_err());
        }
    }
    #[test]
    /// Compare alpha transfer to independently calculated square samples and midpoint interpolation, including invalid
    /// table rejection.
    fn native_alpha_transfer_interpolates_preserves_colors_and_respects_depth() {
        let values: Vec<f32> = (0..256).map(|i| (i as f32 / 255.).powi(2)).collect();
        let r = Rect::new(0, 0, 5, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let ch = PixelFormat::new(mode, depth, true).channels();
                let data: Vec<f32> = (0..5)
                    .flat_map(|i| {
                        let mut p = vec![if depth == SampleType::F32 { -3. } else { 0.4 }; ch];
                        p[ch - 1] = i as f32 / 4.;
                        p
                    })
                    .collect();
                let s = surface(mode, depth, true, r, &data);
                let original = s.read_region(r);
                let got = apply(&s, Operation::AlphaCurve(&values), r).read_region(r);
                for (a, b) in original.chunks_exact(ch).zip(got.chunks_exact(ch)) {
                    assert_eq!(&a[..ch - 1], &b[..ch - 1]);
                    assert!((b[ch - 1] - a[ch - 1].powi(2)).abs() < 0.003);
                }
            }
        }
        assert!((alpha_value(Operation::AlphaCurve(&values), 0.5).unwrap() - (values[127] + values[128]) / 2.).abs() < 1e-7);
        let s = surface(ColorMode::Rgb, SampleType::F32, true, r, &[-3.; 20]);
        for bad in [&values[..255], &[f32::NAN; 256][..], &[1.1; 256][..]] {
            assert_eq!(apply_with(&s, Operation::AlphaCurve(bad), r, r, None, &Interrupt::NONE).unwrap_err(), Error::InvalidParameter);
        }
    }
    #[test]
    /// Verify mode-specific neutral encodings and opaque output, including native CMYK and Lab normalization.
    fn alpha_to_color_is_opaque_native_neutral_data_without_model_conversion() {
        let r = Rect::new(0, 0, 3, 1);
        for mode in [
            ColorMode::Bitmap,
            ColorMode::Grayscale,
            ColorMode::Indexed,
            ColorMode::Rgb,
            ColorMode::Cmyk,
            ColorMode::Lab,
            ColorMode::Multichannel,
            ColorMode::Duotone,
        ] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let ch = PixelFormat::new(mode, depth, true).channels();
                let data: Vec<f32> = (0..3)
                    .flat_map(|i| {
                        let mut p = vec![0.7; ch];
                        p[ch - 1] = i as f32 / 2.;
                        p
                    })
                    .collect();
                let s = surface(mode, depth, true, r, &data);
                let old = s.read_region(r);
                let got = apply(&s, Operation::AlphaToColor, r);
                assert_eq!(got.format(), s.format());
                for (input, p) in old.chunks_exact(ch).zip(got.read_region(r).chunks_exact(ch)) {
                    let a = input[ch - 1];
                    assert_eq!(p[ch - 1], 1.);
                    for (i, value) in p.iter().take(ch - 1).enumerate() {
                        let expected = match mode {
                            ColorMode::Cmyk => {
                                if i == 3 {
                                    1. - a
                                } else {
                                    0.
                                }
                            }
                            ColorMode::Lab => {
                                if i == 0 {
                                    a
                                } else {
                                    128. / 255.
                                }
                            }
                            _ => a,
                        };
                        assert!((value - expected).abs() < 0.003);
                    }
                }
            }
        }
        let normalization = photocraft_raster::from_rgba(&PixelFormat::new(ColorMode::Lab, SampleType::F32, true), [0., 0., 0., 1.]);
        assert_eq!(&normalization[1..3], &[128. / 255., 128. / 255.]);
    }
    #[test]
    /// Pin selection mixing once and require cancelled alpha transfers to leave their HDR source unchanged.
    fn alpha_transfer_selection_and_cancellation_are_atomic() {
        let r = Rect::new(0, 0, 128, 128);
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.fill_rect(r, &[-2., 3., 0.4, 0.25]);
        let original = s.read_region(r);
        let values: Vec<f32> = (0..256).map(|i| i as f32 / 255.).collect();
        let mask = surface(ColorMode::Grayscale, SampleType::F32, false, Rect::new(0, 0, 1, 1), &[0.5]);
        let got = apply_with(&s, Operation::AlphaToColor, Rect::new(0, 0, 1, 1), r, Some(&mask), &Interrupt::NONE).unwrap();
        assert_eq!(got.pixel(0, 0)[3], 0.625);
        assert!((got.pixel(0, 0)[0] + 0.2).abs() < 1e-6);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 2;
        assert_eq!(apply_with(&s, Operation::AlphaCurve(&values), r, r, None, &Interrupt::cancel_only(&cancel)).unwrap_err(), Error::Cancelled);
        assert_eq!(s.read_region(r), original);
    }
    #[test]
    /// Reject nonfinite color or coverage with and without a selection, including failures after a valid tile.
    fn alpha_operations_reject_nonfinite_samples_without_changing_source() {
        let area = Rect::new(0, 0, 65, 1);
        let curve: Vec<f32> = (0..256).map(|value| value as f32 / 255.).collect();
        let selection = surface(ColorMode::Grayscale, SampleType::F32, false, area, &[0.5; 65]);
        for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
            let format = PixelFormat::new(mode, SampleType::F32, true);
            for channel in [0, format.channels() - 1] {
                for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let mut source = Surface::new(format);
                    let mut pixel = vec![0.5; format.channels()];
                    source.fill_rect(area, &pixel);
                    pixel[channel] = invalid;
                    source.write_pixel(64, 0, &pixel);
                    let before: Vec<_> = source.read_region(area).into_iter().map(f32::to_bits).collect();
                    for operation in [
                        Operation::SetAlpha(0.25),
                        Operation::AlphaCurve(&curve),
                        Operation::ClampAlpha { low: 0.1, high: 0.9 },
                        Operation::ThresholdAlpha(0.25),
                        Operation::AlphaToColor,
                    ] {
                        for mask in [None, Some(&selection)] {
                            assert_eq!(
                                apply_with(&source, operation, area, area, mask, &Interrupt::NONE).unwrap_err(),
                                Error::NonFiniteSample,
                                "{mode:?}/{operation:?}/channel={channel}/sample={invalid}"
                            );
                            assert_eq!(source.read_region(area).into_iter().map(f32::to_bits).collect::<Vec<_>>(), before);
                        }
                    }
                }
            }
        }
    }
    #[test]
    /// Fill an alpha hole without changing HDR color, then verify a half selection publishes half coverage only once.
    fn alpha_topology_preserves_hdr_color_and_selection_publishes_only_once() {
        let r = Rect::new(0, 0, 3, 3);
        let data: Vec<f32> = (0..9).flat_map(|i| [-2., 4., 0.3, if i == 4 { 0. } else { 1. }]).collect();
        let s = surface(ColorMode::Rgb, SampleType::F32, true, r, &data);
        let got = apply(&s, Operation::FillHoles { connectivity: 4, alpha_only: true }, r);
        assert_eq!(got.pixel(1, 1), vec![-2., 4., 0.3, 1.]);
        let mask = surface(ColorMode::Grayscale, SampleType::F32, false, r, &[0.5; 9]);
        let got = apply_with(&s, Operation::SetAlpha(1.), r, r, Some(&mask), &Interrupt::NONE).unwrap();
        assert_eq!(got.pixel(1, 1)[3], 0.5);
        assert_eq!(s.pixel(1, 1)[3], 0.);
    }
    #[test]
    /// Ensure hole connectivity is computed over the full owner even when only a small result area is published.
    fn topology_uses_whole_owner_even_with_small_publication_and_filter_tiles() {
        let r = Rect::new(0, 0, 7, 7);
        let mut data = vec![1.; 49];
        for y in 1..6 {
            for x in 1..6 {
                data[y * 7 + x] = 0.;
            }
        }
        let s = surface(ColorMode::Grayscale, SampleType::F32, false, r, &data);
        let fp = crate::FilterParams::FillHoles { connectivity: 4, alpha_only: false };
        for tile in [1, 2, 64] {
            let out = crate::apply_tiled_with(&s, &fp, Rect::new(3, 3, 4, 4), r, None, tile, Some(r), &Interrupt::NONE).unwrap();
            assert_eq!(out.pixel(3, 3), vec![1.]);
            assert_eq!(out.pixel(2, 3), vec![0.]);
        }
    }
    #[test]
    /// Require rejected alpha work on a non-alpha raw image to return the correctly shaped unchanged region.
    fn infallible_raw_adapter_keeps_valid_identity_shape_when_operation_is_rejected() {
        let image = Image { rect: Rect::new(0, 0, 3, 2), ch: 1, data: vec![0.3; 6] };
        let ctx = Ctx { mode: ColorMode::Grayscale, alpha: false, bounds: image.rect };
        assert_eq!(run_image(&image, Rect::new(1, 0, 3, 2), &ctx, Operation::SetAlpha(0.5)), vec![0.3; 4]);
    }
    #[test]
    /// Reject an offset, thin surface whose padded native tiles exceed the workspace despite its smaller visible area.
    fn padded_output_tiles_are_charged_before_allocating_thin_or_unaligned_surfaces() {
        let s = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true));
        let rect = Rect::new(255, 0, 767, 65_536);
        let shape = shape(rect, 5, Operation::FillHoles { connectivity: 4, alpha_only: false }).unwrap();
        assert_eq!(preflight(&s, rect, shape, Operation::FillHoles { connectivity: 4, alpha_only: false }, None), Err(Error::WorkspaceLimit));
    }
    #[test]
    /// Exercise invalid controls, nonfinite samples and mid-work cancellation without mutating the original surface.
    fn hostile_input_and_midwork_cancellation_are_atomic() {
        let r = Rect::new(0, 0, 128, 128);
        let s = surface(ColorMode::Grayscale, SampleType::F32, false, r, &vec![0.2; 128 * 128]);
        let before = s.read_region(r);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 5;
        assert_eq!(
            apply_with(&s, Operation::FillHoles { connectivity: 4, alpha_only: false }, r, r, None, &Interrupt::cancel_only(&cancel)).unwrap_err(),
            Error::Cancelled
        );
        assert_eq!(s.read_region(r), before);
        for op in [Operation::SetAlpha(0.5), Operation::SetAlpha(f32::NAN), Operation::FillHoles { connectivity: 9, alpha_only: false }] {
            assert!(apply_with(&s, op, r, r, None, &Interrupt::NONE).is_err());
        }
        assert!(
            apply_with(&s, Operation::FillHoles { connectivity: 4, alpha_only: false }, r, Rect::new(i32::MIN, 0, i32::MAX, 2), None, &Interrupt::NONE)
                .is_err()
        );
        let n = surface(ColorMode::Grayscale, SampleType::F32, false, Rect::new(0, 0, 1, 1), &[f32::NAN]);
        assert_eq!(
            apply_with(&n, Operation::FillHoles { connectivity: 4, alpha_only: false }, Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1), None, &Interrupt::NONE)
                .unwrap_err(),
            Error::NonFiniteSample
        );
    }
}
