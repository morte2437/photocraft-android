//! Fibers mapping adapted from #10620; dialog limits are now measured.

use super::descriptors::desc;
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// Translate only unique measured keys so later values cannot be lost on save.
fn keys(d: &Descriptor, allowed: &[&str]) -> bool {
    d.items.iter().all(|(key, _)| allowed.iter().any(|name| key.is(name)))
        && allowed.iter().all(|name| d.items.iter().filter(|(key, _)| key.is(name)).count() <= 1)
}

/// Read a recorded integer without accepting another descriptor type as its default.
fn integer(d: &Descriptor, key: &str, default: i32) -> Option<i32> {
    match d.get(key) {
        None => Some(default),
        Some(D::Integer(n)) => Some(*n),
        _ => None,
    }
}

/// Native numeric parameters must fit the descriptor integer without rounding.
fn param_integer(p: &Value, key: &str, default: i32) -> Option<i32> {
    let Some(v) = p.get(key) else { return Some(default) };
    let n = v.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n)).then_some(n as i32)
}

/// Pixel-only export cannot discard an explicit native layer, seed or color setting.
fn params(p: &Value, allowed: &[&str]) -> bool {
    super::events::pixel_target(p) && p.as_object().is_some_and(|p| p.keys().all(|k| k == "target" || allowed.contains(&k.as_str())))
}

/// The measured integer ranges must not replay as the native command's clamped values.
pub(super) fn fibers_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !keys(d, &["Vrnc", "Strg", "RndS"]) {
            return None;
        }
        let p = json!({"variance": integer(d, "Vrnc", 16)?, "strength": integer(d, "Strg", 4)?, "seed": integer(d, "RndS", 0)?});
        (fibers_to("filter.render.fibers", &p)?.items == d.items).then_some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("filter.render.fibers", p))
}

/// Write signed seed bits even when the native command used the equivalent unsigned value.
pub(super) fn fibers_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &["variance", "strength", "seed"]) {
        return None;
    }
    let seed = match p.get("seed").and_then(Value::as_u64) {
        Some(n) => u32::try_from(n).ok()? as i32,
        None => param_integer(p, "seed", 0)?,
    };
    let variance = param_integer(p, "variance", 16)?;
    let strength = param_integer(p, "strength", 4)?;
    if !(1..=64).contains(&variance) || !(1..=64).contains(&strength) {
        return None;
    }
    Some(desc("null").with("Vrnc", D::Integer(variance)).with("Strg", D::Integer(strength)).with("RndS", D::Integer(seed)))
}
