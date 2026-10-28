use super::params::{layer_id, parse_path, path_json};
use super::paths::{is_work, resolve_path};
use super::shape::with_shape;
use super::{EngineError, LayerContent, LayerId, Path, Result, Session, Value, VectorMask, bad, json, vector};

// ---------------------------------------------------------------------------
// Vector masks
// ---------------------------------------------------------------------------

pub(super) fn vector_mask_info(s: &Session, id: LayerId) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    Ok(match (&l.vector_mask, &l.content) {
        (Some(v), _) => {
            json!({ "layer": id.0, "path": path_json(&v.path), "enabled": v.enabled, "linked": v.linked, "density": v.density * 100.0, "feather": v.feather })
        }
        // A shape layer's path is its vector mask.
        (None, LayerContent::Shape(sh)) => {
            json!({ "layer": id.0, "path": path_json(&sh.path), "enabled": true, "linked": true, "shape": true, "density": sh.density * 100.0, "feather": sh.feather })
        }
        (None, _) => json!({ "layer": id.0, "vectorMask": null }),
    })
}

pub(super) fn vector_mask_add(s: &mut Session, p: &Value, from_path: bool) -> Result<Value> {
    const CMD: &str = "layer.vectorMask.add";
    let id = layer_id(s, p)?;
    let mut path = match p.get("path") {
        Some(v) => parse_path(v).map_err(|e| bad(CMD, e))?,
        None if from_path || p.get("name").is_some() => {
            let name = p.get("name").and_then(Value::as_str);
            // Never fall back to the layer's own path here.
            if is_work(name) {
                s.active().and_then(|d| d.doc.work_path.clone()).ok_or_else(|| EngineError::Other("no work path".into()))?
            } else {
                resolve_path(s, name)?
            }
        }
        None => Path::default(),
    };
    if p.get("hide").and_then(Value::as_bool).unwrap_or(false) {
        path.inverted = !path.inverted;
    }
    s.edit("Add Vector Mask", |doc, _| {
        let locked = doc.effective_locks(id).all;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if matches!(l.content, LayerContent::Shape(_)) {
            return Err(EngineError::Other("a shape layer's path is already its vector mask".into()));
        }
        if locked {
            return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
        }
        crate::extra_cmds::background_to_layer(doc, id);
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.vector_mask = Some(VectorMask::new(path));
        Ok(())
    })?;
    vector_mask_info(s, id)
}

pub(super) fn vector_mask_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.vectorMask.edit";
    let id = layer_id(s, p)?;
    let path = p.get("path").map(parse_path).transpose().map_err(|e| bad(CMD, e))?;
    // Same range as a pixel mask's feather (`layer.layerMask.edit`), checked before any edit (#4201).
    let feather = p
        .get("feather")
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && (0.0..=1000.0).contains(n))
                .map(|n| n as f32)
                .ok_or_else(|| bad(CMD, "`feather` must be a finite number within 0..1000"))
        })
        .transpose()?;
    let density = p
        .get("density")
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
                .map(|n| (n / 100.0) as f32)
                .ok_or_else(|| bad(CMD, "`density` must be a finite number within 0..100 (%)"))
        })
        .transpose()?;
    // A shape layer's path is its vector mask (Photoshop's Properties › Masks): Density and
    // Feather soften its fill; the path itself is edited with the shape and path commands.
    if s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Shape(_)) && l.vector_mask.is_none()) {
        if let Some(k) = ["path", "enabled", "linked", "invert"].into_iter().find(|k| p.get(*k).is_some()) {
            return Err(bad(CMD, format!("`{k}`: a shape layer's vector mask is its path; only `density` and `feather` apply")));
        }
        if density.is_none() && feather.is_none() {
            return vector_mask_info(s, id);
        }
        with_shape(s, id, "Edit Vector Mask", |sh, _| {
            if let Some(d) = density {
                sh.density = d;
            }
            if let Some(f) = feather {
                sh.feather = f;
            }
            Ok(())
        })?;
        return vector_mask_info(s, id);
    }
    s.edit("Edit Vector Mask", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let vm = l.vector_mask.as_mut().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        if let Some(pth) = path {
            vm.path = pth;
        }
        if let Some(b) = p.get("enabled").and_then(Value::as_bool) {
            vm.enabled = b;
        }
        if let Some(b) = p.get("linked").and_then(Value::as_bool) {
            vm.linked = b;
        }
        if let Some(d) = density {
            vm.density = d;
        }
        if let Some(f) = feather {
            vm.feather = f;
        }
        if let Some(b) = p.get("invert").and_then(Value::as_bool)
            && b
        {
            vm.path.inverted = !vm.path.inverted;
        }
        Ok(())
    })?;
    vector_mask_info(s, id)
}

