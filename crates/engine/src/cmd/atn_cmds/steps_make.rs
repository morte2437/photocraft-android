use super::adjustments::{HUE_RANGES, adjustment_by_class, adjustment_by_suffix, adjustment_steps, levels_complete};
use super::descriptors::{
    PCT, PX, Target, ang, boolean, class, class_ref, code, color_desc, color_hex, desc, enm, enumerated, get_bool, get_enum, get_list, get_num, get_obj,
    get_ref, get_text, jbool, jnum, jstr, layer_ref, layer_target, long, name_ref, num, pct, px, ref_class, target, target_ref, text, trimmed,
};
use super::fields::{Mapped, none, step, steps};
use super::steps_layer::quick_mask_ref;
use super::tables::{CHANNELS, LABEL_COLORS, MODES, blend_label, blend_ps, channel_of, channel_ref, lookup, lookup_back};
use super::{D, Descriptor, Id, Map, ReferenceItem, UnicodeString, Value, json};

/// A `Md  ` value as a blend-mode label, only when it is a `BlnM` enumeration: another
/// enumeration type sharing a value id (`Mltp`) is not a blend mode.
fn blend_mode(d: &Descriptor) -> Option<&'static str> {
    match d.get("Md  ")? {
        D::Enumerated { type_id, value } if type_id.is("BlnM") => blend_label(&trimmed(value)),
        _ => None,
    }
}

/// A `Nm  ` text that saves back unchanged: no unpaired surrogate replaced, no extra NULs lost.
fn exact_name(d: &Descriptor) -> Option<String> {
    let D::Text(original) = d.get("Nm  ")? else { return None };
    let name = original.to_string_lossy();
    (*original == UnicodeString::new_nul(&name) || *original == UnicodeString::new(&name)).then_some(name)
}

/// A `patternLayer` descriptor's pattern, scale, angle and link as the params of
/// `layer.newFillLayer.pattern` / `layer.fill.pattern.set`. A pattern recorded with both an id
/// and a name goes by the id with the name as `patternName`: ids differ between Photoshop's
/// library and one rebuilt from the same `.pat` file, while names may collide. None for a
/// pattern recorded without a name or id.
fn pattern_params(t: &Descriptor) -> Option<Map<String, Value>> {
    let mut p = Map::new();
    if let Some(ptrn) = get_obj(t, "Ptrn") {
        let name = get_text(ptrn, "Nm  ").filter(|n| !n.is_empty());
        match (get_text(ptrn, "Idnt").filter(|i| !i.is_empty()), name) {
            (Some(id), Some(name)) => {
                p.insert("pattern".into(), id.into());
                p.insert("patternName".into(), name.into());
            }
            (key, name) => {
                p.insert("pattern".into(), key.or(name)?.into());
            }
        }
    }
    if let Some(s) = get_num(t, "Scl ") {
        p.insert("scale".into(), s.into());
    }
    if let Some(a) = get_num(t, "Angl") {
        p.insert("angle".into(), a.into());
    }
    if let Some(l) = get_bool(t, "Algn") {
        p.insert("link".into(), l.into());
    }
    Some(p)
}

/// The `patternLayer` descriptor for [`pattern_params`]' params.
fn pattern_desc(p: &Value) -> Option<Descriptor> {
    let mut t = desc("patternLayer");
    match (jstr(p, "pattern"), jstr(p, "patternName")) {
        (Some(id), Some(n)) => t = t.with("Ptrn", D::Descriptor(desc("Ptrn").with("Nm  ", text(n)).with("Idnt", text(id)))),
        (Some(n), None) | (None, Some(n)) => t = t.with("Ptrn", D::Descriptor(desc("Ptrn").with("Nm  ", text(n)))),
        (None, None) => {}
    }
    if let Some(s) = jnum(p, "scale") {
        t = t.with("Scl ", pct(s));
    }
    if let Some(a) = jnum(p, "angle") {
        t = t.with("Angl", ang(a));
    }
    if let Some(l) = jbool(p, "link") {
        t = t.with("Algn", D::Boolean(l));
    }
    if p.get("phase").is_some() {
        return None;
    }
    Some(t)
}

/// Gradient Fill layers: `GrdT` ↔ `style`.
const GRADIENT_STYLES: &[(&str, &str)] = &[("Lnr", "linear"), ("Rdl", "radial"), ("Angl", "angle"), ("Rflt", "reflected"), ("Dmnd", "diamond")];

/// The `gradientLayer` keys [`gradient_params`] reads; a descriptor with any other key stays a
/// kept Photoshop step.
const GRADIENT_LAYER_KEYS: &[&str] = &["Grad", "Type", "Angl", "Rvrs", "Dthr", "Algn", "Scl ", "Ofst", "gradientsInterpolationMethod"];

/// The hidden param an imported Gradient Fill step keeps its recorded `gradientLayer` descriptor
/// in, so Save Actions writes it back unchanged while its params are the ones it loaded with.
const GRADIENT_DESCRIPTOR: &str = "__atnDescriptor";

/// A `gradientLayer` descriptor as the params of `layer.newFillLayer.gradient` /
/// `gradient.fill.set`. Midpoint k belongs to the segment that ends at stop k + 1, as the PSD
/// reader reads it; all-default midpoints give `[]`, which clears custom ones on a Set. None for
/// what the fill does not play: a Noise gradient, a smoothness below 100 %, a non-classic
/// interpolation, a transparency midpoint, stops out of order or values out of range.
fn gradient_params(t: &Descriptor) -> Option<Map<String, Value>> {
    if t.items.iter().any(|(k, _)| !GRADIENT_LAYER_KEYS.iter().any(|g| k.is(g))) {
        return None;
    }
    let mut p = Map::new();
    if let Some(s) = t.get("Type") {
        p.insert("style".into(), lookup(GRADIENT_STYLES, &enumerated(s)?)?.into());
    }
    if let Some(a) = t.get("Angl") {
        p.insert("angle".into(), num(a)?.into());
    }
    for (key, param) in [("Rvrs", "reverse"), ("Dthr", "dither"), ("Algn", "align")] {
        if let Some(v) = t.get(key) {
            p.insert(param.into(), boolean(v)?.into());
        }
    }
    if let Some(s) = t.get("Scl ") {
        p.insert("scale".into(), num(s)?.into());
    }
    if let Some(o) = t.get("Ofst") {
        let D::Descriptor(o) = o else { return None };
        p.insert("offset".into(), json!([get_num(o, "Hrzn")?, get_num(o, "Vrtc")?]));
    }
    if t.get("gradientsInterpolationMethod").is_some_and(|m| enumerated(m).as_deref() != Some("Gcls")) {
        return None;
    }
    let g = get_obj(t, "Grad")?;
    if get_enum(g, "GrdF").as_deref() != Some("CstS") || g.get("Intr").is_some_and(|i| num(i) != Some(4096.0)) {
        return None;
    }
    let (mut stops, mut mids, mut previous) = (Vec::new(), Vec::new(), 0.0);
    for (i, s) in get_list(g, "Clrs").filter(|c| (2..=1024).contains(&c.len()))?.iter().enumerate() {
        let D::Descriptor(s) = s else { return None };
        let color = match get_enum(s, "Type").as_deref() {
            None | Some("UsrS") => json!(color_hex(get_obj(s, "Clr ")?)?),
            Some("FrgC") => json!("foreground"),
            Some("BckC") => json!("background"),
            Some(_) => return None,
        };
        let at = get_num(s, "Lctn")?;
        if !(previous..=4096.0).contains(&at) {
            return None;
        }
        previous = at;
        stops.push(json!([at / 4096.0, color]));
        if i > 0 {
            // The dialog's midpoints are 5..95 %; playback would clamp anything else.
            let mid = get_num(s, "Mdpn").unwrap_or(50.0);
            if !(5.0..=95.0).contains(&mid) {
                return None;
            }
            mids.push(mid / 100.0);
        }
    }
    p.insert("stops".into(), Value::Array(stops));
    if let Some(trns) = get_list(g, "Trns") {
        let (mut out, mut previous) = (Vec::new(), 0.0);
        for s in trns {
            let D::Descriptor(s) = s else { return None };
            if get_num(s, "Mdpn").is_some_and(|m| m != 50.0) {
                return None;
            }
            // Playback would clamp and sort what is out of range or out of order.
            let (at, opacity) = (get_num(s, "Lctn")?, get_num(s, "Opct")?);
            if !(previous..=4096.0).contains(&at) || !(0.0..=100.0).contains(&opacity) {
                return None;
            }
            previous = at;
            out.push(json!([at / 4096.0, opacity]));
        }
        p.insert("transparency".into(), Value::Array(out));
    }
    if mids.iter().all(|m| *m == 0.5) {
        mids.clear();
    }
    p.insert("midpoints".into(), json!(mids));
    Some(p)
}

