//! Camera Raw's per-pixel develop on the GPU, for the dialog's interactive preview (#6916).
//!
//! One fragment pass (`camera_raw.wgsl`) runs the stages that read only their own pixel: white
//! balance and exposure, Contrast without Texture or Clarity, Whites and Blacks, Vibrance and
//! Saturation, the tone curves, the Color Mixer, Color Grading, the vignette and grain. Their
//! constants and tables come from [`photocraft_algo::camera_raw::per_pixel`], which the CPU
//! develop builds its own from, so both evaluate the same maths; the curves and the Whites and
//! Blacks rows are rows of one `R32Float` texture. Shadows and Highlights run before it as the
//! passes of `camera_raw_tone.rs`, which then also measure the image's points for Whites and
//! Blacks; this pass applies their change. Dehaze runs before them (its filters on the copy of
//! [`Dehaze::cells`], then `fs_dehaze`, which the later passes read instead of the source),
//! Clarity and Texture after them (`fs_luma`, then their filters, applied in `tonal`). Settings with Detail have no
//! [`PerPixel`] and stay on the CPU.
//!
//! The pipelines are built on demand ([`CameraRawDevelop::new`]), not with the compositor
//! (#6419). They render into `Rgba32Float`, `R32Float` and `Rgba32Uint`;
//! [`CameraRawDevelop::supported`] says whether the adapter can.

use photocraft_algo::camera_raw::local_tone::LOG_FLOOR;
use photocraft_algo::camera_raw::spatial::{DEHAZE_T_FLOOR, TEXTURE_LIMIT};
use photocraft_algo::camera_raw::{CURVE_LUT, Dehaze, PerPixel, VignetteStyle, WB_STEPS};

use crate::Unsupported;
use crate::camera_raw_tone::{Segment, ToneFrame, TonePipelines, WB_ROW};

/// The pass's WGSL source.
pub const CAMERA_RAW_SHADER: &str = include_str!("camera_raw.wgsl");

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// `Params` in the shader: 23 rows of 16 bytes.
const PARAMS_SIZE: u64 = 23 * 16;
/// Rows of the table texture: parametric, master, red, green, blue curves; Whites, Blacks.
const LUT_ROWS: u32 = 7;
const LUT_WIDTH: u32 = CURVE_LUT as u32;
const BYTES_PER_PIXEL: u32 = 16;

struct Image {
    key: Option<u64>,
    size: [u32; 2],
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Image {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2], usage: wgpu::TextureUsages) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Image { key: None, size, texture, view }
    }
}

/// The Camera Raw preview pass and its textures (reused while the size stays).
pub struct CameraRawDevelop {
    pipeline: wgpu::RenderPipeline,
    /// `fs_log` and `fs_points`: Shadows and Highlights' input, Whites and Blacks' cells after it.
    log: wgpu::RenderPipeline,
    points: wgpu::RenderPipeline,
    /// `fs_dehaze` and `fs_luma`: Dehaze's result, Clarity and Texture's input.
    dehaze: wgpu::RenderPipeline,
    luma: wgpu::RenderPipeline,
    tone: TonePipelines,
    tone_frame: Option<ToneFrame>,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    luts: wgpu::Texture,
    luts_view: wgpu::TextureView,
    source: Option<Image>,
    target: Option<Image>,
    #[cfg(not(target_arch = "wasm32"))]
    readback: Option<wgpu::Buffer>,
}

