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
                            ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                crate::widgets::value_field(ui, &mut v, min..=max, unit, 74.0);
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
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut cur, &opts, 170.0);
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
                    ui.label(egui::RichText::new(label(&p.key)).color(t.text_dim));
                    ui.add(egui::TextEdit::singleline(&mut v).desired_width(200.0));
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
                    crate::widgets::dropdown(ui, &format!("flt-{cmd}-{}", p.key), &mut sel, &opts, 170.0);
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
                        crate::widgets::value_field(ui, v, -999.0..=999.0, "", 44.0);
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
                    crate::widgets::value_field(ui, &mut v, -30000.0..=30000.0, "", 80.0);
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
mod tests {
    use super::*;

    #[test]
    fn a_unit_after_a_range_bound_does_not_reset_the_range_to_its_fallback() {
        // #7598: `0.5..5000 px?` fell back to a maximum of 100, so the Mixer Brush's Size slider
        // (and every range documented with a px or % suffix) topped out at 100.
        let range = |spec: &str| match parse_spec(spec).into_iter().next().map(|p| p.kind) {
            Some(Kind::Range { min, max, default }) => (min, max, default),
            other => panic!("{spec}: {other:?}"),
        };
        assert_eq!(range(r#"{"size":0.5..5000 px?}"#), (0.5, 5000.0, 0.5));
        assert_eq!(range(r#"{"scale":1..1000 %=100}"#), (1.0, 1000.0, 100.0));
        assert_eq!(range(r#"{"tolerance":0.5..10 px=2}"#), (0.5, 10.0, 2.0));
        assert_eq!(range(r#"{"angle":-180..180}"#), (-180.0, 180.0, -180.0));
        assert_eq!(range(r#"{"radius":0.1..1000=1}"#), (0.1, 1000.0, 1.0));
        assert_eq!(range(r#"{"gain":1e-3..2e2}"#), (0.001, 200.0, 0.001));
        // A bound that is no number at all still takes the old fallback rather than failing.
        assert_eq!(range(r#"{"x":0..many}"#), (0.0, 100.0, 0.0));
        // The documented specs of the real commands read their full ranges.
        let spec = photocraft_engine::commands::find("paint.pencil").expect("paint.pencil is registered");
        let size = parse_spec(spec.params).into_iter().find(|p| p.key == "size");
        assert!(matches!(size.map(|p| p.kind), Some(Kind::Range { max, .. }) if max == 5000.0));
    }

    #[test]
    fn color_settings_working_spaces_list_installed_profiles_of_their_space() {
        // #6323: a press condition installed in the system can be the CMYK working space, as in
        // Photoshop's Color Settings; each working-space menu lists only profiles of its space.
        use photocraft_cms::{ColorSpace, ProfileClass};
        let p = |path: &str, class, color_space| InstalledProfile { path: path.into(), description: path.to_uppercase(), class, color_space };
        let installed = [
            p("gracol.icc", ProfileClass::Output, ColorSpace::Cmyk),
            p("monitor.icc", ProfileClass::Display, ColorSpace::Rgb),
            p("printer-rgb.icc", ProfileClass::Output, ColorSpace::Rgb),
            p("scanner.icc", ProfileClass::Input, ColorSpace::Rgb),
            p("gray.icc", ProfileClass::Output, ColorSpace::Gray),
            p("link.icc", ProfileClass::DeviceLink, ColorSpace::Cmyk),
        ];
        let paths = |key: &str| installed_choices("edit.colorSettings", key, &installed).into_iter().map(|(path, _)| path).collect::<Vec<_>>();
        assert_eq!(paths("workingCmyk"), ["gracol.icc"]);
        assert_eq!(paths("workingRgb"), ["monitor.icc"]);
        assert_eq!(paths("workingGray"), ["gray.icc"]);
        assert!(paths("policyCmyk").is_empty() && paths("intent").is_empty());
        assert_eq!(installed_choices("edit.colorSettings", "workingCmyk", &installed)[0].1, "GRACOL.ICC", "listed by description");
        // The profile dialogs keep listing every destination profile.
        assert_eq!(installed_choices("edit.convertToProfile", "profile", &installed).len(), 5);
        assert!(installed_choices("filter.blur.gaussianBlur", "profile", &installed).is_empty());
    }

    /// #5710: the seed field's Random button always gives a different seed the field can hold.
    #[test]
    fn random_seeds_stay_in_range_and_change() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..200u64 {
            let s = random_seed(i, 7);
            assert!((0..=30_000).contains(&s), "{s}");
            assert_ne!(s, 7, "a click always changes the seed");
            seen.insert(s);
        }
        assert!(seen.len() > 100, "seeds spread out: {}", seen.len());
        assert_ne!(random_seed(1, 30_000), 30_000);
        // Every seed dialog shows the button: Clouds, Add Noise and the like carry a `seed` param.
        let specs = photocraft_engine::command_specs();
        assert!(
            specs
                .iter()
                .filter(|c| c.id.starts_with("filter."))
                .any(|c| parse_spec(c.params).iter().any(|p| p.key == "seed" && matches!(p.kind, Kind::Int { .. })))
        );
    }

    /// Preset names such as "iPhone 14 Pro Max" (the artboard dialog's choices) are text, not
    /// camelCase keys: they used to read "I Phone 14  Pro  Max".
    #[test]
    fn display_names_are_not_split_like_camel_case_keys() {
        assert_eq!(source_label("iPhone 14 Pro Max"), "iPhone 14 Pro Max");
        assert_eq!(source_label("Web 1920"), "Web 1920");
        assert_eq!(source_label("centerX"), "Center X");
    }

    /// Convert to Profile's and Proof Setup's `bpc` read "Bpc": spelled out.
    #[test]
    fn black_point_compensation_is_spelled_out() {
        assert_eq!(source_label("bpc"), "Black Point Compensation");
    }

    fn key() -> FilterPreviewKey {
        FilterPreviewKey {
            doc: photocraft_doc::DocId(7),
            revision: 11,
            dialog: 3,
            active: Some(photocraft_doc::LayerId(2)),
            target: Default::default(),
            command: "filter.blur.gaussianBlur".into(),
            params: json!({"radius": 4.0}),
            k: 1,
        }
    }

    #[test]
    fn same_target_ignores_parameters_but_nothing_else() {
        // A drag only changes the parameters, and that must not hide the finished frame.
        let base = key();
        let mut moved = base.clone();
        moved.params = json!({"radius": 40.0});
        assert!(base.same_target(&moved));
        assert!(moved.same_target(&base));

        // Anything else names a different preview and must not be shown.
        for break_it in [
            |k: &mut FilterPreviewKey| k.doc = photocraft_doc::DocId(8),
            |k: &mut FilterPreviewKey| k.revision = 12,
            |k: &mut FilterPreviewKey| k.dialog = 4,
            |k: &mut FilterPreviewKey| k.active = Some(photocraft_doc::LayerId(5)),
            |k: &mut FilterPreviewKey| k.active = None,
            |k: &mut FilterPreviewKey| k.target = photocraft_engine::channel_cmds::ChannelTarget::LayerMask,
            |k: &mut FilterPreviewKey| k.command = "filter.blur.motionBlur".into(),
            |k: &mut FilterPreviewKey| k.k = 2,
        ] {
            let mut other = base.clone();
            break_it(&mut other);
            assert!(!other.same_target(&base), "{other:?} must not pass as {base:?}");
            assert!(!base.same_target(&other));
        }
    }

    #[test]
    fn only_direction_angles_get_the_dial() {
        // Every listed command still has a degree `angle` range, so the list cannot go stale.
        for cmd in ANGLE_DIAL {
            let spec = photocraft_engine::commands::find(cmd).unwrap_or_else(|| panic!("{cmd} is not a command")).params;
            assert!(
                parse_spec(spec).iter().any(|p| p.key == "angle" && matches!(p.kind, Kind::Range { min, max, .. } if min >= -360.0 && max <= 360.0)),
                "{cmd} has no angle range"
            );
            assert!(uses_angle_dial(cmd, "angle"));
            assert!(!uses_angle_dial(cmd, "distance"));
        }
        // Rotation amounts and angles with another zero keep their slider.
        for cmd in
            ["filter.distort.twirl", "image.rotation.arbitrary", "image.crop", "filter.blurGallery.tiltShift", "filter.lensCorrection", "filter.render.flame"]
        {
            assert!(!uses_angle_dial(cmd, "angle"), "{cmd}");
        }
    }

    #[test]
    fn restores_only_valid_previous_filter_choices() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("filter.blur.gaussianBlur".into(), json!({"radius": 11.0, "bogus": 42}));
        });
        open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["radius"], json!(11.0));
        assert!(!fields.contains_key("bogus"));

        // A changed registry range, corrupt preference or stale path is never restored.
        let mut fields = Map::new();
        fields.insert("radius".into(), json!(2.0));
        restore_remembered(&app, "filter.blur.gaussianBlur", r#"{"radius":0.1..5=1}"#, &mut fields);
        assert_eq!(fields["radius"], json!(2.0));
    }

    #[test]
    fn alt_reset_restores_schema_defaults_not_remembered() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("filter.blur.gaussianBlur".into(), json!({"radius": 11.0}));
        });
        let id = open(&mut app, "filter.blur.gaussianBlur").unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["radius"], json!(11.0));
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        reset_fields(&app, &mut fields);
        app.ui.dialog_mut(id).unwrap().fields = fields;
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        // Registry default for gaussian blur radius is 1.
        assert_eq!(f["radius"], json!(1.0));
        assert!(f.contains_key("__filter"));
        assert!(app.ui.dialogs.iter().any(|d| d.id == id));
    }

    /// #5806: OK in Displace with no map asks for the map file, as Photoshop does, and displaces
    /// with the chosen file; Cancel in the picker applies nothing; a map already given runs at once.
    #[test]
    fn displace_asks_for_its_map_after_ok() {
        let dir = std::env::temp_dir().join(format!("photocraft-displace-map-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let map = dir.join("map.png");
        let mut m = photocraft_engine::Session::new();
        m.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        std::fs::write(&map, photocraft_io::export(&m.active().unwrap().doc, "map.png", &Default::default()).unwrap().bytes).unwrap();
        let answers = vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![map.to_string_lossy().into_owned()])), None];
        let (show, asked) = crate::file_dialog::fake(answers);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let ctx = egui::Context::default();
        let id = open(&mut app, "filter.distort.displace").unwrap();
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.file_dialog_open(), "OK leaves the map picker pending");
        // Another document activated while the picker is open is left alone.
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        let other_steps = app.session.documents()[1].history.past_len();
        // The frame shows the picker (the service is asked then) and acts on its answer.
        app.poll_file_dialog(&ctx, None);
        assert_eq!(asked.borrow().len(), 1, "OK asks for the map");
        assert_eq!(app.session.active_index(), Some(0), "the document that asked is shown");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "Displace ran with the chosen map");
        assert_eq!(app.session.documents()[1].history.past_len(), other_steps, "not on the other document");
        // Cancelled: nothing is applied.
        let id = open(&mut app, "filter.distort.displace").unwrap();
        crate::dialogs::confirm(&mut app, id).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert_eq!(asked.borrow().len(), 2);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        // A map given in the dialog runs without asking.
        assert!(!needs_displace_map("filter.distort.displace", &json!({"mapPath": "/maps/x.psd"})));
        assert!(!needs_displace_map("filter.distort.displace", &json!({"mapDocument": 0})));
        assert!(needs_displace_map("filter.distort.displace", &json!({"mapDocument": -1, "mapPath": " "})));
        assert!(!needs_displace_map("filter.blur.gaussianBlur", &json!({})));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembering_dialogs_never_persists_paths_or_document_indices() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut fields = Map::new();
        fields.insert("__filter".into(), json!(true));
        fields.insert("radius".into(), json!(7.0));
        fields.insert("mapPath".into(), json!("/private/file"));
        fields.insert("document".into(), json!(5));
        remember(&mut app, "filter.blur.gaussianBlur", &fields);
        let saved = &app.session.prefs().dialogs["filter.blur.gaussianBlur"];
        assert_eq!(saved["radius"], json!(7.0));
        assert!(saved.get("mapPath").is_none());
        assert!(saved.get("document").is_none());
    }

    #[test]
    fn parses_registry_notation() {
        let p = parse_spec(r#"{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}"#);
        assert_eq!(p[0], Param { key: "radius".into(), kind: Kind::Range { min: 0.1, max: 1000.0, default: 1.0 } });
        assert_eq!(p[1].kind, Kind::Choice { options: vec!["spin".into(), "zoom".into()], default: None });
        assert_eq!(p[2].kind, Kind::Bool(false));
        assert_eq!(p[3].kind, Kind::Int { default: 0 });
        assert_eq!(p[4].kind, Kind::Int { default: 0 });
        assert!(parse_spec("{}").is_empty());
        let p_choice = parse_spec(r#"{"quality":"draft|good|best"="good"}"#);
        assert_eq!(p_choice[0].kind, Kind::Choice { options: vec!["draft".into(), "good".into(), "best".into()], default: Some("good".into()) });
        let p = parse_spec(r#"{"lighting":bool=true,"kernel":int[25],"pins":json,"points":[[0,0],[1,0]],"mapPath":text,"mapDocument":doc,"scale":1..9999=1}"#);
        let kinds: Vec<&Kind> = p.iter().map(|p| &p.kind).collect();
        assert_eq!(
            kinds,
            [&Kind::Bool(true), &Kind::Grid(25), &Kind::Json, &Kind::Json, &Kind::Text, &Kind::Document, &Kind::Range { min: 1.0, max: 9999.0, default: 1.0 }]
        );
    }

    /// A layer-id param has no number field: drawing the Auto-Align dialog used to write
    /// `reference: 0` back, which the engine rejects as not a selected layer (#674).
    #[test]
    fn layer_id_params_stay_out_of_the_dialog() {
        let mut f = Map::new();
        f.insert("__command".into(), json!("edit.autoAlignLayers"));
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        assert!(!params_of(&f).as_object().unwrap().contains_key("reference"), "{f:?}");
    }

    /// Layer › Layer Style › Global Light (#3054): Global Light dialog opens with the active
    /// document's current shared lighting, preserving it on unchanged OK, ignoring stale
    /// remembered dialog values, tracking document switching, applying edits, and leaving the
    /// document unchanged on Cancel.
    #[test]
    fn global_light_dialog_reflects_active_document_light() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc1_id = app.session.active().unwrap().doc.id;

        // 0. Default-document control: fresh document starts at angle 120 and altitude 30.
        let d_default = crate::menus::invoke(&mut app, &ctx, "layer.layerStyle.globalLight", Value::Null).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(app.ui.dialog_mut(d_default).unwrap().fields["angle"], json!(120.0));
        assert_eq!(app.ui.dialog_mut(d_default).unwrap().fields["altitude"], json!(30.0));
        crate::dialogs::confirm(&mut app, d_default).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, 120.0);
        assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 30.0);

        // 1. Change document's global light to 30 degrees angle and 45 degrees altitude.
        app.run("layer.layerStyle.globalLight", json!({"angle": 30.0, "altitude": 45.0})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, 30.0);
        assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 45.0);

        // 2. Open Global Light dialog: opens with the active document's light (30 / 45).
        let d_id = crate::menus::invoke(&mut app, &ctx, "layer.layerStyle.globalLight", Value::Null).unwrap()["dialog"].as_u64().unwrap();
        let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
        assert_eq!(fields["angle"], json!(30.0));
        assert_eq!(fields["altitude"], json!(45.0));

        // 3. Unchanged OK preserves the document's light.
        crate::dialogs::confirm(&mut app, d_id).unwrap();
        let gl = app.session.active().unwrap().doc.global_light;
        assert_eq!(gl.angle, 30.0);
        assert_eq!(gl.altitude, 45.0);

        // 4. Stale remembered values do not override the active document.
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(
                "layer.layerStyle.globalLight".into(),
                json!({
                    "angle": 75.0,
                    "altitude": 60.0,
                }),
            );
        });
        let d_id = open(&mut app, "layer.layerStyle.globalLight").unwrap();
        let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
        assert_eq!(fields["angle"], json!(30.0), "reads active document, not remembered");
        assert_eq!(fields["altitude"], json!(45.0));

        // 5. Cancel preserves the document's light.
        app.ui.close_dialog(d_id);
        let gl = app.session.active().unwrap().doc.global_light;
        assert_eq!(gl.angle, 30.0);
        assert_eq!(gl.altitude, 45.0);

        // 6. Multiple documents: each document initializes the dialog to its own light.
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc2_id = app.session.active().unwrap().doc.id;
        assert_ne!(doc1_id, doc2_id);
        app.run("layer.layerStyle.globalLight", json!({"angle": -45.0, "altitude": 15.0})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, -45.0);
        assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 15.0);

