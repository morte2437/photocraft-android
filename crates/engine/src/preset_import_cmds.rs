//! Importing Photoshop preset files.
//!
//! - `brush.presets.importAbr` adds the presets of an `.abr` file (v1, v2, v6+) to the brush
//!   library as one group named after the file, with the file's own folders nested inside it
//!   as Photoshop does (Brushes panel › Import Brushes…, Preset Manager). Parsing lives in
//!   `photocraft-psd` (`abr`), the mapping onto [`BrushSettings`] in `photocraft-io` (`abr_map`).
//! - `brush.presets.importImage` turns an image file (anything File › Open reads: PNG, JPEG,
//!   TIFF, PSD…) into a sampled-tip brush preset, as Edit › Define Brush Preset does with a
//!   selection: dark pixels paint, white and transparent ones don't (TOOL-213-12).
//! - `gradient.presets.importGrd` adds the gradients of a `.grd` file (version 5) to the
//!   Gradients panel as one group.
//!
//! [`BrushSettings`]: photocraft_paint::BrushSettings

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Largest accepted file (Photoshop's biggest bundled sets are well under this).
pub const MAX_FILE_BYTES: u64 = 512 << 20;
/// Largest accepted `.abr` file: brush libraries of large sampled tips outgrow [`MAX_FILE_BYTES`].
/// Tied to the decoder's sample budget (1 GiB, 512 MiB on 32-bit targets) so it never refuses
/// what this accepts.
pub const MAX_ABR_FILE_BYTES: u64 = photocraft_psd::abr::MAX_TOTAL_BYTES as u64;

/// File bytes from `data` (base64) or `path`, plus a default group name.
pub(crate) fn file_bytes(p: &Value, cmd: &str, what: &str) -> Result<(Vec<u8>, String)> {
    file_bytes_within(p, cmd, what, MAX_FILE_BYTES)
}

/// [`file_bytes`] with a size cap of `max` bytes.
fn file_bytes_within(p: &Value, cmd: &str, what: &str, max: u64) -> Result<(Vec<u8>, String)> {
    if let Some(d) = p.get("data") {
        let s = d.as_str().ok_or_else(|| bad(cmd, "`data` must be a base64 string"))?;
        if s.len() as u64 > max / 3 * 4 + 4 {
            return Err(bad(cmd, "`data` is too large"));
        }
        let bytes = photocraft_paint::tile::b64_decode(s).ok_or_else(|| bad(cmd, "`data` is not valid base64"))?;
        return Ok((bytes, String::new()));
    }
    let path =
        p.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, format!("give `path` ({what}) or `data` (base64)")))?;
    let len = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.len();
    if len > max {
        return Err(bad(cmd, format!("{path}: file too large ({len} bytes)")));
    }
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok((bytes, stem))
}

/// The preset names in use, for naming new presets.
struct UniqueNames {
    /// Every name in use, ASCII-lowercased.
    taken: std::collections::HashSet<String>,
    /// The next number to try for each repeated name (lowercased).
    next: std::collections::HashMap<String, usize>,
}

impl UniqueNames {
    fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        UniqueNames { taken: names.into_iter().map(str::to_ascii_lowercase).collect(), next: std::collections::HashMap::new() }
    }

    /// A name not in use (case-insensitive): `name`, `name 2`, `name 3`… It is in use from then on.
    fn unique(&mut self, name: &str) -> String {
        let key = name.to_ascii_lowercase();
        if self.taken.insert(key.clone()) {
            return name.to_string();
        }
        // Resume numbering per name, so a file of identical names stays linear.
        let from = self.next.get(&key).copied().unwrap_or(2);
        let i = (from..).find(|i| !self.taken.contains(&format!("{key} {i}"))).unwrap_or(from);
        self.taken.insert(format!("{key} {i}"));
        self.next.insert(key, i + 1);
        format!("{name} {i}")
    }
}

