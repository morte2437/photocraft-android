//! Smart objects warped with Photoshop's Cylinder (#4112) through PSD persistence and a re-render
//! from their contents, as Replace Contents, Relink and Edit Contents do.

use std::sync::Arc;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_geom::warp::{Warp, WarpStyle};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_io::{ExportOptions, export, import, smart_map};
use photocraft_psd::PsdFile;
use photocraft_raster::Surface;

/// Silhouettes at x 10 and 90, edges at y 20 and 80, ellipses 0.3 deep, wrapped as little as
/// Photoshop allows (a sixth of the contents on the back on each side).
const VALUES: [f64; 7] = [10.0, 80.0, 90.0, 20.0, 0.3, 0.3, 0.0];

/// A 100 × 100 source: a red band at the left and a blue band at the right (both on the back of
/// the cylinder), green in between.
fn contents() -> Surface {
    let mut s = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.fill_rect(Rect::new(0, 0, 100, 100), &[0.0, 1.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(0, 0, 5, 100), &[1.0, 0.0, 0.0, 1.0]);
    s.fill_rect(Rect::new(95, 0, 100, 100), &[0.0, 0.0, 1.0, 1.0]);
    s
}

fn document(transform: Affine, perspective: Option<[f64; 9]>) -> (Document, Warp) {
    let pixels = contents();
    let mut source = Document::new("label", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
    source.layers.push(Layer::new("art", LayerContent::Raster(pixels.clone())));
    let bytes = photocraft_format::save_to_bytes(&source, &Default::default()).unwrap();
    let warp = Warp::cylinder(&VALUES, [0.0, 0.0, 100.0, 100.0]).unwrap();
    let mut sm = SmartObject::new(SmartSource::Embedded { file_name: "label.pcraft".into(), bytes: Arc::new(bytes) }, transform, None);
    sm.warp = Some(warp.clone());
    sm.perspective = perspective;
    sm.cache = Some(render(&sm));
    let mut doc = Document::new("jar", Size::new(160, 160), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new("label", LayerContent::Smart(sm)));
    (doc, warp)
}

/// The smart object drawn from its contents, as the engine re-renders it.
fn render(sm: &SmartObject) -> Surface {
    let src = contents();
    match &sm.perspective {
        Some(p) => {
            photocraft_algo::warp::place_source_projective(&src, Rect::new(0, 0, 100, 100), &photocraft_algo::transform::Homography(*p), sm.warp.as_ref())
        }
        None => photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 100, 100), &sm.transform, sm.warp.as_ref()),
    }
    .unwrap()
}

fn smart(doc: &Document) -> &SmartObject {
    let LayerContent::Smart(sm) = &doc.layers[0].content else { panic!("not a smart object") };
    sm
}

fn round_trip(doc: &Document) -> (PsdFile, Document) {
    let out = export(doc, "psd", &ExportOptions::default()).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("warp") || w.contains("written as pixels")), "{:?}", out.warnings);
    (PsdFile::from_bytes(&out.bytes).unwrap(), import("psd", &out.bytes).unwrap().document)
}

/// Opaque pixels of the render that are mostly `channel` (0 red, 1 green, 2 blue).
fn count(s: &Surface, channel: usize) -> usize {
    let r = s.content_bounds();
    s.read_region(r).as_chunks::<4>().0.iter().filter(|p| p[3] > 0.5 && p[channel] > 0.5 && (0..3).all(|c| c == channel || p[c] < 0.5)).count()
}

/// Before #4112 a cylinder was read as no warp: re-rendering the contents drew them flat, the
/// red and blue bands that wrap round the back included. Now the PSD keeps the cylinder and the
/// re-render hides the back and centres the front between the silhouettes.
#[test]
fn a_cylinder_survives_psd_and_wraps_the_contents_again() {
    let transform = Affine { m: [1.0, 0.0, 0.0, 1.0, 30.0, 20.0] };
    let (doc, warp) = document(transform, None);
    let (file, back) = round_trip(&doc);
    let layer = file.layers().iter().find(|l| l.name() == "label").unwrap();
    let sold = smart_map::parse_sold(&layer.block(b"SoLd").unwrap().data).unwrap();
    let Some(photocraft_psd::descriptor::Value::Descriptor(w)) = sold.descriptor.get("warp") else { panic!("no warp") };
    assert!(matches!(w.get("warpStyle"), Some(photocraft_psd::descriptor::Value::Enumerated { value, .. }) if value.as_bytes() == b"warpCylinder"));
    let sm = smart(&back);
    assert_eq!(sm.warp.as_ref().map(|w| w.style), Some(WarpStyle::Cylinder));
    assert_eq!(sm.warp.as_ref(), Some(&warp));
    for (a, b) in sm.transform.m.iter().zip(transform.m) {
        assert!((a - b).abs() < 1e-9, "{:?}", sm.transform);
    }
    let out = render(sm);
    assert_eq!((count(&out, 0), count(&out, 2)), (0, 0), "the bands on the back are hidden");
    assert!(count(&out, 1) > 3000, "the front is drawn");
    // Between the silhouettes (x 10 and 90, shifted by 30) and no further.
    let b = out.content_bounds();
    assert!(b.x0 >= 39 && b.x1 <= 121, "{b:?}");
    // The middle of the contents is at the front, centred.
    let mid = out.pixel(80, 60);
    assert!(mid[1] > 0.9 && mid[3] > 0.9, "{mid:?}");
    // Saved again unchanged, the block is written as it was read.
    let (again, _) = round_trip(&back);
    let layer2 = again.layers().iter().find(|l| l.name() == "label").unwrap();
    assert_eq!(layer2.block(b"SoLd").unwrap().data, layer.block(b"SoLd").unwrap().data);
}

/// Mockups usually place the label in perspective (`nonAffineTransform`): that placement and
/// the cylinder both come back.
#[test]
fn a_cylinder_in_perspective_survives_psd() {
    let cb = Warp::cylinder(&VALUES, [0.0, 0.0, 100.0, 100.0]).unwrap().mesh.unwrap().control_bounds();
    let h = photocraft_algo::transform::Homography::rect_to_quad(cb, [[20.0, 20.0], [140.0, 30.0], [135.0, 150.0], [25.0, 140.0]]).unwrap();
    let (doc, warp) = document(Affine::IDENTITY, Some(h.0));
    let (_, back) = round_trip(&doc);
    let sm = smart(&back);
    assert_eq!(sm.warp.as_ref(), Some(&warp));
    let p = sm.perspective.expect("still projective");
    let n = |m: [f64; 9]| m.map(|v| v / m[8]);
    for (a, b) in n(p).iter().zip(n(h.0)) {
        assert!((a - b).abs() < 1e-9 * (1.0 + b.abs()), "{p:?} vs {:?}", h.0);
    }
    let out = render(sm);
    assert_eq!((count(&out, 0), count(&out, 2)), (0, 0));
    assert!(count(&out, 1) > 3000);
}
