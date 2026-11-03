//! Native OpenEXR blocks go straight into original-depth tiled storage.
use crate::{ImportResult, IoError};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};
use std::io::{Read, Seek};

/// Tiles the decoded image would add must fit the RAM tile budget when no scratch disk can
/// take cold tiles.
fn ram_tile_capacity(resident: u64, budget: u64, growth: u64) -> Result<(), IoError> {
    if resident.checked_add(growth).is_none_or(|required| required > budget) {
        return Err(IoError::OutOfMemory("decoded image tiles exceed the available RAM budget; enable a scratch disk or close other documents".into()));
    }
    Ok(())
}

/// A scratch disk that was on when the import started must still be on: its tiles may
/// already have moved there.
fn require_scratch(expected: bool) -> Result<(), IoError> {
    if expected && !photocraft_raster::spill::enabled() {
        return Err(IoError::Read(
            photocraft_raster::spill::last_error().unwrap_or_else(|| "scratch storage was disabled while loading; retry the import".into()),
        ));
    }
    Ok(())
}

/// Decode a single-part flat scanline EXR from `source` one compression block at a time.
/// `Ok(None)`: the header is not one the band decoder takes (multi-part, deep, tiled, custom
/// chromaticities, over its block limits or malformed by its stricter checks); the caller
/// falls back to the whole-file decoder. Once pixels are being decoded any failure fails the
/// open, so no partial document is published.
pub(crate) fn import<R: Read + Seek>(source: R, ctl: &Interrupt<'_>) -> Result<Option<ImportResult>, IoError> {
    let _metadata = photocraft_raster::memory::try_reserve(32 << 20).map_err(IoError::OutOfMemory)?;
    let mut decoder = match photocraft_codecs::exr_stream::Decoder::new(source, &|| ctl.cancelled()) {
        Ok(decoder) => decoder,
        Err(_) if ctl.cancelled() => return Err(IoError::Cancelled),
        Err(_) => return Ok(None),
    };
    let info = decoder.info();
    let mode = if info.gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let fmt = PixelFormat::new(mode, SampleType::F32, true);
    let scratch_expected = photocraft_raster::spill::enabled();
    if !scratch_expected {
        let budget = photocraft_raster::memory::refresh();
        let side = TILE_SIZE as u64;
        let tiles = u64::from(info.width).div_ceil(side).saturating_mul(u64::from(info.height).div_ceil(side));
        ram_tile_capacity(
            photocraft_raster::spill::stats().resident_bytes as u64,
            budget.tile_bytes,
            tiles.saturating_mul(side * side).saturating_mul(fmt.bytes_per_pixel() as u64),
        )?;
    }
    let _working = photocraft_raster::memory::try_reserve(info.working_bytes).map_err(IoError::OutOfMemory)?;
    let mut surface = Surface::new(fmt);
    let mut done = 0;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        require_scratch(scratch_expected)?;
        let band = decoder.next_band(&|| ctl.cancelled()).map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { e.into() })?;
        let Some(band) = band else {
            break;
        };
        let rect = Rect::from_xywh(band.x as i32, band.y as i32, band.width, band.height);
        if !photocraft_raster::spill::enabled() {
            let missing = rect.tiles().filter(|c| surface.tile(*c).is_none()).count() as u64;
            ram_tile_capacity(
                photocraft_raster::spill::stats().resident_bytes as u64,
                photocraft_raster::memory::stats().1.tile_bytes,
                missing.saturating_mul((TILE_SIZE as u64).pow(2)).saturating_mul(fmt.bytes_per_pixel() as u64),
            )?;
        }
        surface.write_interleaved(rect, &band.data);
        require_scratch(scratch_expected)?;
        done += 1;
        ctl.progress(0.05 + 0.9 * done as f32 / info.blocks.max(1) as f32);
    }
    // Missing alpha is filled with one in every allocated tile. Pruning these opaque
    // tiles cannot remove any and would unnecessarily reload the entire scratch store.
    if info.has_alpha {
        surface.prune();
    }
    require_scratch(scratch_expected)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut document = Document::new("", Size::new(info.width, info.height), mode, SampleType::F32);
    let mut layer = Layer::new("Background", LayerContent::Raster(surface));
    if !info.has_alpha {
        layer.locks.transparency = true;
        layer.locks.position = true;
    }
    document.layers.push(layer);
    document.icc_profile =
        Some(if mode == ColorMode::Rgb { photocraft_cms::Builtin::LinearSrgb.profile().to_bytes() } else { photocraft_cms::builtin::linear_gray().to_bytes() });
    let warnings = if info.all_half { vec!["16-bit float samples are stored as 32-bit float".into()] } else { Vec::new() };
    ctl.progress(1.0);
    Ok(Some(ImportResult { document, warnings, source_read_only: false, preview_only: false }))
}
