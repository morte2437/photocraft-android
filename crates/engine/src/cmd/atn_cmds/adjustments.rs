use super::descriptors::{color_desc, color_hex, desc, enm, get_bool, get_enum, get_list, get_num, get_obj, jbool, jnum, jstr, long, num, pct, text};
use super::fields::{F, Kind, Mapped, fields_from, fields_to, step};
use super::tables::{CHANNELS, channel_key, channel_ref, lookup, lookup_back};
use super::{D, Descriptor, Map, Value, json};

// ------------------------------------------------------------------ adjustments

/// The `Adjs` entries of a per-channel adjustment (Levels, Curves): `(channel key, entry)`.
fn per_channel(d: &Descriptor) -> Option<Vec<(Option<&'static str>, &Descriptor)>> {
    get_list(d, "Adjs")?
        .iter()
        .map(|v| match v {
            D::Descriptor(e) => Some((channel_key(e)?, e)),
            _ => None,
        })
        .collect()
}

pub(super) fn levels_from(d: &Descriptor) -> Option<Value> {
    if get_bool(d, "Auto") == Some(true) {
        return None;
    }
    let mut out = Map::new();
    for (chan, e) in per_channel(d)? {
        let mut m = Map::new();
        if let Some(i) = get_list(e, "Inpt") {
            if let Some(v) = i.first().and_then(num) {
                m.insert("inBlack".into(), v.into());
            }
            if let Some(v) = i.get(1).and_then(num) {
                m.insert("inWhite".into(), v.into());
            }
        }
        if let Some(o) = get_list(e, "Otpt") {
            if let Some(v) = o.first().and_then(num) {
                m.insert("outBlack".into(), v.into());
            }
            if let Some(v) = o.get(1).and_then(num) {
                m.insert("outWhite".into(), v.into());
            }
        }
        if let Some(g) = get_num(e, "Gmm ") {
            m.insert("gamma".into(), g.into());
        }
        match chan {
            None => out.extend(m),
            Some(k) => {
                out.insert(k.into(), Value::Object(m));
            }
        }
    }
    Some(Value::Object(out))
}

fn levels_entry(chan: Option<&str>, p: &Value) -> Descriptor {
    let mut e = desc("LvlA").with("Chnl", channel_ref(chan));
    if jnum(p, "inBlack").is_some() || jnum(p, "inWhite").is_some() {
        e = e.with("Inpt", D::List(vec![long(jnum(p, "inBlack").unwrap_or(0.0)), long(jnum(p, "inWhite").unwrap_or(255.0))]));
    }
    if let Some(g) = jnum(p, "gamma") {
        e = e.with("Gmm ", D::Double(g));
    }
    if jnum(p, "outBlack").is_some() || jnum(p, "outWhite").is_some() {
        e = e.with("Otpt", D::List(vec![long(jnum(p, "outBlack").unwrap_or(0.0)), long(jnum(p, "outWhite").unwrap_or(255.0))]));
    }
    e
}

pub(super) fn levels_to(p: &Value) -> Option<Descriptor> {
    let mut entries = vec![D::Descriptor(levels_entry(None, p))];
    for (_, k) in CHANNELS {
        if let Some(c) = p.get(*k).filter(|v| v.is_object()) {
            entries.push(D::Descriptor(levels_entry(Some(k), c)));
        }
    }
    Some(desc("Lvls").with("presetKind", enm("presetKindType", "presetKindCustom")).with("Adjs", D::List(entries)))
}

fn curve_points(e: &Descriptor) -> Option<Value> {
    let pts: Option<Vec<Value>> = get_list(e, "Crv ")?
        .iter()
        .map(|v| match v {
            D::Descriptor(pt) => Some(json!([get_num(pt, "Hrzn")?, get_num(pt, "Vrtc")?])),
            _ => None,
        })
        .collect();
    pts.map(Value::Array)
}

fn curves_from(d: &Descriptor) -> Option<Value> {
    let mut out = Map::new();
    for (chan, e) in per_channel(d)? {
        let pts = curve_points(e)?;
        out.insert(chan.unwrap_or("points").into(), pts);
    }
    Some(Value::Object(out))
}

