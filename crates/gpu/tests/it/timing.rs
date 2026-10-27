//! The canvas timing must measure GPU *execution*, not encode-and-submit (#7186).
//!
//! `composite_ms` reports the CPU cost of handing the composite to the driver, which on a
//! GPU-bound refresh is a small fraction of what the refresh costs — at 24 MP it read 7.7 ms for
//! work that took ~171 ms. `Compositor::measure_gpu_time` adds timestamp queries whose spans are
//! summed into a [`GpuTiming`]. This test pins that down from both sides: the measurement must
//! track the waited-for wall clock and must be several times the submit time, so an accidental
//! return to timing the submit fails it.
//!
//! **Hardware only.** A software rasteriser composites on the CPU, where submit and execution
//! are the same work, so the whole premise disappears and the test would pass on a bug. It
//! therefore refuses to run on a CPU adapter and says so, as every GPU timing measurement in
//! this project must (egui_kittest's `default_wgpu_setup` prefers CPU adapters on purpose).

use std::time::Instant;

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_geom::Rect;
use photocraft_gpu::{Compositor, DeviceHealth, GpuTiming, timing};

/// Pixels in `r`, as the compositor counts them.
fn area(r: Rect) -> u64 {
    r.width() as u64 * r.height() as u64
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut f = std::pin::pin!(f);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// A real GPU with timestamp queries, or `None` with the reason printed.
fn gpu() -> Option<(wgpu::Device, wgpu::Queue, Compositor)> {
    let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let Ok(adapter) = block_on(wgpu::Instance::default().request_adapter(&options)) else {
        eprintln!("SKIP gpu timing: no adapter");
        return None;
    };
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu || info.backend == wgpu::Backend::Noop {
        // Not a skip we can be quiet about: a GPU timing test on llvmpipe measures nothing.
        eprintln!("SKIP gpu timing: {} is a software adapter, which cannot show a GPU/CPU split", info.name);
        return None;
    }
    if !adapter.features().contains(timing::FEATURES) {
        eprintln!("SKIP gpu timing: {} has no timestamp queries", info.name);
        return None;
    }
    let descriptor = wgpu::DeviceDescriptor { required_features: timing::FEATURES, ..Default::default() };
    let Ok((device, queue)) = block_on(adapter.request_device(&descriptor)) else {
        eprintln!("SKIP gpu timing: no device with timestamp queries");
        return None;
    };
    let mut comp = Compositor::new(&device);
    assert!(comp.measure_gpu_time(&device, &queue), "the device has the features, so the timer must build");
    eprintln!("gpu timing on {} ({:?}, {:?})", info.name, info.device_type, info.backend);
    Some((device, queue, comp))
}

fn rnd(seed: u32, i: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ i.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

/// A document whose composite is clearly GPU-bound: several full-canvas blended photo layers.
fn document(size: Size) -> Document {
    let mut doc = Document::with_background("timing", size, ColorMode::Rgb, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    let fmt = doc.pixel_format();
    let rect = Rect::from_xywh(0, 0, size.width, size.height);
    for (i, blend) in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay, BlendMode::SoftLight].into_iter().enumerate() {
        let mut layer = Layer::raster(format!("l{i}"), fmt);
        let channels = fmt.channels();
        let px: Vec<f32> = (0..rect.width() * rect.height()).flat_map(|p| (0..channels).map(move |c| rnd(7 + i as u32 * 131 + c as u32 * 7919, p))).collect();
        if let Some(s) = layer.surface_mut() {
            s.write_region(rect, &px);
        }
        layer.blend = blend;
        layer.opacity = 0.8;
        doc.layers.push(layer);
    }
    doc
}

/// Render `region` and return (submit ms, waited-for wall ms, the measurement).
fn measure(device: &wgpu::Device, queue: &wgpu::Queue, comp: &mut Compositor, doc: &Document, region: Rect) -> (f64, f64, Option<GpuTiming>) {
    let t0 = Instant::now();
    comp.render(device, queue, doc, region, |_, _| {}).expect("the document composites on the GPU");
    let submit_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
    // The readback map completes on a poll, and its callback runs on the next one.
    let mut timing = None;
    for _ in 0..4 {
        timing = comp.take_gpu_timing();
        if timing.is_some() {
            break;
        }
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    }
    (submit_ms, wall_ms, timing)
}

#[test]
fn gpu_time_tracks_execution_not_submission() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, mut comp)) = gpu() else { return };
    let size = Size::new(3000, 2000);
    let doc = document(size);
    let full = Rect::from_xywh(0, 0, size.width, size.height);

    // Warm the residency and pipelines: the first refresh uploads every layer.
    let _ = measure(&device, &queue, &mut comp, &doc, full);

    // Best of three, by the smallest encode. Everything below is a ratio against `submit_ms`,
    // and CPU contention on a shared machine only ever inflates it — one run that happened to
    // encode behind a busy scheduler (8.2 ms against a typical 1.0 ms) failed the comparison
    // while the measurement itself was right. The GPU side is unaffected by that contention,
    // so taking the least-disturbed encode tests the same property without the false failure.
    let mut best: Option<(f64, f64, Option<GpuTiming>)> = None;
    for _ in 0..3 {
        let run = measure(&device, &queue, &mut comp, &doc, full);
        if run.2.is_some() && best.as_ref().is_none_or(|b| run.0 < b.0) {
            best = Some(run);
        }
    }
    let (submit_ms, wall_ms, timing) = best.expect("at least one of three refreshes reports a GPU time");
    let t = timing.expect("a hardware device with timestamp queries must report a GPU time");
    eprintln!("full {}x{}: submit {submit_ms:.1} ms, waited wall {wall_ms:.1} ms, gpu {:.1} ms", size.width, size.height, t.ms);

    assert_eq!(t.px, area(full), "the measurement carries the pixels of the refresh it measured");
    assert!(t.spans >= 1, "at least one command buffer was timed");

    // The premise: on this adapter the refresh really is dominated by GPU execution. `wall_ms`
    // includes `submit_ms`, so the GPU execution window is `wall_ms - submit_ms`; a machine fast
    // enough that this window isn't well above `3.0 * submit_ms` cannot tell the two timings
    // apart, so say so instead of failing.
    if wall_ms - submit_ms < 3.0 * submit_ms {
        eprintln!("SKIP the comparison: this GPU finishes a 6 MP composite in {wall_ms:.1} ms against {submit_ms:.1} ms of encoding");
        return;
    }

    // Timing the submit again (the bug) would report roughly `submit_ms` here.
    assert!(t.ms > 3.0 * submit_ms, "gpu time {:.1} ms looks like the submit time {submit_ms:.1} ms, not the execution", t.ms);
    // …and it must be the real execution, not merely a bigger number: the waited-for wall clock
    // is encode + execute, so the GPU span sits below it and accounts for most of it.
    assert!(t.ms <= wall_ms * 1.25, "gpu time {:.1} ms exceeds the whole waited-for refresh {wall_ms:.1} ms", t.ms);
    assert!(t.ms >= (wall_ms - submit_ms) * 0.5, "gpu time {:.1} ms is far below the {:.1} ms the refresh waited", t.ms, wall_ms - submit_ms);

    // A quarter of the canvas is a quarter of the per-pixel work: a constant or a stale value
    // would not follow it.
    let quarter = Rect::from_xywh(0, 0, size.width / 2, size.height / 2);
    let (_, _, small) = measure(&device, &queue, &mut comp, &doc, quarter);
    let s = small.expect("the smaller refresh is measured too");
    eprintln!("quarter: gpu {:.1} ms for {} px against {:.1} ms for {} px", s.ms, s.px, t.ms, t.px);
    assert_eq!(s.px, area(quarter));
    assert!(s.ms < t.ms * 0.6, "a quarter-canvas refresh took {:.1} ms against the full canvas's {:.1} ms", s.ms, t.ms);
}

