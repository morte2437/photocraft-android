//! Camera Raw Filter › Save Settings… / Load Settings… (#4274): Adobe Camera Raw settings files
//! (`.xmp`), the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`) on an
//! `x:xmpmeta/rdf:RDF/rdf:Description`, as attributes or as child elements.
//!
//! Loading applies only the settings the file holds, over the current ones, as Camera Raw's
//! Load Settings does; settings PhotoCraft has no control for (lens corrections, geometry,
//! defringe, …) are ignored and named in the report. The same reader fills the dialog from a
//! raw file's sidecar (`IMG_0001.xmp` beside `IMG_0001.CR2`), and the writer keeps it. Saving writes every supported
//! control, so loading the file in Photoshop reproduces the dialog.
//!
//! White balance: a Camera Raw Filter on a rendered image uses the relative
//! `IncrementalTemperature` / `IncrementalTint` (−100..=100), which are PhotoCraft's Temperature
//! and Tint. The absolute `Temperature` (kelvin) / `Tint` of raw-file settings have no
//! counterpart and are ignored.
//!
//! Crop (#9985): `crs:HasCrop="True"` with `crs:CropTop` / `CropLeft` / `CropBottom` /
//! `CropRight`, fractions of the developed frame in the raw's stored orientation (before its
//! Orientation tag turns it upright), is read into [`LoadReport::crop`]; the raw open crops the
//! document with it ([`crate::raw::RawTuning::crop`]). The settings have no crop (Camera Raw
//! Filter has none). Without `HasCrop`, or with `HasCrop="False"`, the crop keys are stale and
//! mean nothing. A straightened crop (`CropAngle` other than 0) isn't applied.

use photocraft_algo::camera_raw::{CameraRaw, validate_curve};

/// The Camera Raw settings namespace.
pub const CRS_NS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// Largest settings file read; real ones are a few kilobytes.
pub const MAX_XMP_BYTES: usize = 4 * 1024 * 1024;

type Field = fn(&mut CameraRaw) -> &mut f32;

