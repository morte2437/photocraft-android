use super::budget::canvas_texture_bytes;
use super::resources::{DocTextures, Resources, Tile, f32_bytes};
use super::texels::texel_bytes;
use super::{Ordering, WHOLE_MIP_SIDE, wgpu};

pub(super) fn mip_count(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// The rect (x0, y0, x1, y1) of a `lw`×`lh` mip level to redraw when `prev` changed in the level
/// above: that damage halved and padded by one texel for odd-size rounding, or the whole level
/// when it is at most [`WHOLE_MIP_SIDE`] on either side. On AMD's Vulkan driver a pass that draws
/// only part of a level of 64 px or less corrupts texels outside its scissor, at the level's
/// top-left (#6418: RX 6900 XT, driver 26.8.1; DX12 on the same card is fine). Such a level costs
/// next to nothing to redraw, so this holds for every adapter; the levels after it are smaller
/// still, so the rest of the chain is redrawn whole too.
pub(super) fn mip_damage([x0, y0, x1, y1]: [u32; 4], lw: u32, lh: u32) -> [u32; 4] {
    if lw.max(lh) <= WHOLE_MIP_SIDE {
        return [0, 0, lw, lh];
    }
    [(x0 / 2).saturating_sub(1), (y0 / 2).saturating_sub(1), (x1.div_ceil(2) + 1).min(lw), (y1.div_ceil(2) + 1).min(lh)]
}

/// A buffer holding `contents`, like `DeviceExt::create_buffer_init` but without its panic when
/// the device was lost (an invalid buffer has no mapped range): the buffer is then just invalid.
pub(super) fn init_buffer(device: &wgpu::Device, label: &str, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let size = (contents.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT).max(wgpu::COPY_BUFFER_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: true });
    if let Ok(mut view) = buffer.slice(..).get_mapped_range_mut()
        && view.len() >= contents.len()
    {
        view.slice(..contents.len()).copy_from_slice(contents);
    }
    buffer.unmap();
    buffer
}

