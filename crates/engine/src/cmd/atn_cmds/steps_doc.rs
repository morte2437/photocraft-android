use super::convert::clip_name;
use super::descriptors::{
    PCT, PX, Target, ang, class, color_desc, color_hex, desc, enm, get_bool, get_enum, get_num, get_obj, get_ref, get_text, jbool, jnum, jstr, long, path_text,
    path_value, pct, px, target, target_ref, text, trimmed,
};
use super::fields::{Mapped, none, step, steps};
use super::tables::{ALIGN, EXTENSION, INTENT, INTERPOLATION, MODES, PROFILES, PURGE, SAVE_FORMATS, lookup, lookup_back};
use super::{D, Descriptor, Id, Map, ReferenceItem, STOP_MESSAGE, UnicodeString, Value, json};

/// `placeEvent`: File › Place Embedded / Linked, with the file and the recorded scale.
pub(super) fn place_from(d: &Descriptor) -> Mapped {
    let Some(path) = d.get("null").and_then(path_text) else { return Mapped::Unsupported { harmless: false } };
    let mut p = json!({"path": path});
    if let Some(w) = get_num(d, "Wdth") {
        p["scale"] = json!(w);
    }
    step(if get_bool(d, "Lnkd") == Some(true) { "file.placeLinked" } else { "file.placeEmbedded" }, p)
}

pub(super) fn place_to(id: &str, p: &Value) -> Option<Descriptor> {
    let mut d = desc("null").with("null", path_value(jstr(p, "path")?)).with("FTcs", enm("QCSt", "Qcsa"));
    if let Some(s) = jnum(p, "scale") {
        d = d.with("Wdth", pct(s)).with("Hght", pct(s));
    }
    Some(d.with("Lnkd", D::Boolean(id == "file.placeLinked")).with("Antl", D::Boolean(true)))
}

/// `open`: File › Open of a recorded path.
pub(super) fn open_from(d: &Descriptor) -> Mapped {
    match d.get("null").and_then(path_text) {
        Some(path) => step("file.openAs", json!({"path": path})),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn open_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", path_value(jstr(p, "path")?)))
}

fn path_extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// `save`: a plain Save, or a Save As (`As` format, `In` destination, `Cpy` As a Copy).
/// Only a destination that is a file of the recorded format imports: a folder (#5430), a missing
/// or unknown format or a mismatched extension stays a kept Photoshop step, so playback stops
/// before writing rather than saving over the played file (#7102).
pub(super) fn save_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.get("As  ").is_none() && d.get("In  ").is_none() {
        return step("file.save", json!({}));
    }
    // A Save As needs a format descriptor to check the destination against: without one (or
    // with a format that isn't a descriptor) the step is kept.
    let Some(format) = get_obj(d, "As  ") else { return unsupported };
    let Some(path) = d.get("In  ").and_then(path_text) else { return unsupported };
    if get_bool(d, "DIDr") == Some(true) {
        return unsupported;
    }
    let Some(ext) = path_extension(&path) else { return unsupported };
    let class = trimmed(&format.class_id);
    let Some((_, exts)) = SAVE_FORMATS.iter().find(|(c, _)| *c == class) else { return unsupported };
    if !exts.contains(&ext.as_str()) {
        return unsupported;
    }
    step(if get_bool(d, "Cpy ") == Some(true) { "file.saveACopy" } else { "file.saveAs" }, json!({"path": path}))
}

pub(super) fn save_to(id: &str, p: &Value) -> Option<Descriptor> {
    if id == "file.save" {
        return Some(desc("null"));
    }
    let path = jstr(p, "path")?;
    let ext = path_extension(path)?;
    let (class, _) = SAVE_FORMATS.iter().find(|(_, exts)| exts.contains(&ext.as_str()))?;
    let d = desc("null").with("As  ", D::Descriptor(desc(class))).with("In  ", path_value(path));
    Some(if id == "file.saveACopy" { d.with("Cpy ", D::Boolean(true)) } else { d })
}

/// Photoshop's four-character condition codes that differ from the string ids.
const CONDITION_CODES: &[(&str, &str)] = &[("Lyrs", "hasLayers")];

/// `conditional`: Insert Conditional, with its Then and Else actions by name.
pub(super) fn conditional_from(d: &Descriptor) -> Mapped {
    let Some(c) = get_enum(d, "null") else { return Mapped::Unsupported { harmless: false } };
    let condition = lookup(CONDITION_CODES, &c).unwrap_or(c.as_str());
    if !crate::actions_edit_cmds::CONDITIONS.contains(&condition) {
        return Mapped::Unsupported { harmless: false };
    }
    let branch = |key: &str| match get_ref(d, key).map(target) {
        // Clipped like the stored action names, so the load-time rename map finds it.
        Some(Target::Name(n)) => Some(clip_name(&n)),
        _ => None,
    };
    step("actions.conditional", json!({"if": condition, "then": branch("then"), "else": branch("else")}))
}

