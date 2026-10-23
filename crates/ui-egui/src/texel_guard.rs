//! Intel Iris Xe on DirectX 12: no egui texture upload carries an all-zero texel, because the
//! driver shows stale memory where it does (the red blocks in Brush Preset picker strokes and
//! around tool icons).
//!
//! wgpu creates every texture on Iris Xe as a committed resource (its workaround for
//! gfx-rs/wgpu#3552, where suballocated textures had the same symptom) and, since wgpu added it,
//! with `D3D12_HEAP_FLAG_CREATE_NOT_ZEROED`. egui-wgpu fills each new texture with one full copy,
//! so wgpu never clears it first. On this driver the copy leaves 4×4 blocks whose texels are all
//! (0, 0, 0, 0) showing whatever the memory last held, typically a freed texture's pixels. Blocks
//! with any other value upload correctly, which is why the garbage sits only in the empty corners
//! of a thumbnail, never next to what is drawn. It shows up after textures have been freed and
//! made again: scrolling the picker (stroke previews come and go), switching brush tools (the
//! options bar and toolbar icons are rasterized again).
//!
//! [`TexelGuard`] rewrites fully transparent texels in every upload as premultiplied
//! (0, 0, 0, 1) before eframe hands the upload to the renderer: alpha 1/255 black, which darkens
//! what is underneath by less than one 8-bit level. It is installed only on the affected adapter
//! ([`affected`]); `PHOTOCRAFT_TEXEL_GUARD=1` forces it on (to check it on any GPU) and `=0` off.

use std::sync::Arc;

use eframe::wgpu;
use egui::{Color32, FullOutput, ImageData, TexturesDelta};

/// What a fully transparent texel is uploaded as.
pub const NEAR_CLEAR: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 1);

/// Whether texture uploads on this adapter need the guard (`env`: `PHOTOCRAFT_TEXEL_GUARD`).
pub fn affected(info: &wgpu::AdapterInfo, env: Option<&str>) -> bool {
    match env {
        Some("1") => true,
        Some("0") => false,
        // The same adapter test wgpu uses for its #3552 workaround.
        _ => info.backend == wgpu::Backend::Dx12 && info.name.contains("Iris(R) Xe"),
    }
}

/// Rewrite the fully transparent texels of every upload in `delta` as [`NEAR_CLEAR`]; returns
/// how many changed. Images without one are not copied.
pub fn guard(delta: &mut TexturesDelta) -> usize {
    let mut changed = 0;
    for d in delta.set.values_mut().flatten() {
        let ImageData::Color(image) = &mut d.image;
        if !image.pixels.contains(&Color32::TRANSPARENT) {
            continue;
        }
        for p in &mut Arc::make_mut(image).pixels {
            if *p == Color32::TRANSPARENT {
                *p = NEAR_CLEAR;
                changed += 1;
            }
        }
    }
    changed
}

/// The egui plugin that runs [`guard`] on each frame's texture uploads.
pub struct TexelGuard;

impl egui::Plugin for TexelGuard {
    fn debug_name(&self) -> &'static str {
        "TexelGuard"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut FullOutput) {
        guard(&mut output.textures_delta);
    }
}

