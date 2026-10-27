//! PSD type layers: the `TySh` block (transform + text descriptor with `EngineData` + warp) ⇄ the
//! typed [`TextLayer`] model.
//!
//! Units: EngineData sizes are in text-space units, which the `TySh` transform maps to document
//! pixels; Photoshop shows them as points at the document resolution, so
//! `size_pt = FontSize × 72 / dpi` (and likewise leading, tracking-independent lengths, indents).
//! Box bounds stay in text-space pixels.
//!
//! Writing starts from the layer's original `TySh` (when present) and patches only the text, runs,
//! paragraphs, fonts, shape and transform, so Photoshop-only settings survive an edit.

use photocraft_color::{Color, ColorMode};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{
    AntiAlias, Caps, CharStyle, FontBaseline, Kerning, Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextRun, TextShape, TextWarp,
};
use photocraft_geom::{Affine, Point};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D, VersionedDescriptor};

use crate::engine_data::{self as ed, Value as E};
use crate::fonts::guess_from_postscript;

mod build;
mod edit;
mod import;

#[cfg(test)]
pub(crate) use build::font_entry;
pub use build::{MAX_TEXT_INDEX, build_engine_data, build_txt2, build_txt2_at, build_tysh, needs_txt2};
pub(crate) use build::{font_set, paragraph_properties, style_sheet_data};
pub use edit::{set_text_index, text_index, txt2_object_count};
#[cfg(test)]
use import::MAX_ENGINE_RUNS;
pub use import::{
    OPENTYPE_KEYS, TXT2_FEATURES, TXT2_STYLISTIC_SETS, apply_txt2, apply_txt2_frame, apply_txt2_path, parse_txt2, refine_legacy_point_origin,
    text_layer_from_tysh,
};
pub(crate) use import::{arr_f, char_style, para_style, postscript_for, utf16_to_byte_lengths};

/// Parsed `TySh` contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TySh {
    pub transform: Affine,
    pub text: Descriptor,
    pub warp: Option<Descriptor>,
    /// The four trailing big-endian f32 values (left, top, right, bottom) after the warp
    /// descriptor. Photoshop writes zeros for unwarped type and the text-space bounds (the
    /// `bounds` descriptor's rectangle) for warped type.
    pub bounds: [f32; 4],
}

/// Whether a type transform can reach rendering or egui. Pixel coordinates use i32 throughout
/// the document, so non-finite and unrepresentable affine terms are rejected (#4758).
pub fn transform_is_safe(t: &Affine) -> bool {
    t.m.iter().all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX))
}

/// Parses a `TySh` block.
pub fn parse_tysh(data: &[u8]) -> Option<TySh> {
    let f = |at: usize| data.get(at..at + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
    let mut m = [0.0; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f(2 + i * 8)?;
    }
    if !transform_is_safe(&Affine { m }) {
        return None;
    }
    let (text, used) = VersionedDescriptor::parse_prefix(data.get(52..)?).ok()?;
    let rest = data.get(52 + used..).unwrap_or(&[]);
    let (warp, wused) = match rest.get(2..).and_then(|r| VersionedDescriptor::parse_prefix(r).ok()) {
        Some((w, n)) => (Some(w.descriptor), n + 2),
        None => (None, 0),
    };
    let tail = rest.get(wused..).unwrap_or(&[]);
    let mut bounds = [0f32; 4];
    for (i, b) in bounds.iter_mut().enumerate() {
        if let Some(x) = tail.get(i * 4..i * 4 + 4) {
            *b = f32::from_be_bytes(x.try_into().unwrap_or([0; 4]));
        }
    }
    Some(TySh { transform: Affine { m }, text: text.descriptor, warp, bounds })
}

/// `tysh` moved by (`dx`, `dy`): only the transform's translation changes. The text, its styles,
/// the warp and the trailing bounds (which are in text space) stay byte for byte, so a moved
/// Photoshop type layer keeps everything Photoshop wrote. `None` when the block is too short.
pub fn translate_tysh(tysh: &[u8], dx: f64, dy: f64) -> Option<Vec<u8>> {
    let mut out = tysh.to_vec();
    // Version (2 bytes), then xx, xy, yx, yy, tx, ty as big-endian f64: tx at 34, ty at 42.
    for (at, d) in [(34, dx), (42, dy)] {
        let b = out.get_mut(at..at + 8)?;
        let v = f64::from_be_bytes(b.try_into().ok()?) + d;
        b.copy_from_slice(&v.to_be_bytes());
    }
    Some(out)
}

/// Serializes a `TySh` block.
pub fn write_tysh(t: &TySh) -> Vec<u8> {
    let mut v = 1u16.to_be_bytes().to_vec();
    for x in t.transform.m {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&50u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.text.clone()).to_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.warp.clone().unwrap_or_else(default_warp)).to_bytes());
    for b in t.bounds {
        v.extend_from_slice(&b.to_be_bytes());
    }
    v
}