/// [`gradient_params`] plus the recorded descriptor, for Save Actions.
fn gradient_step(t: &Descriptor) -> Option<Value> {
    let mut p = gradient_params(t)?;
    p.insert(GRADIENT_DESCRIPTOR.into(), photocraft_paint::tile::b64_encode(&t.to_bytes()).into());
    Some(Value::Object(p))
}

/// The `gradientLayer` descriptor for a Gradient Fill step: the recorded one while the params are
/// still what it imports as, else one written from the params. That needs every stop as
/// `#rrggbb` on a 1/4096 location, whole-percent midpoints and, for a Set (`set`), the
/// transparency stops: Photoshop's Set replaces the whole gradient, where the command keeps the
/// layer's own transparency.
fn gradient_desc(p: &Value, set: bool) -> Option<Descriptor> {
    if let Some(t) = jstr(p, GRADIENT_DESCRIPTOR).and_then(photocraft_paint::tile::b64_decode).and_then(|b| Descriptor::from_bytes(&b).ok()) {
        let (mut a, mut b) = (Value::Object(gradient_params(&t)?), p.clone());
        b.as_object_mut()?.remove(GRADIENT_DESCRIPTOR);
        super::convert::normalise(&mut a);
        super::convert::normalise(&mut b);
        if a == b {
            return Some(t);
        }
    }
    const KEYS: &[&str] = &["style", "angle", "reverse", "dither", "align", "scale", "offset", "stops", "transparency", "midpoints", GRADIENT_DESCRIPTOR];
    if p.as_object()?.iter().any(|(k, v)| !v.is_null() && !KEYS.contains(&k.as_str())) || (set && p.get("transparency").is_none()) {
        return None;
    }
    let location = |v: &Value| {
        let t = v.as_f64()? * 4096.0;
        ((0.0..=4096.0).contains(&t) && t.fract() == 0.0).then(|| long(t))
    };
    let stops = p.get("stops")?.as_array().filter(|s| (2..=1024).contains(&s.len()))?;
    let mids: Vec<f64> = match p.get("midpoints") {
        None | Some(Value::Null) => Vec::new(),
        Some(m) => m.as_array()?.iter().map(Value::as_f64).collect::<Option<_>>()?,
    };
    if mids.len() >= stops.len() {
        return None;
    }
    let (mut clrs, mut previous) = (Vec::new(), 0.0);
    for (i, s) in stops.iter().enumerate() {
        let [at, color] = s.as_array()?.as_slice() else { return None };
        if at.as_f64()? < previous {
            return None;
        }
        previous = at.as_f64()?;
        // A foreground or background stop is left out: Photoshop records the colour it had too.
        let color = color_desc(color.as_str()?)?;
        let mid = if i == 0 { 50.0 } else { mids.get(i - 1).copied().unwrap_or(0.5) * 100.0 };
        if (mid - mid.round()).abs() > 1e-9 || !(5.0..=95.0).contains(&mid.round()) {
            return None;
        }
        clrs.push(D::Descriptor(
            desc("Clrt").with("Clr ", D::Descriptor(color)).with("Type", enm("Clry", "UsrS")).with("Lctn", location(at)?).with("Mdpn", long(mid)),
        ));
    }
    let default_trns = vec![json!([0, 100]), json!([1, 100])];
    let trns: Option<Vec<D>> = p
        .get("transparency")
        .and_then(Value::as_array)
        .unwrap_or(&default_trns)
        .iter()
        .map(|s| {
            let [at, o] = s.as_array()?.as_slice() else { return None };
            Some(D::Descriptor(desc("TrnS").with("Opct", pct(o.as_f64()?)).with("Lctn", location(at)?).with("Mdpn", long(50.0))))
        })
        .collect();
    let grad = desc("Grdn")
        .with("Nm  ", text("Custom"))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(4096.0))
        .with("Clrs", D::List(clrs))
        .with("Trns", D::List(trns?));
    let mut t = desc("gradientLayer");
    if let Some(a) = jnum(p, "angle") {
        t = t.with("Angl", ang(a));
    }
    if let Some(s) = jstr(p, "style") {
        t = t.with("Type", enm("GrdT", lookup_back(GRADIENT_STYLES, s)?));
    }
    for (param, key) in [("reverse", "Rvrs"), ("dither", "Dthr"), ("align", "Algn")] {
        if let Some(v) = p.get(param).filter(|v| !v.is_null()) {
            t = t.with(key, D::Boolean(v.as_bool()?));
        }
    }
    if let Some(s) = jnum(p, "scale") {
        t = t.with("Scl ", pct(s));
    }
    if let Some(o) = p.get("offset").filter(|v| !v.is_null()) {
        let [x, y] = o.as_array()?.as_slice() else { return None };
        t = t.with("Ofst", D::Descriptor(desc("Pnt ").with("Hrzn", pct(x.as_f64()?)).with("Vrtc", pct(y.as_f64()?))));
    }
    Some(t.with("Grad", D::Descriptor(grad)))
}

/// `layer.newFillLayer.gradient` as a Make of a `contentLayer`. The two-colour form (`from`,
/// `to`) becomes the stops it plays as; without them it plays the toolbox colours, which
/// Photoshop records with the colour they had.
fn gradient_make_to(p: &Value) -> Option<Descriptor> {
    let mut p = p.clone();
    let o = p.as_object_mut()?;
    if o.remove("path").is_some_and(|v| !v.is_null()) {
        return None;
    }
    // Stops replace `from` and `to` when the step plays, so those two are left out beside them.
    match (o.remove("from").filter(|v| !v.is_null()), o.remove("to").filter(|v| !v.is_null())) {
        _ if o.contains_key("stops") => {}
        (None, None) => {}
        (Some(a), Some(b)) => {
            o.insert("stops".into(), json!([[0, a], [1, b]]));
        }
        _ => return None,
    }
    let t = gradient_desc(&p, false)?;
    Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t)))))
}

/// `layer.layerStyle.blendingOptions` as a Set of the target layer.
fn blending_options_to(p: &Value) -> Option<Descriptor> {
    let o = p.as_object()?;
    let blend_if = p.get("blendIf");
    let mut t = desc("Lyr ");
    if blend_if.is_some() {
        // With Blend If, the dialog's opacities and mode are in the same `set`; without it,
        // Photoshop's `set` of them loads as `layer.setProps`.
        for (param, key) in [("opacity", "Opct"), ("fillOpacity", "fillOpacity")] {
            if let Some(v) = jnum(p, param) {
                t = t.with(key, pct(v));
            }
        }
        if let Some(m) = jstr(p, "blend") {
            t = t.with("Md  ", enm("BlnM", blend_ps(m)?));
        }
    }
    for (key, param) in ADVANCED_BLENDING {
        if let Some(b) = jbool(p, param) {
            t = t.with(key, D::Boolean(b));
        }
    }
    if let Some(b) = blend_if {
        t = t.with("Blnd", blend_if_to(b)?);
        if let Some(fx) = jstr(p, EFFECTS_SCALE) {
            let fx = photocraft_paint::tile::b64_decode(fx).and_then(|b| Descriptor::from_bytes(&b).ok())?;
            if !scale_only(&fx) {
                return None;
            }
            t = t.with("Lefx", D::Descriptor(fx));
        }
    }
    // Only these export: Photoshop records the dialog's other options differently.
    if t.items.is_empty() || t.items.len() != o.len() {
        return None;
    }
    Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(t)))
}

/// Blend If: a `Blnd` list's `Chnl` ↔ `blendIf` channel (`Gry` is the composite).
fn blend_if_channel(r: &[ReferenceItem]) -> Option<&'static str> {
    match target(r) {
        Target::Ordinal(o) if o == "Cmps" => Some("gray"),
        Target::Ordinal(o) => channel_of(&o),
        _ => None,
    }
}