impl CameraRawDevelop {
    /// Whether `adapter` can render the passes' `Rgba32Float`, `R32Float` and `Rgba32Uint` targets.
    pub fn supported(adapter: &wgpu::Adapter) -> bool {
        [FORMAT, wgpu::TextureFormat::R32Float, wgpu::TextureFormat::Rgba32Uint]
            .iter()
            .all(|f| adapter.get_texture_format_features(*f).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT))
    }

    /// Builds the pipeline. A shader or pipeline the driver can't build is an `Err`, so the
    /// caller keeps the CPU develop (where errors only arrive asynchronously, on WebGPU, creation
    /// is assumed to have worked).
    pub fn new(device: &wgpu::Device) -> Result<Self, Unsupported> {
        match crate::first_error(device, || Self::build(device)) {
            (dev, None) => Ok(dev),
            (_, Some(e)) => Err(Unsupported(format!("couldn't build the RAW Develop pipeline: {e}"))),
        }
    }

    fn build(device: &wgpu::Device) -> Self {
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pc_camera_raw"), source: wgpu::ShaderSource::Wgsl(CAMERA_RAW_SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pc_camera_raw"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                crate::tex_entry(1),
                crate::tex_entry(2),
                crate::tex_entry(3),
                crate::tex_entry(4),
                crate::camera_raw_tone::uint_entry(5),
                crate::tex_entry(6),
                crate::tex_entry(7),
                crate::tex_entry(8),
                crate::tex_entry(9),
                crate::tex_entry(10),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pc_camera_raw"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline_for = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
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
            })
        };
        let pipeline = pipeline_for("fs", FORMAT);
        let log = pipeline_for("fs_log", wgpu::TextureFormat::R32Float);
        let points = pipeline_for("fs_points", FORMAT);
        let dehaze = pipeline_for("fs_dehaze", FORMAT);
        let luma = pipeline_for("fs_luma", wgpu::TextureFormat::R32Float);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pc_camera_raw_params"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let luts = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pc_camera_raw_luts"),
            size: wgpu::Extent3d { width: LUT_WIDTH, height: LUT_ROWS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            // Whites and Blacks' rows are rendered in when Shadows and Highlights measure the points.
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let luts_view = luts.create_view(&Default::default());
        CameraRawDevelop {
            pipeline,
            log,
            points,
            dehaze,
            luma,
            tone: TonePipelines::build(device),
            tone_frame: None,
            layout,
            params,
            luts,
            luts_view,
            source: None,
            target: None,
            #[cfg(not(target_arch = "wasm32"))]
            readback: None,
        }
    }

    /// Records the develop of `source` (`w × h` straight RGBA, display-encoded) with `stages`
    /// into `encoder` and returns the `Rgba32Float` texture it renders. The source is uploaded
    /// unless the last one had the same `key` and size (`None` always uploads).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<&wgpu::Texture, Unsupported> {
        check_size(device, source, size)?;
        let [w, h] = size;
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let src = match self.source.take() {
            Some(s) if s.size == size => s,
            _ => Image::new(device, "pc_camera_raw_source", size, usage),
        };
        let src = self.source.insert(src);
        if key.is_none() || src.key != key {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &src.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(source),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * BYTES_PER_PIXEL), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            src.key = key;
        }
        let clarity = stages.clarity.is_some();
        let texture = stages.texture.as_ref().is_some_and(|t| t.radii.iter().any(|r| *r > 0));
        let filtered = stages.local_tone.is_some() || stages.dehaze.is_some() || clarity || texture || stages.measure_whites_blacks.is_some();
        let frame = if filtered {
            let frame = match self.tone_frame.take() {
                Some(f) if f.size() == size => f,
                _ => ToneFrame::new(device, size),
            };
            let frame = self.tone_frame.insert(frame);
            frame.prepare(device, queue, &self.tone, stages, &self.luts_view)?;
            if let Some(d) = &stages.dehaze {
                // The copy comes from the CPU's own code: which of its brightest cells tie decides
                // the atmospheric light, and stage 1 on the GPU differs by a few ulps.
                let gains = stages.gains.map(|g| g.map(f32::to_bits));
                frame.upload_cells(queue, key.map(|k| (k, gains)), || Dehaze::cells(d, source, w as usize, h as usize, stages.gains))?;
            }
            self.tone.write(queue, stages.local_tone.as_ref(), stages.measure_whites_blacks);
            Some(&*frame)
        } else {
            None
        };
        let slots = stages.local_tone.as_ref().map_or([0, 0], ToneFrame::last_slots);
        let tone = [slots[0], slots[1], frame.map_or(1, |f| f.points().2), 0];
        queue.write_buffer(&self.params, 0, &params(size, stages, tone, texture));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.luts, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&tables(stages)),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(LUT_WIDTH * 4), rows_per_image: Some(LUT_ROWS) },
            wgpu::Extent3d { width: LUT_WIDTH, height: LUT_ROWS, depth_or_array_layers: 1 },
        );
        let target = match self.target.take() {
            Some(t) if t.size == size => t,
            _ => Image::new(device, "pc_camera_raw_target", size, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC),
        };
        let target = self.target.insert(target);
        let (dummy, dummy_uint) = (self.tone.dummy(), self.tone.dummy_uint());
        let mut all = Bound { log_in: dummy, log_out: dummy, stats: dummy_uint, base: dummy, haze: dummy, air: dummy, clarity: dummy, texture: dummy };
        if let Some(f) = frame {
            all.stats = f.stats();
            if let Some((log_in, log_out)) = f.log() {
                (all.log_in, all.log_out) = (log_in, log_out);
            }
            if let Some([t, air, dehazed]) = f.haze() {
                (all.haze, all.air, all.base) = (t, air, dehazed);
            }
            if let Some((_, c, t)) = f.detail() {
                all.clarity = c.unwrap_or(dummy);
                all.texture = t.unwrap_or(dummy);
            }
        }
        let group = |b: &Bound| {
            let entries = [
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&src.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.luts_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(b.log_in) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(b.log_out) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(b.stats) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(b.base) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(b.haze) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(b.air) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(b.clarity) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(b.texture) },
            ];
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pc_camera_raw"), layout: &self.layout, entries: &entries })
        };
        // A pass never binds the texture it renders: those are dummies in its group.
        let main = group(&all);
        if let Some(f) = frame {
            let missing = || Unsupported("a RAW Develop stage's textures are missing".into());
            if stages.dehaze.is_some() {
                let [_, _, dehazed] = f.haze().ok_or_else(missing)?;
                f.record(encoder, &self.tone, Segment::Haze);
                pass(encoder, &self.dehaze, &group(&Bound { base: dummy, log_in: dummy, log_out: dummy, ..all }), dehazed);
            }
            if stages.local_tone.is_some() {
                // The input's log luminance, then the filter on it.
                let (log_in, _) = f.log().ok_or_else(missing)?;
                pass(encoder, &self.log, &group(&Bound { log_in: dummy, log_out: dummy, ..all }), log_in);
                f.record(encoder, &self.tone, Segment::Tone);
            }
            if clarity || texture {
                let (lc, _, _) = f.detail().ok_or_else(missing)?;
                pass(encoder, &self.luma, &main, lc);
                f.record(encoder, &self.tone, Segment::Detail);
            }
            if stages.measure_whites_blacks.is_some() {
                pass(encoder, &self.points, &main, f.points().0);
                f.record(encoder, &self.tone, Segment::WhitesBlacks);
            }
        }
        pass(encoder, &self.pipeline, &main, &target.view);
        Ok(&target.texture)
    }

    /// Develops `source` and reads the result back (blocking; native only, the web has no
    /// blocking wait). GPU errors and a lost device are `Err`; `health`, when the device is
    /// watched, records a loss.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn develop(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<Vec<[f32; 4]>, Unsupported> {
        if let Some(f) = health.and_then(crate::DeviceHealth::fault) {
            return Err(Unsupported(f.to_string()));
        }
        let (result, error) = crate::first_error(device, || self.develop_inner(device, queue, health, source, key, size, stages));
        match error {
            Some(e) => {
                // A half-done upload must not be taken for the cached source next time.
                self.source = None;
                if let Some(f) = &mut self.tone_frame {
                    f.forget_cells();
                }
                Err(Unsupported(format!("GPU RAW Develop failed: {e}")))
            }
            None => result,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    fn develop_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Option<&crate::DeviceHealth>,
        source: &[[f32; 4]],
        key: Option<u64>,
        size: [u32; 2],
        stages: &PerPixel,
    ) -> Result<Vec<[f32; 4]>, Unsupported> {
        check_size(device, source, size)?;
        let [w, h] = size;
        let row = (w * BYTES_PER_PIXEL).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes = u64::from(row) * u64::from(h);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_camera_raw") });
        self.encode(device, queue, &mut encoder, source, key, size, stages)?;
        let Some(target) = self.target.as_ref().map(|t| &t.texture) else { return Err(Unsupported("no RAW Develop target".into())) };
        let buffer = match self.readback.take() {
            Some(b) if b.size() == bytes => b,
            _ => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_camera_raw_readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let index = queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        match health {
            Some(hl) => {
                if !hl.wait(device, Some(index)) {
                    return Err(Unsupported(hl.fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string())));
                }
            }
            None => {
                device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).map_err(|e| Unsupported(format!("GPU wait failed: {e}")))?;
            }
        }
        let mut out = Vec::with_capacity(source.len());
        {
            let data = buffer.slice(..).get_mapped_range().map_err(|e| Unsupported(format!("GPU readback failed: {e:?}")))?;
            let width = w as usize * BYTES_PER_PIXEL as usize;
            for y in 0..h as usize {
                let start = y * row as usize;
                let line = data.get(start..start + width).ok_or_else(|| Unsupported("short GPU readback".into()))?;
                out.extend(bytemuck::pod_collect_to_vec::<u8, [f32; 4]>(line));
            }
        }
        buffer.unmap();
        self.readback = Some(buffer);
        Ok(out)
    }
}

