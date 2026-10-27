//! Styles panel › Import Styles… / Export Styles…: Photoshop layer style libraries (`.asl`).
//!
//! - `style.presets.importAsl` adds every style of an `.asl` file to the Styles library as one
//!   group (named after the file), and the patterns embedded in the file to the pattern library so
//!   the styles' Pattern Overlays, pattern strokes and bevel textures render. A malformed file
//!   changes nothing.
//! - `style.presets.exportAsl` writes styles (all, one group or named ones) as an `.asl`, with the
//!   patterns their effects use.
//!
//! The container lives in `photocraft-psd` (`asl`), the mapping onto effects in `photocraft-io`
//! (`asl_map`).

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::styles::StylePreset;
use crate::presets::{Group, find, unique_name};
use crate::{EngineError, Result, Session};
use photocraft_io::asl_map::{self, AslStyle};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_styles(s: &Session) -> std::result::Result<(), String> {
    if s.presets.styles.iter().any(|g| !g.items.is_empty()) { Ok(()) } else { Err("no styles".into()) }
}

const IMPORT: &str = "style.presets.importAsl";
const EXPORT: &str = "style.presets.exportAsl";

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, IMPORT, ".asl file")?;
    let group = match p.get("group") {
        Some(v) => v.as_str().map(str::trim).filter(|g| !g.is_empty()).ok_or_else(|| bad(IMPORT, "`group` must be a non-empty string"))?.to_string(),
        None if !stem.trim().is_empty() => stem.trim().to_string(),
        None => "Imported Styles".to_string(),
    };
    let replace = match p.get("replace") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(IMPORT, "`replace` must be a boolean"))?,
    };
    // A file named like a built-in group ("Basics.asl") goes to "Basics 2": built-ins are never replaced.
    let builtin: Vec<String> = crate::presets::styles::builtin().into_iter().map(|g| g.name).collect();
    let group = if builtin.contains(&group) { (2..).map(|i| format!("{group} {i}")).find(|g| !builtin.contains(g)).unwrap_or(group) } else { group };
    let imp = asl_map::read_asl(&bytes).map_err(|e| bad(IMPORT, e))?;
    if imp.styles.is_empty() {
        return Err(bad(IMPORT, "the file holds no styles"));
    }
    // Everything is checked: change the library.
    let mut pattern_ids = Vec::new();
    for pat in imp.patterns {
        pattern_ids.push(pat.id.clone());
        s.patterns.upsert(pat);
    }
    let styles = &mut s.presets.styles;
    if replace {
        styles.retain(|g| g.name != group);
    }
    let gi = match styles.iter().position(|g| g.name == group) {
        Some(i) => i,
        None => {
            styles.push(Group::new(&group, Vec::new()));
            styles.len() - 1
        }
    };
    let mut names = Vec::with_capacity(imp.styles.len());
    for st in imp.styles {
        // Agents address styles by name: keep names unique across the library.
        let name = unique_name(styles, &st.name);
        names.push(name.clone());
        let preset = StylePreset { name, effects: st.effects, blend: st.blend, fill_opacity: st.fill_opacity, opacity: st.opacity };
        if let Some(g) = styles.get_mut(gi) {
            g.items.push(preset);
        }
    }
    s.presets_changed();
    Ok(json!({"group": group, "imported": names, "count": names.len(), "patterns": pattern_ids, "warnings": imp.warnings}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let group = match p.get("group") {
        None => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad(EXPORT, "`group` must be a string"))?),
    };
    let chosen: Vec<StylePreset> = match p.get("styles") {
        Some(v) => {
            let list = v.as_array().ok_or_else(|| bad(EXPORT, "`styles` must be a list of style names"))?;
            list.iter()
                .map(|n| {
                    let name = n.as_str().ok_or_else(|| bad(EXPORT, "`styles` must be a list of style names"))?;
                    let (gi, ii) = find(&s.presets.styles, name, group).ok_or_else(|| bad(EXPORT, format!("no style \"{name}\" (see style.presets.list)")))?;
                    s.presets.styles.get(gi).and_then(|g| g.items.get(ii)).cloned().ok_or_else(|| bad(EXPORT, format!("no style \"{name}\"")))
                })
                .collect::<Result<_>>()?
        }
        None => match group {
            Some(g) => s.presets.styles.iter().find(|x| x.name == g).ok_or_else(|| bad(EXPORT, format!("no style group \"{g}\"")))?.items.clone(),
            None => s.presets.styles.iter().flat_map(|g| g.items.iter().cloned()).collect(),
        },
    };
    if chosen.is_empty() {
        return Err(bad(EXPORT, "no styles to export"));
    }
    // The patterns the styles use, from the open document or the library.
    let all_fx: Vec<photocraft_doc::Effect> = chosen.iter().flat_map(|st| st.effects.iter().cloned()).collect();
    let mut patterns: Vec<photocraft_doc::Pattern> = Vec::new();
    let mut warnings = Vec::new();
    for (name, id) in asl_map::referenced_patterns(&all_fx) {
        let key = if id.is_empty() { &name } else { &id };
        match crate::pattern_cmds::resolve(s, key).filter(|_| !key.is_empty()) {
            Some(pat) if !patterns.iter().any(|q| q.id == pat.id) => patterns.push(pat),
            Some(_) => {}
            None => warnings.push(format!("pattern \"{}\" was not found and is not included", if name.is_empty() { &id } else { &name })),
        }
    }
    let styles: Vec<AslStyle> = chosen
        .iter()
        .map(|st| AslStyle { name: st.name.clone(), effects: st.effects.clone(), blend: st.blend, opacity: st.opacity, fill_opacity: st.fill_opacity })
        .collect();
    let bytes = asl_map::write_asl(&styles, &patterns).map_err(|e| bad(EXPORT, e))?;
    let mut out = json!({"count": styles.len(), "bytes": bytes.len(), "patterns": patterns.len(), "warnings": warnings});
    match p.get("path") {
        Some(v) => {
            let path = v.as_str().filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(EXPORT, "`path` must be a non-empty string"))?;
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes)),
    }
    Ok(out)
}

