//! Commands that expand a layer into dense buffers over its content bounds (or the canvas) refuse,
//! before allocating, when those buffers would pass the working budget: a few distant tiles in a
//! small document, or a huge empty canvas, would otherwise ask for terabytes.

use std::sync::Arc;

use photocraft_doc::{LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

use crate::Session;

/// Two 10 px blocks two billion pixels apart: two stored tiles, trillions of pixels of bounds.
fn distant_blocks(s: &Surface) -> Surface {
    let fmt = s.format();
    let mut out = Surface::with_default(fmt, &s.default_pixel());
    out.fill_rect(Rect::new(0, 0, 10, 10), &vec![1.0; fmt.channels()]);
    out.fill_rect(Rect::new(2_000_000_000, 2_000_000_000, 2_000_000_010, 2_000_000_010), &vec![1.0; fmt.channels()]);
    out
}

/// A small document whose active layer holds [`distant_blocks`].
fn sparse_session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
    *surf = distant_blocks(surf);
    (s, id)
}

fn refused(s: &mut Session, cmd: &str, p: serde_json::Value) {
    let before = s.active().unwrap().doc.clone();
    let e = s.execute(cmd, p).unwrap_err().to_string();
    assert!(e.contains("working memory"), "{cmd}: {e}");
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "{cmd} left the document as it was");
}

#[test]
fn apply_layer_mask_over_distant_tiles_is_refused() {
    let (mut s, id) = sparse_session();
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().mask = Some(LayerMask::reveal_all());
    refused(&mut s, "layer.layerMask.apply", json!({"layer": id.0}));
}

#[test]
fn shadows_highlights_over_distant_tiles_is_refused() {
    let (mut s, id) = sparse_session();
    refused(&mut s, "image.adjustments.shadowsHighlights", json!({"layer": id.0}));
}

#[test]
fn image_size_without_distant_tiles_still_resamples() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    s.execute("image.imageSize", json!({"width": 128, "height": 128})).unwrap();
    assert_eq!(s.active().unwrap().doc.size.width, 128);
}

#[test]
fn face_detection_on_a_huge_empty_canvas_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300_000, "height": 300_000, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let e = s.execute("image.detectFaces", json!({"sampleAllLayers": false})).unwrap_err().to_string();
    assert!(e.contains("working memory"), "{e}");
}

#[test]
fn multichannel_on_a_huge_empty_canvas_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300_000, "height": 300_000, "background": "transparent"})).unwrap();
    refused(&mut s, "image.mode.multichannel", json!({}));
}

/// A quadtone keeps an f32 density and a stored plane for each of its four inks beside the
/// flattened image: 52 B per pixel, past the budget at 15,000 × 15,000 px where one colour
/// channel's 28 B would have passed.
#[test]
fn multichannel_counts_every_duotone_ink() {
    use photocraft_color::ColorMode;
    use photocraft_doc::{Duotone, DuotoneInk};
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 15_000, "height": 15_000, "background": "transparent"})).unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    doc.mode = ColorMode::Duotone;
    doc.duotone = Some(Duotone { inks: (0..4).map(|i| DuotoneInk::new(format!("ink {i}"), [0.0; 3])).collect(), psd_raw: None });
    refused(&mut s, "image.mode.multichannel", json!({}));
}
