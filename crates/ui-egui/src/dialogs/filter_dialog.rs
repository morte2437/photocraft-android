//! Filter dialogs generated from engine command parameter specs, with live on-canvas preview and an
//! in-dialog preview window (#3769).
//!
//! The preview runs the *same engine command* on the downsampled proxy document (pixel-sized
//! parameters scaled by the proxy factor), so what you preview is what you get.

use std::sync::Arc;

use photocraft_doc::Document;
use photocraft_engine::color_cmds::ColorSettings;
use photocraft_engine::installed_profiles::InstalledProfile;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

mod zoom_center;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Range {
        min: f32,
        max: f32,
        default: f32,
    },
    Choice {
        options: Vec<String>,
        default: Option<String>,
    },
    Bool(bool),
    Int {
        default: i64,
    },
    /// Free text (e.g. a file path).
    Text,
    /// One of the open documents (stored as its index).
    Document,
    /// A row-major grid of integers (`int[25]` = 5×5).
    Grid(usize),
    /// Structured JSON (pins, curve points…): settable through the command, not shown in the dialog.
    Json,
    /// A `"#rrggbb"` colour (`color`, or `color=#rrggbb` with a default), picked with a swatch.
    /// Without a default the param stays unset until a colour is picked.
    Color(Option<[f32; 3]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub kind: Kind,
}

