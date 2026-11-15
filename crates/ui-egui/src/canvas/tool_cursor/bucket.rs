//! One image owns the bucket artwork and the arrow at its top left whose tip is the fill target
//! (#12539). Splitting those between the frame and the OS pointer lets them drift apart whenever a
//! frame arrives after the pointer.

use super::{CursorIcon, CustomCursorImage, Painter, Pos2, native, native_scale};
use std::sync::Arc;

const SIZE: f32 = 22.0;
/// The bucket icon's top-left corner, in points from the arrow tip.
const ICON: egui::Vec2 = egui::vec2(6.0, 6.0);
/// Our own small arrow, in points from its tip: the hotspot, so a click fills the pixel under it.
const ARROW: [egui::Vec2; 7] = [
    egui::vec2(0.0, 0.0),
    egui::vec2(0.0, 10.5),
    egui::vec2(2.6, 8.2),
    egui::vec2(4.3, 11.8),
    egui::vec2(5.9, 11.1),
    egui::vec2(4.2, 7.5),
    egui::vec2(7.5, 7.5),
];

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
    // The arrow's halo reaches one halo width above and left of its tip, so the tip lands on
    // (1, 1) at 1x, as in the reference cursor. The bucket's halo and fringe get room below right.
    let halo = scale.round();
    let side = (SIZE * scale).round();
    let icon_at = (ICON * scale).round() + egui::Vec2::splat(halo);
    let size = [icon_at.x + side + halo + 1.0, icon_at.y + side + halo + 1.0];
    if size.iter().any(|side| !side.is_finite() || *side > f32::from(super::MAX_SIDE)) {
        return Err(format!("Paint Bucket cursor does not fit the platform bitmap limit at scale {scale}"));
    }
    let size = size.map(|side| side as u16);
    let hotspot = [halo as u16; 2];
    let mut pixels = vec![egui::Color32::TRANSPARENT; usize::from(size[0]) * usize::from(size[1])];
    let icon = crate::icons::rasterize("paint-bucket", side as u32)?;
    let halo = halo as i32;
    let (arrow_width, arrow) = arrow_mask(scale);
    // The same eight dark halo offsets and white SVG as icons::cursor, then the arrow on top of
    // the bucket: black on a white halo. Color32 keeps composition premultiplied until the upload.
    const OFFSETS: [(i32, i32); 9] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1), (0, 0)];
    for (dx, dy) in OFFSETS {
        for (i, pixel) in icon.pixels.iter().enumerate() {
            let x = (i % icon.size[0]) as i32 + icon_at.x as i32 + dx * halo;
            let y = (i / icon.size[0]) as i32 + icon_at.y as i32 + dy * halo;
            let color = if dx == 0 && dy == 0 { *pixel } else { egui::Color32::from_black_alpha((f32::from(pixel.a()) * 200.0 / 255.0).round() as u8) };
            over(&mut pixels, size, x, y, color);
        }
    }
    for (dx, dy) in OFFSETS {
        for (i, alpha) in arrow.iter().enumerate() {
            let x = (i % arrow_width) as i32 + i32::from(hotspot[0]) + dx * halo;
            let y = (i / arrow_width) as i32 + i32::from(hotspot[1]) + dy * halo;
            let color = if dx == 0 && dy == 0 { egui::Color32::from_black_alpha(*alpha) } else { egui::Color32::from_white_alpha(*alpha) };
            over(&mut pixels, size, x, y, color);
        }
    }
    let rgba = pixels.into_iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect::<Vec<_>>();
    Ok(CustomCursorImage { rgba: Arc::from(rgba), size, hotspot })
}

/// The arrow's coverage per physical pixel, its tip at the centre of pixel (0, 0), the texel
/// centre the painted fallback puts on the pointer: the mask's width and its alpha values, row by row.
fn arrow_mask(scale: f32) -> (usize, Vec<u8>) {
    const SAMPLES: u16 = 4;
    let extent = ARROW.iter().fold(egui::Vec2::ZERO, |extent, point| extent.max(*point)) * scale + egui::Vec2::splat(0.5);
    let (width, height) = (extent.x.ceil() as usize, extent.y.ceil() as usize);
    let mut mask = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let mut hits = 0_u16;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let sample = egui::vec2(x as f32 + (f32::from(sx) + 0.5) / f32::from(SAMPLES), y as f32 + (f32::from(sy) + 0.5) / f32::from(SAMPLES));
                    hits += u16::from(inside((sample - egui::Vec2::splat(0.5)) / scale, &ARROW));
                }
            }
            mask.push((f32::from(hits) * 255.0 / f32::from(SAMPLES * SAMPLES)).round() as u8);
        }
    }
    (width, mask)
}

/// Even-odd point in polygon.
fn inside(point: egui::Vec2, polygon: &[egui::Vec2]) -> bool {
    let next = polygon.iter().skip(1).chain(polygon.first());
    polygon.iter().zip(next).fold(false, |inside, (a, b)| {
        let crosses = (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
        inside != crosses
    })
}

fn over(pixels: &mut [egui::Color32], size: [u16; 2], x: i32, y: i32, foreground: egui::Color32) {
    if x < 0 || y < 0 || x >= i32::from(size[0]) || y >= i32::from(size[1]) {
        return;
    }
    if let Some(background) = pixels.get_mut(y as usize * usize::from(size[0]) + x as usize) {
        *background = background.blend(foreground);
    }
}
