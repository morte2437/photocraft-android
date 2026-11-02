use super::budget::{canvas_budget, physical_memory};
use super::doc_textures::init_buffer;
use super::shaders::{CANVAS_WGSL, ENCODE_WGSL, MIP_WGSL};
use super::texels::LayerFit;
use super::{FORMAT, FORMAT_HIGH, HashMap, NonZeroU64, TILE_UNIFORM_SIZE, VIEW_FLOATS, VIEW_UNIFORM_SIZE, wgpu};

pub(super) struct Resources {
    pub(super) view_bgl: wgpu::BindGroupLayout,
    pub(super) tile_bgl: wgpu::BindGroupLayout,
    pub(super) shadow_pipeline: wgpu::RenderPipeline,
    pub(super) tile_pipeline: wgpu::RenderPipeline,
    pub(super) sampler: wgpu::Sampler,
    pub(super) mip_bgl: wgpu::BindGroupLayout,
    mip_pipeline: wgpu::RenderPipeline,
    /// `mip_pipeline` and `encode_pipeline` for `Rgba16Float` tiles (when the adapter supports them).
    mip_pipeline_high: Option<wgpu::RenderPipeline>,
    encode_pipeline_high: Option<wgpu::RenderPipeline>,
    pub(super) mip_sampler: wgpu::Sampler,
    pub(super) docs: HashMap<u64, DocTextures>,
    pub(super) views: HashMap<u64, ViewGpu>,
    pub(super) out_linear: bool,
    /// Build each mip level in a scratch texture and copy it back instead of rendering into
    /// the level directly (see [`separate_mip_targets`]).
    pub(super) separate_mip_targets: bool,
    /// The wgpu layer compositor (created on first use).
    pub(super) compositor: Option<photocraft_gpu::Compositor>,
    /// Why the wgpu compositor couldn't be created (then the CPU compositor is used).
    pub(super) compositor_failed: Option<photocraft_gpu::Unsupported>,
    /// The device's health: the paint callback issues no GPU work once it's lost.
    pub(super) health: photocraft_gpu::DeviceHealth,
    /// GPU memory the compositor may hold (`None`: its default), and the document area the
    /// view shows; applied before every composite.
    pub(super) compositor_budget: Option<u64>,
    /// The layer pages the last full (or large) refresh needed and the budget they had to fit
    /// (bytes): when they don't fit, full refreshes go to the CPU ([`OVER_BUDGET`]).
    pub(super) layer_fit: Option<LayerFit>,
    pub(super) compositor_cache_budget: Option<u64>,
    pub(super) compositor_focus: Option<photocraft_geom::Rect>,
    /// GPU bytes one document's display texture set may take (see [`canvas_budget`]).
    pub(super) canvas_budget: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub(super) encode_bgl: wgpu::BindGroupLayout,
    encode_pipeline: wgpu::RenderPipeline,
    pub(super) lut_bgl: wgpu::BindGroupLayout,
    /// Display LUTs per (document, display) (monitor profile, Proof Colors / Gamut Warning);
    /// `identity_lut` otherwise.
    pub(super) luts: HashMap<(u64, u32), wgpu::BindGroup>,
    /// Signatures belong to their renderer resources, including identity transforms (mode 0).
    pub(super) display_lut_signatures: HashMap<(u64, u32), (u64, u8)>,
    pub(super) identity_lut: wgpu::BindGroup,
    /// Transparency checkerboard and gamut warning colours (Preferences › Transparency & Gamut).
    pub(super) style: CanvasStyle,
}

/// Canvas colours from Preferences › Transparency & Gamut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasStyle {
    /// Checker square side in points (0 = no checkerboard: transparency shows white).
    pub checker_square: f32,
    pub checker_light: [f32; 3],
    pub checker_dark: [f32; 3],
    pub gamut_color: [f32; 3],
    /// Gamut warning opacity 0..1.
    pub gamut_opacity: f32,
}