pub(super) fn conditional_to(_: &str, p: &Value) -> Option<Descriptor> {
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
pub(super) fn close_from(d: &Descriptor) -> Mapped {
    if get_enum(d, "Svng").as_deref() == Some("Ys") { steps(vec![("file.save", json!({})), ("file.close", json!({}))]) } else { none("file.close") }
}

/// A Close of an explicit document has no mapping here: written as a plain Close it would
/// close the active document on playback, so it is left to `skipUnsupported`.
pub(super) fn close_to(_: &str, p: &Value) -> Option<Descriptor> {
    if p.get("document").is_some_and(|d| !d.is_null()) {
        return None;
    }
    Some(desc("null").with("Svng", enm("YsN ", "N   ")))
}

const WEB_FORMATS: &[(&str, &str)] = &[("PN24", "png24"), ("PN8", "png8"), ("JPEG", "jpeg"), ("GIFf", "gif"), ("WBMP", "wbmp")];

/// `export` (Save for Web): needs the destination path (`In`), as the command writes a file.
pub(super) fn export_from(d: &Descriptor) -> Mapped {
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

pub(super) fn export_to(_: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn convert_mode_from(d: &Descriptor) -> Mapped {
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

pub(super) fn convert_mode_to(id: &str, p: &Value) -> Option<Descriptor> {
    Some(match id {
        "image.mode.bits8" => desc("null").with("Dpth", long(8.0)),
        "image.mode.bits16" => desc("null").with("Dpth", long(16.0)),
        "image.mode.bits32" => desc("null").with("Dpth", long(32.0)),
        "image.mode.bitmap" => bitmap_to(p)?,
        _ => desc("null").with("T   ", enm("ClrS", lookup_back(MODES, id.strip_prefix("image.mode.")?)?)),
    })
}

pub(super) fn canvas_size_from(d: &Descriptor) -> Mapped {
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

pub(super) fn canvas_size_to(_: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn image_size_from(d: &Descriptor) -> Mapped {
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

pub(super) fn image_size_to(_: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn crop_from(d: &Descriptor) -> Mapped {
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

pub(super) fn crop_to(_: &str, p: &Value) -> Option<Descriptor> {
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

pub(super) fn stop_from(d: &Descriptor) -> Mapped {
    step(STOP_MESSAGE, json!({"message": get_text(d, "Msge").unwrap_or_default(), "allowContinue": get_bool(d, "Cntn").unwrap_or(false)}))
}

pub(super) fn stop_to(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("Msge", text(jstr(p, "message").unwrap_or(""))).with("Cntn", D::Boolean(jbool(p, "allowContinue").unwrap_or(false))))
}

pub(super) fn paste_from(d: &Descriptor) -> Mapped {
    none(if get_bool(d, "inPlace") == Some(true) { "edit.pasteSpecial.pasteInPlace" } else { "edit.paste" })
}

pub(super) fn paste_to(id: &str, _: &Value) -> Option<Descriptor> {
    let mut d = desc("null");
    if id == "edit.pasteSpecial.pasteInPlace" {
        d = d.with("inPlace", D::Boolean(true));
    }
    Some(d.with("AntA", enm("Annt", "Anno")))
}

pub(super) fn merge_visible_from(d: &Descriptor) -> Mapped {
    none(if get_bool(d, "Dplc") == Some(true) { "layer.stampVisible" } else { "layer.mergeVisible" })
}

pub(super) fn merge_visible_to(id: &str, _: &Value) -> Option<Descriptor> {
    Some(if id == "layer.stampVisible" { desc("null").with("Dplc", D::Boolean(true)) } else { desc("null") })
}

pub(super) fn align_from(d: &Descriptor) -> Mapped {
    match get_enum(d, "Usng").and_then(|u| lookup(ALIGN, &u)) {
        Some(a) => step(&format!("layer.align.{a}"), if get_bool(d, "alignToCanvas") == Some(true) { json!({"to": "canvas"}) } else { json!({}) }),
        None => Mapped::Unsupported { harmless: false },
    }
}

pub(super) fn align_to(id: &str, p: &Value) -> Option<Descriptor> {
    let a = lookup_back(ALIGN, id.strip_prefix("layer.align.")?)?;
    Some(desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", a)).with("alignToCanvas", D::Boolean(jstr(p, "to") == Some("canvas"))))
}

pub(super) fn purge_from(d: &Descriptor) -> Mapped {
    match get_enum(d, "null").and_then(|u| lookup(PURGE, &u)) {
        Some(p) => none(&format!("edit.purge.{p}")),
        None => Mapped::Unsupported { harmless: true },
    }
}

pub(super) fn purge_to(id: &str, _: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", enm("PrgI", lookup_back(PURGE, id.strip_prefix("edit.purge.")?)?)))
}

pub(super) fn reset_from(d: &Descriptor) -> Mapped {
    match get_ref(d, "null").map(target) {
        Some(Target::Property(p)) if p == "Clrs" => none("tools.defaultColors"),
        _ => Mapped::Unsupported { harmless: true },
    }
}

pub(super) fn reset_to(_: &str, _: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: Id::new("Clrs") }])))
}

pub(super) fn profile_from(d: &Descriptor, id: &str) -> Mapped {
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

pub(super) fn profile_to(id: &str, p: &Value) -> Option<Descriptor> {
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
