//! Photoshop's Distort filter steps (#10605): Pinch, Polar Coordinates, Spherize, Twirl, Wave and
//! ZigZag, each key in recorded order with the limits and defaults Photoshop measures.

use super::descriptors::{desc, enm, num, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{D, Descriptor, Map, Value, json};

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

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset).
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.is_null() || p.as_object().is_some_and(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// An enumeration's value when its type is `ty`, the type Photoshop records it with and the
/// step saves back: one of another type isn't that setting.
fn enum_of(ty: &str) -> impl Fn(&D) -> Option<String> + '_ {
    move |v| match v {
        D::Enumerated { type_id, value } if trimmed(type_id) == ty.trim_end() => Some(trimmed(value)),
        _ => None,
    }
}

/// One key of a Distort filter's step, in recorded order.
pub(super) enum Key {
    /// An integer clamped to `min..=max` on import, as Photoshop clamps it; `default` when the
    /// step leaves it out.
    Int { key: &'static str, param: &'static str, min: i32, max: i32, default: i32 },
    /// An enumeration (`ty`, Photoshop ↔ PhotoCraft values); `default` is Photoshop's value when
    /// the step leaves it out. An unknown value, or one of another type, keeps the step.
    Enum { key: &'static str, param: &'static str, ty: &'static str, table: &'static [(&'static str, &'static str)], default: &'static str },
}

impl Key {
    fn param(&self) -> &'static str {
        match self {
            Key::Int { param, .. } | Key::Enum { param, .. } => param,
        }
    }

    fn key(&self) -> &'static str {
        match self {
            Key::Int { key, .. } | Key::Enum { key, .. } => key,
        }
    }
}

/// A step with exactly `keys` (any missing): each value clamped or looked up, Photoshop's
/// default for what it leaves out. Any other key, or a value of the wrong type, keeps the step.
pub(super) fn keys_from(d: &Descriptor, id: &str, keys: &[Key]) -> Mapped {
    keys_map(d, keys, &[]).map_or(Mapped::Unsupported { harmless: false }, |m| step(id, Value::Object(m)))
}