impl Default for CanvasStyle {
    fn default() -> Self {
        Self { checker_square: 8.0, checker_light: [1.0; 3], checker_dark: [0.8; 3], gamut_color: [0.5; 3], gamut_opacity: 1.0 }
    }
}

/// A display transform: either the existing 3D LUT or separate full-float shapers and
/// matrix. Pack curves into short rows to stay within WebGL2's texture-size limits.
pub(super) fn lut_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bgl: &wgpu::BindGroupLayout,
    size: u32,
    bytes: &[u8],
    shaper: Option<&photocraft_cms::GpuMatrixShaper>,
) -> wgpu::BindGroup {
    let extent = wgpu::Extent3d { width: size, height: size, depth_or_array_layers: size };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_display_lut"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: Some(size) },
        extent,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let (width, height, curves, matrix) = if let Some(s) = shaper {
        let width = 1024usize;
        let count = s.input_table().len() + s.output_table().len();
        let height = count.div_ceil(width);
        let mut curves = Vec::with_capacity(width * height * 16);
        for value in s.input_table().iter().chain(s.output_table()).flatten() {
            curves.extend_from_slice(&value.to_le_bytes());
        }
        curves.resize(width * height * 16, 0);
        (width as u32, height as u32, curves, s.matrix_rows())
    } else {
        (1, 1, vec![0; 16], [[0.0; 4]; 3])
    };
    let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let curves_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pc_display_shapers"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // textureLoad + explicit interpolation needs no optional float-filtering feature.
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &curves_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &curves,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 16), rows_per_image: Some(height) },
        extent,
    );
    let curves_view = curves_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let matrix_bytes: Vec<u8> = matrix.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let matrix_buffer = init_buffer(device, "pc_display_matrix", &matrix_bytes, wgpu::BufferUsages::UNIFORM);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pc_display_lut"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&curves_view) },
            wgpu::BindGroupEntry { binding: 2, resource: matrix_buffer.as_entire_binding() },
        ],
    })
}

/// The 2³ identity LUT (unused unless a document has none and `display` is set).
pub(super) fn identity_lut_bytes() -> Vec<u8> {
    (0..8).flat_map(|i| [if i & 1 != 0 { 255 } else { 0 }, if i & 2 != 0 { 255 } else { 0 }, if i & 4 != 0 { 255 } else { 0 }, 0]).collect()
}

pub(super) struct ViewGpu {
    pub(super) buffer: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) frame_data: [f32; VIEW_FLOATS],
}

pub(super) struct DocTextures {
    pub(super) size: [u32; 2],
    pub(super) format: wgpu::TextureFormat,
    pub(super) tiles: Vec<Tile>,
}

pub(super) struct Tile {
    /// Document-pixel rect: x, y, w, h.
    pub(super) rect: [u32; 4],
    pub(super) texture: wgpu::Texture,
    pub(super) levels: Vec<wgpu::TextureView>,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) _uniform: wgpu::Buffer,
}