        let d_id2 = open(&mut app, "layer.layerStyle.globalLight").unwrap();
        let fields2 = &app.ui.dialog_mut(d_id2).unwrap().fields;
        assert_eq!(fields2["angle"], json!(-45.0));
        assert_eq!(fields2["altitude"], json!(15.0));
        app.ui.close_dialog(d_id2);

        // Switch back to doc1: dialog still reflects doc1's light (30 / 45).
        assert!(app.session.set_active(0));
        let d_id1 = open(&mut app, "layer.layerStyle.globalLight").unwrap();
        assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["angle"], json!(30.0));
        assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["altitude"], json!(45.0));

        // 7. Modifying the dialog updates the active document.
        app.ui.dialog_mut(d_id1).unwrap().fields.insert("angle".into(), json!(-60.0));
        app.ui.dialog_mut(d_id1).unwrap().fields.insert("altitude".into(), json!(20.0));
        crate::dialogs::confirm(&mut app, d_id1).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, -60.0);
        assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 20.0);

        // Switch to doc2: doc2 is untouched (-45 / 15).
        assert!(app.session.set_active(1));
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, -45.0);
        assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 15.0);
    }

    /// View › Proof Setup (#3055): Custom Proof Setup opens with the active document's current
    /// proof condition, preserving it on unchanged OK, ignoring stale remembered dialog values,
    /// tracking document switching, applying edits, and leaving the document unchanged on Cancel.
    #[test]
    fn proof_setup_dialog_reflects_active_document_proof_condition() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc1_id = app.session.active().unwrap().doc.id;

        // 0. A fresh document defaults to working-cmyk.
        let d_fresh = crate::menus::invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(app.ui.dialog_mut(d_fresh).unwrap().fields["profile"], json!("working-cmyk"));
        crate::dialogs::confirm(&mut app, d_fresh).unwrap();
        assert_eq!(app.session.color.proof(doc1_id).setup.name, "working-cmyk");

        // 1. Preset sets sRGB and turns Proof Colors on.
        crate::menus::invoke(&mut app, &ctx, "view.proofSetup.internetStandardRgb", Value::Null).unwrap();
        assert_eq!(app.session.color.proof(doc1_id).setup.name, "srgb");
        assert!(app.session.color.proof(doc1_id).enabled);

        // 2. Open Custom Proof Setup: starts with the active document's condition (sRGB).
        let d_id = crate::menus::invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].as_u64().unwrap();
        let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
        assert_eq!(fields["profile"], json!("srgb"));
        assert_eq!(fields["intent"], json!("relative"));
        assert_eq!(fields["bpc"], json!(true));
        assert_eq!(fields["simulatePaper"], json!(false));

        // 3. Unchanged OK preserves the document's condition and leaves Proof Colors enabled.
        crate::dialogs::confirm(&mut app, d_id).unwrap();
        let pv = app.session.color.proof(doc1_id);
        assert_eq!(pv.setup.name, "srgb");
        assert!(pv.enabled);

        // 4. Stale remembered values do not override the active document.
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(
                "view.proofSetup".into(),
                json!({
                    "profile": "display-p3",
                    "intent": "perceptual",
                    "bpc": false,
                    "simulatePaper": true,
                }),
            );
        });
        let d_id = open(&mut app, "view.proofSetup").unwrap();
        let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
        assert_eq!(fields["profile"], json!("srgb"), "reads active document, not remembered");
        assert_eq!(fields["intent"], json!("relative"));
        assert_eq!(fields["bpc"], json!(true));
        assert_eq!(fields["simulatePaper"], json!(false));

        // 5. Cancel preserves the document.
        app.ui.close_dialog(d_id);
        assert_eq!(app.session.color.proof(doc1_id).setup.name, "srgb");

        // 6. Multiple documents: each document initializes the dialog to its own condition.
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc2_id = app.session.active().unwrap().doc.id;
        assert_ne!(doc1_id, doc2_id);
        app.run("view.proofSetup", json!({"profile": "display-p3", "intent": "perceptual", "bpc": false, "simulatePaper": true})).unwrap();
        assert_eq!(app.session.color.proof(doc2_id).setup.name, "display-p3");

        let d_id2 = open(&mut app, "view.proofSetup").unwrap();
        let fields2 = &app.ui.dialog_mut(d_id2).unwrap().fields;
        assert_eq!(fields2["profile"], json!("display-p3"));
        assert_eq!(fields2["intent"], json!("perceptual"));
        assert_eq!(fields2["bpc"], json!(false));
        assert_eq!(fields2["simulatePaper"], json!(true));
        app.ui.close_dialog(d_id2);

        // Switch back to doc1: dialog still reflects doc1's sRGB.
        assert!(app.session.set_active(0));
        let d_id1 = open(&mut app, "view.proofSetup").unwrap();
        assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["profile"], json!("srgb"));

        // 7. Modifying the dialog updates the active document.
        app.ui.dialog_mut(d_id1).unwrap().fields.insert("profile".into(), json!("adobe-rgb-compat"));
        crate::dialogs::confirm(&mut app, d_id1).unwrap();
        assert_eq!(app.session.color.proof(doc1_id).setup.name, "adobe-rgb-compat");
        // doc2 is untouched
        assert_eq!(app.session.color.proof(doc2_id).setup.name, "display-p3");
    }

    #[test]
    fn indexed_counts_are_integer_json_while_dither_amounts_keep_their_decimals() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        open(&mut app, "image.mode.indexedColor").unwrap();
        let mut defaults = app.ui.dialogs.first().unwrap().fields.clone();
        let before = params_of(&defaults)["colors"].clone();
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut defaults)).textures_delta.clear();
        assert_eq!(before.as_u64(), Some(256));
        assert_eq!(params_of(&defaults)["colors"], before, "opening a preview must not schedule a second job just to normalize its count");
        let mut fields = Map::new();
        fields.insert("__command".into(), json!("image.mode.indexedColor"));
        fields.insert("colors".into(), json!(17));
        fields.insert("amount".into(), json!(37.5));
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut fields)).textures_delta.clear();
        let params = params_of(&fields);
        assert_eq!(params["colors"].as_u64(), Some(17));
        assert_eq!(params["amount"].as_f64(), Some(37.5));
    }

    /// #7400: Lens Blur's Source lists the document's alpha channels, by name, each choosable even
    /// when two share a name; `depthChannel` is set through that menu, not shown as a field, and
    /// like the other references it isn't remembered, so a remembered `channel` source opens as None.
    #[test]
    fn lens_blur_source_lists_the_alpha_channels() {
        let spec = photocraft_engine::commands::find(LENS_BLUR).unwrap().params;
        let params = parse_spec(spec);
        assert!(params.iter().any(|p| p.key == "depthChannel" && p.kind == Kind::Json), "depthChannel is no dialog field");
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
        app.run("channel.new", json!({"name": "Depth Map", "fill": "white"})).unwrap();
        let id = open(&mut app, LENS_BLUR).unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__channels"], json!(["Depth Map"]));
        // The menu's values are channel indices, so two channels with one name are both choosable.
        app.run("channel.new", json!({"name": "Depth Map", "fill": "black"})).unwrap();
        let id = open(&mut app, LENS_BLUR).unwrap();
        let names: Vec<String> = serde_json::from_value(app.ui.dialog_mut(id).unwrap().fields["__channels"].clone()).unwrap();
        assert_eq!(names, ["Depth Map", "Depth Map"]);
        assert_eq!((channel_choice(0), channel_choice(1)), ("channel:0".to_string(), "channel:1".to_string()));
        assert_eq!(channel_index(&json!(1), &names), Some(1), "the second one by its index");
        assert_eq!(channel_index(&json!("Depth Map"), &names), Some(0), "a name is the first so named, as the engine reads it");
        assert_eq!(channel_index(&json!(2), &names), None);

        app.session.prefs.edit(|p| {
            p.dialogs.insert(LENS_BLUR.into(), json!({"depthMap": "channel", "depthChannel": "Depth Map"}));
        });
        let id = open(&mut app, LENS_BLUR).unwrap();
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(f["depthMap"], "none", "no channel was remembered with it");
        assert!(f.get("depthChannel").is_none());
    }

    #[test]
    fn wide_positive_ranges_use_the_logarithmic_slider_path() {
        assert!(uses_logarithmic_slider(0.1, 1000.0), "Gaussian Blur radius");
        assert!(uses_logarithmic_slider(1.0, 9999.0));
        assert!(!uses_logarithmic_slider(1.0, 500.0));
        assert!(!uses_logarithmic_slider(0.0, 1000.0));
    }

    #[test]
    fn new_filters_have_dialogs_or_run_directly() {
        for id in [
            "filter.stylize.oilPaint",
            "filter.blur.lensBlur",
            "filter.blurGallery.irisBlur",
            "filter.other.custom",
            "filter.distort.displace",
            "filter.pixelate.mezzotint",
            "filter.render.lightingEffects",
            "filter.render.relight",
            "filter.other.colorToAlpha",
        ] {
            assert!(has_dialog(id), "{id}");
        }
        for id in ["filter.pixelate.facet", "filter.pixelate.fragment", "filter.video.ntscColors"] {
            assert!(!has_dialog(id), "{id}");
        }
    }

    #[test]
    fn every_filter_command_spec_parses() {
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.")) {
            let _ = parse_spec(c.params);
        }
        assert!(has_dialog("filter.blur.gaussianBlur"));
        assert!(!has_dialog("filter.stylize.findEdges"));
    }

    /// Select and Mask's Output To lists all six output targets.
    #[test]
    fn select_and_mask_output_lists_all_six_targets() {
        let spec = photocraft_engine::commands::find("select.refineEdge").unwrap().params;
        let p = parse_spec(spec).into_iter().find(|p| p.key == "output").unwrap();
        let Kind::Choice { options: choices, .. } = p.kind else { panic!("output is not a choice") };
        let labels: Vec<String> = choices.iter().map(|c| source_label(c)).collect();
        assert_eq!(labels, ["Selection", "Layer Mask", "New Layer", "New Layer With Mask", "New Document", "New Document With Mask"]);
    }

    #[test]
    fn preview_runs_engine_command_on_proxy() {
        let mut doc = Document::with_background(
            "p",
            photocraft_doc::Size::new(64, 64),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let bg = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[0.0, 0.0, 0.0, 1.0]);
        let out = preview_document(&doc, Some(bg), "filter.blur.gaussianBlur", &json!({"radius": 4.0}), 1).unwrap();
        let p = out.layers[0].surface().unwrap().pixel(32, 32);
        assert!(p[0] > 0.2 && p[0] < 0.8, "edge blurred: {p:?}");
        assert_eq!(label("wavelengthMin"), "Wavelength Min");
    }

    /// #7423 (review on #7799): re-editing a Displace smart filter and picking another open
    /// document as its map previews with that document, as OK applies it. The preview session held
    /// only the edited document, and the map's `mapDocument` sits inside the filter's `params`.
    #[test]
    fn smart_displace_preview_uses_the_map_document() {
        let mut s = photocraft_engine::Session::new();
        // Documents 0 and 1: a white and a black map, opened before the edited one.
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "black"})).unwrap();
        s.execute("file.new", json!({"width": 48, "height": 32, "background": "transparent"})).unwrap();
        s.edit("square", |doc, active| {
            let fmt = doc.pixel_format();
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surf.fill_rect(photocraft_geom::Rect::new(10, 10, 20, 12), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("filter.distort.displace", json!({"horizontal": 10, "vertical": 0, "mapDocument": 0})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let layer = s.active().unwrap().active_layer;
        let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
        for map in [1, 0] {
            let p = json!({"index": 0, "params": {"mapDocument": map}});
            let mut applied = photocraft_engine::Session::new();
            for d in &documents {
                applied.add_document((**d).clone(), None);
            }
            applied.execute(SMART_FILTER_SET_PARAMS, p.clone()).unwrap();
            let want = photocraft_compose::render(&applied.active().unwrap().doc, doc.bounds()).px;
            let preview = preview_document_for_target(
                &doc,
                layer,
                SMART_FILTER_SET_PARAMS,
                &p,
                1,
                None,
                photocraft_engine::channel_cmds::ChannelTarget::Composite,
                &documents,
                &ColorSettings::default(),
            )
            .unwrap_or_else(|| panic!("map {map}: no preview"));
            assert!(photocraft_compose::render(&preview, doc.bounds()).px == want, "map {map}: the preview differs from what OK applies");
        }
    }

    /// Match Color's preview reads the Source document from the open documents, as OK does: the
    /// preview session held only the edited document, so a Source showed no change (#4851).
    #[test]
    fn match_color_preview_uses_the_source_document() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};
        let solid = |name: &str, c: [f32; 4]| {
            let mut d = Document::with_background(name, Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
            d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 32), &c);
            d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 16, 32), &[c[0] * 0.5, c[1] * 0.5, c[2] * 0.5, 1.0]);
            d
        };
        // The source document is open before the edited one, so indices differ between sessions.
        let mut s = photocraft_engine::Session::new();
        s.add_document(solid("reference", [0.1, 0.3, 0.9, 1.0]), None);
        s.add_document(solid("texture", [0.9, 0.5, 0.2, 1.0]), None);
        let doc = s.active().unwrap().doc.clone();
        let layer = doc.layers[0].id;
        let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
        for (source, label) in [(0, "another document"), (1, "the edited document itself")] {
            let p = json!({"source": source});
            let mut applied = photocraft_engine::Session::new();
            for d in &documents {
                applied.add_document((**d).clone(), None);
            }
            applied.execute("image.adjustments.matchColor", p.clone()).unwrap();
            let want = applied.active().unwrap().doc.layers[0].surface().unwrap().pixel(24, 16);
            let preview = preview_document_for_target(
                &doc,
                Some(layer),
                "image.adjustments.matchColor",
                &p,
                1,
                None,
                photocraft_engine::channel_cmds::ChannelTarget::Composite,
                &documents,
                &ColorSettings::default(),
            )
            .unwrap_or_else(|| panic!("{label}: no preview"));
            let got = preview.layers[0].surface().unwrap().pixel(24, 16);
            assert!((0..3).all(|c| (got[c] - want[c]).abs() < 1.0 / 255.0 + 1e-4), "{label}: preview {got:?}, OK gives {want:?}");
        }
        // The preview is of the matched image: the orange texture took the reference's blue.
        let preview = preview_document_for_target(
            &doc,
            Some(layer),
            "image.adjustments.matchColor",
            &json!({"source": 0}),
            2,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &documents,
            &ColorSettings::default(),
        )
        .unwrap();
        let px = preview.layers[0].surface().unwrap().pixel(12, 8);
        assert!(px[2] > px[0], "proxy preview matched to the reference: {px:?}");
    }

    #[test]
    fn core_filter_previews_keep_minimum_settings_after_downsampling() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};
        use photocraft_geom::Rect;

        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::with_background("preview", Size::new(32, 32), ColorMode::Rgb, depth, Color::WHITE);
            let layer = doc.layers[0].id;
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 32), &[0.0, 0.0, 0.0, 1.0]);
            for (suffix, params) in [
                ("blur.boxBlur", json!({"radius":1})),
                ("blur.motionBlur", json!({"distance":1})),
                ("blur.surfaceBlur", json!({"radius":1})),
                ("noise.median", json!({"radius":1})),
                ("noise.dustAndScratches", json!({"radius":1})),
                ("pixelate.mosaic", json!({"cellSize":2})),
                ("stylize.emboss", json!({"height":1})),
                ("other.minimum", json!({"radius":0.2})),
                ("other.maximum", json!({"radius":0.2})),
                ("distort.wave", json!({"wavelengthMin":1,"wavelengthMax":2,"amplitudeMin":1,"amplitudeMax":1})),
            ] {
                let command = format!("filter.{suffix}");
                for k in [2, 4] {
                    let proxy = crate::proxy::proxy_document(&doc, k);
                    let expected = preview_document(&proxy, Some(layer), &command, &params, 1).unwrap();
                    let out = preview_document(&doc, Some(layer), &command, &params, k).unwrap_or_else(|| panic!("{command}: {depth:?}, proxy factor {k}"));
                    assert_eq!(
                        out.layers[0].surface().unwrap().read_region(proxy.bounds()),
                        expected.layers[0].surface().unwrap().read_region(proxy.bounds()),
                        "{command}: {depth:?}, proxy factor {k}"
                    );
                }
            }
        }
    }

    /// A serde type error from numbers the dialog sends as JSON floats (`1.0`) for an integer
    /// field ("invalid type: floating point `1.0`, expected u32"), as opposed to a legitimate
    /// refusal such as "no selection" or "needs a map".
    fn float_type_error(e: &str) -> bool {
        e.contains("invalid type: floating point")
    }

    /// #2501: every schema dialog, opened with its default fields and drawn once, runs its command
    /// (OK) and its live preview without a type error from integer fields sent as floats.
    #[test]
    fn every_dialog_default_runs_without_float_type_errors() {
        let ids: Vec<&str> = photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id)).collect();
        assert!(ids.len() > 100, "{} dialogs", ids.len());
        let mut failures = Vec::new();
        for id in ids {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 24, "height": 24})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            let Some(dialog) = open(&mut app, id) else { continue };
            let mut f = app.ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields.clone();
            egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
            let params = params_of(&f);
            let st = app.session.active().unwrap();
            let (doc, active) = ((*st.doc).clone(), st.active_layer);
            if let Err(e) = app.session.execute(id, params.clone()) {
                let e = e.to_string();
                if float_type_error(&e) {
                    failures.push(format!("{id} OK {params}: {e}"));
                }
            }
            // The preview swallows the error; rerun its params at full size to see it.
            if preview_document(&doc, active, id, &params, 1).is_none() {
                let mut s = photocraft_engine::Session::new();
                s.add_document(doc, None);
                if let Some(a) = active {
                    s.select_layer(a).unwrap();
                }
                if let Err(e) = s.execute(id, params.clone()).map_err(|e| e.to_string())
                    && float_type_error(&e)
                {
                    failures.push(format!("{id} preview {params}: {e}"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn colour_params_parse_with_and_without_a_default() {
        let p = parse_spec(r##"{"color":color=#ffffff,"vineColor":color,"bad":color=#zz}"##);
        assert_eq!(p[0].kind, Kind::Color(Some([1.0, 1.0, 1.0])));
        assert_eq!(p[1].kind, Kind::Color(None));
        assert_eq!(p[2].kind, Kind::Color(None));
    }

    /// Color to Alpha (#1576): the dialog starts at white and the GIMP-style thresholds, drawing it
    /// keeps those params, and its live preview makes the white of a Background transparent.
    #[test]
    fn color_to_alpha_dialog_defaults_draw_and_preview() {
        const ID: &str = "filter.other.colorToAlpha";
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let id = open(&mut app, ID).unwrap();
        let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
        assert_eq!(f.get("__preview"), Some(&json!(true)));
        let want = json!({"color": "#ffffff", "transparencyThreshold": 0.0, "opacityThreshold": 1.0});
        assert_eq!(params_of(&f), want);
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        assert_eq!(params_of(&f), want, "drawing the dialog keeps its params");

        let mut doc = Document::with_background(
            "p",
            photocraft_doc::Size::new(32, 32),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U16,
            photocraft_doc::Color::WHITE,
        );
        let bg = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 32), &[0.0, 0.0, 0.0, 1.0]);
        for k in [1, 2] {
            let out = preview_document(&doc, Some(bg), ID, &want, k).unwrap();
            let s = out.layers[0].surface().unwrap();
            assert_eq!(out.layers[0].name, "Layer 0", "k={k}");
            assert!(s.pixel(24 / k as i32, 4)[3] < 1e-3, "k={k}: white removed");
            assert_eq!(s.pixel(1, 4)[3], 1.0, "k={k}: black kept");
        }
    }

    #[test]
    fn preview_stays_inside_the_selection_at_every_proxy_factor() {
        use photocraft_doc::SampleType;
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc =
                Document::with_background("p", photocraft_doc::Size::new(64, 64), photocraft_doc::ColorMode::Rgb, depth, photocraft_doc::Color::gray(0.5));
            let bg = doc.layers[0].id;
            let mut sel = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
            sel.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[1.0]);
            doc.selection = Some(sel);
            for k in [1, 2, 4] {
                let out = preview_document(&doc, Some(bg), "filter.noise.addNoise", &json!({"amount": 100.0}), k).unwrap();
                let s = out.layers[0].surface().unwrap();
                let half = 32 / k as i32;
                let changed = |x0: i32, x1: i32| (0..64 / k as i32).any(|y| (x0..x1).any(|x| s.pixel(x, y) != doc.layers[0].surface().unwrap().pixel(0, 0)));
                assert!(changed(0, half), "{depth:?} k={k}: noise inside the selection");
                assert!(!changed(half, 64 / k as i32), "{depth:?} k={k}: nothing outside the selection");
            }
        }
    }

    #[test]
    fn define_pattern_asks_for_a_name() {
        // Edit › Define Pattern… named the pattern after the document without asking ("ask", then
        // "ask 2"). Photoshop's Pattern Name dialog starts from that name and lets you change it.
        let spec = photocraft_engine::commands::find("edit.definePattern").unwrap();
        assert_eq!(parse_spec(spec.params), vec![Param { key: "name".into(), kind: Kind::Text }, Param { key: "rect".into(), kind: Kind::Json }]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "name": "ask.jpg"})).unwrap();
        let ctx = egui::Context::default();
        let define = |app: &mut PhotocraftApp, name: Option<&str>| {
            crate::menus::invoke(app, &ctx, "edit.definePattern", Value::Null).unwrap();
            let d = app.ui.dialogs.last().expect("Define Pattern opens a dialog").clone();
            let shown = d.fields["name"].as_str().unwrap().to_string();
            if let Some(n) = name {
                app.ui.dialog_mut(d.id).unwrap().fields.insert("name".into(), json!(n));
            }
            crate::dialogs::confirm(app, d.id).unwrap();
            shown
        };
        assert_eq!(define(&mut app, Some("Bricks")), "ask", "prefilled with the document's name");
        assert_eq!(define(&mut app, None), "ask", "Bricks didn't take it");
        assert_eq!(define(&mut app, None), "ask 2", "numbered past the library");
        let names: Vec<&str> = app.session.patterns.items.iter().map(|p| p.name.as_str()).collect();
        for n in ["Bricks", "ask", "ask 2"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }
    }

    #[test]
    fn notes_after_a_spec_are_not_parameters() {
        // A note after the object, like Selective Color's "(per-range [c,m,y,k] arrays: …)", was
        // parsed as parameters: its commas split off junk fields and it glued itself onto the last
        // one, so "reds":json became a number field the dialog showed and sent.
        let p = parse_spec(r#"{"a":0..10=1,"b":json} (notes [x,y,z] here, and more) → {"c":id}"#);
        assert_eq!(p, vec![Param { key: "a".into(), kind: Kind::Range { min: 0.0, max: 10.0, default: 1.0 } }, Param { key: "b".into(), kind: Kind::Json }]);
        // Every command with a schema dialog has plain parameter keys.
        for c in photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id)) {
            for p in parse_spec(c.params) {
                assert!(p.key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'), "{}: junk parameter {:?}", c.id, p.key);
            }
        }
    }

    #[test]
    fn selective_color_dialog_ok_applies() {
        // With the junk `reds: 0` the dialog's OK failed (`reds` must be an array of 4 numbers).
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let id = open(&mut app, "image.adjustments.selectiveColor").unwrap();
        let fields = app.ui.dialogs.last().unwrap().fields.clone();
        assert!(fields.keys().all(|k| k.starts_with("__") || k.chars().all(|ch| ch.is_ascii_alphanumeric())), "{fields:?}");
        assert!(!fields.contains_key("reds"), "the per-range arrays are not dialog fields");
        app.ui.dialog_mut(id).unwrap().fields.insert("black".into(), json!(40.0));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.journal.last().map(|j| j.0.as_str()), Some("image.adjustments.selectiveColor"));
    }

    /// A filtered test image with structure at every scale (low-frequency waves plus a ramp).
    fn waves(n: u32) -> Document {
        let mut doc = Document::with_background(
            "w",
            photocraft_doc::Size::new(n, n),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let s = doc.layers[0].surface_mut().unwrap();
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let v = 0.5 + 0.4 * (x as f32 / 23.0).sin() * (y as f32 / 31.0).cos();
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, x as f32 / n as f32, 1.0]);
            }
        }
        doc
    }

    /// Mean absolute RGB difference of two documents, compared at 1/4 scale.
    fn diff_at_quarter(a: &Document, b: &Document) -> f32 {
        let quarter = |d: &Document| photocraft_compose::flatten(&crate::proxy::proxy_document(d, (d.size.width / 64).max(1)));
        let (a, b) = (quarter(a), quarter(b));
        assert_eq!(a.px.len(), b.px.len());
        a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.px.len() as f32
    }

    /// #2063: these filters drew their features in proxy pixels (noise, dots, dither, strokes,
    /// kernels, scan lines, contour lines, streaks, depths, a tree's segments), so a 1/4 preview
    /// showed them 4x larger than OK applied them. Their preview now runs at full resolution and
    /// matches the applied result. Dust & Scratches (#3265) is the mirror image of those: a proxy
    /// averages away the very specks the filter exists to remove, so its preview showed an
    /// untouched image at a blocky scale.
    #[test]
    fn fixed_size_filters_preview_at_the_scale_they_apply() {
        let doc = sharp(256);
        let gray = grayscale(&doc);
        let edge = json!([0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 4, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0]);
        for (cmd, p) in [
            ("filter.distort.ripple", json!({"amount": 300.0, "size": "large"})),
            ("filter.stylize.diffuse", json!({"mode": "normal"})),
            ("filter.stylize.oilPaint", json!({})),
            ("filter.render.flame", json!({})),
            ("filter.render.fibers", json!({})),
            ("filter.pixelate.mezzotint", json!({"type": "mediumDots"})),
            ("filter.noise.addNoise", json!({"amount": 12.5})),
            ("filter.noise.dustAndScratches", json!({"radius": 4.0, "threshold": 0.0})),
            ("filter.noise.reduceNoise", json!({})),
            ("filter.other.custom", json!({"kernel": edge})),
            ("filter.video.deInterlace", json!({})),
            ("filter.stylize.traceContour", json!({})),
            ("filter.stylize.wind", json!({"method": "blast"})),
            ("filter.stylize.extrude", json!({})),
            ("filter.render.tree", json!({"baseTreeType": 1})),
            ("filter.sharpen.sharpen", json!({})),
            ("filter.sharpen.sharpenMore", json!({})),
            ("filter.sharpen.sharpenEdges", json!({})),
            ("filter.stylize.findEdges", json!({})),
            ("filter.stylize.emboss", json!({})),
            ("filter.noise.despeckle", json!({})),
            ("filter.gallery.graphicPen", json!({})),
            ("filter.gallery.stainedGlass", json!({"cellSize": 10})),
            ("image.mode.indexedColor", json!({"colors": 16})),
            ("image.mode.bitmap", json!({"method": "diffusion"})),
        ] {
            let doc = if cmd == "image.mode.bitmap" { &gray } else { &doc };
            let bg = doc.layers[0].id;
            let applied = preview_document(doc, Some(bg), cmd, &p, 1).unwrap();
            let old = preview_document(doc, Some(bg), cmd, &p, 4).unwrap();
            assert!(diff_at_quarter(&applied, &old) > 0.005, "{cmd}: a 1/4 proxy differs from the result");
            let k = preview_factor(cmd, &p, 4);
            assert_eq!(k, 1, "{cmd}");
            let shown = preview_document(doc, Some(bg), cmd, &p, k).unwrap();
            assert!(diff_at_quarter(&applied, &shown) < 1e-4, "{cmd}: the preview is what OK applies");
        }
        // Without dithering, the colour-mode conversions keep the proxy.
        assert_eq!(preview_factor("image.mode.indexedColor", &json!({"dither": "none"}), 4), 4);
        assert_eq!(preview_factor("image.mode.bitmap", &json!({"method": "threshold"}), 4), 4);
    }

    /// #6530: the Content-Aware Fill preview runs at full resolution, for every Sampling choice,
    /// because its patch search finds different matches on a reduced copy.
    #[test]
    fn content_aware_fill_previews_at_full_resolution() {
        for params in [json!({}), json!({"sampling": "auto"}), json!({"sampling": "rectangular"})] {
            assert_eq!(preview_factor("edit.contentAwareFill", &params, 4), 1, "{params}");
        }
    }

    /// #2063: proxy previews scaled what isn't a length (Tree's canvas fraction, Lens Correction's
    /// perspective), clamped negative lengths to +0.1 (Offset left/up), left Shadows/Highlights'
    /// radii in document pixels and rounded Mosaic's 10 px cells to 3 proxy px (12 px).
    #[test]
    fn proxy_previews_scale_only_lengths() {
        let doc = waves(256);
        let bg = doc.layers[0].id;
        let error = |cmd: &str, p: Value| {
            let k = preview_factor(cmd, &p, 4);
            let applied = preview_document(&doc, Some(bg), cmd, &p, 1).unwrap();
            let shown = preview_document(&doc, Some(bg), cmd, &p, k).unwrap();
            (k, mean_abs(&every_kth(&applied, k), &every_kth(&shown, 1)))
        };
        for (cmd, p) in [
            ("filter.lensCorrection", json!({"vertical": -100.0, "horizontal": 60.0})),
            ("filter.other.offset", json!({"horizontal": -40, "vertical": -24, "undefinedAreas": "wrap"})),
            (
                "image.adjustments.shadowsHighlights",
                json!({"shadowAmount": 50.0, "shadowRadius": 30.0, "highlightAmount": 50.0, "highlightRadius": 30.0, "blackClip": 0.0, "whiteClip": 0.0}),
            ),
        ] {
            let (k, diff) = error(cmd, p);
            assert_eq!(k, 4, "{cmd} keeps the proxy");
            assert!(diff < 0.01, "{cmd}: the preview shows what OK applies ({diff})");
        }
        // Mosaic's cells divide evenly: 10 px at k = 2 is 5 proxy px; 40 px keeps k = 4.
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 10.0}), 4), 2);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 40.0}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 7.0}), 4), 1);
        assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": "x"}), 4), 4);
        let (k, diff) = error("filter.pixelate.mosaic", json!({"cellSize": 10.0}));
        assert!(k == 2 && diff < 0.01, "Mosaic: {diff}");
        // Tree's and Picture Frame's sizes are relative, not pixels (and not shown as "px").
        assert!(!is_pixel_param_of("filter.render.tree", "size") && !is_pixel_param_of("filter.render.pictureFrame", "size"));
        assert!(is_pixel_param_of("filter.stylize.extrude", "size"));
    }

    /// #218: Warp Text renders the type again from the layer model, which a reduced proxy doesn't
    /// scale: the preview showed the text k times too large. It is the full-size result, reduced.
    #[test]
    fn warp_text_preview_matches_the_full_size_result_on_a_reduced_proxy() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 4000, "height": 3000})).unwrap();
        s.execute("type.create", json!({"x": 300, "y": 1600, "text": "Warp", "size": 400})).unwrap();
        let st = s.active().unwrap();
        let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
        let p = json!({"style": "arc", "bend": 40.0});
        let k = preview_factor("type.warpText", &p, crate::proxy::preview_factor(&doc, true));
        assert!(k > 1, "the preview runs on a reduced proxy");
        let cache = |d: &Document| match &d.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => t.cache.clone().unwrap(),
            _ => panic!("a type layer"),
        };
        let full = crate::proxy::proxy_document(&preview_document(&doc, Some(layer), "type.warpText", &p, 1).unwrap(), k);
        let shown = preview_document(&doc, Some(layer), "type.warpText", &p, k).unwrap();
        assert_eq!(shown.size, full.size);
        assert_eq!(cache(&shown).content_bounds(), cache(&full).content_bounds());
        assert_eq!(photocraft_compose::flatten(&shown).px, photocraft_compose::flatten(&full).px);
    }

    /// #5766: Curves on a smart object adds a smart filter, which re-renders the source through
    /// the object's placement. On a reduced proxy that placement stayed in full-size document
    /// coordinates, so the preview showed the object `k` times too large until OK.
    #[test]
    fn smart_object_curves_preview_keeps_its_size_on_a_reduced_proxy() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let st = s.active().unwrap();
        let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
        let alpha = |d: &Document, x: i32, y: i32| match &d.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Smart(sm) => sm.cache.as_ref().unwrap().pixel(x, y).last().copied().unwrap(),
            _ => panic!("a smart object"),
        };
        let k = 4;
        let params = json!({"points": [[0, 20], [90, 150], [255, 230]]});
        let shown = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, k).unwrap();
        let full = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, 1).unwrap();
        let photocraft_doc::LayerContent::Smart(full_sm) = &full.layer(layer).unwrap().content else { panic!("a smart object") };
        let reference = photocraft_algo::resample::resize_surface(full_sm.cache.as_ref().unwrap(), 0.25, 0.25, photocraft_algo::resample::Resample::Bicubic);
        let photocraft_doc::LayerContent::Smart(sm) = &shown.layer(layer).unwrap().content else { panic!("a smart object") };
        assert_eq!(sm.smart_filters.len(), 1, "the preview adds the Curves smart filter");
        assert_eq!(shown.size, crate::proxy::proxy_document(&doc, k).size);
        // The 16 px square at (16, 16) is a 4 px square at (4, 4) on the proxy, not 16 px at (16, 16).
        // Bicubic reduction blends the square's boundary with transparency; compare all alpha
        // values with a reduced full-size result, and require opacity only in the interior.
        for y in 0..shown.size.height as i32 {
            for x in 0..shown.size.width as i32 {
                let expected = reference.pixel(x, y).last().copied().unwrap();
                let actual = alpha(&shown, x, y);
                assert!((actual - expected).abs() <= 1.0 / 255.0, "({x}, {y}): alpha {actual}, expected {expected}");
            }
        }
        for (x, y) in [(5, 5), (5, 6), (6, 6)] {
            assert!(alpha(&shown, x, y) > 0.99, "({x}, {y}) is inside the reduced square");
        }
        for (x, y) in [(4, 4), (7, 7)] {
            assert!(alpha(&shown, x, y) > 0.5, "({x}, {y}) is a corner of the reduced square");
        }
        for (x, y) in [(2, 2), (9, 9), (12, 12), (15, 15)] {
            assert!(alpha(&shown, x, y) < 0.01, "({x}, {y}) is outside the reduced square");
        }
    }

    /// #2063: a pixel-sized parameter divided by the proxy factor was clamped to the command's
    /// minimum (Pointillize's 5 px cells became 3 px proxy cells = 12 px), so the preview factor
    /// drops until every pixel-sized value fits its range.
    #[test]
    fn proxy_previews_never_clamp_pixel_sized_params() {
        let pointillize = |c: f64| preview_factor("filter.pixelate.pointillize", &json!({"cellSize": c}), 4);
        assert_eq!((pointillize(5.0), pointillize(7.0), pointillize(12.0), pointillize(300.0)), (1, 2, 4, 4));
        assert_eq!(preview_factor("filter.pixelate.crystallize", &json!({"cellSize": 10.0}), 4), 3);
        assert_eq!(preview_factor("filter.pixelate.colorHalftone", &json!({"maxRadius": 8.0}), 4), 2);
        assert_eq!(preview_factor("filter.blur.gaussianBlur", &json!({"radius": 8.0}), 4), 4, "unclamped filters keep the proxy");
        // Hostile params never panic and leave the factor alone; a full-size document stays at 1.
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": "x"}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!(null), 4), 4);
        assert_eq!(preview_factor("no.such.command", &json!({"cellSize": 1.0}), 4), 4);
        assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": 5.0}), 0), 1);
        // Every previewed command, at each pixel-sized param's minimum, default and in between.
        let previewed = photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id) && (c.id.starts_with("filter.") || PREVIEWED.contains(&c.id)));
        for c in previewed {
            for p in parse_spec(c.params) {
                let Kind::Range { min, default, .. } = p.kind else { continue };
                if min <= 0.0 || !is_pixel_param_of(c.id, &p.key) {
                    continue;
                }
                for v in [f64::from(min), f64::from(default), f64::from(min) * 2.5] {
                    let k = preview_factor(c.id, &json!({ p.key.clone(): v }), 4);
                    assert!(v / f64::from(k) >= f64::from(min) - 1e-9, "{} {}={v}: k={k} clamps to {min}", c.id, p.key);
                }
            }
        }
    }

    /// Sharp structure at every scale: [`waves`] with hard squares from 2 to 32 px, a fine
    /// per-pixel texture and an interlaced band (odd rows inverted) at the bottom.
    fn sharp(n: u32) -> Document {
        let mut doc = waves(n);
        let s = doc.layers[0].surface_mut().unwrap();
        let mut x0 = 4;
        for (i, size) in [2, 3, 4, 6, 8, 12, 16, 24, 32].into_iter().enumerate() {
            for y0 in (4..n as i32 * 3 / 4 - size).step_by((size * 3) as usize) {
                let v = if (i + y0 as usize / 7).is_multiple_of(2) { 0.05 } else { 0.95 };
                s.fill_rect(photocraft_geom::Rect::new(x0, y0, x0 + size, y0 + size), &[v, v * 0.8, 1.0 - v, 1.0]);
            }
            x0 += size + 6;
        }
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let mut px = s.pixel(x, y);
                if (x * 7 + y * 13) % 5 == 0 {
                    px.iter_mut().take(3).for_each(|c| *c = (*c + 0.08).min(1.0));
                }
                if y >= n as i32 * 3 / 4 && y % 2 == 1 {
                    px[0] = 1.0 - px[0];
                }
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
            }
        }
        doc
    }

    /// [`waves`] with strong per-pixel noise, the input of noise reduction.
    fn noisy(n: u32) -> Document {
        let mut doc = waves(n);
        let s = doc.layers[0].surface_mut().unwrap();
        let hash = |x: i32, y: i32| ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 8;
        for y in 0..n as i32 {
            for x in 0..n as i32 {
                let mut px = s.pixel(x, y);
                for (c, v) in px.iter_mut().take(3).enumerate() {
                    *v = (*v + (hash(x * 3 + c as i32, y) as f32 / (1u32 << 24) as f32 - 0.5) * 0.3).clamp(0.0, 1.0);
                }
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
            }
        }
        doc
    }

    /// How a preview at factor `k` compares with what OK applies (factor 1) on a document.
    #[derive(Debug)]
    struct PreviewError {
        /// Mean absolute RGB difference at the pixels the proxy samples.
        diff: f32,
        /// Feature scale of the preview shown at document size, relative to the result: 1 when
        /// the features match, 1/k when the preview draws them k times too large.
        scale: f32,
        /// How much the preview changes the image relative to how much OK does (sampled); NaN
        /// when OK barely changes it.
        change: f32,
    }

    /// Mean absolute difference of neighbouring pixels at the document's own resolution.
    fn detail(d: &Document) -> f32 {
        let b = photocraft_compose::flatten(d);
        let w = b.rect.width() as usize;
        let mut sum = 0.0;
        for (i, p) in b.px.iter().enumerate() {
            for q in [b.px.get(i + 1).filter(|_| (i + 1) % w != 0), b.px.get(i + w)].into_iter().flatten() {
                sum += (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0;
            }
        }
        sum / b.px.len().max(1) as f32
    }

    /// Pixels of `d` at every `k`-th row and column: the document pixels a factor-`k` proxy
    /// samples, so a preview at `k` compares with them one to one.
    fn every_kth(d: &Document, k: u32) -> Vec<[f32; 4]> {
        let b = photocraft_compose::flatten(d);
        let w = b.rect.width() as usize;
        let k = k as usize;
        let h = b.px.len() / w.max(1);
        (0..h.div_ceil(k))
            .flat_map(|y| (0..w.div_ceil(k)).map(move |x| (x * k, y * k)))
            .map(|(x, y)| b.px.get(y * w + x).copied().unwrap_or([f32::NAN; 4]))
            .collect()
    }

    fn mean_abs(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        a.iter().zip(b).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.len().max(1) as f32
    }

    /// `None` when the command can't run on `doc` at all (e.g. Displace without a map).
    fn preview_error(doc: &Document, command: &str, p: &Value, k: u32) -> Option<PreviewError> {
        let bg = doc.layers[0].id;
        let applied = preview_document(doc, Some(bg), command, p, 1)?;
        let shown = preview_document(doc, Some(bg), command, p, k).unwrap_or_else(|| panic!("{command} {p}: the k = {k} preview failed"));
        let (a, s) = (every_kth(&applied, k), every_kth(&shown, 1));
        let (src, src_k) = (every_kth(doc, k), every_kth(&crate::proxy::proxy_document(doc, k), 1));
        let applied_change = mean_abs(&a, &src);
        Some(PreviewError {
            diff: mean_abs(&a, &s),
            scale: detail(&shown) / k as f32 / detail(&applied).max(1e-5),
            change: if applied_change > 0.01 { mean_abs(&s, &src_k) / applied_change } else { f32::NAN },
        })
    }

    /// The parameter sets to preview `command` with: the dialog's defaults, and each pixel-sized
    /// value at its minimum and at a large value (whole numbers as integers: Tree and Picture
    /// Frame read some ranges as `u32`).
    fn sweep_params(app: &mut PhotocraftApp, command: &str) -> Vec<(String, Value)> {
        let Some(id) = open(app, command) else { return Vec::new() };
        let base = params_of(&app.ui.dialog_mut(id).unwrap().fields);
        let mut out = vec![("default".to_string(), base.clone())];
        for p in parse_spec(photocraft_engine::commands::find(command).unwrap().params) {
            if !is_pixel_param_of(command, &p.key) {
                continue;
            }
            let values = match p.kind {
                Kind::Int { .. } => vec![-40.0, 40.0],
                Kind::Range { min, max, default } => [min, (default * 4.0).clamp(min, max.min(64.0))].into_iter().filter(|v| *v != default).collect(),
                _ => continue,
            };
            for v in values {
                let mut q = base.clone();
                q[p.key.as_str()] = json!(v);
                out.push((format!("{}={v}", p.key), q));
            }
        }
        for (_, p) in &mut out {
            for v in p.as_object_mut().into_iter().flat_map(|o| o.values_mut()) {
                if let Some(x) = v.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e9) {
                    *v = json!(x as i64);
                }
            }
        }
        out
    }

    /// Every previewed command with its parameter sets.
    fn previewed_cases() -> Vec<(&'static str, String, Value)> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ids: Vec<&'static str> = photocraft_engine::command_specs()
            .iter()
            .map(|c| c.id)
            .filter(|id| has_dialog(id) && (id.starts_with("filter.") || PREVIEWED.contains(id)))
            .collect();
        ids.into_iter().flat_map(|id| sweep_params(&mut app, id).into_iter().map(move |(label, p)| (id, label, p))).collect()
    }

    fn grayscale(d: &Document) -> Document {
        let mut s = photocraft_engine::Session::new();
        s.add_document(d.clone(), None);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        s.active().map(|d| (*d.doc).clone()).unwrap()
    }

    /// Smooth, sharp and noisy test images, each in RGB and grayscale (for the modes that need it).
    fn test_images(n: u32) -> [(&'static str, Document, Document); 3] {
        let with_gray = |d: Document| (grayscale(&d), d);
        let ((ws, w), (ss, s), (ns, no)) = (with_gray(waves(n)), with_gray(sharp(n)), with_gray(noisy(n)));
        [("smooth", w, ws), ("sharp", s, ss), ("noisy", no, ns)]
    }

    fn image_for<'a>(command: &str, rgb: &'a Document, gray: &'a Document) -> &'a Document {
        if command == "image.mode.bitmap" || command == "image.mode.duotone" { gray } else { rgb }
    }

    /// Previews that only approximate the result at a proxy although their features have the
    /// right size: dots and cells placed on the proxy's pixel grid (Color Halftone, Pointillize)
    /// and tile edges resampled (Tiles). Two more only differ on a proxy as small as this test's
    /// (64 x 64 px): Picture Frame's minimum line widths, and Shadows/Highlights' 0.01 % black and
    /// white clips, which pick single pixels out of 4096 (its radii are checked in
    /// `proxy_previews_scale_only_lengths`).
    const APPROXIMATE_AT_PROXY: &[&str] = &[
        "filter.pixelate.colorHalftone",
        "filter.pixelate.pointillize",
        "filter.stylize.tiles",
        "filter.render.pictureFrame",
        "image.adjustments.shadowsHighlights",
    ];

    /// Why a preview doesn't show what OK applies, if it doesn't: a clearly different result or
    /// features at another size on the smooth image, or (on the sharp and noisy images, where a
    /// sub-pixel radius can't show on a proxy) a filter that acts more strongly in the preview,
    /// i.e. over a k times larger area.
    fn mismatch(image: &str, e: &PreviewError) -> Option<String> {
        let off = if image == "smooth" { e.diff > 0.03 || (!(0.6..=1.6).contains(&e.scale) && e.diff > 0.002) } else { e.change > 1.25 };
        off.then(|| format!("{e:?}"))
    }

    /// #2063, every previewed command: the live preview at the factor [`preview_factor`] picks
    /// for a 1/4 proxy shows what OK applies: the same features at the same size, on smooth,
    /// sharp and noisy images, at the defaults and at small and large pixel sizes.
    #[test]
    fn every_preview_shows_what_ok_applies() {
        let cases: Vec<_> = previewed_cases().into_iter().filter(|(id, _, p)| preview_factor(id, p, 4) > 1 && !APPROXIMATE_AT_PROXY.contains(id)).collect();
        // One thread per test image: about 270 filter runs at 256 x 256.
        let bad: Vec<String> = std::thread::scope(|scope| {
            let runs: Vec<_> = test_images(256)
                .into_iter()
                .map(|(image, rgb, gray)| {
                    let cases = &cases;
                    scope.spawn(move || {
                        let mut bad = Vec::new();
                        for (id, label, p) in cases {
                            let k = preview_factor(id, p, 4);
                            if let Some(why) = preview_error(image_for(id, &rgb, &gray), id, p, k).and_then(|e| mismatch(image, &e)) {
                                bad.push(format!("{id} ({label}) at k = {k} on the {image} image: {why}"));
                            }
                        }
                        bad
                    })
                })
                .collect();
            runs.into_iter().flat_map(|r| r.join().unwrap()).collect()
        });
        assert!(bad.is_empty(), "previews that differ from the result:\n{}", bad.join("\n"));
    }

    /// The table behind [`every_preview_shows_what_ok_applies`]: every previewed command at a
    /// plain 1/4 proxy and at its chosen factor, on each test image.
    /// `cargo test -p photocraft-ui-egui --lib filter_dialog::tests::sweep_all_previews -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sweep_all_previews() {
        let images = test_images(256);
        println!("command | params | image | k | diff k=4 | scale k=4 | change k=4 | diff k | scale k | change k");
        for (id, label, p) in previewed_cases() {
            let k = preview_factor(id, &p, 4);
            for (image, rgb, gray) in &images {
                let doc = image_for(id, rgb, gray);
                match (preview_error(doc, id, &p, 4), preview_error(doc, id, &p, k)) {
                    (Some(a), Some(b)) => println!(
                        "{id} | {label} | {image} | {k} | {:.4} | {:.2} | {:.2} | {:.4} | {:.2} | {:.2}",
                        a.diff, a.scale, a.change, b.diff, b.scale, b.change
                    ),
                    _ => println!("{id} | {label} | {image} | {k} | fails"),
                }
            }
        }
    }

    /// The profiles Assign Profile offers for a new document converted with `mode` (if any),
    /// and which of the command's built-in profiles the command itself accepts for it.
    fn assign_profile_offers(mode: Option<&str>) -> (Vec<String>, Vec<String>) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        if let Some(mode) = mode {
            app.run(mode, json!({})).unwrap();
        }
        open(&mut app, "edit.assignProfile").unwrap();
        let offered: Vec<String> =
            app.ui.dialogs.last().unwrap().fields["__profiles"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        let spec = photocraft_engine::commands::find("edit.assignProfile").unwrap();
        let builtin = parse_spec(spec.params).into_iter().find(|p| p.key == "profile").unwrap();
        let Kind::Choice { options, .. } = builtin.kind else { panic!("profile is a menu") };
        let accepted = options.into_iter().filter(|name| app.session.execute("edit.assignProfile", json!({"profile": name})).is_ok()).collect();
        (offered, accepted)
    }

    /// #4951: Assign Profile lists the profiles the command accepts for the document's colour
    /// mode, and none of the others.
    #[test]
    fn assign_profile_offers_only_profiles_of_the_documents_colour_mode() {
        for mode in [None, Some("image.mode.cmyk"), Some("image.mode.grayscale")] {
            let (offered, accepted) = assign_profile_offers(mode);
            for name in &accepted {
                assert!(offered.contains(name), "{mode:?}: {name} can be assigned, so it is offered");
            }
            for name in ["srgb", "coated-cmyk", "sgray"] {
                assert_eq!(offered.iter().any(|o| o == name), accepted.iter().any(|a| a == name), "{mode:?}: {name}");
            }
            assert!(offered.iter().any(|o| o == "working") && offered.iter().any(|o| o == "none"), "{mode:?}: working space and none always fit");
        }
        let (rgb, _) = assign_profile_offers(None);
        assert!(rgb.iter().any(|o| o == "srgb") && !rgb.iter().any(|o| o == "coated-cmyk"), "an RGB document gets RGB profiles");
        let (cmyk, _) = assign_profile_offers(Some("image.mode.cmyk"));
        assert!(cmyk.iter().any(|o| o == "coated-cmyk") && !cmyk.iter().any(|o| o == "srgb"), "a CMYK document gets CMYK profiles");
    }

    /// #4951: a profile remembered from a document of another colour mode isn't preselected.
    #[test]
    fn assign_profile_drops_a_remembered_profile_that_does_not_fit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("edit.assignProfile".into(), json!({"profile": "coated-cmyk"}));
        });
        open(&mut app, "edit.assignProfile").unwrap();
        assert_eq!(app.ui.dialogs.last().unwrap().fields["profile"], json!("working"));
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("edit.assignProfile".into(), json!({"profile": "display-p3"}));
        });
        open(&mut app, "edit.assignProfile").unwrap();
        assert_eq!(app.ui.dialogs.last().unwrap().fields["profile"], json!("display-p3"), "one that fits is kept");
    }

    #[test]
    fn schema_dialogs_respect_explicit_dropdown_defaults() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());

        // 1. Radial Blur has explicit default "good", not the first option ("draft").
        open(&mut app, "filter.blur.radialBlur").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["quality"], json!("good"), "Radial Blur must select explicit default 'good'");
        assert_eq!(fields["method"], json!("spin"), "Radial Blur method has no explicit default, retains first option");
        let params = params_of(fields);
        assert_eq!(params["quality"], json!("good"));

        // 2. Convert to Profile has explicit default "relative", not first option ("perceptual").
        open(&mut app, "edit.convertToProfile").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["intent"], json!("relative"), "Convert to Profile must select explicit default 'relative'");
        let params = params_of(fields);
        assert_eq!(params["intent"], json!("relative"));

        // 3. Proof Setup has explicit default "relative".
        open(&mut app, "view.proofSetup").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["intent"], json!("relative"), "Proof Setup must select explicit default 'relative'");
        let params = params_of(fields);
        assert_eq!(params["intent"], json!("relative"));

        // 4. Remembered values still override initial defaults.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert("filter.blur.radialBlur".into(), json!({"quality": "best"}));
        });
        open(&mut app, "filter.blur.radialBlur").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["quality"], json!("best"), "Remembered choice must override explicit default");

        // 5. Explicit default that is the first option (e.g. "cw" in arbitrary rotation).
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        open(&mut app, "image.rotation.arbitrary").unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(fields["direction"], json!("cw"), "Explicit default matching first option is preserved");
    }

    /// #4143: Replace Color shows the From colour (defaulting to the foreground) and its canvas
    /// eyedropper samples the merged image into the dialog.
    #[test]
    fn replace_color_eyedropper_samples_the_image() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "background": "#00ff00"})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        let id = open(&mut app, REPLACE_COLOR).unwrap();
        assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#ff0000"), "the dialog starts from the foreground colour");
        // #6279: armed as it opens, so the first click on the image samples.
        assert!(color_eyedropper_armed(&app));
        assert!(color_eyedropper_sample(&mut app, 4.0, 4.0));
        assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#00ff00"));
        // #4457: where it was picked, for Localized Color Clusters.
        assert_eq!(app.ui.dialogs.last().unwrap().fields["points"], json!([[4.0, 4.0]]));
        // Off-image: the click is consumed, nothing changes and the eyedropper stays armed.
        assert!(color_eyedropper_sample(&mut app, -1.0, 4.0));
        assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#00ff00"));
        assert!(color_eyedropper_armed(&app));
        // Disarmed, the dialog never claims the click.
        app.ui.dialog_mut(id).unwrap().fields.insert(COLOR_EYEDROPPER.into(), json!(false));
        assert!(!color_eyedropper_armed(&app));
        assert!(!color_eyedropper_sample(&mut app, 4.0, 4.0));
    }

    /// #4143: the eyedropper button disarms and re-arms the canvas picker (#6279: it opens armed).
    #[test]
    fn replace_color_dialog_arms_the_eyedropper_button() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        open(h.state_mut(), REPLACE_COLOR).unwrap();
        h.run_steps(3);
        assert!(color_eyedropper_armed(h.state()), "armed as the dialog opens");
        h.get_by_label("Pick Color From Image").click();
        h.run_steps(2);
        assert!(!color_eyedropper_armed(h.state()));
        h.get_by_label("Pick Color From Image").click();
        h.run_steps(2);
        assert!(color_eyedropper_armed(h.state()));
    }

    /// #6278: the Result swatch shows the sampled colour after the shift, and a colour picked for
    /// it sets Hue, Saturation and Lightness so the sampled colour becomes it.
    #[test]
    fn replace_color_result_swatch_sets_the_sliders() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.run("tools.setColors", json!({"foreground": "#cc3333"})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
        h.run_steps(3);
        // No shift yet: the Result is the sampled colour.
        assert_eq!(h.get_all_by_label("#cc3333").count(), 2, "the sampled colour and the Result both read #cc3333");
        h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Result").click();
        h.run_steps(3);
        let picker = h.state().ui.dialogs.last().unwrap();
        assert_ne!(picker.id, id, "the Color Picker opened over the dialog");
        assert_eq!(picker.fields["color"], json!("#cc3333"), "on the current Result");
        let picker = picker.id;
        h.state_mut().ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#3355cc"));
        crate::dialogs::confirm(h.state_mut(), picker).unwrap();
        h.state_mut().ui.dialogs.retain(|d| d.id != picker);
        h.run_steps(3);
        let f = h.state_mut().ui.dialog_mut(id).unwrap().fields.clone();
        let n = |k: &str| f[k].as_f64().unwrap() as f32;
        let got = photocraft_algo::tone::replace_color_result([0.8, 0.2, 0.2], n("hue"), n("saturation"), n("lightness"));
        let want = crate::color_picker_ui::parse_hex("#3355cc").unwrap();
        assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.02), "the sliders turn the sampled colour into the pick: {got:?}");
        assert!(n("hue") != 0.0, "the hue moved");
        // Moving a slider afterwards is not mistaken for another pick.
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("lightness".into(), json!(10.0));
        h.run_steps(2);
        assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["lightness"], json!(10.0));
    }

    /// #4457: Localized Color Clusters and its Range, as in Photoshop's dialog; the sampled point
    /// and both settings reach the command.
    #[test]
    fn replace_color_localized_color_clusters() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        // Left half red, right half red too: localized on the left point, the right half stays.
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 10, "background": "#ff0000"})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
        h.run_steps(3);
        let _ = h.get_by_label("Localized Color Clusters");
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(COLOR_EYEDROPPER.into(), json!(true));
        assert!(color_eyedropper_sample(h.state_mut(), 2.0, 5.0));
        h.get_by_label("Localized Color Clusters").click();
        h.run_steps(2);
        let f = h.state().ui.dialogs.last().unwrap().fields.clone();
        assert_eq!((f["localized"].clone(), f["points"].clone()), (json!(true), json!([[2.0, 5.0]])));
        // The command with the dialog's fields, small range and a hue shift: only near (2, 5).
        let mut p: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
        p.insert("range".into(), json!(10));
        p.insert("hue".into(), json!(120));
        h.state_mut().run(REPLACE_COLOR, Value::Object(p)).unwrap();
        let doc = &h.state().session.active().unwrap().doc;
        let px = |x: i32| doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(x, 5, x + 1, 6));
        assert!(px(2)[1] > 0.5, "near the point: shifted to green {:?}", px(2));
        assert!(px(38)[0] > 0.9 && px(38)[1] < 0.1, "far away: still red {:?}", px(38));
    }
}
