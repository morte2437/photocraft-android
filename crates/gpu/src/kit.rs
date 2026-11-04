use super::consts::{CHUNK_UNIFORM, MAP16, MAP32, OP_UNIFORM, STRIDE};
use super::convert::{lut_texture, words};
use super::textures::{ResKey, ResidentRef};
use super::{Arc, HashMap, I, Kernel, NonZeroU64, Rect};

/// Pipelines and layouts, shared by cheap clones (wgpu handles are reference counted).
#[derive(Clone)]
pub(crate) struct Kit {
    pub(crate) pipelines: HashMap<(Kernel, wgpu::TextureFormat), wgpu::RenderPipeline>,
    pub(crate) bgl0: wgpu::BindGroupLayout,
    pub(crate) bgl1: wgpu::BindGroupLayout,
    pub(crate) dummy: wgpu::TextureView,
}

/// One draw of an effect-map kernel (outside the chunk loop).
pub(crate) struct MapDraw {
    pub(crate) kernel: Kernel,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) target: wgpu::TextureView,
    /// Target-local scissor (x, y, w, h).
    pub(crate) scissor: [u32; 4],
    /// Chunk record: origin and size of the target's space.
    pub(crate) space: [i32; 4],
    pub(crate) op: Vec<u8>,
    pub(crate) views: [Option<wgpu::TextureView>; 8],
    pub(crate) lut: Option<Arc<Vec<f32>>>,
}

impl Kit {
    /// Record `draws` (each its own render pass) into `encoder`.
    pub(crate) fn run(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, draws: &[MapDraw]) {
        if draws.is_empty() {
            return;
        }
        let mut data = vec![0u8; draws.len() * 2 * STRIDE as usize];
        for (i, d) in draws.iter().enumerate() {
            let w = words(&[I(d.space[0]), I(d.space[1]), I(d.space[2]), I(d.space[3])]);
            data[2 * i * STRIDE as usize..][..w.len()].copy_from_slice(&w);
            data[(2 * i + 1) * STRIDE as usize..][..d.op.len()].copy_from_slice(&d.op);
        }
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_fx_uniforms"),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubuf, 0, &data);
        let bg0 = uniform_group(device, &self.bgl0, &ubuf);
        for (i, d) in draws.iter().enumerate() {
            let lut = d.lut.as_ref().map(|l| {
                let mut row = [0.0f32; 4096];
                row[..l.len().min(4096)].copy_from_slice(&l[..l.len().min(4096)]);
                lut_texture(device, queue, &[row])
            });
            let mut views: Vec<&wgpu::TextureView> = d.views.iter().map(|v| v.as_ref().unwrap_or(&self.dummy)).collect();
            if let Some((_, lv)) = &lut {
                views[4] = lv;
            }
            let bg1 = texture_group(device, &self.bgl1, &views);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pc_fx_map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &d.target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines[&(d.kernel, d.format)]);
            pass.set_scissor_rect(d.scissor[0], d.scissor[1], d.scissor[2], d.scissor[3]);
            pass.set_bind_group(0, &bg0, &[(2 * i as u64 * STRIDE) as u32, ((2 * i + 1) as u64 * STRIDE) as u32]);
            pass.set_bind_group(1, &bg1, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

pub(crate) fn uniform_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, ubuf: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_compose_uniforms"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: ubuf, offset: 0, size: NonZeroU64::new(CHUNK_UNIFORM) }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: ubuf, offset: 0, size: NonZeroU64::new(OP_UNIFORM) }),
            },
        ],
    })
}

pub(crate) fn texture_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, views: &[&wgpu::TextureView]) -> wgpu::BindGroup {
    let e: Vec<wgpu::BindGroupEntry> =
        views.iter().enumerate().map(|(b, v)| wgpu::BindGroupEntry { binding: b as u32, resource: wgpu::BindingResource::TextureView(v) }).collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pc_compose_pass"), layout, entries: &e })
}

pub(crate) fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

pub(crate) fn uniform_entry(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: NonZeroU64::new(size) },
        count: None,
    }
}

/// The compositor's WGSL source (exposed for validation in tests).
pub const SHADER: &str = include_str!("compose.wgsl");

/// One thread's share of the compositor's pipelines: the vertex entry point (in
/// [`vertex_shader`]) and the (kernel, target format) pairs it builds.
pub(crate) struct BuildJob {
    pub(crate) vertex: String,
    pub(crate) keys: Vec<(Kernel, wgpu::TextureFormat)>,
}

/// The compositor's pipelines, grouped by the thread that builds them: one job per kernel, and
/// an effect-map kernel's R32F and R16F pipelines in the same job. Each job has its own vertex
/// entry point, so no two jobs share a shader (see `Compositor::build`).
pub(crate) fn build_jobs(acc_format: wgpu::TextureFormat) -> Vec<BuildJob> {
    let one = |k: Kernel| if k.is_map() { vec![(k, MAP32), (k, MAP16)] } else { vec![(k, acc_format)] };
    let (plain, maps): (Vec<Kernel>, Vec<Kernel>) = Kernel::DRAWN.iter().partition(|k| !k.is_map());
    plain.into_iter().chain(maps).enumerate().map(|(job, k)| BuildJob { vertex: format!("vs_{job}"), keys: one(k) }).collect()
}

/// The full-chunk triangle's vertex stage, once per build job under that job's name (`vs_0`,
/// `vs_1`, ...): the same code under distinct names, so each job compiles its own vertex shader.
///
/// A workaround, not a design: wgpu-hal's D3D12 shader cache shares a compiled FXC blob between
/// threads, and FXC's blobs don't count references atomically, so a shared one can be freed
/// while still cached (gfx-rs/wgpu#10547, #9173). Each copy costs one compile of a three-line
/// shader. Once a wgpu release fixes the cache, go back to one `vs` in `compose.wgsl` and let
/// `build_jobs` split map kernels by format again.
pub(crate) fn vertex_shader(jobs: &[BuildJob]) -> String {
    let mut s = String::from(
        "struct VOut { @builtin(position) pos: vec4<f32> };
",
    );
    for job in jobs {
        s.push_str(&format!(
            "@vertex fn {}(@builtin(vertex_index) vi: u32) -> VOut {{
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}}
",
            job.vertex
        ));
    }
    s
}

/// Pass-level resources resolved for one page cell: resident pages (texture, mask) and effect
/// maps (view and region) per pass.
pub(crate) struct Bound {
    pub(crate) views: Vec<(Option<ResidentRef>, Option<ResidentRef>)>,
    pub(crate) keys: Vec<ResKey>,
    pub(crate) maps: Vec<Option<(wgpu::TextureView, Rect)>>,
}

/// Pass-level resources shared by every cell of a frame.
pub(crate) struct FrameRes {
    /// Tile area of each pass's (texture, mask) surface within the canvas tile grid.
    pub(crate) areas: Vec<(Rect, Rect)>,
    pub(crate) luts: Vec<Option<(wgpu::Texture, wgpu::TextureView)>>,
    pub(crate) patterns: Vec<Option<wgpu::TextureView>>,
}
