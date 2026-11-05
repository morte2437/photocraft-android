use std::fmt;

use photocraft_color::SampleType;
use photocraft_raster::Interrupt;

use super::textures::Stats;
use super::{Compositor, Document, Rect, Unsupported};

/// Errors encountered during GPU readback.
#[derive(Debug, thiserror::Error)]
pub enum ReadbackError<E = String> {
    #[error("unsupported document or feature: {0}")]
    Unsupported(String),
    #[error("GPU device was lost: {0}")]
    DeviceLost(String),
    #[error("buffer allocation failed: {0}")]
    AllocationFailed(String),
    #[error("buffer mapping failed: {0}")]
    MappingFailed(String),
    #[error("rendering was cancelled")]
    Cancelled,
    #[error("readback failed: {0}")]
    Execution(String),
    #[error("sink error: {0}")]
    Sink(E),
}

impl<E> From<Unsupported> for ReadbackError<E> {
    fn from(e: Unsupported) -> Self {
        ReadbackError::Unsupported(e.0)
    }
}

/// Default maximum bytes of staging buffer memory in flight during band readback (32 MiB).
pub const DEFAULT_MAX_STAGING_BYTES: usize = 32 << 20;

/// How long a readback waits for submitted GPU work before treating the device as lost: a
/// headless export has no user to cancel a wedged GPU, so it must never block forever.
pub const READBACK_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Blocking readback of a composite (native tests and tools).
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<Vec<[f32; 4]>, Unsupported> {
    render_to_vec_stats(comp, device, queue, doc, rect).map(|(v, _)| v)
}

/// [`render_to_vec`] plus the render's [`Stats`].
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_vec_stats(
    comp: &mut Compositor,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    doc: &Document,
    rect: Rect,
) -> Result<(Vec<[f32; 4]>, Stats), Unsupported> {
    let rect = rect.intersect(&doc.bounds());
    let mut staging: Vec<(Rect, wgpu::Buffer, u32)> = Vec::new();
    let mut bpp = 16u32;
    let stats = comp.render(device, queue, doc, rect, |enc, out| {
        // Bytes per pixel of the accumulation format (Rgba32Float = 16, Rgba16Float = 8).
        bpp = if out.texture.format() == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        let row = (out.rect.width() * bpp).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_readback"),
            size: (row * out.rect.height()) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(out.rect.height()) },
            },
            wgpu::Extent3d { width: out.rect.width(), height: out.rect.height(), depth_or_array_layers: 1 },
        );
        staging.push((out.rect, buf, row));
    })?;
    for (_, b, _) in &staging {
        b.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    }
    match comp.health() {
        Some(h) if !h.wait(device, None) => return Err(Unsupported(h.fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string()))),
        Some(_) => {}
        None => {
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        }
    }
    let w = rect.width() as usize;
    let mut out = vec![[0.0f32; 4]; w * rect.height() as usize];
    for (r, b, row) in &staging {
        let data = b.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU readback failed: {e:?}")))?;
        for y in 0..r.height() as usize {
            for x in 0..r.width() as usize {
                let o = y * *row as usize + x * bpp as usize;
                let px: [f32; 4] = if bpp == 16 {
                    std::array::from_fn(|i| f32::from_le_bytes([data[o + i * 4], data[o + i * 4 + 1], data[o + i * 4 + 2], data[o + i * 4 + 3]]))
                } else {
                    std::array::from_fn(|i| half::f16::from_le_bytes([data[o + i * 2], data[o + i * 2 + 1]]).to_f32())
                };
                let (dx, dy) = ((r.x0 - rect.x0) as usize + x, (r.y0 - rect.y0) as usize + y);
                out[dy * w + dx] = px;
            }
        }
    }
    Ok((out, stats))
}