/// The textures a pass of `camera_raw.wgsl` reads besides the source and tables (dummies where
/// unused).
#[derive(Clone, Copy)]
struct Bound<'a> {
    log_in: &'a wgpu::TextureView,
    log_out: &'a wgpu::TextureView,
    stats: &'a wgpu::TextureView,
    /// Stages 1 and 2 when Dehaze runs.
    base: &'a wgpu::TextureView,
    /// Dehaze's transmission and atmospheric light.
    haze: &'a wgpu::TextureView,
    air: &'a wgpu::TextureView,
    /// Clarity's coefficients and Texture's blur.
    clarity: &'a wgpu::TextureView,
    texture: &'a wgpu::TextureView,
}

/// One full-frame pass of `pipeline` into `target`.
fn pass(encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, target: &wgpu::TextureView) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pc_camera_raw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

fn check_size(device: &wgpu::Device, source: &[[f32; 4]], [w, h]: [u32; 2]) -> Result<(), Unsupported> {
    let max = device.limits().max_texture_dimension_2d;
    if w == 0 || h == 0 || w > max || h > max {
        return Err(Unsupported(format!("a {w}×{h} RAW Develop preview doesn't fit the GPU's {max} px textures")));
    }
    if source.len() as u64 != u64::from(w) * u64::from(h) {
        return Err(Unsupported(format!("{} RAW Develop preview pixels for {w}×{h}", source.len())));
    }
    Ok(())
}

