//! Named Export As configurations. They persist with the other presets in the preferences
//! document; applying one only returns settings, and the export itself stays in Export As.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::presets::{always, bad, req_str};
use crate::{Result, Session};

const MAX_PRESETS: usize = 256;
const MAX_PX: u32 = 300_000;
const FORMATS: [&str; 6] = ["png", "jpg", "webp", "tif", "tga", "ico"];

/// Export As › Image Size › Resample, in Photoshop's order: the `image.imageSize` method each
/// choice sends, and its menu label. The first one is the default.
pub const RESAMPLE: [(&str, &str); 6] = [
    ("bicubicAutomatic", "Bicubic Automatic"),
    ("bicubic", "Bicubic"),
    ("bicubicSmoother", "Bicubic Smoother"),
    ("bicubicSharper", "Bicubic Sharper"),
    ("bilinear", "Bilinear"),
    ("nearest", "Nearest Neighbor"),
];

/// Photoshop's Export As default: Bicubic Automatic.
pub const DEFAULT_RESAMPLE: &str = "bicubicAutomatic";

/// `v` when it names one of [`RESAMPLE`]; otherwise an error listing the choices. Export As
/// never falls back silently: Image Size would quietly use Bicubic for an unknown name.
pub fn export_resample(v: &str) -> std::result::Result<&'static str, String> {
    RESAMPLE.iter().map(|(k, _)| *k).find(|k| *k == v).ok_or_else(|| {
        let names: Vec<&str> = RESAMPLE.iter().map(|(k, _)| *k).collect();
        format!("resample must be one of {}", names.join(", "))
    })
}