impl DocTextures {
    /// The texture set for a `size` canvas, or `None` (nothing created) when it doesn't fit
    /// the canvas budget or the GPU's memory. A set the GPU can't hold (a 3 GP canvas is 16 GB
    /// of textures, #4017) lowers the budget below it, so that canvas and larger ones are drawn
    /// on the CPU canvas instead of the out-of-memory error disabling the GPU for the session.
    pub(super) fn new(device: &wgpu::Device, res: &Resources, size: [u32; 2], tile: u32, format: wgpu::TextureFormat) -> Option<Self> {
        let bytes = canvas_texture_bytes(size, tile, texel_bytes(format));
        if bytes > res.canvas_budget.load(Ordering::Relaxed) {
            return None;
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let set = Self::create(device, res, size, tile, format);
        // Native wgpu reports these synchronously; where they arrive later (WebGPU) none is seen.
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut error = None;
        for pop in [memory.pop(), validation.pop()] {
            if let std::task::Poll::Ready(Some(e)) = std::pin::pin!(pop).poll(&mut cx) {
                error.get_or_insert(e);
            }
        }
        if let Some(e) = error {
            log::warn!("{} MB of canvas textures failed ({e}); larger canvases are drawn on the CPU", bytes >> 20);
            res.canvas_budget.fetch_min(bytes.saturating_sub(1), Ordering::Relaxed);
            return None;
        }
        Some(set)
    }

    fn create(device: &wgpu::Device, res: &Resources, size: [u32; 2], tile: u32, format: wgpu::TextureFormat) -> Self {
        let mut tiles = Vec::new();
        for ty in (0..size[1]).step_by(tile as usize) {
            for tx in (0..size[0]).step_by(tile as usize) {
                let (w, h) = (tile.min(size[0] - tx), tile.min(size[1] - ty));
                let levels_n = mip_count(w, h);
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("pc_canvas_tile"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: levels_n,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let levels = (0..levels_n)
                    .map(|l| texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() }))
                    .collect();
                let full = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let uniform = init_buffer(device, "pc_canvas_tile", &f32_bytes(&[tx as f32, ty as f32, w as f32, h as f32]), wgpu::BufferUsages::UNIFORM);
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pc_canvas_tile"),
                    layout: &res.tile_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&full) },
                        wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
                    ],
                });
                tiles.push(Tile { rect: [tx, ty, w, h], texture, levels, bind_group, _uniform: uniform });
            }
        }
        Self { size, format, tiles }
    }

    /// Write `size` pixels at document `origin` from `texels` (in the texture format, row stride
    /// `stride` pixels) into every overlapping tile's level 0.
    pub(super) fn write(&self, queue: &wgpu::Queue, origin: [u32; 2], size: [u32; 2], stride: u32, texels: &[u8]) {
        let bpp = texel_bytes(self.format);
        let (x0, y0, x1, y1) = (origin[0], origin[1], origin[0] + size[0], origin[1] + size[1]);
        for t in &self.tiles {
            let [tx, ty, tw, th] = t.rect;
            let (ix0, iy0, ix1, iy1) = (x0.max(tx), y0.max(ty), x1.min(tx + tw), y1.min(ty + th));
            if ix0 >= ix1 || iy0 >= iy1 {
                continue;
            }
            let offset = ((iy0 - y0) as u64 * stride as u64 + (ix0 - x0) as u64) * bpp;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: ix0 - tx, y: iy0 - ty, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                texels,
                wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(stride * bpp as u32), rows_per_image: Some(iy1 - iy0) },
                wgpu::Extent3d { width: ix1 - ix0, height: iy1 - iy0, depth_or_array_layers: 1 },
            );
        }
    }

    /// Rebuild the mip chain inside document rect `r` (x0, y0, x1, y1) with 2×2 box filtering.
    pub(super) fn regenerate_mips(&self, device: &wgpu::Device, queue: &wgpu::Queue, res: &Resources, r: [u32; 4]) {
        let Some(mip_pipeline) = res.mip_pipeline(self.format) else { return };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mips") });
        for t in &self.tiles {
            let [tx, ty, tw, th] = t.rect;
            let (mut x0, mut y0, mut x1, mut y1) = (r[0].max(tx), r[1].max(ty), r[2].min(tx + tw), r[3].min(ty + th));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            (x0, y0, x1, y1) = (x0 - tx, y0 - ty, x1 - tx, y1 - ty);
            for level in 1..t.levels.len() {
                let (lw, lh) = ((tw >> level).max(1), (th >> level).max(1));
                [x0, y0, x1, y1] = mip_damage([x0, y0, x1, y1], lw, lh);
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("pc_mip"),
                    layout: &res.mip_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.levels[level - 1]) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&res.mip_sampler) },
                    ],
                });
                // Keep the sampled source and render attachment in separate resources on Intel
                // DX12 adapters ([`separate_mip_targets`]): some of their drivers corrupt reduced
                // previews when both are mip levels of the same texture, even though the
                // subresources do not overlap. Elsewhere render straight into the level, which
                // spares a scratch texture and a copy per level on every edit.
                let scratch = res.separate_mip_targets.then(|| {
                    let scratch = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("pc_mip_scratch"),
                        size: wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    let view = scratch.create_view(&Default::default());
                    (scratch, view)
                });
                let (target, load) = match &scratch {
                    Some((_, view)) => (view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                    None => (&t.levels[level], wgpu::LoadOp::Load),
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pc_mip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(mip_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                pass.draw(0..3, 0..1);
                drop(pass);
                if let Some((scratch, _)) = &scratch {
                    let origin = wgpu::Origin3d { x: x0, y: y0, z: 0 };
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo { texture: scratch, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
                        wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: level as u32, origin, aspect: wgpu::TextureAspect::All },
                        wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
                    );
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
}