/// `crs:` scalar name, PhotoCraft field, and the slider range values are clamped to.
const SCALARS: &[(&str, Field, f32, f32)] = &[
    ("IncrementalTemperature", |p| &mut p.temperature, -100.0, 100.0),
    ("IncrementalTint", |p| &mut p.tint, -100.0, 100.0),
    ("Exposure2012", |p| &mut p.exposure, -5.0, 5.0),
    ("Contrast2012", |p| &mut p.contrast, -100.0, 100.0),
    ("Highlights2012", |p| &mut p.highlights, -100.0, 100.0),
    ("Shadows2012", |p| &mut p.shadows, -100.0, 100.0),
    ("Whites2012", |p| &mut p.whites, -100.0, 100.0),
    ("Blacks2012", |p| &mut p.blacks, -100.0, 100.0),
    ("Texture", |p| &mut p.texture, -100.0, 100.0),
    ("Clarity2012", |p| &mut p.clarity, -100.0, 100.0),
    ("Dehaze", |p| &mut p.dehaze, -100.0, 100.0),
    ("Vibrance", |p| &mut p.vibrance, -100.0, 100.0),
    ("Saturation", |p| &mut p.saturation, -100.0, 100.0),
    ("ParametricShadows", |p| &mut p.curve_shadows, -100.0, 100.0),
    ("ParametricDarks", |p| &mut p.curve_darks, -100.0, 100.0),
    ("ParametricLights", |p| &mut p.curve_lights, -100.0, 100.0),
    ("ParametricHighlights", |p| &mut p.curve_highlights, -100.0, 100.0),
    ("ParametricShadowSplit", |p| &mut p.curve_splits[0], 0.0, 100.0),
    ("ParametricMidtoneSplit", |p| &mut p.curve_splits[1], 0.0, 100.0),
    ("ParametricHighlightSplit", |p| &mut p.curve_splits[2], 0.0, 100.0),
    ("HueAdjustmentRed", |p| &mut p.hsl_hue[0], -100.0, 100.0),
    ("HueAdjustmentOrange", |p| &mut p.hsl_hue[1], -100.0, 100.0),
    ("HueAdjustmentYellow", |p| &mut p.hsl_hue[2], -100.0, 100.0),
    ("HueAdjustmentGreen", |p| &mut p.hsl_hue[3], -100.0, 100.0),
    ("HueAdjustmentAqua", |p| &mut p.hsl_hue[4], -100.0, 100.0),
    ("HueAdjustmentBlue", |p| &mut p.hsl_hue[5], -100.0, 100.0),
    ("HueAdjustmentPurple", |p| &mut p.hsl_hue[6], -100.0, 100.0),
    ("HueAdjustmentMagenta", |p| &mut p.hsl_hue[7], -100.0, 100.0),
    ("SaturationAdjustmentRed", |p| &mut p.hsl_sat[0], -100.0, 100.0),
    ("SaturationAdjustmentOrange", |p| &mut p.hsl_sat[1], -100.0, 100.0),
    ("SaturationAdjustmentYellow", |p| &mut p.hsl_sat[2], -100.0, 100.0),
    ("SaturationAdjustmentGreen", |p| &mut p.hsl_sat[3], -100.0, 100.0),
    ("SaturationAdjustmentAqua", |p| &mut p.hsl_sat[4], -100.0, 100.0),
    ("SaturationAdjustmentBlue", |p| &mut p.hsl_sat[5], -100.0, 100.0),
    ("SaturationAdjustmentPurple", |p| &mut p.hsl_sat[6], -100.0, 100.0),
    ("SaturationAdjustmentMagenta", |p| &mut p.hsl_sat[7], -100.0, 100.0),
    ("LuminanceAdjustmentRed", |p| &mut p.hsl_lum[0], -100.0, 100.0),
    ("LuminanceAdjustmentOrange", |p| &mut p.hsl_lum[1], -100.0, 100.0),
    ("LuminanceAdjustmentYellow", |p| &mut p.hsl_lum[2], -100.0, 100.0),
    ("LuminanceAdjustmentGreen", |p| &mut p.hsl_lum[3], -100.0, 100.0),
    ("LuminanceAdjustmentAqua", |p| &mut p.hsl_lum[4], -100.0, 100.0),
    ("LuminanceAdjustmentBlue", |p| &mut p.hsl_lum[5], -100.0, 100.0),
    ("LuminanceAdjustmentPurple", |p| &mut p.hsl_lum[6], -100.0, 100.0),
    ("LuminanceAdjustmentMagenta", |p| &mut p.hsl_lum[7], -100.0, 100.0),
    // Color Grading keeps the split-toning names for the shadow and highlight wheels' hue and
    // saturation, and the balance.
    ("SplitToningShadowHue", |p| &mut p.grade_shadows.hue, 0.0, 360.0),
    ("SplitToningShadowSaturation", |p| &mut p.grade_shadows.sat, 0.0, 100.0),
    ("ColorGradeShadowLum", |p| &mut p.grade_shadows.lum, -100.0, 100.0),
    ("ColorGradeMidtoneHue", |p| &mut p.grade_midtones.hue, 0.0, 360.0),
    ("ColorGradeMidtoneSat", |p| &mut p.grade_midtones.sat, 0.0, 100.0),
    ("ColorGradeMidtoneLum", |p| &mut p.grade_midtones.lum, -100.0, 100.0),
    ("SplitToningHighlightHue", |p| &mut p.grade_highlights.hue, 0.0, 360.0),
    ("SplitToningHighlightSaturation", |p| &mut p.grade_highlights.sat, 0.0, 100.0),
    ("ColorGradeHighlightLum", |p| &mut p.grade_highlights.lum, -100.0, 100.0),
    ("ColorGradeGlobalHue", |p| &mut p.grade_global.hue, 0.0, 360.0),
    ("ColorGradeGlobalSat", |p| &mut p.grade_global.sat, 0.0, 100.0),
    ("ColorGradeGlobalLum", |p| &mut p.grade_global.lum, -100.0, 100.0),
    ("ColorGradeBlending", |p| &mut p.grade_blending, 0.0, 100.0),
    ("SplitToningBalance", |p| &mut p.grade_balance, -100.0, 100.0),
    ("Sharpness", |p| &mut p.sharpen_amount, 0.0, 150.0),
    ("SharpenRadius", |p| &mut p.sharpen_radius, 0.5, 3.0),
    ("SharpenDetail", |p| &mut p.sharpen_detail, 0.0, 100.0),
    ("SharpenEdgeMasking", |p| &mut p.sharpen_masking, 0.0, 100.0),
    ("LuminanceSmoothing", |p| &mut p.noise_luminance, 0.0, 100.0),
    ("LuminanceNoiseReductionDetail", |p| &mut p.noise_luminance_detail, 0.0, 100.0),
    ("ColorNoiseReduction", |p| &mut p.noise_color, 0.0, 100.0),
    ("ColorNoiseReductionDetail", |p| &mut p.noise_color_detail, 0.0, 100.0),
    ("GrainAmount", |p| &mut p.grain_amount, 0.0, 100.0),
    ("GrainSize", |p| &mut p.grain_size, 0.0, 100.0),
    ("GrainFrequency", |p| &mut p.grain_roughness, 0.0, 100.0),
    ("PostCropVignetteAmount", |p| &mut p.vignette_amount, -100.0, 100.0),
    ("PostCropVignetteMidpoint", |p| &mut p.vignette_midpoint, 0.0, 100.0),
    ("PostCropVignetteRoundness", |p| &mut p.vignette_roundness, -100.0, 100.0),
    ("PostCropVignetteFeather", |p| &mut p.vignette_feather, 0.0, 100.0),
    ("PostCropVignetteHighlightContrast", |p| &mut p.vignette_highlights, 0.0, 100.0),
    // Calibration
    ("ShadowTint", |p| &mut p.shadow_tint, -100.0, 100.0),
    ("RedHue", |p| &mut p.calibration_hue[0], -100.0, 100.0),
    ("RedSaturation", |p| &mut p.calibration_sat[0], -100.0, 100.0),
    ("GreenHue", |p| &mut p.calibration_hue[1], -100.0, 100.0),
    ("GreenSaturation", |p| &mut p.calibration_sat[1], -100.0, 100.0),
    ("BlueHue", |p| &mut p.calibration_hue[2], -100.0, 100.0),
    ("BlueSaturation", |p| &mut p.calibration_sat[2], -100.0, 100.0),
];