fn import_abr(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.importAbr";
    let label = "Import Brushes";
    // A background job when started with `Session::start` (#210): reading and decoding the
    // file run on a worker, cancellable before each brush; the library changes on apply.
    let params = p.clone();
    // Only the keys the apply step reads: `data` can be a ~1.3 GiB base64 string.
    let params_apply = json!({ "replace": p.get("replace"), "select": p.get("select") });
    crate::jobs::run(
        s,
        label,
        false,
        move |ctx| {
            ctx.progress(0.0, "Importing brushes");
            let p = &params;
            let (bytes, stem) = file_bytes_within(p, cmd, ".abr file", MAX_ABR_FILE_BYTES)?;
            let group = match p.get("group") {
                Some(v) => v.as_str().map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(cmd, "`group` must be a non-empty string"))?.to_string(),
                None if !stem.is_empty() => stem,
                None => "Imported Brushes".to_string(),
            };
            ctx.check()?;
            let imp = ctx
                .stage(0.05, 1.0, "Importing brushes", |ctl| photocraft_io::abr_map::read_abr_with(&bytes, &group, ctl))
                .map_err(|e| if ctx.cancelled() { EngineError::Cancelled } else { bad(cmd, e) })?;
            Ok((group, imp))
        },
        move |s, (group, imp)| add_abr_presets(s, &params_apply, group, imp),
    )
}

fn add_abr_presets(s: &mut Session, p: &Value, group: String, imp: photocraft_io::abr_map::AbrImport) -> Result<Value> {
    // Re-importing a file replaces its group instead of duplicating it.
    if p.get("replace").and_then(Value::as_bool).unwrap_or(true) {
        s.tools.presets.retain(|x| x.builtin || x.group != group);
    }
    let mut taken = UniqueNames::new(s.tools.presets.iter().map(|x| x.name.as_str()));
    let mut names = Vec::with_capacity(imp.presets.len());
    for mut preset in imp.presets {
        preset.name = taken.unique(&preset.name);
        names.push(preset.name.clone());
        s.tools.presets.push(preset);
    }
    s.brush_presets_changed();
    if p.get("select").and_then(Value::as_bool).unwrap_or(false)
        && let Some(first) = names.first()
        && let Some(pr) = photocraft_paint::presets::find(&s.tools.presets, first)
    {
        let mut b = pr.brush.clone().picked_over(&s.tools.brush);
        s.load_brush_tips(&mut b).map_err(|e| bad("brush.presets.importAbr", e))?;
        s.tools.brush = b;
        s.tools.brush_changed();
    }
    // Distinct folder paths the file brought (its Photoshop folders, nested inside `group`).
    let folders: std::collections::BTreeSet<&[String]> =
        s.tools.presets.iter().filter(|x| !x.builtin && x.group == group).flat_map(|x| (1..=x.folder.len()).filter_map(|d| x.folder.get(..d))).collect();
    let folders = folders.len();
    Ok(json!({ "group": group, "imported": names, "count": names.len(), "folders": folders, "version": imp.version, "warnings": imp.warnings }))
}

