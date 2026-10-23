//! Photoshop Actions files (`.atn`): Load Actions (`actions.load`) and Save Actions
//! (`actions.save`), plus the two step commands a Photoshop set needs: `actions.stopMessage`
//! (Photoshop's Insert Stop) and `actions.photoshopStep` (a step PhotoCraft has no command for
//! yet, kept verbatim so Save Actions writes it back unchanged).
//!
//! Parsing and writing live in `photocraft-psd` (`atn`); this module maps Photoshop *events*
//! (`gaussianBlur` with `Rds`, `make` of an adjustment layer, `set` of the selection…) onto
//! PhotoCraft commands and back. An event is keyed by both its four-character code (`GsnB`,
//! older files) and its string name (`gaussianBlur`, Photoshop 2024 and later), as the two are
//! interchangeable in Photoshop. The table ([`EVENTS`]) is bidirectional: a step that was
//! imported exports to the same event. Events outside it become `actions.photoshopStep` steps,
//! which fail at playback with the step's Photoshop name, so an action never silently skips
//! work; tool selections and other steps with no document effect are imported switched off.
//!
//! Layer references by name (`select` of layer "Shadow") become `{"layer": "Shadow"}`, which
//! `commands::layer_param` resolves in whatever document the action plays on (#219). A switch
//! to another open document (`select` of `Dcmn` by offset) becomes `document.activate
//! {"offset"}`, the tab that many places from the active one (#7103).

use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::{Map, Value, json};

use crate::actions_cmds::Action;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// A step PhotoCraft cannot play yet, kept verbatim (`{"event", "name", "descriptor": base64}`).
pub const PHOTOSHOP_STEP: &str = "actions.photoshopStep";
/// Photoshop's Insert Stop: `{"message", "allowContinue"}`.
pub const STOP_MESSAGE: &str = "actions.stopMessage";
/// The set name Save Actions uses for actions that came from no set.
pub const DEFAULT_SET: &str = "PhotoCraft Actions";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

// ------------------------------------------------------------------ descriptor access

fn num(v: &D) -> Option<f64> {
    match v {
        D::Integer(i) => Some(f64::from(*i)),
        D::LargeInteger(i) => Some(*i as f64),
        D::Double(d) | D::UnitFloat { value: d, .. } => Some(*d),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

fn trimmed(id: &Id) -> String {
    String::from_utf8_lossy(id.as_bytes()).trim_end().to_string()
}

fn get_num(d: &Descriptor, k: &str) -> Option<f64> {
    d.get(k).and_then(num)
}

fn get_bool(d: &Descriptor, k: &str) -> Option<bool> {
    match d.get(k) {
        Some(D::Boolean(b)) => Some(*b),
        _ => None,
    }
}

fn get_text(d: &Descriptor, k: &str) -> Option<String> {
    match d.get(k) {
        Some(D::Text(t)) => Some(t.to_string_lossy()),
        _ => None,
    }
}

/// An enumerated value's id, trailing spaces removed (`Ordn.Trgt` → `"Trgt"`).
fn get_enum(d: &Descriptor, k: &str) -> Option<String> {
    match d.get(k) {
        Some(D::Enumerated { value, .. }) => Some(trimmed(value)),
        _ => None,
    }
}

fn get_obj<'a>(d: &'a Descriptor, k: &str) -> Option<&'a Descriptor> {
    match d.get(k) {
        Some(D::Descriptor(o) | D::GlobalObject(o)) => Some(o),
        _ => None,
    }
}

fn get_list<'a>(d: &'a Descriptor, k: &str) -> Option<&'a [D]> {
    match d.get(k) {
        Some(D::List(l)) => Some(l),
        _ => None,
    }
}

fn get_ref<'a>(d: &'a Descriptor, k: &str) -> Option<&'a [ReferenceItem]> {
    match d.get(k) {
        Some(D::Reference(r)) => Some(r),
        _ => None,
    }
}

/// The class of a reference's first item, trimmed (`Lyr`, `Chnl`, `AdjL`…).
fn ref_class(r: &[ReferenceItem]) -> Option<String> {
    Some(trimmed(match r.first()? {
        ReferenceItem::Property { class, .. }
        | ReferenceItem::Class(class)
        | ReferenceItem::Enumerated { class, .. }
        | ReferenceItem::Offset { class, .. }
        | ReferenceItem::Identifier { class, .. }
        | ReferenceItem::Index { class, .. }
        | ReferenceItem::Name { class, .. } => &class.class_id,
    }))
}

/// How a reference names its target: the ordinal (`Trgt`, `Frnt`…), a name, a property…
enum Target {
    Ordinal(String),
    Name(String),
    Property(String),
    Class,
    Offset(i32),
    Index(u32),
    Other,
}

fn target(r: &[ReferenceItem]) -> Target {
    match r.first() {
        Some(ReferenceItem::Enumerated { value, .. }) => Target::Ordinal(trimmed(value)),
        Some(ReferenceItem::Name { name, .. }) => Target::Name(name.to_string_lossy()),
        Some(ReferenceItem::Property { key, .. }) => Target::Property(trimmed(key)),
        Some(ReferenceItem::Class(_)) => Target::Class,
        Some(ReferenceItem::Offset { offset, .. }) => Target::Offset(*offset),
        Some(ReferenceItem::Index { index, .. }) => Target::Index(*index),
        _ => Target::Other,
    }
}

/// `{"layer": name}` for a reference that names a layer, `{}` for the target layer, `None`
/// for anything else (an index or id: meaningless in another document).
fn layer_target(r: &[ReferenceItem]) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    match target(r) {
        Target::Ordinal(o) if o == "Trgt" => {}
        Target::Name(n) => {
            m.insert("layer".into(), json!(n));
        }
        Target::Property(p) if p == "Bckg" => {
            m.insert("layer".into(), json!("Background"));
        }
        _ => return None,
    }
    Some(m)
}

// ------------------------------------------------------------------ descriptor building

/// A Photoshop id: codes shorter than four characters are space-padded (`H   `, `Rd  `),
/// longer names stay strings.
fn code(s: &str) -> Id {
    if s.len() < 4 { Id::new(&format!("{s:<4}")) } else { Id::new(s) }
}

fn class(c: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: code(c) }
}

/// The pixel and percent units of a `UntF` value.
const PX: [u8; 4] = *b"#Pxl";
const PCT: [u8; 4] = *b"#Prc";
/// Points (1/72 in): what Photoshop calls "relative" units.
const PT: [u8; 4] = *b"#Rlt";

fn px(v: f64) -> D {
    D::UnitFloat { unit: PX, value: v }
}

fn pct(v: f64) -> D {
    D::UnitFloat { unit: *b"#Prc", value: v }
}

fn ang(v: f64) -> D {
    D::UnitFloat { unit: *b"#Ang", value: v }
}

fn long(v: f64) -> D {
    D::Integer(v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

fn text(s: &str) -> D {
    D::Text(UnicodeString::new_nul(s))
}

fn enm(ty: &str, v: &str) -> D {
    D::Enumerated { type_id: code(ty), value: code(v) }
}

fn target_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new("Ordn"), value: Id::new("Trgt") }])
}

fn name_ref(cls: &str, name: &str) -> D {
    D::Reference(vec![ReferenceItem::Name { class: class(cls), name: UnicodeString::new_nul(name) }])
}

fn class_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Class(class(cls))])
}

/// The `null` reference for a command's `layer` param: by name, else the target layer.
fn layer_ref(p: &Value) -> Option<D> {
    match p.get("layer") {
        None | Some(Value::Null) => Some(target_ref("Lyr ")),
        Some(Value::String(n)) => Some(name_ref("Lyr ", n)),
        _ => None,
    }
}

fn desc(cls: &str) -> Descriptor {
    Descriptor::new(cls)
}

// ------------------------------------------------------------------ JSON access

fn jnum(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64).filter(|f| f.is_finite())
}

fn jbool(p: &Value, k: &str) -> Option<bool> {
    p.get(k).and_then(Value::as_bool)
}

fn jstr<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

// ------------------------------------------------------------------ colours and enums

/// A Photoshop colour object as `#rrggbb`: `RGBC` (0..255), `HSBC`, `Grsc`.
fn color_hex(c: &Descriptor) -> Option<String> {
    let hex = |r: f64, g: f64, b: f64| {
        format!("#{:02x}{:02x}{:02x}", r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8)
    };
    if let (Some(r), Some(g), Some(b)) = (get_num(c, "Rd  "), get_num(c, "Grn "), get_num(c, "Bl  ")) {
        return Some(hex(r, g, b));
    }
    if let (Some(h), Some(s), Some(v)) = (get_num(c, "H   "), get_num(c, "Strt"), get_num(c, "Brgh")) {
        let (s, v) = ((s / 100.0).clamp(0.0, 1.0), (v / 100.0).clamp(0.0, 1.0));
        let h = h.rem_euclid(360.0) / 60.0;
        let i = h.floor();
        let f = h - i;
        let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
        let (r, g, b) = match i as i32 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        return Some(hex(r * 255.0, g * 255.0, b * 255.0));
    }
    get_num(c, "Gry ").map(|g| {
        let v = 255.0 - g * 2.55;
        hex(v, v, v)
    })
}

fn color_desc(hex: &str) -> Option<Descriptor> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(f64::from);
    Some(desc("RGBC").with("Rd  ", D::Double(ch(0)?)).with("Grn ", D::Double(ch(2)?)).with("Bl  ", D::Double(ch(4)?)))
}

/// A `Pth ` value's text: Photoshop stores `"txtu"`, a little-endian total length, a
/// little-endian UTF-16 unit count and the units (NUL-terminated).
fn path_text(v: &D) -> Option<String> {
    let D::Path(b) = v else { return None };
    if b.get(0..4)? != b"txtu" {
        return None;
    }
    let n = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?) as usize;
    let units: Vec<u16> = b.get(12..12usize.checked_add(n.checked_mul(2)?)?)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let text = String::from_utf16_lossy(&units);
    let text = text.trim_end_matches('\0');
    (!text.is_empty()).then(|| text.to_string())
}

fn path_value(text: &str) -> D {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut b = b"txtu".to_vec();
    b.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
    b.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        b.extend_from_slice(&u.to_le_bytes());
    }
    D::Path(b)
}

/// Photoshop's `BlnM` enumeration ↔ PhotoCraft's blend mode labels.
const BLEND_MODES: &[(&str, &str)] = &[
    ("Nrml", "Normal"),
    ("Dslv", "Dissolve"),
    ("Drkn", "Darken"),
    ("Mltp", "Multiply"),
    ("CBrn", "Color Burn"),
    ("linearBurn", "Linear Burn"),
    ("darkerColor", "Darker Color"),
    ("Lghn", "Lighten"),
    ("Scrn", "Screen"),
    ("CDdg", "Color Dodge"),
    ("linearDodge", "Linear Dodge (Add)"),
    ("lighterColor", "Lighter Color"),
    ("Ovrl", "Overlay"),
    ("SftL", "Soft Light"),
    ("HrdL", "Hard Light"),
    ("vividLight", "Vivid Light"),
    ("linearLight", "Linear Light"),
    ("pinLight", "Pin Light"),
    ("hardMix", "Hard Mix"),
    ("Dfrn", "Difference"),
    ("Xclu", "Exclusion"),
    ("blendSubtraction", "Subtract"),
    ("blendDivide", "Divide"),
    ("H", "Hue"),
    ("Strt", "Saturation"),
    ("Clr", "Color"),
    ("Lmns", "Luminosity"),
    ("PsTh", "Pass Through"),
];

fn blend_label(ps: &str) -> Option<&'static str> {
    BLEND_MODES.iter().find(|(k, _)| *k == ps).map(|(_, v)| *v)
}

fn blend_ps(label: &str) -> Option<&'static str> {
    crate::commands::blend_from_str(label).and_then(|m| BLEND_MODES.iter().find(|(_, v)| crate::commands::blend_from_str(v) == Some(m))).map(|(k, _)| *k)
}

/// Photoshop ↔ PhotoCraft value tables; Photoshop codes compare without their space padding.
fn lookup<'a>(table: &'a [(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| k.trim_end() == key.trim_end()).map(|(_, v)| *v)
}

fn lookup_back<'a>(table: &'a [(&'a str, &'a str)], value: &str) -> Option<&'a str> {
    table.iter().find(|(_, v)| *v == value).map(|(k, _)| *k)
}

/// `Chnl` enumeration ↔ the per-channel keys of the adjustment commands.
const CHANNELS: &[(&str, &str)] = &[
    ("Rd", "red"),
    ("Grn", "green"),
    ("Bl", "blue"),
    ("Gry", "gray"),
    ("Cyn", "cyan"),
    ("Mgnt", "magenta"),
    ("Yllw", "yellow"),
    ("Blck", "black"),
    ("Lmnc", "lightness"),
    ("A", "a"),
    ("B", "b"),
];

/// The channel an adjustment entry's `Chnl` reference names: `None` = the composite.
fn channel_key(d: &Descriptor) -> Option<Option<&'static str>> {
    let r = get_ref(d, "Chnl")?;
    Some(match target(r) {
        Target::Ordinal(o) if o == "Trgt" || o == "Cmps" => None,
        Target::Ordinal(o) => Some(lookup(CHANNELS, &o)?),
        _ => return None,
    })
}

fn channel_ref(key: Option<&str>) -> D {
    let value = key.and_then(|k| lookup_back(CHANNELS, k));
    D::Reference(vec![ReferenceItem::Enumerated {
        class: class("Chnl"),
        type_id: code(if value.is_some() { "Chnl" } else { "Ordn" }),
        value: code(value.unwrap_or("Trgt")),
    }])
}

// ------------------------------------------------------------------ field tables

#[derive(Clone, Copy)]
enum Kind {
    Px,
    Pct,
    Ang,
    Long,
    /// An integer field whose value must survive conversion without rounding or saturation.
    ExactLong,
    Dbl,
    Bool,
    /// An enumeration: its type id and the Photoshop ↔ PhotoCraft value pairs.
    Enum(&'static str, &'static [(&'static str, &'static str)]),
    /// A blend mode (`BlnM`).
    Blend,
    /// A colour object ↔ `#rrggbb`.
    Color,
}

/// One descriptor key ↔ one command param.
struct F(&'static str, &'static str, Kind);

/// Import mapped descriptor fields, leaving absent values to command defaults.
/// Return `None` when a present enum, blend mode or colour cannot be represented.
fn fields_from(d: &Descriptor, fields: &[F]) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    for F(key, param, kind) in fields {
        let v = match kind {
            Kind::Px | Kind::Pct | Kind::Ang | Kind::Long | Kind::ExactLong | Kind::Dbl => get_num(d, key).map(Value::from),
            Kind::Bool => get_bool(d, key).map(Value::from),
            Kind::Enum(_, table) => match get_enum(d, key) {
                Some(e) => Some(Value::from(lookup(table, &e)?)),
                None => None,
            },
            Kind::Blend => match get_enum(d, key) {
                Some(e) => Some(Value::from(blend_label(&e)?)),
                None => None,
            },
            Kind::Color => match get_obj(d, key) {
                Some(c) => Some(Value::from(color_hex(c)?)),
                None => None,
            },
        };
        if let Some(v) = v {
            m.insert((*param).into(), v);
        }
    }
    Some(m)
}

/// Export supplied mapped params, omitting absent values to preserve event defaults.
/// Return `None` for unsupported values; `ExactLong` also rejects rounding or saturation.
fn fields_to(p: &Value, fields: &[F], mut d: Descriptor) -> Option<Descriptor> {
    for F(key, param, kind) in fields {
        let Some(v) = p.get(param) else { continue };
        let out = match kind {
            Kind::Px => px(v.as_f64()?),
            Kind::Pct => pct(v.as_f64()?),
            Kind::Ang => ang(v.as_f64()?),
            // A value outside the descriptor's 32-bit integer refuses the step rather than being
            // clamped to another value (an Add Noise seed above i32::MAX replayed as i32::MAX, #7287).
            Kind::Long => {
                let n = v.as_f64()?.round();
                if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n) {
                    return None;
                }
                long(n)
            }
            Kind::ExactLong => {
                let number = v.as_f64()?;
                let integer = number as i32;
                if f64::from(integer) != number {
                    return None;
                }
                D::Integer(integer)
            }
            Kind::Dbl => D::Double(v.as_f64()?),
            Kind::Bool => D::Boolean(v.as_bool()?),
            Kind::Enum(ty, table) => enm(ty, lookup_back(table, v.as_str()?)?),
            Kind::Blend => enm("BlnM", blend_ps(v.as_str()?)?),
            Kind::Color => D::Descriptor(color_desc(v.as_str()?)?),
        };
        d = d.with(key, out);
    }
    Some(d)
}

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