fn default_warp() -> Descriptor {
    Descriptor::new("warp")
        .with("warpStyle", enumv("warpStyle", "warpNone"))
        .with("warpValue", D::Double(0.0))
        .with("warpPerspective", D::Double(0.0))
        .with("warpPerspectiveOther", D::Double(0.0))
        .with("warpRotate", enumv("Ornt", "Hrzn"))
}

fn enumv(t: &str, v: &str) -> D {
    D::Enumerated { type_id: Id::new(t), value: Id::new(v) }
}

fn enum_value(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key) {
        Some(D::Enumerated { value, .. }) => Some(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        _ => None,
    }
}

fn dnum(v: Option<&D>) -> Option<f64> {
    match v {
        Some(D::Double(x)) if x.is_finite() => Some(*x),
        Some(D::UnitFloat { value, .. }) if value.is_finite() => Some(*value),
        Some(D::Integer(i)) => Some(f64::from(*i)),
        _ => None,
    }
}

/// The EngineData of a text descriptor.
pub fn engine_data(text: &Descriptor) -> Option<E> {
    match text.get("EngineData") {
        Some(D::RawData(b)) => ed::parse(b).ok(),
        _ => None,
    }
}

/// Whether the text engine of a `TySh` block has nothing to draw: its EngineData text
/// (`EngineDict/Editor/Text`, what Photoshop lays out) is present and only whitespace. Some
/// Photoshop layers carry their characters only in the descriptor's `Txt ` and render nothing.
pub fn engine_text_is_blank(tysh: &[u8]) -> bool {
    parse_tysh(tysh)
        .and_then(|t| engine_data(&t.text))
        .and_then(|e| e.path(&["EngineDict", "Editor", "Text"]).and_then(E::as_str).map(|s| s.trim().is_empty()))
        .unwrap_or(false)
}

#[cfg(test)]
mod run_length_safety_tests {
    use super::*;

    #[test]
    fn utf16_lengths_stop_after_text_is_consumed() {
        assert_eq!(utf16_to_byte_lengths("é😀", &[1.0, 2.0, 1.0, 1.0e300]), [2, 4]);
        assert_eq!(utf16_to_byte_lengths("", &[1.0, 1.0e300]), [0]);
        assert_eq!(utf16_to_byte_lengths("abc", &[0.0, 1.0, 0.0, 1.0, 1.0, 1.0e300]), [0, 1, 0, 1, 1]);
        let zero_runs = vec![0.0; MAX_ENGINE_RUNS + 1];
        assert_eq!(utf16_to_byte_lengths("a", &zero_runs).len(), MAX_ENGINE_RUNS);
    }

    #[test]
    fn engine_run_length_array_conversion_is_bounded() {
        let values = E::Array(vec![E::Real(0.0); MAX_ENGINE_RUNS + 1]);
        assert_eq!(arr_f(Some(&values)).len(), MAX_ENGINE_RUNS);
    }
}

#[cfg(test)]
mod transform_safety_tests {
    use super::*;

    #[test]
    fn unsafe_affine_terms_are_not_editable_type() {
        let mut t = TySh { transform: Affine::IDENTITY, text: Descriptor::new("TxLr"), warp: None, bounds: [0.0; 4] };
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.315776645269349e62, 1.4759871577491642e307] {
            for i in 0..6 {
                t.transform = Affine::IDENTITY;
                t.transform.m[i] = bad;
                assert!(parse_tysh(&write_tysh(&t)).is_none(), "term {i}: {bad}");
            }
        }
        t.transform = Affine { m: [0.0, -2.01242653341062, 2.286521576443544, 0.0, 586.63, 2452.60] };
        assert_eq!(parse_tysh(&write_tysh(&t)).unwrap().transform, t.transform);
    }
}