/// The image's paint amount (darkness × alpha of its composite, as Define Brush Preset reads a
/// selection), cropped to the painted extent. The composite is rendered in bands twice: once to
/// find the painted bounds, then only those, so a large mostly-white image never needs a
/// full-size buffer.
fn image_paint(doc: &photocraft_doc::Document, cmd: &str) -> Result<(Vec<f32>, usize, usize)> {
    let paint = |c: [f32; 4]| ((1.0 - photocraft_color::blend::lum([c[0], c[1], c[2]])) * c[3]).clamp(0.0, 1.0);
    let mut bounds: Option<photocraft_geom::Rect> = None;
    let _ = photocraft_compose::render_bands(doc, doc.bounds(), 0, |band| -> std::result::Result<(), ()> {
        let w = band.rect.width() as usize;
        for (i, c) in band.px.iter().enumerate() {
            if paint(*c) > 1e-4 {
                let (x, y) = (band.rect.x0 + (i % w.max(1)) as i32, band.rect.y0 + (i / w.max(1)) as i32);
                let px = photocraft_geom::Rect::new(x, y, x.saturating_add(1), y.saturating_add(1));
                bounds = Some(bounds.map_or(px, |b| b.union(&px)));
            }
        }
        Ok(())
    });
    let area = bounds.ok_or_else(|| bad(cmd, "the image contains no paint (all white or transparent)"))?;
    let max = crate::brush_cmds::MAX_TIP;
    if area.width().max(area.height()) > max {
        return Err(bad(cmd, format!("the painted part of the image is too large for a brush tip ({}×{}; max {max} px)", area.width(), area.height())));
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut v = vec![0.0f32; w * h];
    let _ = photocraft_compose::render_bands(doc, area, 0, |band| -> std::result::Result<(), ()> {
        let bw = band.rect.width() as usize;
        for (i, c) in band.px.iter().enumerate() {
            let (x, y) = ((band.rect.x0 - area.x0) as usize + i % bw.max(1), (band.rect.y0 - area.y0) as usize + i / bw.max(1));
            if let Some(out) = v.get_mut(y * w + x) {
                *out = paint(*c);
            }
        }
        Ok(())
    });
    Ok((v, w, h))
}

/// Brushes panel › Import Brushes… with an image file: a new sampled-tip preset named after the
/// file (never overwriting one), made the current brush unless `select` is false.
fn import_image(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.importImage";
    let (bytes, stem) = file_bytes(p, cmd, "image file")?;
    let file_name = match p.get("fileName") {
        None => None,
        Some(v) => Some(v.as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "`fileName` must be a non-empty string"))?),
    };
    // The decoder picks the format by content, falling back to the name's extension.
    let decode_name = p.get("path").and_then(Value::as_str).or(file_name).unwrap_or("image");
    let stem = if stem.is_empty() {
        file_name.and_then(|n| std::path::Path::new(n).file_stem()).map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default()
    } else {
        stem
    };
    let name = match p.get("name") {
        None if stem.is_empty() => "Imported Brush".to_string(),
        None => stem,
        Some(v) => v.as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "`name` must be a non-empty string"))?.to_string(),
    };
    let doc = crate::file_cmds::import(decode_name, &bytes)?;
    let (v, w, h) = image_paint(&doc, cmd)?;
    let (brush, tw, th) = crate::brush_cmds::sampled_brush(cmd, &v, w, h)?;
    let name = UniqueNames::new(s.tools.presets.iter().map(|x| x.name.as_str())).unique(&name);
    s.tools.presets.push(photocraft_paint::BrushPreset {
        name: name.clone(),
        brush: brush.clone(),
        builtin: false,
        group: String::new(),
        folder: Vec::new(),
        captures: Default::default(),
    });
    s.brush_presets_changed();
    if p.get("select").and_then(Value::as_bool).unwrap_or(true) {
        s.tools.brush = brush;
        s.tools.current_preset = Some(name.clone());
        s.tools.brush_changed();
    }
    Ok(json!({ "name": name, "width": tw, "height": th }))
}

/// Brush-file import command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "brush.presets.importAbr",
            label: "Import Brushes…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".abr file"?,"data":base64 bytes?,"group":string?=file name,"replace":bool=true (replace a group of the same name),"select":bool=false (make the first imported preset current)} → {group, imported, count, folders (nested folders kept from the file), version, warnings}"##,
            enabled: always,
            run: import_abr,
            journal: true,
        },
        CommandSpec {
            id: "brush.presets.importImage",
            label: "Import Brush from Image…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":"image file (PNG, JPEG, TIFF, PSD…)"?,"data":base64 bytes?,"fileName":string? (with data: the file's name, for its format and the default preset name),"name":string?=file name,"select":bool=true (make it the current brush)} → {name, width, height}"##,
            enabled: always,
            run: import_image,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.importGrd",
            label: "Import Gradients…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".grd file"?,"data":base64 bytes?,"group":string?=file name,"replace":bool=true (replace a group of the same name)}"##,
            enabled: always,
            run: import_grd,
            journal: true,
        },
    ]
}

// ------------------------------------------------------------------ gradients (.grd)

fn hsb_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

/// CIE Lab (D50, as Photoshop stores it) to sRGB through XYZ with Bradford adaptation to D65.
fn lab_to_rgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let inv = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (0.9642 * inv(fx), inv(fy), 0.8249 * inv(fz));
    // XYZ (D50) → linear sRGB (D65), Bradford-adapted matrix.
    let r = 3.1339 * x - 1.6169 * y - 0.4906 * z;
    let g = -0.9788 * x + 1.9161 * y + 0.0335 * z;
    let bb = 0.0719 * x - 0.2290 * y + 1.4052 * z;
    let gamma = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    };
    [gamma(r), gamma(g), gamma(bb)]
}

