//! A shape layer's own Density and Feather (Properties › Masks: its path is its vector mask),
//! against Photoshop's measured pixels and against a fill layer with an equal vector mask.

use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::vector::{Path, ShapeLayer, ShapeStroke, Subpath, VectorMask};
use photocraft_doc::{Fill, Layer, LayerContent};
use photocraft_geom::Size;

/// Photoshop: new 200×200 white RGB document, a black 100×100 rectangle shape at 50..150 with a
/// 10 px vector-mask feather, row y = 100 at x = 30, 33, …, 72 (8-bit grey levels).
const PS_FEATHER_10: [u8; 15] = [249, 242, 232, 216, 195, 170, 142, 113, 85, 60, 39, 23, 13, 6, 2];

fn square() -> Path {
    Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (150.0, 50.0), (150.0, 150.0), (50.0, 150.0)])])
}

fn doc(depth: SampleType) -> Document {
    Document::with_background("t", Size::new(200, 200), ColorMode::Rgb, depth, Color::WHITE)
}

/// The document with a black square shape layer of `density` (0..1) and `feather` (px).
fn shape_doc(depth: SampleType, density: f32, feather: f32, stroke: Option<ShapeStroke>) -> Document {
    let mut d = doc(depth);
    let mut sh = ShapeLayer { path: square(), fill: Some(Fill::Solid(Color::BLACK)), stroke, density, feather, ..ShapeLayer::default() };
    sh.cache = Some(photocraft_vector::render_shape(&sh, d.pixel_format(), d.bounds()));
    d.layers.push(Layer::new("Rectangle 1", LayerContent::Shape(sh)));
    d
}

/// The same appearance built as Photoshop stores it: a black fill layer with a vector mask.
fn fill_doc(depth: SampleType, density: f32, feather: f32) -> Document {
    let mut d = doc(depth);
    let mut l = Layer::new("Color Fill 1", LayerContent::Fill(Fill::Solid(Color::BLACK)));
    l.vector_mask = Some(VectorMask { density, feather, ..VectorMask::new(square()) });
    d.layers.push(l);
    d
}

/// Row `y` of the composite as 8-bit grey levels.
fn row(d: &Document, y: i32) -> Vec<i32> {
    render(d, Rect::new(0, y, 200, y + 1)).px.iter().map(|p| (p[0] * 255.0).round() as i32).collect()
}

/// Row `y` of the composite in 16-bit levels.
fn row16(d: &Document, y: i32) -> Vec<i32> {
    render(d, Rect::new(0, y, 200, y + 1)).px.iter().map(|p| (p[0] * 65535.0).round() as i32).collect()
}

fn max_diff(a: &[i32], b: &[i32]) -> i32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).max().unwrap_or(0)
}

/// A 10 px feather is a symmetric ramp centred on the path, as Photoshop draws it.
#[test]
fn feather_matches_photoshops_ramp() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let r = row(&shape_doc(depth, 1.0, 10.0, None), 100);
        let ours: Vec<i32> = (0..15).map(|i| r[30 + 3 * i]).collect();
        let dev = ours.iter().zip(PS_FEATHER_10).map(|(a, b)| (a - i32::from(b)).abs()).max().unwrap_or(0);
        // Same Gaussian (sigma = feather) as a vector mask: within 1 level of Photoshop (as
        // scorecard TOOL-214-12 claims; measured max deviation 1 at every depth).
        assert!(dev <= 1, "{depth:?}: ours {ours:?} vs Photoshop {PS_FEATHER_10:?} (max {dev})");
        eprintln!("{depth:?}: ours {ours:?}, Photoshop {PS_FEATHER_10:?}, max deviation {dev}");
        // The path runs between pixels 49 and 50: their mean is half way.
        assert!((r[49] + r[50] - 255).abs() <= 2, "{depth:?}: half way on the path: {} {}", r[49], r[50]);
        assert_eq!(r[100], 0, "{depth:?}: the centre stays solid");
        // Symmetric about the square's centre.
        assert!((0..200).all(|x| (r[x] - r[199 - x]).abs() <= 1), "{depth:?}: {r:?}");
    }
}

/// Density 50 %: the fill shows at half strength beyond the path, full inside, hard edge.
#[test]
fn density_shows_the_fill_beyond_the_path() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let r = row(&shape_doc(depth, 0.5, 0.0, None), 100);
        for x in [0, 20, 49, 150, 199] {
            assert!((127..=128).contains(&r[x]), "{depth:?}: x={x} outside the path: {}", r[x]);
        }
        for x in [50, 100, 149] {
            assert_eq!(r[x], 0, "{depth:?}: x={x} inside the path");
        }
        let top = row(&shape_doc(depth, 0.5, 0.0, None), 0);
        assert!(top.iter().all(|v| (127..=128).contains(v)), "{depth:?}: everywhere beyond the path");
    }
}