fn curve_entry(chan: Option<&str>, pts: &Value) -> Option<Descriptor> {
    let list: Option<Vec<D>> = pts
        .as_array()?
        .iter()
        .map(|pt| {
            let a = pt.as_array()?;
            Some(D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(a.first()?.as_f64()?)).with("Vrtc", D::Double(a.get(1)?.as_f64()?))))
        })
        .collect();
    Some(desc("CrvA").with("Chnl", channel_ref(chan)).with("Crv ", D::List(list?)))
}

fn curves_to(p: &Value) -> Option<Descriptor> {
    let mut entries = Vec::new();
    if let Some(pts) = p.get("points") {
        entries.push(D::Descriptor(curve_entry(None, pts)?));
    }
    for (_, k) in CHANNELS {
        if let Some(pts) = p.get(*k) {
            entries.push(D::Descriptor(curve_entry(Some(k), pts)?));
        }
    }
    Some(desc("Crvs").with("presetKind", enm("presetKindType", "presetKindCustom")).with("Adjs", D::List(entries)))
}

/// The hue key of a Hue/Saturation adjustment (`Hst2`): Photoshop's `H   ` (#8302).
const HUE_KEY: &str = "H   ";

pub(super) const HUE_RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];

fn hue_sat_from(d: &Descriptor) -> Option<Value> {
    let mut out = Map::new();
    if let Some(c) = get_bool(d, "Clrz") {
        out.insert("colorize".into(), c.into());
    }
    for e in get_list(d, "Adjs").unwrap_or(&[]) {
        let D::Descriptor(e) = e else { return None };
        let mut m = Map::new();
        // Photoshop's hue key is `H   ` (stringID "hue", #8302); PhotoCraft wrote `Hue ` before,
        // so its own older files still read.
        let hue = get_num(e, HUE_KEY).or_else(|| get_num(e, "Hue "));
        for (param, v) in [("hue", hue), ("saturation", get_num(e, "Strt")), ("lightness", get_num(e, "Lght"))] {
            if let Some(v) = v {
                m.insert(param.into(), v.into());
            }
        }
        match get_num(e, "LclR") {
            None => out.extend(m),
            Some(r) => {
                let name = *HUE_RANGES.get((r as usize).checked_sub(1)?)?;
                if let (Some(a), Some(b), Some(c), Some(dd)) = (get_num(e, "BgnR"), get_num(e, "BgnS"), get_num(e, "EndS"), get_num(e, "EndR")) {
                    m.insert("range".into(), json!([a, b, c, dd]));
                }
                out.insert(name.into(), Value::Object(m));
            }
        }
    }
    Some(Value::Object(out))
}

fn hue_entry(p: &Value, range: Option<(usize, &Value)>) -> Option<Descriptor> {
    let mut e = desc("Hst2");
    if let Some((i, r)) = range {
        e = e.with("LclR", long(i as f64 + 1.0));
        if let Some(a) = r.get("range").and_then(Value::as_array)
            && a.len() == 4
        {
            for (k, v) in ["BgnR", "BgnS", "EndS", "EndR"].iter().zip(a) {
                e = e.with(k, long(v.as_f64()?));
            }
        }
    }
    for (key, param) in [(HUE_KEY, "hue"), ("Strt", "saturation"), ("Lght", "lightness")] {
        e = e.with(key, long(jnum(p, param).unwrap_or(0.0)));
    }
    Some(e)
}

// Exporters leave out a param the step doesn't set only where Photoshop plays the missing key
// as PhotoCraft's default (checked against Photoshop: Fill opacity and mode, Crop delete,
// Selective Color method, Channel Mixer monochrome, Paint Bucket contiguous; Gradient Map
// reverse and dither, which Photoshop itself leaves out when off). Elsewhere they
// write PhotoCraft's default, as Photoshop plays a missing key differently (Color Balance
// preserve luminosity off, Canvas Size anchored top left, Apply Image Normal) or refuses the
// step (Paint Bucket without a tolerance).
fn hue_sat_to(p: &Value) -> Option<Descriptor> {
    let d = desc("HStr").with("presetKind", enm("presetKindType", "presetKindCustom")).with("Clrz", D::Boolean(jbool(p, "colorize").unwrap_or(false)));
    let mut entries = vec![D::Descriptor(hue_entry(p, None)?)];
    for (i, name) in HUE_RANGES.iter().enumerate() {
        if let Some(r) = p.get(*name).filter(|v| v.is_object()) {
            entries.push(D::Descriptor(hue_entry(r, Some((i, r)))?));
        }
    }
    Some(d.with("Adjs", D::List(entries)))
}