/// Parse the registry's parameter notation, e.g.
/// `{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}`.
pub fn parse_spec(spec: &str) -> Vec<Param> {
    let inner = object_body(spec);
    let mut out = Vec::new();
    // Split on commas that start a new `"key":` (not inside strings or brackets).
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut depth = 0i32;
    for ch in inner.chars() {
        if ch == '"' {
            in_str = !in_str;
        }
        if !in_str {
            match ch {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if ch == ',' && !in_str && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    for part in parts {
        let Some((k, v)) = part.split_once(':') else { continue };
        let key = k.trim().trim_matches('"').to_string();
        if key.is_empty() || key == "layer" {
            continue;
        }
        let v = v.trim();
        let kind = if let Some(choices) = v.strip_prefix('"') {
            let (body, rest) = choices.split_once('"').unwrap_or((choices, ""));
            let options: Vec<String> = body.split('|').map(str::to_string).collect();
            let default = if let Some((_, def_part)) = rest.split_once('=') {
                let trimmed = def_part.trim();
                let def = if let Some(unquoted) = trimmed.strip_prefix('"') {
                    unquoted.split('"').next().unwrap_or("")
                } else if let Some(unquoted) = trimmed.strip_prefix('\'') {
                    unquoted.split('\'').next().unwrap_or("")
                } else {
                    trimmed.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == '}').next().unwrap_or("")
                };
                if !def.is_empty() && options.iter().any(|opt| opt == def) { Some(def.to_string()) } else { None }
            } else {
                None
            };
            Kind::Choice { options, default }
        } else if let Some(rest) = v.strip_prefix("bool") {
            Kind::Bool(rest.trim_start_matches('=').trim() == "true")
        } else if v == "text" {
            Kind::Text
        } else if v == "color" {
            Kind::Color(None)
        } else if let Some(default) = v.strip_prefix("color=") {
            Kind::Color(crate::color_picker_ui::parse_hex(default))
        } else if v == "doc" {
            Kind::Document
        } else if v == "json" || v.starts_with("layer id") || v.starts_with('[') || v.starts_with('{') || v.trim_end_matches('?') == "index|name" {
            // `index|name`: a reference (an alpha channel…) the dialog sets through another control.
            Kind::Json
        } else if let Some(n) = v.strip_prefix("int[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse().ok()) {
            Kind::Grid(n)
        } else if let Some((range, default)) = v.split_once('=').map(|(a, b)| (a, b.trim())).or(Some((v, ""))) {
            if let Some((lo, hi)) = range.split_once("..") {
                let min = leading_number(lo).unwrap_or(0.0);
                let max = leading_number(hi).unwrap_or(100.0);
                let default = default.parse().unwrap_or(min);
                Kind::Range { min, max, default }
            } else {
                Kind::Int { default: default.parse().unwrap_or(0) }
            }
        } else {
            continue;
        };
        out.push(Param { key, kind });
    }
    out
}

/// The number a range bound starts with, ignoring a unit or marker after it: `5000 px?` → 5000,
/// `1000 %` → 1000. A bound that is not a number at all is `None`.
fn leading_number(bound: &str) -> Option<f32> {
    let b = bound.trim();
    if let Ok(v) = b.parse() {
        return Some(v);
    }
    let end = b.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(b.len());
    b.get(..end)?.parse().ok()
}

/// The inside of the spec's leading `{…}` object. Notes after it, like `→ {…}` results or
/// `(per-range [c,m,y,k] arrays: …)`, are documentation: read as parameters, their commas split
/// off junk fields and glued the note onto the last parameter (Selective Color's `"reds":json`
/// became a number field, and its OK then failed).
fn object_body(spec: &str) -> &str {
    let s = spec.trim();
    let Some(body) = s.strip_prefix('{') else { return s };
    let (mut depth, mut in_str) = (0i32, false);
    for (i, ch) in body.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '[' | '{' if !in_str => depth += 1,
            ']' | '}' if !in_str => {
                if depth == 0 {
                    return body.get(..i).unwrap_or(body);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    body.trim_end_matches('}')
}

/// Parameter keys measured in pixels (scaled for proxy previews).
fn is_pixel_param(key: &str) -> bool {
    matches!(
        key,
        "radius"
            | "distance"
            | "cellSize"
            | "horizontal"
            | "vertical"
            | "height"
            | "wavelengthMin"
            | "wavelengthMax"
            | "amplitudeMin"
            | "amplitudeMax"
            | "maxRadius"
            | "size"
            | "blur"
            | "speed"
            | "shadowRadius"
            | "highlightRadius"
    )
}

/// [`is_pixel_param`] for `key` of `command`. Some commands use those names for relative values:
/// Tree's and Picture Frame's `size` (a fraction of the canvas, an ornament scale) and Lens
/// Correction's perspective `horizontal` / `vertical` (-100..100). Scaled by the proxy factor,
/// their preview drew a quarter-size tree and a quarter of the perspective (#2063).
fn is_pixel_param_of(command: &str, key: &str) -> bool {
    is_pixel_param(key)
        && !matches!((command, key), ("filter.render.tree" | "filter.render.pictureFrame", "size") | ("filter.lensCorrection", "horizontal" | "vertical"))
}

/// Filters whose features have a fixed size in document pixels that no dialog parameter scales:
/// per-pixel noise and dots (Add Noise, Diffuse, Mezzotint, Fibers, and Dust & Scratches, whose
/// specks a proxy averages away — so the preview would show nothing removed at all, #3265 — while
/// its radius stays in document pixels), pixel kernels and scan lines
/// (Custom, Reduce Noise, De-Interlace, Trace Contour's one-pixel lines), fixed lengths and
/// widths (Ripple, Oil Paint, Wind's streaks, Extrude's depth, Flame's length, width and
/// interval) and Tree, whose branches are built from 6 px segments. On a reduced proxy those
/// features come out k times too large (or a different tree grows), so these preview at full
/// resolution: what the preview shows is what OK applies, as in Photoshop (#2063).
///
/// Sharpen, Sharpen More, Sharpen Edges, Find Edges, Emboss and Despeckle use fixed one-pixel
/// kernels with no size parameter, so a proxy changes their look: edges average away, making
/// the sharpening and edge detection invisible. These must preview at full resolution (#3929).
///
/// Content-Aware Fill searches for matching patches, and the matches found on a reduced copy
/// differ from those found at full size, so its preview would not show what OK writes (#6530).
const FULL_RESOLUTION_PREVIEW: &[&str] = &[
    "filter.distort.ripple",
    "filter.stylize.diffuse",
    "filter.stylize.oilPaint",
    "filter.render.flame",
    "filter.render.fibers",
    "filter.pixelate.mezzotint",
    "filter.noise.addNoise",
    "filter.noise.dustAndScratches",
    "filter.noise.reduceNoise",
    "filter.other.custom",
    "filter.video.deInterlace",
    "filter.stylize.traceContour",
    "filter.stylize.wind",
    "filter.stylize.extrude",
    "filter.render.tree",
    "filter.sharpen.sharpen",
    "filter.sharpen.sharpenMore",
    "filter.sharpen.sharpenEdges",
    "filter.stylize.findEdges",
    "filter.stylize.emboss",
    "filter.noise.despeckle",
    "edit.contentAwareFill",
];

/// Whether `command` with `params` previews at full resolution: [`FULL_RESOLUTION_PREVIEW`], every
/// Filter Gallery filter (its strokes, grain and textures are sized in pixels; the settings are
/// abstract levels, not lengths) and the dithered colour-mode conversions (Indexed Color, Bitmap).
fn full_resolution_preview(command: &str, params: &Value) -> bool {
    let choice = |key: &str| params.get(key).and_then(Value::as_str);
    FULL_RESOLUTION_PREVIEW.contains(&command)
        || command.starts_with("filter.gallery.")
        || (command == "image.mode.indexedColor" && choice("dither") != Some("none"))
        || (command == "image.mode.bitmap" && !matches!(choice("method"), Some("threshold" | "threshold50")))
}

/// Proxy factor for a live preview of `command` with `params`, given the document's reduced factor
/// `k`: 1 for [`full_resolution_preview`] commands, otherwise the largest factor up to `k` at which
/// no pixel-sized parameter, divided by it, drops below the command's minimum. A clamped value
/// would change the feature size: Pointillize's 5 px cells at k = 4 became the 3 px minimum, i.e.
/// 12 px cells in the preview (#2063).
pub fn preview_factor(command: &str, params: &Value, k: u32) -> u32 {
    if k <= 1 || full_resolution_preview(command, params) {
        return 1;
    }
    // A plug-in's pixel lengths are the params its manifest marks `"unit": "px"` (#7524).
    if command == photocraft_engine::plugin_cmds::RUN {
        return photocraft_engine::plugin_cmds::preview_factor(params, k);
    }
    let Some(spec) = photocraft_engine::commands::find(command) else { return k };
    let k = parse_spec(spec.params).into_iter().fold(k, |k, p| match p.kind {
        Kind::Range { min, .. } if min > 0.0 && is_pixel_param_of(command, &p.key) => match params.get(&p.key).and_then(Value::as_f64) {
            Some(v) if v.is_finite() => k.min((v / f64::from(min)).floor().clamp(1.0, f64::from(k)) as u32),
            _ => k,
        },
        _ => k,
    });
    // Mosaic rounds its cells to whole pixels: 10 px cells at k = 4 became 3 proxy px = 12 px.
    // Divide the cell size evenly instead.
    match params.get("cellSize").and_then(Value::as_f64) {
        Some(v) if command == "filter.pixelate.mosaic" && v.is_finite() && v >= 1.0 => {
            let cell = v.round().min(f64::from(u32::MAX)) as u32;
            (1..=k).rev().find(|d| cell.is_multiple_of(*d)).unwrap_or(1)
        }
        _ => k,
    }
}

/// Commands outside `filter.*` that get the schema dialog *with* live preview.
pub const PREVIEWED: &[&str] = &[
    "image.adjustments.selectiveColor",
    "image.adjustments.colorLookup",
    "image.adjustments.shadowsHighlights",
    "image.adjustments.replaceColor",
    "image.adjustments.matchColor",
    "image.adjustments.hdrToning",
    "image.rotation.arbitrary",
    "image.mode.indexedColor",
    "image.mode.bitmap",
    "image.mode.duotone",
    "layer.matting.defringe",
    "layer.matting.colorDecontaminate",
    "layer.layerStyle.scaleEffects",
    "type.warpText",
    // The preview shows the image through the chosen profile, as Photoshop's Preview does.
    "edit.assignProfile",
    "edit.convertToProfile",
];

pub fn has_dialog(command: &str) -> bool {
    // The Filter Gallery has its own full-window dialog (gallery_ui).
    if command == "filter.filterGallery" {
        return false;
    }
    (command.starts_with("filter.")
        || command.starts_with("select.modify.")
        || PREVIEWED.contains(&command)
        || matches!(
            command,
            "image.trim"
                | "select.refineEdge"
                | "view.proofSetup"
                | "layer.layerStyle.globalLight"
                | "image.mode.colorTable"
                | "edit.definePattern"
                | "select.convertToShape"
        ))
        && photocraft_engine::commands::find(command).is_some_and(|c| !parse_spec(c.params).is_empty())
}

/// Keep only portable, schema-valid choices. Never retain document ids, free-form
/// paths, raw JSON, or values that a newer command version no longer accepts.
fn rememberable(kind: &Kind, value: &Value) -> bool {
    match kind {
        Kind::Range { min, max, .. } => value.as_f64().is_some_and(|n| n.is_finite() && n >= f64::from(*min) && n <= f64::from(*max)),
        Kind::Choice { options, .. } => value.as_str().is_some_and(|v| options.iter().any(|choice| choice == v)),
        Kind::Bool(_) => value.is_boolean(),
        Kind::Int { .. } => value.as_i64().is_some(),
        _ => false,
    }
}

fn restore_remembered(app: &PhotocraftApp, command: &str, spec: &str, fields: &mut Map<String, Value>) {
    let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(command) else { return };
    for p in parse_spec(spec) {
        if let Some(v) = saved.get(&p.key).filter(|v| rememberable(&p.kind, v)) {
            fields.insert(p.key, v.clone());
        }
    }
}

/// Remember successful built-in schema dialogs; failed or cancelled dialogs do not persist.
pub(crate) fn remember(app: &mut PhotocraftApp, command: &str, fields: &Map<String, Value>) {
    if !fields.contains_key("__filter") || fields.contains_key("__spec") || fields.contains_key("__smartFilter") {
        return;
    }
    let Some(spec) = photocraft_engine::commands::find(command) else { return };
    let saved: Map<String, Value> =
        parse_spec(spec.params).into_iter().filter_map(|p| fields.get(&p.key).filter(|v| rememberable(&p.kind, v)).map(|v| (p.key, v.clone()))).collect();
    if !saved.is_empty() {
        app.session.prefs.edit(|prefs| prefs.dialogs.insert(command.into(), Value::Object(saved)));
    }
}

/// Schema-default values for one param of `command` (open and Alt+Reset share this).
fn default_value(app: &PhotocraftApp, command: &str, p: &Param) -> Option<Value> {
    Some(match &p.kind {
        Kind::Range { default, .. } if command == "image.mode.indexedColor" && p.key == "colors" => json!(default.round().clamp(2.0, 256.0) as u32),
        Kind::Range { default, .. } => json!(default),
        Kind::Choice { options, default } => {
            let initial = default.as_deref().or_else(|| options.first().map(String::as_str)).unwrap_or("");
            json!(initial)
        }
        Kind::Bool(default) => json!(default),
        Kind::Int { default } => json!(default),
        Kind::Text => json!(""),
        Kind::Document => json!(-1),
        // Identity kernel: 1 in the centre.
        Kind::Grid(n) => json!((0..*n).map(|i| i64::from(i == *n / 2)).collect::<Vec<_>>()),
        Kind::Color(Some(rgb)) => json!(crate::color_picker_ui::hex(*rgb)),
        // Replace Color starts from the foreground colour and can sample a new one off the canvas.
        Kind::Color(None) if command == REPLACE_COLOR => {
            let fg = app.session.tools.foreground;
            json!(crate::color_picker_ui::hex([fg[0], fg[1], fg[2]]))
        }
        Kind::Color(None) => return None,
        // Colour inputs come from the current swatches, so the proxy preview matches the result.
        Kind::Json if p.key == "foreground" => json!(app.session.tools.foreground),
        Kind::Json if p.key == "background" => json!(app.session.tools.background),
        Kind::Json => return None,
    })
}

/// Command-specific open defaults (ruler straighten, warp from layer, …), not remembered prefs.
fn apply_command_defaults(app: &PhotocraftApp, command: &str, fields: &mut Map<String, Value>) {
    if command == "image.rotation.arbitrary" {
        straighten_defaults(app, fields);
    }
    if command == "type.warpText" {
        warp_defaults(app, fields);
    }
    // Photoshop's Pattern Name dialog starts from the name the pattern would get anyway.
    if command == "edit.definePattern" {
        fields.insert("name".into(), json!(photocraft_engine::pattern_cmds::default_name(&app.session)));
    }
    if command == "layer.layerStyle.globalLight" {
        global_light_defaults(app, fields);
    }
    if command == "view.proofSetup" {
        proof_setup_defaults(app, fields);
    }
    if command == "edit.assignProfile" {
        assign_profile_defaults(app, fields);
    }
    if command == crate::bitmap_dialog::COMMAND {
        crate::bitmap_dialog::defaults(app, fields);
    }
    // Photoshop's Replace Color opens with its sample eyedropper active (#6279): the first click
    // on the image picks the colour.
    if command == REPLACE_COLOR {
        fields.insert(COLOR_EYEDROPPER.into(), json!(true));
    }
}

/// Assign Profile offers only the profiles the document's colour mode can take (#4951): the
/// command refuses one of another colour space. `__profiles` lists them for the `profile` menu,
/// and a remembered choice that doesn't fit this document falls back to the working space.
fn assign_profile_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    let Some(spec) = photocraft_engine::commands::find("edit.assignProfile") else { return };
    let space = photocraft_engine::color_cmds::mode_space(st.doc.mode);
    let fits = |name: &str| name == "none" || app.session.color.resolve(name, Some(&st.doc), None).is_ok_and(|p| p.color_space == space);
    let builtin: Vec<String> = parse_spec(spec.params)
        .into_iter()
        .filter(|p| p.key == "profile")
        .flat_map(|p| match p.kind {
            Kind::Choice { options, .. } => options,
            _ => Vec::new(),
        })
        .collect();
    let installed: Vec<_> = photocraft_engine::installed_profiles::installed().into_iter().filter(|i| i.is_destination()).collect();
    let allowed: Vec<String> =
        builtin.iter().filter(|name| fits(name)).cloned().chain(installed.iter().filter(|i| i.color_space == space).map(|i| i.path.clone())).collect();
    // A path nobody lists (typed by an agent) is left alone: the command decides about it.
    let listed = |cur: &str| builtin.iter().any(|name| name == cur) || installed.iter().any(|i| i.path == cur);
    if fields.get("profile").and_then(Value::as_str).is_some_and(|cur| listed(cur) && !allowed.iter().any(|name| name == cur)) {
        fields.insert("profile".into(), json!("working"));
    }
    fields.insert("__profiles".into(), json!(allowed));
}

/// Alt+Cancel (Reset): schema defaults + command hooks, without last-used remembered prefs.
pub fn reset_fields(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    if !fields.contains_key("__filter") || fields.contains_key("__adjust") {
        return;
    }
    let Some(command) = fields.get("__command").and_then(Value::as_str).map(str::to_owned) else { return };
    let spec =
        fields.get("__spec").and_then(Value::as_str).map(str::to_owned).or_else(|| photocraft_engine::commands::find(&command).map(|c| c.params.to_owned()));
    let Some(spec) = spec else { return };
    // Drop user params; keep private `__` chrome (and fixed keys outside the schema for `__spec`).
    let schema_keys: std::collections::HashSet<String> = parse_spec(&spec).into_iter().map(|p| p.key).collect();
    fields.retain(|k, _| k.starts_with("__") || !schema_keys.contains(k));
    for p in parse_spec(&spec) {
        if let Some(v) = default_value(app, &command, &p) {
            fields.insert(p.key, v);
        }
    }
    apply_command_defaults(app, &command, fields);
    // The schema has no default for the Blur Gallery's pins: start again from the opening one.
    if crate::blur_pins::handles(&command) {
        crate::blur_pins::reset(fields);
    }
}

pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let spec = photocraft_engine::commands::find(command)?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__filter".into(), json!(true));
    // Select and Mask previews in its view modes (`select_mask_view`), not as the command's result.
    if command.starts_with("filter.") || PREVIEWED.contains(&command) || command == crate::select_mask_view::COMMAND {
        fields.insert("__preview".into(), json!(true));
    }
    for p in parse_spec(spec.params) {
        if let Some(v) = default_value(app, command, &p) {
            fields.insert(p.key, v);
        }
    }
    restore_remembered(app, command, spec.params, &mut fields);
    apply_command_defaults(app, command, &mut fields);
    if crate::blur_pins::handles(command) {
        crate::blur_pins::init(app, &mut fields);
    }
    if command == LENS_BLUR {
        lens_blur_channels(app, &mut fields);
    }
    if parse_spec(spec.params).iter().any(|p| p.kind == Kind::Document) {
        // The document picker lists every open document (params refer to them by index).
        let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        fields.insert("__docs".into(), json!(names));
    }
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields);
    Some(id)
}

/// Arbitrary rotation starts at the angle that straightens the ruler line, when there is one.
fn straighten_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(r) = app.session.active().and_then(|d| d.doc.measurement.ruler) else { return };
    let rot = photocraft_engine::analysis_cmds::straighten_angle(&r);
    // A ruler read from a damaged file could hold non-finite ends: keep the 0° default then.
    if !rot.is_finite() {
        return;
    }
    fields.insert("angle".into(), json!(rot.abs()));
    fields.insert("direction".into(), json!(if rot < 0.0 { "ccw" } else { "cw" }));
}

