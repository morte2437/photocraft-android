//! Paint.NET PDN3 import. File parsing and pixel decompression live in `craft-pdn`;
//! this module maps its neutral document structures into PhotoCraft's layer model.

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use crate::{ImportResult, IoError};

#[cfg(test)]
mod tests;

const MAX_PIXEL_BYTES: usize = 1 << 30;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Pdn(message.into())
}

fn blend(mode: craft_pdn::BlendMode) -> BlendMode {
    use craft_pdn::BlendMode as P;
    match mode {
        P::Normal => BlendMode::Normal,
        P::Multiply => BlendMode::Multiply,
        P::Additive => BlendMode::LinearDodge,
        P::ColorBurn => BlendMode::PaintNetColorBurn,
        P::ColorDodge => BlendMode::PaintNetColorDodge,
        P::Reflect => BlendMode::Reflect,
        P::Glow => BlendMode::Glow,
        P::Overlay => BlendMode::Overlay,
        P::Difference => BlendMode::Difference,
        P::Negation => BlendMode::Negation,
        P::Lighten => BlendMode::Lighten,
        P::Darken => BlendMode::Darken,
        P::Screen => BlendMode::Screen,
        P::Xor => BlendMode::Xor,
    }
}

pub(super) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult, IoError> {
    let keep_going = || ctl.check().is_ok();
    let parsed = craft_pdn::Document::parse(bytes, &keep_going).map_err(|e| match e {
        craft_pdn::Error::Cancelled => IoError::Cancelled,
        other => invalid(other.to_string()),
    })?;
    let size = Size::new(parsed.width as u32, parsed.height as u32);
    let format = PixelFormat::RGBA8;
    let raster_bytes = usize::try_from(photocraft_raster::tile_storage_bytes(Rect::from_xywh(0, 0, size.width, size.height), &format)).unwrap_or(usize::MAX);
    let total = parsed.decoded_bytes().saturating_add(raster_bytes.saturating_mul(parsed.surface_count()));
    if total > MAX_PIXEL_BYTES {
        return Err(invalid("decoded pixels exceed the 1 GiB limit"));
    }

    let mut surfaces = Vec::with_capacity(parsed.surface_count());
    let mut read_result = Ok(());
    let mut each = |index: usize, mut data: craft_pdn::Surface| {
        if read_result.is_err() {
            return;
        }
        if ctl.check().is_err() {
            read_result = Err(IoError::Cancelled);
            return;
        }
        let mut surface = Surface::new(format);
        for (y, row) in data.data.chunks_exact_mut(data.stride.max(1)).enumerate() {
            let Some(row) = row.get_mut(..data.width.saturating_mul(4)) else {
                read_result = Err(invalid("invalid pixel row"));
                return;
            };
            for pixel in row.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            surface.write_interleaved(Rect::from_xywh(0, y as i32, data.width as u32, 1), row);
        }
        surfaces.push((index, surface));
        ctl.progress(0.05 + 0.9 * surfaces.len() as f32 / parsed.surface_count().max(1) as f32);
    };
    parsed.read_surfaces(&keep_going, &mut each).map_err(|e| match e {
        craft_pdn::Error::Cancelled => IoError::Cancelled,
        other => invalid(other.to_string()),
    })?;
    read_result?;

    let mut document = Document::new(name, size, ColorMode::Rgb, SampleType::U8);
    for source in parsed.layers {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let surface = surfaces
            .get(source.surface)
            .filter(|(index, _)| *index == source.surface)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| invalid("missing pixel memory block"))?;
        let mut layer = Layer::raster(source.name, format);
        layer.visible = source.visible;
        layer.opacity = f32::from(source.opacity) / 255.0;
        layer.blend = blend(source.blend);
        layer.content = LayerContent::Raster(surface);
        document.layers.push(layer);
    }

    let warnings = if parsed.has_user_metadata { vec!["Paint.NET document metadata was not imported".into()] } else { Vec::new() };
    Ok(ImportResult { document, warnings, source_read_only: true, preview_only: false })
}