fn triple(d: &Descriptor, k: &str) -> Option<Value> {
    let l = get_list(d, k)?;
    let v: Option<Vec<f64>> = l.iter().map(num).collect();
    Some(json!(v?))
}

fn triple_to(p: &Value, k: &str) -> Option<D> {
    Some(D::List(p.get(k)?.as_array()?.iter().map(|v| v.as_f64().map(long)).collect::<Option<Vec<_>>>()?))
}

fn color_balance_from(d: &Descriptor) -> Option<Value> {
    let mut out = Map::new();
    for (key, param) in [("ShdL", "shadows"), ("MdtL", "midtones"), ("HghL", "highlights")] {
        if let Some(v) = triple(d, key) {
            out.insert(param.into(), v);
        }
    }
    if let Some(b) = get_bool(d, "PrsL") {
        out.insert("preserveLuminosity".into(), b.into());
    }
    Some(Value::Object(out))
}

fn color_balance_to(p: &Value) -> Option<Descriptor> {
    let mut d = desc("ClrB");
    for (key, param) in [("ShdL", "shadows"), ("MdtL", "midtones"), ("HghL", "highlights")] {
        d = d.with(key, triple_to(p, param).unwrap_or_else(|| D::List(vec![long(0.0), long(0.0), long(0.0)])));
    }
    Some(d.with("PrsL", D::Boolean(jbool(p, "preserveLuminosity").unwrap_or(true))))
}

const BW: &[F] = &[
    F("Rd  ", "reds", Kind::Long),
    F("Yllw", "yellows", Kind::Long),
    F("Grn ", "greens", Kind::Long),
    F("Cyn ", "cyans", Kind::Long),
    F("Bl  ", "blues", Kind::Long),
    F("Mgnt", "magentas", Kind::Long),
    F("useTint", "tint", Kind::Bool),
    F("tintColor", "tintColor", Kind::Color),
];

fn mixer_channel(e: &Descriptor) -> Option<Value> {
    Some(json!([get_num(e, "Rd  ").unwrap_or(0.0), get_num(e, "Grn ").unwrap_or(0.0), get_num(e, "Bl  ").unwrap_or(0.0), get_num(e, "Cnst").unwrap_or(0.0)]))
}

fn channel_mixer_from(d: &Descriptor) -> Option<Value> {
    let mut out = Map::new();
    for (key, param) in [("Rd  ", "red"), ("Grn ", "green"), ("Bl  ", "blue"), ("Gry ", "gray")] {
        if let Some(e) = get_obj(d, key) {
            out.insert(param.into(), mixer_channel(e)?);
        }
    }
    if let Some(m) = get_bool(d, "Mnch") {
        out.insert("monochrome".into(), m.into());
    }
    Some(Value::Object(out))
}

fn channel_mixer_to(p: &Value) -> Option<Descriptor> {
    let mut d = desc("ChnM").with("presetKind", enm("presetKindType", "presetKindCustom"));
    for (key, param) in [("Rd  ", "red"), ("Grn ", "green"), ("Bl  ", "blue"), ("Gry ", "gray")] {
        if let Some(a) = p.get(param).and_then(Value::as_array) {
            let v = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            d = d.with(key, D::Descriptor(desc("ChMx").with("Rd  ", pct(v(0))).with("Grn ", pct(v(1))).with("Bl  ", pct(v(2))).with("Cnst", pct(v(3)))));
        }
    }
    if let Some(m) = jbool(p, "monochrome") {
        d = d.with("Mnch", D::Boolean(m));
    }
    Some(d)
}

/// Photoshop's `Clrs` colour-range enumeration. Yellows is `Ylws` (`Yllw` is the yellow ink
/// key); earlier PhotoCraft builds wrote `Yllw`, which import still reads (#6745).
const SELECTIVE_RANGES: &[(&str, &str)] = &[
    ("Rds", "reds"),
    ("Ylws", "yellows"),
    ("Grns", "greens"),
    ("Cyns", "cyans"),
    ("Bls", "blues"),
    ("Mgnt", "magentas"),
    ("Whts", "whites"),
    ("Ntrl", "neutrals"),
    ("Blks", "blacks"),
];