/// Warp Text edits the type layers it opened on (the selected ones, or the one being edited) and
/// starts from the warp the shown layer has, or None, never from remembered values (#218).
fn warp_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    if let Some(Value::Object(targets)) = crate::type_tool::formatting_params(app) {
        fields.extend(targets);
    }
    let warp = crate::type_tool::target_text(app).and_then(|t| t.warp.clone());
    let w = warp.unwrap_or(photocraft_doc::text::TextWarp {
        style: "warpNone".into(),
        value: 50.0,
        horizontal_distortion: 0.0,
        vertical_distortion: 0.0,
        horizontal: true,
    });
    // A style without a short id (from a PSD) shows as None, with the layer's values.
    fields.insert("style".into(), json!(photocraft_text::warp::short_style(&w.style).unwrap_or("none")));
    fields.insert("bend".into(), json!(w.value));
    fields.insert("horizontalDistortion".into(), json!(w.horizontal_distortion));
    fields.insert("verticalDistortion".into(), json!(w.vertical_distortion));
    fields.insert("orientation".into(), json!(if w.horizontal { "horizontal" } else { "vertical" }));
}

/// Global Light reflects the active document's current shared lighting, never remembered values.
fn global_light_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    fields.insert("angle".into(), json!(st.doc.global_light.angle));
    fields.insert("altitude".into(), json!(st.doc.global_light.altitude));
}

