//! Source-backed Paste modes. Preparation and geometry conversion adapt Skyfuller1303's
//! PR #3409, commit 897f20072762fdf0eb688adcdb8d8cda7b38ce66, to the current clipboard API.

use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, NamedPath, Path};
use photocraft_geom::Affine;
use serde_json::{Value, json};

use super::edit_cmds::Clip;
use crate::{EngineError, Result, Session};

const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE_PIXELS: u64 = 24_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Pixels,
    Path,
    Shape,
}

fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "edit.paste".into(), msg: message.into() }
}

/// Omission alone selects the old command routes; malformed explicit modes must not disappear.
pub(super) fn mode(p: &Value) -> Result<Option<Mode>> {
    match p.get("as") {
        None => Ok(None),
        Some(Value::String(value)) => match value.as_str() {
            "pixels" => Ok(Some(Mode::Pixels)),
            "path" => Ok(Some(Mode::Path)),
            "shape" => Ok(Some(Mode::Shape)),
            _ => Err(bad("`as` must be pixels, path or shape")),
        },
        Some(_) => Err(bad("`as` must be pixels, path or shape")),
    }
}

/// Imported data is temporary; only original bytes and their raster mirror survive in Clip.
fn prepare(bytes: &[u8]) -> Result<(Document, Vec<String>)> {
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_BYTES || !photocraft_io::svg::is_svg(bytes) {
        return Err(bad("SVG clipboard source must be valid markup between 1 byte and 16 MiB"));
    }
    let mut imported = photocraft_io::svg::import_svg_bounded("Clipboard.svg", bytes, MAX_CACHE_PIXELS)
        .map_err(|e| EngineError::Other(format!("can't import clipboard SVG: {e}")))?;
    if imported.document.layers.is_empty() {
        return Err(bad("the SVG contains no drawable objects"));
    }
    imported.document.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    Ok((imported.document, imported.warnings))
}

pub(super) fn from_svg(bytes: &[u8]) -> Result<Clip> {
    let _ = prepare(bytes)?;
    let image = crate::smart_cmds::svg_source_image(bytes, PixelFormat::RGBA32F, 1.0)?;
    let mut owned = crate::allocation::capacity(bytes.len(), "copying SVG clipboard source")?;
    owned.extend_from_slice(bytes);
    Ok(Clip {
        svg: Some(Arc::new(owned)),
        components: None,
        surface: (*image.surface).clone(),
        bounds: image.bounds,
        layers: None,
        resolution: None,
        source: None,
    })
}

/// Explicit modes do not fall through to new-document, mask, channel or in-place operations.
pub(super) fn validate(s: &Session, p: &Value, in_place: bool) -> Result<[f64; 2]> {
    if in_place || ["inPlace", "clampToCanvas", "document", "layer", "layers", "path"].iter().any(|key| p.get(key).is_some()) {
        return Err(bad("explicit Paste modes do not support these placement or target options"));
    }
    if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) {
        return Err(bad("explicit Paste requires the layer composite target"));
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    if s.tools.mask_targeted
        || state.doc.quick_mask.is_some()
        || state.channel_view.layer_mask.is_some()
        || state.channel_view.target != crate::channel_cmds::ChannelTarget::Composite
        || !crate::layer_copy_cmds::layered(state.doc.mode)
    {
        return Err(bad("explicit Paste requires a layered document and the layer composite target"));
    }
    if s.job_on(state.doc.id).is_some() {
        return Err(bad("the destination has a running background job"));
    }
    match p.get("center") {
        None => Ok([f64::from(state.doc.size.width) / 2.0, f64::from(state.doc.size.height) / 2.0]),
        Some(Value::Array(a)) if a.len() == 2 => {
            let x = a.first().and_then(Value::as_f64);
            let y = a.get(1).and_then(Value::as_f64);
            match (x, y) {
                (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Ok([x, y]),
                _ => Err(bad("`center` must contain two finite coordinates within ±10 million pixels")),
            }
        }
        Some(_) => Err(bad("`center` must contain two finite coordinates within ±10 million pixels")),
    }
}

/// Extracting a child must not discard a clipping mask or a group's nonneutral composition.
fn neutral_group(layer: &Layer) -> bool {
    layer.opacity == 1.0
        && layer.fill_opacity == 1.0
        && layer.mask.is_none()
        && layer.vector_mask.is_none()
        && layer.effects.items.is_empty()
        && !layer.clipped
        && layer.advanced.is_default()
        && layer.blend_if.is_default()
        && layer.excluded_channels == 0
        && matches!(layer.blend, BlendMode::Normal | BlendMode::PassThrough)
}

fn paths(doc: &Document) -> Result<Path> {
    let mut result: Option<Path> = None;
    for (_, _, layer) in doc.walk() {
        match &layer.content {
            LayerContent::Group(_) if neutral_group(layer) => {}
            LayerContent::Shape(shape) if layer.mask.is_none() && layer.vector_mask.is_none() && !layer.clipped => {
                if let Some(path) = &mut result {
                    if path.fill_rule != shape.path.fill_rule || path.inverted != shape.path.inverted {
                        return Err(bad("the SVG paths have incompatible fill rules"));
                    }
                    path.subpaths.extend(shape.path.subpaths.clone());
                } else {
                    result = Some(shape.path.clone());
                }
            }
            _ => return Err(bad("the SVG contains rasterized or clipped objects without a supported editable path")),
        }
    }
    result.filter(|p| !p.is_empty()).ok_or_else(|| bad("the SVG contains no editable path"))
}

fn shape(doc: &Document) -> Result<&Layer> {
    let mut found = None;
    for (_, _, layer) in doc.walk() {
        match &layer.content {
            LayerContent::Group(_) if neutral_group(layer) => {}
            LayerContent::Shape(_) if found.is_none() && layer.mask.is_none() && layer.vector_mask.is_none() && !layer.clipped => found = Some(layer),
            _ => return Err(bad("Shape requires one editable shape without clipping or group blending")),
        }
    }
    found.ok_or_else(|| bad("the SVG contains no editable shape"))
}

/// Preflight only the destination caches that adaptation/translation will actually render.
fn shape_budget(layer: &Layer, destination: &Document, dx: i32, dy: i32) -> Result<()> {
    let LayerContent::Shape(shape) = &layer.content else {
        return Err(bad("the source is not an editable shape"));
    };
    // Adaptation can refresh at the source position before translation refreshes the final cache.
    for transform in [Affine::IDENTITY, Affine::translate(f64::from(dx), f64::from(dy))] {
        let mut shape = shape.clone();
        crate::vector_cmds::transform_shape(&mut shape, &transform);
        photocraft_vector::flatten::validate_shape(&shape.path, shape.stroke.as_ref()).map_err(bad)?;
        let compiled = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, destination.bounds());
        if compiled.bounds().is_some_and(|r| r.intersect(&destination.bounds()).size().area() > MAX_CACHE_PIXELS) {
            return Err(bad("the pasted shape cache exceeds 24 megapixels; use Pixels"));
        }
    }
    Ok(())
}

