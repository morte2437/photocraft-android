//! Styled shape group translation: synchronized frame time and effect-map rebuild counters.
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Fill, Layer, LayerContent, Path, ShapeLayer, Subpath};
use photocraft_geom::{Affine, Size};
use photocraft_gpu::Compositor;
use std::time::Instant;

/// Translate the fixture's paths and raster caches together, as a group drag preview does.
fn moved(base: &Document, offset: i32) -> Document {
    let mut doc = base.clone();
    if let Some(group) = doc.layers.last_mut().and_then(Layer::children_mut) {
        for layer in group {
            if let LayerContent::Shape(shape) = &mut layer.content {
                shape.path = shape.path.transform(&Affine::translate(f64::from(offset), f64::from(offset)));
                if let Some(cache) = &mut shape.cache {
                    *cache = cache.translated(offset, offset, photocraft_compose::bounds::content_bounds(cache));
                }
            }
        }
    }
    doc
}
/// Measure synchronized drag frames; quick mode also verifies fresh-render parity.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let quick = std::env::args().any(|a| a == "--quick");
    let size = if quick { Size::new(1600, 1200) } else { Size::new(6000, 4000) };
    let mut doc = Document::with_background("styled shapes", size, ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut children = Vec::new();
    for i in 0..12 {
        let x = 100.1 + f64::from(i % 4) * f64::from(size.width) / 5.0;
        let y = 100.2 + f64::from(i / 4) * f64::from(size.height) / 4.0;
        let w = f64::from(size.width) / 7.0;
        let h = f64::from(size.height) / 6.0;
        let mut shape = ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])]),
            fill: Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.8))),
            ..Default::default()
        };
        shape.cache = Some(photocraft_vector::render_shape(&shape, doc.pixel_format(), doc.bounds()));
        let mut layer = Layer::new(format!("shape {i}"), LayerContent::Shape(shape));
        layer.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        children.push(layer);
    }
    doc.layers.push(Layer::group("group", children));
    let adapter = pollster::block_on(
        wgpu::Instance::default()
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    println!("adapter={:?} document={}x{}", adapter.get_info().name, size.width, size.height);
    let mut comp = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter))?;
    let mut samples = Vec::new();
    for frame in 0..8 {
        let start = Instant::now();
        let current = moved(&doc, frame * 3);
        let preview_ms = start.elapsed().as_secs_f64() * 1000.0;
        let stats = comp.render(&device, &queue, &current, current.bounds(), |_, _| {})?;
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "frame={frame} ms={ms:.3} preview_ms={preview_ms:.3} fx_shapes={} fx_programs={} fx_pixels={}",
            stats.fx_shapes, stats.fx_programs, stats.fx_pixels
        );
        if frame > 1 {
            samples.push(ms);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!("warm_median_ms={:.3}", samples.get(samples.len() / 2).copied().unwrap_or_default());
    if quick {
        let current = moved(&doc, 21);
        let warm = photocraft_gpu::render_to_vec(&mut comp, &device, &queue, &current, current.bounds())?;
        let mut fresh = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter))?;
        let expected = photocraft_gpu::render_to_vec(&mut fresh, &device, &queue, &current, current.bounds())?;
        let difference = warm.iter().zip(&expected).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0f32, f32::max);
        println!("fresh_render_max_difference={difference}");
        if difference >= 1.0 / 255.0 {
            return Err("reused effect maps differ from fresh rendering".into());
        }
    }
    Ok(())
}