/// The slider keys of one `Blnd` entry: This Layer, then Underlying Layer, each black low and
/// high, white low and high.
const BLEND_IF_KEYS: [[&str; 4]; 2] = [["SrcB", "Srcl", "SrcW", "Srcm"], ["DstB", "Dstl", "DstW", "Dstt"]];

/// A `Blnd` list as `layer.layerStyle.blendingOptions`' `blendIf`. Each entry's sliders are
/// whole levels 0..255 (also in a 16-bit document); Underlying Layer may be missing. None for
/// anything else, so the step stays a kept Photoshop step.
fn blend_if_from(list: &[D]) -> Option<Value> {
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let D::Descriptor(e) = e else { return None };
        if e.items.iter().any(|(k, _)| !k.is("Chnl") && !BLEND_IF_KEYS.iter().flatten().any(|b| k.is(b))) {
            return None;
        }
        let channel = blend_if_channel(get_ref(e, "Chnl")?)?;
        let range = |keys: &[&str; 4]| -> Option<Option<Value>> {
            let v: Vec<Option<i32>> = keys
                .iter()
                .map(|k| match e.get(k) {
                    Some(D::Integer(i)) if (0..=255).contains(i) => Some(*i),
                    _ => None,
                })
                .collect();
            match v.iter().flatten().count() {
                0 if keys.iter().all(|k| e.get(k).is_none()) => Some(None),
                // In order, as the command takes them; an inverted range stays a Photoshop step.
                4 => {
                    let q: Vec<i32> = v.into_iter().flatten().collect();
                    q.windows(2).all(|w| w[0] <= w[1]).then(|| Some(json!(q)))
                }
                _ => None,
            }
        };
        let mut m = json!({"channel": channel, "thisLayer": range(&BLEND_IF_KEYS[0])??});
        if let Some(u) = range(&BLEND_IF_KEYS[1])? {
            m["underlying"] = u;
        }
        out.push(m);
    }
    (!out.is_empty()).then_some(Value::Array(out))
}

/// `blendIf` back as a `Blnd` list: named channels and whole levels only (an index or `null`
/// reset has no Photoshop form).
fn blend_if_to(v: &Value) -> Option<D> {
    let entries = match v {
        Value::Array(a) => a.iter().collect::<Vec<_>>(),
        Value::Object(_) => vec![v],
        _ => return None,
    };
    let mut list = Vec::with_capacity(entries.len());
    for e in entries {
        let o = e.as_object()?;
        if o.keys().any(|k| !matches!(k.as_str(), "channel" | "thisLayer" | "underlying")) {
            return None;
        }
        let channel = match e.get("channel") {
            None | Some(Value::Null) => "gray",
            Some(Value::String(c)) if c == "grey" => "gray",
            Some(Value::String(c)) => c.as_str(),
            _ => return None,
        };
        lookup_back(CHANNELS, channel)?;
        let mut d = desc("Blnd").with("Chnl", channel_ref(Some(channel)));
        for (param, keys) in [("thisLayer", &BLEND_IF_KEYS[0]), ("underlying", &BLEND_IF_KEYS[1])] {
            let Some(r) = e.get(param) else {
                if param == "thisLayer" {
                    return None;
                }
                continue;
            };
            let r: Vec<f64> = r.as_array()?.iter().map(Value::as_f64).collect::<Option<_>>()?;
            let q = match r.as_slice() {
                [b, w] => [*b, *b, *w, *w],
                [a, b, c, w] => [*a, *b, *c, *w],
                _ => return None,
            };
            if !q.iter().all(|x| (0.0..=255.0).contains(x) && x.fract() == 0.0) || !q.windows(2).all(|w| w[0] <= w[1]) {
                return None;
            }
            for (k, x) in keys.iter().zip(q) {
                d = d.with(k, long(x));
            }
        }
        list.push(D::Descriptor(d));
    }
    Some(D::List(list))
}

/// Layer › Layer Mask: the `UsrM` value of an Add Layer Mask `Mk` ↔ its command.
const MASK_USING: &[(&str, &str)] = &[
    ("RvlA", "layer.layerMask.revealAll"),
    ("HdAl", "layer.layerMask.hideAll"),
    ("RvlS", "layer.layerMask.revealSelection"),
    ("HdSl", "layer.layerMask.hideSelection"),
];

fn duplicate_layer_from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let r = get_ref(d, "null")?;
    if ref_class(r).as_deref() != Some("Dcmn") || !matches!(target(r), Target::Class) {
        return None;
    }
    let u = get_ref(d, "Usng")?;
    if !matches!(ref_class(u).as_deref(), Some("Lyr") | Some("layerSection")) {
        return None;
    }
    let mut params = layer_target(u)?;
    params.insert("document".into(), "new".into());
    if let Some(n) = get_text(d, "Nm  ") {
        params.insert("documentName".into(), n.into());
    }
    if let Some(n) = get_text(d, "LyrN") {
        params.insert("name".into(), n.into());
    }
    Some(step("layer.duplicate", Value::Object(params)))
}

fn mask_from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let Some(D::Class(c)) = d.get("Nw  ") else { return None };
    if trimmed(&c.class_id) != "Chnl" {
        return None;
    }
    let at_mask =
        get_ref(d, "At  ").is_some_and(|r| r.len() == 1 && ref_class(r).as_deref() == Some("Chnl") && matches!(target(r), Target::Ordinal(o) if o == "Msk"));
    Some(match get_enum(d, "Usng").and_then(|u| lookup(MASK_USING, &u)) {
        Some(id) if at_mask => none(id),
        _ => Mapped::Unsupported { harmless: false },
    })
}

/// Import New Document and selection-based Copy/Cut, retaining the new layer's options.
fn new_document_from_descriptor(d: &Descriptor) -> Option<Mapped> {
    let nw = get_obj(d, "Nw  ")?;
    if get_enum(d, "Usng").as_deref() == Some("Slct") {
        let id = if get_bool(d, "Cpy ") == Some(true) { "layer.new.layerViaCopy" } else { "layer.new.layerViaCut" };
        let mut mapped_steps = vec![(id, json!({}))];
        let mut props = Map::new();
        if let Some(n) = get_text(nw, "Nm  ") {
            props.insert("name".into(), n.into());
        }
        if nw.get("Opct").is_some() {
            if matches!(nw.get("Opct"), Some(D::UnitFloat { unit, .. }) if *unit != PCT) {
                return Some(Mapped::Unsupported { harmless: false });
            }
            let Some(o) = get_num(nw, "Opct").filter(|o| (0.0..=100.0).contains(o)) else {
                return Some(Mapped::Unsupported { harmless: false });
            };
            props.insert("opacity".into(), (o / 100.0).into());
        }
        if let Some(m) = get_enum(nw, "Md  ") {
            let Some(mode) = blend_label(&m) else { return Some(Mapped::Unsupported { harmless: false }) };
            props.insert("blend".into(), mode.into());
        }
        if let Some(c) = get_enum(nw, "Clr ") {
            let Some(color) = lookup(LABEL_COLORS, &c) else { return Some(Mapped::Unsupported { harmless: false }) };
            mapped_steps.push(("layer.setLabelColor", json!({"color": color})));
        }
        if !props.is_empty() {
            mapped_steps.push(("layer.setProps", Value::Object(props)));
        }
        return Some(steps(mapped_steps));
    }
    if get_text(nw, "preset").is_some() {
        return Some(none("file.newFromClipboard"));
    }
    let (Some(w), Some(h)) = (get_num(nw, "Wdth"), get_num(nw, "Hght")) else {
        return Some(Mapped::Unsupported { harmless: false });
    };
    let res = get_num(nw, "Rslt").unwrap_or(72.0);
    let to_px = |key: &str, value: f64| match nw.get(key) {
        Some(D::UnitFloat { unit: PX, .. }) => value,
        _ => value * res / 72.0,
    };
    let mut params = json!({"width": to_px("Wdth", w).round(), "height": to_px("Hght", h).round(), "resolution": res});
    if let Some(n) = get_text(nw, "Nm  ") {
        params["name"] = json!(n);
    }
    if let Some(D::Class(c)) = nw.get("Md  ")
        && let Some(m) = lookup(MODES, &trimmed(&c.class_id))
    {
        params["mode"] = json!(m);
    }
    if let Some(depth) = get_num(nw, "Dpth") {
        params["depth"] = json!(depth);
    }
    if let Some(fill) = get_enum(nw, "Fl  ") {
        params["background"] = json!(match fill.as_str() {
            "Wht" => "white",
            "BckC" => "backgroundColor",
            "Trns" => "transparent",
            _ => "white",
        });
    }
    Some(step("file.new", params))
}

