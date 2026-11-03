//! Probe: do textures uploaded like egui-wgpu's (TEXTURE_BINDING | COPY_DST, one full
//! `write_texture`) read back exactly what was written? Uploads mostly-transparent images with a
//! few opaque texels, samples every texel in a render pass, reads the target back and counts
//! texels that differ. Between uploads it frees render targets cleared to red, so stale memory is
//! red if the driver hands it back without honouring the copy.
//!
//! cargo run --release -p photocraft-gpu --example zero_block_probe -- [dx12|vulkan|gl] [rounds] [guard] [adapter=<name>]
//!
//! `adapter=<name>` probes the first adapter whose name contains `<name>` (case-insensitive), e.g.
//! `adapter=iris` on a machine that also has a discrete GPU; without it wgpu picks its default adapter.

use wgpu::util::DeviceExt;

const SHADER: &str = r"
@group(0) @binding(0) var t: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
    return vec4(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(t, vec2<i32>(p.xy), 0);
}
";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let backends = match args.get(1).map(String::as_str) {
        Some("vulkan") => wgpu::Backends::VULKAN,
        Some("gl") => wgpu::Backends::GL,
        _ => wgpu::Backends::DX12,
    };
    let rounds: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    // `guard`: transparent texels are uploaded as (0, 0, 0, 1), as `photocraft_ui_egui::texel_guard` does.
    let guard = args.get(3).is_some_and(|a| a == "guard");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = match args.iter().find_map(|a| a.strip_prefix("adapter=")).map(str::to_lowercase) {
        Some(want) => {
            let adapters = pollster::block_on(instance.enumerate_adapters(backends));
            let names: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
            adapters
                .into_iter()
                .find(|a| a.get_info().name.to_lowercase().contains(&want))
                .unwrap_or_else(|| panic!("no adapter matches {want:?}; found {names:?}"))
        }
        None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter"),
    };
    let info = adapter.get_info();
    println!("adapter: {} ({:?}) driver {} {}", info.name, info.backend, info.driver, info.driver_info);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");

    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });

    let mut bad_textures = 0;
    let mut bad_texels = 0usize;
    for round in 0..rounds {
        // Vary the size like the brush previews and icons do.
        let (w, h) = (16 + (round as u32 * 37) % 300, 16 + (round as u32 * 13) % 60);
        // Free some red render targets first: their memory is what a not-zeroed heap hands back.
        for _ in 0..2 {
            let rt = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("red-rt"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = rt.create_view(&Default::default());
            let mut enc = device.create_command_encoder(&Default::default());
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.03, b: 0.03, a: 1.0 }), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            queue.submit([enc.finish()]);
        }
        // The upload: transparent except a diagonal band of opaque grey, like a stroke strip.
        let mut px = if guard { [0u8, 0, 0, 1].repeat((w * h) as usize) } else { vec![0u8; (w * h * 4) as usize] };
        for y in 0..h {
            for x in 0..w {
                if (x as i64 * h as i64 / w as i64 - y as i64).abs() < 3 {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[200, 200, 200, 255]);
                }
            }
        }
        let tex = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("egui-like"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &px,
        );
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tex.create_view(&Default::default())) }],
        });
        let tview = target.create_view(&Default::default());
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tview,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLUE), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut bad = 0;
        let mut first = None;
        for y in 0..h {
            for x in 0..w {
                let got = &data[(y * row + x * 4) as usize..(y * row + x * 4 + 4) as usize];
                let want = &px[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
                if got != want {
                    bad += 1;
                    first.get_or_insert((x, y, got.to_vec()));
                }
            }
        }
        if bad > 0 {
            bad_textures += 1;
            bad_texels += bad;
            if bad_textures <= 10 {
                println!("round {round}: {w}x{h} texture has {bad} wrong texels, first {first:?}");
            }
        }
    }
    println!("{bad_textures}/{rounds} textures read back wrong ({bad_texels} texels)");
}
