//! One image owns the bucket artwork and its lower-right fill target. Splitting those between
//! the frame and the OS pointer lets them drift apart whenever a frame arrives after the pointer.

use super::{CursorIcon, CustomCursorImage, Painter, Pos2, native, native_scale};
use std::sync::Arc;

const SIZE: f32 = 22.0;
const HOT: egui::Vec2 = egui::vec2(26.0 / 24.0 * SIZE, 25.0 / 24.0 * SIZE);

#[derive(Clone)]
struct Cached {
    scale: f32,
    image: Result<CustomCursorImage, String>,
    texture: Option<egui::TextureHandle>,
}

pub(super) fn show(painter: &Painter, at: Pos2) -> Result<CursorIcon, String> {
    let ctx = painter.ctx();
    let native = native(ctx);
    let scale = if native { native_scale(ctx) } else { ctx.pixels_per_point() };
    let id = egui::Id::unique("photocraft-bucket-cursor");
    let mut cached = ctx.data(|d| d.get_temp::<Cached>(id)).filter(|cached| cached.scale == scale).unwrap_or_else(|| Cached {
        scale,
        // Keep diagnostics in the log once per cached failure; the error also reaches UI text.
        image: rasterize(scale).inspect_err(|error| log::error!("{error}\n{}", std::backtrace::Backtrace::force_capture())),
        texture: None,
    });
    let result = match &cached.image {
        Err(error) => Err(error.clone()),
        Ok(image) => {
            if native {
                ctx.set_cursor_image(Some(image.clone()));
            } else {
                let texture = cached.texture.get_or_insert_with(|| {
                    let size = image.size.map(usize::from);
                    ctx.load_texture("Paint Bucket cursor", egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba), egui::TextureOptions::LINEAR)
                });
                let size = egui::vec2(f32::from(image.size[0]), f32::from(image.size[1])) / scale;
                // A texture's texel centre is half a physical pixel into its cell. Keep that
                // centre exactly on the pointer, just as the native bitmap hotspot is.
                let hot = (egui::vec2(f32::from(image.hotspot[0]), f32::from(image.hotspot[1])) + egui::Vec2::splat(0.5)) / scale;
                let rect = egui::Rect::from_min_size(at - hot, size);
                egui::Area::new(egui::Id::unique("pc-tool-cursor")).order(egui::Order::Tooltip).fixed_pos(rect.min).constrain(false).interactable(false).show(
                    ctx,
                    |ui| {
                        ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    },
                );
            }
            Ok(CursorIcon::None)
        }
    };
    ctx.data_mut(|d| d.insert_temp(id, cached));
    result
}

pub(super) fn rasterize(scale: f32) -> Result<CustomCursorImage, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!("Invalid Paint Bucket cursor scale: {scale}"));
    }
    // Round the hotspot itself, rather than the box, so it always lands on an exact bitmap pixel.
    // Keep one logical pixel of the existing icon halo and its antialiasing fringe inside the box.
    let margin = (scale + 1.0).ceil();
    let hot = [(HOT.x * scale).round() + margin, (HOT.y * scale).round() + margin];
    let cross = super::rasterize(&super::STANDARD_CROSSHAIR, scale).ok_or_else(|| format!("Paint Bucket crosshair does not fit a cursor at scale {scale}"))?;
    let size = [hot[0] + f32::from(cross.size[0] - cross.hotspot[0]), hot[1] + f32::from(cross.size[1] - cross.hotspot[1])];
    if size.iter().any(|side| !side.is_finite() || *side > f32::from(super::MAX_SIDE)) {
        return Err(format!("Paint Bucket cursor does not fit the platform bitmap limit at scale {scale}"));
    }
    let size = size.map(|side| side as u16);
    let hotspot = hot.map(|coord| coord as u16);
    let mut pixels = vec![egui::Color32::TRANSPARENT; usize::from(size[0]) * usize::from(size[1])];
    let icon = crate::icons::rasterize("paint-bucket", (SIZE * scale).round() as u32)?;
    let margin = margin as i32;
    let halo = scale.round() as i32;
    // The same eight dark halo offsets and white SVG as icons::cursor, now in the image that
    // also owns the crosshair. Color32 keeps composition premultiplied until the final upload.
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1), (0, 0)] {
        for (i, pixel) in icon.pixels.iter().enumerate() {
            let x = (i % icon.size[0]) as i32 + margin + dx * halo;
            let y = (i / icon.size[0]) as i32 + margin + dy * halo;
            let color = if dx == 0 && dy == 0 { *pixel } else { egui::Color32::from_black_alpha((f32::from(pixel.a()) * 200.0 / 255.0).round() as u8) };
            over(&mut pixels, size, x, y, color);
        }
    }
    for (i, pixel) in cross.rgba.as_chunks::<4>().0.iter().enumerate() {
        let x = (i % usize::from(cross.size[0])) as i32 + i32::from(hotspot[0]) - i32::from(cross.hotspot[0]);
        let y = (i / usize::from(cross.size[0])) as i32 + i32::from(hotspot[1]) - i32::from(cross.hotspot[1]);
        over(&mut pixels, size, x, y, egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3]));
    }
    let rgba = pixels.into_iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect::<Vec<_>>();
    Ok(CustomCursorImage { rgba: Arc::from(rgba), size, hotspot })
}

fn over(pixels: &mut [egui::Color32], size: [u16; 2], x: i32, y: i32, foreground: egui::Color32) {
    if x < 0 || y < 0 || x >= i32::from(size[0]) || y >= i32::from(size[1]) {
        return;
    }
    if let Some(background) = pixels.get_mut(y as usize * usize::from(size[0]) + x as usize) {
        *background = background.blend(foreground);
    }
}
