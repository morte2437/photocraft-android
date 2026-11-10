const DIFFUSE_MODE: &[(&str, &str)] = &[("Nrml", "normal"), ("DrkO", "darkenOnly"), ("LghO", "lightenOnly"), ("Anst", "anisotropic")];
use super::adjustments::{adjustment_by_class, adjustment_steps, levels_from, levels_to, shadows_highlights_from, shadows_highlights_to};
use super::descriptors::{
    class, code, desc, get_bool, get_obj, get_ref, jbool, layer_ref, layer_target, path_text, path_value, ref_class, target_ref, trimmed,
};
use super::fields::{F, Kind, Mapped, fields_from, fields_to, none, step};
use super::steps_artboard::{artboard_edit_from, artboard_edit_to, artboard_from, artboard_to};
use super::steps_channels::{match_color_from, match_color_to, merge_spot_from, merge_spot_to, split_from, split_to, trap_from, trap_to};
use super::steps_content_aware::{caf_from, caf_to};
use super::steps_direct_adjustments::{gradient_from, gradient_to, replace_from, replace_to};
use super::steps_distort::{PINCH, POLAR, SPHERIZE, TWIRL, ZIGZAG, displace_from, displace_to, keys_from, keys_to, shear_from, shear_to, wave_from, wave_to};
use super::steps_doc::{
    align_from, align_to, canvas_size_from, canvas_size_to, close_from, close_to, conditional_from, conditional_to, convert_mode_from, convert_mode_to,
    crop_from, crop_to, export_from, export_to, image_size_from, image_size_to, merge_visible_from, merge_visible_to, open_from, open_to, paste_from, paste_to,
    place_from, place_to, profile_from, profile_to, purge_from, purge_to, reset_from, reset_to, save_from, save_to, stop_from, stop_to,
};
use super::steps_image::{
    GALLERY_IDS, REFINE_EDGE, RIPPLE_SIZE, SMART_BRUSH, apply_image_from, apply_image_to, camera_raw_from, camera_raw_to, color_range_from, color_range_to,
    empty_from, empty_to, gallery_from, gallery_to, has_refine_strokes, oil_paint_from, oil_paint_to, radial_blur_from, radial_blur_to, refine_edge_to,
    select_sky_from, select_sky_to, smart_sharpen_from, smart_sharpen_to, transform_from, transform_to,
};
use super::steps_layer::{
    STROKE, apply_comp_from, apply_comp_to, clear_from, clear_to, delete_from, delete_to, duplicate_from, duplicate_to, fill_from, fill_to, flip_from, flip_to,
    move_from, move_to, paste_effects_from, paste_effects_to, play_from, play_to, rotate_from, rotate_to, select_from, select_to, stroke_to, visibility_from,
    visibility_to,
};
use super::steps_layer_style::{scale_from, style_to};
use super::steps_make::{locking_from, locking_to, make_from, make_to, set_from, set_to};
use super::steps_other::{DE_INTERLACE, HSB_HSL, bare_from, bare_to, custom_from, custom_to, enums_from, enums_to};
use super::tables::{NOISE, OFFSET_FILL, SMART_BLUR_MODE, SMART_BLUR_QUALITY, TRIM};
use super::{AtnEvent, D, Descriptor, Id, Map, ReferenceItem, STOP_MESSAGE, Value, json};

/// An event: code (empty when Photoshop only has a string name), string name, the panel
/// label, the command ids it exports (the first is the one simple events import to), and the
/// two converters.
pub(super) struct Event {
    pub(super) code: &'static str,
    pub(super) name: &'static str,
    pub(super) label: &'static str,
    pub(super) ids: &'static [&'static str],
    pub(super) from: fn(&Descriptor) -> Mapped,
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

