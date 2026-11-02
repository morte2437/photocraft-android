//! #4281: Clip Thumbnails to Layer Bounds. A small object in a large document fills its Layers
//! panel thumbnail instead of being a speck in an empty canvas-sized one.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::mask_thumbs_ui::THUMB_LAYER;

fn harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 2000, "height": 1000})).unwrap();
    let id = s.execute("layer.new.layer", json!({"name": "Speck"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 200, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dockTabs": {"layers": 0}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, id)
}

fn frame(h: &Harness<'_, PhotocraftApp>, id: u64) -> Rect {
    *h.state().thumb_frames.get(&LayerId(id)).expect("layer thumbnail drawn")
}

fn texture_area(h: &Harness<'_, PhotocraftApp>, id: u64) -> usize {
    let (_, tex) = h.state().thumbs.get(&(LayerId(id), THUMB_LAYER)).expect("layer thumbnail drawn");
    let [w, h] = tex.size();
    w * h
}

#[test]
fn layer_thumbnails_frame_the_canvas_by_default() {
    let (h, id) = harness();
    assert!(!h.state().session.prefs().interface.clip_thumbnails_to_layer_bounds);
    assert_eq!(frame(&h, id), Rect::new(0, 0, 2000, 1000));
}

#[test]
fn clipped_layer_thumbnails_frame_the_layer_pixels() {
    let (mut h, id) = harness();
    let before = h.state().thumbs.get(&(LayerId(id), THUMB_LAYER)).map(|(r, _)| *r).unwrap();
    h.state_mut().run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": true})).unwrap();
    h.run_steps(4);
    assert_eq!(frame(&h, id), Rect::new(100, 200, 120, 210), "framed on the filled rect");
    let after = h.state().thumbs.get(&(LayerId(id), THUMB_LAYER)).map(|(r, _)| *r).unwrap();
    assert_ne!(before, after, "toggling the preference rebuilds the thumbnail");
    assert_eq!(texture_area(&h, id), 64 * 64, "still a 64 px square texture");

    h.state_mut().run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": false})).unwrap();
    h.run_steps(4);
    assert_eq!(frame(&h, id), Rect::new(0, 0, 2000, 1000), "and back to the canvas");
}

#[test]
fn empty_and_off_canvas_layers_keep_the_canvas_frame() {
    let canvas = Rect::new(0, 0, 50, 40);
    let raster = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let mut empty = photocraft_doc::Layer::new("Empty", photocraft_doc::LayerContent::Raster(raster));
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), canvas);
    empty.surface_mut().unwrap().write_pixel(-10, -10, &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), canvas, "pixels only off the canvas");
    empty.surface_mut().unwrap().write_pixel(45, 5, &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), Rect::new(0, 0, 46, 6), "the layer bounds, cut to the canvas");
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, false), canvas);
}
