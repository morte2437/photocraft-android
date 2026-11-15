//! Measure compositor pipeline construction and, optionally, device work during a background build.
//!
//! `cargo run --release -p photocraft-gpu --example compositor_build_probe -- [dx12|vulkan|metal|gl] [fxc|dxc] [rounds=3] [threads=N] [--background] [--json out.json]`
//!
//! The explicit thread count is passed to the dedicated Rayon build pool.

use photocraft_gpu::Compositor;
use std::fs;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Sample {
    build_ms: f64,
    ok: bool,
    error: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let backend = args.first().map(String::as_str).unwrap_or("dx12");
    let compiler = args.get(1).map(String::as_str).unwrap_or("fxc");
    let rounds = args.iter().find_map(|a| a.strip_prefix("rounds=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(3).max(1);
    let default_threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
    let threads = args.iter().find_map(|a| a.strip_prefix("threads=").and_then(|s| s.parse::<usize>().ok())).unwrap_or(default_threads).max(1);
    let background = args.iter().any(|a| a == "--background");
    let json_path = args.iter().position(|a| a == "--json").and_then(|i| args.get(i + 1));
    let backends = match backend {
        "vulkan" => wgpu::Backends::VULKAN,
        "metal" => wgpu::Backends::METAL,
        "gl" => wgpu::Backends::GL,
        _ => wgpu::Backends::DX12,
    };
    let shader_compiler = match compiler {
        "dxc" => wgpu::Dx12Compiler::default_dynamic_dxc(),
        _ => wgpu::Dx12Compiler::Fxc,
    };
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    descriptor.flags = wgpu::InstanceFlags::empty();
    descriptor.backend_options.dx12.shader_compiler = shader_compiler;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..Default::default() }))?;
    let info = adapter.get_info();
    println!("adapter={} backend={:?} driver={} {} threads={threads}", info.name, info.backend, info.driver, info.driver_info);

    let mut samples = Vec::with_capacity(rounds);
    let mut stall = Vec::new();
    let mut cpu_idle_ms = None;
    let mut cpu_during_ms = None;
    for round in 0..rounds {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        let format = Compositor::preferred_acc_format(&adapter);
        if background {
            if round == 0 {
                let doc = photocraft_doc::Document::with_background(
                    "compositor build probe",
                    photocraft_geom::Size::new(1920, 1080),
                    photocraft_color::ColorMode::Rgb,
                    photocraft_color::SampleType::U8,
                    photocraft_color::Color::WHITE,
                );
                let start = Instant::now();
                let _ = photocraft_compose::render(&doc, doc.bounds());
                cpu_idle_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
            }
            let (build_tx, build_rx) = std::sync::mpsc::channel();
            let start = Instant::now();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let build_start = Instant::now();
                    let result = Compositor::try_new_with_format_threads(&device, format, threads);
                    let elapsed = build_start.elapsed().as_secs_f64() * 1000.0;
                    let error = result.err().map(|e| e.to_string());
                    let _ = build_tx.send((error, elapsed));
                });
                let target = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("compositor probe UI target"),
                    size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let view = target.create_view(&Default::default());
                let mut last_cpu = start;
                loop {
                    match build_rx.try_recv() {
                        Ok((error, build_ms)) => {
                            samples.push(Sample { build_ms, ok: error.is_none(), error });
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            samples.push(Sample { build_ms: start.elapsed().as_secs_f64() * 1000.0, ok: false, error: Some("build thread terminated".into()) });
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => (),
                    }
                    let tick = Instant::now();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("probe frame"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        })],
                        ..Default::default()
                    });
                    drop(_pass);
                    queue.submit([encoder.finish()]);
                    let _ = device.poll(wgpu::PollType::Poll);
                    stall.push(tick.elapsed().as_secs_f64() * 1000.0);
                    if round == 0 && cpu_during_ms.is_none() && last_cpu.elapsed() >= Duration::from_millis(100) {
                        let doc = photocraft_doc::Document::with_background(
                            "compositor build probe",
                            photocraft_geom::Size::new(1920, 1080),
                            photocraft_color::ColorMode::Rgb,
                            photocraft_color::SampleType::U8,
                            photocraft_color::Color::WHITE,
                        );
                        let cpu_start = Instant::now();
                        let _ = photocraft_compose::render(&doc, doc.bounds());
                        cpu_during_ms = Some(cpu_start.elapsed().as_secs_f64() * 1000.0);
                        last_cpu = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
            });
        } else {
            let start = Instant::now();
            let result = Compositor::try_new_with_format_threads(&device, format, threads);
            let build_ms = start.elapsed().as_secs_f64() * 1000.0;
            let error = result.err().map(|e| e.to_string());
            samples.push(Sample { build_ms, ok: error.is_none(), error });
        }
        let s = samples.last().ok_or("missing compositor build sample")?;
        println!("round={round} build_ms={:.3} ok={} error={}", s.build_ms, s.ok, s.error.as_deref().unwrap_or(""));
    }

    stall.sort_by(f64::total_cmp);
    let p95 = stall.get((stall.len() * 95 / 100).min(stall.len().saturating_sub(1))).copied().unwrap_or(0.0);
    let max = stall.last().copied().unwrap_or(0.0);
    println!("ui_stall_p95_ms={p95:.3} ui_stall_max_ms={max:.3} cpu_idle_ms={cpu_idle_ms:?} cpu_during_ms={cpu_during_ms:?}");
    if let Some(path) = json_path {
        let rounds_json = samples
            .iter()
            .map(|s| {
                format!("{{\"build_ms\":{:.3},\"ok\":{},\"error\":{}}}", s.build_ms, s.ok, s.error.as_deref().map(json_string).unwrap_or_else(|| "null".into()))
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"backend\":{},\"compiler\":{},\"adapter\":{},\"threads\":{},\"rounds\":[{}],\"stall\":{{\"p95_ms\":{p95:.3},\"max_ms\":{max:.3}}},\"cpu_composite_ms\":{{\"idle\":{},\"during_build\":{}}}}}",
            json_string(backend),
            json_string(compiler),
            json_string(&info.name),
            threads,
            rounds_json,
            optional_number(cpu_idle_ms),
            optional_number(cpu_during_ms)
        );
        fs::write(path, json)?;
    }
    Ok(())
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn optional_number(n: Option<f64>) -> String {
    n.map(|v| format!("{v:.3}")).unwrap_or_else(|| "null".into())
}