pub(super) fn selective_from(d: &Descriptor) -> Option<Value> {
    let mut out = Map::new();
    if let Some(m) = get_enum(d, "Mthd") {
        out.insert("method".into(), (if m == "Absl" { "absolute" } else { "relative" }).into());
    }
    for e in get_list(d, "ClrC").unwrap_or(&[]) {
        let D::Descriptor(e) = e else { return None };
        let range = get_enum(e, "Clrs")?;
        let name = lookup(SELECTIVE_RANGES, if range == "Yllw" { "Ylws" } else { &range })?;
        let v = |k: &str| get_num(e, k).unwrap_or(0.0);
        out.insert(name.into(), json!([v("Cyn "), v("Mgnt"), v("Yllw"), v("Blck")]));
    }
    Some(Value::Object(out))
}

fn selective_to(p: &Value) -> Option<Descriptor> {
    // Match command execution: scalar components override only the selected range, `relative`
    // takes precedence over `method`, and invalid values must not become a different edit.
    let photocraft_doc::Adjustment::SelectiveColor { relative, adjustments } = crate::adjust_cmds::selective_from_params(p, None).ok()? else {
        return None;
    };
    let mut list = Vec::new();
    for (name, row) in crate::adjust_cmds::RANGES.into_iter().zip(adjustments) {
        if p.get(name).is_some_and(|v| !v.is_null()) || jstr(p, "colors") == Some(name) {
            let ps = lookup_back(SELECTIVE_RANGES, name)?;
            let [cyan, magenta, yellow, black] = row.map(f64::from);
            list.push(D::Descriptor(
                desc("ClrC")
                    .with("Clrs", enm("Clrs", ps))
                    .with("Cyn ", pct(cyan))
                    .with("Mgnt", pct(magenta))
                    .with("Yllw", pct(yellow))
                    .with("Blck", pct(black)),
            ));
        }
    }
    Some(
        desc("SlcC")
            .with("presetKind", enm("presetKindType", "presetKindCustom"))
            .with("Mthd", enm("CrcM", if relative { "Rltv" } else { "Absl" }))
            .with("ClrC", D::List(list)),
    )
}

/// Gradient Map: the gradient's colour stops at 0..4096 → `stops: [[0..1, "#rrggbb"], …]`.
fn gradient_map_from(d: &Descriptor) -> Option<Value> {
    let grad = get_obj(d, "Grad")?;
    let stops: Option<Vec<Value>> = get_list(grad, "Clrs")?
        .iter()
        .map(|v| match v {
            D::Descriptor(stop) => Some(json!([(get_num(stop, "Lctn")? / 4096.0).clamp(0.0, 1.0), color_hex(get_obj(stop, "Clr ")?)?])),
            _ => None,
        })
        .collect();
    let mut out = json!({"stops": stops?});
    if let Some(r) = get_bool(d, "Rvrs") {
        out["reverse"] = json!(r);
    }
    if let Some(dd) = get_bool(d, "Dthr") {
        out["dither"] = json!(dd);
    }
    Some(out)
}

fn gradient_map_to(p: &Value) -> Option<Descriptor> {
    let stops: Option<Vec<D>> = p
        .get("stops")?
        .as_array()?
        .iter()
        .map(|st| {
            let a = st.as_array()?;
            Some(D::Descriptor(
                desc("Clrt")
                    .with("Clr ", D::Descriptor(color_desc(a.get(1)?.as_str()?)?))
                    .with("Type", enm("Clry", "UsrS"))
                    .with("Lctn", long((a.first()?.as_f64()? * 4096.0).round()))
                    .with("Mdpn", long(50.0)),
            ))
        })
        .collect();
    let transparency = D::List(vec![
        D::Descriptor(desc("TrnS").with("Opct", pct(100.0)).with("Lctn", long(0.0)).with("Mdpn", long(50.0))),
        D::Descriptor(desc("TrnS").with("Opct", pct(100.0)).with("Lctn", long(4096.0)).with("Mdpn", long(50.0))),
    ]);
    let grad = desc("Grdn")
        .with("Nm  ", text("Custom"))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(4096.0))
        .with("Clrs", D::List(stops?))
        .with("Trns", transparency);
    let mut d = desc("GdMp").with("Grad", D::Descriptor(grad));
    if let Some(r) = jbool(p, "reverse") {
        d = d.with("Rvrs", D::Boolean(r));
    }
    if let Some(dd) = jbool(p, "dither") {
        d = d.with("Dthr", D::Boolean(dd));
    }
    Some(d)
}

