//! #10868: crop fill samples the Background and editable active raster independently.

use photocraft_doc::{Document, Layer, LayerContent, LayerId, TextLayer};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::{Value, json};

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

struct Layers {
    background: LayerId,
    quads: LayerId,
    small: LayerId,
    edge: LayerId,
    hidden: LayerId,
    text: LayerId,
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn raster(d: &Document, name: &str, bounds: Rect, color: [f32; 4]) -> Layer {
    let fmt = d.pixel_format();
    let mut layer = Layer::raster(name, fmt);
    layer.surface_mut().unwrap().fill_rect(bounds, &photocraft_raster::from_rgba(&fmt, color));
    layer
}

/// Keeps the measured straight geometry and fits turned controls under the 1 MiB transform limit
/// that the preference test can set process-wide.
fn scene_divisor(turned: bool) -> i32 {
    if turned { 2 } else { 1 }
}

/// The measured scene, with a deliberately different crop extension colour.
fn scene(turned: bool) -> (Session, Layers) {
    let divisor = scene_divisor(turned);
    let rect = |x0, y0, x1, y1| Rect::new(x0 / divisor, y0 / divisor, x1 / divisor, y1 / divisor);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":800/divisor,"height":500/divisor,"depth":8,"background":"white"})).unwrap();
    s.execute("tools.setColors", json!({"background":"#ff00ff"})).unwrap();
    let ids = s
        .edit("Fixture", |d, _| {
            let background = d.background().unwrap().id;
            let mut quads = raster(d, "Quads", rect(0, 0, 800, 500), RED);
            let fmt = d.pixel_format();
            let surf = quads.surface_mut().unwrap();
            for (bounds, color) in [(rect(400, 0, 800, 250), GREEN), (rect(0, 250, 400, 500), BLUE), (rect(400, 250, 800, 500), YELLOW)] {
                surf.fill_rect(bounds, &photocraft_raster::from_rgba(&fmt, color));
            }
            let small = raster(d, "Small", rect(350, 200, 450, 300), RED);
            let edge = raster(d, "EdgeHalf", rect(600, 0, 800, 250), BLUE);
            let mut hidden = raster(d, "Hidden", rect(0, 0, 800, 500), RED);
            hidden.visible = false;
            let text = Layer::new("T", LayerContent::Text(TextLayer { text: "T".into(), ..Default::default() }));
            let ids = Layers { background, quads: quads.id, small: small.id, edge: edge.id, hidden: hidden.id, text: text.id };
            d.layers.extend([quads, small, edge, hidden, text]);
            Ok(ids)
        })
        .unwrap();
    (s, ids)
}

fn select(s: &mut Session, id: LayerId) {
    s.execute("layer.select", json!({"layer":id.0})).unwrap();
}

/// Extends the frame to the right by one eighth of the original width.
fn frame(turned: bool) -> Value {
    let divisor = scene_divisor(turned);
    json!({"x":0,"y":0,"width":900/divisor,"height":500/divisor,"angle":if turned { 10 } else { 0 },"deleteCroppedPixels":true})
}

/// Samples the same relative point inside the new strip in both fixture sizes.
fn extension_sample(turned: bool) -> (i32, i32) {
    let divisor = scene_divisor(turned);
    (850 / divisor, 400 / divisor)
}

fn crop(s: &mut Session, mut p: Value, aware: bool) {
    p["contentAware"] = json!(aware);
    s.execute("image.crop", p).unwrap();
}

fn plain_control(s: &mut Session, p: &Value) -> Document {
    crop(s, p.clone(), false);
    let plain = doc(s).clone();
    s.execute("edit.undo", json!({})).unwrap();
    plain
}

/// Checks donor colour with room for the resampling and synthesis tolerance.
fn pixel(d: &Document, id: LayerId, x: i32, y: i32, want: [f32; 4]) {
    let actual = d.layer(id).unwrap().surface().unwrap().rgba(x, y);
    assert!(actual.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.06), "layer {id:?} at ({x},{y}): {actual:?}, expected {want:?}");
}