fn levels_from(d: &Descriptor) -> Option<Value> {
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

fn levels_to(p: &Value) -> Option<Descriptor> {
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

const HUE_RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];

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

fn selective_from(d: &Descriptor) -> Option<Value> {
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
    // Each range, as the command reads it (`adjust_cmds::selective_from_params`): the array form
    // `"reds": [c, m, y, k]` first, then the scalar form, which names one range in `"colors"` and
    // gives the channels it changes. Writing only the arrays dropped the scalar form's values, and
    // the step exported as an adjustment that changes nothing (#7672).
    let mut rows: Vec<Option<[f64; 4]>> = vec![None; SELECTIVE_RANGES.len()];
    for (row, (_, name)) in rows.iter_mut().zip(SELECTIVE_RANGES) {
        if let Some(a) = p.get(*name).and_then(Value::as_array) {
            let v = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            *row = Some([v(0), v(1), v(2), v(3)]);
        }
    }
    if let Some(named) = jstr(p, "colors") {
        let i = SELECTIVE_RANGES.iter().position(|(_, name)| *name == named)?;
        let row = rows.get_mut(i)?;
        let mut v = row.unwrap_or([0.0; 4]);
        for (slot, key) in v.iter_mut().zip(["cyan", "magenta", "yellow", "black"]) {
            // A channel that is given but is not a number is an error in the command
            // (`adjust_cmds::selective_from_params`), so a step carrying one cannot be written
            // either: ignoring it would export a different adjustment than the one recorded.
            // `null` counts as absent there, and so here.
            match p.get(key) {
                None | Some(Value::Null) => {}
                Some(_) => *slot = jnum(p, key)?,
            }
        }
        *row = Some(v);
    }
    let mut list = Vec::new();
    for (row, (ps, _)) in rows.iter().zip(SELECTIVE_RANGES) {
        if let Some(v) = row {
            list.push(D::Descriptor(
                desc("ClrC").with("Clrs", enm("Clrs", ps)).with("Cyn ", pct(v[0])).with("Mgnt", pct(v[1])).with("Yllw", pct(v[2])).with("Blck", pct(v[3])),
            ));
        }
    }
    let mut d = desc("SlcC").with("presetKind", enm("presetKindType", "presetKindCustom"));
    if let Some(m) = jstr(p, "method") {
        d = d.with("Mthd", enm("CrcM", if m == "absolute" { "Absl" } else { "Rltv" }));
    }
    Some(d.with("ClrC", D::List(list)))
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
struct Adjustment {
    class: &'static str,
    suffix: &'static str,
    from: fn(&Descriptor) -> Option<Value>,
    to: fn(&Value) -> Option<Descriptor>,
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
    adj_fields!("BlnW", "blackWhite", BW),
    Adjustment { class: "ChnM", suffix: "channelMixer", from: channel_mixer_from, to: channel_mixer_to },
    Adjustment { class: "SlcC", suffix: "selectiveColor", from: selective_from, to: selective_to },
    Adjustment { class: "GdMp", suffix: "gradientMap", from: gradient_map_from, to: gradient_map_to },
];

fn adjustment_by_class(c: &str) -> Option<&'static Adjustment> {
    ADJUSTMENTS.iter().find(|a| a.class.trim_end() == c.trim_end())
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

fn adjustment_by_suffix(suffix: &str) -> Option<&'static Adjustment> {
    ADJUSTMENTS.iter().find(|a| a.suffix == suffix)
}

// ------------------------------------------------------------------ the event table

/// What one Photoshop step maps to.
enum Mapped {
    /// One or more PhotoCraft steps (a `make` with a name becomes new layer + rename).
    Steps(Vec<(String, Value)>),
    /// No PhotoCraft equivalent; `harmless` steps (tool selections…) are imported switched off.
    Unsupported { harmless: bool },
}

fn step(id: &str, params: Value) -> Mapped {
    Mapped::Steps(vec![(id.to_string(), params)])
}

fn steps(v: Vec<(&str, Value)>) -> Mapped {
    Mapped::Steps(v.into_iter().map(|(id, p)| (id.to_string(), p)).collect())
}

fn none(id: &str) -> Mapped {
    step(id, json!({}))
}

/// An event: code (empty when Photoshop only has a string name), string name, the panel
/// label, the command ids it exports (the first is the one simple events import to), and the
/// two converters.
struct Event {
    code: &'static str,
    name: &'static str,
    label: &'static str,
    ids: &'static [&'static str],
    from: fn(&Descriptor) -> Mapped,
    to: fn(&str, &Value) -> Option<Descriptor>,
}

macro_rules! ev {
    ($code:literal, $name:literal, $label:literal, $id:literal, $fields:expr) => {
        ev!($code, $name, $label, $id, $fields, unless | _ | false)
    };
    // `unless`: params the descriptor can't carry, so such a step isn't exported (it would replay
    // differently) rather than losing them.
    ($code:literal, $name:literal, $label:literal, $id:literal, $fields:expr, unless $skip:expr) => {
        Event {
            code: $code,
            name: $name,
            label: $label,
            ids: &[$id],
            from: |d| match fields_from(d, $fields) {
                Some(m) => step($id, Value::Object(m)),
                None => Mapped::Unsupported { harmless: false },
            },
            to: |_, p| if pixel_target(p) && !($skip)(p) && holds_its_settings($id, p) { fields_to(p, $fields, desc("null")) } else { None },
        }
    };
}

/// Minimum / Maximum with Preserve: Roundness. The descriptor here maps the radius only, and
/// Photoshop's encoding of the shape isn't verified, so such a step is not exported: it would
/// replay as the default Squareness (#7233).
fn preserves_roundness(p: &Value) -> bool {
    p.get("preserve").and_then(Value::as_str) == Some("roundness")
}

/// Whether any of the boolean `flags` is on in `p` (present, not null and not `false`).
fn flag_set(p: &Value, flags: &[&str]) -> bool {
    flags.iter().any(|f| p.get(*f).is_some_and(|v| !v.is_null() && *v != Value::Bool(false)))
}

/// Select > Modify with "Apply effect at canvas bounds": the descriptor has no field for it, so such a
/// step is not exported (it would replay without the option).
fn applies_at_canvas_bounds(p: &Value) -> bool {
    flag_set(p, &["applyAtCanvasBounds"])
}

/// Settings a mapped event has no field for, by command: `(param, the value the event plays as)`.
/// Photoshop's event has nowhere to put these, so a step that sets one cannot be written: it
/// would import as a different edit, silently. Such a step is refused, the way [`pixel_target`]
/// refuses a filter aimed at a mask -- and no key Photoshop does not define is invented for it,
/// which the clean-room rule forbids and which would make the file wrong in another reader.
///
/// A step that leaves the setting at the value the event plays as exports as before.
/// How a setting the event cannot carry behaves once the step is replayed without it. Which of
/// the two a setting takes follows the command's own reading of it, not the parameter spec.
enum Lost {
    /// Replay uses this value, so only a step already at it survives. Numbers and strings are
    /// both compared as written, since the commands below read these settings as written.
    ReplaysAs(&'static str),
    /// The command reads only this one spelling as anything other than what replay does, so every
    /// other value survives -- a differently cased one included, because it too plays as replay's.
    DiffersAt(&'static str),
}

const UNHOLDABLE: &[(&str, &[(&str, Lost)])] = &[
    // Crst carries ClSz only; replay seeds the pattern from 0 (#7668).
    ("filter.pixelate.crystallize", &[("seed", Lost::ReplaysAs("0"))]),
    // RdlB carries Amnt, BlrM and BlrQ; replay spins or zooms about the middle of the layer.
    ("filter.blur.radialBlur", &[("centerX", Lost::ReplaysAs("0.5")), ("centerY", Lost::ReplaysAs("0.5"))]),
    // Mnm /Mxm carry Rds only; replay keeps squareness, and so does the command for anything but
    // the exact string "roundness" -- see `preserve` in filters.rs.
    ("filter.other.minimum", &[("preserve", Lost::DiffersAt("roundness"))]),
    ("filter.other.maximum", &[("preserve", Lost::DiffersAt("roundness"))]),
];

/// Whether every setting of `p` that its event cannot hold would play the same without it
/// (see [`UNHOLDABLE`]).
fn holds_its_settings(id: &str, p: &Value) -> bool {
    let Some((_, settings)) = UNHOLDABLE.iter().find(|(cmd, _)| *cmd == id) else { return true };
    settings.iter().all(|(param, lost)| match (p.get(param), lost) {
        (None | Some(Value::Null), _) => true,
        (Some(v), Lost::ReplaysAs(played)) => v.as_str() == Some(played) || v.as_f64().is_some_and(|n| played.parse::<f64>().is_ok_and(|d| n == d)),
        (Some(v), Lost::DiffersAt(spelling)) => v.as_str() != Some(spelling),
    })
}

/// Whether `p` edits the layer's pixels: no `"target"` or `"target":"pixels"`. Photoshop's
/// descriptor for a plain event has no field for a mask, Quick Mask or channel target, so such
/// a step can't be exported faithfully.
fn pixel_target(p: &Value) -> bool {
    p.get("target").is_none_or(|t| t.as_str() == Some("pixels"))
}

const INTERPOLATION: &[(&str, &str)] = &[
    ("Bcbc", "bicubic"),
    ("Blnr", "bilinear"),
    ("Nrst", "nearest"),
    ("bicubicSharper", "bicubicSharper"),
    ("bicubicSmoother", "bicubicSmoother"),
    ("bicubicAutomatic", "bicubicAutomatic"),
    ("automaticInterpolation", "automatic"),
    ("preserveDetailsUpscale", "preserveDetails"),
    ("deepUpscale", "preserveDetails"),
];
const NOISE: &[(&str, &str)] = &[("Unfr", "uniform"), ("Gsn ", "gaussian")];
const OFFSET_FILL: &[(&str, &str)] = &[("Wrp ", "wrap"), ("Rpt ", "repeat"), ("Bckg", "transparent")];
const TRIM: &[(&str, &str)] = &[("Trns", "transparent"), ("topLeftPixelColor", "topLeft"), ("bottomRightPixelColor", "bottomRight")];
const EXTENSION: &[(&str, &str)] = &[("Bckg", "background"), ("Frgd", "foreground"), ("Wht ", "white"), ("Blck", "black"), ("Trns", "transparent")];
const FILL_CONTENTS: &[(&str, &str)] = &[
    ("FrgC", "foreground"),
    ("BckC", "background"),
    ("Wht ", "white"),
    ("Blck", "black"),
    ("Gry ", "gray"),
    ("Clr ", "color"),
    ("HstS", "history"),
    ("contentAware", "contentAware"),
];
const STROKE_LOCATION: &[(&str, &str)] = &[("Insd", "inside"), ("Cntr", "center"), ("OtsF", "outside")];
const INTENT: &[(&str, &str)] = &[("Img ", "perceptual"), ("Clrm", "relative"), ("Grp ", "saturation"), ("AbsC", "absolute")];
const PROFILES: &[(&str, &str)] =
    &[("sRGB IEC61966-2.1", "srgb"), ("Display P3", "display-p3"), ("Adobe RGB (1998)", "adobe-rgb-compat"), ("ProPhoto RGB", "prophoto-compat")];
/// Smart Blur quality. Photoshop's codes come first, so they are what an export writes (measured in
/// Photoshop: `Hgh `/`Mdm `/`Low ` play back as its fallback, low quality). The old codes after
/// them still read, for actions PhotoCraft wrote before.
const SMART_BLUR_QUALITY: &[(&str, &str)] = &[("SBQL", "low"), ("SBQM", "medium"), ("SBQH", "high"), ("Hgh ", "high"), ("Mdm ", "medium"), ("Low ", "low")];
const SMART_BLUR_MODE: &[(&str, &str)] = &[("SBMN", "normal"), ("SBME", "edgeOnly"), ("SBMO", "overlayEdge")];
const RADIAL_METHOD: &[(&str, &str)] = &[("Spn ", "spin"), ("Zm  ", "zoom")];
const RADIAL_QUALITY: &[(&str, &str)] = &[("Drft", "draft"), ("Gd  ", "good"), ("Bst ", "best")];
const LABEL_COLORS: &[(&str, &str)] =
    &[("None", "none"), ("Rd", "red"), ("Orng", "orange"), ("Ylw", "yellow"), ("Grn", "green"), ("Bl", "blue"), ("Vlt", "violet"), ("Gry", "gray")];
const ALIGN: &[(&str, &str)] =
    &[("AdLf", "leftEdges"), ("AdCH", "horizontalCenters"), ("AdRg", "rightEdges"), ("AdTp", "topEdges"), ("AdCV", "verticalCenters"), ("AdBt", "bottomEdges")];
const PURGE: &[(&str, &str)] = &[("Al", "all"), ("Clpb", "clipboard"), ("Hsty", "histories"), ("Undo", "undo")];
const MODES: &[(&str, &str)] = &[("RGBM", "rgb"), ("Grys", "grayscale"), ("CMYM", "cmyk"), ("LbCM", "lab"), ("Mltc", "multichannel")];

// Events whose target depends on the descriptor (`make`, `set`, `select`…): custom converters.

fn adjustment_steps(a: &Adjustment, prefix: &str, type_desc: Option<&Descriptor>) -> Mapped {
    let params = match type_desc {
        Some(t) if get_enum(t, "presetKind").as_deref() != Some("presetKindDefault") => match (a.from)(t) {
            Some(p) => p,
            None => return Mapped::Unsupported { harmless: false },
        },
        _ => json!({}),
    };
    step(&format!("{prefix}{}", a.suffix), params)
}

/// A `patternLayer` descriptor's pattern, scale, angle and link as the params of
/// `layer.newFillLayer.pattern` / `layer.fill.pattern.set`. The pattern goes by name: ids differ
/// between Photoshop's library and one rebuilt from the same `.pat` file. None for a pattern
/// recorded without a name or id.
fn pattern_params(t: &Descriptor) -> Option<Map<String, Value>> {
    let mut p = Map::new();
    if let Some(ptrn) = get_obj(t, "Ptrn") {
        let key = get_text(ptrn, "Nm  ").filter(|n| !n.is_empty()).or_else(|| get_text(ptrn, "Idnt").filter(|i| !i.is_empty()))?;
        p.insert("pattern".into(), key.into());
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
    if let Some(n) = jstr(p, "pattern") {
        t = t.with("Ptrn", D::Descriptor(desc("Ptrn").with("Nm  ", text(n))));
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

/// Layer › Layer Mask: the `UsrM` value of an Add Layer Mask `Mk` ↔ its command.
const MASK_USING: &[(&str, &str)] = &[
    ("RvlA", "layer.layerMask.revealAll"),
    ("HdAl", "layer.layerMask.hideAll"),
    ("RvlS", "layer.layerMask.revealSelection"),
    ("HdSl", "layer.layerMask.hideSelection"),
];

fn make_from(d: &Descriptor) -> Mapped {
    // Duplicate layer/group into a new document: `Mk` with a document class, a layer
    // `Usng`, an optional `Nm` (the new document's name) and an optional `LyrN`
    // (the copy's name). A `Dplc` with a `T` document reference (duplicate into another
    // open document) stays a kept Photoshop step: document indices mean nothing at replay.
    if let Some(r) = get_ref(d, "null")
        && ref_class(r).as_deref() == Some("Dcmn")
        && matches!(target(r), Target::Class)
        && let Some(u) = get_ref(d, "Usng")
        && matches!(ref_class(u).as_deref(), Some("Lyr") | Some("layerSection"))
        && let Some(mut p) = layer_target(u)
    {
        p.insert("document".into(), "new".into());
        if let Some(n) = get_text(d, "Nm  ") {
            p.insert("documentName".into(), n.into());
        }
        if let Some(n) = get_text(d, "LyrN") {
            p.insert("name".into(), n.into());
        }
        return step("layer.duplicate", Value::Object(p));
    }
    // Add Layer Mask: a `Chnl` class made at the mask channel, `Usng` what it reveals.
    if let Some(D::Class(c)) = d.get("Nw  ")
        && trimmed(&c.class_id) == "Chnl"
    {
        // The target layer's mask only: a further item (a `Lyr` by name…) puts it on another layer.
        let at_mask = get_ref(d, "At  ")
            .is_some_and(|r| r.len() == 1 && ref_class(r).as_deref() == Some("Chnl") && matches!(target(r), Target::Ordinal(o) if o == "Msk"));
        return match get_enum(d, "Usng").and_then(|u| lookup(MASK_USING, &u)) {
            Some(id) if at_mask => none(id),
            _ => Mapped::Unsupported { harmless: false },
        };
    }
    if let Some(nw) = get_obj(d, "Nw  ") {
        // Layer via Copy / Cut (`Usng` = the selection), or a new document.
        if get_enum(d, "Usng").as_deref() == Some("Slct") {
            let id = if get_bool(d, "Cpy ") == Some(true) { "layer.new.layerViaCopy" } else { "layer.new.layerViaCut" };
            let mut v = vec![(id, json!({}))];
            if let Some(n) = get_text(nw, "Nm  ") {
                v.push(("layer.setProps", json!({"name": n})));
            }
            return steps(v);
        }
        if get_text(nw, "preset").is_some() {
            return none("file.newFromClipboard");
        }
        if let (Some(w), Some(h)) = (get_num(nw, "Wdth"), get_num(nw, "Hght")) {
            let res = get_num(nw, "Rslt").unwrap_or(72.0);
            // Dimensions are in points (`#Rlt`) unless given in pixels.
            let to_px = |k: &str, v: f64| match nw.get(k) {
                Some(D::UnitFloat { unit: PX, .. }) => v,
                _ => v * res / 72.0,
            };
            let mut p = json!({"width": to_px("Wdth", w).round(), "height": to_px("Hght", h).round(), "resolution": res});
            if let Some(n) = get_text(nw, "Nm  ") {
                p["name"] = json!(n);
            }
            if let Some(D::Class(c)) = nw.get("Md  ")
                && let Some(m) = lookup(MODES, &trimmed(&c.class_id))
            {
                p["mode"] = json!(m);
            }
            if let Some(depth) = get_num(nw, "Dpth") {
                p["depth"] = json!(depth);
            }
            if let Some(f) = get_enum(nw, "Fl  ") {
                p["background"] = json!(match f.as_str() {
                    "Wht" => "white",
                    "BckC" => "backgroundColor",
                    "Trns" => "transparent",
                    _ => "white",
                });
            }
            return step("file.new", p);
        }
        return Mapped::Unsupported { harmless: false };
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
                if let Some(m) = get_enum(u, "Md  ").and_then(|m| blend_label(&m)) {
                    p.insert("mode".into(), m.into());
                }
            }
            step("layer.new.layer", Value::Object(p))
        }
        Some("AdjL") => match type_desc.and_then(|t| adjustment_by_class(&trimmed(&t.class_id))) {
            Some(a) => adjustment_steps(a, "layer.newAdjustmentLayer.", type_desc),
            None => Mapped::Unsupported { harmless: false },
        },
        Some("contentLayer") => match type_desc {
            Some(t) if t.class_id.is("solidColorLayer") => match get_obj(t, "Clr ").and_then(color_hex) {
                Some(c) => step("layer.newFillLayer.solidColor", json!({"color": c})),
                None => none("layer.newFillLayer.solidColor"),
            },
            Some(t) if t.class_id.is("patternLayer") => match pattern_params(t) {
                Some(p) => step("layer.newFillLayer.pattern", Value::Object(p)),
                None => Mapped::Unsupported { harmless: false },
            },
            _ => Mapped::Unsupported { harmless: false },
        },
        Some("layerSection") => {
            let mut p = Map::new();
            if let Some(n) = using.and_then(|u| get_text(u, "Nm  ")) {
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
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn make_to(id: &str, p: &Value) -> Option<Descriptor> {
    let with_name = |cls: &str| {
        let mut u = desc(cls);
        if let Some(n) = jstr(p, "name") {
            u = u.with("Nm  ", text(n));
        }
        u
    };
    Some(match id {
        "layer.new.layer" => {
            // The descriptor carries the name, opacity and mode only. A step that also clips to
            // the layer below, labels the layer, fills it with the neutral colour or places it
            // below the active layer would replay as a different layer, so it isn't exported
            // (strict export fails, a partial one skips it: #7502, #7512, #7513, #7514).
            let set = |key: &str, default: Value| p.get(key).is_some_and(|v| !v.is_null() && *v != default);
            if set("clipToPrevious", Value::Bool(false))
                || set("color", json!("none"))
                || set("fillNeutral", Value::Bool(false))
                || set("position", json!("above"))
            {
                return None;
            }
            let mut u = with_name("Lyr ");
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
        _ => {
            let suffix = id.strip_prefix("layer.newAdjustmentLayer.")?;
            let a = adjustment_by_suffix(suffix)?;
            let t = if p.as_object().is_some_and(|o| o.is_empty()) {
                desc(a.class).with("presetKind", enm("presetKindType", "presetKindDefault"))
            } else {
                (a.to)(p)?
            };
            desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(t))))
        }
    })
}

fn set_from(d: &Descriptor) -> Mapped {
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    let unsupported = Mapped::Unsupported { harmless: false };
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Chnl"), Target::Property(p)) if p == "fsel" => match d.get("T   ") {
            Some(D::Enumerated { value, .. }) => match trimmed(value).as_str() {
                "Al" => none("select.all"),
                "None" => none("select.deselect"),
                "Prvs" => none("select.reselect"),
                _ => unsupported,
            },
            Some(D::Reference(t)) => match (ref_class(t).as_deref(), target(t)) {
                (Some("Chnl"), Target::Ordinal(o)) if o == "Trsp" => step("select.loadSelection", json!({"channel": "transparency"})),
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
                let (Some(top), Some(left), Some(bottom), Some(right)) = (get_num(t, "Top "), get_num(t, "Left"), get_num(t, "Btom"), get_num(t, "Rght"))
                else {
                    return unsupported;
                };
                let mut p = json!({"x": left.round(), "y": top.round(), "width": (right - left).round().max(0.0), "height": (bottom - top).round().max(0.0)});
                if t.class_id.is("Elps") {
                    p["ellipse"] = json!(true);
                }
                step("select.rect", p)
            }
            _ => unsupported,
        },
        // Quick Mask mode on (`set` of the document's quick-mask property); `clearEvent` turns it off.
        (Some("Prpr"), Target::Property(p)) if p == "QucM" => step("select.editInQuickMaskMode", json!({"on": true})),
        // Layer › New › Layer From Background (Photoshop records the New Layer dialog's values
        // as a `set` of the Background); values other than the defaults then apply to the layer.
        (Some("Lyr"), Target::Property(p)) if p == "Bckg" => {
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
        // The toolbox colours.
        (Some("Clr"), Target::Property(p)) if p == "FrgC" || p == "BckC" => match get_obj(d, "T   ").and_then(color_hex) {
            Some(hex) => step("tools.setColors", json!({(if p == "FrgC" { "foreground" } else { "background" }): hex})),
            None => unsupported,
        },
        (Some("Lyr"), _) => {
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
            if p.keys().any(|k| k != "layer") {
                v.push(("layer.setProps", Value::Object(p)));
            }
            if v.is_empty() { unsupported } else { steps(v) }
        }
        // A Pattern Fill layer's pattern (double-click its thumbnail).
        (Some("contentLayer"), Target::Ordinal(o)) if o == "Trgt" => match get_obj(d, "T   ") {
            Some(t) if t.class_id.is("patternLayer") => match pattern_params(t) {
                Some(p) if !p.is_empty() => step("layer.fill.pattern.set", Value::Object(p)),
                _ => unsupported,
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

fn set_to(id: &str, p: &Value) -> Option<Descriptor> {
    let fsel = D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]);
    let sel = |t: D| desc("null").with("null", fsel.clone()).with("T   ", t);
    Some(match id {
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
            "transparency" => sel(D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") }])),
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
            desc("null").with("null", layer_ref(p)?).with("T   ", D::Descriptor(t))
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
        "layer.setAdjustment" => {
            // The adjustment kind isn't in the params: Photoshop needs it, so only steps that
            // name one (`kind`) export.
            let a = adjustment_by_suffix(jstr(p, "kind")?)?;
            desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor((a.to)(p)?))
        }
        _ => return None,
    })
}

/// Tool classes Photoshop records when an action switches tools: no document effect.
fn is_tool_class(c: &str) -> bool {
    c.ends_with("Tl") || c.ends_with("Tool")
}

fn select_from(d: &Descriptor) -> Mapped {
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Lyr"), Target::Name(n)) => {
            let mut p = json!({"layer": n});
            match get_enum(d, "selectionModifier").as_deref() {
                Some("addToSelection") => p["mode"] = json!("add"),
                Some("removeFromSelection") => p["mode"] = json!("remove"),
                // Shift-click: select everything between the active layer and this one.
                Some("addToSelectionContinuous") => p["mode"] = json!("range"),
                _ => {}
            }
            step("layer.select", p)
        }
        // The Channels panel: the active layer's mask, or back to the composite (`RGB`, `CMYK`…).
        (Some("Chnl"), Target::Ordinal(o)) => match o.as_str() {
            "Msk" => step("channel.target", json!({"channel": "mask"})),
            "RGB" | "CMYK" | "Lab" | "Gry" => step("channel.target", json!({"channel": "composite"})),
            // Re-selecting the channel that is already targeted changes nothing.
            "Trgt" => Mapped::Unsupported { harmless: true },
            _ => Mapped::Unsupported { harmless: false },
        },
        // History › Step Backward / Step Forward.
        (Some("HstS"), Target::Ordinal(o)) if o == "Prvs" => none("edit.undo"),
        (Some("HstS"), Target::Ordinal(o)) if o == "Nxt" => none("edit.redo"),
        (Some("Lyr"), Target::Ordinal(o)) => {
            // Shift+Alt+[ / ] add the next layer to the selection instead of replacing it.
            let add = get_enum(d, "selectionModifier").as_deref() == Some("addToSelection");
            match (o.as_str(), add) {
                ("Frwr", false) => none("layer.selectAbove"),
                ("Frwr", true) => none("layer.addAboveToSelection"),
                ("Bckw", false) => none("layer.selectBelow"),
                ("Bckw", true) => none("layer.addBelowToSelection"),
                ("Frnt", _) => none("layer.selectTop"),
                ("Back", _) => none("layer.selectBottom"),
                _ => Mapped::Unsupported { harmless: false },
            }
        }
        (Some(c), Target::Class) if is_tool_class(c) => Mapped::Unsupported { harmless: true },
        // A menu item chosen while recording (`Mn`): the View menu's zoom items replay.
        (Some("Mn"), Target::Ordinal(item)) => match lookup(MENU_ITEMS, &item) {
            Some(id) => none(id),
            None => Mapped::Unsupported { harmless: false },
        },
        // Switching to another open document: its offset from the active one, -1 for the previous
        // tab and 1 for the next (a template action copies from one and pastes into the other, #7103).
        (Some("Dcmn"), Target::Offset(o)) => step("document.activate", json!({"offset": o})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// `MnIt` menu-item codes ↔ the shell's view commands.
const MENU_ITEMS: &[(&str, &str)] = &[("FtOn", "view.fitOnScreen"), ("ActP", "view.actualPixels"), ("ZmIn", "view.zoomIn"), ("ZmOt", "view.zoomOut")];

fn select_to(id: &str, p: &Value) -> Option<Descriptor> {
    let ord = |o: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code(o) }]);
    let enumerated = |cls: &str, ty: &str, v: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: code(ty), value: code(v) }]);
    Some(match id {
        "layer.select" => {
            if p.get("occurrence").is_some() {
                return None;
            }
            let name = jstr(p, "name").or_else(|| jstr(p, "layer"))?;
            let mut d = desc("null").with("null", name_ref("Lyr ", name));
            match jstr(p, "mode") {
                None | Some("replace") => {}
                Some("add") => d = d.with("selectionModifier", enm("selectionModifierType", "addToSelection")),
                Some("remove") => d = d.with("selectionModifier", enm("selectionModifierType", "removeFromSelection")),
                Some("range") => d = d.with("selectionModifier", enm("selectionModifierType", "addToSelectionContinuous")),
                Some(_) => return None,
            }
            d.with("MkVs", D::Boolean(false))
        }
        "channel.target" => {
            let v = match jstr(p, "channel") {
                Some("mask") => "Msk ",
                None | Some("composite") => "RGB ",
                Some(_) => return None,
            };
            desc("null").with("null", enumerated("Chnl", "Chnl", v)).with("MkVs", D::Boolean(false))
        }
        "edit.undo" => desc("null").with("null", enumerated("HstS", "Ordn", "Prvs")),
        "edit.redo" => desc("null").with("null", enumerated("HstS", "Ordn", "Nxt ")),
        "layer.selectAbove" => desc("null").with("null", ord("Frwr")),
        "layer.selectBelow" => desc("null").with("null", ord("Bckw")),
        "layer.addAboveToSelection" => desc("null").with("null", ord("Frwr")).with("selectionModifier", enm("selectionModifierType", "addToSelection")),
        "layer.addBelowToSelection" => desc("null").with("null", ord("Bckw")).with("selectionModifier", enm("selectionModifierType", "addToSelection")),
        "layer.selectTop" => desc("null").with("null", ord("Frnt")),
        "layer.selectBottom" => desc("null").with("null", ord("Back")),
        // An absolute tab index has no Photoshop equivalent; an offset is what Photoshop records.
        "document.activate" if p.get("document").is_none_or(Value::is_null) => {
            let offset = i32::try_from(p.get("offset")?.as_i64()?).ok()?;
            desc("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]))
        }
        _ => {
            let item = lookup_back(MENU_ITEMS, id)?;
            desc("null").with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Mn  "), type_id: code("MnIt"), value: code(item) }]))
        }
    })
}

fn delete_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null") {
        None => none("edit.clear"),
        Some(r) if ref_class(r).as_deref() == Some("Lyr") => match layer_target(r) {
            Some(p) => step("layer.delete", Value::Object(p)),
            None => Mapped::Unsupported { harmless: false },
        },
        // The targeted channel of a layer is its mask (what Photoshop records after making an
        // adjustment layer over a selection and discarding the mask).
        Some(r) if ref_class(r).as_deref() == Some("Chnl") && matches!(target(r), Target::Ordinal(o) if o == "Trgt") => none("layer.layerMask.delete"),
        Some(_) => Mapped::Unsupported { harmless: false },
    }
}

