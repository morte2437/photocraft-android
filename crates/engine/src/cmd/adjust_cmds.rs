//! Image › Adjustments beyond the per-pixel adjustment-layer kinds:
//!
//! * parameter parsing for Selective Color and Color Lookup (both also adjustment layers; their
//!   commands are generated with the other kinds in `commands.rs`),
//! * the destructive-only adjustments: Shadows/Highlights, Replace Color, Match Color and HDR
//!   Toning (maths in `photocraft_algo::tone`).
//!
//! Destructive adjustments convert the target (layer pixels, or a targeted alpha channel /
//! Quick Mask) to straight RGBA, so every colour model and bit depth is handled, and blend the
//! result through the selection like the other Image › Adjustments commands.

use photocraft_algo::tone::{self, HdrToning, MatchColor, ShadowsHighlights};
use photocraft_doc::adjust::{LutFormat, LutSource};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Rect};
use photocraft_raster::{Surface, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod replace_color_range;

/// Built-in Color Lookup looks `(id, label)`.
pub use craft_cms::lutfile::BUILTIN as LOOKS;

/// Selective Color range keys in storage order.
pub const RANGES: [&str; 9] = ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"];

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn num(p: &Value, key: &str, default: f32) -> f32 {
    p.get(key).and_then(Value::as_f64).map_or(default, |v| v as f32)
}

/// `key`'s value; a null counts as missing (as in `adjust_params`).
fn given<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}

fn opt_str<'a>(cmd: &str, p: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match given(p, key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a string"))),
    }
}

fn opt_bool(cmd: &str, p: &Value, key: &str) -> Result<Option<bool>> {
    match given(p, key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(cmd, format!("`{key}` must be true or false"))),
    }
}

/// A percentage in -100..100 (clamped); anything but a finite number is an error.
fn percent(cmd: &str, key: &str, v: &Value) -> Result<f32> {
    let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
    Ok((x as f32).clamp(-100.0, 100.0))
}

/// Selective Color from params. Accepts per-range arrays (`"reds":[c,m,y,k]` in percent) and the
/// dialog's single-range form (`"colors":"reds","cyan":…,"magenta":…,"yellow":…,"black":…`);
/// `"method":"relative|absolute"`. Unspecified ranges keep `base`'s values; a key of the wrong
/// type is an error.
pub fn selective_from_params(p: &Value, base: Option<&Adjustment>) -> Result<Adjustment> {
    const CMD: &str = "selectiveColor";
    let (mut relative, mut adj) = match base {
        Some(Adjustment::SelectiveColor { relative, adjustments }) => (*relative, *adjustments),
        _ => (true, [[0.0; 4]; 9]),
    };
    match opt_str(CMD, p, "method")? {
        None => {}
        Some("relative") => relative = true,
        Some("absolute") => relative = false,
        Some(m) => return Err(bad(CMD, format!("`method` must be \"relative\" or \"absolute\", not `{m}`"))),
    }
    if let Some(b) = opt_bool(CMD, p, "relative")? {
        relative = b;
    }
    for (row, key) in adj.iter_mut().zip(RANGES) {
        let Some(v) = given(p, key) else { continue };
        let a = v.as_array().filter(|a| a.len() == 4).ok_or_else(|| bad(CMD, format!("`{key}` must be an array of 4 numbers [c, m, y, k]")))?;
        for (slot, x) in row.iter_mut().zip(a) {
            *slot = percent(CMD, key, x)?;
        }
    }
    if let Some(c) = opt_str(CMD, p, "colors")? {
        let Some((row, _)) = adj.iter_mut().zip(RANGES).find(|(_, r)| *r == c) else {
            return Err(bad(CMD, format!("unknown range `{c}` (ranges: {})", RANGES.join(", "))));
        };
        for (slot, key) in row.iter_mut().zip(["cyan", "magenta", "yellow", "black"]) {
            if let Some(v) = given(p, key) {
                *slot = percent(CMD, key, v)?;
            }
        }
    }
    Ok(Adjustment::SelectiveColor { relative, adjustments: adj })
}

