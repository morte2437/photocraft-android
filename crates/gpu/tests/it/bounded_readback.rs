//! Tests for bounded, checked band readback across chunks, pages, and device failure modes.

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Effect, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_gpu::context::{NativeContext, NativeContextOptions};
use photocraft_gpu::readback::{DEFAULT_MAX_STAGING_BYTES, ReadbackError, render_bands, render_to_vec};
use photocraft_gpu::{Compositor, DeviceHealth};
use photocraft_raster::Surface;

use crate::GPU_LOCK;

fn test_gpu() -> Option<(NativeContext, Compositor, std::sync::MutexGuard<'static, ()>)> {
    let lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ctx = match NativeContext::request(&NativeContextOptions::default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping bounded readback test: {e}");
            return None;
        }
    };
    let comp = match ctx.create_compositor() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping bounded readback test: {e}");
            return None;
        }
    };
    Some((ctx, comp, lock))
}

fn sample_doc(w: u32, h: u32) -> Document {
    let mut doc = Document::with_background("sample", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(10, 10, w as i32 - 10, h as i32 - 10), &[0.2, 0.6, 0.8, 0.7]);
    let mut top = Layer::new("top", LayerContent::Raster(s));
    top.opacity = 0.85;
    top.blend = BlendMode::Multiply;
    doc.layers.push(top);
    doc
}

#[test]
fn bounded_readback_matches_independent_cpu_oracle() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(200, 300);
    let rect = doc.bounds();

    // Independent reference CPU compositor oracle
    let mut cpu_pixels = Vec::new();
    photocraft_compose::render_bands(&doc, rect, 0, |buf| {
        cpu_pixels.extend_from_slice(&buf.px);
        Ok::<(), std::convert::Infallible>(())
    })
    .expect("cpu compose");

    // GPU bounded readback in bands of 16 rows
    let mut gpu_banded = Vec::new();
    let mut total_rows = 0;
    render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, rect, 16, 32 * 1024, &photocraft_raster::Interrupt::NONE, |band_rect, px| {
        total_rows += band_rect.height();
        gpu_banded.extend_from_slice(&px);
        Ok::<(), std::convert::Infallible>(())
    })
    .expect("banded readback");

    assert_eq!(total_rows, rect.height());
    assert_eq!(cpu_pixels.len(), gpu_banded.len());
    for (i, (c, g)) in cpu_pixels.iter().zip(&gpu_banded).enumerate() {
        for ch in 0..4 {
            assert!((c[ch] - g[ch]).abs() <= 1.0 / 255.0, "pixel {i} ch {ch} differs from CPU oracle: cpu={}, gpu={}", c[ch], g[ch]);
        }
    }
}

#[test]
fn bounded_readback_strictly_respects_staging_budget() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(256, 256);
    let rect = doc.bounds();
    let budget = 16 * 1024; // 16 KiB strict cap

    let bpp = if comp.acc_format() == wgpu::TextureFormat::Rgba32Float { 16u64 } else { 8u64 };
    let chunk_size = comp.chunk_size() as i32;

    let mut row_staging_bytes: u64 = 0;
    let mut x = rect.x0.div_euclid(chunk_size) * chunk_size;
    while x < rect.x1 {
        let x_end = (x + chunk_size).min(rect.x1);
        let chunk_w = (x_end - x.max(rect.x0)) as u64;
        let chunk_row = (chunk_w * bpp).div_ceil(256) * 256;
        row_staging_bytes += chunk_row;
        x += chunk_size;
    }

    render_bands(
        &mut comp,
        ctx.device(),
        ctx.queue(),
        &doc,
        rect,
        0, // let provider calculate bounded rows
        budget,
        &photocraft_raster::Interrupt::NONE,
        |band_rect, px| {
            let actual_staging = band_rect.height() as u64 * row_staging_bytes;
            assert!(actual_staging <= budget as u64, "band staging {} exceeds budget {}", actual_staging, budget);
            assert_eq!(px.len(), (band_rect.width() * band_rect.height()) as usize);
            Ok::<(), std::convert::Infallible>(())
        },
    )
    .expect("bounded readback budget respected");
}