/// Proof Setup reflects the active document's current proofing condition, never remembered values.
fn proof_setup_defaults(app: &PhotocraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    let pv = app.session.color.proof(st.doc.id);
    fields.insert("profile".into(), json!(pv.setup.name));
    fields.insert("intent".into(), json!(pv.setup.intent.id()));
    fields.insert("bpc".into(), json!(pv.setup.bpc));
    fields.insert("simulatePaper".into(), json!(pv.setup.simulate_paper));
}

/// A filter dialog with live preview for `command` whose parameters follow `spec` (registry
/// notation) instead of the command's own; `fixed` params (e.g. a plug-in id) are passed through.
pub fn open_with_spec(app: &mut PhotocraftApp, command: &str, label: &str, spec: &str, fixed: Map<String, Value>) -> u64 {
    let mut fields = fixed;
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    fields.insert("__spec".into(), json!(spec));
    for p in parse_spec(spec) {
        let v = match &p.kind {
            Kind::Range { default, .. } => json!(default),
            Kind::Choice { options, default } => {
                let initial = default.as_deref().or_else(|| options.first().map(String::as_str)).unwrap_or("");
                json!(initial)
            }
            Kind::Bool(default) => json!(default),
            Kind::Int { default } => json!(default),
            Kind::Color(Some(rgb)) => json!(crate::color_picker_ui::hex(*rgb)),
            _ => continue,
        };
        fields.insert(p.key, v);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, fields)
}

pub(crate) fn label(key: &str) -> String {
    tl!(&source_label(key)).to_owned()
}

pub(crate) fn source_label(key: &str) -> String {
    // Photoshop's name for the localized colour matching (Replace Color, Color Range).
    if key == "localized" {
        return "Localized Color Clusters".into();
    }
    // Convert to Profile's, Proof Setup's…: spelled out, as in Photoshop (not "Bpc").
    if key == "bpc" {
        return "Black Point Compensation".into();
    }
    // A name that already reads as text ("iPhone 14 Pro Max", "Web 1920") is not camelCase.
    if key.contains(' ') {
        return key.to_string();
    }
    // camelCase → "Camel Case"
    let mut s = String::new();
    for (i, ch) in key.chars().enumerate() {
        if i == 0 {
            s.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            s.push(' ');
            s.push(ch);
        } else {
            s.push(ch);
        }
    }
    s
}

fn choice_label(v: &str) -> String {
    label(v)
}

fn uses_logarithmic_slider(min: f32, max: f32) -> bool {
    min > 0.0 && max / min > 500.0
}

const LENS_BLUR: &str = "filter.blur.lensBlur";

/// Lens Blur's Depth Map Source lists the document's alpha channels after None, Transparency
/// and Layer Mask, as in Photoshop (#7400): their names go to `__channels`. A `channel` source
/// without a channel this document has (references aren't remembered) opens as None.
fn lens_blur_channels(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    let names: Vec<String> = app.session.active().map(|d| d.doc.channels.iter().map(|c| c.name.clone()).collect()).unwrap_or_default();
    let known = f.get("depthChannel").and_then(|v| channel_index(v, &names)).is_some();
    if f.get("depthMap").and_then(Value::as_str) == Some("channel") && !known {
        f.insert("depthMap".into(), json!("none"));
        f.remove("depthChannel");
    }
    f.insert("__channels".into(), json!(names));
}

/// Lens Blur's Source menu entry for the alpha channel at `index`. By index, not name: two
/// channels may share a name, and each must be choosable.
fn channel_choice(index: usize) -> String {
    format!("channel:{index}")
}

/// The alpha channel `v` names (an index, or a name: the first channel so named), as the
/// engine reads `depthChannel`.
fn channel_index(v: &Value, names: &[String]) -> Option<usize> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < names.len()),
        Value::String(name) => names.iter().position(|n| n == name),
        _ => None,
    }
}

/// Schema dialogs whose `profile` menu lists the installed profiles after the built-ins.
const PROFILE_DIALOGS: &[&str] = &["edit.assignProfile", "edit.convertToProfile", "view.proofSetup"];

/// Which installed profiles a menu of `cmd` lists after its built-ins, if it lists any: every
/// destination profile for the `profile` menus of [`PROFILE_DIALOGS`], and for Color Settings'
/// working spaces those of that space, as Photoshop lists them (#6323). An RGB working space is a
/// display or colour-space profile, not a scanner's or a printer's.
fn installed_filter(cmd: &str, key: &str) -> Option<fn(&InstalledProfile) -> bool> {
    use photocraft_cms::{ColorSpace, ProfileClass};
    match (cmd, key) {
        (c, "profile") if PROFILE_DIALOGS.contains(&c) => Some(|_| true),
        ("edit.colorSettings", "workingRgb") => {
            Some(|i| i.color_space == ColorSpace::Rgb && matches!(i.class, ProfileClass::Display | ProfileClass::ColorSpace))
        }
        ("edit.colorSettings", "workingCmyk") => Some(|i| i.color_space == ColorSpace::Cmyk),
        ("edit.colorSettings", "workingGray") => Some(|i| i.color_space == ColorSpace::Gray),
        _ => None,
    }
}

/// The `installed` profiles a menu of `cmd` lists (see [`installed_filter`]), by path and description.
fn installed_choices(cmd: &str, key: &str, installed: &[InstalledProfile]) -> Vec<(String, String)> {
    let Some(fits) = installed_filter(cmd, key) else { return Vec::new() };
    installed.iter().filter(|i| i.is_destination() && fits(i)).map(|i| (i.path.clone(), i.description.clone())).collect()
}