/// Color Lookup from params: `"lut"` (a built-in look id or `"none"`), `"file"` (a .cube / .3dl /
/// .look path, native only) or `"data"` (file text, or base64 with `"dataEncoding":"base64"`).
/// `"lutFormat":"cube|3dl|look"` (`"auto"`: detect) can supply the format when the file name doesn't identify it.
/// `"interpolation":"trilinear|tetrahedral"`, `"dither":bool`. The table is embedded in the
/// layer, as Photoshop does. Unspecified fields keep `base`'s values; a key of the wrong type is an
/// error.
pub fn lookup_from_params(p: &Value, base: Option<&Adjustment>) -> Result<Adjustment> {
    const CMD: &str = "colorLookup";
    let (mut name, mut lut, mut size, mut tetrahedral, mut dither, mut source) = match base {
        Some(Adjustment::ColorLookup { name, lut, size, tetrahedral, dither, source }) => {
            (name.clone(), lut.clone(), *size, *tetrahedral, *dither, source.clone())
        }
        _ => (String::new(), None, 0, false, false, None),
    };
    let encoding = opt_str(CMD, p, "dataEncoding")?.unwrap_or("text");
    if !matches!(encoding, "text" | "base64") {
        return Err(bad(CMD, "`dataEncoding` must be \"text\" or \"base64\""));
    }
    let format = match opt_str(CMD, p, "lutFormat")? {
        None | Some("auto") => None,
        Some("cube") => Some(craft_cms::lutfile::Format::Cube),
        Some("3dl") => Some(craft_cms::lutfile::Format::ThreeDl),
        Some("look") => Some(craft_cms::lutfile::Format::Look),
        Some(_) => return Err(bad(CMD, "`lutFormat` must be \"auto\", \"cube\", \"3dl\" or \"look\"")),
    };
    // The parsed table, its label and (for a file) the file as loaded, which a PSD save embeds.
    let mut loaded: Option<(craft_cms::lutfile::LutFile, String, Option<LutSource>)> = None;
    if let Some(id) = opt_str(CMD, p, "lut")? {
        if id == "none" || id.is_empty() {
            name.clear();
            lut = None;
            size = 0;
            source = None;
        } else {
            let f = craft_cms::lutfile::builtin(id).ok_or_else(|| bad(CMD, format!("unknown look `{id}` (built-ins: {})", builtin_ids())))?;
            let label = f.title.clone();
            loaded = Some((f, label, None));
        }
    }
    let file_name = opt_str(CMD, p, "fileName")?.filter(|s| !s.is_empty());
    if let Some(text) = opt_str(CMD, p, "data")?.filter(|s| !s.is_empty()) {
        let file_name = file_name.unwrap_or("lut.cube");
        let bytes = if encoding == "base64" {
            photocraft_paint::tile::b64_decode(text).ok_or_else(|| bad(CMD, "`data` is not valid base64"))?
        } else {
            text.as_bytes().to_vec()
        };
        let format = format.unwrap_or_else(|| craft_cms::lutfile::Format::detect(file_name, &bytes));
        let f = craft_cms::lutfile::parse_as(format, &bytes).map_err(|e| bad(CMD, e.0))?;
        loaded = Some((f, base_name(file_name), Some(lut_source(format, bytes))));
    }
    if let Some(path) = opt_str(CMD, p, "file")?.filter(|s| !s.is_empty()) {
        let bytes = read_file(path).ok_or_else(|| bad(CMD, format!("can't read {path}")))?;
        let format = format.unwrap_or_else(|| craft_cms::lutfile::Format::detect(path, &bytes));
        let f = craft_cms::lutfile::parse_as(format, &bytes).map_err(|e| bad(CMD, format!("{path}: {}", e.0)))?;
        loaded = Some((f, base_name(path), Some(lut_source(format, bytes))));
    }
    if let Some((f, label, src)) = loaded {
        if !f.domain_is_default() {
            return Err(bad(
                CMD,
                format!(
                    "`{}` declares DOMAIN_MIN {:?} / DOMAIN_MAX {:?}: Color Lookup applies the plain 0..1 domain, so the look would shift; honouring a custom domain is not supported yet",
                    label, f.domain_min, f.domain_max
                ),
            ));
        }
        name = label;
        size = f.size as u32;
        lut = Some(std::sync::Arc::new(f.data));
        source = src;
    }
    match opt_str(CMD, p, "interpolation")? {
        None => {}
        Some("trilinear") => tetrahedral = false,
        Some("tetrahedral") => tetrahedral = true,
        Some(i) => return Err(bad(CMD, format!("`interpolation` must be \"trilinear\" or \"tetrahedral\", not `{i}`"))),
    }
    if let Some(b) = opt_bool(CMD, p, "tetrahedral")? {
        tetrahedral = b;
    }
    if let Some(b) = opt_bool(CMD, p, "dither")? {
        dither = b;
    }
    Ok(Adjustment::ColorLookup { name, lut, size, tetrahedral, dither, source })
}

/// A loaded LUT file, tagged with the format [`craft_cms::lutfile::parse`] read it as.
fn lut_source(format: craft_cms::lutfile::Format, bytes: Vec<u8>) -> LutSource {
    let format = match format {
        craft_cms::lutfile::Format::Cube => LutFormat::Cube,
        craft_cms::lutfile::Format::ThreeDl => LutFormat::ThreeDl,
        craft_cms::lutfile::Format::Look => LutFormat::Look,
    };
    LutSource { format, bytes: std::sync::Arc::new(bytes) }
}

fn builtin_ids() -> String {
    craft_cms::lutfile::BUILTIN.iter().map(|b| b.0).collect::<Vec<_>>().join(", ")
}

fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Most LUT files [`folder_luts`] lists.
const MAX_FOLDER_LUTS: usize = 500;
/// Largest LUT file [`folder_luts`] lists: a 129³ `.cube` (the parser's largest) is about 60 MB.
const MAX_FOLDER_LUT_BYTES: u64 = 64 << 20;

