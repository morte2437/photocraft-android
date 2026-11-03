use super::{Class, D, Descriptor, Id, Map, ReferenceItem, UnicodeString, Value, json};

// ------------------------------------------------------------------ descriptor access

pub(super) fn num(v: &D) -> Option<f64> {
    match v {
        D::Integer(i) => Some(f64::from(*i)),
        D::LargeInteger(i) => Some(*i as f64),
        D::Double(d) | D::UnitFloat { value: d, .. } => Some(*d),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

pub(super) fn trimmed(id: &Id) -> String {
    String::from_utf8_lossy(id.as_bytes()).trim_end().to_string()
}

pub(super) fn get_num(d: &Descriptor, k: &str) -> Option<f64> {
    d.get(k).and_then(num)
}

pub(super) fn boolean(v: &D) -> Option<bool> {
    match v {
        D::Boolean(b) => Some(*b),
        _ => None,
    }
}

pub(super) fn get_bool(d: &Descriptor, k: &str) -> Option<bool> {
    d.get(k).and_then(boolean)
}

pub(super) fn get_text(d: &Descriptor, k: &str) -> Option<String> {
    match d.get(k) {
        Some(D::Text(t)) => Some(t.to_string_lossy()),
        _ => None,
    }
}

/// An enumerated value's id, trailing spaces removed (`Ordn.Trgt` → `"Trgt"`).
pub(super) fn enumerated(v: &D) -> Option<String> {
    match v {
        D::Enumerated { value, .. } => Some(trimmed(value)),
        _ => None,
    }
}

pub(super) fn get_enum(d: &Descriptor, k: &str) -> Option<String> {
    d.get(k).and_then(enumerated)
}

pub(super) fn get_obj<'a>(d: &'a Descriptor, k: &str) -> Option<&'a Descriptor> {
    match d.get(k) {
        Some(D::Descriptor(o) | D::GlobalObject(o)) => Some(o),
        _ => None,
    }
}

pub(super) fn get_list<'a>(d: &'a Descriptor, k: &str) -> Option<&'a [D]> {
    match d.get(k) {
        Some(D::List(l)) => Some(l),
        _ => None,
    }
}

pub(super) fn get_ref<'a>(d: &'a Descriptor, k: &str) -> Option<&'a [ReferenceItem]> {
    match d.get(k) {
        Some(D::Reference(r)) => Some(r),
        _ => None,
    }
}

/// The class of a reference's first item, trimmed (`Lyr`, `Chnl`, `AdjL`…).
pub(super) fn ref_class(r: &[ReferenceItem]) -> Option<String> {
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
pub(super) enum Target {
    Ordinal(String),
    Name(String),
    Property(String),
    Class,
    Offset(i32),
    Index(u32),
    Other,
}

pub(super) fn target(r: &[ReferenceItem]) -> Target {
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
pub(super) fn layer_target(r: &[ReferenceItem]) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    match target(r) {
        Target::Ordinal(o) if o == "Trgt" => {}
        Target::Name(n) => {
            m.insert("layer".into(), json!(n));
        }
        Target::Property(p) if p == "Bckg" => {
            m.insert("layer".into(), json!({"background": true}));
        }
        _ => return None,
    }
    Some(m)
}

/// Select and Move retain stack references until playback rather than capturing local ids.
pub(super) fn recorded_layer_target(r: &[ReferenceItem]) -> Option<Map<String, Value>> {
    let selector = match r {
        [ReferenceItem::Index { class, index }] if trimmed(&class.class_id) == "Lyr" => json!({"index": index}),
        [ReferenceItem::Enumerated { class, type_id, value }]
            if trimmed(&class.class_id) == "Lyr" && type_id.is("Ordn") && matches!(trimmed(value).as_str(), "Trgt" | "Frwr" | "Bckw" | "Frnt" | "Back") =>
        {
            json!({"ordinal": trimmed(value)})
        }
        [ReferenceItem::Index { .. } | ReferenceItem::Enumerated { .. }] => return None,
        _ if matches!(target(r), Target::Index(_) | Target::Ordinal(_)) => return None,
        _ => return layer_target(r),
    };
    Some(Map::from_iter([("layer".into(), selector)]))
}

// ------------------------------------------------------------------ descriptor building

/// A Photoshop id: codes shorter than four characters are space-padded (`H   `, `Rd  `),
/// longer names stay strings.
pub(super) fn code(s: &str) -> Id {
    if s.len() < 4 { Id::new(&format!("{s:<4}")) } else { Id::new(s) }
}

pub(super) fn class(c: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: code(c) }
}

/// The pixel and percent units of a `UntF` value.
pub(super) const PX: [u8; 4] = *b"#Pxl";

pub(super) const PCT: [u8; 4] = *b"#Prc";

