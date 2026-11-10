//! Noninteractive RAW opening for Batch, using the same developer and workflow as Camera Raw.

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_io::{ImportResult, raw::RawTuning};

use super::{EngineError, Result, Session, Value, json};

/// The prior settings of a sidecar, and its crop (in the raw's stored orientation).
type Prior = (CameraRaw, Option<photocraft_io::camera_raw_xmp::Crop>);

/// Read only a bounded regular XMP file. No ambient sidecar reads are granted to gated sessions.
#[cfg(not(target_arch = "wasm32"))]
fn prior_settings(path: Option<&str>) -> Result<Option<Prior>> {
    use std::io::Read;
    let Some(path) = path else { return Ok(None) };
    let sidecar = match path.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains(['/', '\\']) => format!("{stem}.xmp"),
        _ => format!("{path}.xmp"),
    };
    let meta = match std::fs::metadata(&sidecar) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(EngineError::Other(format!("{sidecar}: {error}"))),
    };
    let cap = photocraft_io::camera_raw_xmp::MAX_XMP_BYTES;
    if !meta.is_file() || meta.len() > cap as u64 {
        return Err(EngineError::Other(format!("{sidecar}: RAW settings must be a regular file of at most {cap} bytes")));
    }
    let file = std::fs::File::open(&sidecar).map_err(|error| EngineError::Other(format!("{sidecar}: {error}")))?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes).map_err(|error| EngineError::Other(format!("{sidecar}: {error}")))?;
    let mut settings = CameraRaw::default();
    let report = photocraft_io::camera_raw_xmp::load(&mut settings, &bytes).map_err(|error| EngineError::Other(format!("{sidecar}: {error}")))?;
    if !report.ignored.is_empty() {
        return Err(EngineError::Other(format!("{sidecar}: unsupported prior RAW settings: {}", report.ignored.join(", "))));
    }
    Ok(Some((settings, report.crop)))
}

#[cfg(target_arch = "wasm32")]
fn prior_settings(path: Option<&str>) -> Result<Option<Prior>> {
    if path.is_some() { Err(EngineError::Other("cannot read RAW sidecars on the web".into())) } else { Ok(None) }
}

/// Sample the existing content at the Camera Raw histogram spacing, without a full proxy image.
fn auto_sample(surface: &photocraft_raster::Surface) -> Vec<[f32; 4]> {
    let area = surface.content_bounds();
    let (w, h) = (area.x1.saturating_sub(area.x0).max(0) as usize, area.y1.saturating_sub(area.y0).max(0) as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let k = w.max(h).div_ceil(512).max(1);
    let mut out = Vec::with_capacity(w.div_ceil(k) * h.div_ceil(k));
    let mut row = vec![[0.0; 4]; w];
    for y in (0..h).step_by(k) {
        let yy = area.y0.saturating_add(y as i32);
        surface.read_rgba_into(photocraft_geom::Rect::new(area.x0, yy, area.x1, yy.saturating_add(1)), &mut row);
        out.extend(row.iter().step_by(k).copied());
    }
    out
}

/// Develop representable prior settings at the sensor stage, retaining only the remaining RGB work.
pub(super) fn prepare(s: &Session, name: &str, path: Option<&str>, bytes: &[u8]) -> Result<Option<(ImportResult, CameraRaw)>> {
    let Some(defaults) = &s.batch_raw_open else { return Ok(None) };
    if !photocraft_io::raw::is_raw(bytes) {
        return Ok(None);
    }
    if s.authorize.is_some() || crate::jobs::authorization_installed() {
        return Err(EngineError::Other("RAW sidecar opening requires a trusted session; no implicit read capability is available".into()));
    }
    let prior = prior_settings(path)?;
    let crop = prior.as_ref().and_then(|(_, crop)| *crop);
    let prior = prior.map(|(settings, _)| settings);
    let mut settings = prior.clone().unwrap_or_default();
    let opts = crate::jobs::import_options(s.prefs());
    let mut imported = photocraft_io::import_with_options(name, bytes, &opts).map_err(|error| EngineError::Other(format!("{name}: {error}")))?;
    if imported.preview_only || !imported.warnings.iter().any(|warning| warning.contains(photocraft_io::raw::DEVELOPED_NOTE)) {
        return Err(EngineError::Other(format!("{name}: complete RAW development is required")));
    }
    if prior.is_none() && defaults.apply_auto_tone {
        let surface = imported.document.layers.iter().find_map(|layer| layer.surface()).ok_or(EngineError::NoDocument)?;
        photocraft_algo::camera_raw::auto_tone(&auto_sample(surface), &mut settings);
    }
    let tuning = RawTuning {
        temperature: settings.temperature,
        tint: settings.tint,
        exposure: settings.exposure,
        demosaic: defaults.demosaic.method(),
        shadows: settings.shadows,
        highlights: settings.highlights,
        clarity: settings.clarity,
        texture: settings.texture,
        dehaze: settings.dehaze,
        // The sidecar's crop, as the interactive open applies it: before the Camera Raw step.
        crop,
    };
    if tuning != (RawTuning { demosaic: opts.demosaic, ..Default::default() }) {
        super::open::raw_defaults(s, &imported.warnings, &mut imported.document);
        let (mut developed, wb) = photocraft_io::raw::import_raw_tuned(name, bytes, &tuning).map_err(|error| EngineError::Other(format!("{name}: {error}")))?;
        developed.document.resolution_dpi = imported.document.resolution_dpi;
        imported = developed;
        settings.exposure = 0.0;
        settings.shadows = 0.0;
        settings.highlights = 0.0;
        settings.clarity = 0.0;
        settings.texture = 0.0;
        settings.dehaze = 0.0;
        if wb {
            settings.temperature = 0.0;
            settings.tint = 0.0;
        }
    }
    if let Some((amount, radius)) = defaults.sharpen_for.preset()
        && settings.sharpen_amount == 0.0
    {
        settings.sharpen_amount = amount;
        settings.sharpen_radius = radius;
    }
    Ok(Some((imported, settings)))
}

/// Finish the same RGB filter and workflow conversions used by interactive Camera Raw opening.
pub(super) fn finish(s: &mut Session, settings: CameraRaw) -> Result<()> {
    let defaults = s.batch_raw_open.clone().ok_or_else(|| EngineError::Other("RAW opening settings are missing".into()))?;
    let full = serde_json::to_value(&settings).map_err(|error| EngineError::Other(error.to_string()))?;
    let empty = serde_json::to_value(CameraRaw::default()).map_err(|error| EngineError::Other(error.to_string()))?;
    let mut params = serde_json::Map::new();
    if let (Some(full), Some(empty)) = (full.as_object(), empty.as_object()) {
        for (key, value) in full {
            if key != "pixelScale" && empty.get(key) != Some(value) {
                params.insert(key.clone(), value.clone());
            }
        }
    }
    if !params.is_empty() {
        s.execute("filter.cameraRaw", Value::Object(params))?;
    }
    if defaults.color_space != crate::prefs::RawColorSpace::ProPhoto {
        s.execute("edit.convertToProfile", json!({"profile":defaults.color_space.profile_id(),"intent":"relative"}))?;
    }
    if defaults.bit_depth == crate::prefs::RawDepth::Eight {
        s.execute("image.mode.bits8", json!({}))?;
    }
    if defaults.open_as_smart_object {
        s.execute("layer.smartObjects.convertToSmartObject", json!({}))?;
    }
    Ok(())
}