/// Flags of `Params.flags`, in the shader's bit order; `texture` is whether Texture's blur ran.
fn flags(s: &PerPixel, texture: bool) -> u32 {
    let wb = s.whites_blacks.as_ref();
    // Measured after Shadows and Highlights, the amounts keep the sliders' signs.
    let measured = s.measure_whites_blacks;
    let on = [
        s.gains.is_some(),
        // The luminance stage: Contrast, Clarity, Texture.
        s.contrast.is_some() || s.clarity.is_some() || s.texture.is_some(),
        wb.is_some_and(|w| w.whites.is_some()) || measured.is_some_and(|[w, _]| w != 0.0),
        wb.is_some_and(|w| w.blacks.is_some()) || measured.is_some_and(|[_, b]| b != 0.0),
        wb.is_some_and(|w| w.per_channel) || measured.is_some_and(|[w, b]| w > 0.0 || b < 0.0),
        s.presence.is_some(),
        s.parametric.is_some(),
        s.point[0].is_some(),
        s.point[1].is_some(),
        s.point[2].is_some(),
        s.point[3].is_some(),
        s.hsl.is_some(),
        s.grade.is_some(),
        s.vignette.is_some(),
        s.grain.is_some(),
        s.clamp,
        s.local_tone.is_some(),
        s.dehaze.is_some(),
        s.clarity.is_some(),
        texture,
    ];
    on.iter().enumerate().fold(0, |f, (bit, on)| if *on { f | 1 << bit } else { f })
}