type Curve = fn(&mut CameraRaw) -> &mut Vec<[f32; 2]>;
const CURVES: &[(&str, Curve)] = &[
    ("ToneCurvePV2012", |p| &mut p.point_curve),
    ("ToneCurvePV2012Red", |p| &mut p.point_curve_red),
    ("ToneCurvePV2012Green", |p| &mut p.point_curve_green),
    ("ToneCurvePV2012Blue", |p| &mut p.point_curve_blue),
];

const STYLES: &[(&str, &str)] = &[("1", "highlightPriority"), ("2", "colorPriority"), ("3", "paintOverlay")];

/// Bookkeeping fields Camera Raw writes that carry no setting.
const METADATA: &[&str] = &[
    "Version",
    "ProcessVersion",
    "HasSettings",
    "WhiteBalance",
    "ToneCurveName2012",
    "CameraProfile",
    "PresetType",
    "UUID",
    "SupportsAmount",
    "SupportsColor",
    "SupportsMonochrome",
    "SupportsHighDynamicRange",
    "SupportsNormalDynamicRange",
    "SupportsSceneReferred",
    "SupportsOutputReferred",
    "CameraModelRestriction",
    "Copyright",
    "ContactInfo",
    "Name",
    "ShortName",
    "SortName",
    "Group",
    "Description",
    // The raw file the sidecar belongs to (Lightroom and Camera Raw name it).
    "RawFileName",
];

/// The crop's keys besides `HasCrop`, and their values without a crop (the whole frame, level).
pub const CROP_KEYS: [&str; 5] = ["CropTop", "CropLeft", "CropBottom", "CropRight", "CropAngle"];
const NO_CROP: [f32; 5] = [0.0, 0.0, 1.0, 1.0, 0.0];

/// A Camera Raw crop: fractions of the developed frame in the raw's stored orientation, before
/// its Orientation tag turns it upright, with `0 <= left < right <= 1` and
/// `0 <= top < bottom <= 1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl Crop {
    /// The crop of the frame turned upright by TIFF orientation `o` (1–8; anything else is 1),
    /// as fractions of the upright frame: the corners map as the pixels do.
    pub fn oriented(self, o: u16) -> Crop {
        let map = |u: f32, v: f32| match o {
            2 => (1.0 - u, v),
            3 => (1.0 - u, 1.0 - v),
            4 => (u, 1.0 - v),
            5 => (v, u),
            6 => (1.0 - v, u),
            7 => (1.0 - v, 1.0 - u),
            8 => (v, 1.0 - u),
            _ => (u, v),
        };
        let (ax, ay) = map(self.left, self.top);
        let (bx, by) = map(self.right, self.bottom);
        Crop { top: ay.min(by), left: ax.min(bx), bottom: ay.max(by), right: ax.max(bx) }
    }

    /// The crop on a `w × h` frame in pixels, `(x0, y0, x1, y1)` with `x1` and `y1` exclusive:
    /// at least one pixel, inside the frame. `None` for an empty frame.
    pub fn pixels(self, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
        let edge = |f: f32, n: usize| ((f64::from(f).clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
        let span = |a: f32, b: f32, n: usize| {
            let lo = edge(a, n).min(n.checked_sub(1)?);
            Some((lo, edge(b, n).max(lo + 1)))
        };
        let (x0, x1) = span(self.left, self.right, w)?;
        let (y0, y1) = span(self.top, self.bottom, h)?;
        Some((x0, y0, x1, y1))
    }
}

/// What [`load`] applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadReport {
    /// How many settings were applied (a crop counts as one).
    pub applied: usize,
    /// `crs:` settings without a PhotoCraft control (set to something other than their neutral
    /// value), or with an unusable value, in file order; then the keys of a crop that isn't
    /// applied (straightened, unreadable or empty).
    pub ignored: Vec<String>,
    /// The file's crop, when it has one that isn't the whole frame. The settings hold no crop,
    /// so the caller applies it (a raw open crops the document) or reports it.
    pub crop: Option<Crop>,
}