pub(super) fn vector_mask_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    s.edit("Delete Vector Mask", |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.vector_mask.take().map(|_| ()).ok_or_else(|| EngineError::Other("layer has no vector mask".into()))
    })?;
    Ok(Value::Null)
}

/// Layer › Rasterize › Vector Mask: multiplies the vector mask into the pixel mask. When a feather
/// (either mask's) or the pixel mask's density shapes the rendered edge, the pixel mask instead
/// becomes exactly what the compositor showed, with those settings baked in (#992).
pub(super) fn vector_mask_rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    use photocraft_compose::masks::{combined_mask, feather_sigma};
    let id = layer_id(s, p)?;
    s.edit("Rasterize Vector Mask", |doc, _| {
        let area = doc.bounds();
        let depth = doc.depth;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let soft = l.vector_mask.as_ref().is_some_and(|v| v.enabled)
            && (l.vector_mask.as_ref().is_some_and(|v| feather_sigma(v.feather) > 0.0)
                || l.mask.as_ref().is_some_and(|m| m.enabled && (feather_sigma(m.feather) > 0.0 || m.density < 1.0)));
        let shown = if soft { combined_mask(l, area) } else { None };
        let vm = l.vector_mask.take().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        let mut mask = l.mask.take().unwrap_or_else(|| {
            let mut m = photocraft_doc::LayerMask::reveal_all();
            m.surface =
                photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, depth, false), &[1.0]);
            m
        });
        if let Some(shown) = shown {
            // Past the canvas the pixel mask keeps its own (density-applied) default, as below.
            let d = mask.surface.default_pixel().first().copied().unwrap_or(1.0);
            let mut surface = photocraft_raster::Surface::with_default(mask.surface.format(), &[1.0 - mask.density * (1.0 - d)]);
            surface.write_region(area, &shown.read_region(area));
            surface.prune();
            (mask.surface, mask.feather, mask.density) = (surface, 0.0, 1.0);
            l.mask = Some(mask);
            return Ok(());
        }
        let vals = vector::vector_mask_values(&vm, area);
        // Outside the canvas the vector mask is 0 unless it is empty/inverted; keep the old default there.
        let old = mask.surface.read_region(area);
        let merged: Vec<f32> = old.iter().zip(&vals).map(|(a, b)| a * b).collect();
        mask.surface.write_region(area, &merged);
        mask.surface.prune();
        l.mask = Some(mask);
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// `p` with `key` set (for alias commands).
pub(super) fn with(p: &Value, key: &str, v: Value) -> Value {
    let mut o = if p.is_object() { p.clone() } else { json!({}) };
    o[key] = v;
    o
}

pub(super) fn toggle_vector_mask(s: &mut Session, p: &Value, key: &str) -> Result<Value> {
    let id = layer_id(s, p)?;
    let cur = {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let vm = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?.vector_mask.as_ref().ok_or_else(|| EngineError::Other("layer has no vector mask".into()))?;
        if key == "enabled" { vm.enabled } else { vm.linked }
    };
    let v = p.get(key).and_then(Value::as_bool).unwrap_or(!cur);
    vector_mask_edit(s, &json!({ "layer": id.0, key: v }))
}