/// An adjustment: its Photoshop class (usually also the direct event's code; Black & White and
/// Photo Filter run by name), the command suffix (`image.adjustments.<suffix>`, `layer.newAdjustmentLayer.<suffix>`), and
/// the converters between the adjustment descriptor and the command params.
pub(super) struct Adjustment {
    pub(super) class: &'static str,
    pub(super) suffix: &'static str,
    pub(super) from: fn(&Descriptor) -> Option<Value>,
    pub(super) to: fn(&Value) -> Option<Descriptor>,
}

macro_rules! adj_fields {
    ($class:literal, $suffix:literal, $fields:expr) => {
        Adjustment { class: $class, suffix: $suffix, from: |d| fields_from(d, $fields).map(Value::Object), to: |p| fields_to(p, $fields, desc($class)) }
    };
}

const ADJUSTMENTS: &[Adjustment] = &[
    Adjustment { class: "Lvls", suffix: "levels", from: levels_from, to: levels_to },
    Adjustment { class: "Crvs", suffix: "curves", from: curves_from, to: curves_to },
    Adjustment { class: "HStr", suffix: "hueSaturation", from: hue_sat_from, to: hue_sat_to },
    adj_fields!("BrgC", "brightnessContrast", &[F("Brgh", "brightness", Kind::Long), F("Cntr", "contrast", Kind::Long), F("useLegacy", "legacy", Kind::Bool)]),
    adj_fields!("Thrs", "threshold", &[F("Lvl ", "level", Kind::Long)]),
    adj_fields!("Pstr", "posterize", &[F("Lvls", "levels", Kind::Long)]),
    adj_fields!("Invr", "invert", &[]),
    Adjustment { class: "ClrB", suffix: "colorBalance", from: color_balance_from, to: color_balance_to },
    adj_fields!("Vbrn", "vibrance", &[F("vibrance", "vibrance", Kind::Long), F("Strt", "saturation", Kind::Long)]),
    adj_fields!("Exps", "exposure", &[F("Exps", "exposure", Kind::Dbl), F("Ofst", "offset", Kind::Dbl), F("gammaCorrection", "gamma", Kind::Dbl)]),
    Adjustment { class: "PhtF", suffix: "photoFilter", from: |d| fields_from(d, PHOTO_FILTER_FIELDS).map(Value::Object), to: photo_filter_to },
    adj_fields!("BanW", "blackWhite", BW),
    Adjustment { class: "ChnM", suffix: "channelMixer", from: channel_mixer_from, to: channel_mixer_to },
    Adjustment { class: "SlcC", suffix: "selectiveColor", from: selective_from, to: selective_to },
    Adjustment { class: "GdMp", suffix: "gradientMap", from: gradient_map_from, to: gradient_map_to },
];

/// Adjustment classes PhotoCraft once exported, with the class Photoshop uses. Measured in
/// Photoshop (#8481): a Make whose Black & White type is `BlnW` fails ("the command Make is not
/// currently available"), `BanW` runs. Import keeps accepting them so actions saved before still
/// load; export never writes them.
const LEGACY_CLASSES: &[(&str, &str)] = &[("BlnW", "BanW")];

/// The adjustment with Photoshop class `c`, or the one an old PhotoCraft class stands for.
pub(super) fn adjustment_by_class(c: &str) -> Option<&'static Adjustment> {
    let c = c.trim_end();
    let c = LEGACY_CLASSES.iter().find(|(old, _)| *old == c).map_or(c, |(_, class)| class);
    ADJUSTMENTS.iter().find(|a| a.class.trim_end() == c)
}

const PHOTO_FILTER_FIELDS: &[F] = &[F("Clr ", "color", Kind::Color), F("Dnst", "density", Kind::Long), F("PrsL", "preserveLuminosity", Kind::Bool)];

