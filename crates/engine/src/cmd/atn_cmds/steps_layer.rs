use super::convert::clip_name;
use super::descriptors::{
    PT, PX, Target, ang, class, code, color_desc, color_hex, desc, enm, get_bool, get_enum, get_num, get_obj, get_ref, get_text, jbool, jnum, jstr, layer_ref,
    layer_target, long, name_ref, pct, px, ref_class, target, target_ref, text, trimmed,
};
use super::events::pixel_target;
use super::fields::{F, Kind, Mapped, fields_to, none, step, steps};
use super::steps_make::is_tool_class;
use super::tables::{FILL_CONTENTS, INTERPOLATION, MENU_ITEMS, STROKE_LOCATION, blend_label, blend_ps, lookup, lookup_back};
use super::{D, Descriptor, Id, Map, ReferenceItem, UnicodeString, Value, json};

pub(super) fn select_from(d: &Descriptor) -> Mapped {
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

pub(super) fn select_to(id: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn delete_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null") {
        None => none("edit.clear"),
        Some(r) if ref_class(r).as_deref() == Some("Lyr") => match layer_target(r) {
            Some(p) => step("layer.delete", Value::Object(p)),
            None => Mapped::Unsupported { harmless: false },
        },
        // The targeted channel of a layer is its mask (what Photoshop records after making an
        // adjustment layer over a selection and discarding the mask).
        Some(r) if ref_class(r).as_deref() == Some("Chnl") && matches!(target(r), Target::Ordinal(o) if o == "Trgt") => none("layer.layerMask.delete"),
        // The mask Photoshop adds to a new fill or adjustment layer, or to a smart object's filters.
        Some(r) if ref_class(r).as_deref() == Some("Chnl") => match target(r) {
            Target::Ordinal(o) if o == "Msk" => none("layer.layerMask.delete"),
            // Photoshop adds an empty filter mask with the first smart filter; PhotoCraft doesn't.
            Target::Ordinal(o) if o == "filterMask" => step("layer.smartFilter.deleteFilterMask", json!({"ifPresent": true})),
            _ => Mapped::Unsupported { harmless: false },
        },
        // Timeline frames: PhotoCraft has no frame animation, so there is nothing to delete.
        Some(r) if ref_class(r).as_deref() == Some("animationClass") => Mapped::Unsupported { harmless: true },
        Some(_) => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn quick_mask_ref() -> D {
    D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: code("QucM") },
        ReferenceItem::Enumerated { class: class("Dcmn"), type_id: code("Ordn"), value: code("Trgt") },
    ])
}

pub(super) fn clear_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Property(p)) if p == "QucM" => step("select.editInQuickMaskMode", json!({"on": false})),
        // History panel menu › Clear History.
        Some(Target::Property(p)) if p == "HsSt" => none("history.clear"),
        None => none("edit.clear"),
        _ => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn clear_to(id: &str, p: &Value) -> Option<Descriptor> {
    match id {
        "select.editInQuickMaskMode" if jbool(p, "on") == Some(false) => Some(desc("null").with("null", quick_mask_ref())),
        "history.clear" => Some(desc("null").with(
            "null",
            D::Reference(vec![
                ReferenceItem::Property { class: class("Prpr"), key: code("HsSt") },
                ReferenceItem::Enumerated { class: class("Dcmn"), type_id: code("Ordn"), value: code("Trgt") },
            ]),
        )),
        _ => None,
    }
}

pub(super) fn delete_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "edit.clear" => desc("null"),
        "layer.delete" => desc("null").with("null", layer_ref(p)?),
        "layer.layerMask.delete" => desc("null").with("null", target_ref("Chnl")),
        "layer.smartFilter.deleteFilterMask" => desc("null")
            .with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: Id::new("filterMask") }])),
        _ => return None,
    })
}

/// `hide` / `show` take a list of references (or one).
pub(super) fn visibility_from(d: &Descriptor, id: &str) -> Mapped {
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

pub(super) fn visibility_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", D::List(vec![layer_ref(p)?])))
}

pub(super) fn move_from(d: &Descriptor) -> Mapped {
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

pub(super) fn move_to(id: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn duplicate_from(d: &Descriptor) -> Mapped {
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

pub(super) fn duplicate_to(id: &str, p: &Value) -> Option<Descriptor> {
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
pub(super) fn apply_comp_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null") {
        Some([ReferenceItem::Name { class, name }]) if trimmed(&class.class_id) == "compsClass" => {
            step("layerComp.apply", json!({"comp": name.to_string_lossy()}))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn apply_comp_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", name_ref("compsClass", jstr(p, "comp")?)))
}

pub(super) fn play_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Name(n)) => step("actions.play", json!({"action": clip_name(&n)})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn play_to(_: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn fill_from(d: &Descriptor) -> Mapped {
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

pub(super) fn fill_to(id: &str, p: &Value) -> Option<Descriptor> {
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
pub(super) fn stroke_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !pixel_target(p) {
        return None;
    }
    let mut params = if p.is_null() { Map::new() } else { p.as_object()?.clone() };
    params.insert("width".into(), Value::from(crate::extra_cmds::stroke_width(p)));
    fields_to(&Value::Object(params), STROKE, desc("null"))
}

pub(super) const STROKE: &[F] = &[
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

pub(super) fn rotate_from(d: &Descriptor) -> Mapped {
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

pub(super) fn rotate_to(id: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn flip_from(d: &Descriptor) -> Mapped {
    let Some(axis) = get_enum(d, "Axis") else { return Mapped::Unsupported { harmless: false } };
    let horizontal = axis == "Hrzn";
    none(match (targets_layer(d), horizontal) {
        (true, true) => "edit.transform.flipHorizontal",
        (true, false) => "edit.transform.flipVertical",
        (false, true) => "image.imageRotation.flipCanvasHorizontal",
        (false, false) => "image.imageRotation.flipCanvasVertical",
    })
}

pub(super) fn flip_to(id: &str, _: &Value) -> Option<Descriptor> {
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