/// Install [`TexelGuard`] on `ctx` when the window's adapter needs it; returns whether it did.
/// Call once, with the adapter eframe renders the window with.
pub fn install(ctx: &egui::Context, info: &wgpu::AdapterInfo) -> bool {
    let on = affected(info, std::env::var("PHOTOCRAFT_TEXEL_GUARD").ok().as_deref());
    if on && ctx.plugin_opt::<TexelGuard>().is_none() {
        ctx.add_plugin(TexelGuard);
        log::info!("texture uploads: transparent texels written as alpha 1/255 ({}, {:?}; gfx-rs/wgpu#3552)", info.name, info.backend);
    }
    on
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ImageDelta;
    use egui::{ColorImage, TextureId, TextureOptions};

    fn adapter(name: &str, backend: wgpu::Backend) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo { name: name.into(), ..wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, backend) }
    }

    #[test]
    fn only_iris_xe_on_dx12_is_affected_unless_forced() {
        assert!(affected(&adapter("Intel(R) Iris(R) Xe Graphics", wgpu::Backend::Dx12), None));
        assert!(!affected(&adapter("Intel(R) Iris(R) Xe Graphics", wgpu::Backend::Vulkan), None));
        assert!(!affected(&adapter("Intel(R) UHD Graphics 620", wgpu::Backend::Dx12), None));
        assert!(!affected(&adapter("NVIDIA GeForce RTX 4070", wgpu::Backend::Dx12), None));
        assert!(affected(&adapter("NVIDIA GeForce RTX 4070", wgpu::Backend::Dx12), Some("1")));
        assert!(!affected(&adapter("Intel(R) Iris(R) Xe Graphics", wgpu::Backend::Dx12), Some("0")));
        assert!(affected(&adapter("Intel(R) Iris(R) Xe Graphics", wgpu::Backend::Dx12), Some("nonsense")));
    }

    #[test]
    fn transparent_texels_upload_as_near_clear_and_nothing_else_changes() {
        // A stroke-strip-like image: mostly transparent, a grey texel and a faint one.
        let grey = Color32::from_gray(200);
        let faint = Color32::from_rgba_premultiplied(1, 1, 1, 2);
        let mut px = vec![Color32::TRANSPARENT; 16];
        px[5] = grey;
        px[6] = faint;
        let strip = Arc::new(ColorImage::new([4, 4], px));
        let opaque = Arc::new(ColorImage::filled([2, 2], grey));
        let mut delta = TexturesDelta::default();
        delta.set.insert(TextureId::Managed(1), [ImageDelta::full(ImageData::Color(strip.clone()), TextureOptions::LINEAR)].into_iter().collect());
        delta.set.insert(TextureId::Managed(2), [ImageDelta::full(ImageData::Color(opaque.clone()), TextureOptions::LINEAR)].into_iter().collect());
        // A partial update (the font atlas grows this way) is guarded too.
        let patch = Arc::new(ColorImage::new([2, 1], vec![Color32::TRANSPARENT, grey]));
        delta.set.insert(TextureId::Managed(0), [ImageDelta::partial([3, 4], ImageData::Color(patch), TextureOptions::LINEAR)].into_iter().collect());

        assert_eq!(guard(&mut delta), 14 + 1);
        let image = |id| {
            let ImageData::Color(i) = &delta.set[&TextureId::Managed(id)][0].image;
            i.clone()
        };
        let s = image(1);
        assert!(!s.pixels.contains(&Color32::TRANSPARENT));
        assert_eq!((s.pixels[5], s.pixels[6], s.pixels[0]), (grey, faint, NEAR_CLEAR));
        // The caller's image is untouched (copy on write), and an image with nothing to change is
        // not copied at all.
        assert!(strip.pixels.contains(&Color32::TRANSPARENT));
        assert!(Arc::ptr_eq(&image(2), &opaque));
        assert_eq!(image(0).pixels, vec![NEAR_CLEAR, grey]);
        // Idempotent: a second pass finds nothing.
        assert_eq!(guard(&mut delta), 0);
        delta.clear();
    }

    #[test]
    fn near_clear_is_premultiplied_and_invisible_over_any_background() {
        // Premultiplied blend: out = src + dst × (1 − a). Over every 8-bit level the result moves
        // by at most one level.
        let a = f32::from(NEAR_CLEAR.a()) / 255.0;
        assert!(NEAR_CLEAR.r() <= NEAR_CLEAR.a() && NEAR_CLEAR.g() <= NEAR_CLEAR.a() && NEAR_CLEAR.b() <= NEAR_CLEAR.a());
        for dst in 0..=255u8 {
            let out = (f32::from(NEAR_CLEAR.r()) + f32::from(dst) * (1.0 - a)).round();
            assert!((f32::from(dst) - out).abs() <= 1.0, "{dst} -> {out}");
        }
    }

    #[test]
    fn the_plugin_guards_a_frames_uploads() {
        let ctx = egui::Context::default();
        ctx.add_plugin(TexelGuard);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let img = ColorImage::new([8, 8], vec![Color32::TRANSPARENT; 64]);
            let tex = ui.ctx().load_texture("texel-guard-test", img, TextureOptions::LINEAR);
            ui.image((tex.id(), egui::vec2(8.0, 8.0)));
        });
        let uploaded: Vec<_> = out
            .textures_delta
            .set
            .values()
            .flatten()
            .filter_map(|d| {
                let ImageData::Color(i) = &d.image;
                (i.size == [8, 8]).then(|| i.clone())
            })
            .collect();
        assert_eq!(uploaded.len(), 1);
        assert!(uploaded[0].pixels.iter().all(|p| *p == NEAR_CLEAR));
        out.textures_delta.clear();
    }

    #[test]
    fn install_adds_the_plugin_once_and_only_where_needed() {
        if std::env::var_os("PHOTOCRAFT_TEXEL_GUARD").is_some() {
            return;
        }
        let ctx = egui::Context::default();
        assert!(!install(&ctx, &adapter("Apple M2", wgpu::Backend::Metal)));
        assert!(ctx.plugin_opt::<TexelGuard>().is_none());
        let iris = adapter("Intel(R) Iris(R) Xe Graphics", wgpu::Backend::Dx12);
        assert!(install(&ctx, &iris) && install(&ctx, &iris));
        assert!(ctx.plugin_opt::<TexelGuard>().is_some());
    }
}