#[test]
fn an_unmeasured_compositor_reports_nothing() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, _)) = gpu() else { return };
    let mut comp = Compositor::new(&device);
    assert!(!comp.measures_gpu_time(), "timing is off until it is asked for");
    let doc = document(Size::new(256, 256));
    comp.render(&device, &queue, &doc, Rect::from_xywh(0, 0, 256, 256), |_, _| {}).expect("composites");
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    assert_eq!(comp.take_gpu_timing(), None, "no measurement is reported rather than a wrong one");
}

#[test]
fn a_refresh_that_submits_several_times_is_one_measurement() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, mut comp)) = gpu() else { return };
    // A budget far below the document's layer pages makes the refresh evict as it goes, which is
    // what makes it submit in batches: the measurement must span all of them, not just the last.
    comp.set_memory_budget(24 << 20);
    let size = Size::new(3000, 2000);
    let doc = document(size);
    let full = Rect::from_xywh(0, 0, size.width, size.height);

    let _ = measure(&device, &queue, &mut comp, &doc, full);
    let (submit_ms, wall_ms, timing) = measure(&device, &queue, &mut comp, &doc, full);
    let t = timing.expect("a batched refresh is measured like any other");
    eprintln!("batched: {} command buffers, gpu {:.1} ms, submit {submit_ms:.1} ms, waited wall {wall_ms:.1} ms", t.spans, t.ms);
    assert_eq!(t.px, area(full));
    if t.spans < 2 {
        eprintln!("SKIP the sum: this refresh still fit one command buffer");
        return;
    }
    assert!(t.ms <= wall_ms * 1.25, "summed gpu time {:.1} ms exceeds the whole waited-for refresh {wall_ms:.1} ms", t.ms);
    // What this bound excludes is summing only the last command buffer. A correct sum measures
    // ~85 % of the waited-for GPU time here; the flush thresholds split on accumulated work, so
    // no single buffer of the several carries much over half of it, and timing only the last
    // would land near 40 %. 0.6 sits between the two with room on both sides for a machine
    // whose split is less even than this one's.
    assert!(
        t.ms >= (wall_ms - submit_ms) * 0.6,
        "summed gpu time {:.1} ms is far below the {:.1} ms the refresh waited: is only the last command buffer timed?",
        t.ms,
        wall_ms - submit_ms
    );
}

