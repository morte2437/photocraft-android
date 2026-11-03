//! Direct GrMp and RplC forms measured in #10603 (Group E).
use super::adjustments::gradient_descriptor;
use super::descriptors::{desc, enm, get_list, get_num, get_obj};
use super::events::pixel_target;
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// Typed source values needed to retain the direct event and fractional RGB on save.
/// Execution uses ordinary numeric RGB stops, never a stored descriptor.
pub(super) const GRADIENT_ORIGIN: &str = "__atnGrMp";

/// Refuse extra native settings instead of exporting an incomplete edit.
fn only_params(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_some_and(|p| p.keys().all(|k| allowed.contains(&k.as_str())))
}

/// Read exactly three channels; callers enforce their colour-space limits.
fn triple(p: &Value) -> Option<[f64; 3]> {
    let a = p.as_array().filter(|a| a.len() == 3)?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

/// Import only a descriptor the native stop model and the strict writer retain.
/// Comparing the reconstructed typed descriptor rejects dropped settings, unknown
/// keys, duplicates, unsupported units/classes and lossy wire-type conversions.
pub(super) fn gradient_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        let g = get_obj(d, "Grad")?;
        let D::Text(name) = g.get("Nm  ")? else { return None };
        let mut stops = Vec::new();
        let mut colors = Vec::new();
        for s in get_list(g, "Clrs").filter(|s| (2..=64).contains(&s.len()))? {
            let D::Descriptor(s) = s else { return None };
            let c = get_obj(s, "Clr ")?;
            let rgb = [get_num(c, "Rd  ")?, get_num(c, "Grn ")?, get_num(c, "Bl  ")?];
            colors.push(rgb);
            stops.push(json!([get_num(s, "Lctn")? / 4096.0, rgb.map(|v| v / 255.0)]));
        }
        let mut p = json!({"stops": stops, GRADIENT_ORIGIN: {"name": name.to_string_lossy(), "colors": colors}});
        if d.get("gradientsInterpolationMethod").is_none() {
            // The measured default is classic; retain omission as well as meaning.
            p[GRADIENT_ORIGIN]["omitInterpolation"] = json!(true);
        }
        for (key, param) in [("Rvrs", "reverse"), ("Dthr", "dither")] {
            if let Some(v) = d.get(key) {
                let D::Boolean(v) = v else { return None };
                p[param] = json!(v);
            }
        }
        (gradient_to("", &p)?.eq(d)).then_some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("image.adjustments.gradientMap", p))
}

/// Write the measured classic form. The exact source RGB values must still agree
/// with the executed stops, so changing a stop never revives stale source data.
pub(super) fn gradient_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !only_params(p, &["stops", "reverse", "dither", "target", GRADIENT_ORIGIN]) {
        return None;
    }
    let origin = p.get(GRADIENT_ORIGIN)?;
    if !only_params(origin, &["name", "colors", "omitInterpolation"]) {
        return None;
    }
    let name = origin.get("name")?.as_str()?;
    let colors = origin.get("colors")?.as_array()?;
    let stops = p.get("stops")?.as_array().filter(|s| (2..=64).contains(&s.len()) && s.len() == colors.len())?;
    let mut encoded = Vec::with_capacity(stops.len());
    let mut previous = -1.0;
    for (stop, color) in stops.iter().zip(colors) {
        let a = stop.as_array().filter(|a| a.len() == 2)?;
        let position = a.first()?.as_f64()?;
        if !position.is_finite() || !(0.0..=1.0).contains(&position) || position < previous || (position * 4096.0).fract() != 0.0 {
            return None;
        }
        previous = position;
        let rgb = triple(color)?;
        if !rgb.iter().all(|v| v.is_finite() && (0.0..=255.0).contains(v)) || triple(a.get(1)?)? != rgb.map(|v| v / 255.0) {
            return None;
        }
        let rgb = desc("RGBC").with("Rd  ", D::Double(rgb[0])).with("Grn ", D::Double(rgb[1])).with("Bl  ", D::Double(rgb[2]));
        encoded.push(D::Descriptor(
            desc("Clrt")
                .with("Clr ", D::Descriptor(rgb))
                .with("Type", enm("Clry", "UsrS"))
                .with("Lctn", D::Integer((position * 4096.0) as i32))
                .with("Mdpn", D::Integer(50)),
        ));
    }
    let mut d = desc("null");
    let omit = match origin.get("omitInterpolation") {
        None => false,
        Some(v) => v.as_bool()?,
    };
    if !omit {
        d = d.with("gradientsInterpolationMethod", enm("gradientInterpolationMethodType", "Gcls"));
    }
    d = d.with("Grad", D::Descriptor(gradient_descriptor(encoded, name)));
    for (param, key) in [("reverse", "Rvrs"), ("dither", "Dthr")] {
        if let Some(v) = p.get(param) {
            d = d.with(key, D::Boolean(v.as_bool()?));
        }
    }
    Some(d)
}