fn quick_mask_ref() -> D {
    D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: code("QucM") },
        ReferenceItem::Enumerated { class: class("Dcmn"), type_id: code("Ordn"), value: code("Trgt") },
    ])
}

fn clear_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Property(p)) if p == "QucM" => step("select.editInQuickMaskMode", json!({"on": false})),
        None => none("edit.clear"),
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn clear_to(id: &str, p: &Value) -> Option<Descriptor> {
    match id {
        "select.editInQuickMaskMode" if jbool(p, "on") == Some(false) => Some(desc("null").with("null", quick_mask_ref())),
        _ => None,
    }
}

/// `placeEvent`: File › Place Embedded / Linked, with the file and the recorded scale.
fn place_from(d: &Descriptor) -> Mapped {
    let Some(path) = d.get("null").and_then(path_text) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"path": path});
    if let Some(w) = get_num(d, "Wdth") {
        p["scale"] = json!(w);
    }
    step(if get_bool(d, "Lnkd") == Some(true) { "file.placeLinked" } else { "file.placeEmbedded" }, p)
}

fn place_to(id: &str, p: &Value) -> Option<Descriptor> {
    let mut d = desc("null").with("null", path_value(jstr(p, "path")?)).with("FTcs", enm("QCSt", "Qcsa"));
    if let Some(s) = jnum(p, "scale") {
        d = d.with("Wdth", pct(s)).with("Hght", pct(s));
    }
    Some(d.with("Lnkd", D::Boolean(id == "file.placeLinked")).with("Antl", D::Boolean(true)))
}

/// `open`: File › Open of a recorded path.
fn open_from(d: &Descriptor) -> Mapped {
    match d.get("null").and_then(path_text) {
        Some(path) => step("file.openAs", json!({"path": path})),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn open_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", path_value(jstr(p, "path")?)))
}

/// Photoshop's four-character condition codes that differ from the string ids.
const CONDITION_CODES: &[(&str, &str)] = &[("Lyrs", "hasLayers")];

/// `conditional`: Insert Conditional, with its Then and Else actions by name.
fn conditional_from(d: &Descriptor) -> Mapped {
    let Some(c) = get_enum(d, "null") else { return Mapped::Unsupported { harmless: false } };
    let condition = lookup(CONDITION_CODES, &c).unwrap_or(c.as_str());
    if !crate::actions_edit_cmds::CONDITIONS.contains(&condition) {
        return Mapped::Unsupported { harmless: false };
    }
    let branch = |key: &str| match get_ref(d, key).map(target) {
        Some(Target::Name(n)) => Some(n),
        _ => None,
    };
    step("actions.conditional", json!({"if": condition, "then": branch("then"), "else": branch("else")}))
}

fn conditional_to(_: &str, p: &Value) -> Option<Descriptor> {
    let condition = jstr(p, "if")?;
    let code = lookup_back(CONDITION_CODES, condition).unwrap_or(condition);
    let mut d = desc("null").with("null", enm("Cndt", code));
    for key in ["then", "else"] {
        if let Some(name) = jstr(p, key) {
            let mut items = vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul(name) }];
            if let Some(set) = jstr(p, "set") {
                items.push(ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul(set) });
            }
            d = d.with(key, D::Reference(items));
        }
    }
    Some(d)
}

/// `close`: Photoshop's Save prompt answer becomes a Save step before the Close.
fn close_from(d: &Descriptor) -> Mapped {
    if get_enum(d, "Svng").as_deref() == Some("Ys") { steps(vec![("file.save", json!({})), ("file.close", json!({}))]) } else { none("file.close") }
}

/// A Close of an explicit document has no mapping here: written as a plain Close it would
/// close the active document on playback, so it is left to `skipUnsupported`.
fn close_to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.get("document").is_some_and(|d| !d.is_null()) {
        return None;
    }
    Some(desc("null").with("Svng", enm("YsN ", "N   ")))
}

/// `transform`: Photoshop records a Free Transform relative to the frame (offset of its centre,
/// width/height in percent, angle). A layer moved only becomes `layer.translate`; anything
/// else is `edit.transform`'s relative form; the selection's frame is `select.transformSelection`.
fn transform_from(d: &Descriptor) -> Mapped {
    // No reference (a smart object replaced in place, `replaceLayer`) means the target layer.
    let target_layer = [ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") }];
    let r: &[ReferenceItem] = get_ref(d, "null").unwrap_or(&target_layer);
    let unsupported = Mapped::Unsupported { harmless: false };
    let mut p = Map::new();
    if let Some(o) = get_obj(d, "Ofst") {
        let in_px = |k: &str| matches!(o.get(k), Some(D::UnitFloat { unit: PX, .. }));
        if !in_px("Hrzn") || !in_px("Vrtc") {
            return unsupported;
        }
        if let Some(x) = get_num(o, "Hrzn") {
            p.insert("dx".into(), x.into());
        }
        if let Some(y) = get_num(o, "Vrtc") {
            p.insert("dy".into(), y.into());
        }
    }
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY"), ("Angl", "rotate")] {
        if let Some(v) = get_num(d, key) {
            p.insert(param.into(), v.into());
        }
    }
    if d.get("Skew").is_some() || d.get("Usng").is_some() {
        return unsupported;
    }
    // A whole-pixel move is a plain `layer.translate`; a fractional one needs `edit.transform`
    // to keep its sub-pixel placement and interpolation.
    let whole_pixels = ["dx", "dy"].iter().all(|k| p.get(*k).and_then(Value::as_f64).is_none_or(|v| v.fract() == 0.0));
    let geometry_only = p.keys().all(|k| k == "dx" || k == "dy") && whole_pixels;
    // The resampling applies to a selection as much as to a layer (#7515).
    let interpolation = get_enum(d, "Intr").and_then(|i| lookup(INTERPOLATION, &i)).filter(|i| matches!(*i, "bicubic" | "bilinear" | "nearest"));
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Chnl"), Target::Property(prop)) if prop == "fsel" => {
            if let Some(i) = interpolation {
                p.insert("interpolation".into(), i.into());
            }
            step("select.transformSelection", Value::Object(p))
        }
        (Some("Lyr"), _) => {
            let Some(mut lp) = layer_target(r) else { return unsupported };
            if geometry_only {
                lp.insert("dx".into(), p.get("dx").and_then(Value::as_f64).unwrap_or(0.0).into());
                lp.insert("dy".into(), p.get("dy").and_then(Value::as_f64).unwrap_or(0.0).into());
                return step("layer.translate", Value::Object(lp));
            }
            lp.extend(p);
            if let Some(i) = interpolation {
                lp.insert("interpolation".into(), i.into());
            }
            step("edit.transform", Value::Object(lp))
        }
        _ => unsupported,
    }
}

fn transform_to(id: &str, p: &Value) -> Option<Descriptor> {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if ["quad", "corners", "matrix", "rect"].iter().any(|k| given(k)) {
        return None;
    }
    // The descriptor scales and rotates about the centre (`FTcs` average) without skew or warp:
    // another reference point, a skew or a warp would replay as a different transform (#7516,
    // #7517, #7518, #7519).
    let nonzero = |k: &str| p.get(k).is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0));
    let off_centre = p.get("reference").is_some_and(|v| !v.is_null() && *v != json!("center"));
    if nonzero("skewX") || nonzero("skewY") || off_centre || ["warp", "style", "mesh", "grid"].iter().any(|k| given(k)) {
        return None;
    }
    let target = match id {
        // The `Trnf` descriptor has no field for a mask, Quick Mask or alpha channel target; a
        // layer reference would transform the layer's pixels instead.
        "edit.transform" if !pixel_target(p) => return None,
        "edit.transform" => layer_ref(p)?,
        "select.transformSelection" => D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }]),
        _ => return None,
    };
    let mut d = desc("null").with("null", target).with("FTcs", enm("QCSt", "Qcsa"));
    d = d.with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(jnum(p, "dx").unwrap_or(0.0))).with("Vrtc", px(jnum(p, "dy").unwrap_or(0.0)))));
    for (key, param) in [("Wdth", "scaleX"), ("Hght", "scaleY")] {
        if let Some(v) = jnum(p, param) {
            d = d.with(key, pct(v));
        }
    }
    if let Some(a) = jnum(p, "rotate") {
        d = d.with("Angl", ang(a));
    }
    if let Some(i) = jstr(p, "interpolation").and_then(|i| lookup_back(INTERPOLATION, i)) {
        d = d.with("Intr", enm("Intp", i));
    }
    Some(d)
}