/// New Snapshot's From: the full document, the layers merged, or the current layer.
const SNAPSHOT_FROM: &[(&str, &str)] = &[("FllD", "fullDocument"), ("MrgL", "mergedLayers"), ("CrnL", "currentLayer")];

fn type_shape_descriptor() -> Descriptor {
    desc("null").with("null", class_ref("contentLayer")).with("Usng", target_ref("textLayer"))
}

/// Make events another module maps: a type path, an artboard, a layer style or a frame,
/// tried in that order.
fn delegated_make_from(d: &Descriptor) -> Option<Mapped> {
    let class = get_ref(d, "null").and_then(ref_class);
    match class.as_deref() {
        Some("Path") => return Some(super::steps_type_path::from(d)),
        // Layer › New › Artboard and Artboard from Layers (#10723).
        Some("artboardSection") => return Some(super::steps_artboard_section::from(d)),
        _ => {}
    }
    if let Some(mapped) = super::steps_layer_style::create_from_descriptor(d) {
        return Some(mapped);
    }
    // Layer › New › Frame from Layers (#10723).
    (class.as_deref() == Some("framedGroupSection")).then(|| super::steps_frame_section::from(d))
}

/// Import Make events only when their geometry and options can be represented faithfully,
/// including the exact target-text-to-content-layer references.
pub(super) fn make_from(d: &Descriptor) -> Mapped {
    if let Some(mapped) = super::steps_calculations::from_descriptor(d) {
        return mapped;
    }
    if let Some(mapped) = delegated_make_from(d) {
        return mapped;
    }
    if d.items == type_shape_descriptor().items {
        return none("type.convertToShape");
    }
    if let Some(mapped) = duplicate_layer_from_descriptor(d) {
        return mapped;
    }
    if let Some(mapped) = mask_from_descriptor(d) {
        return mapped;
    }
    if let Some(mapped) = new_document_from_descriptor(d) {
        return mapped;
    }
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    let using = get_obj(d, "Usng");
    let type_desc = using.and_then(|u| get_obj(u, "Type"));
    match ref_class(r).as_deref() {
        Some("Lyr") => {
            let mut p = Map::new();
            if let Some(u) = using {
                if let Some(n) = get_text(u, "Nm  ") {
                    p.insert("name".into(), n.into());
                }
                if let Some(o) = get_num(u, "Opct") {
                    p.insert("opacity".into(), (o / 100.0).into());
                }
                if let Some(m) = blend_mode(u) {
                    p.insert("mode".into(), m.into());
                }
                match u.get("Grup") {
                    Some(D::Boolean(true)) => {
                        p.insert("clipToPrevious".into(), true.into());
                    }
                    Some(D::Boolean(false)) | None => {}
                    _ => return Mapped::Unsupported { harmless: false },
                }
            }
            step("layer.new.layer", Value::Object(p))
        }
        Some("AdjL") => match type_desc.and_then(|t| adjustment_by_class(&trimmed(&t.class_id))) {
            Some(a) => {
                let Mapped::Steps(mut mapped) = adjustment_steps(a, "layer.newAdjustmentLayer.", type_desc) else {
                    return Mapped::Unsupported { harmless: false };
                };
                if let Some(u) = using
                    && u.get("Opct").is_some()
                {
                    if matches!(u.get("Opct"), Some(D::UnitFloat { unit, .. }) if *unit != PCT) {
                        return Mapped::Unsupported { harmless: false };
                    }
                    let Some(opacity) = get_num(u, "Opct").filter(|o| (0.0..=100.0).contains(o)) else {
                        return Mapped::Unsupported { harmless: false };
                    };
                    let Some((_, Value::Object(params))) = mapped.first_mut() else { return Mapped::Unsupported { harmless: false } };
                    // Recorded layer opacity uses 0..255 even in a 16-bit document.
                    params.insert("opacity".into(), ((opacity * 255.0 / 100.0).round() / 255.0).into());
                }
                // Name, blend mode and clipping travel on the same command; an unreadable value
                // keeps the whole step rather than replaying a different layer (#8384).
                if let Some(u) = using {
                    let Some((_, Value::Object(params))) = mapped.first_mut() else { return Mapped::Unsupported { harmless: false } };
                    if u.get("Nm  ").is_some() {
                        let Some(name) = exact_name(u) else { return Mapped::Unsupported { harmless: false } };
                        params.insert("name".into(), name.into());
                    }
                    if u.get("Md  ").is_some() {
                        let Some(mode) = blend_mode(u).filter(|m| *m != "Pass Through") else {
                            return Mapped::Unsupported { harmless: false };
                        };
                        params.insert("mode".into(), mode.into());
                    }
                    match u.get("Grup") {
                        None => {}
                        Some(D::Boolean(clip)) => {
                            params.insert("clipToPrevious".into(), (*clip).into());
                        }
                        Some(_) => return Mapped::Unsupported { harmless: false },
                    }
                }
                if let Some(c) = using.and_then(|u| get_enum(u, "Clr ")) {
                    let Some(color) = lookup(LABEL_COLORS, &c) else { return Mapped::Unsupported { harmless: false } };
                    mapped.push(("layer.setLabelColor".into(), json!({"color": color})));
                }
                Mapped::Steps(mapped)
            }
            None => Mapped::Unsupported { harmless: false },
        },
        Some("contentLayer") if d.get("Shp ").is_some() || using.is_some_and(|u| u.get("Shp ").is_some() || u.get("strokeStyle").is_some()) => {
            // A shape's geometry and stroke cannot be represented by a canvas-wide fill.
            Mapped::Unsupported { harmless: false }
        }
        Some("contentLayer") => match type_desc {
            Some(t) if t.class_id.is("solidColorLayer") => match get_obj(t, "Clr ").and_then(color_hex) {
                Some(c) => step("layer.newFillLayer.solidColor", json!({"color": c})),
                None => none("layer.newFillLayer.solidColor"),
            },
            Some(t) if t.class_id.is("patternLayer") => match pattern_params(t) {
                Some(p) => step("layer.newFillLayer.pattern", Value::Object(p)),
                None => Mapped::Unsupported { harmless: false },
            },
            // A name, opacity, mode or clipping beside `Type` would neither play nor save back.
            Some(t) if t.class_id.is("gradientLayer") => match gradient_step(t) {
                Some(p) if using.is_some_and(|u| u.items.iter().all(|(k, _)| k.is("Type"))) => step("layer.newFillLayer.gradient", p),
                _ => Mapped::Unsupported { harmless: false },
            },
            _ => Mapped::Unsupported { harmless: false },
        },
        Some("layerSection") => {
            let mut p = Map::new();
            if let Some(n) = using.and_then(|u| get_text(u, "Nm  ")).or_else(|| get_text(d, "Nm  ")) {
                p.insert("name".into(), n.into());
            }
            match get_ref(d, "From") {
                // A named source layer stays the source (#7236); a target reference and an
                // index/id source (our exporter never writes those) keep the old implicit
                // import: the group lands on the current selection at playback.
                Some(r) => {
                    if let Some(t) = layer_target(r) {
                        p.extend(t);
                    }
                    step("layer.groupLayers", Value::Object(p))
                }
                None => step("layer.new.group", Value::Object(p)),
            }
        }
        // History panel › New Snapshot (of the current state).
        Some("SnpS") => {
            let mut p = Map::new();
            if let Some(n) = get_text(d, "Nm  ") {
                p.insert("name".into(), n.into());
            }
            match get_enum(d, "Usng").as_deref().map(|u| lookup(SNAPSHOT_FROM, u)) {
                None => {}
                Some(Some(f)) => {
                    p.insert("from".into(), f.into());
                }
                Some(None) => return Mapped::Unsupported { harmless: false },
            }
            if get_bool(d, "replaceExisting") == Some(true) {
                p.insert("replaceExisting".into(), true.into());
            }
            step("history.newSnapshot", Value::Object(p))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// The percent to save for an adjustment layer's opacity. Loading quantises a recorded percent
/// to 8 bits (70 % becomes 179/255); a whole percent that quantises to that same level is the
/// one that was recorded, so it is saved back (load then save keeps `70`, not `70.196`).
fn recorded_percent(opacity: f64) -> f64 {
    let level = opacity * 255.0;
    let percent = opacity * 100.0;
    let whole = percent.round();
    if (level - level.round()).abs() < 1e-6 && (whole * 255.0 / 100.0).round() == level.round() { whole } else { percent }
}

/// Write a creation step, refusing native options the target descriptor cannot carry; Type to
/// Shape exports only without native options.
pub(super) fn make_to(id: &str, p: &Value) -> Option<Descriptor> {
    if id == "image.calculations" {
        return super::steps_calculations::to_descriptor(p);
    }
    if id == super::steps_type_path::COMMAND {
        return super::steps_type_path::to(p);
    }
    if matches!(id, super::steps_artboard_section::NEW | super::steps_artboard_section::FROM_LAYERS) {
        return super::steps_artboard_section::to(id, p);
    }
    if id == super::steps_layer_style::CREATE {
        return super::steps_layer_style::style_to(id, p);
    }
    if id == super::steps_frame_section::COMMAND {
        return super::steps_frame_section::to(p);
    }
    if id == "type.convertToShape" {
        return p.as_object().filter(|p| p.is_empty()).map(|_| type_shape_descriptor());
    }
    let with_name = |cls: &str| {
        let mut u = desc(cls);
        if let Some(n) = jstr(p, "name") {
            u = u.with("Nm  ", text(n));
        }
        u
    };
    Some(match id {
        "layer.new.layer" => {
            // The descriptor carries the name, opacity, mode and clipping flag. A step that
            // labels the layer, fills it with the neutral colour or places it
            // below the active layer would replay as a different layer, so it isn't exported
            // (strict export fails, a partial one skips it: #7512, #7513, #7514).
            let set = |key: &str, default: Value| p.get(key).is_some_and(|v| !v.is_null() && *v != default);
            if p.get("clipToPrevious").is_some_and(|v| !v.is_null() && !v.is_boolean())
                || set("color", json!("none"))
                || set("fillNeutral", Value::Bool(false))
                || set("position", json!("above"))
            {
                return None;
            }
            let mut u = with_name("Lyr ");
            if jbool(p, "clipToPrevious") == Some(true) {
                u = u.with("Grup", D::Boolean(true));
            }
            if let Some(o) = jnum(p, "opacity") {
                u = u.with("Opct", pct(o * 100.0));
            }
            if let Some(m) = jstr(p, "mode").and_then(blend_ps) {
                u = u.with("Md  ", enm("BlnM", m));
            }
            desc("null").with("null", class_ref("Lyr ")).with("Usng", D::Descriptor(u))
        }
        "layer.new.group" => desc("null").with("null", class_ref("layerSection")).with("Usng", D::Descriptor(with_name("layerSection"))),
        "layer.groupLayers" => {
            // An explicit source layer goes in by name like every other layer reference; a
            // step without one keeps the target-layer form (#7236).
            desc("null").with("null", class_ref("layerSection")).with("From", layer_ref(p)?).with("Usng", D::Descriptor(with_name("layerSection")))
        }
        "layer.new.layerViaCopy" | "layer.new.layerViaCut" => {
            let mut d = desc("null").with("Nw  ", D::Descriptor(desc("Lyr "))).with("Usng", enm("ArSl", "Slct"));
            if id == "layer.new.layerViaCopy" {
                d = d.with("Cpy ", D::Boolean(true));
            }
            d
        }
        "layer.newFillLayer.solidColor" => {
            let mut t = desc("solidColorLayer");
            // The descriptor's colour is opaque RGB: a colour with alpha, or one it can't parse,
            // would replay as the default black (#7507).
            if let Some(c) = p.get("color").filter(|v| !v.is_null()) {
                t = t.with("Clr ", D::Descriptor(c.as_str().and_then(color_desc)?));
            }
            desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))
        }
        "layer.newFillLayer.pattern" => {
            if p.get("path").is_some() {
                return None;
            }
            let t = pattern_desc(p)?;
            desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))
        }
        "layer.newFillLayer.gradient" => gradient_make_to(p)?,
        "layer.duplicate" => {
            // Duplicate into a new document is a `Mk`, not a `Dplc` (see `make_from`).
            // Anything else falls through to the `Dplc` event below.
            if jstr(p, "document") != Some("new") {
                return None;
            }
            let mut d = desc("null").with("null", class_ref("Dcmn")).with("Usng", layer_ref(p)?);
            if let Some(n) = jstr(p, "documentName") {
                d = d.with("Nm  ", text(n));
            }
            if let Some(n) = jstr(p, "name") {
                d = d.with("LyrN", text(n));
            }
            d
        }
        "layer.layerMask.revealAll" | "layer.layerMask.hideAll" | "layer.layerMask.revealSelection" | "layer.layerMask.hideSelection" => {
            // Photoshop adds the mask to the target layer only.
            if p.get("layer").is_some() {
                return None;
            }
            let at = D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Msk ") }]);
            desc("null").with("Nw  ", D::Class(class("Chnl"))).with("At  ", at).with("Usng", enm("UsrM", lookup_back(MASK_USING, id)?))
        }
        "file.newFromClipboard" => desc("null").with("Nw  ", D::Descriptor(desc("Dcmn").with("preset", text("Clipboard")))),
        "history.newSnapshot" => {
            let current = D::Reference(vec![ReferenceItem::Property { class: class("HstS"), key: code("CrnH") }]);
            let mut d = desc("null").with("null", class_ref("SnpS")).with("From", current);
            if let Some(n) = jstr(p, "name") {
                d = d.with("Nm  ", text(n));
            }
            d = d.with("Usng", enm("HstS", lookup_back(SNAPSHOT_FROM, jstr(p, "from").unwrap_or("fullDocument"))?));
            if jbool(p, "replaceExisting") == Some(true) {
                d = d.with("replaceExisting", D::Boolean(true));
            }
            d
        }
        "file.new" => {
            let mut n = desc("Dcmn");
            if let Some(name) = jstr(p, "name") {
                n = n.with("Nm  ", text(name));
            }
            if let Some(m) = jstr(p, "mode").and_then(|m| lookup_back(MODES, if m == "gray" { "grayscale" } else { m })) {
                n = n.with("Md  ", D::Class(class(m)));
            }
            n = n.with("Wdth", px(jnum(p, "width").unwrap_or(1920.0))).with("Hght", px(jnum(p, "height").unwrap_or(1080.0)));
            n = n.with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: jnum(p, "resolution").unwrap_or(72.0) });
            // `Fl  ` is white, transparent or Photoshop's own background colour: black, a hex
            // colour or a given `backgroundColor` would replay as something else (#7503).
            if p.get("backgroundColor").is_some_and(|v| !v.is_null()) {
                return None;
            }
            let fill = match p.get("background").filter(|v| !v.is_null()) {
                None => "Wht ",
                Some(b) => match b.as_str()? {
                    "white" => "Wht ",
                    "transparent" => "Trns",
                    "backgroundColor" => "BckC",
                    _ => return None,
                },
            };
            n = n.with("Fl  ", enm("Fl  ", fill)).with("Dpth", long(jnum(p, "depth").unwrap_or(8.0)));
            desc("null").with("Nw  ", D::Descriptor(n))
        }
        _ => return adjustment_layer_to(id, p),
    })
}