/// The crop keys as read, resolved by [`crop`] once the whole file is read.
#[derive(Default)]
struct CropFields {
    has: Option<String>,
    values: [Option<String>; 5],
}

/// The file's crop from its keys: applied when `HasCrop` is `True` and the values make a level,
/// non-empty rectangle (fractions are clamped to 0..1); otherwise its keys are reported.
fn crop(fields: &CropFields, report: &mut LoadReport) {
    match fields.has.as_deref().map(str::trim) {
        // No crop: the other keys are what Camera Raw left from an earlier or reset crop.
        None => return,
        Some(v) if v.eq_ignore_ascii_case("false") => return,
        Some(v) if v.eq_ignore_ascii_case("true") => {}
        Some(_) => {
            report.ignored.push("HasCrop".into());
            return;
        }
    }
    let mut v = NO_CROP;
    let mut usable = true;
    for (slot, text) in v.iter_mut().zip(&fields.values) {
        if let Some(text) = text {
            match number(text) {
                Some(n) => *slot = n,
                None => usable = false,
            }
        }
    }
    let [top, left, bottom, right] = [v[0], v[1], v[2], v[3]].map(|f| f.clamp(0.0, 1.0));
    // A straightened crop's rectangle is measured on the rotated frame: not applied.
    usable &= v[4] == 0.0 && top < bottom && left < right;
    if !usable {
        let present = CROP_KEYS.iter().zip(&fields.values).filter(|(_, t)| t.is_some());
        report.ignored.extend(present.map(|(k, _)| k.to_string()));
        return;
    }
    if [top, left, bottom, right] != [0.0, 0.0, 1.0, 1.0] {
        report.crop = Some(Crop { top, left, bottom, right });
        report.applied += 1;
    }
}

fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    let v: f32 = s.strip_prefix('+').unwrap_or(s).parse().ok()?;
    v.is_finite().then_some(v)
}

/// `"0, 0"` items of an `rdf:Seq` → curve points.
fn curve(node: roxmltree::Node<'_, '_>) -> Option<Vec<[f32; 2]>> {
    let seq = node.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(RDF_NS) && c.tag_name().name() == "Seq")?;
    let mut points = Vec::new();
    for li in seq.children().filter(|c| c.is_element() && c.tag_name().name() == "li") {
        let text = li.text().unwrap_or("");
        let (x, y) = text.split_once(',')?;
        points.push([number(x)?, number(y)?]);
    }
    validate_curve(&points).ok()?;
    // Camera Raw's linear curve is the identity's two end points; PhotoCraft's is empty.
    if points == [[0.0, 0.0], [255.0, 255.0]] {
        points.clear();
    }
    Some(points)
}

/// An unsupported setting at its neutral value changes nothing, so it isn't reported: zero,
/// `False`, and the few neutral values that aren't zero.
fn neutral(name: &str, value: &str) -> bool {
    let v = value.trim();
    if v.eq_ignore_ascii_case("false") || number(v) == Some(0.0) {
        return true;
    }
    match name {
        "GrainSeed" => true,
        "VignetteMidpoint" => number(v) == Some(50.0),
        "PerspectiveScale" => number(v) == Some(100.0),
        _ => name.starts_with("Defringe") && name.contains("Hue"),
    }
}

fn apply(params: &mut CameraRaw, report: &mut LoadReport, crop: &mut CropFields, name: &str, value: &str) {
    if name == "HasCrop" {
        crop.has = Some(value.to_string());
    } else if let Some(i) = CROP_KEYS.iter().position(|k| *k == name) {
        if let Some(slot) = crop.values.get_mut(i) {
            *slot = Some(value.to_string());
        }
    } else if let Some((_, field, lo, hi)) = SCALARS.iter().find(|(n, ..)| *n == name) {
        match number(value) {
            Some(v) => {
                *field(params) = v.clamp(*lo, *hi);
                report.applied += 1;
            }
            None => report.ignored.push(name.to_string()),
        }
    } else if name == "PostCropVignetteStyle" {
        match STYLES.iter().find(|(n, _)| *n == value.trim()) {
            Some((_, style)) => {
                params.vignette_style = (*style).into();
                report.applied += 1;
            }
            None => report.ignored.push(name.to_string()),
        }
    } else if !METADATA.contains(&name) && !neutral(name, value) {
        report.ignored.push(name.to_string());
    }
}

