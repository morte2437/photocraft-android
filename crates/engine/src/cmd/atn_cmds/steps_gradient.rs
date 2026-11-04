//! Classic Gradient recordings (#10710), using the existing pixel-paint command.
//!
//! Presets have centred midpoints and RGB byte colours. Other gradient forms stay
//! kept steps rather than losing settings on playback. Source descriptors are
//! retained only after decoding to the same executable parameters.

use super::convert::normalise;
use super::descriptors::{desc, enm, pct, px, trimmed};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::tables::{blend_label, blend_ps};
use super::{D, Descriptor, Value, json};

const ORIGIN: &str = "__atnClassicGradient";
const MAX_STOPS: usize = 64;
const MAX_DESCRIPTOR: usize = 64 * 1024;
const STYLES: &[(&str, &str)] = &[("Lnr ", "linear"), ("Rdl ", "radial"), ("Angl", "angle"), ("Rflc", "reflected"), ("Dmnd", "diamond")];
const METHODS: &[(&str, &str)] = &[("Gcls", "classic"), ("Perc", "perceptual"), ("Lnr ", "linear"), ("Smoo", "smooth")];

/// Extra or repeated fields must not disappear from the operation being played.
fn shape(d: &Descriptor, class: &str, keys: &[&str]) -> Option<()> {
    if !d.class_id.is(class) || d.name.0.len() > 1024 || d.items.len() > keys.len() {
        return None;
    }
    for (i, (key, _)) in d.items.iter().enumerate() {
        if !keys.iter().any(|k| key.is(k)) || d.items.iter().take(i).any(|(other, _)| other.as_bytes() == key.as_bytes()) {
            return None;
        }
    }
    Some(())
}

fn object<'a>(d: &'a Descriptor, key: &str) -> Option<&'a Descriptor> {
    match d.get(key)? {
        D::Descriptor(d) => Some(d),
        _ => None,
    }
}

fn number(v: &D, unit: Option<&[u8; 4]>) -> Option<f64> {
    let n = match (v, unit) {
        (D::Integer(n), None) => f64::from(*n),
        (D::Double(n), None) => *n,
        (D::UnitFloat { unit: actual, value }, Some(expected)) if actual == expected => *value,
        _ => return None,
    };
    (n.is_finite() && (n as f32).is_finite()).then_some(n)
}

fn enumeration<'a>(v: &'a D, ty: &str) -> Option<&'a super::Id> {
    match v {
        D::Enumerated { type_id, value } if type_id.is(ty) => Some(value),
        _ => None,
    }
}

fn flag(d: &Descriptor, key: &str, default: bool) -> Option<bool> {
    match d.get(key) {
        None => Some(default),
        Some(D::Boolean(v)) => Some(*v),
        _ => None,
    }
}

fn point(d: &Descriptor, key: &str) -> Option<Value> {
    let p = object(d, key)?;
    shape(p, "Pnt ", &["Hrzn", "Vrtc"])?;
    Some(json!([number(p.get("Hrzn")?, Some(b"#Pxl"))?, number(p.get("Vrtc")?, Some(b"#Pxl"))?]))
}

fn list<'a>(d: &'a Descriptor, key: &str) -> Option<&'a [D]> {
    match d.get(key)? {
        D::List(v) if (2..=MAX_STOPS).contains(&v.len()) => Some(v),
        _ => None,
    }
}

fn location(d: &Descriptor, previous: &mut f64) -> Option<f64> {
    let D::Integer(position) = d.get("Lctn")? else { return None };
    let n = f64::from(*position) / 4096.0;
    if !(0.0..=1.0).contains(&n) || n < *previous || !matches!(d.get("Mdpn"), Some(D::Integer(50))) {
        return None;
    }
    *previous = n;
    Some(n)
}

