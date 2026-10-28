use super::batch::tile_grid;
use super::consts::{ACC_FORMAT, CHUNK, CHUNK_UNIFORM, FX_BUDGET, MAP16, MAP32, OP_UNIFORM, PAGE, PATTERN_BUDGET, RESIDENT_BUDGET};
use super::kit::{Kit, SHADER, tex_entry, uniform_entry};
use super::paging::{cell_rect, field_reaches, first_error, grid_rects};
use super::textures::{FxEntry, Resident, Tex, TexKind};
use super::{
    Compositor, DeviceHealth, DocId, Document, Fault, HashMap, Kernel, LayerContent, Plan, Rect, Role, TILE_SIZE, Unsupported, fx, plan, prefix, timing,
};

impl Compositor {
    /// Create a compositor with the default accumulation format ([`ACC_FORMAT`]).
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_format(device, ACC_FORMAT)
    }

    /// The accumulation format to use on `adapter`: `Rgba32Float` when it can be a render target,
    /// else `Rgba16Float`. Some drivers (e.g. Intel Vulkan) don't support rendering to `Rgba32Float`
    /// and would otherwise panic at render-pipeline creation.
    pub fn preferred_acc_format(adapter: &wgpu::Adapter) -> wgpu::TextureFormat {
        let feats = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba32Float);
        if feats.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) { wgpu::TextureFormat::Rgba32Float } else { wgpu::TextureFormat::Rgba16Float }
    }

    /// [`Compositor::new_with_format`], but a shader or pipeline the driver can't build (e.g. a
    /// D3D12 shader-compiler failure) is an `Err` instead of a panic, so the caller can fall back
    /// to the CPU compositor. Where errors only arrive asynchronously (WebGPU), creation is assumed
    /// to have worked.
    pub fn try_new_with_format(device: &wgpu::Device, acc_format: wgpu::TextureFormat) -> Result<Self, Unsupported> {
        match first_error(device, || Self::build(device, acc_format)) {
            ((comp, None), None) => Ok(comp),
            ((_, Some(e)), _) | (_, Some(e)) => Err(Unsupported(format!("couldn't build the compositor's pipelines: {e}"))),
        }
    }

    /// Create a compositor whose accumulation/render-target format is `acc_format`. Pass
    /// `Rgba16Float` on adapters that can't render to `Rgba32Float` (e.g. some Intel Vulkan drivers),
    /// which otherwise panics at render-pipeline creation. See `Compositor::preferred_acc_format`.
    pub fn new_with_format(device: &wgpu::Device, acc_format: wgpu::TextureFormat) -> Self {
        let (comp, error) = Self::build(device, acc_format);
        if let Some(e) = error {
            log::error!("GPU compositor: couldn't build its pipelines: {e}");
        }
        comp
    }

    /// The compositor and the first error raised building its (non-map) pipelines. They are
    /// built in parallel, each inside its own error scope on the thread that builds it (wgpu's
    /// scopes are per thread): on DX12, FXC compiles one shader at a time and the ~30 pipelines
    /// took about 8 s on one core (#6419).
    fn build(device: &wgpu::Device, acc_format: wgpu::TextureFormat) -> (Self, Option<wgpu::Error>) {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_compose"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_compose_uniforms"),
            entries: &[uniform_entry(0, CHUNK_UNIFORM), uniform_entry(1, OP_UNIFORM)],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_compose_textures"),
            entries: &(0..8).map(tex_entry).collect::<Vec<_>>(),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_compose"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
            immediate_size: 0,
        });
        // `None` only for kernels without a fragment entry point, which are never drawn.
        let pipeline = |k: Kernel, format: wgpu::TextureFormat| {
            let entry = k.entry()?;
            Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            }))
        };
        // Effect maps render to R32Float / R16Float, which some adapters (e.g. GL without float
        // render targets) can't: then layer effects use the CPU compositor and the rest stays here.
        let keys = Kernel::DRAWN.iter().filter(|k| !k.is_map()).map(|&k| (k, acc_format));
        let keys: Vec<_> = keys.chain(Kernel::DRAWN.iter().filter(|k| k.is_map()).flat_map(|&k| [(k, MAP32), (k, MAP16)])).collect();
        let built = fx::par_map(keys, |key| {
            let (built, error) = first_error(device, || pipeline(key.0, key.1));
            (key, built, error)
        });
        let (mut pipelines, mut maps) = (HashMap::new(), Vec::new());
        let (mut error, mut map_error) = (None, None);
        for (key, built, e) in built {
            let map = key.0.is_map();
            // A failed map pipeline turns all effect maps off below. A failed non-map pipeline
            // keeps its (invalid) handle: draws look pipelines up by key, so dropping it would
            // turn a validation error into a missing-key panic on the first draw.
            if let Some(e) = e {
                if map {
                    map_error.get_or_insert(e);
                    continue;
                }
                error.get_or_insert(e);
            }
            let Some(p) = built else { continue };
            if map {
                maps.push((key, p));
            } else {
                pipelines.insert(key, p);
            }
        }
        if let Some(e) = &map_error {
            log::info!("GPU compositor: no effect-map pipelines ({e}); layer effects use the CPU");
        } else {
            pipelines.extend(maps);
        }
        let dummy = Tex::new(device, "pc_compose_dummy", 1, 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING).view;
        let comp = Self {
            kit: Kit { pipelines, bgl0, bgl1, dummy },
            pool: Vec::new(),
            residents: HashMap::new(),
            fx: HashMap::new(),
            prefix: prefix::Prefix::default(),
            temps: Vec::new(),
            patterns: HashMap::new(),
            pattern_mode: photocraft_color::ColorMode::Rgb,
            pattern_doc: DocId(0),
            pattern_budget: PATTERN_BUDGET,
            max_dim: 0,
            device_max: device.limits().max_texture_dimension_2d,
            page: PAGE,
            chunk: CHUNK,
            resident_budget: RESIDENT_BUDGET,
            fx_budget: (FX_BUDGET as u64).saturating_sub(prefix::MAX_BYTES),
            layer_need: 0,
            lent: 0,
            exact_resident: false,
            focus: None,
            frame: 0,
            stamp: 0,
            staged: 0,
            recorded: 0,
            layer_alloc: None,
            admit_freed: 0,
            fail_allocs: 0,
            acc_format,
            effect_maps: map_error.is_none(),
            cmyk: 0,
            health: None,
            timer: None,
        }
        .with_texture_limit(device.limits().max_texture_dimension_2d);
        (comp, error)
    }

    fn with_texture_limit(mut self, max: u32) -> Self {
        self.set_texture_limit(max);
        self
    }

    /// Plan paging for a texture side limit of `max` (clamped to the device's own limit and at
    /// least one tile). Tests use a small limit to exercise documents larger than it on any
    /// adapter. Drops every cached texture.
    pub fn set_texture_limit(&mut self, max: u32) {
        let tile = TILE_SIZE as u32;
        self.max_dim = max.min(self.device_max).max(tile);
        let mut page = tile;
        while page * 2 <= self.max_dim.min(PAGE) {
            page *= 2;
        }
        self.page = page;
        self.chunk = CHUNK.min(page);
        self.prefix.set_step(self.chunk);
        self.pool.clear();
        self.residents.clear();
        self.fx.clear();
        self.temps.clear();
        self.prefix.clear();
    }

    /// The page side layer surfaces are stored in.
    pub fn page_size(&self) -> u32 {
        self.page
    }

    /// Test hook: the next `n` layer page textures fail as if the device were out of memory
    /// (a real one can't be forced), so the fallback paths of #6563 can be tested.
    #[doc(hidden)]
    pub fn fail_layer_allocs(&mut self, n: u32) {
        self.fail_allocs = n;
    }

    /// Set the GPU memory resident layer pages may hold (default [`RESIDENT_BUDGET`]), exactly:
    /// nothing is lent to it ([`Self::fit_layers`]) until the next [`Self::set_memory_budget`].
    pub fn set_resident_budget(&mut self, bytes: u64) {
        // The loan goes back to the effect maps, so the total budget stays what it was.
        self.fx_budget = self.fx_budget.saturating_add(self.lent);
        self.lent = 0;
        self.resident_budget = bytes;
        self.exact_resident = true;
    }

    /// Set the GPU memory the compositor may hold for layer pages and caches together
    /// (a quarter of it, at most [`FX_BUDGET`], for effect maps and lower-stack chunks). Uploads and evicted pages in
    /// flight during a refresh stay within about a quarter more.
    pub fn set_memory_budget(&mut self, bytes: u64) {
        let caches = (FX_BUDGET as u64).min(bytes / 4);
        self.prefix.set_budget((caches / 2).min(prefix::MAX_BYTES));
        self.fx_budget = caches.saturating_sub(self.prefix.budget());
        self.resident_budget = bytes.saturating_sub(caches);
        self.lent = 0;
        self.exact_resident = false;
        self.lend();
    }

    /// Record that a render needs `needed` bytes of layer pages and return the resident budget
    /// they now have. A render of the `whole` document sets the need; a part of it only raises
    /// it, so a large partial refresh never shrinks what the next full one keeps resident.
    ///
    /// Pages that miss their share by less than the effect-map share borrow from it: an evicted
    /// effect map is rebuilt by the next refresh that draws it, but one page too many sends the
    /// whole refresh to the CPU compositor, 10× slower (#5973, P60: 150 layers of 6 MP on an
    /// integrated GPU missed by 5 % and took 680 ms instead of 62).
    /// The lower-stack share is never lent: it is what keeps a mid-stack edit at half the work.
    /// The loan holds across [`Self::set_memory_budget`] until a later call needs less or more.
    pub fn fit_layers(&mut self, needed: u64, whole: bool) -> u64 {
        self.layer_need = if whole { needed } else { self.layer_need.max(needed) };
        self.lend();
        self.resident_budget
    }

    /// Return the loan to the effect maps, then lend what [`Self::layer_need`] still lacks, if
    /// the effect-map share covers it (lending part of it would fit nothing and only evict maps).
    fn lend(&mut self) {
        self.fx_budget = self.fx_budget.saturating_add(self.lent);
        self.resident_budget = self.resident_budget.saturating_sub(self.lent);
        let short = self.layer_need.saturating_sub(self.resident_budget);
        self.lent = if short <= self.fx_budget && !self.exact_resident { short } else { 0 };
        self.fx_budget = self.fx_budget.saturating_sub(self.lent);
        self.resident_budget = self.resident_budget.saturating_add(self.lent);
    }

    /// The budget of layer pages, effect maps and lower-stack chunks (bytes).
    pub fn memory_budget(&self) -> u64 {
        self.resident_budget.saturating_add(self.fx_budget).saturating_add(self.prefix.budget())
    }

    /// Bytes held by reusable lower-stack chunks, within the overall cache allowance.
    pub fn composite_cache_bytes(&self) -> u64 {
        self.prefix.bytes()
    }

    /// Cap lower-stack reuse (default at most 512 MiB); zero disables it. The released allowance
    /// goes back to effect maps, preserving the total budget and layer-page allowance.
    pub fn set_composite_cache_budget(&mut self, bytes: u64) {
        let caches = self.fx_budget.saturating_add(self.prefix.budget());
        self.prefix.set_budget(bytes.min(caches));
        self.fx_budget = caches.saturating_sub(self.prefix.budget());
    }

    /// Bytes of the layer pages a render of `region` needs resident (each layer and mask
    /// surface's tiles within the region's page cells, in their texture format).
    pub fn working_set(&self, doc: &Document, region: Rect) -> u64 {
        let Ok(plan) = plan(doc) else { return 0 };
        let grid = tile_grid(doc.bounds());
        let cells = grid_rects(region.intersect(&doc.bounds()), self.page)
            .into_iter()
            .map(|r| cell_rect((r.x0.div_euclid(self.page as i32), r.y0.div_euclid(self.page as i32)), self.page))
            .fold(Rect::EMPTY, |a, c| if a.is_empty() { c } else { a.union(&c) });
        let mut seen = std::collections::HashSet::new();
        let mut total = 0u64;
        for p in &plan.passes {
            let sources = [p.tex.as_ref().map(|t| (t.layer, t.role, t.surface.get())), p.mask.as_ref().map(|m| (m.layer, Role::Mask, m.surface.get()))];
            for (layer, role, s) in sources.into_iter().flatten() {
                if seen.insert((layer, role)) {
                    let a = s.tile_bounds().intersect(&grid).intersect(&cells);
                    total += a.width() as u64 * a.height() as u64 * TexKind::for_surface(role, s.format()).bytes_per_pixel() as u64;
                }
            }
        }
        total
    }

    /// The GPU memory resident layer pages may hold (bytes; see [`Compositor::set_memory_budget`]).
    pub fn resident_budget(&self) -> u64 {
        self.resident_budget
    }

    /// Whether a render of `region` keeps every layer page it needs within the budget; when it
    /// doesn't, each render re-uploads what was evicted.
    pub fn fits_budget(&self, doc: &Document, region: Rect) -> bool {
        self.working_set(doc, region) <= self.resident_budget
    }

    /// The document area the view shows (`None`: unknown). Full refreshes draw its cells last.
    pub fn set_focus(&mut self, focus: Option<Rect>) {
        self.focus = focus;
    }

    /// GPU memory held by resident layer pages (bytes).
    pub fn resident_bytes(&self) -> u64 {
        self.residents.values().map(Resident::bytes).sum()
    }

    /// GPU memory held by cached pattern textures (bytes).
    pub fn pattern_cache_bytes(&self) -> u64 {
        self.patterns.values().map(|p| p.bytes).sum()
    }

    /// Set the GPU memory pattern textures may hold (default [`PATTERN_BUDGET`]). It takes effect
    /// at the end of the next refresh.
    pub fn set_pattern_budget(&mut self, bytes: u64) {
        self.pattern_budget = bytes;
    }

    /// Whether `doc` can be composited on the GPU. Documents and layers larger than the texture
    /// limit are fine: surfaces are stored in page-sized textures and effect maps of regions
    /// larger than the limit are built per page cell.
    pub fn supports(&self, doc: &Document) -> Result<(), Unsupported> {
        let p = plan(doc)?;
        self.check_fx(doc, &p)
    }

    /// Whether the maps of an effect region are built per page cell (it exceeds the limit).
    pub(crate) fn fx_paged(&self, region: Rect) -> bool {
        region.width() > self.max_dim || region.height() > self.max_dim
    }

    /// How far beyond a page cell the effect maps of `f` must be computed to be exact in it.
    pub(crate) fn fx_apron(doc: &Document, f: &plan::FxLayer<'_>) -> i32 {
        let vector_shape = matches!(f.layer.content, LayerContent::Shape(_)) && photocraft_compose::effect_outline(f.layer).is_none();
        let progs: Vec<fx::MapProgram> = f
            .layer
            .effects
            .items
            .iter()
            .filter(|e| e.enabled())
            .map(|e| fx::program_with(e, &doc.global_light, vector_shape, &doc.patterns, (0.0, 0.0)))
            .collect();
        let want = field_reaches(&progs);
        // Distance fields are exact one pixel inside the window they are computed from.
        progs.iter().map(|p| p.reach(&|k| want.get(&k).copied().unwrap_or(0))).max().unwrap_or(0).saturating_add(2)
    }

    /// Effect maps must fit in textures (per page cell when the region exceeds the limit), and
    /// patterns in one texture.
    pub(crate) fn check_fx(&self, doc: &Document, p: &Plan<'_>) -> Result<(), Unsupported> {
        if !self.effect_maps && !p.fx.is_empty() {
            return Err(Unsupported("layer effects need float effect-map render targets, which this GPU lacks".into()));
        }
        for f in &p.fx {
            if self.fx_paged(f.region) {
                let side = i64::from(self.page) + 2 * i64::from(Self::fx_apron(doc, f));
                if side > i64::from(self.device_max) {
                    return Err(Unsupported(format!("effects of `{}` reach too far to build per page", f.layer.name)));
                }
            }
            for e in f.layer.effects.items.iter().filter(|e| e.enabled()) {
                // Effect noise is speckled in the CPU maps; keep CPU and GPU identical by
                // rendering noise-carrying effects on the CPU path.
                let noisy = match e {
                    photocraft_doc::Effect::DropShadow(s) | photocraft_doc::Effect::InnerShadow(s) => s.noise > 0.0,
                    photocraft_doc::Effect::OuterGlow(g) | photocraft_doc::Effect::InnerGlow(g) => g.noise > 0.0,
                    _ => false,
                };
                if noisy {
                    return Err(Unsupported(format!("effect noise on `{}` uses the CPU path", f.layer.name)));
                }
                for s in fx::program_with(e, &doc.global_light, false, &doc.patterns, (0.0, 0.0)).stages {
                    if s.lut.as_ref().is_some_and(|l| l.len() > 4096) {
                        return Err(Unsupported(format!("effect blur on `{}` too wide for the GPU path", f.layer.name)));
                    }
                }
            }
        }
        for pass in &p.passes {
            if let Some(pat) = pass.pattern
                && (pat.width > self.device_max || pat.height > self.device_max)
            {
                return Err(Unsupported(format!("pattern `{}` larger than the GPU texture limit", pat.name)));
            }
        }
        Ok(())
    }

    /// Drop GPU textures of a closed document.
    pub fn forget_doc(&mut self, doc: DocId) {
        self.residents.retain(|_, r| r.doc != doc);
        self.fx.retain(|_, e| e.doc != doc);
        self.patterns.retain(|_, p| p.doc != doc);
        self.prefix.forget(doc);
    }

    /// [`Self::forget_doc`] for every document `keep` rejects: what the compositor holds for
    /// documents that were closed, however their canvas entry went (#4529).
    pub fn retain_docs(&mut self, keep: impl Fn(DocId) -> bool) {
        let mut gone: Vec<DocId> = self
            .residents
            .values()
            .map(|r| r.doc)
            .chain(self.fx.values().map(|e| e.doc))
            .chain(self.patterns.values().map(|p| p.doc))
            .chain(self.prefix.doc())
            .filter(|d| !keep(*d))
            .collect();
        gone.sort_by_key(|d| d.0);
        gone.dedup();
        for d in gone {
            self.forget_doc(d);
        }
    }

    /// Documents the compositor holds pages, effect maps, pattern textures or a lower-stack
    /// composite for.
    pub fn held_docs(&self) -> Vec<DocId> {
        let mut docs: Vec<DocId> = self
            .residents
            .values()
            .map(|r| r.doc)
            .chain(self.fx.values().map(|e| e.doc))
            .chain(self.patterns.values().map(|p| p.doc))
            .chain(self.prefix.doc())
            .collect();
        docs.sort_by_key(|d| d.0);
        docs.dedup();
        docs
    }

    /// GPU memory held by cached effect maps (bytes).
    pub fn fx_cache_bytes(&self) -> usize {
        self.fx.values().map(FxEntry::bytes).sum()
    }

    /// Watch `health` (see [`DeviceHealth::watch`]): once the device is lost or reported an
    /// error, every render returns [`Unsupported`] without touching the GPU, so callers use the
    /// CPU compositor instead.
    pub fn set_health(&mut self, health: DeviceHealth) {
        self.health = Some(health);
    }

    /// Why the device can't be used (`None` while healthy or unwatched).
    pub fn fault(&self) -> Option<Fault> {
        self.health.as_ref().and_then(DeviceHealth::fault)
    }

    /// Measure how long the GPU spends executing each [`Compositor::render`], with timestamp
    /// queries read back asynchronously (nothing waits for the GPU). Returns whether the device
    /// supports it: without [`timing::FEATURES`] there is no measurement at all, and callers
    /// report no GPU time rather than the encode-and-submit cost `composite_ms` already is.
    ///
    /// A measurement arrives a frame or two after the refresh it belongs to, so it carries that
    /// refresh's own pixel count; take it with [`Compositor::take_gpu_timing`].
    pub fn measure_gpu_time(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        // `PHOTOCRAFT_NO_GPU_TIMING` turns it off: a kill switch for a driver whose timestamp
        // queries misbehave, and the control arm for measuring what the queries themselves
        // cost — the device is identical either way, so only the instrumentation varies.
        if std::env::var_os("PHOTOCRAFT_NO_GPU_TIMING").is_some() {
            return false;
        }
        if self.timer.is_none() {
            self.timer = timing::GpuTimer::new(device, queue);
        }
        self.timer.is_some()
    }

    /// Whether [`Compositor::measure_gpu_time`] is on for this compositor.
    pub fn measures_gpu_time(&self) -> bool {
        self.timer.is_some()
    }

    /// The most recent finished GPU measurement, leaving none behind (a stale number is never
    /// reported twice as new). `None` while unmeasured, unsupported, or still in flight.
    ///
    /// [`Compositor::render`] already drains this into [`Stats::gpu_time`], so the app never
    /// calls it; it is how a bench or a test collects a measurement without issuing another
    /// render to carry it.
    pub fn take_gpu_timing(&mut self) -> Option<timing::GpuTiming> {
        self.timer.as_mut().and_then(timing::GpuTimer::take)
    }
}