/// `layer.newAdjustmentLayer.*` as a Make step: the adjustment's settings under `Type`, the new
/// layer's opacity, name, blend mode and clipping beside it.
fn adjustment_layer_to(id: &str, p: &Value) -> Option<Descriptor> {
    let suffix = id.strip_prefix("layer.newAdjustmentLayer.")?;
    let a = adjustment_by_suffix(suffix)?;
    let opacity = match p.get("opacity") {
        None => None,
        Some(value) => Some(value.as_f64().filter(|o| o.is_finite() && (0.0..=1.0).contains(o))?),
    };
    // Layer properties go on the `Usng` layer, not into the adjustment's own settings; a
    // value the reference application cannot record makes the step unexportable (#8384).
    let name = match p.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str()?),
    };
    let mode = match p.get("mode") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let label = v.as_str()?;
            if crate::commands::blend_from_str(label)? == photocraft_doc::BlendMode::PassThrough {
                return None;
            }
            Some(blend_ps(label)?)
        }
    };
    let clip = match p.get("clipToPrevious") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool()?),
    };
    let mut params = p.clone();
    if let Some(params) = params.as_object_mut() {
        for key in ["opacity", "name", "mode", "clipToPrevious"] {
            params.remove(key);
        }
    }
    // Layer opacity must not turn default adjustment settings into a custom preset.
    let t = if params.as_object().is_some_and(|o| o.is_empty()) {
        desc(a.class).with("presetKind", enm("presetKindType", "presetKindDefault"))
    } else {
        (a.to)(&params)?
    };
    let mut using = desc("AdjL").with("Type", D::Descriptor(t));
    if let Some(opacity) = opacity {
        using = using.with("Opct", pct(recorded_percent(opacity)));
    }
    if let Some(name) = name {
        using = using.with("Nm  ", text(name));
    }
    if let Some(mode) = mode {
        using = using.with("Md  ", enm("BlnM", mode));
    }
    if let Some(clip) = clip {
        using = using.with("Grup", D::Boolean(clip));
    }
    Some(desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(using)))
}