/// Compares complete layers with plain crop, including any legitimate turn or clipping.
fn unchanged(d: &Document, plain: &Document, ids: &[LayerId]) {
    for id in ids {
        // Includes bounds, pixels and metadata; turned crops legitimately transform these layers.
        assert_eq!(d.layer(*id), plain.layer(*id), "non-target {id:?} differs from plain crop");
    }
}

/// Every pixel of `id` inside `area` (the old canvas) is the plain crop's.
fn covered_as_plain(d: &Document, plain: &Document, id: LayerId, area: Rect) {
    let (got, want) = (d.layer(id).unwrap().surface().unwrap(), plain.layer(id).unwrap().surface().unwrap());
    for y in (area.y0..area.y1).step_by(7) {
        for x in (area.x0..area.x1).step_by(7) {
            assert_eq!(got.rgba(x, y), want.rgba(x, y), "layer {id:?} at ({x},{y}) differs from the plain crop");
        }
    }
}

fn named_one_step(s: &Session, past: usize, name: &str) {
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), past + 1);
    assert_eq!(st.history.undo_label(), Some(name));
}

fn undo_redo(s: &mut Session, before: &Document) {
    let after = doc(s).clone();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(s), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(doc(s), &after);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Content-aware Crop"));
}

/// Matches the three measured rows at their original size, bounds and sample points.
#[test]
fn measured_straight_rows_fill_background_and_only_the_active_raster() {
    for row in 0..3 {
        let (mut s, ids) = scene(false);
        let active = if row == 1 { ids.edge } else { ids.quads };
        if row == 2 {
            s.edit("Layer 0", |d, _| {
                let bg = d.layer_mut(ids.background).unwrap();
                bg.name = "Layer 0".into();
                bg.locks = Default::default();
                Ok(())
            })
            .unwrap();
        }
        select(&mut s, active);
        let before = doc(&s).clone();
        let p = frame(false);
        let plain = plain_control(&mut s, &p);
        let past = s.active().unwrap().history.past_len();
        crop(&mut s, p, true);
        let d = doc(&s);
        assert_eq!((d.size.width, d.size.height), (900, 500));
        let mut untouched = vec![ids.small, ids.hidden, ids.text];
        untouched.push(if row == 1 { ids.quads } else { ids.edge });
        if row == 2 {
            assert!(d.background().is_none());
            untouched.push(ids.background);
        } else {
            assert_eq!(d.layer(ids.background).unwrap().surface().unwrap().content_bounds(), Rect::new(0, 0, 900, 500));
            pixel(d, ids.background, 850, 400, WHITE);
        }
        let bounds = if row == 1 { Rect::new(600, 0, 900, 500) } else { Rect::new(0, 0, 900, 500) };
        assert_eq!(d.layer(active).unwrap().surface().unwrap().content_bounds(), bounds);
        pixel(d, active, 850, if row == 1 { 400 } else { 125 }, if row == 1 { BLUE } else { GREEN });
        // The fill adds pixels only where the crop uncovered the canvas: the active layer's own
        // pixels inside the old canvas are the plain crop's.
        covered_as_plain(d, &plain, active, Rect::new(0, 0, 800, 500));
        unchanged(d, &plain, &untouched);
        named_one_step(&s, past, "Content-aware Crop");
        undo_redo(&mut s, &before);
    }
}

