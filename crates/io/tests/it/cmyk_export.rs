//! A CMYK document is written to flat formats as CMYK with its own profile whatever its layers
//! (#2220), mixing layers on their ink values so opaque pixels keep their exact inks.

use std::sync::Arc;

use photocraft_cms::Builtin;
use photocraft_codecs::ChannelLayout;
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};
use photocraft_io::*;

fn cmyk_doc(depth: SampleType) -> Document {
    let mut d = Document::new("c", Size::new(12, 8), ColorMode::Cmyk, depth);
    d.icc_profile = Some(Arc::new(Builtin::CoatedCmyk.profile().to_bytes().to_vec()));
    d
}

fn fill(depth: SampleType, alpha: bool, rects: &[(Rect, [f32; 5])]) -> Layer {
    let mut l = Layer::raster("l", PixelFormat::new(ColorMode::Cmyk, depth, alpha));
    if let Some(s) = l.surface_mut() {
        for (r, c) in rects {
            s.fill_rect(*r, &c[..if alpha { 5 } else { 4 }]);
        }
    }
    l
}

/// Paper (no ink), K-only black "type", and a four-colour patch, as separate layers.
fn layered(depth: SampleType) -> Document {
    let mut d = cmyk_doc(depth);
    d.layers.push(fill(depth, false, &[(Rect::new(0, 0, 12, 8), [0.0; 5])]));
    d.layers.push(fill(depth, true, &[(Rect::new(0, 0, 6, 8), [0.0, 0.0, 0.0, 1.0, 1.0])]));
    d.layers.push(fill(depth, true, &[(Rect::new(6, 0, 12, 4), [0.2, 0.4, 0.6, 0.1, 1.0])]));
    d
}

fn decode(bytes: &[u8]) -> photocraft_codecs::Image {
    photocraft_codecs::decode(bytes).unwrap_or_else(|e| panic!("decode: {e}"))
}

/// Normalized C, M, Y, K (and alpha) of pixel (x, y).
fn pixel(img: &photocraft_codecs::Image, x: u32, y: u32) -> Vec<f32> {
    let n = img.layout().channels();
    let (w, _) = img.dimensions();
    let v = img.to_normalized();
    let i = ((y * w + x) as usize) * n;
    v[i..i + n].to_vec()
}

fn q8(v: f32) -> f32 {
    (v * 255.0).round() / 255.0
}

#[test]
fn layered_cmyk_tiff_stays_cmyk_with_the_document_profile_and_exact_inks() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = layered(depth);
        let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{depth:?}: {e}"));
        assert!(!r.warnings.iter().any(|w| w.contains("written as sRGB") || w.contains("converted back")), "{depth:?}: {:?}", r.warnings);
        let img = decode(&r.bytes);
        assert_eq!(img.layout(), ChannelLayout::Cmyk, "{depth:?}: opaque CMYK, no alpha");
        assert_eq!(img.icc.as_deref(), d.icc_profile.as_deref().map(|v| &v[..]), "{depth:?}: the document's own profile");
        // Exact inks: K-only black stays K-only, the four-colour patch keeps every ink, paper stays blank.
        let (k, rich, paper) = (pixel(&img, 2, 3), pixel(&img, 8, 1), pixel(&img, 8, 6));
        let tol = if depth == SampleType::U8 { 0.0 } else { 1e-4 };
        let close = |a: &[f32], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol);
        let want_rich = if depth == SampleType::U8 { [q8(0.2), q8(0.4), q8(0.6), q8(0.1)] } else { [0.2, 0.4, 0.6, 0.1] };
        assert!(close(&k, [0.0, 0.0, 0.0, 1.0]), "{depth:?}: {k:?}");
        assert!(close(&rich, want_rich), "{depth:?}: {rich:?}");
        assert!(close(&paper, [0.0; 4]), "{depth:?}: {paper:?}");
        // And it reads back as a CMYK document.
        let back = import("a.tif", &r.bytes).unwrap_or_else(|e| panic!("{e}")).document;
        assert_eq!(back.mode, ColorMode::Cmyk, "{depth:?}");
    }
}

#[test]
fn the_reported_case_an_empty_extra_layer() {
    // #2220: one layer converted to CMYK plus an empty layer used to export as RGB.
    let mut d = cmyk_doc(SampleType::U8);
    d.layers.push(fill(SampleType::U8, false, &[(Rect::new(0, 0, 12, 8), [0.3, 0.2, 0.1, 0.0, 1.0])]));
    d.layers.push(Layer::raster("empty", PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true)));
    for ext in ["tif", "jpg"] {
        let r = export(&d, &format!("a.{ext}"), &ExportOptions::default()).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(decode(&r.bytes).layout(), ChannelLayout::Cmyk, "{ext}");
    }
}

