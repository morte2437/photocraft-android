//! Photoshop's Distort filter steps, with keys in recorded order and measured limits and defaults.

use super::descriptors::{code, desc, enm, jstr, num, path_text, path_value, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{lookup, lookup_back};
use super::{D, Descriptor, Id, Map, Value, json};

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

/// Only the measured Windows Displace descriptor has a verified map path and settings.
pub(super) fn displace_from(d: &Descriptor) -> Mapped {
    displace_params(d).map_or(Mapped::Unsupported { harmless: false }, |params| step("filter.distort.displace", params))
}

/// Decode only the measured, byte-preserving Windows descriptor.
fn displace_params(d: &Descriptor) -> Option<Value> {
    if d.class_id != code("null")
        || d.items.len() != 5
        || d.items.iter().any(|(key, _)| !matches!(key, Id::Code(_)))
        || d.get("DspM") != Some(&enm("DspM", "StrF"))
        || d.get("UndA") != Some(&enm("UndA", "RptE"))
    {
        return None;
    }
    let scale = |key| match d.get(key) {
        Some(D::Integer(value)) if (-999..=999).contains(value) => Some(*value),
        _ => None,
    };
    let horizontal = scale("HrzS")?;
    let vertical = scale("VrtS")?;
    let wire = d.get("DspF")?;
    let path = path_text(wire)?;
    // The shared decoder tolerates malformed lengths and UTF-16 for legacy callers. The
    // measured form must reproduce every byte, and an embedded NUL is never a file name.
    if path.contains('\0') || &path_value(&path) != wire {
        return None;
    }
    // An action file is shared like a document, and playback reads this path. Map only a path a
    // document may name: a local absolute file. Others stay kept steps, so playing a downloaded
    // set never opens a network share (which sends the user's credentials) or a device.
    crate::doc_path::resolve_in(&path, None).ok()?;
    Some(json!({"horizontal":horizontal,"vertical":vertical,"fit":"stretch","undefinedAreas":"repeat","mapPath":path}))
}

/// Write the measured form with native defaults for omitted settings, without dropping
/// settings the descriptor cannot represent.
pub(super) fn displace_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["horizontal", "vertical", "fit", "undefinedAreas", "mapPath", "target"]) {
        return None;
    }
    if p.get("fit").is_some_and(|v| v.as_str() != Some("stretch")) || p.get("undefinedAreas").is_some_and(|v| v.as_str() != Some("repeat")) {
        return None;
    }
    let scale = |key| {
        let number = match p.get(key) {
            None => 10.0,
            Some(value) => value.as_f64()?,
        };
        let value = number as i32;
        ((-999..=999).contains(&value) && f64::from(value) == number).then_some(D::Integer(value))
    };
    let path = jstr(p, "mapPath")?;
    // The header carries a u32 byte length, including its 12 bytes and the UTF-16 terminator.
    if path.is_empty() || path.contains('\0') || path.len() > ((u32::MAX - 14) / 2) as usize {
        return None;
    }
    Some(
        desc("null")
            .with("HrzS", scale("horizontal")?)
            .with("VrtS", scale("vertical")?)
            .with("DspM", enm("DspM", "StrF"))
            .with("UndA", enm("UndA", "RptE"))
            .with("DspF", path_value(path)),
    )
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

// Adapter safety budget, not a Photoshop format maximum: at most 64 KiB per f64 point
// vector and 32 KiB in the native renderer, whose per-row curve search is linear.
const MAX_SHEAR_POINTS: usize = 4096;

/// A curve must fit the playback budget and retain its order through native deduplication.
fn shear_curve(points: &[[f64; 2]]) -> bool {
    (2..=MAX_SHEAR_POINTS).contains(&points.len())
        && points.iter().all(|p| p[0].is_finite() && (0.0..=1.0).contains(&p[0]) && (-1.0..=63.0 / 64.0).contains(&p[1]))
        && points.windows(2).all(|p| p[1][0] as f32 - p[0][0] as f32 >= 1e-6)
}

/// Recorded grid coordinates become native top-to-bottom positions and half-width offsets.
fn shear_normalized(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    points.iter().map(|p| [(p[1] - 1.0) / 127.0, p[0] / 64.0]).collect()
}

/// Read every measured point; an invalid entry must not disappear in the native list parser.
fn shear_points(value: &D) -> Option<Vec<[f64; 2]>> {
    let D::List(values) = value else { return None };
    if !(2..=MAX_SHEAR_POINTS).contains(&values.len()) {
        return None;
    }
    values
        .iter()
        .map(|v| {
            let D::Descriptor(point) = v else { return None };
            if trimmed(&point.class_id) != "Pnt" {
                return None;
            }
            let [(h, D::Double(x)), (v, D::Double(y))] = point.items.as_slice() else { return None };
            (trimmed(h) == "Hrzn" && trimmed(v) == "Vrtc" && x.is_finite() && y.is_finite()).then(|| [x.clamp(-64.0, 63.0), y.clamp(1.0, 128.0)])
        })
        .collect()
}