/// The `.cube`, `.3dl` and `.look` files directly in `dir`, as (file name, path) sorted by name,
/// for Color Lookup's dropdown (Preferences › File Handling › LUT Folder, #6558). An empty,
/// missing or unreadable folder lists nothing; hidden files, subfolders and files too large to be
/// a LUT are skipped. A symlink to a LUT file is listed, as a studio's shared library often is.
/// Contents are only parsed when one is picked, which reports a malformed file.
#[cfg(not(target_arch = "wasm32"))]
pub fn folder_luts(dir: &str) -> Vec<(String, String)> {
    let dir = dir.trim();
    let Some(entries) = (!dir.is_empty()).then(|| std::fs::read_dir(dir).ok()).flatten() else { return Vec::new() };
    let mut out: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
            if name.starts_with('.') || !matches!(ext.as_str(), "cube" | "3dl" | "look") {
                return None;
            }
            let path = e.path();
            let meta = std::fs::metadata(&path).ok()?;
            (meta.is_file() && meta.len() <= MAX_FOLDER_LUT_BYTES).then_some((name, path.to_str()?.to_string()))
        })
        .collect();
    out.sort_by_key(|(name, _)| name.to_lowercase());
    out.truncate(MAX_FOLDER_LUTS);
    out
}

/// No file system on the web: a LUT folder lists nothing there.
#[cfg(target_arch = "wasm32")]
pub fn folder_luts(_: &str) -> Vec<(String, String)> {
    Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Option<Vec<u8>> {
    crate::allocation::read_file_within(path, MAX_FOLDER_LUT_BYTES).ok()
}
#[cfg(target_arch = "wasm32")]
fn read_file(_: &str) -> Option<Vec<u8>> {
    None
}

/// `layer.setAdjustment` for the kinds edited one range / option at a time: params merge into the
/// current values; a call with no parameters resets to defaults.
pub fn update_adjustment(existing: &Adjustment, p: &Value) -> Option<Result<Adjustment>> {
    let empty = p.as_object().is_none_or(|o| o.keys().all(|k| matches!(k.as_str(), "layer" | "coalesce" | "__kind")));
    match existing {
        Adjustment::SelectiveColor { .. } => Some(selective_from_params(p, (!empty).then_some(existing))),
        Adjustment::ColorLookup { .. } => Some(lookup_from_params(p, (!empty).then_some(existing))),
        _ => None,
    }
}

// ---------- destructive helpers ----------

/// Runs `f` on the target surface's content as straight RGBA (`w × h` = content bounds), blending
/// the result through the selection. Refused before reading when the dense buffers over the
/// content bounds (which span the gaps between distant tiles) would pass the working budget.
fn process_surface(surf: &mut Surface, selection: Option<&Surface>, f: &mut dyn FnMut(&mut Vec<[f32; 4]>, Rect)) -> Result<()> {
    let r = surf.content_bounds();
    if r.is_empty() {
        return Ok(());
    }
    let fmt = surf.format();
    let n = fmt.channels();
    // The native read and the blended output (`n` f32 each), and the edited and original RGBA.
    crate::allocation::check_dense(r, (n as u64).saturating_mul(8).saturating_add(32), "this adjustment")?;
    let raw = surf.read_region(r);
    let mut px: Vec<[f32; 4]> = raw.chunks_exact(n).map(|q| to_rgba(&fmt, q)).collect();
    let orig = px.clone();
    f(&mut px, r);
    surf.write_region(r, &crate::pixels::mix_through_selection(&fmt, r, &orig, &px, selection));
    Ok(())
}

/// One history step running `f` on the active pixel layer (or targeted channel or layer mask).
fn rgba_edit(s: &mut Session, label: &str, p: &Value, mut f: impl FnMut(&mut Vec<[f32; 4]>, Rect)) -> Result<Value> {
    let id = if p.get("layer").is_some() { Some(crate::commands::layer_param(s, p)?) } else { crate::commands::layer_param(s, p).ok() };
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, id, p)? {
                process_surface(surf, sel.as_ref(), &mut f)?;
                surf.prune();
            }
            Ok(Value::Null)
        });
    }
    let id = id.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        process_surface(surf, sel.as_ref(), &mut f)?;
        Ok(Value::Null)
    })
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

/// Image › Adjustments targets: a pixel layer, a smart object (the adjustment is recorded as a
/// smart filter, as in Photoshop), or a targeted alpha channel / Quick Mask.
pub(crate) fn has_adjustable(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    match &l.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(sm) if sm.cache.is_some() => Ok(()),
        other => Err(format!("active layer is {} {} layer, not a pixel layer", other.article(), other.kind_name())),
    }
}

/// The smart object an adjustment's pixels would go to: the layer the params resolve to (the
/// active one by default), when the command edits its pixels and that layer is a smart object.
/// `None` for a mask or channel target, or a layer that is not a smart object.
pub(crate) fn smart_pixel_target(s: &Session, p: &Value) -> Result<Option<photocraft_doc::LayerId>> {
    if crate::commands::is_mask_target(p) || crate::channel_cmds::is_channel_target(p) || crate::color_channels::target(s, p)?.is_some() {
        return Ok(None);
    }
    let Ok(id) = crate::commands::layer_param(s, p) else { return Ok(None) };
    let smart = s.active().is_some_and(|d| matches!(d.doc.layer(id).map(|l| &l.content), Some(LayerContent::Smart(_))));
    Ok(smart.then_some(id))
}