/// Import selection geometry or a channel source, including its optional source layer.
fn selection_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    match d.get("T   ") {
        Some(D::Enumerated { value, .. }) => match trimmed(value).as_str() {
            "Al" => none("select.all"),
            "None" => none("select.deselect"),
            "Prvs" => none("select.reselect"),
            _ => unsupported,
        },
        Some(D::Reference(t)) => match (ref_class(t).as_deref(), target(t)) {
            (Some("Chnl"), Target::Ordinal(o)) if o == "Trsp" => {
                let mut p = Map::new();
                p.insert("channel".into(), "transparency".into());
                if t.len() > 1 {
                    if t.len() != 2 || ref_class(&t[1..]).as_deref() != Some("Lyr") {
                        return unsupported;
                    }
                    let Some(mut layer) = layer_target(&t[1..]) else { return unsupported };
                    if let Some(Value::String(name)) = layer.get("layer")
                        && (name.is_empty() || name == "merged")
                    {
                        let selector = json!({"name": name});
                        layer.insert("layer".into(), selector);
                    }
                    p.extend(layer);
                }
                step("select.loadSelection", Value::Object(p))
            }
            // ⌘-click on the targeted channel (a layer mask after `select` of `Msk`).
            (Some("Chnl"), Target::Ordinal(o)) if o == "Trgt" => step("select.loadSelection", json!({"channel": "targeted"})),
            (Some("Chnl"), Target::Name(n)) => step("select.loadSelection", json!({"channel": n})),
            _ => unsupported,
        },
        Some(D::Descriptor(t)) if t.class_id.is("Plgn") => {
            let pts: Option<Vec<Value>> = get_list(t, "Pts ")
                .unwrap_or(&[])
                .iter()
                .map(|v| match v {
                    D::Descriptor(pt) => Some(json!([get_num(pt, "Hrzn")?, get_num(pt, "Vrtc")?])),
                    _ => None,
                })
                .collect();
            match pts {
                Some(pts) if pts.len() >= 3 => step("select.lasso", json!({"points": pts})),
                _ => unsupported,
            }
        }
        Some(D::Descriptor(t)) if t.class_id.is("Rctn") || t.class_id.is("Elps") => {
            let (Some(top), Some(left), Some(bottom), Some(right)) = (get_num(t, "Top "), get_num(t, "Left"), get_num(t, "Btom"), get_num(t, "Rght")) else {
                return unsupported;
            };
            let mut p = json!({"x": left.round(), "y": top.round(), "width": (right - left).round().max(0.0), "height": (bottom - top).round().max(0.0)});
            if t.class_id.is("Elps") {
                p["ellipse"] = json!(true);
            }
            step("select.rect", p)
        }
        _ => unsupported,
    }
}

fn background_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    // On an image that opened as a normal layer there is no Background to convert;
    // Photoshop stops there, but the step's purpose (an editable layer) is met.
    // Photoshop's `set` of another layer doesn't target it: the selected layer stays.
    let mut v = vec![("layer.new.layerFromBackground", json!({"ifPresent": true, "select": false}))];
    let mut props = Map::new();
    props.insert("layer".into(), "Layer 0".into());
    if let Some(t) = get_obj(d, "T   ") {
        if let Some(n) = get_text(t, "Nm  ") {
            props.insert("name".into(), n.into());
        }
        if let Some(o) = get_num(t, "Opct").filter(|o| (o - 100.0).abs() > 1e-9) {
            props.insert("opacity".into(), (o / 100.0).into());
        }
        match get_enum(t, "Md  ").as_deref() {
            None | Some("Nrml") => {}
            Some(m) => match blend_label(m) {
                Some(label) => {
                    props.insert("blend".into(), label.into());
                }
                None => return unsupported,
            },
        }
    }
    if props.len() > 1 {
        v.push(("layer.setProps", Value::Object(props)));
    }
    steps(v)
}

/// The Layers panel lock buttons (`layerLocking`) ↔ `layer.lockLayers` params.
const LOCKS: &[(&str, &str)] = &[
    ("protectTransparency", "transparency"),
    ("protectComposite", "pixels"),
    ("protectPosition", "position"),
    ("protectArtboardAutonest", "artboard"),
    ("protectAll", "all"),
];

/// The Blending Options keys besides the Advanced checkboxes that `layer.layerStyle.blendingOptions`
/// takes from a `set` of the layer: opacity, fill opacity, mode and Blend If.
const BLENDING_DIALOG: &[&str] = &["Opct", "fillOpacity", "Md  ", "Blnd"];

/// The hidden `layer.layerStyle.blendingOptions` param that keeps the `Lefx` = {Scl} the Layer
/// Style dialog's OK records beside Blend If, so Save Actions writes the Set back unchanged.
const EFFECTS_SCALE: &str = "__atnEffectsScale";

/// A `Lefx` holding only the effects' scale: what the dialog's OK records on a layer without
/// effects. It changes nothing.
fn scale_only(fx: &Descriptor) -> bool {
    fx.items.iter().all(|(k, _)| k.is("Scl "))
}

/// A `T` key that is such a `Lefx`.
fn is_scale_only(k: &Id, v: &D) -> bool {
    matches!(v, D::Descriptor(fx) if k.is("Lefx") && scale_only(fx))
}

/// Blending Options' Advanced checkboxes ↔ `layer.layerStyle.blendingOptions` params.
const ADVANCED_BLENDING: &[(&str, &str)] = &[("blendClipped", "blendClippedLayersAsGroup"), ("transparencyShapesLayer", "transparencyShapesLayer")];

fn lock_params(locks: &Descriptor) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    // "Unlock all" clears every lock.
    if get_bool(locks, "protectNone") == Some(true) {
        for (_, param) in LOCKS {
            m.insert((*param).into(), false.into());
        }
    }
    for (key, param) in LOCKS {
        if let Some(b) = get_bool(locks, key) {
            m.insert((*param).into(), b.into());
        }
    }
    (!m.is_empty()).then_some(m)
}

