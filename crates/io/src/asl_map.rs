//! Layer style libraries (`.asl`) ⇄ PhotoCraft styles.
//!
//! The container is parsed by [`photocraft_psd::asl`]. Each style's `Lefx` is the same effects
//! tree as a layer's `lfx2` block ([`crate::effects_map`]); `blendOptions` ("Include Layer
//! Blending Options") carries the blend mode, opacity and fill opacity; the file's patterns feed
//! the styles' Pattern Overlays, pattern strokes and bevel textures.
//!
//! What PhotoCraft doesn't model (an effect kind it doesn't know, advanced blending keys, an
//! effects scale other than 100 %) is left out with a warning naming the style, and the rest of
//! the file still imports.

use std::collections::{BTreeSet, HashMap, HashSet};

use photocraft_color::BlendMode;
use photocraft_doc::{Effect, FxPaint, Pattern};
use photocraft_psd::asl::{self, AslFile};
use photocraft_psd::descriptor::{Descriptor, Id, Value};

use crate::blocks::{enum_of, get_desc, num};
use crate::effects_map::{blend_from_id, blend_value, effects_descriptor, effects_from_descriptor, is_known_effects_key};
use crate::pattern_map::{from_psd, to_psd};

/// One style as PhotoCraft keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct AslStyle {
    /// Unique within the file (empty names get "Style N", repeats get "Name 2").
    pub name: String,
    /// The effects (switched off when the style's effects were).
    pub effects: Vec<Effect>,
    /// Blending options, when the style was saved with them.
    pub blend: Option<BlendMode>,
    /// 0..1.
    pub opacity: Option<f32>,
    /// 0..1.
    pub fill_opacity: Option<f32>,
}

/// The result of reading a library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AslImport {
    pub styles: Vec<AslStyle>,
    /// Patterns embedded in the file.
    pub patterns: Vec<Pattern>,
    /// What was left out, for the user.
    pub warnings: Vec<String>,
}

/// Patterns an effect list refers to: `(name, id)` in first-use order, without repeats.
pub fn referenced_patterns(effects: &[Effect]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |name: &str, id: &str| {
        if !out.iter().any(|(n, i)| if id.is_empty() { n == name } else { i == id }) {
            out.push((name.to_string(), id.to_string()));
        }
    };
    for e in effects {
        match e {
            Effect::PatternOverlay { name, id, .. } => push(name, id),
            Effect::Stroke(s) => {
                if let FxPaint::Pattern { name, id, .. } = &s.paint {
                    push(name, id);
                }
            }
            Effect::BevelEmboss(b) => {
                if let Some(t) = &b.texture {
                    push(&t.name, &t.id);
                }
            }
            _ => {}
        }
    }
    out
}

fn pct(d: &Descriptor, key: &str) -> Option<f32> {
    num(d.get(key)).filter(|v| v.is_finite()).map(|v| (v as f32 / 100.0).clamp(0.0, 1.0))
}

/// Style names in use within one file (ASCII-lowercased), with the next number to try for each
/// repeated name, so a file of identical names stays linear.
#[derive(Default)]
struct Taken {
    names: HashSet<String>,
    next: HashMap<String, usize>,
}

impl Taken {
    /// `name`, else `name 2`, `name 3`… (case-insensitive); in use from then on.
    fn unique(&mut self, name: &str) -> String {
        let key = name.to_ascii_lowercase();
        if self.names.insert(key.clone()) {
            return name.to_string();
        }
        let from = self.next.get(&key).copied().unwrap_or(2);
        let i = (from..).find(|i| !self.names.contains(&format!("{key} {i}"))).unwrap_or(from);
        self.names.insert(format!("{key} {i}"));
        self.next.insert(key, i + 1);
        format!("{name} {i}")
    }
}