#[test]
fn transparency_is_kept_in_tiff_and_composited_over_paper_in_jpeg() {
    let mut d = cmyk_doc(SampleType::U8);
    d.layers.push(fill(SampleType::U8, true, &[(Rect::new(0, 0, 6, 8), [0.0, 0.0, 0.0, 1.0, 1.0])]));
    d.layers.push(fill(SampleType::U8, true, &[(Rect::new(6, 0, 12, 4), [0.0, 1.0, 0.0, 0.0, 0.5])]));
    let tif = decode(&export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}")).bytes);
    assert_eq!(tif.layout(), ChannelLayout::CmykA);
    assert_eq!(pixel(&tif, 8, 1), vec![0.0, 1.0, 0.0, 0.0, q8(0.5)], "straight ink and coverage");
    assert_eq!(pixel(&tif, 8, 6)[4], 0.0, "uncovered stays transparent");
    let r = export(&d, "a.jpg", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    let jpg = decode(&r.bytes);
    assert_eq!(jpg.layout(), ChannelLayout::Cmyk, "{:?}", r.warnings);
    // JPEG is lossy: half-covered magenta over blank paper is about M50; uncovered paper has no ink.
    let (half, blank, k) = (pixel(&jpg, 9, 1), pixel(&jpg, 9, 6), pixel(&jpg, 2, 3));
    assert!((half[1] - 0.5).abs() < 0.06 && half[0] < 0.06 && half[3] < 0.06, "{half:?}");
    assert!(blank.iter().all(|v| *v < 0.06), "{blank:?}");
    assert!(k[3] > 0.94 && k[..3].iter().all(|v| *v < 0.06), "{k:?}");
}

#[test]
fn multiply_adds_inks_exactly() {
    // Multiply darkens: each ink of the result is b + s − b·s, with no ink the layers don't have.
    let mut d = layered(SampleType::U16);
    let mut m = fill(SampleType::U16, true, &[(Rect::new(0, 0, 12, 8), [0.0, 0.5, 0.0, 0.0, 1.0])]);
    m.blend = BlendMode::Multiply;
    d.layers.push(m);
    let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(!r.warnings.iter().any(|w| w.contains("converted back")), "{:?}", r.warnings);
    let img = decode(&r.bytes);
    assert_eq!(img.layout(), ChannelLayout::Cmyk);
    let mul = |b: f32, s: f32| b + s - b * s;
    let close = |a: &[f32], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-4);
    let (k, rich, paper) = (pixel(&img, 2, 3), pixel(&img, 8, 1), pixel(&img, 8, 6));
    assert!(close(&k, [0.0, 0.5, 0.0, 1.0]), "{k:?}");
    assert!(close(&rich, [0.2, mul(0.4, 0.5), 0.6, 0.1]), "{rich:?}");
    assert!(close(&paper, [0.0, 0.5, 0.0, 0.0]), "{paper:?}");
}

#[test]
fn other_stacks_stay_cmyk_through_the_profile() {
    // Hue mixes channels, so it isn't composited on ink values: the RGB composite is converted back.
    let mut d = layered(SampleType::U8);
    let mut m = fill(SampleType::U8, true, &[(Rect::new(0, 0, 12, 8), [0.0, 0.5, 0.0, 0.0, 1.0])]);
    m.blend = BlendMode::Hue;
    d.layers.push(m);
    let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(r.warnings.iter().any(|w| w.contains("converted back through the document's CMYK profile")), "{:?}", r.warnings);
    let img = decode(&r.bytes);
    assert_eq!(img.layout(), ChannelLayout::Cmyk);
    assert_eq!(img.icc.as_deref(), d.icc_profile.as_deref().map(|v| &v[..]));
    // Magenta's hue on the four-colour patch shifts its colour; the page still has ink.
    assert!(pixel(&img, 2, 3)[3] > 0.5, "{:?}", pixel(&img, 2, 3));
}