/// Bound native/source JSON lists before conversion, without cloning the input array.
fn shear_json_points(value: &Value) -> Option<Vec<[f64; 2]>> {
    let points = value.as_array()?;
    if !(2..=MAX_SHEAR_POINTS).contains(&points.len()) {
        return None;
    }
    points
        .iter()
        .map(|point| {
            let [x, y] = point.as_array()?.as_slice() else { return None };
            Some([x.as_f64()?, y.as_f64()?])
        })
        .collect()
}

/// Import only the complete measured curve range, with dialog clamps and exact fractional save data.
pub(super) fn shear_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !only_keys(d, &["ShrP", "UndA", "ShrS", "ShrE"]) {
            return None;
        }
        let grid = setting(d, "ShrP", shear_points, vec![[0.0, 1.0], [0.0, 128.0]])?;
        let index = |v: &D| if let D::Integer(n) = v { Some(*n) } else { None };
        if setting(d, "ShrS", index, 0)? != 0 || setting(d, "ShrE", index, 1)? != i32::try_from(grid.len().checked_sub(1)?).ok()? {
            return None;
        }
        let points = shear_normalized(&grid);
        if !shear_curve(&points) {
            return None;
        }
        let undefined = setting(d, "UndA", enum_of("UndA"), "WrpA".into())?;
        let mut p = json!({"points": points, "undefinedAreas": lookup(WAVE_UNDEFINED, &undefined)?});
        // Dividing by 127 can lose a wire bit (for example Vrtc = 4.975609756097561).
        // Preserve only that case, and reject stale metadata on export after a point edit.
        if points.iter().zip(&grid).any(|(p, g)| [p[1] * 64.0, p[0] * 127.0 + 1.0] != *g) {
            p["_atnShearPoints"] = json!(grid);
        }
        Some(step("filter.distort.shear", p))
    };
    mapped().unwrap_or(Mapped::Unsupported { harmless: false })
}

/// Export the effective native curve; settings without a faithful measured wire form are refused.
pub(super) fn shear_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !sets_only(p, &["points", "amount", "undefinedAreas", "target", "_atnShearPoints"]) {
        return None;
    }
    let points: Vec<[f64; 2]> = match p.get("points") {
        None | Some(Value::Null) => {
            let amount = match p.get("amount") {
                None | Some(Value::Null) => 0.0,
                Some(value) => value.as_f64().filter(|n| n.is_finite())?,
            };
            // Match the native command's f32 conversion and amount clamp.
            let amount = (amount as f32).clamp(-100.0, 100.0) / 100.0;
            vec![[0.0, 0.0], [0.5, f64::from(amount)], [1.0, 0.0]]
        }
        Some(value) => shear_json_points(value)?,
    };
    if !shear_curve(&points) {
        return None;
    }
    let stored = p.get("_atnShearPoints");
    let grid: Vec<[f64; 2]> = match stored {
        None => points.iter().map(|p| [p[1] * 64.0, p[0] * 127.0 + 1.0]).collect(),
        Some(value) => shear_json_points(value)?,
    };
    // Derived grid arithmetic may change an f64 wire bit without changing the f32 curve
    // the native command consumes. Stored source coordinates still require exact agreement:
    // even an edit below f32 precision must not revive stale recorded metadata.
    let matches = grid.len() == points.len()
        && grid.iter().zip(&points).all(|(grid, point)| {
            let normalized = [(grid[1] - 1.0) / 127.0, grid[0] / 64.0];
            if stored.is_some() { normalized == *point } else { normalized.map(|v| v as f32) == point.map(|v| v as f32) }
        });
    if grid.iter().any(|p| !(-64.0..=63.0).contains(&p[0]) || !(1.0..=128.0).contains(&p[1])) || !matches {
        return None;
    }
    let end = i32::try_from(grid.len().checked_sub(1)?).ok()?;
    let undefined = match p.get("undefinedAreas") {
        None | Some(Value::Null) => "WrpA",
        Some(value) => lookup_back(WAVE_UNDEFINED, value.as_str()?)?,
    };
    let grid = grid.into_iter().map(|p| D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(p[0])).with("Vrtc", D::Double(p[1])))).collect();
    Some(desc("null").with("ShrP", D::List(grid)).with("UndA", enm("UndA", undefined)).with("ShrS", D::Integer(0)).with("ShrE", D::Integer(end)))
}