/// `Clcn` (Apply Image / Calculations blending) ↔ `image.applyImage`'s `blending`.
const CALC_BLEND: &[(&str, &str)] = &[
    ("Nrml", "normal"),
    ("Mltp", "multiply"),
    ("Scrn", "screen"),
    ("Ovrl", "overlay"),
    ("SftL", "softLight"),
    ("HrdL", "hardLight"),
    ("CDdg", "colorDodge"),
    ("CBrn", "colorBurn"),
    ("Drkn", "darken"),
    ("Lghn", "lighten"),
    ("Dfrn", "difference"),
    ("Xclu", "exclusion"),
    ("linearBurn", "linearBurn"),
    ("linearDodge", "linearDodge"),
    ("Add ", "add"),
    ("Sbtr", "subtract"),
];
const SOURCE_CHANNELS: &[(&str, &str)] =
    &[("RGB", "composite"), ("CMYK", "composite"), ("Lab", "composite"), ("Trsp", "transparency"), ("Msk", "mask"), ("fsel", "selection")];

fn apply_image_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(with) = get_obj(d, "With") else { return unsupported };
    let Some(r) = get_ref(with, "T   ") else { return unsupported };
    let mut source = Map::new();
    for item in r {
        match item {
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Chnl") => {
                let v = trimmed(value);
                let Some(channel) = lookup(SOURCE_CHANNELS, &v).or_else(|| lookup(CHANNELS, &v)) else { return unsupported };
                source.insert("channel".into(), channel.into());
            }
            ReferenceItem::Name { class, name } if class.class_id.is("Lyr ") => {
                source.insert("layer".into(), name.to_string_lossy().into());
            }
            ReferenceItem::Enumerated { class, value, .. } if class.class_id.is("Lyr ") && trimmed(value) == "Mrgd" => {
                source.insert("layer".into(), "merged".into());
            }
            _ => return unsupported,
        }
    }
    if let Some(i) = get_bool(with, "Invr") {
        source.insert("invert".into(), i.into());
    }
    let mut p = json!({"source": source});
    if let Some(c) = get_enum(with, "Clcl") {
        match lookup(CALC_BLEND, &c) {
            Some(b) => p["blending"] = json!(b),
            None => return unsupported,
        }
    }
    for (key, param) in [("Scl ", "scale"), ("Ofst", "offset"), ("Opct", "opacity")] {
        if let Some(v) = get_num(with, key) {
            p[param] = json!(v);
        }
    }
    if let Some(t) = get_bool(with, "PrsT") {
        p["preserveTransparency"] = json!(t);
    }
    step("image.applyImage", p)
}

fn apply_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    // The optional blending mask has no descriptor mapping: refuse it rather than drop it,
    // which would widen the edit to the whole layer.
    let mask_channel = match p.get("mask") {
        Some(m) => m.get("channel"),
        None => p.get("maskChannel"),
    };
    if mask_channel.is_some_and(|c| !(c.is_null() || c == "none" || c == "")) {
        return None;
    }
    let source = p.get("source").cloned().unwrap_or(json!({}));
    let channel = jstr(&source, "channel").unwrap_or("composite");
    let chan = lookup_back(SOURCE_CHANNELS, channel).or_else(|| lookup_back(CHANNELS, channel))?;
    let mut items = vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(chan) }];
    match jstr(&source, "layer") {
        Some("merged") | None => items.push(ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Mrgd") }),
        Some(name) => items.push(ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) }),
    }
    let mut with = desc("null").with("T   ", D::Reference(items));
    if let Some(i) = jbool(&source, "invert") {
        with = with.with("Invr", D::Boolean(i));
    }
    with = with.with("Clcl", enm("Clcn", lookup_back(CALC_BLEND, jstr(p, "blending").unwrap_or("multiply"))?));
    if let Some(v) = jnum(p, "opacity") {
        with = with.with("Opct", pct(v));
    }
    if let Some(v) = jnum(p, "scale") {
        with = with.with("Scl ", D::Double(v));
    }
    if let Some(v) = jnum(p, "offset") {
        with = with.with("Ofst", D::Double(v));
    }
    if let Some(t) = jbool(p, "preserveTransparency") {
        with = with.with("PrsT", D::Boolean(t));
    }
    Some(desc("null").with("With", D::Descriptor(with)))
}

const WEB_FORMATS: &[(&str, &str)] = &[("PN24", "png24"), ("PN8", "png8"), ("JPEG", "jpeg"), ("GIFf", "gif"), ("WBMP", "wbmp")];

/// `export` (Save for Web): needs the destination path (`In`), as the command writes a file.
fn export_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(u) = get_obj(d, "Usng") else { return unsupported };
    let Some(path) = d.get("In  ").and_then(path_text) else { return unsupported };
    let mut p = json!({"path": path});
    if let Some(f) = get_enum(u, "Fmt ") {
        match lookup(WEB_FORMATS, &f) {
            Some(f) => p["format"] = json!(f),
            None => return unsupported,
        }
    }
    for (key, param) in [("Intr", "interlaced"), ("Trns", "transparency"), ("EICC", "embedIcc"), ("Prgr", "progressive"), ("Optm", "optimized")] {
        if let Some(b) = get_bool(u, key) {
            p[param] = json!(b);
        }
    }
    for (key, param) in [("Qlty", "quality"), ("NCol", "colors")] {
        if let Some(v) = get_num(u, key) {
            p[param] = json!(v);
        }
    }
    if get_bool(u, "Mtt ") == Some(false) {
        p["matte"] = json!("none");
    } else if let (Some(r), Some(g), Some(b)) = (get_num(u, "MttR"), get_num(u, "MttG"), get_num(u, "MttB")) {
        p["matte"] =
            json!(format!("#{:02x}{:02x}{:02x}", r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8));
    }
    step("file.export.saveForWebLegacy", p)
}

fn export_to(_: &str, p: &Value) -> Option<Descriptor> {
    let path = jstr(p, "path")?;
    let mut u = desc("SaveForWeb").with("Op  ", enm("SWOp", "OpSa")).with("Fmt ", enm("IRFm", lookup_back(WEB_FORMATS, jstr(p, "format").unwrap_or("png24"))?));
    for (key, param, default) in [
        ("Intr", "interlaced", false),
        ("Trns", "transparency", true),
        ("EICC", "embedIcc", false),
        ("Prgr", "progressive", false),
        ("Optm", "optimized", true),
    ] {
        u = u.with(key, D::Boolean(jbool(p, param).unwrap_or(default)));
    }
    if let Some(q) = jnum(p, "quality") {
        u = u.with("Qlty", long(q));
    }
    if let Some(c) = jnum(p, "colors") {
        u = u.with("NCol", long(c));
    }
    match jstr(p, "matte") {
        Some("none") => u = u.with("Mtt ", D::Boolean(false)),
        matte => {
            let c = color_desc(matte.unwrap_or("#ffffff"))?;
            u = u
                .with("Mtt ", D::Boolean(true))
                .with("MttR", get_num(&c, "Rd  ").map(long)?)
                .with("MttG", get_num(&c, "Grn ").map(long)?)
                .with("MttB", get_num(&c, "Bl  ").map(long)?);
        }
    }
    Some(desc("null").with("Usng", D::Descriptor(u)).with("In  ", path_value(path)))
}

fn delete_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "edit.clear" => desc("null"),
        "layer.delete" => desc("null").with("null", layer_ref(p)?),
        "layer.layerMask.delete" => desc("null").with("null", target_ref("Chnl")),
        _ => return None,
    })
}

/// `hide` / `show` take a list of references (or one).
fn visibility_from(d: &Descriptor, id: &str) -> Mapped {
    let refs: Vec<&[ReferenceItem]> = match d.get("null") {
        Some(D::List(l)) => l.iter().filter_map(|v| if let D::Reference(r) = v { Some(r.as_slice()) } else { None }).collect(),
        Some(D::Reference(r)) => vec![r.as_slice()],
        _ => return Mapped::Unsupported { harmless: false },
    };
    let mut v = Vec::new();
    for r in refs {
        match layer_target(r) {
            Some(p) => v.push((id, Value::Object(p))),
            None => return Mapped::Unsupported { harmless: false },
        }
    }
    if v.is_empty() { Mapped::Unsupported { harmless: false } } else { steps(v) }
}

fn visibility_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", D::List(vec![layer_ref(p)?])))
}

fn move_from(d: &Descriptor) -> Mapped {
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    // The selection outline nudged.
    if ref_class(r).as_deref() == Some("Chnl") && matches!(target(r), Target::Property(p) if p == "fsel") {
        return match get_obj(d, "T   ") {
            Some(t) if matches!(t.get("Hrzn"), Some(D::UnitFloat { unit: PX, .. })) => match (get_num(t, "Hrzn"), get_num(t, "Vrtc")) {
                (Some(dx), Some(dy)) => step("select.transformSelection", json!({"dx": dx.round(), "dy": dy.round()})),
                _ => Mapped::Unsupported { harmless: false },
            },
            _ => Mapped::Unsupported { harmless: false },
        };
    }
    if ref_class(r).as_deref() != Some("Lyr") {
        return Mapped::Unsupported { harmless: false };
    }
    let Some(mut p) = layer_target(r) else { return Mapped::Unsupported { harmless: false } };
    match d.get("T   ") {
        Some(D::Descriptor(t)) => {
            let unit = |k: &str| match t.get(k) {
                Some(D::UnitFloat { unit, .. }) => Some(*unit),
                _ => None,
            };
            let (Some(dx), Some(dy)) = (get_num(t, "Hrzn"), get_num(t, "Vrtc")) else { return Mapped::Unsupported { harmless: false } };
            match (unit("Hrzn"), unit("Vrtc")) {
                (Some(PX), Some(PX)) => {
                    p.insert("dx".into(), dx.round().into());
                    p.insert("dy".into(), dy.round().into());
                }
                // A nudge recorded with the rulers in other units: points, converted to pixels
                // at the resolution of the document the action plays on.
                (Some(PT), Some(PT)) => {
                    p.insert("dx".into(), dx.into());
                    p.insert("dy".into(), dy.into());
                    p.insert("units".into(), "pt".into());
                }
                _ => return Mapped::Unsupported { harmless: false },
            }
            step("layer.translate", Value::Object(p))
        }
        Some(D::Reference(t)) => {
            let id = match target(t) {
                Target::Ordinal(o) if o == "Frnt" => "layer.arrange.bringToFront",
                Target::Ordinal(o) if o == "Back" => "layer.arrange.sendToBack",
                Target::Ordinal(o) if o == "Nxt" => "layer.arrange.bringForward",
                Target::Ordinal(o) if o == "Prvs" => "layer.arrange.sendBackward",
                Target::Offset(1) => "layer.arrange.bringForward",
                Target::Offset(-1) => "layer.arrange.sendBackward",
                // Index 0 is the bottom of the stack.
                Target::Index(0) => "layer.arrange.sendToBack",
                _ => return Mapped::Unsupported { harmless: false },
            };
            step(id, Value::Object(p))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn move_to(id: &str, p: &Value) -> Option<Descriptor> {
    let ord = |o: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code(o) }]);
    let t = match id {
        "layer.translate" => {
            let unit = match jstr(p, "units") {
                None | Some("px") => PX,
                Some("pt") => PT,
                Some(_) => return None,
            };
            let v = |k: &str| D::UnitFloat { unit, value: jnum(p, k).unwrap_or(0.0) };
            D::Descriptor(desc("Ofst").with("Hrzn", v("dx")).with("Vrtc", v("dy")))
        }
        "layer.arrange.bringToFront" => ord("Frnt"),
        "layer.arrange.sendToBack" => ord("Back"),
        "layer.arrange.bringForward" => ord("Nxt "),
        "layer.arrange.sendBackward" => ord("Prvs"),
        _ => return None,
    };
    Some(desc("null").with("null", layer_ref(p)?).with("T   ", t))
}

fn duplicate_from(d: &Descriptor) -> Mapped {
    let Some(r) = get_ref(d, "null") else { return Mapped::Unsupported { harmless: false } };
    match ref_class(r).as_deref() {
        Some("Lyr") => {
            // A `T` document reference (duplicate into another open document) has no
            // replayable meaning here: keep the step verbatim rather than misplay it.
            if get_ref(d, "T   ").is_some() {
                return Mapped::Unsupported { harmless: false };
            }
            let Some(mut p) = layer_target(r) else { return Mapped::Unsupported { harmless: false } };
            if let Some(n) = get_text(d, "Nm  ") {
                p.insert("name".into(), n.into());
            }
            step("layer.duplicate", Value::Object(p))
        }
        Some("Dcmn") => {
            let mut p = Map::new();
            if let Some(n) = get_text(d, "Nm  ") {
                p.insert("name".into(), n.into());
            }
            if let Some(m) = get_bool(d, "Mrgd") {
                p.insert("mergedOnly".into(), m.into());
            }
            step("image.duplicate", Value::Object(p))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn duplicate_to(id: &str, p: &Value) -> Option<Descriptor> {
    // Duplicate into a new document is a `Mk` (handled above); let it fall through.
    if id == "layer.duplicate" && jstr(p, "document") == Some("new") {
        return None;
    }
    // Photoshop's Duplicate has no in-place flag: a plain one would place the artboard copy beside
    // the original on playback.
    if id == "layer.duplicate" && jbool(p, "inPlace") == Some(true) {
        return None;
    }
    let mut d = match id {
        "layer.duplicate" => desc("null").with("null", layer_ref(p)?),
        "image.duplicate" => {
            desc("null").with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Dcmn"), type_id: Id::new("Ordn"), value: Id::new("Frst") }]))
        }
        _ => return None,
    };
    if let Some(n) = jstr(p, "name") {
        d = d.with("Nm  ", text(n));
    }
    if let Some(m) = jbool(p, "mergedOnly") {
        d = d.with("Mrgd", D::Boolean(m));
    }
    Some(d)
}

/// Only a comp name in the active document is portable across action playback.
fn apply_comp_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null") {
        Some([ReferenceItem::Name { class, name }]) if trimmed(&class.class_id) == "compsClass" => {
            step("layerComp.apply", json!({"comp": name.to_string_lossy()}))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn apply_comp_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", name_ref("compsClass", jstr(p, "comp")?)))
}

fn play_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Name(n)) => step("actions.play", json!({"action": n})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn play_to(_: &str, p: &Value) -> Option<Descriptor> {
    let name = jstr(p, "action")?;
    // The Play Action descriptor written here carries no starting step, so a call from a later step is not a whole call.
    if p.get("from").is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0)) {
        return None;
    }
    let mut items = vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul(name) }];
    if let Some(set) = jstr(p, "set") {
        items.push(ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul(set) });
    }
    Some(desc("null").with("null", D::Reference(items)))
}

fn fill_from(d: &Descriptor) -> Mapped {
    if let Some(from) = get_obj(d, "From") {
        // The Paint Bucket tool's click: its point is in pixels, or in percent of the canvas
        // (not replayable without the document size).
        let in_px = |k: &str| matches!(from.get(k), Some(D::UnitFloat { unit: PX, .. }));
        let (Some(x), Some(y)) = (get_num(from, "Hrzn"), get_num(from, "Vrtc")) else { return Mapped::Unsupported { harmless: false } };
        if !in_px("Hrzn") || !in_px("Vrtc") {
            return Mapped::Unsupported { harmless: false };
        }
        let mut p = json!({"x": x.round(), "y": y.round()});
        match get_enum(d, "Usng").as_deref() {
            Some("FrgC") | None => {}
            Some("Pttn") => p["contents"] = json!("pattern"),
            Some("Clr") => match get_obj(d, "Clr ").and_then(color_hex) {
                Some(c) => p["color"] = json!(c),
                None => return Mapped::Unsupported { harmless: false },
            },
            Some(_) => return Mapped::Unsupported { harmless: false },
        }
        if let Some(t) = get_num(d, "Tlrn") {
            p["tolerance"] = json!(t);
        }
        if let Some(c) = get_bool(d, "Cntg") {
            p["contiguous"] = json!(c);
        }
        if let Some(a) = get_bool(d, "AntA") {
            p["antiAlias"] = json!(a);
        }
        if let Some(m) = get_bool(d, "Mrgd") {
            p["sampleAllLayers"] = json!(m);
        }
        if let Some(o) = get_num(d, "Opct") {
            p["opacity"] = json!(o);
        }
        return step("paint.bucket", p);
    }
    let Some(contents) = get_enum(d, "Usng").and_then(|u| lookup(FILL_CONTENTS, &u)) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"contents": contents});
    if contents == "color" {
        match get_obj(d, "Clr ").and_then(color_hex) {
            Some(c) => p["color"] = json!(c),
            None => return Mapped::Unsupported { harmless: false },
        }
    }
    if let Some(o) = get_num(d, "Opct") {
        p["opacity"] = json!(o);
    }
    if let Some(m) = get_enum(d, "Md  ") {
        match blend_label(&m) {
            Some(l) => p["mode"] = json!(l),
            None => return Mapped::Unsupported { harmless: false },
        }
    }
    if let Some(t) = get_bool(d, "PrsT") {
        p["preserveTransparency"] = json!(t);
    }
    step("edit.fill", p)
}

fn fill_to(id: &str, p: &Value) -> Option<Descriptor> {
    if id == "paint.bucket" {
        let mut d = desc("null").with("From", D::Descriptor(desc("Pnt ").with("Hrzn", px(jnum(p, "x")?)).with("Vrtc", px(jnum(p, "y")?))));
        d = d.with("Tlrn", long(jnum(p, "tolerance").unwrap_or(32.0))).with("AntA", D::Boolean(jbool(p, "antiAlias").unwrap_or(true)));
        // `source` is the command's older name for `contents`.
        d = if p.get("contents").or_else(|| p.get("source")).and_then(Value::as_str) == Some("pattern") {
            // `Pttn` names no pattern and holds no placement: a chosen pattern, a scale, an angle
            // or a phase would replay with the current pattern at its defaults (#7543, #7544,
            // #7545, #7546).
            let num = |k: &str, default: f64| p.get(k).is_some_and(|v| !v.is_null() && v.as_f64() != Some(default));
            let phase = p.get("phase").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| a.iter().any(|c| c.as_f64() != Some(0.0))));
            if p.get("pattern").is_some_and(|v| !v.is_null()) || num("scale", 100.0) || num("angle", 0.0) || phase {
                return None;
            }
            d.with("Usng", enm("FlCn", "Pttn"))
        } else {
            // An explicit colour overrides the foreground (#7199): it travels as Fill's `Clr `.
            match bucket_color(p.get("color"))? {
                Some(c) => d.with("Usng", enm("FlCn", "Clr ")).with("Clr ", D::Descriptor(c)),
                None => d.with("Usng", enm("FlCn", "FrgC")),
            }
        };
        if let Some(o) = jnum(p, "opacity") {
            d = d.with("Opct", pct(o));
        }
        if let Some(c) = jbool(p, "contiguous") {
            d = d.with("Cntg", D::Boolean(c));
        }
        if let Some(m) = jbool(p, "sampleAllLayers") {
            d = d.with("Mrgd", D::Boolean(m));
        }
        return Some(d);
    }
    let contents = jstr(p, "contents").unwrap_or("color");
    let mut d = desc("null").with("Usng", enm("FlCn", lookup_back(FILL_CONTENTS, contents)?));
    if contents == "color" {
        d = d.with("Clr ", D::Descriptor(color_desc(jstr(p, "color")?)?));
    }
    if let Some(o) = jnum(p, "opacity") {
        d = d.with("Opct", pct(o));
    }
    if let Some(m) = jstr(p, "mode") {
        d = d.with("Md  ", enm("BlnM", blend_ps(m)?));
    }
    if let Some(t) = jbool(p, "preserveTransparency") {
        d = d.with("PrsT", D::Boolean(t));
    }
    Some(d)
}

