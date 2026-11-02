//! GIMP XCF files written by GIMP 3.2.6 itself (feature `corpus`; `cargo xtask test-corpus -p io`).
//! They come from `corpus/gimp/` (`cargo xtask corpus --gimp`; pin `PHOTOCRAFT_CORPUS_GIMP_COMMIT` in
//! `xtask/src/corpus_pins.rs`, generator `tools/gimp-xcf/xcf_fixtures.py` in photocraft-corpus); a
//! missing corpus fails. What the reader gets out is checked against the procedural images the
//! generator drew (`pattern`, `alpha`), and our composite of each file against GIMP's own merged
//! rendering saved next to it (`<name>.png`).

#![cfg(feature = "corpus")]

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_io::{ExportOptions, export, import};

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}: run `cargo xtask corpus --gimp`", p.display()))
}

fn open(name: &str) -> photocraft_io::ImportResult {
    import(name, &fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

const W: u32 = 40;
const H: u32 = 30;

fn pattern(x: u32, y: u32, c: u32, maxval: u32) -> u32 {
    (x * 31 + y * 17 + c * 101 + 7) * 13 % (maxval + 1)
}

fn alpha(x: u32, y: u32, w: u32, h: u32, maxval: u32) -> u32 {
    ((x + y) * maxval / (w + h - 4).max(1)).min(maxval)
}

fn raster(layer: &Layer) -> &photocraft_raster::Surface {
    match &layer.content {
        LayerContent::Raster(s) => s,
        _ => panic!("{} is not a pixel layer", layer.name),
    }
}

fn px(layer: &Layer, x: i32, y: i32) -> Vec<f32> {
    raster(layer).read_region(Rect::new(x, y, x + 1, y + 1))
}

fn names(layers: &[Layer]) -> Vec<&str> {
    layers.iter().map(|l| l.name.as_str()).collect()
}

/// Largest premultiplied channel difference between our composite and GIMP's merged PNG, and
/// the share of pixels over `tolerance`.
fn composite_error(doc: &Document, png: &str, tolerance: f32) -> (f32, f32, String) {
    let oracle = import(png, &fixture(png)).unwrap().document;
    assert_eq!(doc.size, oracle.size, "{png}");
    let ours = photocraft_compose::flatten(doc);
    let theirs = photocraft_compose::flatten(&oracle);
    let w = doc.size.width as usize;
    let mut worst = (0.0f32, String::new());
    let mut over = 0usize;
    for (i, (a, b)) in ours.px.iter().zip(&theirs.px).enumerate() {
        let e = (0..4).map(|c| if c == 3 { (a[3] - b[3]).abs() } else { (a[c] * a[3] - b[c] * b[3]).abs() }).fold(0.0f32, f32::max);
        over += usize::from(e > tolerance);
        if e > worst.0 {
            let q = |p: &[f32; 4]| p.map(|v| (v * 255.0).round() as i32);
            worst = (e, format!("at ({}, {}): ours {:?}, GIMP {:?}", i % w, i / w, q(a), q(b)));
        }
    }
    (worst.0, over as f32 / ours.px.len().max(1) as f32, worst.1)
}

#[test]
fn layers_8bit_open_with_their_properties_and_pixels() {
    let r = open("layers-8bit.xcf");
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height, d.mode, d.depth), (W, H, ColorMode::Rgb, SampleType::U8));
    assert_eq!(names(&d.layers), ["Background", "Multiply 60%", "Screen off-canvas", "Hidden"]);
    assert!((d.resolution_dpi - 300.0).abs() < 0.01);
    assert_eq!((d.guides.horizontal.as_slice(), d.guides.vertical.as_slice()), (&[10.0][..], &[25.0][..]));
    assert!(r.source_read_only);
    // The background: opaque, every sample as drawn.
    let bg = &d.layers[0];
    assert!(bg.locks.transparency, "an opaque XCF layer opens as a locked-transparency layer");
    for (x, y) in [(0, 0), (39, 29), (17, 11)] {
        let p = px(bg, x as i32, y as i32);
        for c in 0..3 {
            assert!((p[c] * 255.0 - pattern(x, y, c as u32, 255) as f32).abs() < 0.51, "background at {x},{y} channel {c}: {p:?}");
        }
        assert_eq!(p[3], 1.0);
    }
    // Multiply at 60%, offset (5, 4), lock alpha, green tag.
    let m = &d.layers[1];
    assert_eq!((m.blend, m.opacity, m.visible, m.locks.transparency, m.label), (BlendMode::Multiply, 0.6, true, true, photocraft_doc::LabelColor::Green));
    let p = px(m, 5, 4);
    assert!(
        (p[0] * 255.0 - pattern(3, 0, 0, 255) as f32).abs() < 0.51 && p[3] == 0.0,
        "top-left of the layer is transparent (alpha gradient starts at 0): {p:?}"
    );
    let p = px(m, 24, 19);
    assert!((p[0] * 255.0 - pattern(19 + 3, 15, 0, 255) as f32).abs() < 0.51 && (p[3] * 255.0 - alpha(19, 15, 20, 16, 255) as f32).abs() < 0.51, "{p:?}");
    assert_eq!(px(m, 4, 4)[3], 0.0, "outside the layer");
    // Screen, partly off-canvas at (-8, 20): its pixels at x < 0 exist.
    let s = &d.layers[2];
    assert_eq!(s.blend, BlendMode::Screen);
    assert!((px(s, -8, 20)[0] * 255.0 - pattern(7, 0, 0, 255) as f32).abs() < 0.51);
    assert!((px(s, 15, 31)[3] * 255.0 - alpha(23, 11, 24, 12, 255) as f32).abs() < 0.51);
    // Hidden.
    assert!(!d.layers[3].visible);
    // The only note: GIMP composites this file in linear light.
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    assert!(r.warnings[0].contains("linear light"), "{:?}", r.warnings);
}