/// Commands whose `angle` is a direction in document angles (counter-clockwise from three
/// o'clock): a light or a blur direction, edited on a dial as in Photoshop's Motion Blur, Emboss
/// and Global Light (#3152). Rotation amounts (Twirl, Rotate Arbitrary, Crop, Lens Correction,
/// the Blur Gallery shapes) and angles with another zero (Flame's points up) keep the slider.
const ANGLE_DIAL: &[&str] = &[
    "filter.blur.motionBlur",
    "filter.stylize.emboss",
    "filter.stylize.oilPaint",
    "filter.render.lightingEffects",
    "filter.render.relight",
    "layer.layerStyle.globalLight",
];

fn uses_angle_dial(command: &str, key: &str) -> bool {
    key == "angle" && ANGLE_DIAL.contains(&command)
}

/// Replace Color (#4143): the dialog's "From" colour, sampled from the image with a canvas
/// eyedropper like Photoshop's. While armed, the next click on the image writes the merged
/// composite colour into the `color` field (the same sampler the Color Picker uses).
const REPLACE_COLOR: &str = "image.adjustments.replaceColor";
/// What OK on an edited smart filter runs (`smart_ui::confirm_command`).
const SMART_FILTER_SET_PARAMS: &str = "layer.smartFilter.setParams";
const COLOR_EYEDROPPER: &str = "__colorEyedropper";

fn color_eyedropper_owns(f: &Map<String, Value>) -> bool {
    f.get("__filter").and_then(Value::as_bool) == Some(true)
        && f.get("__command").and_then(Value::as_str) == Some(REPLACE_COLOR)
        && f.get(COLOR_EYEDROPPER).and_then(Value::as_bool) == Some(true)
}

/// Whether the top dialog is a Replace Color with its canvas eyedropper armed.
pub(crate) fn color_eyedropper_armed(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| color_eyedropper_owns(&d.fields))
}

/// Sample the merged image at document point (x, y) into the armed dialog's "From" colour.
/// Returns true when the click was consumed, even off-image or over transparency, where nothing
/// changes and the eyedropper stays armed (like the Curves picker).
pub(crate) fn color_eyedropper_sample(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some(id) = app.ui.dialogs.last().filter(|d| color_eyedropper_owns(&d.fields)).map(|d| d.id) else {
        return false;
    };
    if let Some(rgb) = crate::canvas::composite_color(app, x, y)
        && let Some(d) = app.ui.dialog_mut(id)
    {
        d.fields.insert("color".into(), json!(crate::color_picker_ui::hex(rgb)));
        // Where it was picked: Localized Color Clusters match the colour near this point (#4457).
        d.fields.insert("points".into(), json!([[x.floor(), y.floor()]]));
    }
    true
}

/// The Color Picker of Replace Color's Result swatch writes here (#6278).
const RESULT_PICKED: &str = "__resultPicked";
/// The Result the picker opened on: a picked colour that differs from it sets the sliders.
const RESULT_FROM: &str = "__resultFrom";

/// Replace Color's Result swatch (#6278), as in Photoshop: the sampled colour after the Hue,
/// Saturation and Lightness shift. Picking a Result in its Color Picker sets the three sliders so
/// the sampled colour becomes it.
fn replace_color_result(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    use photocraft_algo::tone;
    let t = Tokens::get(ui.ctx());
    let from = f.get("color").and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex).unwrap_or([0.0; 3]);
    let picked = f.get(RESULT_PICKED).and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex);
    if let Some(to) = picked
        && f.get(RESULT_PICKED) != f.get(RESULT_FROM)
    {
        let (hue, saturation, lightness) = tone::replace_color_shift(from, to);
        for (key, v) in [("hue", hue), ("saturation", saturation), ("lightness", lightness)] {
            f.insert(key.into(), json!(v.round()));
        }
        f.remove(RESULT_PICKED);
        f.remove(RESULT_FROM);
    }
    let num = |key: &str| f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let rgb = tone::replace_color_result(from, num("hue"), num("saturation"), num("lightness"));
    let bytes = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let name = tl!("Result");
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(bytes[0], bytes[1], bytes[2]), name).clicked();
            ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
        });
    });
    if clicked {
        let hex = json!(crate::color_picker_ui::hex(rgb));
        f.insert(RESULT_PICKED.into(), hex.clone());
        f.insert(RESULT_FROM.into(), hex);
        crate::color_picker_ui::request_field(f, RESULT_PICKED, &crate::color_picker_ui::title_for(name));
    }
}

