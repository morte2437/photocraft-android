//! Flatten Image, Stamp Visible, Background from Layer, Merge Down and Merge Visible keep the
//! inks of a CMYK document tagged with a profile other than the built-in one (#3778).
//!
//! A document whose composite is one plain pixel layer keeps its inks exactly, as in Photoshop
//! (black generation and inks outside the display gamut included). Merges of several layers are
//! composited in RGB and separated back through the profile the layers were read with, so they
//! keep the inks up to that round trip's residual instead of moving every pixel.

use std::sync::Arc;

use photocraft_color::convert::{CmykSpace, with_cmyk_space};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

const W: i32 = 8;
const H: i32 = 2;

/// RGB colours whose separations make the first row: mid tones, saturated colours, near-black.
const RGB: [[f32; 3]; 8] =
    [[0.8, 0.3, 0.2], [0.1, 0.5, 0.9], [0.5, 0.5, 0.5], [0.08, 0.06, 0.05], [0.95, 0.9, 0.85], [0.9, 0.8, 0.1], [0.2, 0.7, 0.3], [0.6, 0.2, 0.6]];

/// Inks no separation produces, for the second row: K only, rich black, solid cyan (outside
/// sRGB), paper, 50 % K, a heavy four-colour mix, and two odd mixes.
const INKS: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.0, 1.0],
    [0.75, 0.68, 0.67, 0.9],
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.5],
    [0.9, 0.9, 0.9, 0.9],
    [0.886, 0.329, 0.0, 0.0],
    [0.1, 0.0, 0.6, 0.3],
];

fn profile() -> Arc<Vec<u8>> {
    photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
        description: "Test Uncoated".into(),
        tvi: [0.26, 0.26, 0.26, 0.3],
        ..Default::default()
    })
    .to_bytes()
}

/// A one-layer (Background) CMYK document, tagged with `profile()` when `tag`: the first row is
/// separated from [`RGB`] through that profile (a separation read back and separated again a few
/// times, so the profile reproduces it), the second row holds [`INKS`].
fn document(depth: u32, tag: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "mode": "cmyk", "depth": depth, "background": "white"})).unwrap();
    let icc = profile();
    let space = CmykSpace::for_profile(Some(&icc));
    assert!(space.is_some(), "the fixture is not the built-in profile");
    s.edit("tag", |doc, _| {
        if tag {
            doc.icc_profile = Some(icc.clone());
        }
        let bg = doc.layers[0].surface_mut().unwrap();
        let fmt = bg.format();
        with_cmyk_space(if tag { space.as_ref() } else { None }, || {
            for (x, rgb) in RGB.iter().enumerate() {
                let mut px = photocraft_raster::from_rgba(&fmt, [rgb[0], rgb[1], rgb[2], 1.0]);
                for _ in 0..3 {
                    let back = photocraft_raster::to_rgba(&fmt, &px);
                    px = photocraft_raster::from_rgba(&fmt, back);
                }
                bg.write_pixel(x as i32, 0, &px);
            }
        });
        for (x, ink) in INKS.iter().enumerate() {
            bg.write_pixel(x as i32, 1, &[ink[0], ink[1], ink[2], ink[3], 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s
}

/// The four ink values of every pixel of the active layer.
fn inks(s: &Session) -> Vec<[f32; 4]> {
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let n = surf.channels();
    surf.read_region(Rect::new(0, 0, W, H)).chunks_exact(n).map(|p| [p[0], p[1], p[2], p[3]]).collect()
}

fn max_diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter().zip(b).flat_map(|(p, q)| (0..4).map(move |c| (p[c] - q[c]).abs())).fold(0.0, f32::max)
}

/// Runs `run` on a document and returns how far the inks of `row` (0 = separations, 1 = any
/// inks) moved.
fn drift(depth: u32, tag: bool, row: usize, run: impl FnOnce(&mut Session)) -> f32 {
    let mut s = document(depth, tag);
    let before = inks(&s);
    run(&mut s);
    let after = inks(&s);
    let r = row * W as usize..(row + 1) * W as usize;
    max_diff(&before[r.clone()], &after[r])
}

fn exact(what: &str, run: impl Fn(&mut Session)) {
    for depth in [8, 16] {
        for tag in [true, false] {
            for row in [0, 1] {
                let d = drift(depth, tag, row, &run);
                assert!(d == 0.0, "{depth}-bit {what} (tagged: {tag}, row {row}): inks moved by {d}");
            }
        }
    }
}

#[test]
fn flatten_keeps_the_inks_of_a_single_layer_exactly() {
    exact("Flatten Image", |s| {
        s.execute("layer.flattenImage", json!({})).unwrap();
    });
}

#[test]
fn stamp_visible_keeps_the_inks_of_a_single_layer_exactly() {
    exact("Stamp Visible", |s| {
        s.execute("layer.stampVisible", json!({})).unwrap();
    });
}

#[test]
fn background_from_layer_keeps_the_inks_exactly() {
    exact("Background from Layer", |s| {
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        s.execute("layer.new.backgroundFromLayer", json!({})).unwrap();
    });
}

#[test]
fn flatten_of_a_partly_transparent_layer_still_composites_over_white() {
    for depth in [8, 16] {
        let mut s = document(depth, true);
        s.edit("hole", |doc, _| {
            doc.layers[0].surface_mut().unwrap().write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0, 0.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.flattenImage", json!({})).unwrap();
        let px = inks(&s)[0];
        // Transparent over white paper: no ink, not the hidden K.
        assert!(px.iter().all(|v| *v < 0.02), "{depth}-bit: {px:?}");
    }
}

/// Merges of several layers go through the RGB composite: separations the profile reproduces
/// come back within this (the profile's own RGB round trip, a few levels), where reading with
/// the document's profile and separating with the built-in one moved them by tens of levels.
const ROUND_TRIP: f32 = 10.0 / 255.0;

#[test]
fn merge_down_separates_through_the_document_profile() {
    for depth in [8, 16] {
        let d = drift(depth, true, 0, |s| {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("layer.mergeDown", json!({})).unwrap();
        });
        assert!(d <= ROUND_TRIP, "{depth}-bit Merge Down: inks moved by {d}");
    }
}

#[test]
fn merge_visible_separates_through_the_document_profile() {
    for depth in [8, 16] {
        let d = drift(depth, true, 0, |s| {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("layer.mergeVisible", json!({})).unwrap();
        });
        assert!(d <= ROUND_TRIP, "{depth}-bit Merge Visible: inks moved by {d}");
    }
}

/// Rasterize Layer Style bakes the layer through the RGB composite and separates the result
/// back: through the document's profile, the one the layer was read with (#3778), so the inks
/// under the effect-free area come back within the profile's round trip. The drop shadow falls
/// off the canvas, so every pixel checked is the layer's own.
#[test]
fn rasterize_layer_style_separates_through_the_document_profile() {
    for depth in [8, 16] {
        let d = drift(depth, true, 0, |s| {
            s.execute("layer.new.layerFromBackground", json!({})).unwrap();
            s.execute("layer.layerStyle.dropShadow", json!({"distance": 40, "size": 0})).unwrap();
            s.execute("layer.rasterize.layerStyle", json!({})).unwrap();
        });
        assert!(d <= ROUND_TRIP, "{depth}-bit Rasterize Layer Style: inks moved by {d}");
    }
}