/// Per mode: our composite of the `legacy-modes-perceptual.xcf` cell against GIMP's. Modes with
/// a Photoshop equivalent must match; the approximated ones are reported.
#[test]
fn each_mode_against_gimp_in_gamma_space() {
    let d = open("legacy-modes-perceptual.xcf").document;
    let oracle = import("legacy-modes-perceptual.png", &fixture("legacy-modes-perceptual.png")).unwrap().document;
    let ours = photocraft_compose::flatten(&d);
    let theirs = photocraft_compose::flatten(&oracle);
    let w = d.size.width as usize;
    // Where the layer pixel is fully opaque the formulas must agree. Where it is translucent,
    // GIMP applies the pixel's alpha before clamping the result of a mode that can leave 0..1
    // (Addition, Subtract, Divide, Dodge, Burn, Linear light) and Photoshop clamps first, so
    // those pixels are only reported.
    let exact = ["Addition", "Subtract", "Divide", "Dodge", "Burn", "Hard light", "Exclusion", "Linear light"];
    let mut report = Vec::new();
    let mut failed = false;
    for (i, layer) in d.layers.iter().skip(1).enumerate() {
        let (x0, y0) = ((i % 5) * 8, (i / 5) * 10);
        let (mut worst_opaque, mut worst_all) = (0.0f32, 0.0f32);
        for y in y0..y0 + 8 {
            for x in x0..x0 + 8 {
                let (a, b) = (ours.px[y * w + x], theirs.px[y * w + x]);
                let e = (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max);
                worst_all = worst_all.max(e);
                let opaque = alpha((x - x0) as u32, (y - y0) as u32, 8, 8, 255) == 255;
                if opaque {
                    worst_opaque = worst_opaque.max(e);
                }
            }
        }
        let must = exact.contains(&layer.name.as_str());
        let ok = !must || worst_opaque <= 2.5 / 255.0;
        failed |= !ok;
        report.push(format!(
            "{} {:<14} {:?}: opaque pixels worst {:.1}/255, all {:.1}/255",
            if ok { "ok  " } else { "FAIL" },
            layer.name,
            layer.blend,
            worst_opaque * 255.0,
            worst_all * 255.0
        ));
    }
    println!("{}", report.join("\n"));
    assert!(!failed, "\n{}", report.join("\n"));
}

#[test]
fn groups_nest_and_pass_through() {
    let r = open("groups.xcf");
    let d = &r.document;
    assert_eq!(names(&d.layers), ["Background", "Group", "Top"]);
    let g = &d.layers[1];
    assert!(g.is_group());
    assert!((g.opacity - 0.7).abs() < 1e-6);
    assert_eq!(g.blend, BlendMode::Normal, "a GIMP group in Normal mode composites as a unit");
    let children = g.children().unwrap();
    assert_eq!(names(children), ["Child Multiply", "Nested pass-through"]);
    assert_eq!(children[0].blend, BlendMode::Multiply);
    let nested = &children[1];
    assert!(nested.is_group());
    assert_eq!(nested.blend, BlendMode::PassThrough);
    assert_eq!(names(nested.children().unwrap()), ["Nested child Screen"]);
    assert_eq!(nested.children().unwrap()[0].blend, BlendMode::Screen);
    assert!((px(&nested.children().unwrap()[0], 18, 10)[0] * 255.0 - pattern(5, 0, 0, 255) as f32).abs() < 0.51, "nested child at its offset");
}