/// Dialog body for filter commands.
pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let cmd = f.get("__command").and_then(Value::as_str).unwrap_or_default().to_string();
    // Plug-in dialogs carry their own spec (from the plug-in's manifest).
    let spec = match f.get("__spec").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => match photocraft_engine::commands::find(&cmd) {
            Some(c) => c.params.to_string(),
            None => return,
        },
    };
    if crate::select_mask_view::owns(f) {
        crate::select_mask_view::controls(ui, f);
    }
    for p in parse_spec(&spec) {
        if zoom_center::owns(&cmd, f) && matches!(p.key.as_str(), "centerX" | "centerY") {
            if p.key == "centerX" {
                zoom_center::row(ui, f);
            }
            continue;
        }
        match p.kind {
            Kind::Range { min, max, default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                if cmd == "image.mode.indexedColor" && p.key == "colors" {
                    let mut count = v.round().clamp(2.0, 256.0) as u32;
                    crate::widgets::color_count_row(ui, &label(&p.key), &mut count);
                    f.insert(p.key, json!(count));
                    continue;
                }
                // Replace Color's Range only applies with Localized Color Clusters on, as in Photoshop.
                let enabled = !(cmd == REPLACE_COLOR && p.key == "range") || f.get("localized").and_then(Value::as_bool) == Some(true);
                let unit = if is_pixel_param_of(&cmd, &p.key) {
                    "px"
                } else if p.key == "angle" {
                    "°"
                } else if p.key == "amount" && max <= 500.0 {
                    "%"
                } else {
                    ""
                };
                ui.scope(|ui| {
                    if !enabled {
                        ui.disable();
                    }
                    if uses_angle_dial(&cmd, &p.key) {
                        crate::widgets::angle_dial_row(ui, &label(&p.key), &mut v, min..=max);
                        ui.add_space(4.0);
                    } else if uses_logarithmic_slider(min, max) {
                        // Keep one numeric field for direct entry and one log-scaled slider for
                        // wide ranges. slider_row would add a second, linear slider (#146).
                        ui.horizontal(|ui| {
                            let name = label(&p.key);
                            ui.label(egui::RichText::new(&name).color(t.text_dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                crate::widgets::value_field(ui, &mut v, min..=max, unit, 74.0).accessible_name(name);
                            });
                        });
                        let mut lv = v.clamp(min, max).ln();
                        if crate::widgets::slider(ui, &mut lv, min.ln()..=max.ln(), None).changed() {
                            v = lv.exp();
                        }
                        ui.add_space(4.0);
                    } else {
                        // A double-click on the slider puts it back to the filter's default.
                        let gestures = crate::widgets::RowGestures { reset: Some(default), wheel_step: None, ..Default::default() };
                        crate::widgets::slider_row_with(ui, &label(&p.key), &mut v, min..=max, unit, None, gestures);
                    }
                });
                // Small ranges (0–1 thresholds and centres) keep the two decimals their field shows.
                let scale = if max - min <= 10.0 { 100.0 } else { 10.0 };
                f.insert(p.key, json!((v * scale).round() / scale));
            }
            Kind::Choice { options, default } => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    let initial = default.as_deref().or_else(|| options.first().map(String::as_str)).unwrap_or("");
                    let mut cur = f.get(&p.key).and_then(Value::as_str).unwrap_or(initial).to_string();
                    let labels: Vec<String> = options.iter().map(|o| choice_label(o)).collect();
                    let mut opts: Vec<(String, &str)> = options.iter().cloned().zip(labels.iter().map(String::as_str)).collect();
                    // Lens Blur's Source: each alpha channel by its name, in place of `channel`.
                    let depth = cmd == LENS_BLUR && p.key == "depthMap";
                    let names: Vec<String> = if depth {
                        f.get("__channels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
                    } else {
                        Vec::new()
                    };
                    let channels: Vec<String> = (0..names.len()).map(channel_choice).collect();
                    if depth {
                        opts.retain(|(v, _)| v != "channel");
                        opts.extend(channels.iter().cloned().zip(names.iter().map(String::as_str)));
                        if cur == "channel"
                            && let Some(i) = f.get("depthChannel").and_then(|v| channel_index(v, &names))
                        {
                            cur = channel_choice(i);
                        }
                    }
                    // Like Photoshop, the profile menus also list the profiles installed in the
                    // system (printer and paper profiles, press conditions), by description.
                    let lists_installed = installed_filter(&cmd, &p.key).is_some();
                    let installed =
                        if lists_installed { installed_choices(&cmd, &p.key, &photocraft_engine::installed_profiles::installed()) } else { Vec::new() };
                    opts.extend(installed.iter().map(|(path, description)| (path.clone(), description.as_str())));
                    // Assign Profile: only what the document's colour mode can take (#4951).
                    if p.key == "profile"
                        && let Some(allowed) = f.get("__profiles").and_then(Value::as_array)
                    {
                        opts.retain(|(v, _)| allowed.iter().any(|a| a.as_str() == Some(v.as_str())));
                    }
                    // A profile given by path that isn't listed (typed by an agent, or spelled with
                    // different case) still shows, by its file name, instead of a blank menu.
                    let stem = std::path::Path::new(&cur).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if lists_installed && !opts.iter().any(|(v, _)| *v == cur) {
                        opts.push((cur.clone(), stem.as_str()));
                    }
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &label(&p.key), &mut cur, &opts, 170.0);
                    if let Some(i) = cur.strip_prefix("channel:").filter(|_| depth).and_then(|i| i.parse::<usize>().ok()) {
                        f.insert("depthChannel".into(), json!(i));
                        cur = "channel".into();
                    } else if depth {
                        f.remove("depthChannel");
                    }
                    f.insert(p.key.clone(), json!(cur));
                });
            }
            Kind::Bool(default) => {
                let mut b = f.get(&p.key).and_then(Value::as_bool).unwrap_or(default);
                crate::widgets::checkbox(ui, &mut b, &label(&p.key));
                f.insert(p.key, json!(b));
            }
            Kind::Text => {
                let mut v = f.get(&p.key).and_then(Value::as_str).unwrap_or_default().to_string();
                ui.horizontal(|ui| {
                    let lp = ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(200.0)).labelled_by(lp.id);
                    // Install Plug-in…: pick the .wasm file instead of typing its path (#7531).
                    if cmd == "plugin.install" && p.key == "path" && crate::widgets::secondary_button(ui, tl!("Browse…"), 0.0).clicked() {
                        f.insert(crate::dialogs::BROWSE_FIELD.into(), json!(p.key));
                    }
                });
                if v.is_empty() {
                    f.remove(&p.key);
                } else {
                    f.insert(p.key, json!(v));
                }
            }
            Kind::Document => {
                let names: Vec<String> =
                    f.get("__docs").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let cur = f.get(&p.key).and_then(Value::as_i64).unwrap_or(-1);
                let mut sel = cur.to_string();
                let mut opts: Vec<(String, &str)> = vec![("-1".to_string(), tl!("None"))];
                opts.extend(names.iter().enumerate().map(|(i, n)| (i.to_string(), n.as_str())));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &label(&p.key), &mut sel, &opts, 170.0);
                });
                match sel.parse::<i64>() {
                    Ok(i) if i >= 0 => f.insert(p.key, json!(i)),
                    _ => f.insert(p.key, json!(-1)),
                };
            }
            Kind::Grid(n) => {
                let side = (n as f32).sqrt().round().max(1.0) as usize;
                let mut vals: Vec<f32> =
                    f.get(&p.key).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                vals.resize(n, 0.0);
                ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                egui::Grid::new(format!("flt-grid-{cmd}-{}", p.key)).spacing([4.0, 4.0]).show(ui, |ui| {
                    for (i, v) in vals.iter_mut().enumerate() {
                        crate::widgets::value_field(ui, v, -999.0..=999.0, "", 44.0).accessible_name(format!("{} {}", label(&p.key), i + 1));
                        if (i + 1) % side == 0 {
                            ui.end_row();
                        }
                    }
                });
                f.insert(p.key, json!(vals.iter().map(|v| v.round() as i64).collect::<Vec<_>>()));
            }
            Kind::Json => {}
            Kind::Color(default) => {
                let set = f.get(&p.key).and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex);
                let rgb = set.or(default).unwrap_or([0.0; 3]);
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let bytes = rgb.map(q);
                let name = label(&p.key);
                let mut clicked = false;
                let eyedropper = cmd == REPLACE_COLOR && p.key == "color";
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&name).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // PhotoCraft's Color Picker, as everywhere else (#2144).
                        clicked = crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(bytes[0], bytes[1], bytes[2]), &name).clicked();
                        if eyedropper {
                            let armed = f.get(COLOR_EYEDROPPER).and_then(Value::as_bool).unwrap_or(false);
                            if crate::icons::button(ui, "pipette", 24.0, armed, "Pick Color From Image").clicked() {
                                f.insert(COLOR_EYEDROPPER.into(), json!(!armed));
                            }
                        }
                        ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
                    });
                });
                if set.is_none() && default.is_some() {
                    f.insert(p.key.clone(), json!(crate::color_picker_ui::hex(rgb)));
                }
                if clicked {
                    if !f.contains_key(&p.key) {
                        f.insert(p.key.clone(), json!(crate::color_picker_ui::hex(rgb)));
                    }
                    crate::color_picker_ui::request_field(f, &p.key, &crate::color_picker_ui::title_for(&name));
                }
            }
            Kind::Int { default } => {
                let mut v = f.get(&p.key).and_then(Value::as_f64).unwrap_or(default as f64) as f32;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    crate::widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", 80.0).accessible_name(label(&p.key));
                    // A seed gets a fresh random value in one click (#5710): Clouds, Add Noise,
                    // Fibers… then render a new pattern, as each run of Photoshop's does.
                    if p.key == "seed" && ui.button(tl!("Random")).clicked() {
                        v = random_seed(ui.input(|i| i.time).to_bits(), v as i64) as f32;
                    }
                });
                f.insert(p.key, json!(v.round() as i64));
            }
        }
    }
    if cmd == REPLACE_COLOR {
        replace_color_result(ui, f);
    }
    // Read-only context the dialog opener supplies (e.g. the monitor profile in use).
    if let Some(note) = f.get("__note").and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(note).color(t.text_dim)).wrap());
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