/// Reads an `.asl` style library.
pub fn read_asl(bytes: &[u8]) -> Result<AslImport, String> {
    let file = asl::parse(bytes).map_err(|e| format!("not a readable style library (.asl): {e}"))?;
    let mut out = AslImport { patterns: file.patterns.iter().filter_map(from_psd).collect(), ..Default::default() };
    if let Some(e) = &file.pattern_error {
        out.warnings.push(format!("the file's patterns could not be read and were skipped ({e})"));
    }
    let skipped_patterns = file.patterns.len().saturating_sub(out.patterns.len());
    if skipped_patterns > 0 {
        out.warnings.push(format!("{skipped_patterns} empty or unreadable patterns were skipped"));
    }
    let mut taken = Taken::default();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for (i, s) in file.styles.iter().enumerate() {
        let base = if s.name.trim().is_empty() { format!("Style {}", i + 1) } else { s.name.trim().to_string() };
        let name = taken.unique(&base);
        let mut style = AslStyle { name: name.clone(), effects: Vec::new(), blend: None, opacity: None, fill_opacity: None };
        let mut left_out: Vec<String> = Vec::new();
        if let Some(fx) = get_desc(&s.style, "Lefx") {
            let (master, mut effects) = effects_from_descriptor(fx);
            if !master {
                effects.iter_mut().for_each(|e| e.set_enabled(false));
            }
            for (k, _) in &fx.items {
                if !is_known_effects_key(k.as_bytes()) {
                    left_out.push(String::from_utf8_lossy(k.as_bytes()).trim().to_string());
                }
            }
            if let Some(scale) = num(fx.get("Scl ")).filter(|v| v.is_finite() && (v - 100.0).abs() > 0.01) {
                left_out.push(format!("effects scale {scale:.0} %"));
            }
            style.effects = effects;
        }
        if let Some(b) = get_desc(&s.style, "blendOptions") {
            style.blend = enum_of(b, "Md  ").and_then(blend_from_id);
            style.opacity = pct(b, "Opct");
            style.fill_opacity = pct(b, "fillOpacity");
            for (k, _) in &b.items {
                if !matches!(k.as_bytes(), b"Md  " | b"Opct" | b"fillOpacity") {
                    left_out.push(format!("blending option {}", String::from_utf8_lossy(k.as_bytes()).trim()));
                }
            }
        }
        for (pname, pid) in referenced_patterns(&style.effects) {
            if !out.patterns.iter().any(|p| if pid.is_empty() { p.name == pname } else { p.id == pid }) {
                missing.insert(if pname.is_empty() { pid } else { pname });
            }
        }
        if !left_out.is_empty() {
            out.warnings.push(format!("\"{name}\": left out {}", left_out.join(", ")));
        }
        out.styles.push(style);
    }
    if !missing.is_empty() {
        let list: Vec<String> = missing.into_iter().collect();
        out.warnings.push(format!("patterns not in the file (they must be in the pattern library to render): {}", list.join(", ")));
    }
    Ok(out)
}