/// Decode every accepted setting; no source metadata is ever executed.
fn params(d: &Descriptor) -> Option<Value> {
    // Recordings use the event class; PhotoCraft's canonical exports use null.
    let class = if d.class_id.is("Grdn") { "Grdn" } else { "null" };
    shape(d, class, &["From", "T   ", "Opct", "Md  ", "Type", "Rvrs", "Dthr", "UsMs", "gradientsInterpolationMethod", "Grad"])?;
    let style = enumeration(d.get("Type")?, "GrdT")?;
    let style = STYLES.iter().find(|(ps, _)| style.is(ps))?.1;
    // No omitted-method default was established by the recorded-tools reference.
    let method = enumeration(d.get("gradientsInterpolationMethod")?, "gradientInterpolationMethodType")?;
    let method = METHODS.iter().find(|(ps, _)| method.is(ps))?.1;
    let opacity = match d.get("Opct") {
        None => 100.0,
        Some(v) => number(v, Some(b"#Prc"))?,
    };
    if !(1.0..=100.0).contains(&opacity) {
        return None;
    }
    let mode = match d.get("Md  ") {
        None => "Normal",
        Some(v) => blend_label(&trimmed(enumeration(v, "BlnM")?))?,
    };
    if mode == "Pass Through" {
        return None;
    }
    let g = object(d, "Grad")?;
    shape(g, "Grdn", &["Nm  ", "GrdF", "Intr", "Clrs", "Trns"])?;
    let D::Text(name) = g.get("Nm  ")? else { return None };
    if name.0.len() > 1024 || !enumeration(g.get("GrdF")?, "GrdF")?.is("CstS") {
        return None;
    }
    let smoothness = number(g.get("Intr")?, None)? * 100.0 / 4096.0;
    if !(0.0..=100.0).contains(&smoothness) {
        return None;
    }
    let mut stops = Vec::new();
    let mut previous = -1.0;
    for s in list(g, "Clrs")? {
        let D::Descriptor(s) = s else { return None };
        shape(s, "Clrt", &["Clr ", "Type", "Lctn", "Mdpn"])?;
        if !enumeration(s.get("Type")?, "Clry")?.is("UsrS") {
            return None;
        }
        let c = object(s, "Clr ")?;
        shape(c, "RGBC", &["Rd  ", "Grn ", "Bl  "])?;
        let rgb = [number(c.get("Rd  ")?, None)?, number(c.get("Grn ")?, None)?, number(c.get("Bl  ")?, None)?];
        if !rgb.iter().all(|v| (0.0..=255.0).contains(v) && v.fract() == 0.0) {
            return None;
        }
        stops.push(json!([location(s, &mut previous)?, format!("#{:02x}{:02x}{:02x}", rgb[0] as u8, rgb[1] as u8, rgb[2] as u8)]));
    }
    let use_transparency = flag(d, "UsMs", false)?;
    let mut transparency = Vec::new();
    previous = -1.0;
    for s in list(g, "Trns")? {
        let D::Descriptor(s) = s else { return None };
        shape(s, "TrnS", &["Opct", "Lctn", "Mdpn"])?;
        let opacity = number(s.get("Opct")?, Some(b"#Prc"))?;
        if !(0.0..=100.0).contains(&opacity) {
            return None;
        }
        let t = location(s, &mut previous)?;
        if use_transparency {
            transparency.push(json!([t, opacity]));
        }
    }
    Some(json!({"from":point(d,"From")?,"to":point(d,"T   ")?,"style":style,"opacity":opacity,"mode":mode,
        "reverse":flag(d,"Rvrs",false)?,"dither":flag(d,"Dthr",false)?,"stops":stops,
        "transparency":transparency,"method":method,"smoothness":smoothness}))
}

pub(super) fn from(d: &Descriptor) -> Mapped {
    let mapped = || {
        let mut p = params(d)?;
        let bytes = d.to_bytes();
        if bytes.len() > MAX_DESCRIPTOR {
            return None;
        }
        p[ORIGIN] = json!(crate::paint::tile::b64_encode(&bytes));
        Some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("paint.gradient", p))
}

fn finite(v: &Value, lo: f64, hi: f64) -> Option<f64> {
    v.as_f64().filter(|v| v.is_finite() && (lo..=hi).contains(v))
}

fn native_point(p: &Value, key: &str) -> Option<D> {
    let a = p.get(key)?.as_array().filter(|a| a.len() == 2)?;
    let coordinate = |i: usize| a.get(i)?.as_f64().filter(|v| v.is_finite() && (*v as f32).is_finite());
    Some(D::Descriptor(desc("Pnt ").with("Hrzn", px(coordinate(0)?)).with("Vrtc", px(coordinate(1)?))))
}

fn native_location(a: &[Value], previous: &mut f64) -> Option<i32> {
    let t = finite(a.first()?, 0.0, 1.0)? * 4096.0;
    if t.fract() != 0.0 || t < *previous {
        return None;
    }
    *previous = t;
    Some(t as i32)
}