fn stop_color(c: photocraft_psd::grd::GrdColor) -> crate::presets::gradients::StopColor {
    use crate::presets::gradients::StopColor;
    use photocraft_psd::grd::GrdColor;
    let clamp = |c: [f32; 3]| StopColor::Rgb(c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
    match c {
        GrdColor::Foreground => StopColor::Foreground,
        GrdColor::Background => StopColor::Background,
        GrdColor::Rgb(c) => clamp(c),
        GrdColor::Hsb(c) => clamp(hsb_to_rgb(c)),
        // Naive (profile-free) ink → RGB, like an uncalibrated preview.
        GrdColor::Cmyk([c, m, y, k]) => clamp([(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]),
        GrdColor::Lab(c) => clamp(lab_to_rgb(c)),
        GrdColor::Gray(g) => clamp([1.0 - g; 3]),
    }
}

fn import_grd(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::presets::{Group, gradients::GradientPreset};
    let cmd = "gradient.presets.importGrd";
    let (bytes, stem) = file_bytes(p, cmd, ".grd file")?;
    let group = match p.get("group") {
        Some(v) => v.as_str().map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(cmd, "`group` must be a non-empty string"))?.to_string(),
        None if !stem.is_empty() => stem,
        None => "Imported Gradients".to_string(),
    };
    let grads = photocraft_psd::grd::parse(&bytes).map_err(|e| bad(cmd, format!("not a readable gradient file (.grd): {e}")))?;
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    let (mut noise, mut midpoints) = (0, false);
    for (i, g) in grads.iter().enumerate() {
        if g.noise || g.stops.len() < 2 {
            noise += 1;
            continue;
        }
        midpoints |= g.stops.iter().any(|st| (st.midpoint - 0.5).abs() > 0.01) || g.opacity.iter().any(|o| (o.2 - 0.5).abs() > 0.01);
        let mut stops: Vec<(f32, _)> = g.stops.iter().map(|st| (st.location, stop_color(st.color))).collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut opacity: Vec<(f32, f32)> = g.opacity.iter().map(|o| (o.0, o.1)).collect();
        opacity.sort_by(|a, b| a.0.total_cmp(&b.0));
        if opacity.iter().all(|o| o.1 >= 1.0) {
            opacity.clear();
        }
        let name = if g.name.trim().is_empty() { format!("Gradient {}", i + 1) } else { g.name.trim().to_string() };
        items.push(GradientPreset { name, stops, opacity });
    }
    if noise > 0 {
        warnings.push(format!("{noise} noise gradients are not supported and were skipped"));
    }
    if midpoints {
        warnings.push("colour and opacity midpoints other than 50 % are drawn at 50 %".to_string());
    }
    if items.is_empty() {
        return Err(bad(cmd, "the file holds no gradients PhotoCraft can use (noise gradients only)"));
    }
    let names: Vec<String> = items.iter().map(|g| g.name.clone()).collect();
    let st = &mut s.presets;
    if p.get("replace").and_then(Value::as_bool).unwrap_or(true) {
        st.gradients.retain(|g| g.name != group);
    }
    st.gradients.push(Group::new(&group, items));
    s.presets_changed();
    Ok(json!({"group": group, "imported": names, "count": names.len(), "warnings": warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v6, write_v12};
    use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value as DV};

    fn abr_v2() -> Vec<u8> {
        let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: vec![255; 24] };
        write_v12(
            2,
            &[
                LegacyBrush { name: "Hard Round".into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip) },
                LegacyBrush {
                    name: String::new(),
                    spacing: 10,
                    anti_alias: true,
                    tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
                },
            ],
            true,
        )
        .unwrap()
    }

    #[test]
    fn import_adds_a_group_and_paints() {
        let mut s = Session::new();
        let before = s.tools.presets.len();
        let data = photocraft_paint::tile::b64_encode(&abr_v2());
        let r = s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy", "select": true})).unwrap();
        assert_eq!(r["count"], 2);
        // The name clashing with the built-in Hard Round is made unique.
        assert_eq!(r["imported"], json!(["Hard Round 2", "9 px Round"]));
        assert_eq!(s.tools.presets.len(), before + 2);
        assert!(s.tools.presets.iter().filter(|p| p.group == "Legacy").all(|p| !p.builtin));
        assert!(matches!(s.tools.brush.tip, photocraft_paint::TipShape::Sampled(_)));
        // Re-import replaces the group.
        s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy"})).unwrap();
        assert_eq!(s.tools.presets.len(), before + 2);
        // Imported presets paint.
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("paint.stroke", json!({"points": [[5, 30], [60, 30]], "preset": "9 px Round"})).unwrap();
        // From a file on disk, the group defaults to the file name.
        let dir = std::env::temp_dir().join(format!("pc-abr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("My Set.abr");
        let preset = Descriptor::new("brushPreset")
            .with("Nm  ", DV::Text(UnicodeString::new_nul("Speckle")))
            .with("Brsh", DV::Descriptor(Descriptor::new("sampledBrush").with("sampledData", DV::Text(UnicodeString::new_nul("$s")))));
        let tip = AbrSample { id: "$s".into(), width: 5, height: 5, depth: 16, data: vec![0x80; 50] };
        std::fs::write(&path, write_v6(2, &[tip], &[], &[preset], true).unwrap()).unwrap();
        let r = s.execute("brush.presets.importAbr", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!((r["group"].as_str(), r["imported"][0].as_str(), r["version"].as_u64()), (Some("My Set"), Some("Speckle"), Some(6)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_params_fail_gracefully() {
        let mut s = Session::new();
        let n = s.tools.presets.len();
        for p in [
            json!({}),
            json!({"path": ""}),
            json!({"path": "/nonexistent/brushes.abr"}),
            json!({"data": 42}),
            json!({"data": "%%%not base64"}),
            json!({"data": ""}),
            json!({"data": "AAAA"}),
            json!({"data": photocraft_paint::tile::b64_encode(b"8BPS\0\x01junk")}),
            json!({"data": photocraft_paint::tile::b64_encode(&abr_v2()), "group": 5}),
            json!({"data": photocraft_paint::tile::b64_encode(&abr_v2()), "group": "  "}),
            json!({"data": photocraft_paint::tile::b64_encode(&abr_v2()[..20])}),
        ] {
            assert!(s.execute("brush.presets.importAbr", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.tools.presets.len(), n);
    }

    #[test]
    fn grd_import_adds_a_gradient_group() {
        use crate::presets::gradients::StopColor;
        use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
        let grads = vec![
            GrdGradient {
                name: "Ember".into(),
                noise: false,
                stops: vec![
                    GrdStop { location: 1.0, midpoint: 0.5, color: GrdColor::Rgb([1.0, 0.0, 0.0]) },
                    GrdStop { location: 0.0, midpoint: 0.5, color: GrdColor::Foreground },
                ],
                opacity: vec![(0.0, 1.0, 0.5), (1.0, 0.0, 0.5)],
            },
            GrdGradient { name: "Static".into(), noise: true, ..Default::default() },
        ];
        let mut s = Session::new();
        let data = photocraft_paint::tile::b64_encode(&write(&grads));
        let r = s.execute("gradient.presets.importGrd", json!({"data": data, "group": "Fire"})).unwrap();
        assert_eq!(r["imported"], json!(["Ember"]));
        assert_eq!(r["warnings"].as_array().map(Vec::len), Some(1));
        let g = s.presets.gradients.iter().find(|g| g.name == "Fire").unwrap();
        assert_eq!(g.items[0].stops, vec![(0.0, StopColor::Foreground), (1.0, StopColor::Rgb([1.0, 0.0, 0.0]))]);
        assert_eq!(g.items[0].opacity, vec![(0.0, 1.0), (1.0, 0.0)]);
        // Re-import replaces the group.
        let n = s.presets.gradients.len();
        s.execute("gradient.presets.importGrd", json!({"data": data, "group": "Fire"})).unwrap();
        assert_eq!(s.presets.gradients.len(), n);
        for p in [json!({}), json!({"data": "@@"}), json!({"data": photocraft_paint::tile::b64_encode(b"8BGR\0\x03")}), json!({"path": "/nope.grd"})] {
            assert!(s.execute("gradient.presets.importGrd", p.clone()).is_err(), "{p}");
        }
        let only_noise = photocraft_paint::tile::b64_encode(&write(&grads[1..]));
        assert!(s.execute("gradient.presets.importGrd", json!({"data": only_noise})).is_err());
    }

    #[test]
    fn colour_conversions() {
        assert_eq!(hsb_to_rgb([120.0, 1.0, 1.0]), [0.0, 1.0, 0.0]);
        let w = lab_to_rgb([100.0, 0.0, 0.0]);
        assert!(w.iter().all(|v| (v - 1.0).abs() < 0.01), "{w:?}");
        let k = lab_to_rgb([0.0, 0.0, 0.0]);
        assert!(k.iter().all(|v| *v < 0.01), "{k:?}");
    }

    /// A 40×30 16-bit RGBA PNG: transparent, a white opaque band (no paint) and a black 10×6 block
    /// at (5, 8) (full paint).
    fn stamp_png16() -> Vec<u8> {
        let mut px = vec![0u16; 40 * 30 * 4];
        for y in 0..30 {
            for x in 0..40 {
                let i = (y * 40 + x) * 4;
                if (5..15).contains(&x) && (8..14).contains(&y) {
                    px[i + 3] = u16::MAX;
                } else if y >= 20 {
                    px[i..i + 4].copy_from_slice(&[u16::MAX; 4]);
                }
            }
        }
        let img = photocraft_codecs::Image::from_u16(40, 30, photocraft_codecs::ChannelLayout::Rgba, &px).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    }

    #[test]
    fn imports_an_image_as_a_brush_tip() {
        use photocraft_paint::TipShape;
        let mut s = Session::new();
        let data = photocraft_paint::tile::b64_encode(&stamp_png16());
        let r = s.execute("brush.presets.importImage", json!({"data": data, "fileName": "Stamp.png"})).unwrap();
        assert_eq!(r, json!({"name": "Stamp", "width": 10, "height": 6}), "cropped to the dark block");
        let pr = s.tools.presets.iter().find(|p| p.name == "Stamp").unwrap();
        let TipShape::Sampled(tip) = &pr.brush.tip else { panic!("sampled tip expected") };
        assert_eq!((tip.width, tip.height), (10, 6));
        assert!((pr.brush.size - 10.0).abs() < 1e-6);
        assert_eq!(s.tools.current_preset.as_deref(), Some("Stamp"), "the imported brush is current");
        assert!(matches!(s.tools.brush.tip, TipShape::Sampled(_)));
        // A second import keeps the first: a new name, and `select: false` leaves the brush alone.
        let r = s.execute("brush.presets.importImage", json!({"data": data, "fileName": "Stamp.png", "select": false})).unwrap();
        assert_eq!(r["name"], "Stamp 2");
        assert_eq!(s.tools.current_preset.as_deref(), Some("Stamp"));

        // An 8-bit grayscale JPEG from disk: named after the file.
        let px: Vec<u8> = (0..24 * 24).map(|i| if (8..16).contains(&(i % 24)) && (4..20).contains(&(i / 24)) { 0 } else { 255 }).collect();
        let img = photocraft_codecs::Image::from_u8(24, 24, photocraft_codecs::ChannelLayout::Gray, px).unwrap();
        let jpeg = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &Default::default()).unwrap();
        let path = std::env::temp_dir().join(format!("pc-brush-tip-{}.jpg", std::process::id()));
        std::fs::write(&path, jpeg).unwrap();
        let r = s.execute("brush.presets.importImage", json!({"path": path.to_string_lossy()}));
        let _ = std::fs::remove_file(&path);
        let r = r.unwrap();
        assert!(r["name"].as_str().unwrap().starts_with("pc-brush-tip-"), "{r}");
        let (w, h) = (r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap());
        assert!((8..=10).contains(&w) && (16..=18).contains(&h), "JPEG ringing aside, the bar: {r}");
    }

    #[test]
    fn image_brush_import_fails_cleanly() {
        let mut s = Session::new();
        let white = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Gray, vec![255; 16]).unwrap();
        let white = photocraft_paint::tile::b64_encode(&photocraft_codecs::encode(&white, photocraft_codecs::Format::Png, &Default::default()).unwrap());
        let n = s.tools.presets.len();
        for p in [
            json!({}),
            json!({"data": white}),
            json!({"data": "@@"}),
            json!({"data": photocraft_paint::tile::b64_encode(b"not an image")}),
            json!({"data": white, "name": 3}),
            json!({"data": white, "fileName": ""}),
            json!({"path": "/nope/brush.png"}),
        ] {
            assert!(s.execute("brush.presets.importImage", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.tools.presets.len(), n, "a failed import adds nothing");
    }

    /// A version 1 brush with a computed tip and no name is named after its tip ("9 px Round"). The
    /// import used to number each repeated name by searching the preset names once for every number
    /// it tried, so importing 10,000 such brushes took minutes. The test fails when the import takes
    /// more than 20 seconds.
    #[test]
    fn repeated_brush_names_are_numbered_in_linear_time() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = Session::new();
            s.execute("brush.presets.save", json!({"name": "9 PX ROUND 3"})).unwrap();
            let round = LegacyBrush {
                name: String::new(),
                spacing: 25,
                anti_alias: true,
                tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
            };
            let data = photocraft_paint::tile::b64_encode(&write_v12(1, &vec![round; 10_000], false).unwrap());
            let r = s.execute("brush.presets.importAbr", json!({"data": data})).unwrap();
            let names: Vec<String> = r["imported"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            let _ = tx.send(names);
        });
        let names = match rx.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(names) => names,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("importing the brushes did not finish"),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("the import thread panicked"),
        };
        // Numbering starts at 2 and skips the number a preset already uses, in any letter case.
        assert_eq!(names[..4], ["9 px Round", "9 px Round 2", "9 px Round 4", "9 px Round 5"]);
        assert_eq!(names.last(), Some(&format!("9 px Round {}", names.len() + 1)));
        assert_eq!(names.iter().collect::<std::collections::HashSet<_>>().len(), names.len());
    }

    #[test]
    fn repeated_brush_names_take_the_first_free_number() {
        let mut s = Session::new();
        s.execute("brush.presets.save", json!({"name": "speckle 3"})).unwrap();
        let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: vec![255; 24] };
        let brush = |name: &str| LegacyBrush { name: name.into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip.clone()) };
        let file = write_v12(2, &["Speckle", "SPECKLE", "speckle", "Speckle 2", "Speckle", "hard round"].map(brush), true).unwrap();
        let r = s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&file)})).unwrap();
        // Names that differ only in letter case count as the same name. The file's "Speckle 2" comes
        // after "SPECKLE" took that name, and "hard round" clashes with the built-in Hard Round.
        assert_eq!(r["imported"], json!(["Speckle", "SPECKLE 2", "speckle 4", "Speckle 2 2", "Speckle 5", "hard round 2"]));
    }

    #[test]
    fn abr_files_get_a_larger_size_cap() {
        // 1 GiB on 64-bit; 32-bit (wasm) keeps 512 MiB so a big file fails cleanly instead of aborting.
        const WANT: u64 = if cfg!(target_pointer_width = "64") { 1 << 30 } else { 512 << 20 };
        const _: () = assert!(MAX_ABR_FILE_BYTES == WANT && MAX_ABR_FILE_BYTES >= MAX_FILE_BYTES);
        let dir = std::env::temp_dir().join(format!("pc-abr-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cap.abr");
        std::fs::write(&path, [0u8; 10]).unwrap();
        let p = json!({"path": path.to_string_lossy()});
        assert!(file_bytes_within(&p, "t", "f", 10).is_ok());
        assert!(file_bytes_within(&p, "t", "f", 9).is_err(), "a file over the cap is refused before reading");
        // A sparse file between the two caps: refused as a .grd, which is checked before reading.
        let big = dir.join("big.grd");
        std::fs::File::create(&big).unwrap().set_len(MAX_FILE_BYTES + 1).unwrap();
        let err = Session::new().execute("gradient.presets.importGrd", json!({"path": big.to_string_lossy()})).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
        let data = json!({"data": "A".repeat(16)});
        assert!(file_bytes_within(&data, "t", "f", 12).is_ok());
        assert!(file_bytes_within(&data, "t", "f", 8).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unique_names() {
        let mut taken = UniqueNames::new(["A", "a 2"]);
        assert_eq!(taken.unique("a"), "a 3");
        assert_eq!(taken.unique("B"), "B");
    }
}