/// A stable UUID-shaped id for a style (Photoshop gives every style one).
fn style_id(index: usize, name: &str) -> String {
    // FNV-1a over the name, mixed with the index: deterministic output for deterministic tests.
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in name.bytes().chain(index.to_le_bytes()) {
        h ^= u128::from(b);
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    let x = format!("{h:032x}");
    let p = |a: usize, b: usize| x.get(a..b).unwrap_or("0");
    format!("{}-{}-{}-{}-{}", p(0, 8), p(8, 12), p(12, 16), p(16, 20), p(20, 32))
}

/// Writes styles (and the patterns their effects use) as an `.asl` style library.
pub fn write_asl(styles: &[AslStyle], patterns: &[Pattern]) -> Result<Vec<u8>, String> {
    let pct = |v: f32| Value::UnitFloat { unit: *b"#Prc", value: f64::from(v.clamp(0.0, 1.0)) * 100.0 };
    let file = AslFile {
        patterns: patterns.iter().map(to_psd).collect(),
        pattern_error: None,
        styles: styles
            .iter()
            .enumerate()
            .map(|(i, s)| {
                // A style library's effects object has class `Lefx` (a layer's `lfx2` uses `null`).
                let mut lefx = effects_descriptor(true, &s.effects);
                lefx.class_id = Id::new("Lefx");
                let mut d =
                    Descriptor::new("Styl").with("documentMode", Value::Descriptor(Descriptor::new("documentMode"))).with("Lefx", Value::Descriptor(lefx));
                if s.blend.is_some() || s.opacity.is_some() || s.fill_opacity.is_some() {
                    d = d.with(
                        "blendOptions",
                        Value::Descriptor(
                            Descriptor::new("blendOptions")
                                .with("Md  ", blend_value(s.blend.unwrap_or(BlendMode::Normal)))
                                .with("Opct", pct(s.opacity.unwrap_or(1.0)))
                                .with("fillOpacity", pct(s.fill_opacity.unwrap_or(1.0))),
                        ),
                    );
                }
                asl::AslStyle { name: s.name.clone(), id: style_id(i, &s.name), style: d }
            })
            .collect(),
    };
    asl::write(&file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{BevelTexture, FxCommon, StrokePosition};
    use photocraft_psd::descriptor::UnicodeString;
    use photocraft_raster::Surface;

    fn pattern(id: &str, s: SampleType) -> Pattern {
        let fmt = PixelFormat::new(ColorMode::Rgb, s, true);
        let mut surf = Surface::new(fmt);
        for y in 0..3 {
            for x in 0..4 {
                surf.write_pixel(x, y, &[x as f32 / 4.0, y as f32 / 3.0, 0.5, 1.0]);
            }
        }
        let mut p = Pattern::new("Grid", surf, 4, 3);
        p.id = id.into();
        p
    }

    /// Every effect kind with its descriptor defaults (the reader fills in what's absent).
    fn defaults() -> Vec<Effect> {
        let d = |k: &str| Value::Descriptor(Descriptor::new(k));
        let lefx = Descriptor::new("null")
            .with("DrSh", d("DrSh"))
            .with("IrSh", d("IrSh"))
            .with("OrGl", d("OrGl"))
            .with("IrGl", d("IrGl"))
            .with("ebbl", d("ebbl"))
            .with("ChFX", d("ChFX"))
            .with("SoFi", d("SoFi"))
            .with("GrFl", d("GrFl"))
            .with("FrFX", d("FrFX"));
        effects_from_descriptor(&lefx).1
    }

    fn styles() -> Vec<AslStyle> {
        let mut fx = defaults();
        assert_eq!(fx.len(), 9);
        for e in &mut fx {
            match e {
                Effect::DropShadow(s) => (s.distance, s.size, s.color) = (7.0, 9.0, Color::rgb(0.2, 0.1, 0.0)),
                Effect::Stroke(s) => (s.size, s.position, s.paint) = (4.0, StrokePosition::Inside, FxPaint::Color(Color::rgb(0.0, 0.0, 1.0))),
                Effect::ColorOverlay { common, color } => (*common, *color) = (FxCommon::new(BlendMode::Multiply, 0.5), Color::rgb(1.0, 0.5, 0.0)),
                _ => {}
            }
        }
        let mut bevel = defaults().into_iter().find(|e| matches!(e, Effect::BevelEmboss(_))).unwrap();
        if let Effect::BevelEmboss(b) = &mut bevel {
            b.size = 6.0;
            b.texture = Some(BevelTexture { name: "Grid".into(), id: "pat-2".into(), scale: 0.5, depth: 1.0, invert: false, link: true, phase: (0.0, 0.0) });
        }
        vec![
            AslStyle { name: "Gold".into(), effects: fx, blend: Some(BlendMode::Screen), opacity: Some(0.8), fill_opacity: Some(0.25) },
            AslStyle {
                name: "Textured".into(),
                effects: vec![
                    bevel,
                    Effect::PatternOverlay {
                        common: FxCommon { enabled: true, blend: BlendMode::Normal, opacity: 1.0 },
                        name: "Grid".into(),
                        id: "pat-1".into(),
                        scale: 2.0,
                        angle: 0.0,
                        link: true,
                        phase: (0.0, 0.0),
                    },
                ],
                blend: None,
                opacity: None,
                fill_opacity: None,
            },
        ]
    }

    #[test]
    fn round_trip_keeps_effects_blending_and_patterns() {
        let pats = vec![pattern("pat-1", SampleType::U8), pattern("pat-2", SampleType::U16)];
        let bytes = write_asl(&styles(), &pats).unwrap();
        let back = read_asl(&bytes).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.patterns, pats);
        assert_eq!(back.styles.len(), 2);
        let (a, b) = (&back.styles[0], &styles()[0]);
        assert_eq!((a.blend, a.opacity, a.fill_opacity), (b.blend, b.opacity, b.fill_opacity));
        // Effects go through the same descriptor writer and reader as PSD `lfx2`.
        let via_lfx2 = |fx: &[Effect]| effects_from_descriptor(&effects_descriptor(true, fx)).1;
        for (x, y) in back.styles.iter().zip(styles()) {
            assert_eq!(x.name, y.name);
            assert_eq!(x.effects, via_lfx2(&y.effects));
            assert_eq!(x.effects.len(), y.effects.len());
        }
        assert_eq!(referenced_patterns(&back.styles[1].effects), vec![("Grid".to_string(), "pat-2".to_string()), ("Grid".to_string(), "pat-1".to_string())]);
    }

    /// A library as Photoshop lays it out, built by hand from descriptors.
    fn synthetic(lefx: Descriptor, blend: Option<Descriptor>, names: &[&str]) -> Vec<u8> {
        let mut style = Descriptor::new("Styl").with("documentMode", Value::Descriptor(Descriptor::new("documentMode"))).with("Lefx", Value::Descriptor(lefx));
        if let Some(b) = blend {
            style = style.with("blendOptions", Value::Descriptor(b));
        }
        let file = AslFile {
            patterns: vec![],
            pattern_error: None,
            styles: names.iter().map(|n| asl::AslStyle { name: (*n).into(), id: "x".into(), style: style.clone() }).collect(),
        };
        asl::write(&file).unwrap()
    }

    #[test]
    fn blend_options_names_and_warnings() {
        let en = |t: &str, v: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(v) };
        let pc = |v: f64| Value::UnitFloat { unit: *b"#Prc", value: v };
        let lefx = Descriptor::new("null")
            .with("Scl ", pc(100.0))
            .with("masterFXSwitch", Value::Boolean(false))
            .with(
                "SoFi",
                Value::Descriptor(Descriptor::new("SoFi").with("enab", Value::Boolean(true)).with("Md  ", en("BlnM", "Nrml")).with("Opct", pc(100.0)).with(
                    "Clr ",
                    Value::Descriptor(
                        Descriptor::new("RGBC").with("Rd  ", Value::Double(255.0)).with("Grn ", Value::Double(0.0)).with("Bl  ", Value::Double(0.0)),
                    ),
                )),
            )
            .with("futureFX", Value::Descriptor(Descriptor::new("futureFX")));
        let blend = Descriptor::new("blendOptions")
            .with("Md  ", en("BlnM", "multiply"))
            .with("Opct", pc(40.0))
            .with("fillOpacity", pc(0.0))
            .with("knockout", Value::Integer(1));
        let bytes = synthetic(lefx, Some(blend), &["Red", "", "red", "  "]);
        let imp = read_asl(&bytes).unwrap();
        let names: Vec<&str> = imp.styles.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Red", "Style 2", "red 2", "Style 4"]);
        let s = &imp.styles[0];
        assert_eq!((s.blend, s.opacity, s.fill_opacity), (Some(BlendMode::Multiply), Some(0.4), Some(0.0)));
        assert_eq!(s.effects.len(), 1);
        // The style's effects were switched off.
        assert!(!s.effects[0].enabled());
        assert!(imp.warnings.iter().any(|w| w.contains("futureFX") && w.contains("knockout")), "{:?}", imp.warnings);
    }

    #[test]
    fn missing_patterns_are_reported() {
        let po = Descriptor::new("patternFill").with("enab", Value::Boolean(true)).with(
            "Ptrn",
            Value::Descriptor(
                Descriptor::new("Ptrn").with("Nm  ", Value::Text(UnicodeString::new_nul("Bubbles"))).with("Idnt", Value::Text(UnicodeString::new_nul("b-1"))),
            ),
        );
        let bytes = synthetic(Descriptor::new("null").with("patternFill", Value::Descriptor(po)), None, &["P"]);
        let imp = read_asl(&bytes).unwrap();
        assert_eq!(imp.styles[0].effects.len(), 1);
        assert_eq!(imp.styles[0].blend, None);
        assert!(imp.warnings.iter().any(|w| w.contains("Bubbles")), "{:?}", imp.warnings);
    }

    #[test]
    fn hostile_bytes_are_errors() {
        let bytes = write_asl(&styles(), &[pattern("pat-1", SampleType::U8)]).unwrap();
        for cut in 0..bytes.len() {
            assert!(read_asl(&bytes[..cut]).is_err(), "cut {cut}");
        }
        for i in 0..bytes.len() {
            let mut c = bytes.clone();
            c[i] ^= 0xa5;
            let _ = read_asl(&c);
        }
        assert!(read_asl(b"").is_err());
        assert!(read_asl(&photocraft_psd::grd::write(&[])).is_err());
    }

    #[test]
    fn exported_effects_object_has_class_lefx() {
        let bytes = write_asl(&styles(), &[]).unwrap();
        let file = asl::parse(&bytes).unwrap();
        for s in &file.styles {
            let lefx = get_desc(&s.style, "Lefx").unwrap();
            assert_eq!(lefx.class_id, Id::new("Lefx"), "{}", s.name);
        }
    }

    #[test]
    fn many_identical_names_import_quickly() {
        let names = vec!["A"; asl::MAX_STYLES];
        let bytes = synthetic(Descriptor::new("Lefx"), None, &names);
        let start = std::time::Instant::now();
        let imp = read_asl(&bytes).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
        assert_eq!(imp.styles.len(), asl::MAX_STYLES);
        assert_eq!(imp.styles[1].name, "A 2");
        assert_eq!(imp.styles.last().unwrap().name, format!("A {}", asl::MAX_STYLES));
    }

    #[test]
    fn style_ids_are_uuid_shaped() {
        let id = style_id(3, "Gold");
        assert_eq!(id.len(), 36);
        assert_ne!(id, style_id(4, "Gold"));
    }
}
