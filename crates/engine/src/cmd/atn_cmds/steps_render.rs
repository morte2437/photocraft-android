//! Recorded descriptors for the Render filters.

use std::hash::BuildHasher;

use super::descriptors::{desc, enm};
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{D, Descriptor, Value, json};

const CENTER: f64 = 0.40000000596046;
const LENSES: &[(&str, &str)] = &[("Zm", "zoom"), ("Nkn", "prime35"), ("Nkn1", "prime105"), ("PnVs", "moviePrime")];

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

/// Clouds events have no fields; tool colors are read when the step plays.
pub(super) fn clouds_from(id: &str, d: &Descriptor) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Explicit colors and layer references have no fields in these ATN events. A seed has none
/// either, but `seed: 0` (what PhotoCraft's dialog records) is what the imported empty step
/// plays as, so it is dropped; any other seed would play differently and is refused.
pub(super) fn clouds_to(_: &str, p: &Value) -> Option<Descriptor> {
    let seed_replays = p.get("seed").is_none_or(|s| s.as_f64() == Some(0.0));
    (params(p, &["seed"]) && seed_replays).then(|| desc("null"))
}

/// Capture a missing recorded seed once; later playback and export reuse the stored bits.
fn new_seed() -> i32 {
    std::collections::hash_map::RandomState::new().hash_one(0u8) as i32
}

/// Fibers keeps raw variance/strength until their dialog limits are measured.
pub(super) fn fibers_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !keys(d, &["Vrnc", "Strg", "RndS"]) {
            return None;
        }
        let seed = if d.get("RndS").is_none() { new_seed() } else { integer(d, "RndS", 0)? };
        Some(json!({"variance": integer(d, "Vrnc", 16)?, "strength": integer(d, "Strg", 4)?, "seed": seed}))
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
    Some(
        desc("null")
            .with("Vrnc", D::Integer(param_integer(p, "variance", 16)?))
            .with("Strg", D::Integer(param_integer(p, "strength", 4)?))
            .with("RndS", D::Integer(seed)),
    )
}

/// Read the point's doubles without changing the recorded descriptor precision.
fn coordinate(d: &Descriptor, key: &str) -> Option<f64> {
    match d.get(key) {
        None => Some(CENTER),
        Some(D::Double(n)) if n.is_finite() => Some(*n),
        _ => None,
    }
}

/// Lens Flare materializes the measured defaults and retains valid off-canvas centers.
pub(super) fn flare_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !keys(d, &["Brgh", "FlrC", "Lns "]) {
            return None;
        }
        let center = match d.get("FlrC") {
            None => [CENTER; 2],
            Some(D::Descriptor(point)) if point.class_id.is("Pnt ") && keys(point, &["Hrzn", "Vrtc"]) => {
                [coordinate(point, "Hrzn")?, coordinate(point, "Vrtc")?]
            }
            _ => return None,
        };
        let lens = match d.get("Lns ") {
            None => "zoom",
            Some(D::Enumerated { type_id, value }) if type_id.is("Lns ") => lookup(LENSES, &super::descriptors::trimmed(value))?,
            _ => return None,
        };
        Some(json!({"brightness": integer(d, "Brgh", 100)?.clamp(10, 300), "centerX": center[0], "centerY": center[1], "lens": lens}))
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("filter.render.lensFlare", p))
}

/// Native omitted centers mean 0.5; imported steps already carry the measured 0.4 values.
pub(super) fn flare_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &["brightness", "centerX", "centerY", "lens"]) {
        return None;
    }
    let number = |key| match p.get(key) {
        None => Some(0.5),
        Some(n) => n.as_f64().filter(|n| n.is_finite()),
    };
    let lens = match p.get("lens") {
        None => "zoom",
        Some(v) => v.as_str()?,
    };
    Some(
        desc("null")
            .with("Brgh", D::Integer(param_integer(p, "brightness", 100)?.clamp(10, 300)))
            .with("FlrC", D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(number("centerX")?)).with("Vrtc", D::Double(number("centerY")?))))
            .with("Lns ", enm("Lns ", lookup_back(LENSES, lens)?)),
    )
}