/// `applyLocking`: a lock button in the Layers panel, on the target layer.
pub(super) fn locking_from(d: &Descriptor) -> Mapped {
    let on_target = get_ref(d, "null").is_none_or(|r| ref_class(r).as_deref() == Some("Lyr") && matches!(target(r), Target::Ordinal(o) if o == "Trgt"));
    match get_obj(d, "layerLocking").and_then(lock_params) {
        Some(l) if on_target => step("layer.lockLayers", Value::Object(l)),
        _ => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn locking_to(_: &str, p: &Value) -> Option<Descriptor> {
    let mut locks = desc("layerLocking");
    for (key, param) in LOCKS {
        if let Some(b) = jbool(p, param) {
            locks = locks.with(key, D::Boolean(b));
        }
    }
    if locks.items.is_empty() {
        return None;
    }
    Some(desc("null").with("null", target_ref("Lyr ")).with("layerLocking", D::Descriptor(locks)))
}

fn layer_set_from(d: &Descriptor, r: &[ReferenceItem]) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(mut p) = layer_target(r) else { return unsupported };
    let Some(t) = get_obj(d, "T   ") else { return unsupported };
    let mut v = Vec::new();
    if let Some(c) = get_enum(t, "Clr ").and_then(|c| lookup(LABEL_COLORS, &c)) {
        let mut lp = p.clone();
        lp.insert("color".into(), c.into());
        v.push(("layer.setLabelColor", Value::Object(lp)));
    }
    if let Some(n) = get_text(t, "Nm  ") {
        p.insert("name".into(), n.into());
    }
    if let Some(o) = get_num(t, "Opct") {
        p.insert("opacity".into(), (o / 100.0).into());
    }
    if let Some(f) = get_num(t, "fillOpacity") {
        p.insert("fill".into(), (f / 100.0).into());
    }
    if let Some(m) = get_enum(t, "Md  ") {
        let Some(label) = blend_label(&m) else { return unsupported };
        p.insert("blend".into(), label.into());
    }
    if let Some(vis) = get_bool(t, "Vsbl") {
        p.insert("visible".into(), vis.into());
    }
    // Lock buttons and Blending Options' Advanced checkboxes act on the target layer.
    let targeted = !p.contains_key("layer");
    if let Some(locks) = get_obj(t, "layerLocking") {
        match lock_params(locks) {
            Some(l) if targeted => v.push(("layer.lockLayers", Value::Object(l))),
            _ => return unsupported,
        }
    }
    let mut advanced = Map::new();
    for (key, param) in ADVANCED_BLENDING {
        if let Some(b) = get_bool(t, key) {
            advanced.insert((*param).into(), b.into());
        }
    }
    if let Some(list) = get_list(t, "Blnd") {
        // Effects beside Blend If have no step here: keep the recorded step rather than play
        // and save it back without them.
        if t.items.iter().any(|(k, v)| k.is("Lefx") && !is_scale_only(k, v)) {
            return unsupported;
        }
        let Some(b) = blend_if_from(list) else { return unsupported };
        advanced.insert("blendIf".into(), b);
        // The Blending Options dialog's OK: with nothing but its own options, the opacities and
        // the mode go with Blend If into one step, saved back as the one `set`.
        if t.items.iter().all(|(k, v)| BLENDING_DIALOG.iter().any(|b| k.is(b)) || ADVANCED_BLENDING.iter().any(|(a, _)| k.is(a)) || is_scale_only(k, v)) {
            if let Some(D::Descriptor(fx)) = t.get("Lefx") {
                advanced.insert(EFFECTS_SCALE.into(), photocraft_paint::tile::b64_encode(&fx.to_bytes()).into());
            }
            for (key, param) in [("Opct", "opacity"), ("fillOpacity", "fillOpacity")] {
                if let Some(v) = get_num(t, key) {
                    advanced.insert(param.into(), v.into());
                }
            }
            p.remove("opacity");
            p.remove("fill");
            if let Some(b) = p.remove("blend") {
                advanced.insert("blend".into(), b);
            }
        }
    }
    if !advanced.is_empty() {
        if !targeted {
            return unsupported;
        }
        v.push(("layer.layerStyle.blendingOptions", Value::Object(advanced)));
    }
    if p.keys().any(|k| k != "layer") {
        v.push(("layer.setProps", Value::Object(p)));
    }
    // The Layer Style dialog's OK on a layer without effects records only the effects'
    // scale (`Lefx` = {Scl}); an empty `T` is an OK that changed nothing. Neither changes
    // the document.
    let no_effect = t.items.iter().all(|(k, val)| is_scale_only(k, val));
    if !v.is_empty() {
        steps(v)
    } else if no_effect {
        Mapped::Unsupported { harmless: true }
    } else {
        unsupported
    }
}

pub(super) fn set_from(d: &Descriptor) -> Mapped {
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    let unsupported = Mapped::Unsupported { harmless: false };
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Path"), Target::Property(p)) if p == "WrPt" => super::steps_path::from(d),
        (Some("Chnl"), Target::Property(p)) if p == "fsel" => selection_from(d),
        // Quick Mask mode on (`set` of the document's quick-mask property); `clearEvent` turns it off.
        (Some("Prpr"), Target::Property(p)) if p == "QucM" => step("select.editInQuickMaskMode", json!({"on": true})),
        // Image › Analysis › Set Measurement Scale › Custom (#10723).
        (Some("Prpr"), Target::Property(p)) if p == "measurementScale" => super::steps_measurement::from(d),
        // Layer › New › Layer From Background (Photoshop records the New Layer dialog's values
        // as a `set` of the Background); values other than the defaults then apply to the layer.
        (Some("Lyr"), Target::Property(p)) if p == "Bckg" => background_from(d),
        // The toolbox colours.
        (Some("Clr"), Target::Property(p)) if p == "FrgC" || p == "BckC" => match get_obj(d, "T   ").and_then(color_hex) {
            Some(hex) => step("tools.setColors", json!({(if p == "FrgC" { "foreground" } else { "background" }): hex})),
            None => unsupported,
        },
        (Some("Lyr"), _) => layer_set_from(d, r),
        // Layer › Smart Filter › Blending Options (`indx` counts the filters from 1, bottom first).
        (Some("filterFX"), _) => {
            let index = match r.first() {
                Some(ReferenceItem::Index { index, .. }) if *index >= 1 => index - 1,
                _ => return unsupported,
            };
            let on_target =
                matches!(r.get(1..), Some(rest) if ref_class(rest).as_deref() == Some("Lyr") && matches!(target(rest), Target::Ordinal(o) if o == "Trgt"));
            if !on_target {
                return unsupported;
            }
            let Some(opts) = get_obj(d, "filterFX").and_then(|f| get_obj(f, "blendOptions")) else { return unsupported };
            let mut p = json!({"index": index});
            if let Some(o) = get_num(opts, "Opct") {
                p["opacity"] = json!(o / 100.0);
            }
            if let Some(m) = get_enum(opts, "Md  ") {
                let Some(label) = blend_label(&m) else { return unsupported };
                p["blend"] = json!(label);
            }
            step("layer.smartFilter.blendingOptions", p)
        }
        // A Pattern Fill layer's pattern (double-click its thumbnail).
        (Some("contentLayer"), Target::Ordinal(o)) if o == "Trgt" => match get_obj(d, "T   ") {
            Some(t) if t.class_id.is("patternLayer") => match pattern_params(t) {
                Some(p) if !p.is_empty() => step("layer.fill.pattern.set", Value::Object(p)),
                _ => unsupported,
            },
            // Photoshop's Set replaces the whole gradient; one without transparency stops would
            // keep the layer's own on play, and could not be written back once edited.
            Some(t) if t.class_id.is("gradientLayer") => match gradient_step(t).filter(|p| p.get("transparency").is_some()) {
                Some(p) => step("gradient.fill.set", p),
                None => unsupported,
            },
            _ => unsupported,
        },
        (Some("AdjL"), Target::Ordinal(o)) if o == "Trgt" => {
            match get_obj(d, "T   ").and_then(|t| adjustment_by_class(&trimmed(&t.class_id)).map(|a| (a, t))) {
                Some((a, t)) => match (a.from)(t) {
                    // `kind` names the adjustment so Save Actions can write the class back.
                    // Not on an empty set (Photoshop's "reset to default"): params with any key
                    // merge, none reset.
                    Some(mut p) => {
                        if let Some(o) = p.as_object_mut().filter(|o| !o.is_empty()) {
                            o.insert("kind".into(), json!(a.suffix));
                        }
                        step("layer.setAdjustment", p)
                    }
                    None => unsupported,
                },
                None => unsupported,
            }
        }
        _ => unsupported,
    }
}

fn layer_set_to(p: &Value) -> Option<Descriptor> {
    // Only these keys map onto the `Lyr ` descriptor; locks, clipping, Advanced Blending
    // channels or an explicit `layers` list would be dropped (#7196, #7246).
    const MAPPED: [&str; 7] = ["layer", "name", "opacity", "fill", "blend", "visible", "color"];
    if p.as_object().is_some_and(|m| m.iter().any(|(k, v)| !v.is_null() && !MAPPED.contains(&k.as_str()))) {
        return None;
    }
    let mut t = desc("Lyr ");
    if let Some(n) = jstr(p, "name") {
        t = t.with("Nm  ", text(n));
    }
    if let Some(o) = jnum(p, "opacity") {
        t = t.with("Opct", pct(o * 100.0));
    }
    if let Some(f) = jnum(p, "fill") {
        t = t.with("fillOpacity", pct(f * 100.0));
    }
    if let Some(m) = jstr(p, "blend") {
        t = t.with("Md  ", enm("BlnM", blend_ps(m)?));
    }
    if let Some(v) = jbool(p, "visible") {
        t = t.with("Vsbl", D::Boolean(v));
    }
    if let Some(c) = jstr(p, "color") {
        t = t.with("Clr ", enm("Clr ", lookup_back(LABEL_COLORS, c)?));
    }
    if t.items.is_empty() {
        return None;
    }
    Some(desc("null").with("null", layer_ref(p)?).with("T   ", D::Descriptor(t)))
}