#[cfg(test)]
mod nonfinite_tests {
    use super::*;
    use crate::engine_data::parse;

    #[test]
    fn non_finite_style_numbers_fall_back_to_defaults() {
        let d = parse(b"<< /FontSize 1e999 /Tracking nan /HorizontalScale inf /Kerning -inf /FillColor << /Values [1.0 nan 0.0 0.0] >> >>").unwrap();
        let s = char_style(&[], &d, &[], 1.0);
        let dflt = CharStyle::default();
        assert_eq!(s.size_pt, dflt.size_pt);
        assert_eq!((s.tracking, s.horizontal_scale, s.kern), (0.0, 1.0, 0.0));
        assert_eq!(s.color, Color::BLACK);
        // A finite f64 too big for f32 is dropped as well.
        let d = parse(b"<< /FontSize 1e300 /StartIndent inf >> ").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).size_pt, dflt.size_pt);
        assert_eq!(para_style(&[], &d, 1.0).start_indent_pt, 0.0);
        let d = parse(b"<< /FontSize 24.0 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).size_pt, 24.0);
    }

    /// #5645: `FontBaseline` (1 superscript, 2 subscript) is read and written back.
    #[test]
    fn font_baseline_round_trips() {
        use photocraft_doc::text::FontBaseline;
        let d = parse(b"<< /FontBaseline 2 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).baseline, FontBaseline::Subscript);
        let d = parse(b"<< /FontBaseline 1 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).baseline, FontBaseline::Superscript);
        let s = CharStyle { baseline: FontBaseline::Subscript, ..Default::default() };
        let back = char_style(&[], &style_sheet_data(&s, 0, 1.0), &[], 1.0);
        assert_eq!(back.baseline, FontBaseline::Subscript);
    }

    #[test]
    fn area_path_that_is_not_finite_saves_as_point_text() {
        let path = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[(0.0, 0.0), (f64::NAN, 0.0), (10.0, f64::INFINITY)])]);
        let layer = TextLayer {
            text: "x".into(),
            runs: vec![TextRun { len: 1, style: CharStyle { size_pt: 12.0, ..Default::default() } }],
            shape: TextShape::Area { path },
            ..Default::default()
        };
        let out = String::from_utf8_lossy(&crate::engine_data::write(&build_engine_data(&layer, None, 72.0))).into_owned();
        assert!(out.contains("/PointBase") && !out.contains("/BoxBounds"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("nan") && !out.to_ascii_lowercase().contains("inf"), "{out}");
        let t = parse_tysh(&build_tysh(&layer, 72.0, None)).unwrap();
        for key in ["bounds", "boundingBox"] {
            let Some(D::Descriptor(d)) = t.text.get(key) else { panic!("no {key}") };
            for side in ["Left", "Top ", "Rght", "Btom"] {
                assert!(matches!(d.get(side), Some(D::UnitFloat { value, .. }) if value.is_finite()), "{key} {side}: {:?}", d.get(side));
            }
        }
    }

    #[test]
    fn warp_values_beyond_f32_clamp_to_the_warp_range() {
        let warp = Descriptor::new("warp")
            .with("warpStyle", enumv("warpStyle", "warpArc"))
            .with("warpValue", D::Double(1e39))
            .with("warpPerspective", D::Double(-1e39))
            .with("warpPerspectiveOther", D::Double(250.0))
            .with("warpRotate", enumv("Ornt", "Hrzn"));
        let tysh = write_tysh(&TySh { transform: Affine::IDENTITY, text: Descriptor::new("TxLr"), warp: Some(warp), bounds: [0.0; 4] });
        let w = text_layer_from_tysh(&tysh, 72.0).and_then(|l| l.warp).unwrap();
        assert_eq!((w.value, w.horizontal_distortion, w.vertical_distortion), (100.0, -100.0, 100.0));
    }

    #[test]
    fn infinite_dpi_does_not_write_a_zero_size() {
        let layer =
            TextLayer { text: "x".into(), runs: vec![TextRun { len: 1, style: CharStyle { size_pt: 12.0, ..Default::default() } }], ..Default::default() };
        let out = String::from_utf8_lossy(&crate::engine_data::write(&build_engine_data(&layer, None, f32::INFINITY))).into_owned();
        assert!(out.contains("/FontSize 12.0"), "{out}");
    }
}
