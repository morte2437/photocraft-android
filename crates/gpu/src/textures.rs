use super::{Arc, DocId, FieldKind, HashMap, LayerId, PixelFormat, Rect, Role, SampleType, Tile, TileCoord, fx, timing};

/// A finished chunk: straight-alpha RGBA32F pixels of `rect` (document coordinates) at the
/// texture's origin.
pub struct ChunkOut<'a> {
    pub rect: Rect,
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
}

/// What a render did (for profiling and tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// How long the GPU spent executing a composite, once its timestamps came back. Because the
    /// readback is asynchronous this belongs to an *earlier* render than the one returning it
    /// (it carries that render's own pixel count, so the two are never confused), and it is
    /// `None` unless [`Compositor::measure_gpu_time`] is on and a measurement has landed.
    /// Always `None` from [`Compositor::encode`], which submits nothing of its own.
    pub gpu_time: Option<timing::GpuTiming>,
    pub passes: usize,
    pub chunks: usize,
    pub slots: u32,
    pub tiles_uploaded: usize,
    pub bytes_uploaded: usize,
    /// Tiles a new layer texture took from another layer's texture by a GPU copy (a duplicate
    /// shares its source's tiles) instead of uploading them.
    pub tiles_shared: usize,
    /// Effect layers whose shape was (re)rendered, fully or partially.
    pub fx_shapes: usize,
    /// Effect map programs run (one per effect whose maps were rebuilt).
    pub fx_programs: usize,
    /// Effect-map pixels written (all stages).
    pub fx_pixels: u64,
    /// Page cells the region spans (each binds only the layer pages it overlaps).
    pub cells: usize,
    /// Layer pages and paged effect maps evicted to stay within the memory budgets.
    pub evicted: usize,
    /// Intermediate submits of a huge refresh (to bound staging and evicted memory).
    pub flushes: usize,
    /// Chunks restored from an unchanged lower-stack composite.
    pub prefix_hits: usize,
    /// Planned prefix passes skipped, summed over the reused chunks.
    pub prefix_passes_skipped: usize,
    /// Plain layers blended inside another layer's pass (`fs_layerbatch`), summed over chunks.
    pub layers_batched: usize,
    /// Batched passes drawn with Normal's code alone (`fs_layerbatch_normal`), summed over chunks.
    pub normal_batches: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TexKind {
    /// RGBA8 tiles copied verbatim.
    Rgba8Direct,
    /// Other 8-bit formats converted to RGBA8.
    Rgba8,
    /// 16/32-bit formats converted to RGBA16F.
    Rgba16F,
    /// GRAY8 masks copied verbatim.
    R8Direct,
    /// 16-bit masks as R16F (the precision 16-bit pixels get as RGBA16F).
    R16F,
    /// 32-bit masks as R32F.
    R32F,
}

impl TexKind {
    pub(crate) fn for_surface(role: Role, f: PixelFormat) -> Self {
        match role {
            Role::Content | Role::Stroke if f == PixelFormat::RGBA8 => TexKind::Rgba8Direct,
            Role::Content | Role::Stroke if f.sample == SampleType::U8 => TexKind::Rgba8,
            Role::Content | Role::Stroke => TexKind::Rgba16F,
            Role::Mask if f == PixelFormat::GRAY8 => TexKind::R8Direct,
            Role::Mask if f.sample == SampleType::F32 => TexKind::R32F,
            Role::Mask => TexKind::R16F,
        }
    }
    pub(crate) fn format(self) -> wgpu::TextureFormat {
        match self {
            TexKind::Rgba8Direct | TexKind::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            TexKind::Rgba16F => wgpu::TextureFormat::Rgba16Float,
            TexKind::R8Direct => wgpu::TextureFormat::R8Unorm,
            TexKind::R16F => wgpu::TextureFormat::R16Float,
            TexKind::R32F => wgpu::TextureFormat::R32Float,
        }
    }
    pub(crate) fn bytes_per_pixel(self) -> usize {
        match self {
            TexKind::Rgba8Direct | TexKind::Rgba8 => 4,
            TexKind::Rgba16F => 8,
            TexKind::R8Direct => 1,
            TexKind::R16F => 2,
            TexKind::R32F => 4,
        }
    }
}

/// Index into the render's resident key list, and the texture's region (x, y, w, h).
pub(crate) type ResidentRef = (usize, [i32; 4]);

/// A page cell: the square `[x·page, (x+1)·page) × [y·page, (y+1)·page)` of document pixels.
pub(crate) type Cell = (i32, i32);

/// A resident layer page.
pub(crate) type ResKey = (LayerId, Role, Cell);