/// A new seed for a dialog's Random button: 0..=30000 (what the field takes), never `current`.
/// `entropy` varies per click (the input time); the process's random hasher keys mix it.
fn random_seed(entropy: u64, current: i64) -> i64 {
    use std::hash::BuildHasher;
    let h = std::collections::hash_map::RandomState::new().hash_one(entropy);
    let seed = (h % 30_001) as i64;
    if seed == current { (seed + 1) % 30_001 } else { seed }
}

/// User-facing params (strip the dialog's private `__` keys).
pub fn params_of(f: &Map<String, Value>) -> Value {
    // A document picker left at "None" (-1) means "not given".
    let unset = |k: &str, v: &Value| k == "mapDocument" && v.as_i64() == Some(-1);
    Value::Object(f.iter().filter(|(k, v)| !k.starts_with("__") && !unset(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Filter › Distort › Displace confirmed with no map: no open document picked and no path typed
/// (#5806). Photoshop asks for the displacement map file after OK; so does PhotoCraft.
pub(crate) fn needs_displace_map(command: &str, params: &Value) -> bool {
    command == "filter.distort.displace"
        && params.get("mapPath").and_then(Value::as_str).is_none_or(|p| p.trim().is_empty())
        && params.get("mapDocument").and_then(Value::as_i64).is_none_or(|i| i < 0)
}

/// Asks for the displacement map, then runs Displace with the dialog's values and that file, as
/// OK in a dialog that had one would (smart filters included). Cancel applies nothing. In the
/// browser the picker gives contents, not a path the filter can read: the map has to be opened
/// as a document and picked as Map Document there. It displaces the document that asked, even
/// when another one was activated while the picker was open.
pub(crate) fn ask_displace_map(app: &mut PhotocraftApp, command: String, fields: Map<String, Value>) -> Result<Value, String> {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    let doc = app.active_doc_id()?;
    let initial_dir = crate::file_dialog::last_used_dir(&app.ui.recent_files);
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir, extensions: None }, move |app, answer| {
        let path = match answer {
            FileDialogAnswer::Paths(paths) => paths.into_iter().next().ok_or("no displacement map was chosen")?,
            FileDialogAnswer::Unavailable(e) => return Err(e),
            FileDialogAnswer::Contents(..) | FileDialogAnswer::ContentsMany(_) => {
                return Err("open the displacement map as a document, then choose it as the Map Document".into());
            }
            FileDialogAnswer::SaveTo(_) | FileDialogAnswer::FolderTo(_) => return Err("unexpected answer from the file dialog".into()),
        };
        let mut fields = fields;
        fields.insert("mapPath".into(), json!(path));
        app.refocus(doc)?;
        let (cmd, params) = crate::smart_ui::confirm_command(app, &fields, command, params_of(&fields))?;
        let result = app.run(&cmd, params);
        if result.is_ok() {
            remember(app, &cmd, &fields);
            let _ = crate::actions::resume(app, &fields);
        }
        result
    })
}

/// Compute a preview document: run `command` with `params` on the proxy (scaled) copy of `doc`.
pub fn preview_document(doc: &Document, active: Option<photocraft_doc::LayerId>, command: &str, params: &Value, k: u32) -> Option<Document> {
    preview_document_with(doc, active, command, params, k, None)
}

/// [`preview_document`] whose filter stops early, giving `None`, once `cancel` is cancelled
/// (a preview superseded by newer dialog values).
pub fn preview_document_with(
    doc: &Document,
    active: Option<photocraft_doc::LayerId>,
    command: &str,
    params: &Value,
    k: u32,
    cancel: Option<&photocraft_engine::jobs::JobCtx>,
) -> Option<Document> {
    preview_document_for_target(
        doc,
        active,
        command,
        params,
        k,
        cancel,
        photocraft_engine::channel_cmds::ChannelTarget::Composite,
        &[],
        &ColorSettings::default(),
    )
}

/// Run the preview with the same pixel target as the final command, including individual channels.
/// `documents` are the open documents in session order: document params (Match Color's Source,
/// Displace's map) index into them. `color` is the app's Edit › Color Settings: Assign and Convert
/// to Profile's `working` is the working space chosen there, not the built-in default.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_document_for_target(
    doc: &Document,
    active: Option<photocraft_doc::LayerId>,
    command: &str,
    params: &Value,
    k: u32,
    cancel: Option<&photocraft_engine::jobs::JobCtx>,
    target: photocraft_engine::channel_cmds::ChannelTarget,
    documents: &[Arc<Document>],
    color: &ColorSettings,
) -> Option<Document> {
    if command == crate::select_mask_view::COMMAND {
        return crate::select_mask_view::preview_document(doc, active, params, k, cancel);
    }
    // Warp Text renders the type again from the layer model (font size, position), which a
    // reduced proxy doesn't scale: it runs at full size and the result is reduced (#218).
    let (run_k, reduce_k) = if command == "type.warpText" { (1, k) } else { (k, 1) };
    let mut proxy = crate::proxy::proxy_document(doc, run_k);
    let mut p = params.clone();
    if is_smart_edit(command) {
        let layer = match params.get("layer") {
            Some(id) => photocraft_doc::LayerId(id.as_u64()?),
            None => active?,
        };
        let index = usize::try_from(params.get("index")?.as_u64()?).ok()?;
        let photocraft_doc::LayerContent::Smart(sm) = &mut proxy.layer_mut(layer)?.content else { return None };
        // Proxy_document scales the cache, placement and shared mask (#5766). Editing a filter
        // re-renders its source: scale every filter's lengths and the mask's feather as well.
        if run_k > 1 {
            if let Some(mask) = &mut sm.filter_mask {
                mask.feather /= run_k as f32;
            }
            for f in &mut sm.smart_filters {
                scale_preview_params(&f.command, &mut f.params, run_k);
            }
            if command == "layer.smartFilter.setParams" {
                let f = sm.smart_filters.get(index)?;
                scale_preview_params(&f.command, p.get_mut("params")?, run_k);
            }
        }
    } else {
        scale_preview_params(command, &mut p, run_k);
    }
    let mut s = photocraft_engine::Session::new();
    s.color.settings = color.clone();
    s.set_inline_job_ctx(cancel.cloned());
    let doc_id = proxy.id;
    s.add_document(proxy, None);
    // Bitmap's Output resolution resamples the image: the preview keeps the canvas's size and
    // resolution, so its dots and halftone screen show at their printed size (#5559).
    if command == crate::bitmap_dialog::COMMAND
        && let Some(o) = p.as_object_mut()
    {
        o.remove("resolution");
    }
    add_source_documents(&mut s, doc_id, command, &mut p, documents, run_k);
    if let Some(id) = active {
        s.select_layer(id).ok()?;
    }
    s.active_mut()?.channel_view.target = target;
    s.execute(command, p).ok()?;
    s.active().map(|d| crate::proxy::proxy_document(&d.doc, reduce_k))
}

/// Whether `command` is a Smart Filter edit (its parameters or its blending) rather than a filter run on its own.
fn is_smart_edit(command: &str) -> bool {
    matches!(command, "layer.smartFilter.setParams" | "layer.smartFilter.blendingOptions")
}

/// Each stage constrains the proxy factor; an unscalable filter keeps full-resolution preview.
pub(crate) fn smart_preview_factor(doc: &Document, command: &str, params: &Value, k: u32) -> u32 {
    if !is_smart_edit(command) {
        return preview_factor(command, params, k);
    }
    let Some(layer) = params.get("layer").and_then(Value::as_u64).and_then(|id| doc.layer(photocraft_doc::LayerId(id))) else { return 1 };
    let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { return 1 };
    let mut factor = sm.smart_filters.iter().fold(k, |k, f| preview_factor(&f.command, &f.params, k));
    if command == "layer.smartFilter.setParams"
        && let Some(index) = params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
        && let Some(f) = sm.smart_filters.get(index)
        && let Some(p) = params.get("params")
    {
        factor = preview_factor(&f.command, p, factor);
    }
    factor
}

/// Scale the pixel-sized parameters of `command` down by the preview proxy factor `k`, keeping each above its minimum and negative lengths negative.
fn scale_preview_params(command: &str, params: &mut Value, k: u32) {
    if command == photocraft_engine::plugin_cmds::RUN {
        // A plug-in declares its pixel lengths (`"unit": "px"` in its manifest), and only those
        // are scaled: a 12 px shift previewed 4 times too wide at k = 4, and a unitless param that
        // happens to be called `size` or `radius` must not be divided (#7524).
        photocraft_engine::plugin_cmds::scale_for_preview(params, k);
    } else if k > 1
        && let Some(o) = params.as_object_mut()
    {
        let spec = photocraft_engine::commands::find(command).map(|c| parse_spec(c.params)).unwrap_or_default();
        for (key, v) in o.iter_mut() {
            if is_pixel_param_of(command, key)
                && let Some(x) = v.as_f64()
            {
                // Match the engine's minimum after scaling (e.g. Box Blur 1, Mosaic 2).
                // Previously its tolerant decoder clamped these; new calls validate first.
                let minimum = spec.iter().find(|p| p.key == key.as_str()).and_then(|p| match &p.kind {
                    Kind::Range { min, .. } => Some(f64::from(*min)),
                    _ => None,
                });
                let floor = minimum.unwrap_or(0.0).max(if key == "cellSize" { 1.0 } else { 0.1 });
                // Negative lengths (Offset's left/up shift, Displace's inverted scale) keep their
                // sign: the floor that keeps a radius positive turned them into +0.1 (#2063).
                let x = x / k as f64;
                *v = json!(if x > 0.0 { x.max(floor) } else { minimum.map_or(x, |m| x.max(m)) });
            }
        }
    }
    // Blur Gallery pins carry their blur in document pixels too.
    if command.starts_with("filter.blurGallery.") {
        crate::blur_pins::scale_for_proxy(params, k);
    }
}

/// The preview session holds only the proxy of the edited document (index 0), so a document param
/// pointing at another open document found nothing there: Match Color's preview with a Source
/// document failed, or matched the image to itself (#4851). Add each referenced document (as a
/// proxy at the same factor) and point the param at its index in the preview session.
fn add_source_documents(s: &mut photocraft_engine::Session, doc_id: photocraft_doc::DocId, command: &str, p: &mut Value, documents: &[Arc<Document>], k: u32) {
    let Some(spec) = photocraft_engine::commands::find(command) else { return };
    let Some(o) = p.as_object_mut() else { return };
    let mut added: Vec<(usize, usize)> = Vec::new();
    let mut remap = |v: &mut Value| {
        let Some(i) = v.as_u64().and_then(|i| usize::try_from(i).ok()) else { return };
        // An index past the open documents stays as given, so the preview fails like OK would.
        let Some(src) = documents.get(i) else { return };
        let at = if src.id == doc_id {
            0
        } else if let Some(&(_, at)) = added.iter().find(|(j, _)| *j == i) {
            at
        } else {
            let at = s.add_document(crate::proxy::proxy_document(src, k), None);
            added.push((i, at));
            at
        };
        *v = json!(at);
    };
    for param in parse_spec(spec.params).into_iter().filter(|p| p.kind == Kind::Document) {
        if let Some(v) = o.get_mut(&param.key) {
            remap(v);
        }
    }
    // A smart filter being edited carries its filter's params one level down: Displace's
    // `mapDocument` there names an open document too.
    if command == SMART_FILTER_SET_PARAMS
        && let Some(v) = o.get_mut("params").and_then(Value::as_object_mut).and_then(|f| f.get_mut("mapDocument"))
    {
        remap(v);
    }
    // Adding a document makes it active: the command still runs on the edited one.
    s.set_active(0);
}

/// Cached preview state on the app.
pub struct FilterPreview {
    pub key: FilterPreviewKey,
    pub result: Option<Arc<Document>>,
    /// OK was pressed and the filter runs as a background job: the preview stays on screen until
    /// the job lands, so the canvas doesn't flash the unfiltered image in between.
    pub committing: bool,
}

/// Identity of one filter dialog's live preview.
///
/// `PartialEq` is the whole request: it says a worker result is exactly what the dialog is asking
/// for now. [`Self::same_target`] is the looser question of whether a result belongs to this
/// preview at all; two keys that agree on it name the same dialog, layer, document and preview
/// resolution, and differ only in parameters — which is what a slider drag produces, since the
/// worker computed for the value the pointer passed a moment ago.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterPreviewKey {
    pub doc: photocraft_doc::DocId,
    pub revision: u64,
    pub dialog: u64,
    pub active: Option<photocraft_doc::LayerId>,
    pub target: photocraft_engine::channel_cmds::ChannelTarget,
    pub command: String,
    pub params: Value,
    pub k: u32,
}

impl FilterPreviewKey {
    /// Everything except the parameters: same dialog, document, revision, layer, channel target,
    /// command and proxy factor.
    ///
    /// The canvas shows whatever this preview was computed for, so a result whose parameters are
    /// a few frames old still belongs on screen while a slider is moving; matching on everything
    /// would mean no frame of a drag is ever shown, because every completion lands after the
    /// parameters moved on. A reopened dialog has a new id, a different command or a different
    /// document, and a committed document a new revision, so neither survives this test.
    pub fn same_target(&self, other: &Self) -> bool {
        self.doc == other.doc
            && self.revision == other.revision
            && self.dialog == other.dialog
            && self.active == other.active
            && self.target == other.target
            && self.command == other.command
            && self.k == other.k
    }
}

#[cfg(test)]
mod tests;
