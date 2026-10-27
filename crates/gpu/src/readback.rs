use super::textures::Stats;
use super::*;

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
    match &comp.health {
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