/// Apply a Camera Raw settings file over `params`. Fails on input that isn't XMP or holds no
/// Camera Raw settings; never panics on malformed input.
pub fn load(params: &mut CameraRaw, bytes: &[u8]) -> Result<LoadReport, String> {
    if bytes.len() > MAX_XMP_BYTES {
        return Err("the settings file is too large".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not a Camera Raw settings file (not UTF-8 text)".to_string())?;
    // Some writers put a byte order mark or an `<?xpacket?>` wrapper first; roxmltree takes both.
    let text = text.trim_start_matches('\u{feff}');
    let doc = crate::xml::parse(text, roxmltree::ParsingOptions::default()).map_err(|e| format!("not a Camera Raw settings file ({e})"))?;
    let mut next = params.clone();
    let mut report = LoadReport::default();
    let mut found = false;
    let mut crop_fields = CropFields::default();
    for desc in doc.descendants().filter(|n| n.is_element() && n.tag_name().namespace() == Some(RDF_NS) && n.tag_name().name() == "Description") {
        for a in desc.attributes().filter(|a| a.namespace() == Some(CRS_NS)) {
            found = true;
            apply(&mut next, &mut report, &mut crop_fields, a.name(), a.value());
        }
        for c in desc.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(CRS_NS)) {
            found = true;
            let name = c.tag_name().name();
            if let Some((_, field)) = CURVES.iter().find(|(n, _)| *n == name) {
                match curve(c) {
                    Some(points) => {
                        *field(&mut next) = points;
                        report.applied += 1;
                    }
                    None => report.ignored.push(name.to_string()),
                }
            } else if c.children().any(|g| g.is_element()) {
                // Structured settings (masks, looks, lens profiles…) have no counterpart.
                if !METADATA.contains(&name) {
                    report.ignored.push(name.to_string());
                }
            } else {
                apply(&mut next, &mut report, &mut crop_fields, name, c.text().unwrap_or(""));
            }
        }
    }
    if !found {
        return Err("no Camera Raw settings in the file".into());
    }
    crop(&crop_fields, &mut report);
    *params = next;
    Ok(report)
}

fn signed(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if v > 0.0 && !s.starts_with('+') { format!("+{s}") } else { s }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The XMP toolkit [`save`] names on `x:xmpmeta`.
const TOOLKIT: &str = "PhotoCraft";
const META_NS: &str = "adobe:ns:meta/";

/// Did [`save`] write this file? True when its `x:xmpmeta` names PhotoCraft as the toolkit
/// (`x:xmptk`). Lightroom, Bridge and Camera Raw put their own toolkit there when they write a
/// file, so a sidecar another application touched since reads false. Never panics.
pub fn is_photocraft_xmp(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_XMP_BYTES {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else { return false };
    let Ok(doc) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else { return false };
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().namespace() == Some(META_NS) && n.tag_name().name() == "xmpmeta")
        .and_then(|n| n.attribute((META_NS, "xmptk")))
        == Some(TOOLKIT)
}

/// Write `params` as a Camera Raw settings file Photoshop's Load Settings… reads.
pub fn save(params: &CameraRaw) -> String {
    save_with_crop(params, None)
}

/// [`save`] with a raw file's crop, so rewriting a sidecar keeps the crop it was opened with.
pub fn save_with_crop(params: &CameraRaw, crop: Option<Crop>) -> String {
    let mut p = params.clone();
    let wb = if p.temperature != 0.0 || p.tint != 0.0 { "Custom" } else { "As Shot" };
    let mut out = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"PhotoCraft\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"\n",
    );
    out.push_str("   crs:Version=\"18.4\"\n   crs:ProcessVersion=\"15.4\"\n");
    out.push_str(&format!("   crs:WhiteBalance=\"{wb}\"\n"));
    for (name, field, lo, _) in SCALARS {
        let v = *field(&mut p);
        // Exposure has two decimals in Camera Raw, the sharpening radius one; the rest are levels.
        let text = match *name {
            "Exposure2012" => signed(v, 2),
            "SharpenRadius" => signed(v, 1),
            _ if *lo < 0.0 => signed(v.round(), 0),
            _ => format!("{:.0}", v.round()),
        };
        out.push_str(&format!("   crs:{name}=\"{text}\"\n"));
    }
    let style = STYLES.iter().find(|(_, s)| *s == p.vignette_style).map_or("1", |(n, _)| n);
    out.push_str(&format!("   crs:PostCropVignetteStyle=\"{}\"\n", escape(style)));
    if let Some(c) = crop {
        out.push_str(&format!(
            "   crs:HasCrop=\"True\"\n   crs:CropTop=\"{:.6}\"\n   crs:CropLeft=\"{:.6}\"\n   crs:CropBottom=\"{:.6}\"\n   crs:CropRight=\"{:.6}\"\n   crs:CropAngle=\"0\"\n",
            c.top, c.left, c.bottom, c.right
        ));
    }
    out.push_str("   crs:ToneCurveName2012=\"Custom\"\n   crs:HasSettings=\"True\">\n");
    for (name, field) in CURVES {
        let points = field(&mut p);
        let points = if points.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { points.clone() };
        out.push_str(&format!("   <crs:{name}>\n    <rdf:Seq>\n"));
        for [x, y] in points {
            out.push_str(&format!("     <rdf:li>{}, {}</rdf:li>\n", x.round(), y.round()));
        }
        out.push_str(&format!("    </rdf:Seq>\n   </crs:{name}>\n"));
    }
    out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A hand-written settings file laid out as Photoshop 26's Camera Raw Filter saves one (the
    /// field set and order follow Camera Raw 18; the values are synthetic), plus element-form
    /// settings, which XMP allows and some writers use.
    const FIXTURE: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   crs:Version="18.1.1" crs:ProcessVersion="15.4" crs:WhiteBalance="Custom"
   crs:IncrementalTemperature="0" crs:IncrementalTint="+10" crs:Exposure2012="+0.05"
   crs:Contrast2012="-5" crs:Highlights2012="-20" crs:Shadows2012="+10" crs:Whites2012="+35"
   crs:Blacks2012="0" crs:Texture="+30" crs:Clarity2012="+5" crs:Vibrance="+20" crs:Saturation="-5"
   crs:ParametricShadowSplit="25" crs:ParametricMidtoneSplit="50" crs:ParametricHighlightSplit="75"
   crs:Sharpness="15" crs:SharpenRadius="+1.0" crs:LuminanceSmoothing="5"
   crs:LuminanceNoiseReductionContrast="0" crs:HueAdjustmentGreen="+25" crs:SaturationAdjustmentGreen="+20"
   crs:ColorGradeBlending="50" crs:LensProfileEnable="0" crs:VignetteAmount="-20"
   crs:PerspectiveScale="100" crs:GrainAmount="10" crs:RedHue="-5" crs:GrainSeed="2834497962"
   crs:ToneCurveName2012="Linear" crs:CameraProfile="Default Color" crs:HasSettings="True" crs:HasCrop="False">
   <crs:Dehaze>+12</crs:Dehaze>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:ToneCurvePV2012Red>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>128, 140</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012Red>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    #[test]
    fn reads_a_photoshop_preset() {
        let mut p = CameraRaw::default();
        let r = load(&mut p, FIXTURE).unwrap();
        assert_eq!(p.tint, 10.0);
        assert_eq!(p.exposure, 0.05);
        assert_eq!((p.contrast, p.highlights, p.shadows, p.whites), (-5.0, -20.0, 10.0, 35.0));
        assert_eq!((p.texture, p.clarity, p.vibrance, p.saturation), (30.0, 5.0, 20.0, -5.0));
        assert_eq!((p.hsl_hue[3], p.hsl_sat[3]), (25.0, 20.0));
        assert_eq!((p.sharpen_amount, p.noise_luminance, p.grain_amount), (15.0, 5.0, 10.0));
        assert_eq!(p.grade_blending, 50.0);
        // The linear tone curve is PhotoCraft's empty one; an element-form setting is read too.
        assert!(p.point_curve.is_empty());
        assert_eq!(p.point_curve_red, vec![[0.0, 0.0], [128.0, 140.0], [255.0, 255.0]]);
        assert_eq!(p.dehaze, 12.0);
        // Lens vignetting, calibration and geometry have no counterpart.
        assert_eq!(r.ignored, vec!["VignetteAmount".to_string()], "the Calibration panel's keys load now");
        assert_eq!(p.calibration_hue[0], -5.0);
    }

    /// A settings file nested 20 000 deep overflowed the XML parser's recursion (an abort nothing
    /// can catch); it is an error now. The small stack makes a regression abort instead of passing.
    #[test]
    fn deeply_nested_settings_are_an_error_not_a_stack_overflow() {
        let xml = format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">{}{}</x:xmpmeta>", "<a>".repeat(20_000), "</a>".repeat(20_000));
        let e =
            std::thread::Builder::new().stack_size(512 * 1024).spawn(move || load(&mut CameraRaw::default(), xml.as_bytes()).err()).unwrap().join().unwrap();
        assert!(e.as_deref().is_some_and(|m| m.contains("nested")), "{e:?}");
    }

    #[test]
    fn save_then_load_round_trips() {
        let mut p = CameraRaw {
            temperature: -12.0,
            exposure: 1.25,
            contrast: 30.0,
            sharpen_radius: 1.4,
            vignette_amount: -40.0,
            vignette_style: "paintOverlay".into(),
            curve_splits: [20.0, 45.0, 80.0],
            point_curve: vec![[0.0, 10.0], [128.0, 150.0], [255.0, 240.0]],
            ..CameraRaw::default()
        };
        p.hsl_lum[5] = -33.0;
        p.grade_shadows.hue = 210.0;
        p.grade_shadows.sat = 40.0;
        p.grade_global.lum = 7.0;
        let text = save(&p);
        assert!(text.contains("crs:Exposure2012=\"+1.25\""));
        assert!(text.contains("crs:WhiteBalance=\"Custom\""));
        let mut back = CameraRaw::default();
        let r = load(&mut back, text.as_bytes()).unwrap();
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        assert_eq!(back, p);
    }

    #[test]
    fn load_applies_only_the_settings_in_the_file() {
        let mut p = CameraRaw { contrast: 40.0, ..CameraRaw::default() };
        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"><crs:Exposure2012>-9</crs:Exposure2012>
            <crs:Clarity2012>abc</crs:Clarity2012></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let r = load(&mut p, xmp).unwrap();
        assert_eq!((p.exposure, p.contrast, p.clarity), (-5.0, 40.0, 0.0));
        assert_eq!(r, LoadReport { applied: 1, ignored: vec!["Clarity2012".into()], crop: None });
    }

    #[test]
    fn malformed_input_is_an_error_and_leaves_settings_alone() {
        let before = CameraRaw { exposure: 0.5, ..CameraRaw::default() };
        for bad in [
            &b""[..],
            b"\xff\xfe\x00garbage",
            b"<x:xmpmeta><unclosed",
            b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>",
            b"<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>",
        ] {
            let mut p = before.clone();
            assert!(load(&mut p, bad).is_err());
            assert_eq!(p, before);
        }
        // A broken curve or NaN is skipped, not fatal.
        let xmp = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Contrast2012="NaN"><crs:ToneCurvePV2012><rdf:Seq>
            <rdf:li>10</rdf:li></rdf:Seq></crs:ToneCurvePV2012></rdf:Description></rdf:RDF>"#;
        let mut p = before.clone();
        let r = load(&mut p, xmp).unwrap();
        assert_eq!(p, before);
        assert_eq!(r.ignored, vec!["Contrast2012".to_string(), "ToneCurvePV2012".to_string()]);
    }

    #[test]
    fn is_photocraft_xmp_reads_the_toolkit_marker() {
        let own = save(&CameraRaw { contrast: 15.0, ..CameraRaw::default() });
        assert!(is_photocraft_xmp(own.as_bytes()));
        let wrapped = format!("\u{feff}<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n{own}<?xpacket end=\"w\"?>");
        assert!(is_photocraft_xmp(wrapped.as_bytes()), "an xpacket wrapper and a byte order mark");
        assert!(!is_photocraft_xmp(FIXTURE), "Camera Raw's own toolkit");
        for other in [
            &b""[..],
            b"<x:xmpmeta",
            b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>",
            b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" xmptk=\"PhotoCraft\"/>",
            b"<x:xmpmeta xmlns:x=\"other:ns/\" x:xmptk=\"PhotoCraft\"/>",
            b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"PhotoCraft 2\"/>",
            b"\xff\xfe",
        ] {
            assert!(!is_photocraft_xmp(other), "{}", String::from_utf8_lossy(other));
        }
    }

    /// A settings file with the `crs:` attributes `attrs`.
    fn with(attrs: &str) -> String {
        format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:RawFileName=\"IMG_0004.CR2\" {attrs}/></rdf:RDF></x:xmpmeta>"
        )
    }

    fn report(attrs: &str) -> LoadReport {
        load(&mut CameraRaw::default(), with(attrs).as_bytes()).unwrap()
    }

    /// #9985: Lightroom's sidecar named the crop's bottom and right edges and the raw's file name
    /// as unsupported; the crop is read now, and the file name is bookkeeping.
    #[test]
    fn reads_the_crop_and_not_the_file_name() {
        let r = report(r#"crs:HasCrop="True" crs:CropTop="0.1" crs:CropLeft="0.2" crs:CropBottom="0.9" crs:CropRight="0.75" crs:CropAngle="0""#);
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        assert_eq!(r.crop, Some(Crop { top: 0.1, left: 0.2, bottom: 0.9, right: 0.75 }));
        assert_eq!(r.applied, 1);
        // Element form, and the keys spread over two descriptions, read the same.
        let xmp = format!(
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"{CRS_NS}\"><crs:HasCrop>True</crs:HasCrop><crs:CropBottom>0.5</crs:CropBottom></rdf:Description><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:CropRight=\"0.5\"/></rdf:RDF>"
        );
        let r = load(&mut CameraRaw::default(), xmp.as_bytes()).unwrap();
        assert_eq!(r.crop, Some(Crop { top: 0.0, left: 0.0, bottom: 0.5, right: 0.5 }), "missing edges are the frame's");
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
    }

    #[test]
    fn no_crop_without_has_crop_true() {
        let keys = r#"crs:CropTop="0.1" crs:CropLeft="0.2" crs:CropBottom="0.9" crs:CropRight="0.75" crs:CropAngle="3""#;
        for has in ["", r#"crs:HasCrop="False""#, r#"crs:HasCrop="false""#] {
            let r = report(&format!("{has} {keys}"));
            assert_eq!((r.crop, r.ignored.clone(), r.applied), (None, Vec::new(), 0), "{has:?}: {r:?}");
        }
        // The whole frame is no crop, and nothing to report.
        let r = report(r#"crs:HasCrop="True" crs:CropTop="0" crs:CropLeft="0" crs:CropBottom="1" crs:CropRight="1" crs:CropAngle="0""#);
        assert_eq!((r.crop, r.ignored.len()), (None, 0));
        let r = report(r#"crs:HasCrop="yes" crs:CropBottom="0.5""#);
        assert_eq!((r.crop, r.ignored), (None, vec!["HasCrop".to_string()]));
    }

    /// Hostile and straightened crops never panic: out-of-range fractions are clamped to the
    /// frame; an inverted, empty, unreadable or straightened crop isn't applied and its keys are
    /// named.
    #[test]
    fn unusable_crops_are_reported_not_applied() {
        let r = report(r#"crs:HasCrop="True" crs:CropTop="-0.5" crs:CropLeft="0.25" crs:CropBottom="7" crs:CropRight="1.0000001""#);
        assert_eq!(r.crop, Some(Crop { top: 0.0, left: 0.25, bottom: 1.0, right: 1.0 }));
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        let all = || CROP_KEYS.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        for bad in [
            r#"crs:CropTop="0.8" crs:CropLeft="0.1" crs:CropBottom="0.2" crs:CropRight="0.9" crs:CropAngle="0""#,
            r#"crs:CropTop="0.5" crs:CropLeft="0.1" crs:CropBottom="0.5" crs:CropRight="0.9" crs:CropAngle="0""#,
            r#"crs:CropTop="2" crs:CropLeft="0.1" crs:CropBottom="3" crs:CropRight="0.9" crs:CropAngle="0""#,
            r#"crs:CropTop="NaN" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="0""#,
            r#"crs:CropTop="0.1" crs:CropLeft="inf" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="0""#,
            r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="" crs:CropAngle="0""#,
            r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="1.38""#,
            r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="NaN""#,
        ] {
            let r = report(&format!(r#"crs:HasCrop="True" {bad}"#));
            assert_eq!((r.crop, r.ignored, r.applied), (None, all(), 0), "{bad}");
        }
        let r = report(r#"crs:HasCrop="True" crs:CropAngle="-2.5""#);
        assert_eq!(r.ignored, vec!["CropAngle".to_string()], "only the keys the file has");
    }

    /// Corners map as `photocraft-raw`'s orientation maps pixels (`develop::source_xy`).
    #[test]
    fn crops_follow_the_orientation() {
        let c = Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 };
        let want = [
            (1, Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 }),
            (2, Crop { top: 0.1, left: 0.5, bottom: 0.6, right: 0.8 }),
            (3, Crop { top: 0.4, left: 0.5, bottom: 0.9, right: 0.8 }),
            (4, Crop { top: 0.4, left: 0.2, bottom: 0.9, right: 0.5 }),
            (5, Crop { top: 0.2, left: 0.1, bottom: 0.5, right: 0.6 }),
            (6, Crop { top: 0.2, left: 0.4, bottom: 0.5, right: 0.9 }),
            (7, Crop { top: 0.5, left: 0.4, bottom: 0.8, right: 0.9 }),
            (8, Crop { top: 0.5, left: 0.1, bottom: 0.8, right: 0.6 }),
            (0, Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 }),
        ];
        for (o, w) in want {
            let got = c.oriented(o);
            let close = [got.top - w.top, got.left - w.left, got.bottom - w.bottom, got.right - w.right].iter().all(|d| d.abs() < 1e-6);
            assert!(close, "orientation {o}: {got:?}");
        }
        assert_eq!(c.pixels(10, 20), Some((2, 2, 5, 12)));
        assert_eq!(Crop { top: 0.5, left: 0.5, bottom: 0.5001, right: 0.5001 }.pixels(10, 10), Some((5, 5, 6, 6)), "at least a pixel");
        assert_eq!(Crop { top: 0.999, left: 0.999, bottom: 1.0, right: 1.0 }.pixels(10, 10), Some((9, 9, 10, 10)), "inside the frame");
        assert_eq!(c.pixels(0, 10), None);
    }

    #[test]
    fn save_with_crop_round_trips() {
        let p = CameraRaw { contrast: 20.0, ..CameraRaw::default() };
        let crop = Crop { top: 0.125, left: 0.25, bottom: 0.875, right: 0.5 };
        let text = save_with_crop(&p, Some(crop));
        assert!(is_photocraft_xmp(text.as_bytes()));
        let mut back = CameraRaw::default();
        let r = load(&mut back, text.as_bytes()).unwrap();
        assert_eq!((back, r.crop), (p.clone(), Some(crop)));
        assert!(r.ignored.is_empty(), "{:?}", r.ignored);
        assert_eq!(load(&mut CameraRaw::default(), save(&p).as_bytes()).unwrap().crop, None);
    }
}
