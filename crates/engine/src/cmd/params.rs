use super::layer_params::layer_param;
use super::{EngineError, LayerId, Result, Session, Value, json};

// ---------- param helpers ----------

pub(super) fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Integer parameter that also accepts JSON floats (UIs send `12.0`); rounds.
pub(crate) fn int(p: &Value, key: &str) -> Option<i64> {
    p.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64)))
}

/// [`int`] narrowed to `i32` for geometry: `None` when absent, and a bad-params error when the
/// value would wrap through `as i32` — out-of-range integers truncate by multiples of 2^32,
/// which silently relocates geometry (`dx = 2^32 + 50` moves 50 px, `3e9` goes negative)
/// instead of erroring.
pub(crate) fn int_i32(cmd: &str, p: &Value, key: &str) -> Result<Option<i32>> {
    match int(p, key) {
        None => Ok(None),
        Some(v) => i32::try_from(v).map(Some).map_err(|_| bad(cmd, format!("`{key}` = {v} is outside the 32-bit coordinate range"))),
    }
}

/// An id (layer comp, slice, style…) from a JSON value: a bad-params error unless it is a whole
/// number in `u32` range. `as u32` would wrap `2^32 + 1` to `1` and target a real item.
pub(crate) fn u32_id(cmd: &str, key: &str, v: &Value) -> Result<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(cmd, format!("`{key}` = {v} is not a valid id (0..={})", u32::MAX)))
}

/// [`u32_id`] of `p[key]`: `None` when absent.
pub(crate) fn u32_id_param(cmd: &str, p: &Value, key: &str) -> Result<Option<u32>> {
    p.get(key).map(|v| u32_id(cmd, key, v)).transpose()
}

/// JSON numbers may be finite f64 values but overflow when stored in a document's f32 field.
pub(crate) fn finite_f32(cmd: &str, key: &str, value: f64) -> Result<f32> {
    let narrowed = value as f32;
    if narrowed.is_finite() { Ok(narrowed) } else { Err(bad(cmd, format!("`{key}` must fit a finite 32-bit number"))) }
}

/// Effect/type parameters contain nested colour arrays and snapshots as well as scalars.
/// Validate before parsing or editing, including numbers hidden in an effect snapshot.
pub(crate) fn check_f32_values(cmd: &str, value: &Value) -> Result<()> {
    // Bound the call stack, without allocating a work list proportional to an input array.
    fn check(cmd: &str, value: &Value, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(bad(cmd, "parameters nested deeper than 64 levels"));
        }
        match value {
            Value::Number(n) => {
                finite_f32(cmd, "number", n.as_f64().ok_or_else(|| bad(cmd, "invalid number"))?)?;
            }
            Value::Array(a) => {
                for v in a {
                    check(cmd, v, depth + 1)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    check(cmd, v, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    check(cmd, value, 0)
}

/// `p[key]` as CMYK inks 0..1 (`[c, m, y, k]`), the values a colour was picked as, or `None`
/// when absent or null. A CMYK document stores exactly these instead of what the colour's sRGB
/// stand-in converts back to (#4196, #8568).
pub(crate) fn cmyk_param(cmd: &str, p: &Value, key: &str) -> Result<Option<[f32; 4]>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let inks: Option<Vec<f32>> =
        v.as_array().filter(|a| a.len() == 4).and_then(|a| a.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32)).collect());
    match inks.as_deref() {
        Some(&[c, m, y, k]) => Ok(Some([c, m, y, k])),
        _ => Err(bad(cmd, format!("`{key}` must be [c, m, y, k] numbers 0..1"))),
    }
}

pub(crate) fn color_param(cmd: &str, p: &Value, key: &str, default: [f32; 4]) -> Result<[f32; 4]> {
    Ok(match p.get(key) {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let mut out = [0.0, 0.0, 0.0, 1.0];
            for (slot, value) in out.iter_mut().zip(a) {
                *slot = finite_f32(cmd, key, value.as_f64().unwrap_or(0.0))?;
            }
            out
        }
        Some(Value::String(s)) => parse_hex(s).unwrap_or(default),
        _ => default,
    })
}

pub(crate) fn parse_hex(s: &str) -> Option<[f32; 4]> {
    let s = s.trim_start_matches('#');
    // `from_str_radix` accepts a leading '+', so "+f" would read as 0x0f.
    if !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    match s.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 1.0]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}

/// Layer › Create / Release Clipping Mask. With a `layer` param, just that layer. Otherwise, as in
/// Photoshop, the selection (#1248): Create clips every selected layer except the lowest, which
/// becomes the base; Release releases every selected layer. Releasing a clipped layer also
/// releases the clipped layers above it in its group, as Adobe documents ("removes the selected
/// layer and any layers above it from the clipping mask"), so none is left clipped to a layer
/// that is no longer clipped. One history step either way.
pub(super) fn set_clipped(s: &mut Session, p: &Value, clip: bool) -> Result<Value> {
    let label = if clip { "Create Clipping Mask" } else { "Release Clipping Mask" };
    let ids: Vec<LayerId> = if p.get("layer").is_some() {
        vec![layer_param(s, p)?]
    } else {
        let selected = s.active().ok_or(EngineError::NoDocument)?.selected_layers();
        match (clip, selected.as_slice()) {
            (_, []) => vec![layer_param(s, p)?],
            (_, [one]) => vec![*one],
            (true, [_base, rest @ ..]) => rest.to_vec(),
            (false, all) => all.to_vec(),
        }
    };
    let ids = if clip {
        ids
    } else {
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let mut all = ids.clone();
        for id in ids.iter().filter(|id| doc.layer(**id).is_some_and(|l| l.clipped)) {
            for above in crate::clip_group::clipped_above(doc, *id) {
                if !all.contains(&above) {
                    all.push(above);
                }
            }
        }
        all
    };
    s.edit(label, |doc, _| {
        for id in &ids {
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.clipped = clip;
        }
        Ok(())
    })?;
    Ok(json!({"layers": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))
}