fn keys_map(d: &Descriptor, keys: &[Key], extra: &[&'static str]) -> Option<Map<String, Value>> {
    let known: Vec<&str> = keys.iter().map(Key::key).chain(extra.iter().copied()).map(str::trim_end).collect();
    if !only_keys(d, &known) {
        return None;
    }
    let mut m = Map::new();
    for k in keys {
        let v = match k {
            Key::Int { key, min, max, default, .. } => {
                Value::from(setting(d, key, |v| num(v).map(|n| n.round().clamp(f64::from(*min), f64::from(*max)) as i32), *default)?)
            }
            Key::Enum { key, ty, table, default, .. } => Value::from(lookup(table, &setting(d, key, enum_of(ty), (*default).to_string())?)?),
        };
        m.insert(k.param().into(), v);
    }
    Some(m)
}

/// Every key in recorded order, the command's default for what the step leaves out
/// (`ours`, by param). An integer key refuses a fractional value, which would replay rounded,
/// and one outside its `min..=max`, which would load back clamped.
pub(super) fn keys_to(p: &Value, keys: &[Key], ours: &[(&str, Value)], extra: &[&'static str]) -> Option<Descriptor> {
    let params: Vec<&str> = keys.iter().map(Key::param).chain(extra.iter().copied()).chain(["target"]).collect();
    if !pixel_target(p) || !sets_only(p, &params) {
        return None;
    }
    let value = |param: &str| match p.get(param) {
        None | Some(Value::Null) => ours.iter().find(|(k, _)| *k == param).map(|(_, v)| v.clone()),
        Some(v) => Some(v.clone()),
    };
    let mut d = desc("null");
    for k in keys {
        let v = value(k.param())?;
        d = match k {
            Key::Int { key, min, max, .. } => {
                let n = v.as_f64()?;
                if n.fract() != 0.0 || !(f64::from(*min)..=f64::from(*max)).contains(&n) {
                    return None;
                }
                d.with(key, D::Integer(n as i32))
            }
            Key::Enum { key, ty, table, .. } => d.with(key, enm(ty, lookup_back(table, v.as_str()?)?)),
        };
    }
    Some(d)
}

pub(super) const PINCH: &[Key] = &[Key::Int { key: "Amnt", param: "amount", min: -100, max: 100, default: 50 }];

pub(super) const TWIRL: &[Key] = &[Key::Int { key: "Angl", param: "angle", min: -999, max: 999, default: 50 }];

pub(super) const POLAR: &[Key] =
    &[Key::Enum { key: "Cnvr", param: "mode", ty: "Cnvr", table: &[("RctP", "rectangularToPolar"), ("PlrR", "polarToRectangular")], default: "RctP" }];

pub(super) const SPHERIZE: &[Key] = &[
    Key::Int { key: "Amnt", param: "amount", min: -100, max: 100, default: 100 },
    Key::Enum { key: "SphM", param: "mode", ty: "SphM", table: &[("Nrml", "normal"), ("HrzO", "horizontalOnly"), ("VrtO", "verticalOnly")], default: "Nrml" },
];

pub(super) const ZIGZAG: &[Key] = &[
    Key::Int { key: "Amnt", param: "amount", min: -100, max: 100, default: 10 },
    Key::Int { key: "NmbR", param: "ridges", min: 0, max: 20, default: 5 },
    Key::Enum {
        key: "ZZTy",
        param: "style",
        ty: "ZZTy",
        table: &[("PndR", "pondRipples"), ("OtFr", "outFromCenter"), ("ArnC", "aroundCenter")],
        default: "PndR",
    },
];

/// Wave's keys before its scale, undefined areas and seed (handled in `wave_from` / `wave_to`).
const WAVE: &[Key] = &[
    Key::Enum { key: "Wvtp", param: "type", ty: "Wvtp", table: &[("WvSn", "sine"), ("WvSq", "square"), ("WvTr", "triangle")], default: "WvSn" },
    Key::Int { key: "NmbG", param: "generators", min: 1, max: 999, default: 5 },
    Key::Int { key: "WLMn", param: "wavelengthMin", min: 1, max: 999, default: 10 },
    Key::Int { key: "WLMx", param: "wavelengthMax", min: 1, max: 999, default: 120 },
    Key::Int { key: "AmMn", param: "amplitudeMin", min: 1, max: 999, default: 5 },
    Key::Int { key: "AmMx", param: "amplitudeMax", min: 1, max: 999, default: 35 },
];

const WAVE_UNDEFINED: &[(&str, &str)] = &[("RptE", "repeat"), ("WrpA", "wrap")];

/// `filter.distort.wave`'s own limits where the command plays narrower than Photoshop's 1..999
/// (the renderer draws at most [`photocraft_algo::MAX_WAVE_GENERATORS`] generators). A value
/// Photoshop plays but the command rejects or plays differently keeps the step on import and
/// refuses it on export, rather than mapping a step that fails or differs when it plays.
const WAVE_COMMAND_LIMITS: &[(&str, f64, f64)] = &[
    ("generators", 1.0, photocraft_algo::MAX_WAVE_GENERATORS as f64),
    ("wavelengthMin", 1.0, 998.0),
    ("wavelengthMax", 2.0, 999.0),
    ("amplitudeMin", 1.0, 998.0),
];

/// Whether every value `p` sets for a key of [`WAVE_COMMAND_LIMITS`] is within the command's limits.
fn wave_command_takes(p: &Map<String, Value>) -> bool {
    WAVE_COMMAND_LIMITS.iter().all(|(param, min, max)| p.get(*param).and_then(Value::as_f64).is_none_or(|n| (*min..=*max).contains(&n)))
}

/// Wave (`Wave`). The command has no horizontal or vertical scale, so a step that scales the
/// waves below 100 %, or by a fraction (saved back as 100), keeps the Photoshop step. Missing undefined areas are Photoshop's Repeat
/// Edge Pixels (the command's own default is wrap); `RndS` keeps its 32 bits as the seed.
pub(super) fn wave_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(mut m) = keys_map(d, WAVE, &["SclH", "SclV", "UndA", "RndS"]).filter(wave_command_takes) else { return unsupported };
    if ["SclH", "SclV"].iter().any(|k| setting(d, k, num, 100.0).is_none_or(|n| n.fract() != 0.0 || n.clamp(1.0, 100.0) != 100.0)) {
        return unsupported;
    }
    let Some(undefined) = setting(d, "UndA", enum_of("UndA"), "RptE".into()).and_then(|e| lookup(WAVE_UNDEFINED, &e)) else { return unsupported };
    m.insert("undefinedAreas".into(), undefined.into());
    match setting(d, "RndS", |v| seed_bits(v).map(Some), None) {
        Some(Some(seed)) => {
            m.insert("seed".into(), Value::from(seed));
        }
        Some(None) => {}
        None => return unsupported,
    }
    step("filter.distort.wave", Value::Object(m))
}

/// `RndS` as the seed's 32 bits: a negative value is a signed seed's bit pattern. A fraction
/// or a value past 32 bits is no seed, rather than one truncated or saturated to another.
fn seed_bits(v: &D) -> Option<u32> {
    let n = num(v).filter(|n| n.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(u32::MAX)).contains(n))?;
    Some(if n < 0.0 { n as i32 as u32 } else { n as u32 })
}

pub(super) fn wave_to(_: &str, p: &Value) -> Option<Descriptor> {
    let ours = [
        ("type", json!("sine")),
        ("generators", json!(5)),
        ("wavelengthMin", json!(10)),
        ("wavelengthMax", json!(120)),
        ("amplitudeMin", json!(5)),
        ("amplitudeMax", json!(35)),
    ];
    if !p.as_object().is_none_or(wave_command_takes) {
        return None;
    }
    let d = keys_to(p, WAVE, &ours, &["undefinedAreas", "seed"])?;
    let undefined = match p.get("undefinedAreas") {
        None | Some(Value::Null) => "WrpA",
        Some(v) => lookup_back(WAVE_UNDEFINED, v.as_str()?)?,
    };
    let seed = match p.get("seed") {
        None | Some(Value::Null) => 0,
        Some(v) => u32::try_from(v.as_u64()?).ok()? as i32,
    };
    Some(d.with("SclH", D::Integer(100)).with("SclV", D::Integer(100)).with("UndA", enm("UndA", undefined)).with("RndS", D::Integer(seed)))
}