/// Image › Adjustments on a smart object record `image.adjustments.<kind>` as a smart filter
/// (Photoshop): the adjustment re-runs from the source whenever the object re-renders, through
/// [`adjust_as_filter`]. `Ok(true)` when it was recorded (one history step, the selection becomes
/// the filter mask); `Ok(false)` when the command should edit pixels instead (a mask or channel
/// target, or the active layer is not a smart object).
pub(crate) fn adjust_as_smart_filter(s: &mut Session, kind: &str, label: &str, p: &Value) -> Result<bool> {
    let Some(id) = smart_pixel_target(s, p)? else { return Ok(false) };
    // A pixel or full lock refuses the smart filter, as for `filter.*` on a smart object.
    crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    // The recorded params are the command's own, minus the engine's private `__kind` key.
    let mut params = p.clone();
    if let Value::Object(m) = &mut params {
        m.remove("__kind");
    }
    let sf = photocraft_doc::SmartFilter {
        command: format!("image.adjustments.{kind}"),
        params,
        blend: photocraft_color::BlendMode::Normal,
        opacity: 1.0,
        visible: true,
    };
    let sel = s.active().and_then(|d| d.doc.selection.clone());
    s.edit(label, |doc, _| crate::smart_cmds::add_smart_filter(doc, id, sf, sel.as_ref()))?;
    Ok(true)
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

// ---------- commands ----------

/// Image › Adjustments run as a smart filter (`image.adjustments.<kind>` recorded on a smart
/// object): the adjustment applied to a copy of `surf`. `None` for kinds or params it can't run.
pub(crate) fn adjust_as_filter(kind: &str, p: &Value, surf: &Surface, histogram_mask: Option<&photocraft_doc::LayerMask>) -> Result<Option<Surface>> {
    // A Color Lookup filter re-reads its LUT `file`, a path stored in the document: followed only
    // as `doc_path` allows, else the filter can't run (like a missing file).
    let resolved;
    let p = match p.get("file").and_then(Value::as_str).filter(|f| kind == "colorLookup" && !f.is_empty()) {
        Some(file) => {
            let mut q = p.clone();
            let Some(path) = crate::doc_path::resolve(file).ok().and_then(|p| p.to_str().map(str::to_owned)) else { return Ok(None) };
            q["file"] = json!(path);
            resolved = q;
            &resolved
        }
        None => p,
    };
    let mut out = surf.clone();
    if kind == "replaceColor" {
        let Ok((c, fuzz, h, sat, l)) = replace_color_params("image.adjustments.replaceColor", p, [0.0, 0.0, 0.0, 1.0]) else { return Ok(None) };
        process_surface(&mut out, None, &mut |px, _| {
            tone::replace_color(px, [c[0], c[1], c[2]], fuzz, h, sat, l);
        })?;
    } else if kind == "desaturate" {
        let mode = surf.format().mode;
        if crate::pixels::adjust_surface(&mut out, &crate::commands::desaturate_adjustment(), None, mode).is_err() {
            return Ok(None);
        }
    } else if kind == "equalize" {
        process_surface(&mut out, None, &mut |px, area| {
            let width = area.width() as usize;
            let cover =
                (0..px.len()).map(|i| histogram_mask.map_or(1.0, |m| m.value(area.x0 + (i % width) as i32, area.y0 + (i / width) as i32))).collect::<Vec<_>>();
            crate::extra_cmds::equalize_pixels(px, &cover);
        })?;
    } else if kind == "shadowsHighlights" {
        let sh = shadows_highlights_params(p);
        process_surface(&mut out, None, &mut |px, r| tone::shadows_highlights(px, r.width() as usize, r.height() as usize, &sh))?;
    } else {
        let mode = surf.format().mode;
        let Ok(adj) = crate::adjust_params::from_params(kind, p, None, mode) else { return Ok(None) };
        if crate::pixels::adjust_surface(&mut out, &adj, None, mode).is_err() {
            return Ok(None);
        }
    }
    out.prune();
    Ok(Some(out))
}

fn shadows_highlights(s: &mut Session, p: &Value) -> Result<Value> {
    if adjust_as_smart_filter(s, "shadowsHighlights", "Shadows/Highlights", p)? {
        return Ok(Value::Null);
    }
    let sh = shadows_highlights_params(p);
    rgba_edit(s, "Shadows/Highlights", p, |px, r| tone::shadows_highlights(px, r.width() as usize, r.height() as usize, &sh))
}

fn shadows_highlights_params(p: &Value) -> ShadowsHighlights {
    let d = ShadowsHighlights::default();
    ShadowsHighlights {
        shadow_amount: num(p, "shadowAmount", d.shadow_amount).clamp(0.0, 100.0),
        shadow_tone: num(p, "shadowTone", d.shadow_tone).clamp(0.0, 100.0),
        shadow_radius: num(p, "shadowRadius", d.shadow_radius).clamp(0.0, 2500.0),
        highlight_amount: num(p, "highlightAmount", d.highlight_amount).clamp(0.0, 100.0),
        highlight_tone: num(p, "highlightTone", d.highlight_tone).clamp(0.0, 100.0),
        highlight_radius: num(p, "highlightRadius", d.highlight_radius).clamp(0.0, 2500.0),
        color: num(p, "color", d.color).clamp(-100.0, 100.0),
        midtone: num(p, "midtone", d.midtone).clamp(-100.0, 100.0),
        black_clip: num(p, "blackClip", d.black_clip).clamp(0.0, 50.0),
        white_clip: num(p, "whiteClip", d.white_clip).clamp(0.0, 50.0),
    }
}

fn replace_color_params(cmd: &str, p: &Value, default_color: [f32; 4]) -> Result<([f32; 4], f32, f32, f32, f32)> {
    match given(p, "color") {
        None => {}
        Some(Value::String(s)) if crate::commands::parse_hex(s).is_some() => {}
        Some(Value::Array(a)) if (3..=4).contains(&a.len()) && a.iter().all(Value::is_number) => {}
        Some(_) => return Err(bad(cmd, "`color` must be a hex colour or an array of 3 or 4 finite numbers")),
    }
    let c = crate::commands::color_param(cmd, p, "color", default_color)?;
    let fuzz = num(p, "fuzziness", 40.0).clamp(0.0, 200.0);
    let (h, sat, l) = (num(p, "hue", 0.0).clamp(-180.0, 180.0), num(p, "saturation", 0.0).clamp(-100.0, 100.0), num(p, "lightness", 0.0).clamp(-100.0, 100.0));
    Ok((c, fuzz, h, sat, l))
}

fn replace_color(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.replaceColor";
    let localized_requested = p.get("localized").and_then(Value::as_bool).unwrap_or(false);
    // Only the smart object's own pixels are restricted: a mask, alpha channel or Quick Mask over
    // it edits as for any layer.
    let on_smart = smart_pixel_target(s, p)?.is_some();
    // Recorded Lab bounds use native profile matching and edit pixels in place; ordinary RGB
    // samples keep their existing path. A smart object has no pixels to edit that way.
    if on_smart && (p.get("labMinimum").is_some() || p.get("labMaximum").is_some()) {
        return Err(bad(CMD, "Lab bounds require a pixel layer, not a smart object"));
    }
    if let Some(result) = replace_color_range::apply(s, p)? {
        return Ok(result);
    }
    if localized_requested && on_smart {
        return Err(bad(CMD, "Localized Color Clusters are not available as a smart filter yet"));
    }
    // Several samples (Add/Subtract from Sample) edit pixels through Color Range's matcher; the
    // smart filter replays one colour only, so recording it would silently drop the extra samples.
    if on_smart {
        let multi = p.get("points").and_then(Value::as_array).is_some_and(|a| a.len() > 1);
        let subtracted = p.get("subtractPoints").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        if multi || subtracted {
            return Err(bad(CMD, "Add/Subtract from Sample are not available as a smart filter yet"));
        }
    }
    let (c, fuzz, h, sat, l) = replace_color_params(CMD, p, s.tools.foreground)?;
    if !localized_requested {
        // Resolve and validate before recording, including the null foreground fallback.
        let mut q = Value::Object(p.as_object().cloned().unwrap_or_default());
        if let Value::Object(m) = &mut q {
            m.insert("color".into(), json!(c));
        }
        if adjust_as_smart_filter(s, "replaceColor", "Replace Color", &q)? {
            return Ok(Value::Null);
        }
    }
    // Localized Color Clusters, as in Select › Color Range: the colour only matches within Range
    // (a percentage of the canvas's longer side) of where it was picked.
    let points = sample_points(p).map_err(|m| bad(CMD, m))?;
    // Several samples (Add to Sample / Subtract from Sample, #6280): the colours match as
    // Select › Color Range's do for the same clicks, through its own matcher.
    // A malformed `subtractPoints` is refused here, as malformed `points` are, rather than read as
    // no subtraction by the single-colour path.
    let subtracted = match p.get("subtractPoints") {
        None => false,
        Some(v) => !v.as_array().ok_or_else(|| bad(CMD, "`subtractPoints` must be [[x, y], …]"))?.is_empty(),
    };
    if subtracted || points.len() > 1 {
        let mut q = json!({"points": p.get("points").cloned().unwrap_or(Value::Array(Vec::new())), "fuzziness": fuzz});
        // With no Add samples, the subtraction starts from the dialog's From colour, not the
        // foreground; with them, the samples alone span the range, as in Color Range.
        // Forwarded as resolved (a `null` or missing colour is the foreground), as RGB 0..1.
        if points.is_empty() {
            q["color"] = json!([c[0], c[1], c[2]]);
        }
        for key in ["subtractPoints", "order", "localized", "range"] {
            if let Some(v) = p.get(key) {
                q[key] = v.clone();
            }
        }
        let samples = crate::selection_cmds::ColorRangeSamples::new(s, &q).map_err(|e| bad(CMD, e.to_string()))?;
        return rgba_edit(s, "Replace Color", p, |px, r| {
            let mask = samples.coverage(px, r.width() as usize, 1.0, [r.x0 as f32, r.y0 as f32]);
            tone::replace_color_masked(px, &mask, h, sat, l);
        });
    }
    let localized = if p.get("localized").and_then(Value::as_bool).unwrap_or(false) {
        if points.is_empty() {
            return Err(bad(CMD, "localized clusters need eyedropper `points` (where the colour was picked)"));
        }
        let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
        Some(num(p, "range", 100.0).clamp(0.0, 100.0) / 100.0 * canvas.width().max(canvas.height()) as f32)
    } else {
        None
    };
    rgba_edit(s, "Replace Color", p, |px, r| match localized {
        Some(radius) => {
            let samples: Vec<_> = points
                .iter()
                .map(|&(x, y)| photocraft_algo::selection::RangeSample { color: [c[0], c[1], c[2]], at: Some((x - r.x0 as f32, y - r.y0 as f32)) })
                .collect();
            let mask = photocraft_algo::selection::color_range_samples(px, r.width() as usize, &samples, fuzz, Some(radius));
            tone::replace_color_masked(px, &mask, h, sat, l);
        }
        None => {
            tone::replace_color(px, [c[0], c[1], c[2]], fuzz, h, sat, l);
        }
    })
}

/// The `points` of a command (`[[x, y], …]` in document pixels), or a message saying what is wrong.
fn sample_points(p: &Value) -> std::result::Result<Vec<(f32, f32)>, String> {
    let Some(pts) = p.get("points") else { return Ok(Vec::new()) };
    let pts = pts.as_array().ok_or("`points` must be [[x, y], …]")?;
    pts.iter()
        .map(|pt| {
            pt.as_array()
                .and_then(|a| match a.as_slice() {
                    [x, y] => Some((x.as_f64()? as f32, y.as_f64()? as f32)),
                    _ => None,
                })
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .ok_or_else(|| format!("bad point {pt} in `points` (want [x, y])"))
        })
        .collect()
}

/// Straight RGBA pixels and an optional coverage mask for a document's layer (or its composite).
/// Pixels plus an optional coverage mask.
type Sample = (Vec<[f32; 4]>, Option<Vec<f32>>);

fn stats_pixels(doc: &Document, layer: Option<u64>, use_selection: bool) -> Result<Sample> {
    let area = doc.bounds();
    let px: Vec<[f32; 4]> = match layer {
        Some(id) => {
            let l = doc.layer(photocraft_doc::LayerId(id)).ok_or(EngineError::NoLayer(photocraft_doc::LayerId(id)))?;
            photocraft_compose::render_layer(l, area).into_px()
        }
        None => photocraft_compose::flatten(doc).into_px(),
    };
    let mask = doc.selection.as_ref().filter(|_| use_selection).map(|sel| photocraft_algo::selection::mask_from_surface(Some(sel), area));
    Ok((px, mask))
}

fn match_color(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "image.adjustments.matchColor";
    let o = MatchColor {
        luminance: num(p, "luminance", 100.0).clamp(1.0, 200.0),
        intensity: num(p, "intensity", 100.0).clamp(1.0, 200.0),
        fade: num(p, "fade", 0.0).clamp(0.0, 100.0),
        neutralize: p.get("neutralize").and_then(Value::as_bool).unwrap_or(false),
    };
    let source = match p.get("source").and_then(Value::as_i64) {
        Some(i) if i >= 0 => {
            let d = s.documents().get(i as usize).ok_or_else(|| bad(CMD, format!("no document {i}")))?;
            let layer = p.get("sourceLayer").and_then(Value::as_u64);
            let (px, mask) = stats_pixels(&d.doc, layer, p.get("useSelectionInSource").and_then(Value::as_bool).unwrap_or(false))?;
            Some(tone::lab_stats(&px, mask.as_deref()).ok_or_else(|| bad(CMD, "the source has no opaque pixels"))?)
        }
        _ => None,
    };
    // Source "None" with default options matches the image to its own statistics: a no-op, as in
    // Photoshop (Neutralize / Luminance / Color Intensity still act).
    // Target statistics: the layer itself, restricted to the selection when asked.
    let target_sel = p.get("useSelectionInTarget").and_then(Value::as_bool).unwrap_or(false);
    let sel = s.active().and_then(|d| d.doc.selection.clone());
    rgba_edit(s, "Match Color", p, |px, r| {
        let mask: Option<Vec<f32>> = sel.as_ref().filter(|_| target_sel).map(|sel| photocraft_algo::selection::mask_from_surface(Some(sel), r));
        let Some(target) = tone::lab_stats(px, mask.as_deref()) else {
            return;
        };
        tone::match_color(px, &target, source.as_ref(), &o);
    })
}

fn hdr_toning(s: &mut Session, p: &Value) -> Result<Value> {
    let d = HdrToning::default();
    let curve: Vec<(f32, f32)> = p
        .get("curve")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| Some((v.get(0)?.as_f64()? as f32 / 255.0, v.get(1)?.as_f64()? as f32 / 255.0))).collect())
        .unwrap_or_default();
    let h = HdrToning {
        radius: num(p, "radius", d.radius).clamp(1.0, 500.0),
        strength: num(p, "strength", d.strength).clamp(0.1, 4.0),
        gamma: num(p, "gamma", d.gamma).clamp(0.1, 2.0),
        exposure: num(p, "exposure", d.exposure).clamp(-5.0, 5.0),
        detail: num(p, "detail", d.detail).clamp(-100.0, 300.0),
        shadow: num(p, "shadow", d.shadow).clamp(-100.0, 100.0),
        highlight: num(p, "highlight", d.highlight).clamp(-100.0, 100.0),
        vibrance: num(p, "vibrance", d.vibrance).clamp(-100.0, 100.0),
        saturation: num(p, "saturation", d.saturation).clamp(-100.0, 100.0),
        curve,
    };
    // Like Photoshop, HDR Toning flattens the image first (one history step).
    s.edit("HDR Toning", |doc, active| {
        let buf = photocraft_compose::flatten(doc).over_background([1.0, 1.0, 1.0]);
        let (w, hgt) = (buf.rect.width() as usize, buf.rect.height() as usize);
        let mut px = buf.into_px();
        tone::hdr_toning(&mut px, w, hgt, &h);
        let fmt = doc.pixel_format();
        let data: Vec<f32> = px.iter().flat_map(|q| photocraft_raster::from_rgba(&fmt, *q)).collect();
        let mut bg = Layer::raster("Background", fmt);
        bg.locks.transparency = true;
        bg.locks.position = true;
        crate::pixels_mut(&mut bg)?.write_region(doc.bounds(), &data);
        *active = Some(bg.id);
        doc.layers = vec![bg];
        doc.selection = None;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The Color Lookup built-ins (id, label) for UIs and agents.
fn list_looks(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!(craft_cms::lutfile::BUILTIN.iter().map(|(id, label)| json!({"id": id, "label": label})).collect::<Vec<_>>()))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.adjustments.shadowsHighlights",
            "Shadows/Highlights…",
            ["Image", "Adjustments"],
            r##"{"shadowAmount":0..100=35,"shadowTone":0..100=50,"shadowRadius":0..2500=30,"highlightAmount":0..100=0,"highlightTone":0..100=50,"highlightRadius":0..2500=30,"color":-100..100=20,"midtone":-100..100=0,"blackClip":0..50=0.01,"whiteClip":0..50=0.01}"##,
            has_adjustable,
            shadows_highlights
        ),
        spec!(
            "image.adjustments.replaceColor",
            "Replace Color…",
            ["Image", "Adjustments"],
            r##"{"localized":bool=false,"color":color,"fuzziness":0..200=40,"range":0..100=100,"hue":-180..180=0,"saturation":-100..100=0,"lightness":-100..100=0,"points":[[x,y],…]?,"subtractPoints":[[x,y],…]?,"order":["+"|"-",…]?,"labMinimum":[L,a,b]?,"labMaximum":[L,a,b]?} (Lab bounds: a finite ordered D50 box, mutually exclusive with color/points/subtractPoints/order/localized/range; composite pixel-layer target only. Otherwise color ("#rrggbb") defaults to the foreground; localized points use range % of the longer side; several "points" (Add to Sample) or any "subtractPoints" (Subtract from Sample) match as select.colorRange does for the same samples, "order" the click order.)"##,
            has_adjustable,
            replace_color
        ),
        spec!(
            "image.adjustments.matchColor",
            "Match Color…",
            ["Image", "Adjustments"],
            r##"{"source":doc,"sourceLayer":json,"luminance":1..200=100,"intensity":1..200=100,"fade":0..100=0,"neutralize":bool=false,"useSelectionInSource":bool=false,"useSelectionInTarget":bool=false} (source: document index; sourceLayer: layer id, default the merged image)"##,
            has_pixels,
            match_color
        ),
        spec!(
            "image.adjustments.hdrToning",
            "HDR Toning…",
            ["Image", "Adjustments"],
            r##"{"radius":1..500=30,"strength":0.1..4=0.5,"gamma":0.1..2=1,"exposure":-5..5=0,"detail":-100..300=30,"shadow":-100..100=0,"highlight":-100..100=0,"vibrance":-100..100=0,"saturation":-100..100=20,"curve":[[in,out],…] 0..255} (flattens the image)"##,
            has_doc,
            hdr_toning
        ),
        CommandSpec {
            id: "image.adjustments.blackWhite.auto",
            label: "Black & White Auto",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"filter":index?} → {"reds","yellows","greens","cyans","blues","magentas"} (%: the Auto mix for the image a Black & White adjustment layer sees, a smart object's pixels below its smart filter `filter`, or a pixel layer's own pixels; nothing changes)"##,
            enabled: has_doc,
            run: black_white_auto,
            journal: false,
        },
        CommandSpec {
            id: "image.adjustments.colorLookup.list",
            label: "List Color Lookup Looks",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: list_looks,
            journal: false,
        },
    ]
}