macro_rules! stylize_event {
    ($code:literal, $name:literal, $label:literal, $id:literal) => {
        Event { code: $code, name: $name, label: $label, ids: &[$id], from: |d| super::steps_stylize::from($id, d), to: super::steps_stylize::to }
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
/// The exported event has nowhere to put these, so a step that sets one cannot be written: it
/// would import as a different edit, silently. Such a step is refused, the way [`pixel_target`]
/// refuses a filter aimed at a mask -- and no key Photoshop does not define is invented for it,
/// which the clean-room rule forbids and which would make the file wrong in another reader.
///
/// A step that leaves the setting at the value the event plays as exports as before.
/// How a setting the event cannot carry behaves once the step is replayed without it. Which of
/// these a setting takes follows the command's own reading of it, not the parameter spec.
enum Lost {
    /// Replay uses this value, so only a step already at it survives. Numbers, strings and
    /// booleans are compared as written, since the commands below read these settings as written.
    ReplaysAs(&'static str),
    /// Replay uses this boolean, and the command refuses any other type, so only a step at this
    /// boolean survives: a string such as `"true"` is refused.
    ReplaysAsBool(bool),
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
    // Eqlz is exported with no fields; replay equalizes the selected area only.
    ("image.adjustments.equalize", &[("selectedAreaOnly", Lost::ReplaysAsBool(true))]),
];

/// Whether every setting of `p` that its event cannot hold would play the same without it
/// (see [`UNHOLDABLE`]).
fn holds_its_settings(id: &str, p: &Value) -> bool {
    let Some((_, settings)) = UNHOLDABLE.iter().find(|(cmd, _)| *cmd == id) else { return true };
    settings.iter().all(|(param, lost)| match (p.get(param), lost) {
        (None, _) => true,
        // The command refuses null here too, so a null step is refused like any other non-boolean.
        (Some(v), Lost::ReplaysAsBool(played)) => v.as_bool() == Some(*played),
        (Some(Value::Null), _) => true,
        (Some(v), Lost::ReplaysAs(played)) => {
            v.as_str() == Some(played)
                || v.as_f64().is_some_and(|n| played.parse::<f64>().is_ok_and(|d| n == d))
                || v.as_bool().is_some_and(|b| played.parse::<bool>() == Ok(b))
        }
        (Some(v), Lost::DiffersAt(spelling)) => v.as_str() != Some(spelling),
    })
}

/// Whether `p` edits the layer's pixels: no `"target"` or `"target":"pixels"`. Photoshop's
/// descriptor for a plain event has no field for a mask, Quick Mask or channel target, so such
/// a step can't be exported faithfully.
pub(super) fn pixel_target(p: &Value) -> bool {
    p.get("target").is_none_or(|t| t.as_str() == Some("pixels"))
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

/// Check Spelling's `null`: the `spellCheck` property of all text layers.
fn spell_check_ref() -> D {
    D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: Id::new("spellCheck") },
        ReferenceItem::Enumerated { class: class("TxLr"), type_id: code("Ordn"), value: code("Al") },
    ])
}

/// Edit › Check Spelling (`spellCheck`): `null` is [`spell_check_ref`] and `Usng` holds only
/// `checkAll`. Any other form stays a kept Photoshop step.
fn spell_check_from(d: &Descriptor) -> Mapped {
    let same_ref = |r: &[ReferenceItem]| match r {
        [ReferenceItem::Property { class: p, key }, ReferenceItem::Enumerated { class: t, type_id, value }] => {
            trimmed(&p.class_id) == "Prpr" && key.is("spellCheck") && trimmed(&t.class_id) == "TxLr" && trimmed(type_id) == "Ordn" && trimmed(value) == "Al"
        }
        _ => false,
    };
    // Two keys, read by name: `null` and `Usng` in either order, each once.
    let using = get_obj(d, "Usng").filter(|u| u.class_id.is("spellCheck") && u.items.len() == 1);
    match (d.items.len(), get_ref(d, "null"), using.and_then(|u| get_bool(u, "checkAll"))) {
        (2, Some(r), Some(all)) if same_ref(r) => step("edit.checkSpelling", json!({"allLayers": all})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// A Check Spelling step that holds nothing but `allLayers` (Check All Layers, on by default).
fn spell_check_to(_: &str, p: &Value) -> Option<Descriptor> {
    let params = p.as_object()?;
    if params.keys().any(|k| k != "allLayers") {
        return None;
    }
    let all = match params.get("allLayers") {
        None => true,
        Some(v) => v.as_bool()?,
    };
    Some(desc("null").with("null", spell_check_ref()).with("Usng", D::Descriptor(desc("spellCheck").with("checkAll", D::Boolean(all)))))
}

// Adapter-only transport bit: the reference permits either no target key or the
// current-layer target. Save must retain that distinction; native behavior is identical.
const NO_TARGET: &str = "_atnNoTarget";

/// Accept only bare or current-layer targets, retaining their key-presence distinction for save.
fn target_params(d: &Descriptor) -> Option<Map<String, Value>> {
    let mut p = Map::new();
    if d.items.is_empty() {
        p.insert(NO_TARGET.into(), Value::Bool(true));
    } else if d.items != desc("null").with("null", target_ref("Lyr ")).items {
        return None;
    }
    Some(p)
}

/// Restore the bare or explicit current-layer descriptor selected by the private transport bit.
fn target_descriptor(p: &Value) -> Option<Descriptor> {
    match p.get(NO_TARGET) {
        None => Some(desc("null").with("null", target_ref("Lyr "))),
        Some(Value::Bool(true)) => Some(desc("null")),
        Some(_) => None,
    }
}

// Native-parameter allowlist adapted from #10628; refuse unrepresented options on export.
/// Refuse native options that the selected Photoshop form cannot represent.
fn params(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_some_and(|p| p.keys().all(|key| allowed.contains(&key.as_str())))
}

/// Import a target-only event, refusing other targets and extra descriptor settings.
fn target_only_from(d: &Descriptor, id: &str) -> Mapped {
    target_params(d).map_or(Mapped::Unsupported { harmless: false }, |p| step(id, Value::Object(p)))
}

/// Export a current-target native step only when no unrepresented options are present.
fn target_only_to(_: &str, p: &Value) -> Option<Descriptor> {
    if !params(p, &[NO_TARGET]) {
        return None;
    }
    target_descriptor(p)
}

const SHAPE_BLUR: &[F] = &[F("Rds ", "radius", Kind::Px)];

/// Dispatch the radius-only and installed custom-shape forms through one event.
fn shape_blur_from(d: &Descriptor) -> Mapped {
    if d.get("customShape").is_some() {
        return super::steps_shape_blur::from(d);
    }
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(p) = fields_from(d, SHAPE_BLUR) else { return unsupported };
    let p = Value::Object(p);
    if shape_blur_to("filter.blur.shapeBlur", &p).is_some_and(|expected| d.items == expected.items) { step("filter.blur.shapeBlur", p) } else { unsupported }
}

/// Keep both represented forms; arbitrary native geometry and other options still refuse export.
fn shape_blur_to(id: &str, p: &Value) -> Option<Descriptor> {
    if ["shapeName", "shapeId", "shapeOriginType"].iter().any(|key| p.get(*key).is_some()) {
        return super::steps_shape_blur::to(id, p);
    }
    if !params(p, &["radius"]) {
        return None;
    }
    if let Some(radius) = p.get("radius") {
        let radius = radius.as_f64()?;
        if !radius.is_finite() || !(5.0..=1000.0).contains(&radius) {
            return None;
        }
    }
    fields_to(p, SHAPE_BLUR, desc("null"))
}

// Recorded Link and Unlink need explicit state: replaying the UI toggle can
// perform the opposite operation. Target matching follows #10628's adapter.
/// Import Link or Unlink as an explicit operation rather than the state-dependent UI toggle.
fn link_from(d: &Descriptor, linked: bool) -> Mapped {
    let Some(mut p) = target_params(d) else { return Mapped::Unsupported { harmless: false } };
    p.insert("linked".into(), Value::Bool(linked));
    step("layer.linkLayers", Value::Object(p))
}

/// Export only the matching explicit link state and supported current-layer target form.
fn link_to(p: &Value, linked: bool) -> Option<Descriptor> {
    if !params(p, &["linked", NO_TARGET]) || p.get("linked") != Some(&Value::Bool(linked)) {
        return None;
    }
    target_descriptor(p)
}

/// Map canonical Smart Object file descriptors, refusing unrepresented paths, targets or keys.
fn smart_path_from(d: &Descriptor, id: &str) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let key = if id == "layer.smartObjects.convertToLinked" { "Usng" } else { "null" };
    let Some(path) = d.get(key).and_then(path_text) else { return unsupported };
    let p = json!({"path": path});
    let Some(mut expected) = smart_path_to(id, &p) else { return unsupported };
    if let Some(hint) = d.get("LyrI") {
        // The reference explicitly omits recording-session IDs on save (#10603,
        // docs/atn.md:200). They must never become PhotoCraft's playback target.
        if !matches!(id, "layer.smartObjects.relinkToFile" | "layer.smartObjects.replaceContents") || !matches!(hint, D::Integer(n) if *n > 0) {
            return unsupported;
        }
        expected = expected.with("LyrI", hint.clone());
    }
    // Normalize the top-level header as Open/Place do, but refuse unknown keys,
    // targets and path encodings rather than partly replaying them.
    if d.items == expected.items { step(id, p) } else { unsupported }
}

/// Export path-only native parameters using the existing Open/Place path encoding.
fn smart_path_to(id: &str, p: &Value) -> Option<Descriptor> {
    let p = p.as_object()?;
    let path = p.get("path")?.as_str()?;
    if p.len() != 1 || path.is_empty() || path.contains('\0') {
        return None;
    }
    Some(if id == "layer.smartObjects.convertToLinked" {
        desc("null").with("null", target_ref("Lyr ")).with("Usng", path_value(path))
    } else {
        desc("null").with("null", path_value(path))
    })
}

/// A clipping mask event: `clipping_from` / `clipping_to` with one command id.
macro_rules! clipping_event {
    ($code:literal, $name:literal, $label:literal, $id:literal) => {
        Event { code: $code, name: $name, label: $label, ids: &[$id], from: |d| clipping_from(d, $id), to: clipping_to }
    };
}

/// Group F routes every event through `steps_selection::from`; the outer header (name, class) is not checked.
macro_rules! selection_event {
    ($code:literal, $name:literal, $label:literal, $ids:expr, $from:expr, $to:expr) => {
        Event { code: $code, name: $name, label: $label, ids: $ids, from: |d| super::steps_selection::from(d, $from), to: $to }
    };
}

macro_rules! target_event {
    ($name:literal, $label:literal, $id:literal) => {
        Event { code: "", name: $name, label: $label, ids: &[$id], from: |d| target_only_from(d, $id), to: target_only_to }
    };
}

/// Every Photoshop event PhotoCraft maps. The order matters only for the panel label.
pub(super) const EVENTS: &[Event] = &[
    selection_event!("AddT", "addTo", "Add to Selection", &["select.rect"], |d| super::steps_selection::selection_from(d, "add"), |_, p| {
        super::steps_selection::selection_to(p, "add")
    }),
    selection_event!("SbtF", "subtractFrom", "Subtract from Selection", &["select.rect"], |d| super::steps_selection::selection_from(d, "subtract"), |_, p| {
        super::steps_selection::selection_to(p, "subtract")
    }),
    selection_event!(
        "IntW",
        "intersectWith",
        "Intersect with Selection",
        &["select.rect"],
        |d| super::steps_selection::selection_from(d, "intersect"),
        |_, p| super::steps_selection::selection_to(p, "intersect")
    ),
    selection_event!(
        "PstI",
        "pasteInto",
        "Paste Into",
        &["edit.pasteSpecial.pasteInto"],
        |d| super::steps_selection::paste_from(d, "edit.pasteSpecial.pasteInto"),
        super::steps_selection::paste_to
    ),
    selection_event!(
        "PstO",
        "pasteOutside",
        "Paste Outside",
        &["edit.pasteSpecial.pasteOutside"],
        |d| super::steps_selection::paste_from(d, "edit.pasteSpecial.pasteOutside"),
        super::steps_selection::paste_to
    ),
    selection_event!("Fade", "fade", "Fade", &["edit.fade"], super::steps_selection::fade_from, super::steps_selection::fade_to),
    selection_event!("", "linkSelectedLayers", "Link Layers", &["layer.linkLayers"], super::steps_selection::link_from, super::steps_selection::link_to),
    selection_event!(
        "Dstr",
        "distribute",
        "Distribute",
        &[
            "layer.distribute.leftEdges",
            "layer.distribute.horizontalCenters",
            "layer.distribute.rightEdges",
            "layer.distribute.topEdges",
            "layer.distribute.verticalCenters",
            "layer.distribute.bottomEdges",
        ],
        super::steps_selection::distribute_from,
        super::steps_selection::distribute_to
    ),
    selection_event!(
        "RmvL",
        "removeLayerMask",
        "Remove Layer Mask",
        &["layer.layerMask.delete"],
        |d| super::steps_selection::empty_from(d, "layer.layerMask.delete", Some("RmvL")),
        super::steps_selection::empty_to
    ),
    selection_event!(
        "DfnP",
        "definePattern",
        "Define Pattern",
        &["edit.definePattern"],
        |d| super::steps_selection::empty_from(d, "edit.definePattern", None),
        super::steps_selection::empty_to
    ),
    selection_event!(
        "MrgL",
        "mergeLayers",
        "Merge Layers",
        &["layer.mergeLayers"],
        |d| super::steps_selection::empty_from(d, "layer.mergeLayers", Some("MrgL")),
        super::steps_selection::empty_to
    ),
    // Filters.
    Event {
        code: "Clds",
        name: "clouds",
        label: "Clouds",
        ids: &["filter.render.clouds"],
        from: |d| super::steps_render::clouds_from("filter.render.clouds", d),
        to: super::steps_render::clouds_to,
    },
    Event {
        code: "DfrC",
        name: "differenceClouds",
        label: "Difference Clouds",
        ids: &["filter.render.differenceClouds"],
        from: |d| super::steps_render::clouds_from("filter.render.differenceClouds", d),
        to: super::steps_render::clouds_to,
    },
    Event {
        code: "Fbrs",
        name: "",
        label: "Fibers",
        ids: &["filter.render.fibers"],
        from: super::steps_render::fibers_from,
        to: super::steps_render::fibers_to,
    },
    Event {
        code: "LnsF",
        name: "lensFlare",
        label: "Lens Flare",
        ids: &["filter.render.lensFlare"],
        from: super::steps_render::flare_from,
        to: super::steps_render::flare_to,
    },
    stylize_event!("FndE", "findEdges", "Find Edges", "filter.stylize.findEdges"),
    stylize_event!("Slrz", "solarize", "Solarize", "filter.stylize.solarize"),
    stylize_event!("Fct ", "facet", "Facet", "filter.pixelate.facet"),
    stylize_event!("Frgm", "fragment", "Fragment", "filter.pixelate.fragment"),
    stylize_event!("Extr", "extrude", "Extrude", "filter.stylize.extrude"),
    stylize_event!("TrcC", "traceContour", "Trace Contour", "filter.stylize.traceContour"),
    stylize_event!("Wnd ", "wind", "Wind", "filter.stylize.wind"),
    stylize_event!("Tls ", "tiles", "Tiles", "filter.stylize.tiles"),
    stylize_event!("ClrH", "colorHalftone", "Color Halftone", "filter.pixelate.colorHalftone"),
    stylize_event!("Mztn", "mezzotint", "Mezzotint", "filter.pixelate.mezzotint"),
    stylize_event!("Msc ", "mosaic", "Mosaic", "filter.pixelate.mosaic"),
    stylize_event!("Pntl", "pointillize", "Pointillize", "filter.pixelate.pointillize"),
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
    Event { code: "", name: "shapeBlur", label: "Shape Blur", ids: &["filter.blur.shapeBlur"], from: shape_blur_from, to: shape_blur_to },
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
    // Distort (#10605): every key in recorded order, Photoshop's limits and defaults.
    Event { code: "", name: "displace", label: "Displace", ids: &["filter.distort.displace"], from: displace_from, to: displace_to },
    Event { code: "Shr ", name: "shear", label: "Shear", ids: &["filter.distort.shear"], from: shear_from, to: shear_to },
    Event {
        code: "Pnch",
        name: "pinch",
        label: "Pinch",
        ids: &["filter.distort.pinch"],
        from: |d| keys_from(d, "filter.distort.pinch", PINCH),
        to: |_, p| keys_to(p, PINCH, &[("amount", json!(50))], &[]),
    },
    Event {
        code: "Plr ",
        name: "polar",
        label: "Polar Coordinates",
        ids: &["filter.distort.polarCoordinates"],
        from: |d| keys_from(d, "filter.distort.polarCoordinates", POLAR),
        to: |_, p| keys_to(p, POLAR, &[("mode", json!("rectangularToPolar"))], &[]),
    },
    Event {
        code: "Sphr",
        name: "spherize",
        label: "Spherize",
        ids: &["filter.distort.spherize"],
        from: |d| keys_from(d, "filter.distort.spherize", SPHERIZE),
        to: |_, p| keys_to(p, SPHERIZE, &[("amount", json!(100)), ("mode", json!("normal"))], &[]),
    },
    Event {
        code: "Twrl",
        name: "twirl",
        label: "Twirl",
        ids: &["filter.distort.twirl"],
        from: |d| keys_from(d, "filter.distort.twirl", TWIRL),
        to: |_, p| keys_to(p, TWIRL, &[("angle", json!(50))], &[]),
    },
    Event { code: "Wave", name: "wave", label: "Wave", ids: &["filter.distort.wave"], from: wave_from, to: wave_to },
    Event {
        code: "ZgZg",
        name: "zigZag",
        label: "ZigZag",
        ids: &["filter.distort.zigZag"],
        from: |d| keys_from(d, "filter.distort.zigZag", ZIGZAG),
        to: |_, p| keys_to(p, ZIGZAG, &[("amount", json!(10)), ("ridges", json!(5)), ("style", json!("pondRipples"))], &[]),
    },
    ev!("Rple", "ripple", "Ripple", "filter.distort.ripple", &[F("Amnt", "amount", Kind::ExactLong), F("RplS", "size", Kind::Enum("RplS", RIPPLE_SIZE))]),
    ev!("Crst", "crystallize", "Crystallize", "filter.pixelate.crystallize", &[F("ClSz", "cellSize", Kind::ExactLong)]),
    ev!("Dfs ", "diffuse", "Diffuse", "filter.stylize.diffuse", &[F("Md  ", "mode", Kind::Enum("DfsM", DIFFUSE_MODE)), F("FlRs", "seed", Kind::Long)]),
    Event { code: "GEfc", name: "filterGallery", label: "Filter Gallery", ids: GALLERY_IDS, from: gallery_from, to: gallery_to },
    // What Photoshop needs in a written step (process version, the full settings set) is
    // unverified, so only an imported step saves back, as its recorded descriptor while unedited.
    // The label is interchange data, not a display name: Save Actions writes it into the step's
    // dictionary (convert.rs), so it stays Photoshop's wording and a loaded step round-trips. The
    // menu item and window title come from `lens_cmds` and read "RAW Develop Filter".
    Event { code: "", name: "Adobe Camera Raw Filter", label: "Camera Raw Filter", ids: &[crate::lens_cmds::RAW], from: camera_raw_from, to: camera_raw_to },
    // Video, Other, Average and matting (#10607).
    Event {
        code: "Dntr",
        name: "deInterlace",
        label: "De-Interlace",
        ids: &["filter.video.deInterlace"],
        from: |d| enums_from(d, "filter.video.deInterlace", DE_INTERLACE),
        to: |_, p| enums_to(p, DE_INTERLACE),
    },
    Event {
        code: "NTSC",
        name: "NTSC",
        label: "NTSC Colors",
        ids: &["filter.video.ntscColors"],
        from: |d| bare_from(d, "filter.video.ntscColors"),
        to: bare_to,
    },
    Event {
        code: "HsbP",
        name: "HsbP",
        label: "HSB/HSL",
        ids: &["filter.other.hsbHsl"],
        from: |d| enums_from(d, "filter.other.hsbHsl", HSB_HSL),
        to: |_, p| enums_to(p, HSB_HSL),
    },
    Event { code: "Cstm", name: "custom", label: "Custom", ids: &["filter.other.custom"], from: custom_from, to: custom_to },
    Event { code: "Avrg", name: "Avrg", label: "Average", ids: &["filter.blur.average"], from: |d| bare_from(d, "filter.blur.average"), to: bare_to },
    Event {
        code: "RmvB",
        name: "removeBlackMatte",
        label: "Remove Black Matte",
        ids: &["layer.matting.removeBlackMatte"],
        from: |d| bare_from(d, "layer.matting.removeBlackMatte"),
        to: bare_to,
    },
    Event {
        code: "RmvW",
        name: "removeWhiteMatte",
        label: "Remove White Matte",
        ids: &["layer.matting.removeWhiteMatte"],
        from: |d| bare_from(d, "layer.matting.removeWhiteMatte"),
        to: bare_to,
    },
    // Newer features (#10610): Photoshop records these by string ID only.
    Event { code: "", name: "oilPaint", label: "Oil Paint", ids: &["filter.stylize.oilPaint"], from: oil_paint_from, to: oil_paint_to },
    Event { code: "", name: "smartSharpen", label: "Smart Sharpen", ids: &["filter.sharpen.smartSharpen"], from: smart_sharpen_from, to: smart_sharpen_to },
    Event { code: "", name: "focusMask", label: "Focus Area", ids: &["select.focusArea"], from: |d| empty_from(d, "select.focusArea"), to: empty_to },
    Event { code: "", name: "selectSky", label: "Sky", ids: &["select.sky"], from: select_sky_from, to: select_sky_to },
    Event {
        code: "",
        name: "removeBackground",
        label: "Remove Background",
        ids: &["layer.removeBackground"],
        from: |d| empty_from(d, "layer.removeBackground"),
        to: empty_to,
    },
    // Adjustments (Image › Adjustments).
    Event {
        code: "",
        name: "colorLookup",
        label: "Color Lookup",
        ids: &["image.adjustments.colorLookup"],
        from: super::steps_lookup::from,
        to: super::steps_lookup::to,
    },
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
    // Written as the string event the reference application runs, with the `vibrance` class it
    // records; the old `Vbrn` code still loads through LEGACY_CODES.
    adjustment_event!("", "vibrance", "vibrance", "Vibrance"),
    adjustment_event!("Exps", "Exps", "exposure", "Exposure"),
    // Photoshop runs Photo Filter by name only: `PhtF` is the class, not an event (#8301).
    adjustment_event!("", "PhtF", "photoFilter", "Photo Filter"),
    adjustment_event!("", "BanW", "blackAndWhite", "Black & White", "image.adjustments.blackWhite"),
    adjustment_event!("ChnM", "ChnM", "channelMixer", "Channel Mixer"),
    Event { code: "GrMp", name: "", label: "Gradient Map", ids: &["image.adjustments.gradientMap"], from: gradient_from, to: gradient_to },
    adjustment_event!("", "GdMp", "gradientMapEvent", "Gradient Map", "image.adjustments.gradientMap"),
    Event { code: "RplC", name: "replaceColor", label: "Replace Color", ids: &["image.adjustments.replaceColor"], from: replace_from, to: replace_to },
    adjustment_event!("SlcC", "SlcC", "selectiveColor", "Selective Color"),
    Event { code: "", name: "matchColor", label: "Match Color", ids: &["image.adjustments.matchColor"], from: match_color_from, to: match_color_to },
    ev!("Dstt", "desaturate", "Desaturate", "image.adjustments.desaturate", &[]),
    ev!("Eqlz", "equalize", "Equalize", "image.adjustments.equalize", &[]),
    ev!("", "autoColor", "Auto Color", "image.autoColor", &[]),
    // Channels and Trap (#10609).
    Event { code: "SplC", name: "splitChannels", label: "Split Channels", ids: &["channel.split"], from: split_from, to: split_to },
    Event { code: "MSpt", name: "mergeSpotChannel", label: "Merge Spot Channel", ids: &["channel.mergeSpot"], from: merge_spot_from, to: merge_spot_to },
    Event { code: "Trap", name: "trap", label: "Trap", ids: &["image.trap"], from: trap_from, to: trap_to },
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
    Event { code: "", name: "scaleEffectsEvent", label: "Scale Effects", ids: &["layer.layerStyle.scaleEffects"], from: scale_from, to: style_to },
    target_event!("reverse", "Reverse", "layer.arrange.reverse"),
    target_event!("selectLinkedLayers", "Select Linked Layers", "layer.selectLinkedLayers"),
    target_event!("disableLayerStyle", "Clear Layer Style", "layer.layerStyle.clear"),
    target_event!("rasterizeAll", "Rasterize All Layers", "layer.rasterize.allLayers"),
    target_event!("addBlankVideoLayer", "New Blank Video Layer", "layer.videoLayers.newBlankVideoLayer"),
    Event {
        code: "",
        name: "insertBlankFrame",
        label: "Insert Blank Frame",
        ids: &["layer.videoLayers.insertBlankFrame"],
        from: |d| super::steps_video::from(d, "layer.videoLayers.insertBlankFrame"),
        to: |_, p| super::steps_video::to(p),
    },
    Event {
        code: "",
        name: "duplicateFrame",
        label: "Duplicate Frame",
        ids: &["layer.videoLayers.duplicateFrame"],
        from: |d| super::steps_video::from(d, "layer.videoLayers.duplicateFrame"),
        to: |_, p| super::steps_video::to(p),
    },
    Event {
        code: "",
        name: "deleteFrame",
        label: "Delete Frame",
        ids: &["layer.videoLayers.deleteFrame"],
        from: |d| super::steps_video::from(d, "layer.videoLayers.deleteFrame"),
        to: |_, p| super::steps_video::to(p),
    },
    target_event!("isolateLayers", "Isolate Layers", "select.isolateLayers"),
    target_event!("textLoadDefaultTypeStyles", "Load Default Type Styles", "type.loadDefaultTypeStyles"),
    target_event!("textSaveDefaultTypeStyles", "Save Default Type Styles", "type.saveDefaultTypeStyles"),
    target_event!("recordMeasurements", "Record Measurements", "image.analysis.recordMeasurements"),
    target_event!("CropPhotosAuto0001", "Crop and Straighten Photos", "file.automate.cropAndStraightenPhotos"),
    Event {
        code: "",
        name: "artboardFromLayerGroupEvent",
        label: "Artboard from Group",
        ids: &["layer.new.artboardFromGroup"],
        from: artboard_from,
        to: artboard_to,
    },
    Event { code: "", name: "editArtboardEvent", label: "Edit Artboard", ids: &["layer.artboard.set"], from: artboard_edit_from, to: artboard_edit_to },
    Event {
        code: "",
        name: "unlinkSelectedLayers",
        label: "Unlink Layers",
        ids: &["layer.linkLayers"],
        from: |d| link_from(d, false),
        to: |_, p| link_to(p, false),
    },
    Event {
        code: "Mk  ",
        name: "make",
        label: "Make",
        ids: &[
            "image.calculations",
            "type.convertToShape",
            "type.createWorkPath",
            "layer.new.layer",
            "layer.layerStyle.createLayer",
            "layer.new.group",
            "layer.groupLayers",
            "layer.duplicate",
            "layer.new.layerViaCopy",
            "layer.new.layerViaCut",
            "layer.newFillLayer.solidColor",
            "layer.newFillLayer.pattern",
            "layer.newFillLayer.gradient",
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
            "history.newSnapshot",
        ],
        from: make_from,
        to: make_to,
    },
    Event {
        code: "setd",
        name: "set",
        label: "Set",
        ids: &[
            super::steps_measurement::SET_SCALE,
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
            "gradient.fill.set",
            "layer.new.layerFromBackground",
            "layer.layerStyle.blendingOptions",
            "layer.smartFilter.blendingOptions",
            "path.set",
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
    Event {
        code: "Dlt ",
        name: "delete",
        label: "Delete",
        ids: &["layer.delete", "edit.clear", "layer.layerMask.delete", "layer.smartFilter.deleteFilterMask"],
        from: delete_from,
        to: delete_to,
    },
    Event { code: "", name: "applyLocking", label: "Apply Locking", ids: &["layer.lockLayers"], from: locking_from, to: locking_to },
    // Timeline › Make Frames from Layers: no frame animation in PhotoCraft, no document change.
    Event {
        code: "",
        name: "animationFramesFromLayers",
        label: "Make Frames from Layers",
        ids: &[],
        from: |_| Mapped::Unsupported { harmless: true },
        to: |_, _| None,
    },
    Event { code: "", name: "clearEvent", label: "Clear", ids: &["select.editInQuickMaskMode", "history.clear"], from: clear_from, to: clear_to },
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
    Event {
        code: "",
        name: "applyImageStackPluginRenderer",
        label: "Stack Mode",
        ids: super::steps_stack::MODE_IDS,
        from: super::steps_stack::mode_from,
        to: super::steps_stack::mode_to,
    },
    Event {
        code: "",
        name: "imageStackConvertSmartObject",
        label: "Stack Mode: None",
        ids: &[super::steps_stack::NONE],
        from: super::steps_stack::none_from,
        to: super::steps_stack::none_to,
    },
    target_event!("placedLayerMakeCopy", "New Smart Object via Copy", "layer.smartObjects.newSmartObjectViaCopy"),
    target_event!("placedLayerConvertToLayers", "Convert to Layers", "layer.smartObjects.convertToLayers"),
    target_event!("placedLayerConvertToEmbedded", "Convert to Embedded", "layer.smartObjects.convertToEmbedded"),
    target_event!("placedLayerUpdateAllModified", "Update All Modified Content", "layer.smartObjects.updateAllModifiedContent"),
    Event {
        code: "",
        name: "placedLayerExportContents",
        label: "Export Contents",
        ids: &["layer.smartObjects.exportContents"],
        from: |d| smart_path_from(d, "layer.smartObjects.exportContents"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerConvertToLinked",
        label: "Convert to Linked",
        ids: &["layer.smartObjects.convertToLinked"],
        from: |d| smart_path_from(d, "layer.smartObjects.convertToLinked"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerRelinkToFile",
        label: "Relink to File",
        ids: &["layer.smartObjects.relinkToFile"],
        from: |d| smart_path_from(d, "layer.smartObjects.relinkToFile"),
        to: smart_path_to,
    },
    Event {
        code: "",
        name: "placedLayerReplaceContents",
        label: "Replace Contents",
        ids: &["layer.smartObjects.replaceContents"],
        from: |d| smart_path_from(d, "layer.smartObjects.replaceContents"),
        to: smart_path_to,
    },
    Event { code: "Hd  ", name: "hide", label: "Hide", ids: &["layer.hideLayers"], from: |d| visibility_from(d, "layer.hideLayers"), to: visibility_to },
    Event {
        code: "Shw ",
        name: "show",
        label: "Show",
        ids: &["layer.showLayers", "layer.showOnly"],
        from: |d| visibility_from(d, "layer.showLayers"),
        to: visibility_to,
    },
    Event {
        code: "move",
        name: "move",
        label: "Move",
        ids: &[
            "layer.translate",
            "layer.moveTo",
            "layer.arrange.bringToFront",
            "layer.arrange.sendToBack",
            "layer.arrange.bringForward",
            "layer.arrange.sendBackward",
        ],
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
    Event {
        code: "",
        name: "pasteEffects",
        label: "Paste Layer Style",
        ids: &["layer.layerStyle.pasteLayerStyle"],
        from: paste_effects_from,
        to: paste_effects_to,
    },
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
    Event {
        code: "",
        name: "perspectiveWarpTransform",
        label: "Perspective Warp",
        ids: &[crate::distort_cmds::PERSPECTIVE],
        from: super::steps_perspective_warp::from,
        to: super::steps_perspective_warp::to,
    },
    Event {
        code: "Grdn",
        name: "gradientClassEvent",
        label: "Gradient",
        ids: &["paint.gradient"],
        from: super::steps_gradient::from,
        to: super::steps_gradient::to,
    },
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
    Event { code: "", name: "spellCheck", label: "Check Spelling", ids: &["edit.checkSpelling"], from: spell_check_from, to: spell_check_to },
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
    Event { code: "", name: "cafWorkspace", label: "Content-Aware Fill", ids: &["edit.contentAwareFill"], from: caf_from, to: caf_to },
    // File and actions.
    Event { code: "Cls ", name: "close", label: "Close", ids: &["file.close"], from: close_from, to: close_to },
    Event { code: "Expr", name: "export", label: "Export", ids: &["file.export.saveForWebLegacy"], from: export_from, to: export_to },
    Event { code: "Plc ", name: "placeEvent", label: "Place", ids: &["file.placeEmbedded", "file.placeLinked"], from: place_from, to: place_to },
    Event { code: "Opn ", name: "open", label: "Open", ids: &["file.openAs"], from: open_from, to: open_to },
    Event { code: "", name: "conditional", label: "If", ids: &["actions.conditional"], from: conditional_from, to: conditional_to },
    Event { code: "save", name: "save", label: "Save", ids: &["file.save", "file.saveAs", "file.saveACopy"], from: save_from, to: save_to },
    ev!("Rvrt", "revert", "Revert", "file.revert", &[]),
    Event { code: "Ply ", name: "play", label: "Play", ids: &["actions.play"], from: play_from, to: play_to },
    Event {
        code: "Btch",
        name: "batch",
        label: "Batch",
        ids: &["file.automate.batch"],
        from: super::steps_batch::batch_from,
        to: super::steps_batch::batch_to,
    },
    // Create Droplet (#10710): Photoshop has no string ID for it.
    Event {
        code: "CrtD",
        name: "CrtD",
        label: "Create Droplet",
        ids: &["file.automate.createDroplet"],
        from: super::steps_droplet::droplet_from,
        to: super::steps_droplet::droplet_to,
    },
    Event { code: "", name: "applyComp", label: "Apply Layer Comp", ids: &["layerComp.apply"], from: apply_comp_from, to: apply_comp_to },
    Event { code: "Stop", name: "stop", label: "Stop", ids: &[STOP_MESSAGE], from: stop_from, to: stop_to },
];

/// Four-char codes PhotoCraft once exported for events Photoshop only runs by name (or by
/// another code), with the name they stand for. Measured in Photoshop: `executeAction` with
/// these codes fails ("the command is not currently available"), and `Fltn` is the
/// `flatness` property, not the flatten event. Import keeps accepting them so actions saved
/// before still load; export never writes them.
const LEGACY_CODES: &[(&str, &str)] = &[
    ("BoxB", "boxblur"),
    ("SrfB", "surfaceBlur"),
    ("Fltn", "flattenImage"),
    ("BlnW", "blackAndWhite"),
    ("PhtF", "photoFilter"),
    ("Vbrn", "vibrance"),
    ("GdMp", "gradientMapEvent"),
];

/// Older string names a step can carry for an event that is listed under another name: the
/// reference application reports `IntW` and `Dstr` as `interfaceWhite` and `distort` at run time.
const NAME_ALIASES: &[(&str, &str)] = &[("interfaceWhite", "intersectWith"), ("distort", "distribute")];

/// Match populated event identifiers, retaining legacy code aliases on import.
pub(super) fn event_for(item: &AtnEvent) -> Option<&'static Event> {
    let id = item.id();
    let by_name = |name: &str| EVENTS.iter().find(|e| !e.name.is_empty() && e.name == name);
    let found = match item {
        AtnEvent::Code(_) => EVENTS.iter().find(|e| !e.code.is_empty() && e.code.trim_end() == id.trim_end()),
        AtnEvent::Name(_) => by_name(&id),
    };
    found.or_else(|| match item {
        // An effect run from its own menu item records its id as the event (#10592).
        // By 4-char code or, as newer versions record it, by string ID (`cutout`).
        _ if super::steps_gallery::is_effect(item) => by_name("filterGallery"),
        AtnEvent::Code(_) => LEGACY_CODES.iter().find(|(code, _)| *code == id).and_then(|(_, name)| by_name(name)),
        AtnEvent::Name(_) => NAME_ALIASES.iter().find(|(old, _)| *old == id).and_then(|(_, name)| by_name(name)),
    })
}

/// The event `id` exports as: the first event listing it whose converter accepts the params
/// (Quick Mask on is a `set`, off a `clearEvent`).
pub(super) fn event_and_descriptor(id: &str, params: &Value) -> Option<(&'static Event, Descriptor)> {
    if params.get("_atnBounds").is_some()
        && (id != "select.rect" || !matches!(params.get("mode").and_then(Value::as_str), Some("add" | "subtract" | "intersect")))
    {
        return None;
    }
    // A string names the recorded event of a mask or merge step; an object is a Filter Gallery
    // effect's own event, which `export_item` checks (`steps_gallery::recorded_event`).
    let gallery_event = id.starts_with("filter.gallery.") && params.get("_atnEvent").is_some_and(Value::is_object);
    if let Some(origin) = params.get("_atnEvent").filter(|_| !gallery_event) {
        let origin = origin.as_str()?;
        if !matches!((id, origin), ("layer.layerMask.delete", "RmvL") | ("layer.mergeLayers", "MrgL")) {
            return None;
        }
        return EVENTS.iter().find(|e| e.code == origin).and_then(|e| (e.to)(id, params).map(|d| (e, d)));
    }
    EVENTS.iter().filter(|e| e.ids.contains(&id)).find_map(|e| (e.to)(id, params).map(|d| (e, d)))
}