/// The Paint Bucket's `color` as `paint.bucket` reads it (`[r, g, b(, a)]` in 0..=1 or
/// `#rrggbb(aa)`): `Some(None)` when it fills with the foreground (no colour, or a string the
/// command doesn't read), `None` when the colour can't be written (not opaque, or outside 0..=1).
fn bucket_color(v: Option<&Value>) -> Option<Option<Descriptor>> {
    let rgba = match v {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            [*c.first()?, *c.get(1)?, *c.get(2)?, c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f64::from(v) / 255.0);
            let (Some(r), Some(g), Some(b)) = (c(0), c(2), c(4)) else { return Some(None) };
            [r, g, b, c(6).unwrap_or(1.0)]
        }
        _ => return Some(None),
    };
    if rgba[3] != 1.0 || !rgba.iter().all(|v| (0.0..=1.0).contains(v)) {
        return None;
    }
    Some(Some(desc("RGBC").with("Rd  ", D::Double(rgba[0] * 255.0)).with("Grn ", D::Double(rgba[1] * 255.0)).with("Bl  ", D::Double(rgba[2] * 255.0))))
}

/// Preserve the command's effective integer width instead of rounding its input again.
fn stroke_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) {
        return None;
    }
    let mut params = if p.is_null() { Map::new() } else { p.as_object()?.clone() };
    params.insert("width".into(), Value::from(crate::extra_cmds::stroke_width(p)));
    fields_to(&Value::Object(params), STROKE, desc("null"))
}

const STROKE: &[F] = &[
    F("Wdth", "width", Kind::Long),
    F("Lctn", "location", Kind::Enum("StrL", STROKE_LOCATION)),
    F("Opct", "opacity", Kind::Pct),
    F("Md  ", "mode", Kind::Blend),
    F("Clr ", "color", Kind::Color),
    F("PrsT", "preserveTransparency", Kind::Bool),
];

/// Whether an event's `null` reference targets a layer (Edit › Transform) rather than the
/// document (Image › Image Rotation).
fn targets_layer(d: &Descriptor) -> bool {
    get_ref(d, "null").and_then(ref_class).as_deref() == Some("Lyr")
}

fn rotate_from(d: &Descriptor) -> Mapped {
    let Some(angle) = get_num(d, "Angl") else { return Mapped::Unsupported { harmless: false } };
    let a = angle.rem_euclid(360.0);
    let (layer, doc) = if (a - 90.0).abs() < 1e-6 {
        ("edit.transform.rotate90Cw", "image.imageRotation.90cw")
    } else if (a - 270.0).abs() < 1e-6 {
        ("edit.transform.rotate90Ccw", "image.imageRotation.90ccw")
    } else if (a - 180.0).abs() < 1e-6 {
        ("edit.transform.rotate180", "image.imageRotation.180")
    } else if targets_layer(d) {
        return Mapped::Unsupported { harmless: false };
    } else {
        let mut p = json!({"angle": angle.abs(), "direction": if angle < 0.0 { "ccw" } else { "cw" }});
        if let Some(i) = get_enum(d, "Intr").and_then(|i| lookup(INTERPOLATION, &i)).filter(|i| matches!(*i, "bicubic" | "bilinear" | "nearest")) {
            p["interpolation"] = i.into();
        }
        return step("image.rotation.arbitrary", p);
    };
    none(if targets_layer(d) { layer } else { doc })
}

fn rotate_to(id: &str, p: &Value) -> Option<Descriptor> {
    let (angle, layer) = match id {
        "edit.transform.rotate90Cw" => (90.0, true),
        "edit.transform.rotate90Ccw" => (-90.0, true),
        "edit.transform.rotate180" => (180.0, true),
        "image.imageRotation.90cw" => (90.0, false),
        "image.imageRotation.90ccw" => (-90.0, false),
        "image.imageRotation.180" => (180.0, false),
        "image.rotation.arbitrary" => (jnum(p, "angle")? * if jstr(p, "direction") == Some("ccw") { -1.0 } else { 1.0 }, false),
        _ => return None,
    };
    let mut d = desc("null");
    if layer {
        d = d.with("null", target_ref("Lyr "));
    }
    d = d.with("Angl", ang(angle));
    if id == "image.rotation.arbitrary"
        && let Some(i) = p.get("interpolation")
    {
        // The command resamples with nearest, bilinear or bicubic; any other value can't be
        // carried faithfully, so refuse rather than replay with a different interpolation.
        let i = match i.as_str()? {
            "nearestNeighbor" => "nearest",
            i @ ("bicubic" | "bilinear" | "nearest") => i,
            _ => return None,
        };
        d = d.with("Intr", enm("Intp", lookup_back(INTERPOLATION, i)?));
    }
    Some(d)
}

fn flip_from(d: &Descriptor) -> Mapped {
    let Some(axis) = get_enum(d, "Axis") else { return Mapped::Unsupported { harmless: false } };
    let horizontal = axis == "Hrzn";
    none(match (targets_layer(d), horizontal) {
        (true, true) => "edit.transform.flipHorizontal",
        (true, false) => "edit.transform.flipVertical",
        (false, true) => "image.imageRotation.flipCanvasHorizontal",
        (false, false) => "image.imageRotation.flipCanvasVertical",
    })
}

fn flip_to(id: &str, _: &Value) -> Option<Descriptor> {
    let (layer, axis) = match id {
        "edit.transform.flipHorizontal" => (true, "Hrzn"),
        "edit.transform.flipVertical" => (true, "Vrtc"),
        "image.imageRotation.flipCanvasHorizontal" => (false, "Hrzn"),
        "image.imageRotation.flipCanvasVertical" => (false, "Vrtc"),
        _ => return None,
    };
    let mut d = desc("null");
    if layer {
        d = d.with("null", target_ref("Lyr "));
    }
    Some(d.with("Axis", enm("Ornt", axis)))
}

fn convert_mode_from(d: &Descriptor) -> Mapped {
    if let Some(depth) = get_num(d, "Dpth") {
        return match depth as i64 {
            8 => none("image.mode.bits8"),
            16 => none("image.mode.bits16"),
            32 => none("image.mode.bits32"),
            _ => Mapped::Unsupported { harmless: false },
        };
    }
    // The mode as an enumeration (`ClrS.Grys`), a class (`Grys`), or an object of the mode's
    // class holding its options (`BtmM {Rslt, Mthd}`, `Grys {Rt}` from Bitmap).
    let (mode, options) = match d.get("T   ") {
        Some(D::Enumerated { value, .. }) => (trimmed(value), None),
        Some(D::Class(c)) => (trimmed(&c.class_id), None),
        Some(D::Descriptor(o)) => (trimmed(&o.class_id), Some(o)),
        _ => return Mapped::Unsupported { harmless: false },
    };
    if mode == "BtmM" {
        return bitmap_from(options);
    }
    // Bitmap to Grayscale records a size ratio; only 1 keeps the pixels as they are.
    if options.and_then(|o| get_num(o, "Rt  ")).is_some_and(|r| (r - 1.0).abs() > 1e-9) {
        return Mapped::Unsupported { harmless: false };
    }
    match lookup(MODES, &mode) {
        Some(m) => none(&format!("image.mode.{m}")),
        None => Mapped::Unsupported { harmless: false },
    }
}

const BITMAP_METHOD: &[(&str, &str)] = &[("Thrh", "threshold"), ("PtnD", "pattern"), ("DfnD", "diffusion")];

/// Image › Mode › Bitmap. Its Output resolution resamples the image, keeping the print size,
/// before the conversion; at the document's own resolution that is a no-op. Halftone screens
/// and custom patterns stay unsupported.
fn bitmap_from(options: Option<&Descriptor>) -> Mapped {
    let Some(o) = options else { return none("image.mode.bitmap") };
    let method = match get_enum(o, "Mthd") {
        None => "diffusion",
        Some(m) => match lookup(BITMAP_METHOD, &m) {
            Some(m) => m,
            None => return Mapped::Unsupported { harmless: false },
        },
    };
    let convert = ("image.mode.bitmap", json!({"method": method}));
    match get_num(o, "Rslt") {
        Some(r) => steps(vec![("image.imageSize", json!({"resolution": r, "resample": "bicubic", "keepPrintSize": true})), convert]),
        None => steps(vec![convert]),
    }
}

fn bitmap_to(p: &Value) -> Option<Descriptor> {
    let method = lookup_back(BITMAP_METHOD, jstr(p, "method").unwrap_or("diffusion"))?;
    Some(desc("null").with("T   ", D::Descriptor(desc("BtmM").with("Mthd", enm("Mthd", method)))))
}

fn convert_mode_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "image.mode.bits8" => desc("null").with("Dpth", long(8.0)),
        "image.mode.bits16" => desc("null").with("Dpth", long(16.0)),
        "image.mode.bits32" => desc("null").with("Dpth", long(32.0)),
        "image.mode.bitmap" => bitmap_to(p)?,
        _ => desc("null").with("T   ", enm("ClrS", lookup_back(MODES, id.strip_prefix("image.mode.")?)?)),
    })
}

fn canvas_size_from(d: &Descriptor) -> Mapped {
    let mut p = Map::new();
    for (k, param) in [("Wdth", "width"), ("Hght", "height")] {
        match d.get(k) {
            Some(D::UnitFloat { unit: PX, value }) => {
                p.insert(param.into(), value.round().into());
            }
            Some(_) => return Mapped::Unsupported { harmless: false },
            None => {}
        }
    }
    if let Some(r) = get_bool(d, "Rltv") {
        p.insert("relative".into(), r.into());
    }
    let v = get_enum(d, "Vrtc").unwrap_or_else(|| "Cntr".into());
    let h = get_enum(d, "Hrzn").unwrap_or_else(|| "Cntr".into());
    let anchor = match (v.as_str(), h.as_str()) {
        ("Top", "Left") => "topLeft",
        ("Top", "Cntr") => "top",
        ("Top", "Rght") => "topRight",
        ("Cntr", "Left") => "left",
        ("Cntr", "Cntr") => "center",
        ("Cntr", "Rght") => "right",
        ("Bttm", "Left") => "bottomLeft",
        ("Bttm", "Cntr") => "bottom",
        ("Bttm", "Rght") => "bottomRight",
        _ => return Mapped::Unsupported { harmless: false },
    };
    p.insert("anchor".into(), anchor.into());
    if let Some(c) = get_enum(d, "canvasExtensionColorType") {
        match lookup(EXTENSION, &c) {
            Some(c) => {
                p.insert("extensionColor".into(), c.into());
            }
            None => match get_obj(d, "canvasExtensionColor").and_then(color_hex) {
                Some(hex) => {
                    p.insert("extensionColor".into(), hex.into());
                }
                None => return Mapped::Unsupported { harmless: false },
            },
        }
    }
    step("image.canvasSize", Value::Object(p))
}

fn canvas_size_to(_: &str, p: &Value) -> Option<Descriptor> {
    let anchor = jstr(p, "anchor").unwrap_or("center");
    let v = if anchor.starts_with("top") {
        "Top "
    } else if anchor.starts_with("bottom") {
        "Bttm"
    } else {
        "Cntr"
    };
    let h = if anchor.to_ascii_lowercase().ends_with("left") {
        "Left"
    } else if anchor.to_ascii_lowercase().ends_with("right") {
        "Rght"
    } else {
        "Cntr"
    };
    let mut d = desc("null");
    if let Some(r) = jbool(p, "relative") {
        d = d.with("Rltv", D::Boolean(r));
    }
    if let Some(w) = jnum(p, "width") {
        d = d.with("Wdth", px(w));
    }
    if let Some(hh) = jnum(p, "height") {
        d = d.with("Hght", px(hh));
    }
    d = d.with("Hrzn", enm("HrzL", h)).with("Vrtc", enm("VrtL", v));
    if let Some(c) = jstr(p, "extensionColor") {
        d = match lookup_back(EXTENSION, c) {
            Some(e) => d.with("canvasExtensionColorType", enm("canvasExtensionColorType", e)),
            None => d.with("canvasExtensionColorType", enm("canvasExtensionColorType", "Othr")).with("canvasExtensionColor", D::Descriptor(color_desc(c)?)),
        };
    }
    Some(d)
}

fn image_size_from(d: &Descriptor) -> Mapped {
    let mut p = Map::new();
    let mut percent: Option<f64> = None;
    for (k, param) in [("Wdth", "width"), ("Hght", "height")] {
        match d.get(k) {
            Some(D::UnitFloat { unit: PX, value }) => {
                p.insert(param.into(), value.round().into());
            }
            // Percent scales both sides at playback; two different percentages, or physical
            // units, need the document's size and resolution and stay unsupported.
            Some(D::UnitFloat { unit: PCT, value }) => match percent {
                Some(other) if (other - value).abs() > 1e-9 => return Mapped::Unsupported { harmless: false },
                _ => percent = Some(*value),
            },
            Some(_) => return Mapped::Unsupported { harmless: false },
            None => {}
        }
    }
    if let Some(pc) = percent {
        p.insert("percent".into(), pc.into());
    }
    if let Some(r) = get_num(d, "Rslt") {
        p.insert("resolution".into(), r.into());
    }
    if let Some(i) = get_enum(d, "Intr") {
        match lookup(INTERPOLATION, &i) {
            Some(i) => {
                p.insert("resample".into(), i.into());
                // Resample on with only the resolution changed: Photoshop keeps the print size.
                if p.contains_key("resolution") && !p.contains_key("width") && !p.contains_key("height") && !p.contains_key("percent") {
                    p.insert("keepPrintSize".into(), true.into());
                }
            }
            None => return Mapped::Unsupported { harmless: false },
        }
    } else if p.contains_key("resolution") && !p.contains_key("width") && !p.contains_key("height") && !p.contains_key("percent") {
        p.insert("resample".into(), "none".into());
    }
    if p.is_empty() {
        return Mapped::Unsupported { harmless: false };
    }
    step("image.imageSize", Value::Object(p))
}

/// Shadows/Highlights (`adaptCorrect`, #5830): the shadow and highlight tones are `shadowMode`
/// and `highlightMode` objects (amount and width in %, radius in px), then the black and white
/// clips (%), midtone contrast (`center`) and colour correction. Each key is read as its char ID
/// or its string ID, whichever the file used; absent keys keep the command's defaults.
const SHADOWS_HIGHLIGHTS_TONES: [(&str, &str, &str); 2] = [("sdwM", "shadowMode", "shadow"), ("hglM", "highlightMode", "highlight")];
const SHADOWS_HIGHLIGHTS_TONE: [(&str, &str, &str); 3] = [("Amnt", "amount", "Amount"), ("Wdth", "width", "Tone"), ("Rds ", "radius", "Radius")];
const SHADOWS_HIGHLIGHTS: [(&str, &str, &str); 4] =
    [("BlcC", "blackClip", "blackClip"), ("WhtC", "whiteClip", "whiteClip"), ("Cntr", "center", "midtone"), ("ClrC", "colorCorrection", "color")];

fn shadows_highlights_from(d: &Descriptor) -> Mapped {
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

fn shadows_highlights_to(_: &str, p: &Value) -> Option<Descriptor> {
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

fn image_size_to(_: &str, p: &Value) -> Option<Descriptor> {
    // Resolution only with Resample on imports as keep-print-size; a command without it cannot be told apart.
    if jnum(p, "resolution").is_some()
        && jnum(p, "width").is_none()
        && jnum(p, "height").is_none()
        && jnum(p, "percent").is_none()
        && jstr(p, "resample").is_some_and(|r| r != "none")
        && jbool(p, "keepPrintSize") != Some(true)
    {
        return None;
    }
    let mut d = desc("null");
    // `resample: "none"` changes the resolution only and the command ignores any size then. The
    // descriptor has no "none" (it is the absence of `Intr`), so writing the ignored size would
    // turn the step into a pixel resize on playback (#7189).
    let resolution_only = jstr(p, "resample") == Some("none");
    if resolution_only && jnum(p, "resolution").is_none() {
        // Nothing left to record: the step changes nothing.
        return None;
    }
    if let Some(pc) = jnum(p, "percent").filter(|_| !resolution_only) {
        d = d.with("Wdth", pct(pc)).with("Hght", pct(pc)).with("CnsP", D::Boolean(true));
    }
    if let Some(w) = jnum(p, "width").filter(|_| !resolution_only) {
        d = d.with("Wdth", px(w));
    }
    if let Some(h) = jnum(p, "height").filter(|_| !resolution_only) {
        d = d.with("Hght", px(h));
    }
    if let Some(r) = jnum(p, "resolution") {
        d = d.with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: r });
    }
    if let Some(i) = jstr(p, "resample").filter(|r| *r != "none") {
        d = d.with("Intr", enm("Intp", lookup_back(INTERPOLATION, i)?));
    }
    Some(d)
}

fn crop_from(d: &Descriptor) -> Mapped {
    let mut p = Map::new();
    if let Some(t) = get_obj(d, "To  ") {
        // Pixels only, like Canvas Size: a side in percent or points needs the document's size
        // or resolution and must not be played as pixels.
        let side = |k: &str| match t.get(k) {
            Some(D::UnitFloat { unit: PX, value }) if value.is_finite() => Some(*value),
            _ => None,
        };
        let (Some(top), Some(left), Some(bottom), Some(right)) = (side("Top "), side("Left"), side("Btom"), side("Rght")) else {
            return Mapped::Unsupported { harmless: false };
        };
        p.insert("x".into(), left.round().into());
        p.insert("y".into(), top.round().into());
        p.insert("width".into(), (right - left).round().into());
        p.insert("height".into(), (bottom - top).round().into());
    }
    if let Some(a) = get_num(d, "Angl") {
        p.insert("angle".into(), a.into());
    }
    if let Some(del) = get_bool(d, "Dlt ") {
        p.insert("deleteCroppedPixels".into(), del.into());
    }
    step("image.crop", Value::Object(p))
}