#[test]
fn hostile_dimensions_rejects_without_huge_allocation() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };

    // 1. Budget smaller than device 256-byte alignment
    let doc = sample_doc(64, 64);
    let res = render_bands(
        &mut comp,
        ctx.device(),
        ctx.queue(),
        &doc,
        doc.bounds(),
        0,
        128, // less than 256
        &photocraft_raster::Interrupt::NONE,
        |_r, _px| Ok::<(), String>(()),
    );
    assert!(matches!(res, Err(ReadbackError::AllocationFailed(_))));

    // 2. Wide document where 1 full row cannot fit in staging budget
    let wide_doc = sample_doc(10_000, 10);
    let small_budget = 4096; // 4 KiB cannot fit 1 row of 10,000 pixels (>= 80 KiB)
    let res =
        render_bands(&mut comp, ctx.device(), ctx.queue(), &wide_doc, wide_doc.bounds(), 0, small_budget, &photocraft_raster::Interrupt::NONE, |_r, _px| {
            Ok::<(), String>(())
        });
    assert!(matches!(res, Err(ReadbackError::Unsupported(_))), "must safely fall back when 1 row cannot fit in budget: {res:?}");

    // 3. Multi-chunk wide document spanning multiple chunk boundaries works via banding
    let multi_chunk_doc = sample_doc(2500, 64);
    let mut multi_chunk_px = Vec::new();
    let res = render_bands(
        &mut comp,
        ctx.device(),
        ctx.queue(),
        &multi_chunk_doc,
        multi_chunk_doc.bounds(),
        16,
        DEFAULT_MAX_STAGING_BYTES,
        &photocraft_raster::Interrupt::NONE,
        |_r, px| {
            multi_chunk_px.extend_from_slice(&px);
            Ok::<(), String>(())
        },
    );
    assert!(res.is_ok(), "multi-chunk canvas must succeed via banding: {res:?}");
    assert_eq!(multi_chunk_px.len(), 2500 * 64);
}

#[test]
fn sink_cancellation_and_error_preservation() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(128, 128);

    let mut band_count = 0;
    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, &photocraft_raster::Interrupt::NONE, |_r, _px| {
        band_count += 1;
        Err("abort_sink_now")
    });

    assert_eq!(band_count, 1, "sink failure must immediately stop further band rendering");
    match res {
        Err(ReadbackError::Sink(e)) => assert_eq!(e, "abort_sink_now"),
        other => panic!("expected ReadbackError::Sink, got {other:?}"),
    }
}

#[test]
fn readback_honors_cancellation_before_waiting() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(128, 128);
    let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
    let mut called = false;
    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, &cancelled, |_rect, _px| {
        called = true;
        Ok::<(), std::convert::Infallible>(())
    });
    assert!(matches!(res, Err(ReadbackError::Cancelled)), "a cancelled readback must stop before the device wait: {res:?}");
    assert!(!called, "no band may be delivered after cancellation");
}

#[test]
fn readback_honors_cancellation_between_bands() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(128, 128);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let is_cancelled = || cancel.load(std::sync::atomic::Ordering::SeqCst);
    let cancelled = photocraft_raster::Interrupt::cancel_only(&is_cancelled);
    let mut calls = 0usize;
    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, &cancelled, |_rect, _px| {
        calls += 1;
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok::<(), std::convert::Infallible>(())
    });
    assert!(matches!(res, Err(ReadbackError::Cancelled)), "cancellation set after the first band must stop the readback: {res:?}");
    assert_eq!(calls, 1, "only the first band may be delivered");
}