/// At most this many pixels are sampled for [`auto_black_white`].
const AUTO_BW_SAMPLES: usize = 1 << 16;
/// Grey levels [`auto_black_white`] tells apart.
const AUTO_BW_BINS: usize = 32;

/// Black & White's Auto (#9269): the colour weights (%, in [`crate::adjust_params::HUE_RANGES`]
/// order) that spread the image's greys over the most grey levels. PhotoCraft's own search, not
/// Photoshop's: from the default mix, each weight in turn steps through 0..=150 % to the value
/// that most raises the entropy of the greys' histogram ([`AUTO_BW_BINS`] levels), less a pull
/// back towards the default, until a pass changes nothing. So colours the default mix turns into
/// the same grey are told apart, with the mildest change that does it; colours the image lacks
/// keep their default weight.
pub fn auto_black_white(px: &[[f32; 3]]) -> [f32; 6] {
    let defaults = crate::adjust_params::BW_DEFAULTS;
    let score = |w: &[f32; 6]| -> f32 {
        if px.is_empty() {
            return 0.0;
        }
        let mut bins = [0u32; AUTO_BW_BINS];
        for c in px {
            let g = photocraft_compose::adjust::black_white_gray(*c, w).clamp(0.0, 1.0);
            if let Some(b) = bins.get_mut(((g * AUTO_BW_BINS as f32) as usize).min(AUTO_BW_BINS - 1)) {
                *b += 1;
            }
        }
        let n = px.len() as f32;
        let entropy: f32 = bins.iter().filter(|&&k| k > 0).map(|&k| k as f32 / n).map(|q| -q * q.log2()).sum();
        let pull: f32 = w.iter().zip(defaults).map(|(v, d)| (v - d).abs()).sum::<f32>() / 100.0;
        entropy - 0.05 * pull
    };
    let mut w = defaults;
    let mut best = score(&w);
    for _ in 0..4 {
        let mut changed = false;
        for k in 0..6 {
            for step in 0..=15 {
                let mut t = w;
                t[k] = step as f32 * 10.0;
                let s = score(&t);
                if s > best + 1e-6 {
                    (best, w, changed) = (s, t, true);
                }
            }
        }
        if !changed {
            break;
        }
    }
    w
}