/// Editable insertions may bypass a locked active group, but cannot modify a locked ancestor.
pub(super) fn layer_target(s: &Session) -> Result<()> {
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let parent = state
        .active_layer
        .and_then(|id| state.doc.path_of(id))
        .map(|mut path| {
            path.pop();
            path
        })
        .unwrap_or_default();
    let locks = state.doc.locks_at(&parent);
    if locks.all || locks.pixels || locks.position || locks.transparency {
        return Err(bad("the destination group is locked"));
    }
    Ok(())
}

/// Prepare all fallible work before a single insertion, retaining the original clipboard source.
pub(super) fn paste(s: &mut Session, p: &Value, mode: Mode) -> Result<Value> {
    let center = validate(s, p, false)?;
    let bytes = s.clipboard.as_ref().and_then(|clip| clip.svg.clone()).ok_or_else(|| bad("this Paste mode requires original vector clipboard source"))?;
    let (mut copies, warnings) = prepare(&bytes)?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let destination = state.doc.clone();
    let (dx, dy) = ((center[0] - f64::from(copies.size.width) / 2.0).round(), (center[1] - f64::from(copies.size.height) / 2.0).round());
    if !dx.is_finite() || !dy.is_finite() || dx < f64::from(i32::MIN) || dx > f64::from(i32::MAX) || dy < f64::from(i32::MIN) || dy > f64::from(i32::MAX) {
        return Err(bad("the paste offset is not addressable"));
    }
    let (dx, dy) = (dx as i32, dy as i32);
    if mode == Mode::Path {
        let path = paths(&copies)?.transform(&Affine::translate(f64::from(dx), f64::from(dy)));
        photocraft_vector::flatten::validate_path(&path).map_err(bad)?;
        let name = (1..=destination.paths.len().saturating_add(1))
            .map(|i| format!("Pasted Path {i}"))
            .find(|name| !destination.paths.iter().any(|p| &p.name == name))
            .ok_or_else(|| bad("no free path name"))?;
        s.edit("Paste Path", |doc, _| {
            doc.paths.push(NamedPath { name: name.clone(), path, psd_raw: None });
            Ok(())
        })?;
        return Ok(json!({"as":"path","path":name,"offset":[dx,dy],"warnings":warnings}));
    }
    layer_target(s)?;
    copies.layers = if mode == Mode::Shape {
        let layer = shape(&copies)?;
        shape_budget(layer, &destination, dx, dy)?;
        vec![layer.duplicate()]
    } else {
        let image = crate::smart_cmds::svg_source_image(&bytes, PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), 1.0)?;
        copies.depth = SampleType::F32;
        vec![Layer::new("Pasted SVG", LayerContent::Raster((*image.surface).clone()))]
    };
    crate::layer_copy_cmds::adapt_copies(s, &mut copies, &destination)?;
    crate::layer_copy_cmds::translate_copies(&mut copies, &destination, dx, dy);
    let layer = copies.layers.pop().ok_or_else(|| bad("the SVG contains no paste output"))?;
    let id = s.edit("Paste", |doc, active| {
        let id = crate::layer_place::insert_new(doc, *active, layer, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        if mode == Mode::Pixels {
            doc.selection = None;
        }
        Ok(id)
    })?;
    Ok(json!({"as":if mode==Mode::Shape{"shape"}else{"pixels"},"layer":id.0,"offset":[dx,dy],"warnings":warnings}))
}