fn crop_to(_: &str, p: &Value) -> Option<Descriptor> {
    // The W x H x Resolution output options have no field here: refuse rather than export a plain
    // rectangle crop that keeps the old size and resolution on replay.
    if ["targetWidth", "targetHeight", "resolution"].iter().any(|k| p.get(k).is_some_and(|v| !v.is_null())) {
        return None;
    }
    let mut d = desc("null");
    if let (Some(x), Some(y), Some(w), Some(h)) = (jnum(p, "x"), jnum(p, "y"), jnum(p, "width"), jnum(p, "height")) {
        d = d.with("To  ", D::Descriptor(desc("Rctn").with("Top ", px(y)).with("Left", px(x)).with("Btom", px(y + h)).with("Rght", px(x + w))));
    }
    if let Some(a) = jnum(p, "angle") {
        d = d.with("Angl", ang(a));
    }
    if let Some(del) = jbool(p, "deleteCroppedPixels") {
        d = d.with("Dlt ", D::Boolean(del));
    }
    Some(d)
}

fn stop_from(d: &Descriptor) -> Mapped {
    step(STOP_MESSAGE, json!({"message": get_text(d, "Msge").unwrap_or_default(), "allowContinue": get_bool(d, "Cntn").unwrap_or(false)}))
}

fn stop_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("Msge", text(jstr(p, "message").unwrap_or(""))).with("Cntn", D::Boolean(jbool(p, "allowContinue").unwrap_or(false))))
}

fn paste_from(d: &Descriptor) -> Mapped {
    none(if get_bool(d, "inPlace") == Some(true) { "edit.pasteSpecial.pasteInPlace" } else { "edit.paste" })
}

fn paste_to(id: &str, _: &Value) -> Option<Descriptor> {
    let mut d = desc("null");
    if id == "edit.pasteSpecial.pasteInPlace" {
        d = d.with("inPlace", D::Boolean(true));
    }
    Some(d.with("AntA", enm("Annt", "Anno")))
}

fn merge_visible_from(d: &Descriptor) -> Mapped {
    none(if get_bool(d, "Dplc") == Some(true) { "layer.stampVisible" } else { "layer.mergeVisible" })
}

fn merge_visible_to(id: &str, _: &Value) -> Option<Descriptor> {
    Some(if id == "layer.stampVisible" { desc("null").with("Dplc", D::Boolean(true)) } else { desc("null") })
}

fn align_from(d: &Descriptor) -> Mapped {
    match get_enum(d, "Usng").and_then(|u| lookup(ALIGN, &u)) {
        Some(a) => step(&format!("layer.align.{a}"), if get_bool(d, "alignToCanvas") == Some(true) { json!({"to": "canvas"}) } else { json!({}) }),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn align_to(id: &str, p: &Value) -> Option<Descriptor> {
    let a = lookup_back(ALIGN, id.strip_prefix("layer.align.")?)?;
    Some(desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", a)).with("alignToCanvas", D::Boolean(jstr(p, "to") == Some("canvas"))))
}

fn purge_from(d: &Descriptor) -> Mapped {
    match get_enum(d, "null").and_then(|u| lookup(PURGE, &u)) {
        Some(p) => none(&format!("edit.purge.{p}")),
        None => Mapped::Unsupported { harmless: true },
    }
}

fn purge_to(id: &str, _: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", enm("PrgI", lookup_back(PURGE, id.strip_prefix("edit.purge.")?)?)))
}

fn reset_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Property(p)) if p == "Clrs" => none("tools.defaultColors"),
        _ => Mapped::Unsupported { harmless: true },
    }
}

fn reset_to(_: &str, _: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: Id::new("Clrs") }])))
}

fn profile_from(d: &Descriptor, id: &str) -> Mapped {
    let name = get_text(d, "T   ").or_else(|| get_text(d, "profile"));
    let Some(profile) = name.as_deref().and_then(|n| lookup(PROFILES, n)) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"profile": profile});
    if id == "edit.convertToProfile" {
        if let Some(i) = get_enum(d, "Inte").and_then(|i| lookup(INTENT, &i)) {
            p["intent"] = json!(i);
        }
        if let Some(b) = get_bool(d, "MpBl") {
            p["bpc"] = json!(b);
        }
    }
    step(id, p)
}

fn profile_to(id: &str, p: &Value) -> Option<Descriptor> {
    let name = lookup_back(PROFILES, jstr(p, "profile")?)?;
    let mut d = desc("null").with("null", target_ref("Dcmn"));
    if id == "edit.convertToProfile" {
        d = d
            .with("T   ", text(name))
            .with("Inte", enm("Inte", lookup_back(INTENT, jstr(p, "intent").unwrap_or("relative"))?))
            .with("MpBl", D::Boolean(jbool(p, "bpc").unwrap_or(true)));
    } else {
        d = d.with("profile", text(name));
    }
    Some(d)
}

fn levels_event_from(d: &Descriptor) -> Mapped {
    if get_bool(d, "Auto") == Some(true) {
        return none("image.autoTone");
    }
    if get_bool(d, "AuCo") == Some(true) {
        return none("image.autoContrast");
    }
    match levels_from(d) {
        Some(p) => step("image.adjustments.levels", p),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn levels_event_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "image.autoTone" => desc("null").with("Auto", D::Boolean(true)),
        "image.autoContrast" => desc("null").with("AuCo", D::Boolean(true)),
        _ => levels_to(p)?,
    })
}

/// A direct adjustment event (Image › Adjustments › …). The command id is `image.adjustments.<name>`
/// unless it is given.
macro_rules! adjustment_event {
    ($code:literal, $class:literal, $name:literal, $label:literal) => {
        adjustment_event!($code, $class, $name, $label, concat!("image.adjustments.", $name))
    };
    ($code:literal, $class:literal, $name:literal, $label:literal, $id:expr) => {
        Event {
            code: $code,
            name: $name,
            label: $label,
            ids: &[$id],
            from: |d| match adjustment_by_class($class) {
                Some(a) => adjustment_steps(a, "image.adjustments.", Some(d)),
                None => Mapped::Unsupported { harmless: false },
            },
            to: |_, p| adjustment_by_class($class).and_then(|a| (a.to)(p)),
        }
    };
}

const RIPPLE_SIZE: &[(&str, &str)] = &[("Sml", "small"), ("Mdm", "medium"), ("Lrg", "large")];

/// Whether a Select and Mask step carries Refine Edge Brush strokes or the workspace's tool
/// edits (Quick Selection, Brush, Lasso…), which no descriptor holds.
fn has_refine_strokes(p: &Value) -> bool {
    p.get("refineStrokes").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty())) || crate::select_mask_edits::has_edits(p)
}

/// Select and Mask to Photoshop. The descriptor has no field for the Refine Edge Brush or the
/// workspace's tools, so a step with `refineStrokes` or `edits` can't be exported faithfully. One with `sampleAllLayers` is not
/// exported here either: it exports as `smartBrushWorkspace`.
fn refine_edge_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) || p.get("sampleAllLayers").is_some() || has_refine_strokes(p) {
        return None;
    }
    fields_to(p, REFINE_EDGE, desc("null"))
}

/// Select and Mask (`refineSelectionEdge`). Smart Radius, decontamination and the output follow
/// Photoshop's; the edge itself is PhotoCraft's own matting, so results differ in detail.
const REFINE_EDGE: &[F] = &[
    F("refineEdgeBorderRadius", "radius", Kind::Px),
    F("refineEdgeAutoRadius", "smartRadius", Kind::Bool),
    F("refineEdgeSmooth", "smooth", Kind::Long),
    F("refineEdgeFeatherRadius", "feather", Kind::Px),
    F("refineEdgeBorderContrast", "contrast", Kind::Pct),
    F("refineEdgeChoke", "shiftEdge", Kind::Pct),
    F("refineEdgeDecontaminate", "decontaminate", Kind::Bool),
    F("refineEdgeDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
];
const REFINE_OUTPUT: &[(&str, &str)] = &[
    ("refineEdgeOutputSelection", "selection"),
    ("refineEdgeOutputUserMask", "layerMask"),
    ("refineEdgeOutputNewLayer", "newLayer"),
    ("refineEdgeOutputNewLayerWithMask", "newLayerWithMask"),
    ("refineEdgeOutputNewDocument", "newDocument"),
    ("refineEdgeOutputNewDocumentWithMask", "newDocumentWithMask"),
];

/// Filter Gallery effects (`GEfc` with the effect in `GEfk`): Photoshop's effect code, the
/// command, and its fields. A stack of several effects stays unsupported.
const GALLERY: &[(&str, &str, &[F])] = &[("Spt", "filter.gallery.spatter", &[F("SprR", "sprayRadius", Kind::Long), F("Smth", "smoothness", Kind::Long)])];
const GALLERY_IDS: &[&str] = &["filter.gallery.spatter"];

fn gallery_from(d: &Descriptor) -> Mapped {
    let Some(effect) = get_enum(d, "GEfk") else { return Mapped::Unsupported { harmless: false } };
    match GALLERY.iter().find(|(code, _, _)| *code == effect).and_then(|(_, id, fields)| Some((*id, fields_from(d, fields)?))) {
        Some((id, p)) => step(id, Value::Object(p)),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn gallery_to(id: &str, p: &Value) -> Option<Descriptor> {
    let (code, _, fields) = GALLERY.iter().find(|(_, cmd, _)| *cmd == id)?;
    fields_to(p, fields, desc("null").with("GEfk", enm("GEft", code)))
}

/// Select and Mask as Photoshop CC 2015.5 and later record it (`smartBrushWorkspace`); the
/// older Refine Edge event is [`REFINE_EDGE`], which is also what Select and Mask exports as.
const SMART_BRUSH: &[F] = &[
    F("smartBrushRadius", "radius", Kind::Px),
    F("smartBrushUseSmartRadius", "smartRadius", Kind::Bool),
    F("smartBrushSmooth", "smooth", Kind::Long),
    F("smartBrushFeather", "feather", Kind::Px),
    F("smartBrushContrast", "contrast", Kind::Pct),
    F("smartBrushShiftEdge", "shiftEdge", Kind::Pct),
    F("smartBrushDecontaminate", "decontaminate", Kind::Bool),
    F("smartBrushDeconAmount", "amount", Kind::Pct),
    F("refineEdgeOutput", "output", Kind::Enum("refineEdgeOutput", REFINE_OUTPUT)),
    F("sampleAllLayers", "sampleAllLayers", Kind::Bool),
];

/// Select › Color Range's `Clrs` presets ↔ `select.colorRange`'s `select`.
const COLOR_RANGE: &[(&str, &str)] = &[
    ("Rds", "reds"),
    ("Ylws", "yellows"),
    ("Grns", "greens"),
    ("Cyns", "cyans"),
    ("Bls", "blues"),
    ("Mgnt", "magentas"),
    ("Hghl", "highlights"),
    ("Mdtn", "midtones"),
    ("Shdw", "shadows"),
    ("OtOf", "outOfGamut"),
    ("skinTone", "skinTones"),
];

/// The Color Range keys the converter reads (or that don't change a preset's result:
/// `colorModel` is the model of sampled colours).
const COLOR_RANGE_KEYS: &[&str] = &[
    "Clrs",
    "Fzns",
    "Invr",
    "colorModel",
    "shadowsFuzziness",
    "shadowsUpperLimit",
    "midtonesFuzziness",
    "midtonesLowerLimit",
    "midtonesUpperLimit",
    "highlightsFuzziness",
    "highlightsLowerLimit",
];

/// Color Range (`ClrR`) with a preset. Sampled Colors records Lab minimum and maximum colours,
/// which `select.colorRange` (eyedropper points on the image) can't reproduce, and an unknown
/// key (Detect Faces, Localized Color Clusters…) could change the result: both stay
/// unsupported.
fn color_range_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items.iter().any(|(k, _)| !COLOR_RANGE_KEYS.contains(&trimmed(k).as_str())) {
        return unsupported;
    }
    let Some(preset) = get_enum(d, "Clrs").and_then(|c| lookup(COLOR_RANGE, &c)) else { return unsupported };
    let mut p = Map::new();
    p.insert("select".into(), preset.into());
    let key = |suffix: &str| get_num(d, &format!("{preset}{suffix}"));
    match preset {
        "skinTones" => {
            if let Some(f) = get_num(d, "Fzns") {
                p.insert("fuzziness".into(), f.into());
            }
        }
        "shadows" | "midtones" | "highlights" => {
            if let Some(f) = key("Fuzziness") {
                p.insert("fuzziness".into(), f.into());
            }
            let range = match (preset, key("LowerLimit"), key("UpperLimit")) {
                ("shadows", _, hi) => hi.map(Value::from),
                ("highlights", lo, _) => lo.map(Value::from),
                (_, Some(lo), Some(hi)) => Some(json!([lo, hi])),
                (_, None, None) => None,
                _ => return unsupported,
            };
            if let Some(r) = range {
                p.insert("tonalRange".into(), r);
            }
        }
        _ => {}
    }
    if get_bool(d, "Invr") == Some(true) {
        p.insert("invert".into(), true.into());
    }
    step("select.colorRange", Value::Object(p))
}

fn color_range_to(_: &str, p: &Value) -> Option<Descriptor> {
    let preset = jstr(p, "select")?;
    if p.as_object()?.keys().any(|k| !matches!(k.as_str(), "select" | "fuzziness" | "tonalRange" | "invert")) {
        return None;
    }
    let mut d = desc("null").with("Clrs", enm("Clrs", lookup_back(COLOR_RANGE, preset)?));
    let tonal = matches!(preset, "shadows" | "midtones" | "highlights");
    if let Some(f) = jnum(p, "fuzziness") {
        match preset {
            "skinTones" => d = d.with("Fzns", long(f)),
            _ if tonal => d = d.with(&format!("{preset}Fuzziness"), long(f)),
            _ => {}
        }
    }
    if let Some(r) = p.get("tonalRange") {
        match preset {
            "shadows" => d = d.with("shadowsUpperLimit", long(r.as_f64()?)),
            "highlights" => d = d.with("highlightsLowerLimit", long(r.as_f64()?)),
            "midtones" => {
                let a = r.as_array()?;
                d = d.with("midtonesLowerLimit", long(a.first()?.as_f64()?)).with("midtonesUpperLimit", long(a.get(1)?.as_f64()?));
            }
            _ => return None,
        }
    }
    if jbool(p, "invert") == Some(true) {
        d = d.with("Invr", D::Boolean(true));
    }
    Some(d)
}

/// Create / Release Clipping Mask (`groupEvent` / `ungroup`) on the target or a named layer.
fn clipping_from(d: &Descriptor, id: &str) -> Mapped {
    match get_ref(d, "null") {
        None => none(id),
        Some(r) if ref_class(r).as_deref() == Some("Lyr") => match layer_target(r) {
            Some(p) => step(id, Value::Object(p)),
            None => Mapped::Unsupported { harmless: false },
        },
        Some(_) => Mapped::Unsupported { harmless: false },
    }
}

fn clipping_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", layer_ref(p)?))
}

/// A clipping mask event: `clipping_from` / `clipping_to` with one command id.
macro_rules! clipping_event {
    ($code:literal, $name:literal, $label:literal, $id:literal) => {
        Event { code: $code, name: $name, label: $label, ids: &[$id], from: |d| clipping_from(d, $id), to: clipping_to }
    };
}

const RADIAL_BLUR_FIELDS: &[F] =
    &[F("Amnt", "amount", Kind::Long), F("BlrM", "method", Kind::Enum("BlrM", RADIAL_METHOD)), F("BlrQ", "quality", Kind::Enum("BlrQ", RADIAL_QUALITY))];

fn radial_blur_from(d: &Descriptor) -> Mapped {
    match fields_from(d, RADIAL_BLUR_FIELDS) {
        Some(m) => step("filter.blur.radialBlur", Value::Object(m)),
        None => Mapped::Unsupported { harmless: false },
    }
}

fn radial_blur_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) {
        return None;
    }
    // Photoshop's Radial Blur has no center parameter and always blurs from the center (0.5, 0.5).
    // An explicit center must match 0.5 exactly; any non-default center cannot be faithfully exported.
    let is_default_center = |k: &str| match p.get(k) {
        None | Some(Value::Null) => true,
        Some(v) => v.as_f64() == Some(0.5),
    };
    if !is_default_center("centerX") || !is_default_center("centerY") {
        return None;
    }
    fields_to(p, RADIAL_BLUR_FIELDS, desc("null"))
}