/// The shader's `Params` for a `size` image; `tone` is `Params.tone`, `texture` whether Texture's
/// blur ran.
fn params(size: [u32; 2], s: &PerPixel, tone: [u32; 4], texture: bool) -> Vec<u8> {
    let g = s.gains.unwrap_or([1.0; 3]);
    let presence = s.presence.unwrap_or_default();
    let hsl = s.hsl.unwrap_or_default();
    let grade = s.grade.as_ref();
    let vig = s.vignette.as_ref();
    let grain = s.grain.as_ref();
    let mut out: Vec<u8> = Vec::with_capacity(PARAMS_SIZE as usize);
    out.extend([size[0], size[1], flags(s, texture)].iter().flat_map(|v| v.to_le_bytes()).chain(s.encoding.to_le_bytes()));
    let mut f = |v: [f32; 4]| out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    f([g[0], g[1], g[2], s.contrast.unwrap_or(0.0)]);
    f([presence[0], presence[1], 0.0, 0.0]);
    for row in hsl {
        f([row[0], row[1], row[2], row[3]]);
        f([row[4], row[5], row[6], row[7]]);
    }
    for k in 0..4 {
        let (t, l) = grade.map_or(([0.0; 3], 0.0), |gr| (gr.tints[k], gr.lums[k]));
        f([t[0], t[1], t[2], l]);
    }
    f(grade.map_or([0.0; 4], |gr| [gr.blend, gr.balance, 0.0, 0.0]));
    let style = |v: VignetteStyle| match v {
        VignetteStyle::HighlightPriority => 0.0,
        VignetteStyle::ColorPriority => 1.0,
        VignetteStyle::PaintOverlay => 2.0,
    };
    f(vig.map_or([0.0, 0.0, 0.0, 2.0], |v| [v.amount, v.highlights, style(v.style), v.power]));
    f(vig.map_or([0.0, 0.0, 1.0, 1.0], |v| [v.centre[0], v.centre[1], v.axes[0], v.axes[1]]));
    f(vig.map_or([0.0; 4], |v| [v.edges[0], v.edges[1], 0.0, 0.0]));
    f(grain.map_or([1.0, 0.0, 0.0, 1.0], |gr| [gr.cell, gr.roughness, gr.amount, gr.scale]));
    let seed = grain.map_or(0, |gr| gr.seed);
    let fine = seed.wrapping_add(7);
    out.extend([seed as u32, (seed >> 32) as u32, fine as u32, (fine >> 32) as u32].iter().flat_map(|v| v.to_le_bytes()));
    out.extend(tone.iter().flat_map(|v| v.to_le_bytes()));
    let amount = |v: Option<f32>| v.unwrap_or(0.0);
    let rows = [
        LOG_FLOOR,
        0.0,
        0.0,
        0.0,
        amount(s.dehaze.as_ref().map(|d| d.amount)),
        amount(s.clarity.as_ref().map(|c| c.amount)),
        amount(s.texture.as_ref().map(|t| t.amount)),
        TEXTURE_LIMIT,
        s.dehaze.as_ref().map_or(1.0, |d| d.factor as f32),
        s.clarity.as_ref().map_or(1.0, |c| c.subsample as f32),
        DEHAZE_T_FLOOR,
        0.0,
    ];
    out.extend(rows.iter().flat_map(|v| v.to_le_bytes()));
    out
}

/// The table texture's rows (see [`LUT_ROWS`]); rows of stages that are off stay zero.
fn tables(s: &PerPixel) -> Vec<f32> {
    let mut t = vec![0.0f32; (LUT_WIDTH * LUT_ROWS) as usize];
    let mut put = |row: usize, v: &[f32]| {
        let start = row * LUT_WIDTH as usize;
        let n = v.len().min(LUT_WIDTH as usize);
        if let (Some(dst), Some(src)) = (t.get_mut(start..start + n), v.get(..n)) {
            dst.copy_from_slice(src);
        }
    };
    if let Some(l) = &s.parametric {
        put(0, l);
    }
    for (k, l) in s.point.iter().enumerate() {
        if let Some(l) = l {
            put(1 + k, l);
        }
    }
    if let Some(wb) = &s.whites_blacks {
        if let Some(r) = &wb.whites {
            put(5, r);
        }
        if let Some(r) = &wb.blacks {
            put(6, r);
        }
    }
    t
}

// The Whites and Blacks rows fit a table row, at the rows the tone passes render them into.
const _: () = assert!(WB_STEPS <= CURVE_LUT && WB_ROW == 5 && LUT_ROWS == WB_ROW + 2);