/// Checks own-layer donors and exact non-target parity after rotation.
#[test]
fn turned_rows_sample_each_target_and_leave_other_layers_at_plain_crop() {
    let (x, y) = extension_sample(true);
    for row in 0..3 {
        let (mut s, ids) = scene(true);
        // Uniform donors isolate target selection from the existing turned-fill sampling policy.
        s.edit("Uniform Quads", |d, _| {
            let fmt = d.pixel_format();
            let bounds = d.bounds();
            d.layer_mut(ids.quads).unwrap().surface_mut().unwrap().fill_rect(bounds, &photocraft_raster::from_rgba(&fmt, GREEN));
            if row == 2 {
                d.layer_mut(ids.background).unwrap().locks = Default::default();
            }
            Ok(())
        })
        .unwrap();
        let active = if row == 1 { ids.edge } else { ids.quads };
        select(&mut s, active);
        let before = doc(&s).clone();
        let p = frame(true);
        let plain = plain_control(&mut s, &p);
        let past = s.active().unwrap().history.past_len();
        let mut p = p;
        p["contentAware"] = json!(true);
        s.execute("image.crop", p).unwrap_or_else(|error| panic!("turned row {row}, active {active:?}: {error}"));
        let d = doc(&s);
        pixel(d, active, x, y, if row == 1 { BLUE } else { GREEN });
        assert!(d.layer(active).unwrap().surface().unwrap().content_bounds().contains(x, y));
        let mut untouched = vec![ids.small, ids.hidden, ids.text, if row == 1 { ids.quads } else { ids.edge }];
        if row == 2 {
            untouched.push(ids.background);
        } else {
            pixel(d, ids.background, x, y, WHITE);
        }
        unchanged(d, &plain, &untouched);
        named_one_step(&s, past, "Content-aware Crop");
        undo_redo(&mut s, &before);
    }
}

/// Selecting the Background leaves each other layer identical to its plain crop.
#[test]
fn background_active_is_not_a_second_target() {
    for turned in [false, true] {
        let (x, y) = extension_sample(turned);
        let (mut s, ids) = scene(turned);
        select(&mut s, ids.background);
        let p = frame(turned);
        let plain = plain_control(&mut s, &p);
        let past = s.active().unwrap().history.past_len();
        crop(&mut s, p, true);
        pixel(doc(&s), ids.background, x, y, WHITE);
        unchanged(doc(&s), &plain, &[ids.quads, ids.small, ids.edge, ids.hidden, ids.text]);
        named_one_step(&s, past, "Content-aware Crop");
    }
}

/// Verifies Background-only fill for ineligible active layers and Delete Cropped Pixels off.
#[test]
fn unmeasured_active_targets_and_delete_off_keep_the_legacy_target() {
    for turned in [false, true] {
        let (x, y) = extension_sample(turned);
        for case in 0..10 {
            let (mut s, ids) = scene(turned);
            let active = match case {
                0 => ids.text,
                1 => ids.hidden,
                _ => ids.quads,
            };
            if (2..7).contains(&case) {
                s.edit("Lock active", |d, _| {
                    assert!(d.layer_mut(active).unwrap().locks.set(["all", "pixels", "transparency", "position", "artboard"][case - 2], true));
                    Ok(())
                })
                .unwrap();
            }
            if case >= 8 {
                s.edit("Group active", |d, _| {
                    let i = d.layers.iter().position(|layer| layer.id == active).unwrap();
                    let child = d.layers.remove(i);
                    let mut group = Layer::group("Ancestor", vec![child]);
                    if case == 8 {
                        group.visible = false;
                    } else {
                        assert!(group.locks.set("pixels", true));
                    }
                    d.layers.insert(i, group);
                    Ok(())
                })
                .unwrap();
            }
            select(&mut s, active);
            let mut p = frame(turned);
            if case == 7 {
                p["deleteCroppedPixels"] = json!(false);
            }
            let plain = plain_control(&mut s, &p);
            crop(&mut s, p, true);
            pixel(doc(&s), ids.background, x, y, WHITE);
            unchanged(doc(&s), &plain, &[ids.quads, ids.small, ids.edge, ids.hidden, ids.text]);
        }
    }
}

/// Checks the plain history name for interior crops and crops with synthesis disabled.
#[test]
fn no_fill_or_content_aware_off_keeps_the_crop_history_name() {
    for (p, aware, turned) in [
        (json!({"x":20,"y":20,"width":40,"height":30}), true, false),
        (json!({"x":20,"y":20,"width":40,"height":30,"angle":10}), true, true),
        (frame(false), false, false),
        (frame(true), false, true),
    ] {
        let (mut s, ids) = scene(turned);
        select(&mut s, ids.quads);
        let past = s.active().unwrap().history.past_len();
        crop(&mut s, p, aware);
        named_one_step(&s, past, "Crop");
    }
}

