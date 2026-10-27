use super::adjustments::{HUE_RANGES, adjustment_by_class, adjustment_by_suffix, adjustment_steps};
use super::descriptors::{
    PX, Target, ang, class, class_ref, code, color_desc, color_hex, desc, enm, get_bool, get_enum, get_list, get_num, get_obj, get_ref, get_text, jbool, jnum,
    jstr, layer_ref, layer_target, long, name_ref, pct, px, ref_class, target, target_ref, text, trimmed,
};
use super::fields::{Mapped, none, step, steps};
use super::steps_layer::quick_mask_ref;
use super::tables::{LABEL_COLORS, MODES, blend_label, blend_ps, lookup, lookup_back};
use super::*;

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

pub(super) fn make_from(d: &Descriptor) -> Mapped {
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

pub(super) fn make_to(id: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn set_from(d: &Descriptor) -> Mapped {
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

pub(super) fn set_to(id: &str, p: &Value) -> Option<Descriptor> {
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
            let kind = jstr(p, "kind")?;
            // `layer.setAdjustment` changes only the settings it names. Color Balance and
            // Hue/Saturation descriptors carry every setting, filling absent ones with defaults,
            // so a partial edit would replay as one that also resets the rest (#8531).
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