/// Canonical edited/native form. Never substitute current presets or drop a target.
fn canonical(p: &Value) -> Option<Descriptor> {
    let style = STYLES.iter().find(|(_, native)| Some(*native) == p.get("style").map_or(Some("linear"), Value::as_str))?.0;
    let method = METHODS.iter().find(|(_, native)| Some(*native) == p.get("method").and_then(Value::as_str))?.0;
    let smoothness = finite(p.get("smoothness").unwrap_or(&json!(100)), 0.0, 100.0)?;
    let mut colors = Vec::new();
    let mut previous = -1.0;
    for a in p.get("stops")?.as_array().filter(|a| (2..=MAX_STOPS).contains(&a.len()))? {
        let a = a.as_array().filter(|a| a.len() == 2)?;
        let text = a.get(1)?.as_str()?.strip_prefix('#')?;
        if text.len() != 6 || !text.is_ascii() {
            return None;
        }
        let rgb = u32::from_str_radix(text, 16).ok()?;
        let c = desc("RGBC")
            .with("Rd  ", D::Double(f64::from((rgb >> 16) & 255)))
            .with("Grn ", D::Double(f64::from((rgb >> 8) & 255)))
            .with("Bl  ", D::Double(f64::from(rgb & 255)));
        colors.push(D::Descriptor(
            desc("Clrt")
                .with("Clr ", D::Descriptor(c))
                .with("Type", enm("Clry", "UsrS"))
                .with("Lctn", D::Integer(native_location(a, &mut previous)?))
                .with("Mdpn", D::Integer(50)),
        ));
    }
    let mut transparency = Vec::new();
    previous = -1.0;
    let opacity_stops = p.get("transparency")?.as_array().filter(|a| a.len() <= MAX_STOPS)?;
    if opacity_stops.len() == 1 {
        return None;
    }
    for a in opacity_stops {
        let a = a.as_array().filter(|a| a.len() == 2)?;
        transparency.push(D::Descriptor(
            desc("TrnS")
                .with("Opct", pct(finite(a.get(1)?, 0.0, 100.0)?))
                .with("Lctn", D::Integer(native_location(a, &mut previous)?))
                .with("Mdpn", D::Integer(50)),
        ));
    }
    let use_transparency = !transparency.is_empty();
    if !use_transparency {
        transparency = [0, 4096]
            .into_iter()
            .map(|t| D::Descriptor(desc("TrnS").with("Opct", pct(100.0)).with("Lctn", D::Integer(t)).with("Mdpn", D::Integer(50))))
            .collect();
    }
    let gradient = desc("Grdn")
        .with("Nm  ", super::descriptors::text("Custom"))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(smoothness * 4096.0 / 100.0))
        .with("Clrs", D::List(colors))
        .with("Trns", D::List(transparency));
    let mut d = desc("null").with("From", native_point(p, "from")?).with("T   ", native_point(p, "to")?);
    let opacity = finite(p.get("opacity").unwrap_or(&json!(100)), 1.0, 100.0)?;
    if opacity != 100.0 {
        d = d.with("Opct", pct(opacity));
    }
    let mode = blend_ps(p.get("mode").map_or(Some("Normal"), Value::as_str)?)?;
    if mode == "passThrough" || mode == "PsTh" {
        return None;
    }
    if mode != "Nrml" {
        d = d.with("Md  ", enm("BlnM", mode));
    }
    d = d.with("Type", enm("GrdT", style));
    for (param, key, default) in [("reverse", "Rvrs", false), ("dither", "Dthr", true)] {
        let value = p.get(param).map_or(Some(default), Value::as_bool)?;
        if value {
            d = d.with(key, D::Boolean(true));
        }
    }
    if use_transparency {
        d = d.with("UsMs", D::Boolean(true));
    }
    Some(d.with("gradientsInterpolationMethod", enm("gradientInterpolationMethodType", method)).with("Grad", D::Descriptor(gradient)))
}

pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    let values = p.as_object()?;
    if !pixel_target(p)
        || !values.keys().all(|k| {
            ["from", "to", "style", "opacity", "mode", "reverse", "dither", "stops", "transparency", "method", "smoothness", "target", ORIGIN]
                .contains(&k.as_str())
        })
    {
        return None;
    }
    if let Some(origin) = p.get(ORIGIN) {
        let text = origin.as_str().filter(|v| v.len() <= MAX_DESCRIPTOR * 2)?;
        let bytes = crate::paint::tile::b64_decode(text)?;
        if bytes.len() > MAX_DESCRIPTOR {
            return None;
        }
        let d = Descriptor::from_bytes(&bytes).ok()?;
        let mut expected = params(&d)?;
        let mut actual = p.clone();
        actual.as_object_mut()?.remove(ORIGIN);
        actual.as_object_mut()?.remove("target");
        normalise(&mut expected);
        normalise(&mut actual);
        if expected == actual {
            return Some(d);
        }
    }
    canonical(p)
}