/// `.asl` command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: IMPORT,
            label: "Import Styles…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".asl file"?,"data":base64 bytes?,"group":string?=file name (a built-in group's name becomes "<name> 2": built-ins are never replaced),"replace":bool=true (replace a group of the same name)} → {group, imported, count, patterns (ids added to the pattern library), warnings}"##,
            enabled: always,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: EXPORT,
            label: "Export Styles…",
            menu: &[],
            shortcut: None,
            params: r##"{"group":name? (only that group),"styles":[names]? (only these),"path":".asl file"? (else the bytes come back as base64 `data`)} → {count, bytes, patterns, warnings, path|data}"##,
            enabled: has_styles,
            run: export,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_paint::tile::{b64_decode, b64_encode};

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    }

    #[test]
    fn export_then_import_round_trips_a_group() {
        let mut s = session();
        let before = s.presets.styles.clone();
        let r = s.execute(EXPORT, json!({"group": "Text Effects"})).unwrap();
        let n = before.iter().find(|g| g.name == "Text Effects").unwrap().items.len();
        assert_eq!(r["count"], n);
        let data = r["data"].as_str().unwrap().to_string();
        let r = s.execute(IMPORT, json!({"data": data, "group": "Back"})).unwrap();
        assert_eq!(r["count"], n);
        let back = s.presets.styles.iter().find(|g| g.name == "Back").unwrap().items.clone();
        let orig = &before.iter().find(|g| g.name == "Text Effects").unwrap().items;
        for (a, b) in back.iter().zip(orig) {
            // Names stay unique across the library: "Gold" comes back as "Gold 2".
            assert!(a.name.starts_with(&b.name), "{} vs {}", a.name, b.name);
            assert_eq!(a.effects.len(), b.effects.len(), "{}", b.name);
            assert_eq!(a.fill_opacity, b.fill_opacity, "{}", b.name);
        }
        // The imported copy applies like the original.
        s.execute("style.presets.apply", json!({"preset": back[0].name})).unwrap();
        let fx = &s.active().unwrap().doc.layers.last().unwrap().effects.items;
        assert_eq!(fx.len(), orig[0].effects.len());
        // Re-import replaces the group instead of adding another.
        let groups = s.presets.styles.len();
        s.execute(IMPORT, json!({"data": data, "group": "Back"})).unwrap();
        assert_eq!(s.presets.styles.len(), groups);
    }

    #[test]
    fn a_file_named_like_a_built_in_group_keeps_that_group() {
        let mut s = session();
        let basics = s.presets.styles.iter().find(|g| g.name == "Basics").unwrap().items.clone();
        let data = s.execute(EXPORT, json!({"group": "Text Effects"})).unwrap()["data"].as_str().unwrap().to_string();
        for _ in 0..2 {
            let r = s.execute(IMPORT, json!({"data": data, "group": "Basics"})).unwrap();
            assert_eq!(r["group"], "Basics 2");
        }
        assert_eq!(s.presets.styles.iter().find(|g| g.name == "Basics").unwrap().items, basics);
        // Re-importing replaces the copy instead of adding another.
        assert_eq!(s.presets.styles.iter().filter(|g| g.name.starts_with("Basics")).count(), 2);
    }

    #[test]
    fn blending_options_survive_and_apply() {
        let mut s = session();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.layerStyle.blendingOptions", json!({"layer": id.0, "opacity": 40, "blend": "multiply", "fillOpacity": 30})).unwrap();
        let l = s.active().unwrap().doc.layer(id).unwrap().clone();
        assert_eq!(l.blend, photocraft_color::BlendMode::Multiply);
        s.execute("style.presets.new", json!({"name": "Blended", "group": "Mine", "layer": id.0})).unwrap();
        let data = s.execute(EXPORT, json!({"styles": ["Blended"]})).unwrap()["data"].as_str().unwrap().to_string();
        s.execute(IMPORT, json!({"data": data, "group": "Again"})).unwrap();
        let st = s.presets.styles.iter().find(|g| g.name == "Again").unwrap().items[0].clone();
        assert_eq!(st.blend, Some(l.blend));
        assert!((st.opacity.unwrap() - l.opacity).abs() < 1e-4);
        assert!((st.fill_opacity.unwrap() - l.fill_opacity).abs() < 1e-4);
        // Applying it to a fresh layer sets the blending options.
        s.execute("layer.new.layer", json!({})).unwrap();
        let fresh = s.active().unwrap().active_layer.unwrap();
        s.execute("style.presets.apply", json!({"preset": st.name, "layer": fresh.0})).unwrap();
        let f = s.active().unwrap().doc.layer(fresh).unwrap();
        assert_eq!(f.blend, l.blend);
        assert!((f.opacity - l.opacity).abs() < 1e-4);
    }

    /// #9318 review: an ASL that replaces a library pattern by id marks its pixels changed, so the
    /// preset store writes the replacement (see `PatternLibrary::pixels_rev`).
    #[test]
    fn an_asl_pattern_replacement_bumps_pixels_rev() {
        let mut s = session();
        let mut pat = s.patterns.items[1].clone();
        pat.id = "asl-replaced-pattern".into();
        let asl = |p: &photocraft_doc::Pattern| {
            let style = AslStyle { name: "Pat".into(), effects: vec![], blend: None, opacity: None, fill_opacity: None };
            b64_encode(&asl_map::write_asl(&[style], std::slice::from_ref(p)).unwrap())
        };
        s.execute(IMPORT, json!({"data": asl(&pat), "group": "A"})).unwrap();
        let rev = s.patterns.pixels_rev;
        pat.surface = s.patterns.items[2].surface.clone();
        s.execute(IMPORT, json!({"data": asl(&pat), "group": "B"})).unwrap();
        assert!(s.patterns.pixels_rev > rev, "the same-id replacement is marked");
    }

    #[test]
    fn embedded_patterns_reach_the_library_and_the_document() {
        let mut s = session();
        let pat = s.patterns.items[0].clone();
        let fx = crate::layer_style::effect_from_params("patternOverlay", &json!({"pattern": pat.id})).unwrap();
        let (fx, _) = crate::pattern_cmds::resolve_effect(&s, fx).unwrap();
        let mut renamed = pat.clone();
        renamed.id = "asl-test-pattern".into();
        renamed.name = "Asl Test".into();
        let fx = match fx {
            photocraft_doc::Effect::PatternOverlay { common, scale, angle, link, phase, .. } => {
                photocraft_doc::Effect::PatternOverlay { common, name: renamed.name.clone(), id: renamed.id.clone(), scale, angle, link, phase }
            }
            other => other,
        };
        let style = AslStyle { name: "Pat".into(), effects: vec![fx], blend: None, opacity: None, fill_opacity: None };
        let bytes = asl_map::write_asl(&[style], std::slice::from_ref(&renamed)).unwrap();
        let r = s.execute(IMPORT, json!({"data": b64_encode(&bytes), "group": "Pats"})).unwrap();
        assert_eq!(r["patterns"], json!(["asl-test-pattern"]));
        assert!(s.patterns.items.iter().any(|p| p.id == "asl-test-pattern"));
        s.execute("style.presets.apply", json!({"preset": "Pat", "group": "Pats"})).unwrap();
        assert!(s.active().unwrap().doc.patterns.iter().any(|p| p.id == "asl-test-pattern"));
        // Export embeds it again.
        let r = s.execute(EXPORT, json!({"group": "Pats"})).unwrap();
        assert_eq!(r["patterns"], 1);
        let back = asl_map::read_asl(&b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(back.patterns.len(), 1);
    }

    #[test]
    fn files_on_disk() {
        let mut s = session();
        let dir = std::env::temp_dir().join(format!("pc-asl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("My Styles.asl");
        let r = s.execute(EXPORT, json!({"group": "Basics", "path": path.to_string_lossy()})).unwrap();
        assert!(r.get("data").is_none());
        let r = s.execute(IMPORT, json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["group"], "My Styles");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_params_fail_gracefully_and_change_nothing() {
        let mut s = session();
        let good = s.execute(EXPORT, json!({"group": "Basics"})).unwrap()["data"].as_str().unwrap().to_string();
        let raw = b64_decode(&good).unwrap();
        let styles = s.presets.styles.clone();
        let pats = s.patterns.items.len();
        let empty = b64_encode(&asl_map::write_asl(&[], &[]).unwrap());
        for p in [
            json!({}),
            json!({"path": ""}),
            json!({"path": "/nonexistent/styles.asl"}),
            json!({"data": 42}),
            json!({"data": "%%%not base64"}),
            json!({"data": ""}),
            json!({"data": b64_encode(b"8BPS\0\x01junk")}),
            json!({"data": b64_encode(&raw[..raw.len() / 2])}),
            json!({"data": empty}),
            json!({"data": good, "group": 5}),
            json!({"data": good, "group": "  "}),
            json!({"data": good, "replace": "yes"}),
        ] {
            assert!(s.execute(IMPORT, p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.presets.styles, styles);
        assert_eq!(s.patterns.items.len(), pats);
        for p in [
            json!({"group": "No Such Group"}),
            json!({"group": 7}),
            json!({"styles": "Gold"}),
            json!({"styles": [3]}),
            json!({"styles": ["Nope"]}),
            json!({"styles": []}),
            json!({"path": 9}),
            json!({"path": " "}),
        ] {
            assert!(s.execute(EXPORT, p.clone()).is_err(), "{p}");
        }
        s.presets.styles.clear();
        assert!(s.execute(EXPORT, json!({})).is_err());
    }
}