/// Effect maps of a layer: over its whole effect region (`None`), or one page cell of a region
/// larger than the texture limit.
pub(crate) type FxKey = (LayerId, Option<Cell>);

pub(crate) struct CachedPattern {
    pub(crate) fingerprint: u64,
    pub(crate) format: PixelFormat,
    pub(crate) default_bits: Vec<u32>,
    pub(crate) texture: Tex,
    /// The document that last painted with it: closing that document frees the texture (#4971).
    pub(crate) doc: DocId,
    /// [`Compositor::frame`] when it last painted.
    pub(crate) last_used: u64,
    /// GPU bytes of the texture (RGBA32F).
    pub(crate) bytes: u64,
}

pub(crate) struct Resident {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    /// Document-pixel rect covered (tile aligned, inside one page cell).
    pub(crate) region: Rect,
    pub(crate) kind: TexKind,
    pub(crate) format: PixelFormat,
    pub(crate) tiles: HashMap<TileCoord, Arc<Tile>>,
    pub(crate) default_nonzero: bool,
    /// Bit pattern of the surface's default pixel when the texture was filled. Tiles the surface
    /// doesn't store hold it, so a changed default (Invert, Fill) makes the texture stale.
    pub(crate) default_bits: Vec<u32>,
    pub(crate) doc: DocId,
    pub(crate) last_used: u64,
    /// Cell visit that last used it (least recently used pages are evicted first).
    pub(crate) stamp: u64,
    /// CMYK profile the texels were converted with (`CmykSpace::id`, 0 = built-in / not CMYK).
    pub(crate) cmyk: u64,
    /// Its texels were copied to or from another layer texture by the encoder, so every write
    /// to it goes through the encoder too: a queued write lands before the encoder runs, and
    /// would be overwritten by the copy, or leak into it.
    pub(crate) ordered: bool,
}

impl Resident {
    pub(crate) fn bytes(&self) -> u64 {
        self.region.width() as u64 * self.region.height() as u64 * self.kind.bytes_per_pixel() as u64
    }
}

/// A texture and its default view.
#[derive(Clone)]
pub(crate) struct Tex {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

impl Tex {
    pub(crate) fn new(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Tex { texture, view }
    }
    pub(crate) fn map(device: &wgpu::Device, label: &str, r: Rect, format: wgpu::TextureFormat) -> Self {
        Tex::new(
            device,
            label,
            r.width(),
            r.height(),
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        )
    }
    /// Upload R32F values (row-major over `r`) into this region-space texture at `r`.
    pub(crate) fn write_r32(&self, queue: &wgpu::Queue, region: Rect, r: Rect, v: &[f32]) {
        if r.is_empty() {
            return;
        }
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: (r.x0 - region.x0) as u32, y: (r.y0 - region.y0) as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(r.width() * 4), rows_per_image: Some(r.height()) },
            wgpu::Extent3d { width: r.width(), height: r.height(), depth_or_array_layers: 1 },
        );
    }
    fn bytes(&self) -> usize {
        let s = self.texture.size();
        let bpp = match self.texture.format() {
            wgpu::TextureFormat::R16Float => 2,
            wgpu::TextureFormat::Rgba32Float => 16,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => 4,
        };
        s.width as usize * s.height as usize * bpp
    }
}

/// Cached maps of one effect (one enabled item of a layer).
pub(crate) struct ProgState {
    pub(crate) key: u64,
    pub(crate) maps: Vec<Option<Tex>>,
}

/// Cached effect state of one layer.
pub(crate) struct FxEntry {
    pub(crate) doc: DocId,
    pub(crate) region: Rect,
    pub(crate) shape_key: u64,
    /// Tiles the shape was rendered from (content, mask): damage is their difference.
    pub(crate) tiles: [Option<fx::Tiles>; 2],
    /// Groups: the tiles `shape_key` hashes, by address only (see [`fx::Tiles`]).
    pub(crate) _pin: Vec<std::sync::Weak<Tile>>,
    /// The shape on the CPU (distance fields are computed from it) and on the GPU.
    pub(crate) shape_cpu: Vec<f32>,
    pub(crate) shape: Tex,
    /// Distance fields: the reach each is exact to, and its texture.
    pub(crate) fields: HashMap<FieldKind, (i32, Tex)>,
    pub(crate) progs: Vec<ProgState>,
    pub(crate) last_used: u64,
    pub(crate) stamp: u64,
}

impl FxEntry {
    pub(crate) fn bytes(&self) -> usize {
        self.shape.bytes()
            + self.shape_cpu.len() * 4
            + self.fields.values().map(|(_, t)| t.bytes()).sum::<usize>()
            + self.progs.iter().flat_map(|p| p.maps.iter().flatten()).map(Tex::bytes).sum::<usize>()
    }
}