fn uniform_entry(binding: u32, size: u64, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(size) },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub(super) fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    (vs, fs): (&str, &str),
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState { module, entry_point: Some(vs), buffers: &[], compilation_options: Default::default() },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Resources {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue, target: wgpu::TextureFormat, high: bool) -> Self {
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_canvas"), source: wgpu::ShaderSource::Wgsl(CANVAS_WGSL.into()) });
        let vis = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let view_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_canvas_view"),
            entries: &[uniform_entry(0, VIEW_UNIFORM_SIZE, vis), sampler_entry(1)],
        });
        let tile_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_canvas_tile"),
            entries: &[texture_entry(0), uniform_entry(1, TILE_UNIFORM_SIZE, vis)],
        });
        let lut_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let curves_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let lut_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_display_lut"),
            entries: &[lut_entry, curves_entry, uniform_entry(2, 48, wgpu::ShaderStages::FRAGMENT)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_canvas"),
            bind_group_layouts: &[Some(&view_bgl), Some(&tile_bgl), Some(&lut_bgl)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_canvas_shadow"),
            bind_group_layouts: &[Some(&view_bgl)],
            immediate_size: 0,
        });
        // Premultiplied "over", with egui's alpha rule (keeps a transparent window's alpha sane).
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let shadow_pipeline = pipeline(device, "pc_canvas_shadow", &shadow_layout, &module, ("vs_shadow", "fs_shadow"), target, Some(blend));
        let tile_pipeline = pipeline(device, "pc_canvas_tile", &layout, &module, ("vs_tile", "fs_tile"), target, Some(blend));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pc_canvas_trilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_mip"), source: wgpu::ShaderSource::Wgsl(MIP_WGSL.into()) });
        let mip_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pc_mip"), entries: &[texture_entry(0), sampler_entry(1)] });
        let mip_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pc_mip"), bind_group_layouts: &[Some(&mip_bgl)], immediate_size: 0 });
        let mip_pipeline = pipeline(device, "pc_mip", &mip_layout, &mip_module, ("vs", "fs"), FORMAT, None);
        let mip_pipeline_high = high.then(|| pipeline(device, "pc_mip_16f", &mip_layout, &mip_module, ("vs", "fs"), FORMAT_HIGH, None));
        let mip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pc_mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Compositor output (straight RGBA32F chunk) -> premultiplied RGBA8 display tile.
        let encode_module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_encode"), source: wgpu::ShaderSource::Wgsl(ENCODE_WGSL.into()) });
        let unfilterable = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let encode_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_encode"),
            entries: &[unfilterable, uniform_entry(1, 16, wgpu::ShaderStages::FRAGMENT)],
        });
        let encode_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_encode"),
            bind_group_layouts: &[Some(&encode_bgl)],
            immediate_size: 0,
        });
        let encode_pipeline = pipeline(device, "pc_encode", &encode_layout, &encode_module, ("vs", "fs"), FORMAT, None);
        let encode_pipeline_high = high.then(|| pipeline(device, "pc_encode_16f", &encode_layout, &encode_module, ("vs", "fs"), FORMAT_HIGH, None));
        let identity_lut = lut_bind_group(device, queue, &lut_bgl, 2, &identity_lut_bytes(), None);
        Self {
            lut_bgl,
            luts: HashMap::new(),
            display_lut_signatures: HashMap::new(),
            identity_lut,
            style: CanvasStyle::default(),
            view_bgl,
            tile_bgl,
            shadow_pipeline,
            tile_pipeline,
            sampler,
            mip_bgl,
            mip_pipeline,
            mip_pipeline_high,
            encode_pipeline_high,
            mip_sampler,
            docs: HashMap::new(),
            views: HashMap::new(),
            out_linear: target.is_srgb(),
            separate_mip_targets: false,
            compositor: None,
            compositor_failed: None,
            health: photocraft_gpu::DeviceHealth::new(),
            compositor_budget: None,
            layer_fit: None,
            compositor_cache_budget: None,
            compositor_focus: None,
            canvas_budget: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(canvas_budget(physical_memory(), None))),
            encode_bgl,
            encode_pipeline,
        }
    }

    /// The pipeline encoding compositor output into a `format` tile.
    pub(super) fn encode_pipeline(&self, format: wgpu::TextureFormat) -> Option<&wgpu::RenderPipeline> {
        if format == FORMAT_HIGH { self.encode_pipeline_high.as_ref() } else { Some(&self.encode_pipeline) }
    }

    /// The mip-chain pipeline for `format` tiles.
    pub(super) fn mip_pipeline(&self, format: wgpu::TextureFormat) -> Option<&wgpu::RenderPipeline> {
        if format == FORMAT_HIGH { self.mip_pipeline_high.as_ref() } else { Some(&self.mip_pipeline) }
    }
}
