//! Photoshop's Video, Other, Average and matting steps (#10607): De-Interlace, NTSC Colors,
//! HSB/HSL, Custom, Average and Remove Black / White Matte, with the keys Photoshop records.

use super::descriptors::{desc, enm, enumerated, get_list, num, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{D, Descriptor, Map, Value, json};
use std::ops::RangeInclusive;

/// Whether every key of `d` is one of `keys` (codes compare without their space padding).
fn only_keys(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.iter().all(|(k, _)| keys.contains(&trimmed(k).as_str()))
}

/// The step's `key` read by `read` (its space padding ignored, as `only_keys` admits it),
/// `default` when the step leaves it out, `None` when it holds a value `read` can't take: that
/// is still the step's setting, so the step is kept rather than played with the default. A key
/// recorded more than once is `None` too: the step saves one value, so it would drop the other.
fn setting<T>(d: &Descriptor, key: &str, read: impl Fn(&D) -> Option<T>, default: T) -> Option<T> {
    let mut found = d.items.iter().filter(|(k, _)| trimmed(k) == key.trim_end());
    match (found.next(), found.next()) {
        (Some((_, v)), None) => read(v),
        (None, _) => Some(default),
        (Some(_), Some(_)) => None,
    }
}

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset). Params that are neither an
/// object nor `null` are no command's params, so they set something no event can hold.
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.is_null() || p.as_object().is_some_and(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// NTSC Colors, Average and Remove Black / White Matte: Photoshop runs them with an empty
/// descriptor. A step that carries a key keeps the Photoshop step.
pub(super) fn bare_from(d: &Descriptor, id: &str) -> Mapped {
    if d.items.is_empty() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// The empty descriptor, for a step that edits the layer's pixels and sets nothing else.
pub(super) fn bare_to(_: &str, p: &Value) -> Option<Descriptor> {
    (pixel_target(p) && sets_only(p, &["target"])).then(|| desc("null"))
}

/// An enumeration key: (key, param, type, Photoshop ↔ PhotoCraft values, Photoshop's default).
type EnumKey = (&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)], &'static str);

pub(super) const DE_INTERLACE: &[EnumKey] = &[
    ("IntE", "eliminate", "IntE", &[("ElmO", "oddFields"), ("ElmE", "evenFields")], "ElmO"),
    ("IntC", "createBy", "IntC", &[("CrtI", "interpolation"), ("CrtD", "duplication")], "CrtI"),
];

const HSB_MODELS: &[(&str, &str)] = &[("RGBC", "rgb"), ("HSBl", "hsb"), ("HSLC", "hsl")];

pub(super) const HSB_HSL: &[EnumKey] = &[("Inpt", "inputMode", "ClrS", HSB_MODELS, "RGBC"), ("Otpt", "rowOrder", "ClrS", HSB_MODELS, "HSBl")];

/// A step of enumerations only: each value looked up, Photoshop's default for a missing one.
/// An unknown value or key, or a value that is not an enumeration, keeps the Photoshop step.
pub(super) fn enums_from(d: &Descriptor, id: &str, keys: &[EnumKey]) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let names: Vec<&str> = keys.iter().map(|k| k.0).collect();
    if !only_keys(d, &names) {
        return unsupported;
    }
    let mut p = Map::new();
    for (key, param, _, table, default) in keys {
        let Some(v) = setting(d, key, enumerated, (*default).to_string()).and_then(|e| lookup(table, &e)) else { return unsupported };
        p.insert((*param).into(), v.into());
    }
    step(id, Value::Object(p))
}

/// Every key in recorded order; a value left out is the command's default, which is
/// Photoshop's for these two filters.
pub(super) fn enums_to(p: &Value, keys: &[EnumKey]) -> Option<Descriptor> {
    let params: Vec<&str> = keys.iter().map(|k| k.1).chain(["target"]).collect();
    if !pixel_target(p) || !sets_only(p, &params) {
        return None;
    }
    let mut d = desc("null");
    for (key, param, ty, table, default) in keys {
        let code = match p.get(*param) {
            None | Some(Value::Null) => *default,
            Some(v) => lookup_back(table, v.as_str()?)?,
        };
        d = d.with(key, enm(ty, code));
    }
    Some(d)
}

/// `filter.other.custom`'s ranges: (scale, offset, kernel cell). The command clamps a value
/// outside them when it plays, so `Scl 0` would play as 1 and save back as 0.
const CUSTOM_SCALE: RangeInclusive<f64> = 1.0..=9999.0;
const CUSTOM_OFFSET: RangeInclusive<f64> = -9999.0..=9999.0;
const CUSTOM_CELL: RangeInclusive<f64> = -999.0..=999.0;

/// A whole number within `range`.
fn within(range: &RangeInclusive<f64>) -> impl Fn(&D) -> Option<f64> + '_ {
    move |v| num(v).filter(|n| n.fract() == 0.0 && range.contains(n))
}

/// Custom (`Cstm`): `Mtrx` is the 5 × 5 kernel, row by row. Photoshop takes exactly 25 cells (a
/// 3 × 3 list makes it fail), so any other matrix keeps the Photoshop step, as does a `Scl `,
/// `Ofst` or cell that is not a number, one that is not a whole number (export writes integers,
/// so Save Actions would refuse it), or one outside the command's range (it would play clamped).
pub(super) fn custom_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if !only_keys(d, &["Scl", "Ofst", "Mtrx"]) {
        return unsupported;
    }
    let Some(cells) = get_list(d, "Mtrx").filter(|c| c.len() == 25) else { return unsupported };
    let Some(kernel) = cells.iter().map(within(&CUSTOM_CELL)).collect::<Option<Vec<f64>>>() else { return unsupported };
    let (Some(scale), Some(offset)) = (setting(d, "Scl ", within(&CUSTOM_SCALE), 1.0), setting(d, "Ofst", within(&CUSTOM_OFFSET), 0.0)) else {
        return unsupported;
    };
    let mut p = Map::new();
    p.insert("kernel".into(), json!(kernel));
    p.insert("scale".into(), scale.into());
    p.insert("offset".into(), offset.into());
    step("filter.other.custom", Value::Object(p))
}

/// `Scl `, `Ofst` and the 25 cells as integers; a kernel left out is the command's identity
/// (1 in the middle). A fraction anywhere refuses the step: it would replay rounded; so does a
/// value outside the command's range, which it doesn't play.
pub(super) fn custom_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["target", "kernel", "scale", "offset"]) {
        return None;
    }
    let int = |range: &RangeInclusive<f64>, v: &Value| {
        let n = v.as_f64()?;
        (n.fract() == 0.0 && range.contains(&n)).then_some(n as i32)
    };
    let field = |k: &str, range: &RangeInclusive<f64>, default: i32| match p.get(k) {
        None | Some(Value::Null) => Some(default),
        Some(v) => int(range, v),
    };
    let cells: Vec<i32> = match p.get("kernel") {
        None | Some(Value::Null) => (0..25).map(|i| i32::from(i == 12)).collect(),
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() == 25)?;
            a.iter().map(|c| int(&CUSTOM_CELL, c)).collect::<Option<_>>()?
        }
    };
    Some(
        desc("null")
            .with("Scl ", D::Integer(field("scale", &CUSTOM_SCALE, 1)?))
            .with("Ofst", D::Integer(field("offset", &CUSTOM_OFFSET, 0)?))
            .with("Mtrx", D::List(cells.into_iter().map(D::Integer).collect())),
    )
}