fn default_resample() -> String {
    DEFAULT_RESAMPLE.into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPresetSettings {
    pub format: String,
    pub quality: u8,
    pub lossless: bool,
    pub transparency: bool,
    pub scale: f64,
    /// Output pixels. Ignored when [`Self::sizing`] is `"scale"`.
    pub width: u32,
    pub height: u32,
    pub lock_aspect: bool,
    /// `"scale"` uses [`Self::scale`] on the current document; `"pixels"` uses width and height.
    pub sizing: String,
    /// `"none"` drops XMP and EXIF/resolution, `"exif"` keeps EXIF and resolution, `"all"` keeps both.
    pub metadata: String,
    /// `"document"` keeps the document profile; `"srgb"` converts to sRGB before export.
    pub profile: String,
    pub embed_profile: bool,
    /// `"auto"`, `"24"` or `"32"`.
    pub tga_bits: String,
    /// One of [`RESAMPLE`]; presets saved before the choice existed load as the default.
    #[serde(default = "default_resample")]
    pub resample: String,
}

impl ExportPresetSettings {
    fn validate(&self) -> std::result::Result<(), String> {
        if !FORMATS.contains(&self.format.as_str()) {
            return Err(format!("format must be one of {}", FORMATS.join(", ")));
        }
        if !(1..=100).contains(&self.quality) {
            return Err("quality must be within 1..=100".into());
        }
        if !self.scale.is_finite() || !(1.0..=1000.0).contains(&self.scale) {
            return Err("scale must be within 1..=1000 percent".into());
        }
        if !matches!(self.sizing.as_str(), "scale" | "pixels") {
            return Err("sizing must be scale or pixels".into());
        }
        if self.sizing == "pixels" && (!(1..=MAX_PX).contains(&self.width) || !(1..=MAX_PX).contains(&self.height)) {
            return Err(format!("width and height must be within 1..={MAX_PX} pixels"));
        }
        if !matches!(self.metadata.as_str(), "none" | "exif" | "all") {
            return Err("metadata must be none, exif or all".into());
        }
        if !matches!(self.profile.as_str(), "document" | "srgb") {
            return Err("profile must be document or srgb".into());
        }
        if !matches!(self.tga_bits.as_str(), "auto" | "24" | "32") {
            return Err("tgaBits must be auto, 24 or 32".into());
        }
        export_resample(&self.resample)?;
        Ok(())
    }

    fn from_params(p: &Value, cmd: &str) -> Result<Self> {
        let mut fields = p.as_object().cloned().ok_or_else(|| bad(cmd, "`settings` must be an object"))?;
        let defaults = json!({
            "format": "png",
            "quality": 85,
            "lossless": false,
            "transparency": true,
            "scale": 100.0,
            "width": 0,
            "height": 0,
            "lockAspect": true,
            "sizing": "scale",
            "metadata": "none",
            "profile": "document",
            "embedProfile": true,
            "tgaBits": "auto",
            "resample": DEFAULT_RESAMPLE
        });
        if let Some(defaults) = defaults.as_object() {
            for (k, v) in defaults {
                fields.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        for key in ["width", "height"] {
            if let Some(n) = fields.get(key).and_then(Value::as_f64) {
                if !n.is_finite() || n < 0.0 || n > f64::from(MAX_PX) {
                    return Err(bad(cmd, format!("`{key}` must be within 0..={MAX_PX} pixels")));
                }
                fields.insert(key.into(), json!(n.round() as u32));
            }
        }
        if let Some(q) = fields.get("quality").and_then(Value::as_f64) {
            if !q.is_finite() || !(1.0..=100.0).contains(&q) {
                return Err(bad(cmd, "`quality` must be within 1..=100"));
            }
            fields.insert("quality".into(), json!(q.round() as u8));
        }
        let settings: Self = serde_json::from_value(Value::Object(fields)).map_err(|e| bad(cmd, format!("invalid settings: {e}")))?;
        settings.validate().map_err(|e| bad(cmd, e))?;
        Ok(settings)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub settings: ExportPresetSettings,
}

fn checked_name(name: &str) -> std::result::Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > crate::brush_preset_cmds::MAX_NAME {
        return Err("Preset names must contain 1–255 characters after trimming".into());
    }
    Ok(name.into())
}

/// Read entries independently so a damaged preset cannot discard other presets or preferences.
pub(crate) fn load(value: Value) -> Vec<ExportPreset> {
    let Some(entries) = value.as_array() else { return Vec::new() };
    let mut presets: Vec<ExportPreset> = Vec::new();
    for entry in entries.iter().take(MAX_PRESETS) {
        let Ok(mut preset) = serde_json::from_value::<ExportPreset>(entry.clone()) else { continue };
        let Ok(name) = checked_name(&preset.name) else { continue };
        if preset.settings.validate().is_err() || presets.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        preset.name = name;
        presets.push(preset);
    }
    presets
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.presets.save";
    let name = checked_name(req_str(p, "name", CMD)?).map_err(|e| bad(CMD, e))?;
    if s.presets.export_presets.len() >= MAX_PRESETS && s.presets.export_presets.iter().all(|pr| !pr.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(CMD, format!("At most {MAX_PRESETS} export presets can be saved")));
    }
    let settings = ExportPresetSettings::from_params(p.get("settings").unwrap_or(&Value::Null), CMD)?;
    let preset = ExportPreset { name: name.clone(), settings };
    if let Some(existing) = s.presets.export_presets.iter_mut().find(|pr| pr.name.eq_ignore_ascii_case(&name)) {
        *existing = preset.clone();
    } else {
        s.presets.export_presets.push(preset.clone());
    }
    s.presets_changed();
    Ok(json!({"preset": preset}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.presets.get";
    let name = req_str(p, "name", CMD)?;
    let preset = s.presets.export_presets.iter().find(|pr| pr.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(CMD, "No such export preset"))?;
    Ok(json!({"preset": preset}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.presets.delete";
    let name = req_str(p, "name", CMD)?;
    let index = s.presets.export_presets.iter().position(|pr| pr.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(CMD, "No such export preset"))?;
    s.presets.export_presets.remove(index);
    s.presets_changed();
    Ok(json!({"deleted": name}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.export.presets.list",
            label: "List Export Presets",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(json!({"presets": s.presets.export_presets})),
            journal: false,
        },
        CommandSpec {
            id: "file.export.presets.get",
            label: "Get Export Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str} -> {preset} (settings for Export As; nothing is exported)"#,
            enabled: always,
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "file.export.presets.save",
            label: "Save Export Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str,"settings":{"format":"png|jpg|webp|tif|tga|ico","quality":1..100,"lossless":bool,"transparency":bool,"scale":1..1000,"width":px,"height":px,"lockAspect":bool,"sizing":"scale|pixels","metadata":"none|exif|all","profile":"document|srgb","embedProfile":bool,"tgaBits":"auto|24|32","resample":"bicubicAutomatic|bicubic|bicubicSmoother|bicubicSharper|bilinear|nearest"="bicubicAutomatic"}} (a saved name is replaced)"#,
            enabled: always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: "file.export.presets.delete",
            label: "Delete Export Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str}"#,
            enabled: always,
            run: delete,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(sizing: &str) -> Value {
        json!({
            "format": "jpg",
            "quality": 70,
            "lossless": false,
            "transparency": false,
            "scale": 50,
            "width": 640,
            "height": 480,
            "lockAspect": true,
            "sizing": sizing,
            "metadata": "exif",
            "profile": "srgb",
            "embedProfile": false,
            "tgaBits": "24",
            "resample": "nearest"
        })
    }

    #[test]
    fn presets_round_trip_through_the_preferences_document() {
        let mut s = Session::new();
        s.execute("file.export.presets.save", json!({"name": "  Web JPEG  ", "settings": settings("pixels")})).unwrap();
        let again = s.execute("file.export.presets.save", json!({"name": "web jpeg", "settings": settings("scale")})).unwrap();
        assert_eq!(again["preset"]["settings"]["sizing"], "scale");
        assert_eq!(s.presets.export_presets.len(), 1, "saving the same name replaces it");

        let raw = s.prefs_to_json();
        let mut restored = Session::new();
        restored.load_prefs_json(&raw).unwrap();
        let listed = restored.execute("file.export.presets.list", json!({})).unwrap();
        let presets = listed["presets"].as_array().unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0]["name"], "web jpeg");
        assert_eq!(presets[0]["settings"]["format"], "jpg");
        assert_eq!(presets[0]["settings"]["width"], 640);
        assert_eq!(presets[0]["settings"]["embedProfile"], false);
        assert_eq!(presets[0]["settings"]["metadata"], "exif");
        assert_eq!(presets[0]["settings"]["resample"], "nearest");

        let got = restored.execute("file.export.presets.get", json!({"name": "WEB JPEG"})).unwrap();
        assert_eq!(got["preset"]["settings"]["profile"], "srgb");
        restored.execute("file.export.presets.delete", json!({"name": "web jpeg"})).unwrap();
        assert!(restored.execute("file.export.presets.list", json!({})).unwrap()["presets"].as_array().unwrap().is_empty());
        let raw = restored.prefs_to_json();
        let mut after = Session::new();
        after.load_prefs_json(&raw).unwrap();
        assert!(after.presets.export_presets.is_empty());
    }

    #[test]
    fn bad_presets_are_rejected_and_a_damaged_entry_is_dropped() {
        let mut s = Session::new();
        assert!(s.execute("file.export.presets.save", json!({})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "  ", "settings": settings("scale")})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"format": "bmp"}})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"sizing": "pixels", "width": 0, "height": 10}})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"scale": 0}})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"metadata": "copyright"}})).is_err());
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"profile": "fogra39"}})).is_err());
        let err = s.execute("file.export.presets.save", json!({"name": "x", "settings": {"resample": "lanczos"}})).unwrap_err().to_string();
        assert!(err.contains("resample must be one of"), "{err}");
        assert!(s.execute("file.export.presets.save", json!({"name": "x", "settings": {"resample": 3}})).is_err());
        assert!(s.execute("file.export.presets.delete", json!({"name": "missing"})).is_err());
        assert!(s.execute("file.export.presets.get", json!({"name": ""})).is_err());

        let loaded = load(json!([
            {"name": "ok", "settings": {"format": "png", "quality": 85, "lossless": false, "transparency": true, "scale": 100.0, "width": 0, "height": 0, "lockAspect": true, "sizing": "scale", "metadata": "none", "profile": "document", "embedProfile": true, "tgaBits": "auto"}},
            {"name": "broken", "settings": {"format": "nope"}},
            "not an object"
        ]));
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "ok");
        assert_eq!(loaded[0].settings.resample, DEFAULT_RESAMPLE, "a preset saved before Resample existed loads with the default");
    }
}