/// A shape and a fill layer with an equal vector mask render the same pixels. (Density 50 % puts
/// white × 0.5 = 127.5 exactly on a rounding tie, which the shape's stored pixels and the fill's
/// mask break differently: 127 and 128, both what Photoshop's "127/128" allows.)
#[test]
fn soft_shape_matches_an_equal_vector_mask() {
    // 40 px reaches past the canvas edge: both feather it the same way there (#9475).
    for (density, feather) in [(1.0, 10.0), (0.45, 0.0), (0.3, 6.5), (0.0, 0.0), (1.0, 0.0), (0.5, 3.0), (1.0, 40.0)] {
        for depth in [SampleType::U8, SampleType::U16] {
            let (s, f) = (shape_doc(depth, density, feather, None), fill_doc(depth, density, feather));
            for y in [0, 45, 50, 100, 151, 199] {
                // An 8-bit shape keeps its rendering in 8-bit pixels (the fill layer's mask is
                // 16-bit), which can round its alpha one level apart, as at any anti-aliased edge.
                let d8 = max_diff(&row(&s, y), &row(&f, y));
                assert!(d8 <= i32::from(depth == SampleType::U8), "{depth:?} density {density} feather {feather}, row {y}: {d8} levels");
                // The compositor keeps masks at 16 bits (an 8-bit shape's pixels are 8-bit).
                if depth == SampleType::U16 {
                    let d16 = max_diff(&row16(&s, y), &row16(&f, y));
                    assert!(d16 <= 2, "{depth:?} density {density} feather {feather}, row {y}: {d16} 16-bit levels");
                }
            }
        }
    }
}

/// Density and feather soften the fill only: the vector stroke keeps its crisp edge.
#[test]
fn soft_shape_keeps_its_stroke_crisp() {
    let stroke = ShapeStroke { width: 4.0, paint: Fill::Solid(Color::rgb(1.0, 0.0, 0.0)), ..ShapeStroke::default() };
    let d = shape_doc(SampleType::U8, 0.5, 10.0, Some(stroke));
    let px = render(&d, Rect::new(0, 100, 200, 101)).px;
    // Centred 4 px stroke over x = 48..52: opaque red.
    for x in [48, 49, 50, 51] {
        let p = px[x];
        assert!(p[0] > 0.99 && p[1] < 0.01 && p[2] < 0.01, "x={x}: {p:?}");
    }
    // Beyond the stroke the feathered fill continues: at 10 px out, 50 % density plus half of
    // the feather's 16 % ≈ 58 % black.
    assert!((px[40][1] - 0.42).abs() < 0.03, "{:?}", px[40]);
}

/// Defaults (density 100 %, no feather) leave a shape exactly as before.
#[test]
fn default_shape_is_unchanged() {
    let d = shape_doc(SampleType::U8, 1.0, 0.0, None);
    let r = row(&d, 100);
    assert_eq!(r[49], 255);
    assert_eq!(r[50], 0);
    assert_eq!(r[149], 0);
    assert_eq!(r[150], 255);
    assert!(row(&d, 10).iter().all(|v| *v == 255));
}

/// File-provided values outside the range never panic and render as the nearest valid ones.
#[test]
fn hostile_values_render() {
    for (density, feather) in [(f32::NAN, f32::NAN), (-1.0, -5.0), (7.0, 1.0e9), (f32::INFINITY, f32::NEG_INFINITY)] {
        let d = shape_doc(SampleType::U8, density, feather, None);
        let r = row(&d, 100);
        assert_eq!(r.len(), 200, "{density} {feather}");
    }
}

/// The layer's bounds reach as far as its soft fill.
#[test]
fn soft_shape_bounds_cover_the_fill() {
    let canvas = Rect::new(0, 0, 200, 200);
    let feathered = shape_doc(SampleType::U8, 1.0, 10.0, None);
    let b = layer_bounds(&feathered.layers[1], canvas);
    assert!(b.x0 <= 50 - 30 && b.x1 >= 150 + 30, "feather reach: {b:?}");
    let dense = shape_doc(SampleType::U8, 0.5, 0.0, None);
    assert_eq!(layer_bounds(&dense.layers[1], canvas), canvas);
    assert!(effect_outline(&dense.layers[1]).is_none(), "a soft fill's alpha is its outline");
}

/// A soft shape's frame is the hard shape's painted pixels, also where a subtracted subpath
/// leaves most of the rasterizers' bounds empty (#5846), at every depth.
#[test]
fn soft_shape_frame_skips_a_subtracted_margin() {
    let canvas = Rect::new(0, 0, 200, 200);
    let mut cut = Subpath::polygon(&[(40.0, 40.0), (160.0, 40.0), (160.0, 140.0), (40.0, 140.0)]);
    cut.op = photocraft_doc::vector::PathOp::Subtract;
    let path = Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (150.0, 50.0), (150.0, 150.0), (50.0, 150.0)]), cut]);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let sh = ShapeLayer { path: path.clone(), fill: Some(Fill::Solid(Color::BLACK)), density: 0.5, feather: 4.0, ..ShapeLayer::default() };
        assert_eq!(soft_shape_frame(&sh, doc(depth).pixel_format(), canvas), Some(Rect::new(50, 140, 150, 150)), "{depth:?}");
    }
}

/// The frame scan renders a few strips per edge, not one per empty row or column: a 2-px
/// painted spot in a 4000 × 3000 bounds takes a handful of renders and is found exactly.
#[test]
fn soft_shape_frame_scan_is_bounded() {
    let spot = Rect::new(2500, 1700, 2502, 1702);
    let mut calls = 0;
    let got = trim_to_painted(Rect::new(0, 0, 4000, 3000), |strip| {
        calls += 1;
        strip.intersect(&spot)
    });
    assert_eq!(got, Some(spot));
    assert!(calls <= 4 * 14, "{calls} renders");
    let mut calls = 0;
    assert_eq!(
        trim_to_painted(Rect::new(0, 0, 4000, 3000), |_| {
            calls += 1;
            Rect::EMPTY
        }),
        None
    );
    assert!(calls <= 14, "an empty shape stops after the top edge: {calls} renders");
    assert_eq!(trim_to_painted(Rect::EMPTY, |r| r), None);
    assert_eq!(trim_to_painted(Rect::new(3, 4, 9, 7), |r| r), Some(Rect::new(3, 4, 9, 7)), "a full rect keeps its edges");
}