/// Every Photoshop event PhotoCraft maps. The order matters only for the panel label.
const EVENTS: &[Event] = &[
    // Filters.
    ev!("GsnB", "gaussianBlur", "Gaussian Blur", "filter.blur.gaussianBlur", &[F("Rds ", "radius", Kind::Px)]),
    ev!(
        "UnsM",
        "unsharpMask",
        "Unsharp Mask",
        "filter.sharpen.unsharpMask",
        &[F("Amnt", "amount", Kind::Pct), F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]
    ),
    ev!(
        "AdNs",
        "addNoise",
        "Add Noise",
        "filter.noise.addNoise",
        &[
            F("Dstr", "distribution", Kind::Enum("Dstr", NOISE)),
            F("Nose", "amount", Kind::Pct),
            F("Mnch", "monochromatic", Kind::Bool),
            F("FlRs", "seed", Kind::Long)
        ]
    ),
    ev!("Mdn ", "median", "Median", "filter.noise.median", &[F("Rds ", "radius", Kind::Px)]),
    ev!("DstS", "dustAndScratches", "Dust & Scratches", "filter.noise.dustAndScratches", &[F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]),
    ev!("Dspc", "despeckle", "Despeckle", "filter.noise.despeckle", &[]),
    // Photoshop records Emboss's angle, height and amount as plain integers (#7946).
    ev!(
        "Embs",
        "emboss",
        "Emboss",
        "filter.stylize.emboss",
        &[F("Angl", "angle", Kind::Long), F("Hght", "height", Kind::Long), F("Amnt", "amount", Kind::Long)]
    ),
    ev!("MtnB", "motionBlur", "Motion Blur", "filter.blur.motionBlur", &[F("Angl", "angle", Kind::Ang), F("Dstn", "distance", Kind::Px)]),
    ev!("", "boxblur", "Box Blur", "filter.blur.boxBlur", &[F("Rds ", "radius", Kind::Px)]),
    ev!("", "surfaceBlur", "Surface Blur", "filter.blur.surfaceBlur", &[F("Rds ", "radius", Kind::Px), F("Thsh", "threshold", Kind::Long)]),
    ev!(
        "SmrB",
        "smartBlur",
        "Smart Blur",
        "filter.blur.smartBlur",
        &[
            F("Rds ", "radius", Kind::Dbl),
            F("Thsh", "threshold", Kind::Dbl),
            F("SmBQ", "quality", Kind::Enum("SmBQ", SMART_BLUR_QUALITY)),
            F("SmBM", "mode", Kind::Enum("SmBM", SMART_BLUR_MODE))
        ]
    ),
    Event { code: "RdlB", name: "radialBlur", label: "Radial Blur", ids: &["filter.blur.radialBlur"], from: radial_blur_from, to: radial_blur_to },
    ev!("Blr ", "blur", "Blur", "filter.blur.blur", &[]),
    ev!("BlrM", "blurMore", "Blur More", "filter.blur.blurMore", &[]),
    ev!("Shrp", "sharpen", "Sharpen", "filter.sharpen.sharpen", &[]),
    ev!("ShrM", "sharpenMore", "Sharpen More", "filter.sharpen.sharpenMore", &[]),
    ev!("ShrE", "sharpenEdges", "Sharpen Edges", "filter.sharpen.sharpenEdges", &[]),
    ev!("HghP", "highPass", "High Pass", "filter.other.highPass", &[F("Rds ", "radius", Kind::Px)]),
    ev!("Mnm ", "minimum", "Minimum", "filter.other.minimum", &[F("Rds ", "radius", Kind::Px)], unless preserves_roundness),
    ev!("Mxm ", "maximum", "Maximum", "filter.other.maximum", &[F("Rds ", "radius", Kind::Px)], unless preserves_roundness),
    ev!(
        "Ofst",
        "offset",
        "Offset",
        "filter.other.offset",
        &[F("Hrzn", "horizontal", Kind::Long), F("Vrtc", "vertical", Kind::Long), F("Fl  ", "undefinedAreas", Kind::Enum("FlMd", OFFSET_FILL))]
    ),
    ev!("Rple", "ripple", "Ripple", "filter.distort.ripple", &[F("Amnt", "amount", Kind::ExactLong), F("RplS", "size", Kind::Enum("RplS", RIPPLE_SIZE))]),
    ev!("Crst", "crystallize", "Crystallize", "filter.pixelate.crystallize", &[F("ClSz", "cellSize", Kind::ExactLong)]),
    Event { code: "GEfc", name: "filterGallery", label: "Filter Gallery", ids: GALLERY_IDS, from: gallery_from, to: gallery_to },
    // Adjustments (Image › Adjustments).
    Event {
        code: "",
        name: "adaptCorrect",
        label: "Shadows/Highlights",
        ids: &["image.adjustments.shadowsHighlights"],
        from: shadows_highlights_from,
        to: shadows_highlights_to,
    },
    Event {
        code: "Lvls",
        name: "levels",
        label: "Levels",
        ids: &["image.adjustments.levels", "image.autoTone", "image.autoContrast"],
        from: levels_event_from,
        to: levels_event_to,
    },
    adjustment_event!("Crvs", "Crvs", "curves", "Curves"),
    adjustment_event!("HStr", "HStr", "hueSaturation", "Hue/Saturation"),
    adjustment_event!("BrgC", "BrgC", "brightnessEvent", "Brightness/Contrast", "image.adjustments.brightnessContrast"),
    adjustment_event!("Thrs", "Thrs", "threshold", "Threshold"),
    adjustment_event!("Pstr", "Pstr", "posterize", "Posterize"),
    adjustment_event!("Invr", "Invr", "invert", "Invert"),
    adjustment_event!("ClrB", "ClrB", "colorBalance", "Color Balance"),
    adjustment_event!("Vbrn", "Vbrn", "vibrance", "Vibrance"),
    adjustment_event!("Exps", "Exps", "exposure", "Exposure"),
    // Photoshop runs Photo Filter by name only: `PhtF` is the class, not an event (#8301).
    adjustment_event!("", "PhtF", "photoFilter", "Photo Filter"),
    adjustment_event!("", "BlnW", "blackAndWhite", "Black & White", "image.adjustments.blackWhite"),
    adjustment_event!("ChnM", "ChnM", "channelMixer", "Channel Mixer"),
    adjustment_event!("GdMp", "GdMp", "gradientMapEvent", "Gradient Map", "image.adjustments.gradientMap"),
    adjustment_event!("SlcC", "SlcC", "selectiveColor", "Selective Color"),
    ev!("Dstt", "desaturate", "Desaturate", "image.adjustments.desaturate", &[]),
    ev!("Eqlz", "equalize", "Equalize", "image.adjustments.equalize", &[]),
    ev!("", "autoColor", "Auto Color", "image.autoColor", &[]),
    // Image.
    Event { code: "ImgS", name: "imageSize", label: "Image Size", ids: &["image.imageSize"], from: image_size_from, to: image_size_to },
    Event { code: "CnvS", name: "canvasSize", label: "Canvas Size", ids: &["image.canvasSize"], from: canvas_size_from, to: canvas_size_to },
    Event {
        code: "Rtte",
        name: "rotateEventEnum",
        label: "Rotate",
        ids: &[
            "image.imageRotation.90cw",
            "image.imageRotation.90ccw",
            "image.imageRotation.180",
            "image.rotation.arbitrary",
            "edit.transform.rotate90Cw",
            "edit.transform.rotate90Ccw",
            "edit.transform.rotate180",
        ],
        from: rotate_from,
        to: rotate_to,
    },
    Event {
        code: "Flip",
        name: "flip",
        label: "Flip",
        ids: &[
            "image.imageRotation.flipCanvasHorizontal",
            "image.imageRotation.flipCanvasVertical",
            "edit.transform.flipHorizontal",
            "edit.transform.flipVertical",
        ],
        from: flip_from,
        to: flip_to,
    },
    Event { code: "Crop", name: "crop", label: "Crop", ids: &["image.crop"], from: crop_from, to: crop_to },
    ev!(
        "",
        "trim",
        "Trim",
        "image.trim",
        &[
            F("trimBasedOn", "basedOn", Kind::Enum("trimBasedOn", TRIM)),
            F("Top ", "top", Kind::Bool),
            F("Btom", "bottom", Kind::Bool),
            F("Left", "left", Kind::Bool),
            F("Rght", "right", Kind::Bool)
        ]
    ),
    ev!("RvlA", "revealAll", "Reveal All", "image.revealAll", &[]),
    Event {
        code: "CnvM",
        name: "convertMode",
        label: "Convert Mode",
        ids: &[
            "image.mode.rgb",
            "image.mode.grayscale",
            "image.mode.cmyk",
            "image.mode.lab",
            "image.mode.multichannel",
            "image.mode.bits8",
            "image.mode.bits16",
            "image.mode.bits32",
            "image.mode.bitmap",
        ],
        from: convert_mode_from,
        to: convert_mode_to,
    },
    Event { code: "Dplc", name: "duplicate", label: "Duplicate", ids: &["layer.duplicate", "image.duplicate"], from: duplicate_from, to: duplicate_to },
    // Layers.
    Event {
        code: "Mk  ",
        name: "make",
        label: "Make",
        ids: &[
            "layer.new.layer",
            "layer.new.group",
            "layer.groupLayers",
            "layer.duplicate",
            "layer.new.layerViaCopy",
            "layer.new.layerViaCut",
            "layer.newFillLayer.solidColor",
            "layer.newFillLayer.pattern",
            "layer.newAdjustmentLayer.levels",
            "layer.newAdjustmentLayer.curves",
            "layer.newAdjustmentLayer.hueSaturation",
            "layer.newAdjustmentLayer.brightnessContrast",
            "layer.newAdjustmentLayer.threshold",
            "layer.newAdjustmentLayer.posterize",
            "layer.newAdjustmentLayer.invert",
            "layer.newAdjustmentLayer.colorBalance",
            "layer.newAdjustmentLayer.vibrance",
            "layer.newAdjustmentLayer.exposure",
            "layer.newAdjustmentLayer.photoFilter",
            "layer.newAdjustmentLayer.blackWhite",
            "layer.newAdjustmentLayer.channelMixer",
            "layer.newAdjustmentLayer.selectiveColor",
            "layer.newAdjustmentLayer.gradientMap",
            "layer.layerMask.revealAll",
            "layer.layerMask.hideAll",
            "layer.layerMask.revealSelection",
            "layer.layerMask.hideSelection",
            "file.new",
            "file.newFromClipboard",
        ],
        from: make_from,
        to: make_to,
    },
    Event {
        code: "setd",
        name: "set",
        label: "Set",
        ids: &[
            "select.all",
            "select.deselect",
            "select.reselect",
            "select.loadSelection",
            "select.rect",
            "select.lasso",
            "select.editInQuickMaskMode",
            "tools.setColors",
            "layer.setProps",
            "layer.renameLayer",
            "layer.setLabelColor",
            "layer.setAdjustment",
            "layer.fill.pattern.set",
            "layer.new.layerFromBackground",
        ],
        from: set_from,
        to: set_to,
    },
    Event {
        code: "slct",
        name: "select",
        label: "Select",
        ids: &[
            "layer.select",
            "layer.selectAbove",
            "layer.selectBelow",
            "layer.addAboveToSelection",
            "layer.addBelowToSelection",
            "layer.selectTop",
            "layer.selectBottom",
            "channel.target",
            "edit.undo",
            "edit.redo",
            "view.fitOnScreen",
            "view.actualPixels",
            "view.zoomIn",
            "view.zoomOut",
            "document.activate",
        ],
        from: select_from,
        to: select_to,
    },
    Event { code: "Dlt ", name: "delete", label: "Delete", ids: &["layer.delete", "edit.clear", "layer.layerMask.delete"], from: delete_from, to: delete_to },
    Event { code: "", name: "clearEvent", label: "Clear", ids: &["select.editInQuickMaskMode"], from: clear_from, to: clear_to },
    Event {
        code: "Trnf",
        name: "transform",
        label: "Transform",
        ids: &["edit.transform", "select.transformSelection"],
        from: transform_from,
        to: transform_to,
    },
    Event { code: "AppI", name: "applyImageEvent", label: "Apply Image", ids: &["image.applyImage"], from: apply_image_from, to: apply_image_to },
    ev!("", "newPlacedLayer", "Convert to Smart Object", "layer.smartObjects.convertToSmartObject", &[]),
    ev!("", "placedLayerEditContents", "Edit Contents", "layer.smartObjects.editContents", &[]),
    ev!("", "updatePlacedLayer", "Update Smart Objects", "layer.smartObjects.updateModifiedContent", &[]),
    Event { code: "Hd  ", name: "hide", label: "Hide", ids: &["layer.hideLayers"], from: |d| visibility_from(d, "layer.hideLayers"), to: visibility_to },
    Event { code: "Shw ", name: "show", label: "Show", ids: &["layer.showLayers"], from: |d| visibility_from(d, "layer.showLayers"), to: visibility_to },
    Event {
        code: "move",
        name: "move",
        label: "Move",
        ids: &["layer.translate", "layer.arrange.bringToFront", "layer.arrange.sendToBack", "layer.arrange.bringForward", "layer.arrange.sendBackward"],
        from: move_from,
        to: move_to,
    },
    ev!("FltI", "flattenImage", "Flatten Image", "layer.flattenImage", &[]),
    Event {
        code: "MrgV",
        name: "mergeVisible",
        label: "Merge Visible",
        ids: &["layer.mergeVisible", "layer.stampVisible"],
        from: merge_visible_from,
        to: merge_visible_to,
    },
    ev!("Mrg2", "mergeLayersNew", "Merge Layers", "layer.mergeLayers", &[]),
    clipping_event!("Ungr", "ungroup", "Release Clipping Mask", "layer.releaseClippingMask"),
    ev!("CpTL", "copyToLayer", "Layer Via Copy", "layer.new.layerViaCopy", &[]),
    ev!("CtTL", "cutToLayer", "Layer Via Cut", "layer.new.layerViaCut", &[]),
    ev!("", "rasterizeLayer", "Rasterize", "layer.rasterize.layer", &[]),
    ev!("", "selectAllLayers", "Select All Layers", "select.allLayers", &[]),
    ev!("", "selectNoLayers", "Deselect Layers", "select.deselectLayers", &[]),
    clipping_event!("GrpL", "groupEvent", "Create Clipping Mask", "layer.createClippingMask"),
    ev!("", "ungroupLayersEvent", "Ungroup Layers", "layer.ungroupLayers", &[]),
    ev!("", "copyEffects", "Copy Layer Style", "layer.layerStyle.copyLayerStyle", &[]),
    ev!("", "pasteEffects", "Paste Layer Style", "layer.layerStyle.pasteLayerStyle", &[]),
    Event {
        code: "",
        name: "collapseAllGroupsEvent",
        label: "Collapse All Groups",
        ids: &["layer.setExpanded"],
        from: |_| step("layer.setExpanded", json!({"all": true, "expanded": false})),
        to: |_, p| (jbool(p, "all") == Some(true) && jbool(p, "expanded") == Some(false)).then(|| desc("null")),
    },
    ev!("Dfrg", "defringe", "Defringe", "layer.matting.defringe", &[F("Wdth", "width", Kind::Px)]),
    Event {
        code: "Algn",
        name: "align",
        label: "Align",
        ids: &[
            "layer.align.leftEdges",
            "layer.align.horizontalCenters",
            "layer.align.rightEdges",
            "layer.align.topEdges",
            "layer.align.verticalCenters",
            "layer.align.bottomEdges",
        ],
        from: align_from,
        to: align_to,
    },
    // Selection.
    ev!("Invs", "inverse", "Inverse", "select.inverse", &[]),
    ev!("Fthr", "feather", "Feather", "select.modify.feather", &[F("Rds ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Expn", "expand", "Expand", "select.modify.expand", &[F("By  ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Cntc", "contract", "Contract", "select.modify.contract", &[F("By  ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Smth", "smoothness", "Smooth", "select.modify.smooth", &[F("Rds ", "radius", Kind::Px)], unless applies_at_canvas_bounds),
    ev!("Brdr", "border", "Border", "select.modify.border", &[F("Wdth", "radius", Kind::Px)]),
    ev!("Grow", "grow", "Grow", "select.grow", &[F("Tlrn", "tolerance", Kind::Long)]),
    ev!("Smlr", "similar", "Similar", "select.similar", &[F("Tlrn", "tolerance", Kind::Long)]),
    // `refineSelectionEdge` has no Sample All Layers, so a step with it exports as
    // `smartBrushWorkspace` instead of dropping it.
    Event {
        code: "",
        name: "refineSelectionEdge",
        label: "Select and Mask",
        ids: &["select.refineEdge"],
        from: |d| match fields_from(d, REFINE_EDGE) {
            Some(m) => step("select.refineEdge", Value::Object(m)),
            None => Mapped::Unsupported { harmless: false },
        },
        to: refine_edge_to,
    },
    ev!("", "smartBrushWorkspace", "Select and Mask", "select.refineEdge", SMART_BRUSH, unless has_refine_strokes),
    ev!("", "autoCutout", "Select Subject", "select.subject", &[F("sampleAllLayers", "sampleAllLayers", Kind::Bool)]),
    Event { code: "ClrR", name: "colorRange", label: "Color Range", ids: &["select.colorRange"], from: color_range_from, to: color_range_to },
    // Edit.
    Event { code: "Fl  ", name: "fill", label: "Fill", ids: &["edit.fill", "paint.bucket"], from: fill_from, to: fill_to },
    Event {
        code: "Strk",
        name: "stroke",
        label: "Stroke",
        ids: &["edit.stroke"],
        from: |d| match fields_from(d, STROKE) {
            Some(m) => step("edit.stroke", Value::Object(m)),
            None => Mapped::Unsupported { harmless: false },
        },
        to: stroke_to,
    },
    ev!("copy", "copyEvent", "Copy", "edit.copy", &[]),
    ev!("CpyM", "copyMerged", "Copy Merged", "edit.copyMerged", &[]),
    ev!("cut ", "cut", "Cut", "edit.cut", &[]),
    Event { code: "past", name: "paste", label: "Paste", ids: &["edit.paste", "edit.pasteSpecial.pasteInPlace"], from: paste_from, to: paste_to },
    Event {
        code: "Prge",
        name: "purge",
        label: "Purge",
        ids: &["edit.purge.all", "edit.purge.clipboard", "edit.purge.histories", "edit.purge.undo"],
        from: purge_from,
        to: purge_to,
    },
    Event { code: "Rset", name: "reset", label: "Reset", ids: &["tools.defaultColors"], from: reset_from, to: reset_to },
    ev!("Exch", "exchange", "Exchange", "tools.swapColors", &[]),
    Event {
        code: "",
        name: "assignProfile",
        label: "Assign Profile",
        ids: &["edit.assignProfile"],
        from: |d| profile_from(d, "edit.assignProfile"),
        to: profile_to,
    },
    Event {
        code: "",
        name: "convertToProfile",
        label: "Convert to Profile",
        ids: &["edit.convertToProfile"],
        from: |d| profile_from(d, "edit.convertToProfile"),
        to: profile_to,
    },
    // File and actions.
    Event { code: "Cls ", name: "close", label: "Close", ids: &["file.close"], from: close_from, to: close_to },
    Event { code: "Expr", name: "export", label: "Export", ids: &["file.export.saveForWebLegacy"], from: export_from, to: export_to },
    Event { code: "Plc ", name: "placeEvent", label: "Place", ids: &["file.placeEmbedded", "file.placeLinked"], from: place_from, to: place_to },
    Event { code: "Opn ", name: "open", label: "Open", ids: &["file.openAs"], from: open_from, to: open_to },
    Event { code: "", name: "conditional", label: "If", ids: &["actions.conditional"], from: conditional_from, to: conditional_to },
    ev!("save", "save", "Save", "file.save", &[]),
    ev!("Rvrt", "revert", "Revert", "file.revert", &[]),
    Event { code: "Ply ", name: "play", label: "Play", ids: &["actions.play"], from: play_from, to: play_to },
    Event { code: "", name: "applyComp", label: "Apply Layer Comp", ids: &["layerComp.apply"], from: apply_comp_from, to: apply_comp_to },
    Event { code: "Stop", name: "stop", label: "Stop", ids: &[STOP_MESSAGE], from: stop_from, to: stop_to },
];

/// Four-char codes PhotoCraft once exported for events Photoshop only runs by name (or by
/// another code), with the name they stand for. Measured in Photoshop: `executeAction` with
/// these codes fails ("the command is not currently available"), and `Fltn` is the
/// `flatness` property, not the flatten event. Import keeps accepting them so actions saved
/// before still load; export never writes them.
const LEGACY_CODES: &[(&str, &str)] =
    &[("BoxB", "boxblur"), ("SrfB", "surfaceBlur"), ("Fltn", "flattenImage"), ("BlnW", "blackAndWhite"), ("PhtF", "photoFilter")];

fn event_for(item: &AtnEvent) -> Option<&'static Event> {
    let id = item.id();
    let find = |id: &str| EVENTS.iter().find(|e| (!e.code.is_empty() && e.code.trim_end() == id.trim_end()) || e.name == id);
    find(&id).or_else(|| match item {
        AtnEvent::Code(_) => LEGACY_CODES.iter().find(|(code, _)| *code == id).and_then(|(_, name)| find(name)),
        AtnEvent::Name(_) => None,
    })
}

/// The event `id` exports as: the first event listing it whose converter accepts the params
/// (Quick Mask on is a `set`, off a `clearEvent`).
fn event_and_descriptor(id: &str, params: &Value) -> Option<(&'static Event, Descriptor)> {
    EVENTS.iter().filter(|e| e.ids.contains(&id)).find_map(|e| (e.to)(id, params).map(|d| (e, d)))
}

// ------------------------------------------------------------------ import

/// A step with no PhotoCraft equivalent.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Unsupported {
    pub action: String,
    pub step: usize,
    pub event: String,
    pub name: String,
}

/// What a `.atn` file imports to.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub set: String,
    pub actions: Vec<Action>,
    /// Function keys the file assigns, as `(action name, key)`.
    pub function_keys: Vec<(String, String)>,
    pub unsupported: Vec<Unsupported>,
    pub warnings: Vec<String>,
}

fn photoshop_step(item: &AtnItem) -> (String, Value) {
    let descriptor = item.descriptor.as_ref().map(|d| photocraft_paint::tile::b64_encode(&d.to_bytes()));
    (PHOTOSHOP_STEP.into(), json!({"event": item.event.id(), "name": item.label(), "descriptor": descriptor}))
}

/// Integral floats become integers (`2.0` → `2`), as the commands' own docs and recordings
/// use, so an imported step reads and compares like a recorded one.
fn normalise(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && f.abs() < 9.0e15
                && n.as_i64().is_none()
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalise),
        Value::Object(o) => o.values_mut().for_each(normalise),
        _ => {}
    }
}

fn convert_action(a: &AtnAction, set: &str, unsupported: &mut Vec<Unsupported>) -> Action {
    let name = a.name.to_string_lossy();
    let mut action = Action::new(name.clone(), Vec::new());
    action.set = Some(set.to_string());
    for (i, item) in a.items.iter().enumerate() {
        let mapped = match (event_for(&item.event), &item.descriptor) {
            (Some(e), Some(d)) => (e.from)(d),
            (Some(e), None) => (e.from)(&Descriptor::default()),
            (None, _) => Mapped::Unsupported { harmless: false },
        };
        let (new_steps, enabled) = match mapped {
            Mapped::Steps(s) => (s, item.enabled),
            Mapped::Unsupported { harmless } => {
                unsupported.push(Unsupported { action: name.clone(), step: i, event: item.event.id(), name: item.label() });
                (vec![photoshop_step(item)], item.enabled && !harmless)
            }
        };
        for mut s in new_steps {
            normalise(&mut s.1);
            let idx = action.steps.len();
            action.steps.push(s);
            if !enabled {
                action.disabled.insert(idx);
            }
            if item.with_dialog {
                action.dialogs.insert(idx);
            }
        }
    }
    action
}

/// Convert a `.atn` file. Fails only when the file is unreadable: every step imports, as a
/// command or as a kept Photoshop step.
pub fn import(bytes: &[u8]) -> std::result::Result<Imported, String> {
    let file = atn::parse(bytes).map_err(|e| e.to_string())?;
    let set = file.name.to_string_lossy();
    let set = if set.trim().is_empty() { DEFAULT_SET.to_string() } else { set };
    let mut out = Imported { set: set.clone(), warnings: file.warnings.clone(), ..Default::default() };
    for a in &file.actions {
        let action = convert_action(a, &set, &mut out.unsupported);
        if a.function_key > 0 {
            out.function_keys.push((action.name.clone(), function_key_name(a.function_key, a.shift, a.command)));
        }
        out.actions.push(action);
    }
    Ok(out)
}

// ------------------------------------------------------------------ export

/// A step Save Actions could not express as a Photoshop event.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Skipped {
    pub action: String,
    pub step: usize,
    pub id: String,
}