/// Points (1/72 in): what Photoshop calls "relative" units.
pub(super) const PT: [u8; 4] = *b"#Rlt";

pub(super) fn px(v: f64) -> D {
    D::UnitFloat { unit: PX, value: v }
}

pub(super) fn pct(v: f64) -> D {
    D::UnitFloat { unit: *b"#Prc", value: v }
}

pub(super) fn ang(v: f64) -> D {
    D::UnitFloat { unit: *b"#Ang", value: v }
}

pub(super) fn long(v: f64) -> D {
    D::Integer(v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

pub(super) fn text(s: &str) -> D {
    D::Text(UnicodeString::new_nul(s))
}

pub(super) fn enm(ty: &str, v: &str) -> D {
    D::Enumerated { type_id: code(ty), value: code(v) }
}

pub(super) fn target_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new("Ordn"), value: Id::new("Trgt") }])
}

pub(super) fn name_ref(cls: &str, name: &str) -> D {
    D::Reference(vec![ReferenceItem::Name { class: class(cls), name: UnicodeString::new_nul(name) }])
}

pub(super) fn class_ref(cls: &str) -> D {
    D::Reference(vec![ReferenceItem::Class(class(cls))])
}

/// The `null` reference for a command's `layer` param: by name, else the target layer.
pub(super) fn layer_ref(p: &Value) -> Option<D> {
    match p.get("layer") {
        None | Some(Value::Null) => Some(target_ref("Lyr ")),
        Some(Value::String(n)) => Some(name_ref("Lyr ", n)),
        Some(v) if *v == json!({"background": true}) => Some(D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: code("Bckg") }])),
        _ => None,
    }
}

/// Write the recorded form, including the padded four-byte ordinal codes.
pub(super) fn recorded_layer_ref(p: &Value) -> Option<D> {
    let Some(Value::Object(selector)) = p.get("layer") else { return layer_ref(p) };
    if !selector.contains_key("index") && !selector.contains_key("ordinal") {
        return layer_ref(p);
    }
    if selector.len() != 1 {
        return None;
    }
    let item = if let Some(index) = selector.get("index") {
        ReferenceItem::Index { class: class("Lyr "), index: u32::try_from(index.as_u64()?).ok()? }
    } else {
        let ordinal = selector.get("ordinal")?.as_str()?;
        if !matches!(ordinal, "Trgt" | "Frwr" | "Bckw" | "Frnt" | "Back") {
            return None;
        }
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code(ordinal) }
    };
    Some(D::Reference(vec![item]))
}

pub(super) fn desc(cls: &str) -> Descriptor {
    Descriptor::new(cls)
}

// ------------------------------------------------------------------ JSON access

pub(super) fn jnum(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64).filter(|f| f.is_finite())
}

pub(super) fn jbool(p: &Value, k: &str) -> Option<bool> {
    p.get(k).and_then(Value::as_bool)
}

pub(super) fn jstr<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

// ------------------------------------------------------------------ colours and enums

/// A recorded D50 Lab colour converted through the CMS to sRGB `#rrggbb`.
pub(super) fn lab_color_hex(c: &Descriptor) -> Option<String> {
    use photocraft_cms::{Builtin, TransformOptions};
    if !c.class_id.is("LbCl") {
        return None;
    }
    let (l, a, b) = (get_num(c, "Lmnc")?, get_num(c, "A   ")?, get_num(c, "B   ")?);
    let transform = photocraft_cms::cached(Builtin::LabD50.profile(), Builtin::Srgb.profile(), TransformOptions { bpc: false, ..Default::default() }).ok()?;
    // ICC v4 Lab device values are normalized, unlike the recorded L*, a*, b*.
    let input = [(l / 100.0) as f32, ((a + 128.0) / 255.0) as f32, ((b + 128.0) / 255.0) as f32];
    let mut rgb = [0.0; 3];
    transform.convert_f32(&input, 3, &mut rgb, 3, false);
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// A Photoshop colour object as `#rrggbb`: `RGBC` (0..255), `HSBC`, `Grsc`.
pub(super) fn color_hex(c: &Descriptor) -> Option<String> {
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

pub(super) fn color_desc(hex: &str) -> Option<Descriptor> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(f64::from);
    Some(desc("RGBC").with("Rd  ", D::Double(ch(0)?)).with("Grn ", D::Double(ch(2)?)).with("Bl  ", D::Double(ch(4)?)))
}

/// A `Pth ` value's text: Photoshop stores `"txtu"`, a little-endian total length, a
/// little-endian UTF-16 unit count and the units (NUL-terminated).
pub(super) fn path_text(v: &D) -> Option<String> {
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

pub(super) fn path_value(text: &str) -> D {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut b = b"txtu".to_vec();
    b.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
    b.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        b.extend_from_slice(&u.to_le_bytes());
    }
    D::Path(b)
}