/// At most [`AUTO_BW_SAMPLES`] of the pixels with alpha: one from each run of `stride` of them, at
/// a position that varies from run to run (Fibonacci hashing of the run's index), so a pattern
/// whose period divides `stride` (alternating columns on an even-width canvas) isn't only ever
/// sampled in one phase.
fn auto_bw_sample(rgba: &[[f32; 4]]) -> Vec<[f32; 3]> {
    let covered = || rgba.iter().filter(|c| c[3] > 0.0);
    let stride = covered().count().div_ceil(AUTO_BW_SAMPLES).max(1);
    let pick = |run: usize| ((run as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize % stride;
    covered().enumerate().filter(|(i, _)| i % stride == pick(i / stride)).map(|(_, c)| [c[0], c[1], c[2]]).collect()
}

/// Hides `id` and everything composited above it. Returns whether `id` is in `layers`.
fn hide_from(layers: &mut [Layer], id: photocraft_doc::LayerId) -> bool {
    let mut found = false;
    for l in layers.iter_mut() {
        if found || l.id == id {
            found = true;
            l.visible = false;
        } else if l.children_mut().is_some_and(|c| hide_from(c, id)) {
            found = true;
        }
    }
    found
}

/// `image.adjustments.blackWhite.auto`: [`auto_black_white`] for `layer` (default the active one):
/// the composite of what lies below a Black & White adjustment layer (what it adjusts), a smart
/// object's pixels below smart filter `filter` (what that filter edits), or a pixel layer's own
/// pixels (what Image › Adjustments › Black & White changes). As in the reference application,
/// the whole canvas counts: a selection, the adjustment's mask and its clip base don't narrow it.
fn black_white_auto(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.adjustments.blackWhite.auto";
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?,
        Some(v) => photocraft_doc::LayerId(v.as_u64().ok_or_else(|| bad(cmd, "`layer` must be a layer id"))?),
    };
    let filter = match p.get("filter") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "`filter` must be a smart filter index"))?),
    };
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let area = st.doc.bounds();
    // Rendering or reading a canvas past the render limit gives nothing to sample: say so rather
    // than answer with the default mix.
    if u64::from(area.width()) * u64::from(area.height()) > photocraft_compose::MAX_RENDER_PIXELS {
        return Err(bad(cmd, "the canvas is too large to analyse"));
    }
    let read = |surf: &Surface| -> Vec<[f32; 4]> {
        let fmt = surf.format();
        surf.read_region(area).chunks_exact(fmt.channels()).map(|c| to_rgba(&fmt, c)).collect()
    };
    let rgba: Vec<[f32; 4]> = match (&layer.content, filter) {
        (LayerContent::Smart(sm), Some(i)) => {
            if i >= sm.smart_filters.len() {
                return Err(bad(cmd, format!("layer \"{}\" has no smart filter {i}", layer.name)));
            }
            let below = crate::smart_cmds::render_below_filter(&st.doc, sm, i)?
                .ok_or_else(|| EngineError::Other("the smart object's contents are unavailable".into()))?;
            read(&below)
        }
        (_, Some(_)) => return Err(bad(cmd, "`filter` needs a smart object layer")),
        (LayerContent::Adjustment(_), None) => {
            let mut below = (*st.doc).clone();
            hide_from(&mut below.layers, id);
            photocraft_compose::render(&below, area).into_px()
        }
        (_, None) => read(layer.surface().ok_or_else(|| EngineError::Other(format!("layer \"{}\" has no pixels", layer.name)))?),
    };
    let w = auto_black_white(&auto_bw_sample(&rgba));
    Ok(Value::Object(crate::adjust_params::HUE_RANGES.iter().zip(w).map(|(k, v)| ((*k).to_string(), json!(v))).collect()))
}

#[cfg(test)]
mod tests;