#[test]
fn rgb_only_formats_still_get_rgb_through_the_profile() {
    let r = export(&layered(SampleType::U8), "a.png", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    let img = decode(&r.bytes);
    assert!(matches!(img.layout(), ChannelLayout::Rgb | ChannelLayout::Rgba), "{:?}", img.layout());
    assert!(r.warnings.iter().any(|w| w.contains("Cmyk composite written as sRGB RGB")), "{:?}", r.warnings);
}

#[test]
fn ink_adjustments_export_exact_inks() {
    // Invert in CMYK turns every ink into its opposite: blank paper becomes four-colour black,
    // K-only black loses its K and gains full C, M and Y.
    let mut d = layered(SampleType::U8);
    d.layers.push(Layer::new("inv", photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(!r.warnings.iter().any(|w| w.contains("converted back")), "{:?}", r.warnings);
    let img = decode(&r.bytes);
    assert_eq!(img.layout(), ChannelLayout::Cmyk);
    assert_eq!(pixel(&img, 8, 6), vec![1.0, 1.0, 1.0, 1.0], "paper");
    assert_eq!(pixel(&img, 2, 3), vec![1.0, 1.0, 1.0, 0.0], "K-only black");
    let inv = |v: f32| (255.0 - (v * 255.0).round()) / 255.0;
    assert_eq!(pixel(&img, 8, 1), vec![inv(0.2), inv(0.4), inv(0.6), inv(0.1)], "four-colour patch");
}

#[test]
fn channel_weighing_adjustments_go_through_the_profile() {
    // Threshold weighs the channels together: it isn't an ink-by-ink edit.
    let mut d = layered(SampleType::U8);
    d.layers.push(Layer::new("t", photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::Threshold { level: 0.5 })));
    let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(r.warnings.iter().any(|w| w.contains("converted back through the document's CMYK profile")), "{:?}", r.warnings);
    assert_eq!(decode(&r.bytes).layout(), ChannelLayout::Cmyk);
}

#[test]
fn layer_effects_export_their_inks() {
    // K-only type with a K-only drop shadow on blank paper: the TIFF has black ink only.
    use photocraft_doc::effects::{Contour, Effect, FxCommon, Shadow};
    let mut d = cmyk_doc(SampleType::U8);
    d.layers.push(fill(SampleType::U8, false, &[(Rect::new(0, 0, 12, 8), [0.0; 5])]));
    let mut t = fill(SampleType::U8, true, &[(Rect::new(2, 2, 6, 5), [0.0, 0.0, 0.0, 1.0, 1.0])]);
    t.effects.enabled = true;
    t.effects.items.push(Effect::DropShadow(Shadow {
        common: FxCommon::new(BlendMode::Multiply, 0.75),
        color: photocraft_color::Color { mode: ColorMode::Cmyk, c: [0.0, 0.0, 0.0, 1.0], alpha: 1.0 },
        angle: 135.0,
        use_global_light: false,
        distance: 2.0,
        spread: 0.0,
        size: 1.0,
        contour: Contour::Linear,
        anti_alias: false,
        noise: 0.0,
        knocks_out: true,
    }));
    d.layers.push(t);
    let r = export(&d, "a.tif", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert!(!r.warnings.iter().any(|w| w.contains("converted back")), "{:?}", r.warnings);
    let img = decode(&r.bytes);
    assert_eq!(img.layout(), ChannelLayout::Cmyk);
    let mut shadow = 0;
    for y in 0..8 {
        for x in 0..12 {
            let p = pixel(&img, x, y);
            assert_eq!(&p[..3], &[0.0; 3], "({x},{y}) {p:?}");
            shadow += usize::from(p[3] > 0.0 && p[3] < 1.0);
        }
    }
    assert!(shadow > 0, "a soft shadow");
    assert_eq!(pixel(&img, 3, 3), vec![0.0, 0.0, 0.0, 1.0], "type");
}

#[test]
fn psd_merged_image_keeps_exact_inks() {
    // The PSD's merged image (what other apps show and print) is mixed on the inks too: K-only
    // black under a Multiply layer stays K-only, and the four-colour patch keeps its inks.
    let mut d = layered(SampleType::U8);
    let mut m = fill(SampleType::U8, true, &[(Rect::new(0, 0, 12, 8), [0.0, 0.4, 0.0, 0.0, 1.0])]);
    m.blend = BlendMode::Multiply;
    d.layers.push(m);
    let r = export(&d, "a.psd", &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    let f = photocraft_psd::PsdFile::from_bytes(&r.bytes).unwrap_or_else(|e| panic!("{e}"));
    let planes = f.decode_merged().unwrap_or_else(|e| panic!("{e}"));
    let n = 12 * 8;
    // Planar, inverted (255 = no ink).
    let ink = |c: usize, x: usize, y: usize| 255 - i32::from(planes.get(c * n + y * 12 + x).copied().unwrap_or(0));
    let q = |v: f32| (v * 255.0).round() as i32;
    assert_eq!([ink(0, 2, 3), ink(1, 2, 3), ink(2, 2, 3), ink(3, 2, 3)], [0, q(0.4), 0, 255], "K-only black with magenta multiplied in");
    let mul = |b: f32, s: f32| b + s - b * s;
    let qb = |v: f32| ((v * 255.0).round()) / 255.0;
    let want = [q(0.2), q(mul(qb(0.4), qb(0.4))), q(0.6), q(0.1)];
    let got = [ink(0, 8, 1), ink(1, 8, 1), ink(2, 8, 1), ink(3, 8, 1)];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1), "{got:?} vs {want:?}");
}