/// Gives exactly one target a strip of held pixels outside the original canvas.
fn overhang_scene(held_active: bool) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":48,"background":"white"})).unwrap();
    s.execute("tools.setColors", json!({"background":"#ff00ff"})).unwrap();
    let (bg, active) = s
        .edit("Distinct hidden pixels", |d, active| {
            let bg = d.background().unwrap().id;
            let mut layer = raster(d, "Active", Rect::new(0, 0, 64, 48), GREEN);
            let fmt = d.pixel_format();
            let surf = if held_active { layer.surface_mut().unwrap() } else { d.layer_mut(bg).unwrap().surface_mut().unwrap() };
            surf.fill_rect(Rect::new(-16, 0, 0, 48), &photocraft_raster::from_rgba(&fmt, if held_active { BLUE } else { RED }));
            let id = layer.id;
            d.layers.push(layer);
            *active = Some(id);
            Ok((bg, id))
        })
        .unwrap();
    (s, bg, active)
}

/// Confirms that held pixels protect only the layer that owned them before the turn.
#[test]
fn turned_hidden_overhang_coverage_belongs_to_each_layer() {
    for held_active in [false, true] {
        let (mut s, bg, active) = overhang_scene(held_active);
        crop(&mut s, json!({"x":-10,"y":14,"width":20,"height":20,"angle":45}), true);
        let mut held_pixels = 0;
        for y in 0..20 {
            for x in 0..20 {
                let (held, other, want) = if held_active { (active, bg, WHITE) } else { (bg, active, GREEN) };
                pixel(doc(&s), other, x, y, want);
                let px = doc(&s).layer(held).unwrap().surface().unwrap().rgba(x, y);
                let color = if held_active { BLUE } else { RED };
                held_pixels += usize::from(px.iter().zip(color).all(|(a, b)| (a - b).abs() < 0.06));
            }
        }
        assert!(held_pixels >= 100, "the target's own hidden pixels survive: {held_pixels}");
    }
}

/// An empty active raster leaves the successful Background fill and undo step intact.
#[test]
fn donorless_added_target_keeps_the_background_fill() {
    for turned in [false, true] {
        let (x, y) = extension_sample(turned);
        let (mut s, ids) = scene(turned);
        let empty = s
            .edit("Empty target", |d, active| {
                let layer = Layer::raster("Empty", d.pixel_format());
                let id = layer.id;
                d.layers.push(layer);
                *active = Some(id);
                Ok(id)
            })
            .unwrap();
        assert_ne!(empty, ids.background);
        let before = doc(&s).clone();
        let p = frame(turned);
        let plain = plain_control(&mut s, &p);
        let past = s.active().unwrap().history.past_len();
        crop(&mut s, p, true);
        pixel(doc(&s), ids.background, x, y, WHITE);
        unchanged(doc(&s), &plain, &[empty, ids.quads, ids.small, ids.edge, ids.hidden, ids.text]);
        assert_eq!(s.active().unwrap().active_layer, Some(empty));
        named_one_step(&s, past, "Content-aware Crop");
        undo_redo(&mut s, &before);
    }
}

/// Confirms both layers synthesize their own colours at every RGB depth.
#[test]
fn both_target_colours_remain_native_at_each_rgb_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":32,"height":24,"depth":depth,"background":"white"})).unwrap();
        let (bg, active) = s
            .edit("Own colours", |d, active| {
                let bg = d.background().unwrap().id;
                let layer = raster(d, "Green", Rect::new(0, 0, 32, 24), GREEN);
                let id = layer.id;
                d.layers.push(layer);
                *active = Some(id);
                Ok((bg, id))
            })
            .unwrap();
        crop(&mut s, json!({"x":0,"y":0,"width":40,"height":24}), true);
        pixel(doc(&s), bg, 36, 12, WHITE);
        pixel(doc(&s), active, 36, 12, GREEN);
    }
}