/// Wait for submitted readback work in short slices, checking `ctl` between them and keeping the
/// overall `timeout`. A slice timeout means the work is still running, not that the device failed.
#[cfg(not(target_arch = "wasm32"))]
fn poll_wait_cancellable<E>(device: &wgpu::Device, timeout: std::time::Duration, ctl: &Interrupt<'_>) -> Result<(), ReadbackError<E>> {
    const SLICE: std::time::Duration = std::time::Duration::from_millis(100);
    let start = std::time::Instant::now();
    loop {
        if ctl.cancelled() {
            return Err(ReadbackError::Cancelled);
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(ReadbackError::DeviceLost(format!("GPU readback did not finish within {} s", timeout.as_secs())));
        }
        let poll = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(remaining.min(SLICE)) })
        }));
        match poll {
            Ok(Ok(status)) if status.wait_finished() => return Ok(()),
            Ok(Ok(_)) => {}
            Ok(Err(wgpu::PollError::Timeout)) => {}
            Ok(Err(e)) => return Err(ReadbackError::DeviceLost(format!("GPU device poll failed: {e}"))),
            Err(_) => return Err(ReadbackError::DeviceLost("GPU device lost during poll".into())),
        }
    }
}

/// Readback of a composite in bounded horizontal bands.
///
/// Binds staging buffer memory to at most `max_staging_bytes`, mapping and transferring
/// one band of rows at a time, so arbitrarily large documents can be exported without
/// unbounded staging allocation. `ctl` cancels the wait between bounded poll slices.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub fn render_bands<E>(
    comp: &mut Compositor,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    doc: &Document,
    rect: Rect,
    band_rows: i32,
    max_staging_bytes: usize,
    ctl: &Interrupt<'_>,
    mut sink: impl FnMut(Rect, Vec<[f32; 4]>) -> Result<(), E>,
) -> Result<(), ReadbackError<E>>
where
    E: fmt::Display,
{
    let rect = rect.intersect(&doc.bounds());
    if rect.is_empty() {
        return Ok(());
    }
    ctl.check().map_err(|_| ReadbackError::Cancelled)?;
    // Layer content is uploaded as half-precision textures on every adapter. Values above 65504
    // become infinity, and the rest lose mantissa bits: 16-bit samples near 1.0 already merge
    // before compositing, so only 8-bit output can be read back losslessly. Keep U16 and F32
    // exports on the CPU compositor, which composites and writes in 32-bit float.
    if matches!(doc.depth, SampleType::U16 | SampleType::F32) {
        return Err(ReadbackError::Unsupported("16-bit and 32-bit float documents need the CPU compositor".into()));
    }

    let bpp = if comp.acc_format() == wgpu::TextureFormat::Rgba32Float { 16u64 } else { 8u64 };
    let max_staging = if max_staging_bytes == 0 { DEFAULT_MAX_STAGING_BYTES } else { max_staging_bytes };
    if max_staging < 256 {
        return Err(ReadbackError::AllocationFailed(format!("staging budget {max_staging} bytes is smaller than device minimum row alignment (256 bytes)")));
    }

    // Account for actual aligned chunks staging across the horizontal span.
    let chunk_size = comp.chunk_size() as i32;
    if chunk_size <= 0 {
        return Err(ReadbackError::AllocationFailed("invalid compositor chunk size".into()));
    }
    let mut row_staging_bytes: u64 = 0;
    let mut x =
        rect.x0.div_euclid(chunk_size).checked_mul(chunk_size).ok_or_else(|| ReadbackError::AllocationFailed("coordinate calculation overflow".into()))?;
    while x < rect.x1 {
        let next_x = x.checked_add(chunk_size).ok_or_else(|| ReadbackError::AllocationFailed("coordinate overflow".into()))?;
        let chunk_right = next_x.min(rect.x1);
        let chunk_left = x.max(rect.x0);
        let chunk_w = chunk_right.checked_sub(chunk_left).ok_or_else(|| ReadbackError::AllocationFailed("chunk width underflow".into()))? as u64;
        let chunk_row = (chunk_w.checked_mul(bpp).ok_or_else(|| ReadbackError::AllocationFailed("dimension overflow".into()))?)
            .checked_add(255)
            .ok_or_else(|| ReadbackError::AllocationFailed("dimension overflow".into()))?
            / 256
            * 256;
        row_staging_bytes = row_staging_bytes.checked_add(chunk_row).ok_or_else(|| ReadbackError::AllocationFailed("staging bytes overflow".into()))?;
        x = next_x;
    }

    if row_staging_bytes > max_staging as u64 {
        return Err(ReadbackError::Unsupported(format!("staging budget {max_staging} bytes cannot fit a full output row ({row_staging_bytes} bytes)")));
    }

    let max_rows = (max_staging as u64 / row_staging_bytes.max(1)) as u32;
    let want_rows = if band_rows > 0 { (band_rows as u32).min(max_rows) } else { max_rows };
    let step_rows = want_rows.min(rect.height()).max(1);

    let mut y = rect.y0;
    while y < rect.y1 {
        let y_span = (rect.y1.checked_sub(y).ok_or_else(|| ReadbackError::AllocationFailed("coordinate underflow".into()))?) as u32;
        let step = step_rows.min(y_span);
        let y1 = y.checked_add(step as i32).ok_or_else(|| ReadbackError::AllocationFailed("coordinate overflow".into()))?;
        let band_rect = Rect::new(rect.x0, y, rect.x1, y1);

        if let Some(fault) = comp.fault() {
            return Err(ReadbackError::DeviceLost(fault.to_string()));
        }

        // A sink can cancel after its band is delivered: stop before staging or submitting
        // another band, instead of finding out only after the next device wait.
        ctl.check().map_err(|_| ReadbackError::Cancelled)?;

        let mut staging: Vec<(Rect, wgpu::Buffer, u32)> = Vec::new();
        let mut accumulated_staging: u64 = 0;
        let mut alloc_err: Option<ReadbackError<E>> = None;

        comp.render(device, queue, doc, band_rect, |_enc, out| {
            if alloc_err.is_some() {
                return;
            }
            let chunk_bpp = if out.texture.format() == wgpu::TextureFormat::Rgba32Float { 16u64 } else { 8u64 };
            let chunk_w = out.rect.width() as u64;
            let chunk_h = out.rect.height() as u64;
            let chunk_row = match (chunk_w.checked_mul(chunk_bpp)).and_then(|v| v.checked_add(255)) {
                Some(v) => (v / 256 * 256) as u32,
                None => {
                    alloc_err = Some(ReadbackError::AllocationFailed("row byte calculation overflow".into()));
                    return;
                }
            };
            let size = match (chunk_row as u64).checked_mul(chunk_h) {
                Some(s) => s,
                None => {
                    alloc_err = Some(ReadbackError::AllocationFailed("staging buffer size overflow".into()));
                    return;
                }
            };

            accumulated_staging = match accumulated_staging.checked_add(size) {
                Some(acc) if acc <= max_staging as u64 => acc,
                _ => {
                    alloc_err = Some(ReadbackError::AllocationFailed(format!(
                        "accumulated staging bytes ({accumulated_staging} + {size}) exceeds strict cap {max_staging}"
                    )));
                    return;
                }
            };

            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_bounded_readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            _enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(chunk_row), rows_per_image: Some(out.rect.height()) },
                },
                wgpu::Extent3d { width: out.rect.width(), height: out.rect.height(), depth_or_array_layers: 1 },
            );
            staging.push((out.rect, buf, chunk_row));
        })?;

        if let Some(err) = alloc_err {
            return Err(err);
        }

        for (_, b, _) in &staging {
            b.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }

        match comp.health() {
            Some(h) => match h.wait_timeout_cancellable(device, None, READBACK_WAIT_TIMEOUT, &|| ctl.cancelled()) {
                super::health::WaitOutcome::Finished => {}
                super::health::WaitOutcome::Cancelled => return Err(ReadbackError::Cancelled),
                super::health::WaitOutcome::TimedOut => {
                    let message =
                        h.fault().map_or_else(|| format!("GPU readback did not finish within {} s", READBACK_WAIT_TIMEOUT.as_secs()), |f| f.to_string());
                    return Err(ReadbackError::DeviceLost(message));
                }
            },
            None => {
                // No health monitor: poll in the same short slices so cancellation is not
                // blocked until the whole readback deadline elapses.
                poll_wait_cancellable(device, READBACK_WAIT_TIMEOUT, ctl)?;
            }
        }

        if let Some(fault) = comp.fault() {
            return Err(ReadbackError::DeviceLost(fault.to_string()));
        }

        let band_w = band_rect.width() as usize;
        let band_h = band_rect.height() as usize;
        let total_px = band_w.checked_mul(band_h).ok_or_else(|| ReadbackError::AllocationFailed("band pixel count overflow".into()))?;
        let mut band_pixels: Vec<[f32; 4]> = Vec::new();
        band_pixels.try_reserve_exact(total_px).map_err(|e| ReadbackError::AllocationFailed(format!("failed to allocate band pixels: {e}")))?;
        band_pixels.resize(total_px, [0.0f32; 4]);

        for (r, b, chunk_row) in &staging {
            let data = b.slice(..).get_mapped_range().map_err(|e| ReadbackError::MappingFailed(format!("GPU readback map failed: {e:?}")))?;
            let chunk_bpp = bpp as usize;
            for cy in 0..r.height() as usize {
                for cx in 0..r.width() as usize {
                    let offset = cy
                        .checked_mul(*chunk_row as usize)
                        .and_then(|o| o.checked_add(cx.checked_mul(chunk_bpp)?))
                        .ok_or_else(|| ReadbackError::Execution("offset overflow".into()))?;
                    let slice = data.get(offset..offset + chunk_bpp).ok_or_else(|| ReadbackError::Execution("mapped range out of bounds".into()))?;
                    let px: [f32; 4] = if chunk_bpp == 16 {
                        [
                            f32::from_le_bytes(slice.get(0..4).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?),
                            f32::from_le_bytes(slice.get(4..8).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?),
                            f32::from_le_bytes(
                                slice.get(8..12).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            ),
                            f32::from_le_bytes(
                                slice.get(12..16).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            ),
                        ]
                    } else {
                        [
                            half::f16::from_le_bytes(
                                slice.get(0..2).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            )
                            .to_f32(),
                            half::f16::from_le_bytes(
                                slice.get(2..4).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            )
                            .to_f32(),
                            half::f16::from_le_bytes(
                                slice.get(4..6).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            )
                            .to_f32(),
                            half::f16::from_le_bytes(
                                slice.get(6..8).and_then(|s| s.try_into().ok()).ok_or_else(|| ReadbackError::Execution("slice bounds".into()))?,
                            )
                            .to_f32(),
                        ]
                    };
                    let dx = (r.x0.checked_sub(band_rect.x0).ok_or_else(|| ReadbackError::Execution("coord underflow".into()))? as usize)
                        .checked_add(cx)
                        .ok_or_else(|| ReadbackError::Execution("coord overflow".into()))?;
                    let dy = (r.y0.checked_sub(band_rect.y0).ok_or_else(|| ReadbackError::Execution("coord underflow".into()))? as usize)
                        .checked_add(cy)
                        .ok_or_else(|| ReadbackError::Execution("coord overflow".into()))?;
                    let dst_idx =
                        dy.checked_mul(band_w).and_then(|i| i.checked_add(dx)).ok_or_else(|| ReadbackError::Execution("dst index overflow".into()))?;
                    if let Some(target) = band_pixels.get_mut(dst_idx) {
                        *target = px;
                    } else {
                        return Err(ReadbackError::Execution("destination pixel out of bounds".into()));
                    }
                }
            }
            drop(data);
            b.unmap();
        }
        drop(staging);

        sink(band_rect, band_pixels).map_err(ReadbackError::Sink)?;
        y = y1;
    }
    Ok(())
}