fn export_item(id: &str, params: &Value, set: &str) -> Option<AtnItem> {
    if id == PHOTOSHOP_STEP {
        let event = jstr(params, "event")?;
        let event = match event.as_bytes() {
            [a, b, c, d] => AtnEvent::Code([*a, *b, *c, *d]),
            _ => AtnEvent::name(event),
        };
        let descriptor = match jstr(params, "descriptor") {
            Some(b64) => Some(Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(b64)?).ok()?),
            None => None,
        };
        return Some(AtnItem { event, dictionary: jstr(params, "name").unwrap_or("").as_bytes().to_vec(), descriptor, ..Default::default() });
    }
    let mut params = params.clone();
    if (id == "actions.play" || id == "actions.conditional") && params.get("set").is_none() {
        params["set"] = json!(set);
    }
    let (e, descriptor) = event_and_descriptor(id, &params)?;
    let event = if e.code.is_empty() { AtnEvent::name(e.name) } else { AtnEvent::Code(e.code.as_bytes().try_into().ok()?) };
    Some(AtnItem { event, dictionary: e.label.as_bytes().to_vec(), descriptor: Some(descriptor), ..Default::default() })
}

/// Photoshop applies only the rename when a Set mixes it with opacity or blend mode (#8304).
/// Write each layer property separately, renaming last so named references still resolve.
fn export_items(id: &str, params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    let item = export_item(id, params, set)?;
    if id != "layer.setProps" {
        return Some(vec![item]);
    }
    let descriptor = item.descriptor.as_ref()?;
    let properties = get_obj(descriptor, "T   ")?;
    if properties.items.len() <= 1 {
        return Some(vec![item]);
    }
    let target = descriptor.get("null")?;
    let mut items = Vec::with_capacity(properties.items.len());
    for (key, value) in properties.items.iter().filter(|(k, _)| !k.is("Nm  ")).chain(properties.items.iter().filter(|(k, _)| k.is("Nm  "))) {
        let mut property = properties.clone();
        property.items = vec![(key.clone(), value.clone())];
        let descriptor = desc("null").with("null", target.clone()).with("T   ", D::Descriptor(property));
        items.push(AtnItem { descriptor: Some(descriptor), ..item.clone() });
    }
    Some(items)
}

/// An action's key as a shortcut string: `F5`, with Photoshop's Shift and Command options
/// `Shift+F5`, `Cmd+F5` or `Cmd+Shift+F5` (#7646).
pub fn function_key_name(key: u16, shift: bool, command: bool) -> String {
    format!("{}{}F{key}", if command { "Cmd+" } else { "" }, if shift { "Shift+" } else { "" })
}

/// The `.atn` function key (1…15), Shift and Command of a shortcut string, or `None` for
/// anything else.
pub fn parse_function_key(key: &str) -> Option<(u16, bool, bool)> {
    let (mut shift, mut command) = (false, false);
    let mut rest = key;
    loop {
        if let Some(r) = rest.strip_prefix("Cmd+").filter(|_| !command) {
            (command, rest) = (true, r);
        } else if let Some(r) = rest.strip_prefix("Shift+").filter(|_| !shift) {
            (shift, rest) = (true, r);
        } else {
            break;
        }
    }
    let n = rest.strip_prefix('F')?.parse::<u16>().ok().filter(|n| (1..=15).contains(n))?;
    Some((n, shift, command))
}

/// The `.atn` bytes of `actions` as the set `set`. `function_keys` gives each action's key
/// (`F1`…`F15`, with `Shift+` and `Cmd+` as [`function_key_name`] writes them). Steps with no Photoshop equivalent are an error unless `skip_unsupported`.
pub fn export(
    set: &str,
    actions: &[&Action],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    let mut skipped = Vec::new();
    let mut file = AtnFile { name: UnicodeString::new_nul(set), expanded: true, actions: Vec::new(), warnings: Vec::new() };
    for a in actions {
        let mut out = AtnAction { name: UnicodeString::new_nul(&a.name), expanded: false, ..Default::default() };
        if let Some((n, shift, command)) = function_keys.iter().find(|(n, _)| n == &a.name).and_then(|(_, key)| parse_function_key(key)) {
            (out.function_key, out.shift, out.command) = (n, shift, command);
        }
        for (i, (id, params)) in a.steps.iter().enumerate() {
            let Some(items) = export_items(id, params, set) else {
                if skip_unsupported {
                    skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                    continue;
                }
                let step_name = match params.get("layer") {
                    Some(Value::Number(n)) => format!("`{id}` with numeric layer {n}"),
                    _ => format!("`{id}`"),
                };
                return Err(format!(
                    "step {} of `{}` ({step_name}) has no Photoshop equivalent; pass \"skipUnsupported\": true to leave it out",
                    i + 1,
                    a.name
                ));
            };
            for mut item in items {
                item.enabled = a.step_enabled(i);
                item.with_dialog = a.dialogs.contains(&i);
                item.dialog_options = if item.with_dialog { 2 } else { 0 };
                out.items.push(item);
            }
        }
        file.actions.push(out);
    }
    let bytes = atn::write(&file).map_err(|e| e.to_string())?;
    Ok((bytes, skipped))
}

// ------------------------------------------------------------------ commands

/// Assigns names not already used, as `name`, `name 2`, `name 3`… A plain linear scan of the
/// taken list per candidate is cubic when an action file repeats one base name many times, so
/// membership uses a set and the next suffix to try is cached per base name.
struct UniqueNames {
    taken: std::collections::HashSet<String>,
    next_suffix: std::collections::HashMap<String, u32>,
}

impl UniqueNames {
    fn new<I: IntoIterator<Item = String>>(existing: I) -> Self {
        UniqueNames { taken: existing.into_iter().collect(), next_suffix: std::collections::HashMap::new() }
    }

    /// Reserves and returns a unique name derived from `name`, recording it as taken.
    fn claim(&mut self, name: &str) -> String {
        if self.taken.insert(name.to_string()) {
            return name.to_string();
        }
        let mut i = self.next_suffix.get(name).copied().unwrap_or(2);
        loop {
            let candidate = format!("{name} {i}");
            i += 1;
            if self.taken.insert(candidate.clone()) {
                self.next_suffix.insert(name.to_string(), i);
                return candidate;
            }
        }
    }
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.load";
    if s.actions.recording.is_some() || s.actions.playing > 0 {
        return Err(bad(cmd, "stop recording or playing before loading actions"));
    }
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, cmd, ".atn file")?;
    let mut imp = import(&bytes).map_err(|e| bad(cmd, e))?;
    match p.get("set") {
        None | Some(Value::Null) => {}
        Some(Value::String(n)) if !n.trim().is_empty() => {
            imp.set = n.trim().to_string();
            for a in &mut imp.actions {
                a.set = Some(imp.set.clone());
            }
        }
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    }
    let replace = match p.get("replace") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`replace` must be true or false")),
    };
    if replace {
        s.actions.list.retain(|a| a.set.as_deref() != Some(imp.set.as_str()));
    }
    let mut taken = UniqueNames::new(s.actions.list.iter().map(|a| a.name.clone()));
    let mut names = Vec::with_capacity(imp.actions.len());
    // The file's name of each action that had to be renamed, and its new name.
    let mut renamed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut steps = 0usize;
    let mut added = Vec::with_capacity(imp.actions.len());
    for mut a in imp.actions {
        let name = taken.claim(&a.name);
        if name != a.name {
            renamed.entry(a.name.clone()).or_insert_with(|| name.clone());
        }
        a.name = name.clone();
        steps = steps.saturating_add(a.steps.len());
        names.push(name);
        added.push(a);
    }
    // Play and Conditional steps call actions by name: follow the renames in the actions just
    // loaded, each target looked up once by the name the file gave it.
    for a in &mut added {
        for (id, params) in &mut a.steps {
            let keys: &[&str] = match id.as_str() {
                "actions.play" => &["action"],
                "actions.conditional" => &["then", "else"],
                _ => &[],
            };
            for key in keys {
                if let Some(to) = params.get(*key).and_then(Value::as_str).and_then(|n| renamed.get(n)) {
                    params[*key] = json!(to);
                }
            }
        }
    }
    s.actions.list.extend(added);
    let keys: Vec<(String, String)> = imp.function_keys.into_iter().map(|(n, k)| (renamed.get(&n).cloned().unwrap_or(n), k)).collect();
    if !keys.is_empty() {
        s.edit_prefs(|prefs| {
            for (name, key) in &keys {
                // A function key belongs to one action.
                prefs.shortcuts.retain(|k, v| !(k.starts_with(crate::actions_cmds::PLAY_SHORTCUT_PREFIX) && v == key));
                prefs.shortcuts.insert(format!("{}{name}", crate::actions_cmds::PLAY_SHORTCUT_PREFIX), key.clone());
            }
        });
    }
    s.actions.touch();
    Ok(json!({
        "set": imp.set,
        "actions": names,
        "steps": steps,
        "unsupported": imp.unsupported,
        "functionKeys": keys.iter().map(|(n, k)| json!({"action": n, "key": k})).collect::<Vec<_>>(),
        "warnings": imp.warnings,
    }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.save";
    crate::actions_cmds::flush_recording(s)?;
    let skip = match p.get("skipUnsupported") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`skipUnsupported` must be true or false")),
    };
    let set_param = match p.get("set") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
        Some(_) => return Err(bad(cmd, "`set` must be a non-empty string")),
    };
    let (set, actions): (String, Vec<&Action>) = if p.get("action").is_some() {
        let idx = crate::actions_cmds::resolve(&s.actions.list, p, cmd)?;
        let a = &s.actions.list[idx];
        (set_param.or_else(|| a.set.clone()).unwrap_or_else(|| a.name.clone()), vec![a])
    } else if let Some(set) = set_param {
        let list: Vec<&Action> = s.actions.list.iter().filter(|a| a.set.as_deref() == Some(set.as_str())).collect();
        if list.is_empty() {
            return Err(bad(cmd, format!("no actions in a set named `{set}`")));
        }
        (set, list)
    } else {
        (DEFAULT_SET.to_string(), s.actions.list.iter().collect())
    };
    if actions.is_empty() {
        return Err(bad(cmd, "there are no actions to save"));
    }
    let names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let keys: Vec<(String, String)> =
        s.prefs().shortcuts.iter().filter_map(|(k, v)| k.strip_prefix(crate::actions_cmds::PLAY_SHORTCUT_PREFIX).map(|n| (n.to_string(), v.clone()))).collect();
    let (bytes, skipped) = export(&set, &actions, &keys, skip).map_err(|e| bad(cmd, e))?;
    let steps: usize = actions.iter().map(|a| a.steps.len()).sum::<usize>().saturating_sub(skipped.len());
    let mut out = json!({"set": set, "actions": names, "steps": steps, "bytes": bytes.len(), "skipped": skipped});
    match p.get("path") {
        None | Some(Value::Null) => {
            out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes));
        }
        Some(Value::String(path)) if !path.trim().is_empty() => {
            write_file(path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
            out["path"] = json!(path);
        }
        Some(_) => return Err(bad(cmd, "`path` must be a non-empty string")),
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    // Atomic replacement (temporary file + rename): a failed write keeps the previous ATN (#7290).
    photocraft_format::atomic_write(std::path::Path::new(path), bytes)
}

#[cfg(target_arch = "wasm32")]
fn write_file(path: &str, _bytes: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(format!("cannot write paths on the web ({path}); omit `path` to get the bytes back as `data`")))
}

fn stop_message(_: &mut Session, p: &Value) -> Result<Value> {
    let message = jstr(p, "message").unwrap_or("Stop").to_string();
    if jbool(p, "allowContinue").unwrap_or(false) {
        Ok(json!({"message": message, "continued": true}))
    } else {
        Err(EngineError::Other(format!("stopped by the action: {message}")))
    }
}

fn photoshop_step_run(_: &mut Session, p: &Value) -> Result<Value> {
    let name = jstr(p, "name").filter(|n| !n.is_empty()).unwrap_or("this step");
    let event = jstr(p, "event").unwrap_or("?");
    Err(EngineError::Other(format!("Photoshop step \"{name}\" ({event}) has no PhotoCraft equivalent yet; switch it off in the Actions panel to skip it")))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.load",
            label: "Load Actions…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".atn file"?,"data":base64 bytes?,"set":name? (override the file's set name),"replace":bool=true (replace the actions of a set of the same name)} → {set, actions:[names], steps, unsupported:[{action, step, event, name}], functionKeys:[{action, key}], warnings}. Photoshop steps PhotoCraft cannot play become actions.photoshopStep steps (kept for Save Actions; they fail at playback with the step's name); tool selections and other steps with no document effect are imported switched off. Layer references by name replay in any document."##,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "actions.save",
            label: "Save Actions…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".atn file"? (else the bytes come back as base64 `data`),"set":name? (that set's actions; default: every action, as the set "PhotoCraft Actions"),"action":name|index? (one action),"skipUnsupported":bool=false (leave out steps with no Photoshop equivalent instead of failing)} → {set, actions:[names], steps, bytes, skipped:[{action, step, id}], path|data}"##,
            enabled: always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: STOP_MESSAGE,
            label: "Stop",
            menu: &[],
            shortcut: None,
            params: r##"{"message":str,"allowContinue":bool=false} (Photoshop's Insert Stop: a step that shows the message; headless playback continues when allowContinue is set and otherwise stops the action with the message)"##,
            enabled: always,
            run: stop_message,
            journal: true,
        },
        CommandSpec {
            id: PHOTOSHOP_STEP,
            label: "Photoshop Step",
            menu: &[],
            shortcut: None,
            params: r##"{"event":id,"name":str,"descriptor":base64?} (a step from a .atn file PhotoCraft has no command for: fails at playback, written back by actions.save)"##,
            enabled: always,
            run: photoshop_step_run,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod layer_props_tests;