#[test]
fn masks_open_applied_or_disabled() {
    let d = open("mask.xcf").document;
    assert_eq!(names(&d.layers), ["Background", "Masked", "Mask disabled"]);
    let masked = d.layers[1].mask.as_ref().expect("mask");
    assert!(masked.enabled);
    // A left-to-right gradient across 30 px at the layer's offset (5, 5). The script drew it as
    // gamma-encoded values and GIMP stores mask coverage linearly, so the midpoint is where
    // sRGB 15/29 lands in linear light.
    let v = |x: i32| masked.surface.read_region(Rect::new(x, 10, x + 1, 11))[0];
    let linear = |s: f32| if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) };
    assert!(v(5) < 0.01 && (v(5 + 29) - 1.0).abs() < 0.01 && (v(5 + 15) - linear(15.0 / 29.0)).abs() < 0.01, "{} {} {}", v(5), v(34), v(20));
    let disabled = d.layers[2].mask.as_ref().expect("mask");
    assert!(!disabled.enabled);
    assert!(disabled.surface.read_region(Rect::new(20, 20, 21, 21))[0] < 0.01, "a black mask, kept but not applied");
}

#[test]
fn gray16_keeps_its_depth_and_mode() {
    let d = open("gray16.xcf").document;
    assert_eq!((d.mode, d.depth), (ColorMode::Grayscale, SampleType::U16));
    assert_eq!(names(&d.layers), ["Background", "Gray alpha"]);
    let bg = &d.layers[0];
    assert_eq!(raster(bg).channels(), 2);
    for (x, y) in [(0, 0), (39, 29), (13, 7)] {
        let p = px(bg, x as i32, y as i32);
        assert!((p[0] * 65535.0 - pattern(x, y, 0, 65535) as f32).abs() < 0.6, "gray at {x},{y}: {p:?}");
    }
    let top = &d.layers[1];
    let p = px(top, 10 + 19, 8 + 14);
    assert!((p[0] * 65535.0 - pattern(19 + 9, 14, 0, 65535) as f32).abs() < 0.6 && (p[1] * 65535.0 - alpha(19, 14, 20, 15, 65535) as f32).abs() < 0.6, "{p:?}");
}

#[test]
fn float_linear_opens_as_32_bit_float_with_a_linear_profile() {
    let d = open("float-linear.xcf").document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32));
    let icc = d.icc_profile.as_ref().expect("a linear-light profile");
    assert_eq!(**icc, photocraft_cms::Builtin::LinearSrgb.profile().to_bytes().as_ref().clone());
    let bg = &d.layers[0];
    for (x, y) in [(0, 0), (39, 29), (5, 21)] {
        let p = px(bg, x as i32, y as i32);
        for (c, v) in p.iter().take(3).enumerate() {
            assert_eq!(*v, (x * 3 + y * 5 + c as u32 * 7) as f32 / 64.0, "float at {x},{y} channel {c}");
        }
    }
    assert!(px(bg, 39, 29)[0] > 1.0, "values above 1 are kept");
    let top = &d.layers[1];
    let p = px(top, 12 + 4, 7 + 6);
    assert_eq!(p[3], 10.0 / 30.0);
}

#[test]
fn indexed_opens_as_rgb_with_a_note() {
    let r = open("indexed.xcf");
    assert_eq!((r.document.mode, r.document.depth), (ColorMode::Rgb, SampleType::U8));
    assert!(r.warnings.iter().any(|w| w.contains("indexed")), "{:?}", r.warnings);
    assert_eq!(names(&r.document.layers), ["Background", "Top"]);
}

#[test]
fn text_layers_open_as_pixels_with_a_note() {
    let r = open("text.xcf");
    assert_eq!(names(&r.document.layers), ["Background", "XCF"]);
    assert!(r.warnings.iter().any(|w| w.contains("text layer")), "{:?}", r.warnings);
    let text = &r.document.layers[1];
    let s = raster(text);
    let region = s.read_region(Rect::new(10, 10, 10 + 60, 10 + 30));
    assert!(region.as_chunks::<4>().0.iter().any(|p| p[3] > 0.5), "the rendered glyphs are there");
}

#[test]
fn channels_and_selection_come_through() {
    let d = open("channel.xcf").document;
    assert_eq!(d.channels.len(), 1);
    assert_eq!(d.channels[0].name, "Alpha 1");
    let v = |y: i32| d.channels[0].surface.read_region(Rect::new(3, y, 4, y + 1))[0];
    assert!(v(0) < 0.01 && (v(29) - 1.0).abs() < 0.01, "a top-to-bottom gradient");
    let sel = d.selection.as_ref().expect("the saved selection");
    assert!((sel.read_region(Rect::new(10, 10, 11, 11))[0] - 1.0).abs() < 0.01 && sel.read_region(Rect::new(1, 1, 2, 2))[0] < 0.01);
}