/// A `PhtF` step whose params carry a `filter` preset without an explicit `color`: the descriptor
/// form is colour-only (Photoshop stores no preset key — the `phfl` notes in `crates/io` conclude
/// the same from its CS2 oracles), so the resolved colour must be written, or re-imported playback
/// falls back to the default warming filter. With both present, the explicit colour wins, as when
/// running the command. An unknown preset id keeps the old export (no colour) instead of failing.
fn photo_filter_to(p: &Value) -> Option<Descriptor> {
    let mut p = p.clone();
    if p.get("color").is_none() {
        let hex = p.get("filter").and_then(Value::as_str).and_then(preset_color);
        if let (Some(hex), Some(o)) = (hex, p.as_object_mut()) {
            o.insert("color".into(), hex.into());
        }
    }
    fields_to(&p, PHOTO_FILTER_FIELDS, desc("PhtF"))
}

/// The preset id's resolved `#rrggbb`, or none (no id given, or an unknown one).
fn preset_color(id: &str) -> Option<String> {
    crate::adjust_params::PHOTO_FILTERS.iter().find(|f| f.0 == id).map(|(_, _, rgb)| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

pub(super) fn adjustment_by_suffix(suffix: &str) -> Option<&'static Adjustment> {
    ADJUSTMENTS.iter().find(|a| a.suffix == suffix)
}

// Events whose target depends on the descriptor (`make`, `set`, `select`…): custom converters.

pub(super) fn adjustment_steps(a: &Adjustment, prefix: &str, type_desc: Option<&Descriptor>) -> Mapped {
    let params = match type_desc {
        Some(t) if get_enum(t, "presetKind").as_deref() != Some("presetKindDefault") => match (a.from)(t) {
            Some(p) => p,
            None => return Mapped::Unsupported { harmless: false },
        },
        _ => json!({}),
    };
    step(&format!("{prefix}{}", a.suffix), params)
}

/// Shadows/Highlights (`adaptCorrect`, #5830): the shadow and highlight tones are `shadowMode`
/// and `highlightMode` objects (amount and width in %, radius in px), then the black and white
/// clips (%), midtone contrast (`center`) and colour correction. Each key is read as its char ID
/// or its string ID, whichever the file used; absent keys keep the command's defaults.
const SHADOWS_HIGHLIGHTS_TONES: [(&str, &str, &str); 2] = [("sdwM", "shadowMode", "shadow"), ("hglM", "highlightMode", "highlight")];

const SHADOWS_HIGHLIGHTS_TONE: [(&str, &str, &str); 3] = [("Amnt", "amount", "Amount"), ("Wdth", "width", "Tone"), ("Rds ", "radius", "Radius")];

const SHADOWS_HIGHLIGHTS: [(&str, &str, &str); 4] =
    [("BlcC", "blackClip", "blackClip"), ("WhtC", "whiteClip", "whiteClip"), ("Cntr", "center", "midtone"), ("ClrC", "colorCorrection", "color")];

pub(super) fn shadows_highlights_from(d: &Descriptor) -> Mapped {
    let mut m = Map::new();
    for (code, name, prefix) in SHADOWS_HIGHLIGHTS_TONES {
        let Some(t) = get_obj(d, code).or_else(|| get_obj(d, name)) else { continue };
        for (code, name, suffix) in SHADOWS_HIGHLIGHTS_TONE {
            if let Some(v) = get_num(t, code).or_else(|| get_num(t, name)) {
                m.insert(format!("{prefix}{suffix}"), Value::from(v));
            }
        }
    }
    for (code, name, param) in SHADOWS_HIGHLIGHTS {
        if let Some(v) = get_num(d, code).or_else(|| get_num(d, name)) {
            m.insert(param.into(), Value::from(v));
        }
    }
    step("image.adjustments.shadowsHighlights", Value::Object(m))
}

pub(super) fn shadows_highlights_to(_: &str, p: &Value) -> Option<Descriptor> {
    let mut d = desc("null");
    for (_, name, prefix) in SHADOWS_HIGHLIGHTS_TONES {
        let mut t = desc("adaptCorrectTones");
        for (_, key, suffix) in SHADOWS_HIGHLIGHTS_TONE {
            if let Some(v) = jnum(p, &format!("{prefix}{suffix}")) {
                t = t.with(key, if suffix == "Radius" { long(v) } else { pct(v) });
            }
        }
        d = d.with(name, D::Descriptor(t));
    }
    for (_, key, param) in SHADOWS_HIGHLIGHTS {
        if let Some(v) = jnum(p, param) {
            d = d.with(key, if matches!(param, "midtone" | "color") { long(v) } else { D::Double(v) });
        }
    }
    Some(d)
}