/// Retain source precision while requiring coordinates to stay finite in native f32.
fn lab_bound(p: &Value) -> Option<[f64; 3]> {
    let values = triple(p)?;
    values.iter().all(|v| v.is_finite() && (*v as f32).is_finite()).then_some(values)
}

/// Reject extra, duplicate or reordered keys that the strict writer cannot retain.
fn ordered(d: &Descriptor, keys: &[&str]) -> bool {
    d.items.len() == keys.len() && d.items.iter().zip(keys).all(|((key, _), expected)| key.is(expected))
}

/// Read the measured Lab encoding directly, without RGB clipping or type coercion.
fn lab_from(v: &D) -> Option<[f64; 3]> {
    let D::Descriptor(d) = v else { return None };
    if !d.class_id.is("LbCl") || !d.name.0.is_empty() || !ordered(d, &["Lmnc", "A   ", "B   "]) {
        return None;
    }
    let values = d
        .items
        .iter()
        .map(|(_, v)| match v {
            D::Double(n) => Some(*n),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    lab_bound(&json!(values))
}

/// RplC records the sampled D50 box directly. RGB conversion would lose both the
/// box width and out-of-gamut coordinates, so pass its bounds to the native command.
pub(super) fn replace_from(d: &Descriptor) -> Mapped {
    let mapped = || {
        if !d.class_id.is("null") || !d.name.0.is_empty() || !ordered(d, &["Fzns", "Mnm ", "Mxm ", "H   ", "Strt", "Lght", "colorModel"]) {
            return None;
        }
        let (lo, hi) = (lab_from(d.get("Mnm ")?)?, lab_from(d.get("Mxm ")?)?);
        if lo.iter().zip(hi).any(|(lo, hi)| *lo > hi) || !matches!(d.get("colorModel"), Some(D::Integer(0))) {
            return None;
        }
        let mut p = json!({"labMinimum": lo, "labMaximum": hi});
        for (key, param, lo, hi) in
            [("Fzns", "fuzziness", 0, 200), ("H   ", "hue", -180, 180), ("Strt", "saturation", -100, 100), ("Lght", "lightness", -100, 100)]
        {
            let D::Integer(n) = d.get(key)? else { return None };
            p[param] = json!((*n).clamp(lo, hi));
        }
        Some(p)
    };
    mapped().map_or(Mapped::Unsupported { harmless: false }, |p| step("image.adjustments.replaceColor", p))
}

/// Emit all measured keys in order. Fractional native shifts have no integer ATN
/// representation and are refused; an ordinary RGB-sample call is not a Lab box.
pub(super) fn replace_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || !only_params(p, &["labMinimum", "labMaximum", "fuzziness", "hue", "saturation", "lightness", "target"]) {
        return None;
    }
    let (lo, hi) = (lab_bound(p.get("labMinimum")?)?, lab_bound(p.get("labMaximum")?)?);
    if lo.iter().zip(hi).any(|(lo, hi)| *lo > hi) {
        return None;
    }
    let lab = |v: [f64; 3]| D::Descriptor(desc("LbCl").with("Lmnc", D::Double(v[0])).with("A   ", D::Double(v[1])).with("B   ", D::Double(v[2])));
    let field = |key, default: f64, lo, hi| {
        let n = match p.get(key) {
            None => default,
            Some(v) => v.as_f64()?,
        };
        let n = n.is_finite().then_some(n)?.clamp(lo, hi);
        (n.fract() == 0.0).then_some(D::Integer(n as i32))
    };
    Some(
        desc("null")
            .with("Fzns", field("fuzziness", 40.0, 0.0, 200.0)?)
            .with("Mnm ", lab(lo))
            .with("Mxm ", lab(hi))
            .with("H   ", field("hue", 0.0, -180.0, 180.0)?)
            .with("Strt", field("saturation", 0.0, -100.0, 100.0)?)
            .with("Lght", field("lightness", 0.0, -100.0, 100.0)?)
            .with("colorModel", D::Integer(0)),
    )
}