#[test]
fn modes_map_to_photoshops_and_the_rest_warn() {
    let r = open("legacy-modes.xcf");
    let by_name = |n: &str| r.document.layers.iter().find(|l| l.name == n).unwrap_or_else(|| panic!("{n}"));
    assert_eq!(by_name("Addition").blend, BlendMode::LinearDodge);
    assert_eq!(by_name("Subtract").blend, BlendMode::Subtract);
    assert_eq!(by_name("Divide").blend, BlendMode::Divide);
    assert_eq!(by_name("Dodge").blend, BlendMode::ColorDodge);
    assert_eq!(by_name("Burn").blend, BlendMode::ColorBurn);
    assert_eq!(by_name("Hard light").blend, BlendMode::HardLight);
    assert_eq!(by_name("Exclusion").blend, BlendMode::Exclusion);
    assert_eq!(by_name("Linear light").blend, BlendMode::LinearLight);
    assert_eq!(by_name("Grain merge").blend, BlendMode::Normal);
    assert_eq!(by_name("HSV Hue").blend, BlendMode::Hue);
    assert_eq!(by_name("LCH Color").blend, BlendMode::Color);
    let w = r.warnings.join("\n");
    assert!(w.contains("Grain merge") && w.contains("HSV Value") && w.contains("LCH Color"), "{w}");
}

#[test]
fn xcf_cannot_be_written_and_says_so() {
    let d = open("layers-8bit.xcf").document;
    let e = export(&d, "out.xcf", &ExportOptions::default()).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("pcraft"), "{e}");
}

#[test]
fn broken_files_are_errors_not_crashes() {
    let bytes = fixture("groups.xcf");
    assert!(import("x.xcf", b"gimp xcf v011\0").is_err());
    assert!(import("x.xcf", &bytes[..bytes.len() / 2]).is_err());
    for cut in (0..bytes.len()).step_by(7) {
        let _ = import("x.xcf", &bytes[..cut]);
    }
}

/// Our composite against GIMP's merged rendering. The `-perceptual` files have every layer set
/// to blend and composite in gamma-encoded RGB, which is how PhotoCraft composes, so they must
/// match tightly. GIMP's defaults compose in linear light, so those files measure the known gap
/// (the tolerance and the allowed share of pixels over it are the numbers seen, not goals).
/// Per file: tolerance (premultiplied, 0..1), the share of pixels allowed over it, and why.
const COMPOSITES: &[(&str, f32, f32, &str)] = &[
    ("layers-8bit-perceptual.xcf", 2.5 / 255.0, 0.0, "Normal, Multiply and Screen in gamma space"),
    (
        "groups-perceptual.xcf",
        2.5 / 255.0,
        0.16,
        "groups and pass-through in gamma space; the 15% over are where GIMP's Clip to backdrop hides the Multiply and Screen children above the transparent part of the background",
    ),
    ("mask-perceptual.xcf", 2.5 / 255.0, 0.0, "masks in gamma space"),
    ("text-perceptual.xcf", 2.5 / 255.0, 0.0, "antialiased text in gamma space"),
    (
        "legacy-modes-perceptual.xcf",
        2.5 / 255.0,
        0.40,
        "modes without an equivalent open as Normal and HSV/LCH modes are approximated; see each_mode_against_gimp_in_gamma_space",
    ),
    ("gray16.xcf", 2.0 / 255.0, 0.0, "16-bit gray, Normal"),
    ("indexed.xcf", 2.0 / 255.0, 0.0, "indexed to RGB"),
    ("channel.xcf", 2.0 / 255.0, 0.0, "channels are not rendered"),
    ("layers-8bit.xcf", 40.0 / 255.0, 0.0, "GIMP's default: Multiply blended in linear light"),
    ("groups.xcf", 140.0 / 255.0, 0.0, "GIMP's default: linear light and Clip to backdrop over a transparent background"),
    ("mask.xcf", 60.0 / 255.0, 0.0, "GIMP's default: alpha composited in linear light"),
    ("text.xcf", 75.0 / 255.0, 0.0, "GIMP's default: antialiased edges composited in linear light"),
];

#[test]
fn composites_match_gimps_merged_rendering() {
    let mut report = Vec::new();
    let mut failed = false;
    for (name, tolerance, allowed, why) in COMPOSITES {
        let d = open(name).document;
        let png = name.replace(".xcf", ".png");
        let (worst, over, at) = composite_error(&d, &png, *tolerance);
        let ok = over <= *allowed;
        failed |= !ok;
        report.push(format!(
            "{} {name}: worst {:.1}/255, {:.1}% over {:.1}/255 ({why}) {at}",
            if ok { "ok  " } else { "FAIL" },
            worst * 255.0,
            over * 100.0,
            tolerance * 255.0
        ));
    }
    println!("{}", report.join("\n"));
    assert!(!failed, "\n{}", report.join("\n"));
}