#[test]
fn bounded_readback_preserves_effect_halos_across_seams() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let (w, h) = (256, 256);
    let mut doc = Document::with_background("fx_doc", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);

    // Layer positioned right across row 128 (the middle of the canvas)
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(50, 110, 200, 145), &[0.9, 0.1, 0.1, 1.0]);
    let mut layer = Layer::new("shadow_layer", LayerContent::Raster(s));

    // Drop shadow with blur radius 16 extending downward and upward
    let Effect::DropShadow(mut shadow) = Effect::default_drop_shadow() else { unreachable!() };
    shadow.distance = 8.0;
    shadow.size = 16.0;
    shadow.common.opacity = 0.8;
    layer.effects.items.push(Effect::DropShadow(shadow));
    doc.layers.push(layer);

    let rect = doc.bounds();
    if let Err(reason) = comp.supports(&doc) {
        assert!(reason.0.contains("float effect-map render targets"), "unexpected unsupported feature: {reason:?}");
        let result = render_bands(
            &mut comp,
            ctx.device(),
            ctx.queue(),
            &doc,
            rect,
            32,
            32 * 1024,
            &photocraft_raster::Interrupt::NONE,
            |_, _| -> Result<(), std::convert::Infallible> {
                panic!("unsupported layer effects must not emit pixels");
            },
        );
        assert!(matches!(result, Err(ReadbackError::Unsupported(_))));
        eprintln!("SKIPPED effect seam parity: {reason:?}; verified unsupported-feature rejection instead");
        return;
    }
    let full = render_to_vec(&mut comp, ctx.device(), ctx.queue(), &doc, rect).expect("full readback");

    // Render with bands of 32 rows so the shadow seamlessly crosses multiple band boundaries
    let mut banded = Vec::new();
    render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, rect, 32, 32 * 1024, &photocraft_raster::Interrupt::NONE, |_band_rect, px| {
        banded.extend_from_slice(&px);
        Ok::<(), std::convert::Infallible>(())
    })
    .expect("banded readback with effects");

    assert_eq!(full.len(), banded.len());
    for (i, (f, b)) in full.iter().zip(&banded).enumerate() {
        for c in 0..4 {
            assert!((f[c] - b[c]).abs() < 2.0 / 255.0, "halo pixel {i} channel {c} differs: full={}, banded={}", f[c], b[c]);
        }
    }
}

#[test]
fn high_depth_documents_are_rejected_before_readback() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    // Half-precision uploads cannot preserve 16-bit samples, so U16 joins F32 on the CPU path.
    for depth in [SampleType::U16, SampleType::F32] {
        let doc = Document::with_background("high_depth", Size::new(64, 64), ColorMode::Rgb, depth, Color::WHITE);
        let mut called = false;
        let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, &photocraft_raster::Interrupt::NONE, |_rect, _px| {
            called = true;
            Ok::<(), std::convert::Infallible>(())
        });
        assert!(matches!(res, Err(ReadbackError::Unsupported(_))), "{depth:?} must be rejected before readback: {res:?}");
        assert!(!called, "no band may be read for a {depth:?} document");
    }
}

#[test]
fn bounded_readback_handles_empty_rect() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(100, 100);
    let mut called = false;
    let res =
        render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, Rect::new(0, 0, 0, 0), 16, 64 * 1024, &photocraft_raster::Interrupt::NONE, |_rect, _px| {
            called = true;
            Ok::<(), std::convert::Infallible>(())
        });
    assert!(res.is_ok());
    assert!(!called, "sink should not be called for empty rect");
}

#[test]
fn bounded_readback_notices_device_fault() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(100, 100);

    // Simulate an explicit device fault
    let health = DeviceHealth::new();
    health.mark(photocraft_gpu::Fault::Lost("simulated driver crash".into()));
    comp.set_health(health);

    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, &photocraft_raster::Interrupt::NONE, |_rect, _px| {
        Ok::<(), std::convert::Infallible>(())
    });

    assert!(matches!(res, Err(ReadbackError::DeviceLost(_))));
}