#[test]
fn refreshes_faster_than_the_readback_stay_valid() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, mut comp)) = gpu() else { return };
    // Refreshes arriving before the last measurement has been read back must simply go
    // unmeasured. Mapping the readback buffer again while its first map is still pending is a
    // validation error, and an unusable device drops the whole canvas to the CPU compositor.
    let health = DeviceHealth::watch(&device);
    comp.set_health(health.clone());
    let size = Size::new(600, 400);
    let doc = document(size);
    let full = Rect::from_xywh(0, 0, size.width, size.height);
    for i in 0..16 {
        comp.render(&device, &queue, &doc, full, |_, _| {}).unwrap_or_else(|e| panic!("refresh {i} failed: {e}"));
    }
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    assert_eq!(comp.fault(), None, "hurried refreshes must not fault the device");
    assert_eq!(health.errors(), 0, "no uncaptured wgpu errors");
}

/// A refresh that recorded nothing must not stop the timer for good. `finish` leaves the
/// measurement's slots protected by the pending readback, not by `flushed`; if `flushed`
/// survived a successful finish, the next refresh to fail while that readback was still in
/// flight would set `stopped` on the strength of the *earlier* refresh's flush — even though it
/// wrote no timestamps itself — and the compositor would silently never measure again.
#[test]
fn a_failed_refresh_that_recorded_nothing_keeps_the_timer_alive() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, _)) = gpu() else { return };
    let mut timer = timing::GpuTimer::new(&device, &queue).expect("the device has timestamp queries");

    // A refresh that flushes once, then succeeds.
    let mut first = device.create_command_encoder(&Default::default());
    let mut second = device.create_command_encoder(&Default::default());
    timer.start(&mut first, 1024);
    timer.split(&mut first, &mut second);
    queue.submit([first.finish()]);
    timer.finish(&mut second);
    queue.submit([second.finish()]);
    timer.submitted();

    // A later refresh fails before recording anything, because the readback is still in flight.
    let mut third = device.create_command_encoder(&Default::default());
    timer.start(&mut third, 2048);
    timer.cancel();
    drop(third);

    // The first measurement still lands...
    let mut first_timing = None;
    for _ in 0..8 {
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        first_timing = timer.take();
        if first_timing.is_some() {
            break;
        }
    }
    let landed = first_timing.expect("the flushed measurement completes");
    assert_eq!(landed.spans, 2, "both command buffers are one measurement");

    // ...and the timer still measures afterwards, which is what the bug destroyed.
    let mut next = device.create_command_encoder(&Default::default());
    timer.start(&mut next, 4096);
    timer.finish(&mut next);
    queue.submit([next.finish()]);
    timer.submitted();
    let mut after = None;
    for _ in 0..8 {
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        after = timer.take();
        if after.is_some() {
            break;
        }
    }
    let after = after.expect("the timer still measures after a refresh that recorded nothing");
    assert_eq!(after.px, 4096, "the measurement belongs to the refresh that produced it");
}

/// A refresh whose layer pages ran out of GPU memory even after the submit-and-retry ends on the
/// CPU (#6563), but measuring goes on: the retry waited for the GPU, so none of the dropped
/// measurement's timestamps were still in flight.
#[test]
fn an_out_of_memory_refresh_does_not_stop_gpu_timing() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((device, queue, mut comp)) = gpu() else { return };
    let doc = document(Size::new(640, 480));
    let full = doc.bounds();
    // The first page texture fails twice, and twice more in the retry after the submit.
    comp.fail_layer_allocs(4);
    assert!(comp.render(&device, &queue, &doc, full, |_, _| {}).is_err(), "every try failed");
    let mut timing = None;
    for _ in 0..4 {
        timing = measure(&device, &queue, &mut comp, &doc, full).2;
        if timing.is_some() {
            break;
        }
    }
    assert!(timing.is_some(), "GPU timing stopped for good after one out-of-memory refresh");
}