/// Encode selection and colour-setting steps as Photoshop Set descriptors.
pub(super) fn set_to(id: &str, p: &Value) -> Option<Descriptor> {
    let fsel = D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]);
    let sel = |t: D| desc("null").with("null", fsel.clone()).with("T   ", t);
    Some(match id {
        "path.set" => return super::steps_path::to(p),
        super::steps_measurement::SET_SCALE => return super::steps_measurement::to(p),
        "select.all" => sel(enm("Ordn", "Al  ")),
        "select.deselect" => sel(enm("Ordn", "None")),
        "select.reselect" => sel(enm("Ordn", "Prvs")),
        // The descriptor loads a plain new selection: inverting it or combining it with the current
        // one (add, subtract, intersect) would be lost (#7506).
        "select.loadSelection"
            if p.get("invert").is_some_and(|v| *v != Value::Bool(false) && !v.is_null())
                || p.get("operation").is_some_and(|v| *v != json!("new") && !v.is_null()) =>
        {
            return None;
        }
        "select.loadSelection" => match jstr(p, "channel")? {
            "transparency" => {
                let mut r = vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") }];
                if p.get("layer").is_some_and(|v| !v.is_null()) {
                    let reference = match p.get("layer") {
                        // These native source defaults mean the active layer for transparency.
                        Some(Value::String(name)) if name.is_empty() || name == "merged" => target_ref("Lyr "),
                        Some(Value::Object(selector)) if selector.len() == 1 && selector.contains_key("name") => {
                            name_ref("Lyr ", selector.get("name")?.as_str()?)
                        }
                        _ => layer_ref(p)?,
                    };
                    let D::Reference(layer) = reference else { return None };
                    r.extend(layer);
                }
                sel(D::Reference(r))
            }
            "targeted" => sel(D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Ordn"), value: Id::new("Trgt") }])),
            name => sel(name_ref("Chnl", name)),
        },
        "select.lasso" => {
            let pts: Option<Vec<D>> = p
                .get("points")?
                .as_array()?
                .iter()
                .map(|pt| {
                    let a = pt.as_array()?;
                    Some(D::Descriptor(desc("Pnt ").with("Hrzn", px(a.first()?.as_f64()?)).with("Vrtc", px(a.get(1)?.as_f64()?))))
                })
                .collect();
            sel(D::Descriptor(desc("Plgn").with("Pts ", D::List(pts?))))
        }
        "select.editInQuickMaskMode" => {
            if jbool(p, "on") != Some(true) {
                return None;
            }
            desc("null").with("null", quick_mask_ref())
        }
        "tools.setColors" => {
            // One colour per Photoshop step: a step that sets both (or any other key) doesn't
            // export rather than lose one.
            if p.as_object().is_some_and(|o| o.keys().any(|k| k != "foreground" && k != "background")) {
                return None;
            }
            let (key, hex) = match (jstr(p, "foreground"), jstr(p, "background")) {
                (Some(f), None) => ("FrgC", f),
                (None, Some(b)) => ("BckC", b),
                _ => return None,
            };
            desc("null")
                .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: code(key) }]))
                .with("T   ", D::Descriptor(color_desc(hex)?))
        }
        "select.rect" => {
            match jstr(p, "mode") {
                None | Some("replace") => {}
                Some(_) => return None,
            }
            let (x, y, w, h) = (jnum(p, "x")?, jnum(p, "y")?, jnum(p, "width")?, jnum(p, "height")?);
            let cls = if jbool(p, "ellipse") == Some(true) { "Elps" } else { "Rctn" };
            sel(D::Descriptor(desc(cls).with("Top ", px(y)).with("Left", px(x)).with("Btom", px(y + h)).with("Rght", px(x + w))))
        }
        "layer.setProps" | "layer.renameLayer" | "layer.setLabelColor" => {
            return layer_set_to(p);
        }
        "layer.new.layerFromBackground" => {
            let bckg = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") }]);
            desc("null").with("null", bckg).with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml"))))
        }
        "layer.fill.pattern.set" => {
            if p.get("layer").is_some() || p.get("document").is_some() {
                return None;
            }
            desc("null").with("null", target_ref("contentLayer")).with("T   ", D::Descriptor(pattern_desc(p)?))
        }
        "gradient.fill.set" => {
            // A named layer or dragged handles have no place in the event.
            if ["layer", "from", "to"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
                return None;
            }
            desc("null").with("null", target_ref("contentLayer")).with("T   ", D::Descriptor(gradient_desc(p, true)?))
        }
        "layer.layerStyle.blendingOptions" => blending_options_to(p)?,
        "layer.smartFilter.blendingOptions" => {
            if p.get("layer").is_some() {
                return None;
            }
            let index = u32::try_from(p.get("index")?.as_u64()?).ok()?.checked_add(1)?;
            let mut opts = desc("blendOptions");
            if let Some(o) = jnum(p, "opacity") {
                opts = opts.with("Opct", pct(o * 100.0));
            }
            if let Some(m) = jstr(p, "blend") {
                opts = opts.with("Md  ", enm("BlnM", blend_ps(m)?));
            }
            let r = D::Reference(vec![
                ReferenceItem::Index { class: class("filterFX"), index },
                ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") },
            ]);
            desc("null").with("null", r).with("filterFX", D::Descriptor(desc("filterFX").with("blendOptions", D::Descriptor(opts))))
        }
        "layer.setAdjustment" => {
            // The event edits the current adjustment layer: a step that names its layer or
            // document would play back on whichever one is active instead (#8532). A null
            // `layer` or `document` is the target one, as the command reads it.
            if ["layer", "document"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
                return None;
            }
            // The adjustment kind isn't in the params: Photoshop needs it, so only steps that
            // name one (`kind`) export.
            let kind = jstr(p, "kind")?;
            // `layer.setAdjustment` changes only the settings it names. Color Balance and
            // Hue/Saturation descriptors carry every setting, filling absent ones with defaults,
            // so a partial edit would replay as one that also resets the rest (#8531). Levels writes
            // each range whole, so one end alone resets the other (#8530).
            // Present and of the shape the writer reads: anything else it would write as 0.
            let number = |p: &Value, k: &str| p.get(k).and_then(Value::as_f64).is_some_and(f64::is_finite);
            let numbers = |p: &Value, keys: &[&str]| keys.iter().all(|k| number(p, k));
            let flag = |k: &str| p.get(k).is_some_and(Value::is_boolean);
            let triplet =
                |k: &str| p.get(k).and_then(Value::as_array).is_some_and(|a| a.len() == 3 && a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)));
            let complete = match kind {
                "colorBalance" => ["shadows", "midtones", "highlights"].iter().all(|k| triplet(k)) && flag("preserveLuminosity"),
                // Every colour range written carries its own hue, saturation and lightness too.
                "hueSaturation" => {
                    flag("colorize")
                        && numbers(p, &["hue", "saturation", "lightness"])
                        && HUE_RANGES.iter().filter_map(|r| p.get(*r).filter(|v| !v.is_null())).all(|r| {
                            // Bounds, when given, are the four whole numbers `hue_entry` writes (as integers);
                            // otherwise it drops or rounds them.
                            let bounds = r.get("range").filter(|v| !v.is_null()).is_none_or(|v| {
                                v.as_array().is_some_and(|a| a.len() == 4 && a.iter().all(|b| b.as_f64().is_some_and(|b| b.is_finite() && b.fract() == 0.0)))
                            });
                            r.is_object() && numbers(r, &["hue", "saturation", "lightness"]) && bounds
                        })
                }
                // Both ends of each range it names (#8530).
                "levels" => levels_complete(p),
                _ => true,
            };
            if !complete {
                return None;
            }
            let a = adjustment_by_suffix(kind)?;
            desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor((a.to)(p)?))
        }
        _ => return None,
    })
}

/// Tool classes Photoshop records when an action switches tools: no document effect.
pub(super) fn is_tool_class(c: &str) -> bool {
    c.ends_with("Tl") || c.ends_with("Tool")
}
