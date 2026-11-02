use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

#[test]
fn dragging_a_layer_larger_than_the_canvas_redraws_what_comes_in_from_beyond_the_edge() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(-32, -32, 96, 96), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, m);
    let mut keys = Vec::new();
    for x in [42.0, 52.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        keys.push(super::display_doc(&mut app, 0).unwrap().1);
    }
    let st = app.session.active().unwrap();
    // From +10 to +20 the strip x 0..10 shows pixels that were off the canvas a frame ago.
    let r = super::damage(&app, st.doc.id, st.revision, keys[0], keys[1]).unwrap();
    assert_eq!(r, Rect::new(0, 0, 64, 64));
}

/// The Move tool shows how far it has moved the layer, ΔX/ΔY in whole pixels, as Photoshop
/// and Free Transform do (#5727): once the pointer moves, until the release.
#[test]
fn a_move_drag_reads_out_the_offset_it_will_commit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(16, 16, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    assert_eq!(super::offset_readout(&app), None, "no drag");
    // Pressed on the red square (16..32): with Auto-Select (on by default) an empty spot would
    // start the layer marquee instead of a move.
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 24.0, y: 24.0, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), None, "pressed, not moved yet");
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 34.4, y: 20.8, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), Some(["10 px".to_string(), "-3 px".to_string()]));
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 34.4, y: 20.8 }, m);
    assert_eq!(super::offset_readout(&app), None, "released");
}

/// A release past the ±10,000,000 px the preview and the readout show commits the offset they
/// showed, not the raw pointer distance.
#[test]
fn a_release_commits_the_offset_the_readout_caps_at() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    super::finish(&mut app, 2.5e7, -2.5e7);
    let st = app.session.active().unwrap();
    let s = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()).unwrap();
    assert!(s.pixel(16 + 10_000_000, 16 - 10_000_000)[3] > 0.5, "moved by the capped offset");
}

/// A shape reaching past the canvas edge renders cut at the canvas, but its path goes on: a
/// drag frame must redraw where the rest of it was, or the cut edge stays behind (#3946).
#[test]
fn dragging_a_shape_past_the_canvas_edge_leaves_no_trail() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    let id = app.session.execute("shape.create", json!({"kind": "rect", "rect": [40, 16, 50, 32], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let id = photocraft_doc::LayerId(id);
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 32.0, pressure: 1.0 }, m);
    let mut frames = Vec::new();
    for x in [40.0, 10.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        frames.push(super::display_doc(&mut app, 0).unwrap());
    }
    let st = app.session.active().unwrap();
    let r = super::damage(&app, st.doc.id, st.revision, frames[0].1, frames[1].1).unwrap();
    let alpha = |d: &photocraft_doc::Document, x, y| d.layer(id).and_then(|l| l.surface()).map_or(0.0, |s| s.pixel(x, y)[3]);
    for y in 0..64 {
        for x in 0..64 {
            if alpha(&frames[0].0, x, y) != alpha(&frames[1].0, x, y) {
                assert!(r.contains(x, y), "({x}, {y}) changed outside the redrawn {r:?}");
            }
        }
    }
}

/// Dragging with an unlinked mask targeted moves the mask alone, shown live and committed;
/// with the mask linked (or the pixels targeted) the layer moves.
#[test]
fn dragging_with_an_unlinked_mask_targeted_moves_the_mask_alone() {
    for (linked, mask_target) in [(false, true), (false, false), (true, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        app.session.execute("layer.layerMask.linked", json!({"linked": linked})).unwrap();
        app.ui.mask_target = mask_target;
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 16.0, pressure: 1.0 }, m);
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let id = app.session.active().unwrap().active_layer.unwrap();
        let layer_moved = |d: &photocraft_doc::Document| d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(28, 16)[3] > 0.5);
        let alone = !linked && mask_target;
        assert_eq!(layer_moved(&shown), !alone, "preview, linked {linked} mask target {mask_target}");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 16.0 }, m);
        let d = &app.session.active().unwrap().doc;
        assert_eq!(layer_moved(d), !alone, "commit, linked {linked} mask target {mask_target}");
    }
}

/// Mid-drag on a painted layer: is the layer shown at the pointer?
fn live(outline: bool) -> bool {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.edit_prefs(|p| p.interface.show_bounding_box_when_dragging_layer = outline);
    app.ui.tool = Tool::Move;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    let shown = super::display_doc(&mut app, 0).is_some();
    assert_eq!(shown, super::showing(&app));
    shown
}

/// A layer past the pixel-memory limit isn't copied for each frame of a drag (#6114, #6163).
#[test]
fn layers_over_the_limit_are_not_followed_live() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let one_tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * 4;
    assert!(super::exceeds(&st.doc, &[id], one_tile - 1, false), "a tile of pixels is over a limit below its size");
    assert!(super::exceeds(&st.doc, &[id], one_tile * 4 - 1, false), "a shift can write four tiles for each stored one");
    assert!(!super::exceeds(&st.doc, &[id], u64::MAX, false), "nothing is over an unlimited drag");
    assert!(!super::exceeds(&st.doc, &[], 0, false), "no layers hold no pixels");
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "an ordinary layer follows the pointer");
}

/// A drag whose layers are past the limit shows no live copy (the outline and arrow stay), and
/// the release still moves the layer once (#6114, #6163).
#[test]
fn a_drag_over_the_limit_shows_no_live_copy_and_still_commits() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    // Enough stored tiles, all sharing one allocation, to be past the limit before the drag starts.
    let step = photocraft_geom::TILE_SIZE;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile_bytes = u64::from(step.unsigned_abs()).pow(2) * surface.format().bytes_per_pixel() as u64;
            let count = (super::LIVE_LIMIT / (tile_bytes * 2) + 2) as i32; // a row writes about (n + 1) x 2 tiles
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..count).map(|x| (photocraft_geom::TileCoord::new(x, 0), tile.clone())));
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    let to = 16.0 + f64::from(step); // a whole tile, so the release shares the tiles too
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: to, y: 16.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "a layer over the limit isn't copied for the frame");
    assert!(!super::showing(&app));
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: to, y: 16.0 }, m);
    let d = &app.session.active().unwrap().doc;
    assert!(d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(to as i32, 16)[3] > 0.5), "the release moved the layer");
}

/// What a live drag frame writes is the tiles the content can reach, not four per stored tile for
/// every layer: a dense photo stays live, isolated tiles (each straddles four) don't (#6114).
#[test]
fn the_estimate_counts_reachable_target_tiles_and_what_moved_target_copies() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let step = photocraft_geom::TILE_SIZE;
    let tile_bytes = u64::from(step.unsigned_abs()).pow(2) * 4;
    let tiles_over_limit = (super::LIVE_LIMIT / tile_bytes + 1) as i32;
    // A dense block of the same number of stored tiles as the sparse one below.
    let side = (f64::from(tiles_over_limit / 4)).sqrt() as i32;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..side * side).map(|i| (photocraft_geom::TileCoord::new(i % side, i / side), tile.clone())));
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "a dense layer writes about as many tiles as it stores");
    // The same count of isolated tiles: each straddles four target tiles.
    let sparse = tiles_over_limit / 4 + 1;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..sparse).map(|i| (photocraft_geom::TileCoord::new(i * 10, i * 10 + 1000), tile.clone())));
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "isolated tiles each write up to four");
    // Only the mask is copied when an unlinked mask is dragged alone: the huge content isn't charged.
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[id], 1, false), "dragging the layer copies its content");
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT / 4, true), "dragging the unlinked mask alone copies the mask only");
}

/// A group moves its contents whole; a group's unlinked mask dragged alone leaves them where they are.
#[test]
fn the_estimate_follows_a_group_into_its_contents() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let inner = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..600).map(|i| (photocraft_geom::TileCoord::new(i * 10, i * 10), tile.clone())));
            Ok(())
        })
        .unwrap();
    let group = app.session.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    app.session.execute("layer.moveTo", json!({"layer": inner.0, "target": group, "position": "into"})).unwrap();
    let group = photocraft_doc::LayerId(group);
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[group], super::LIVE_LIMIT, false), "the group's contents are shifted with it");
    app.session.execute("layer.layerMask.revealAll", json!({"layer": group.0})).unwrap();
    app.session.execute("layer.layerMask.linked", json!({"layer": group.0, "linked": false})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(group).is_some_and(|l| l.mask.as_ref().is_some_and(|m| !m.linked)), "the group has an unlinked mask");
    assert!(!super::exceeds(&st.doc, &[group], super::LIVE_LIMIT, true), "an unlinked group mask dragged alone leaves the contents");
}

/// A `width` × `height` `depth`-bit document whose new layer covers the canvas, one fill per tile
/// with its own colour (a photo: no two tiles share pixels), and the Move tool on.
fn photo(width: u32, height: u32, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": width, "height": height, "depth": depth})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let canvas = Rect::new(0, 0, width as i32, height as i32);
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            for (i, c) in canvas.tiles().enumerate() {
                let v = (i % 251) as f32 / 251.0;
                surface.fill_rect(c.rect().intersect(&canvas), &[v, 1.0 - v, 0.5, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

/// Mid-drag by (37, 11): is the layer shown at the pointer?
fn follows_a_drag(app: &mut PhotocraftApp) -> bool {
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: 137.0, y: 111.0, pressure: 1.0 }, m);
    let shown = super::display_doc(app, 0).is_some();
    crate::canvas::tool_event(app, ToolEvent::Up { x: 137.0, y: 111.0 }, m);
    shown
}

/// Ordinary photos keep the live drag: a 36 MP 8-bit layer (580 stored tiles) and a 24 MP 16-bit
/// one write their tiles plus one edge row and column, not four tiles for each stored one.
#[test]
fn ordinary_photos_follow_the_pointer() {
    for (w, h, depth) in [(7360, 4912, 8), (6000, 4000, 16)] {
        let mut app = photo(w, h, depth);
        assert!(follows_a_drag(&mut app), "{w} × {h} {depth}-bit");
    }
}

/// A solid fill stores one tile shared by every covered position, and a shift shares it too (only
/// the edge is written), so a large filled layer drags live as it does without the limit.
#[test]
fn a_large_solid_layer_follows_the_pointer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 12000, "height": 12000, "depth": 16})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.all", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    assert!(follows_a_drag(&mut app), "a filled 12000 × 12000 16-bit layer");
}

/// Content off the tile grid: the 2 × 2 blocks round its stored tiles reach a column and a row
/// past what a shift writes, which is at most the tiles the content can reach.
#[test]
fn the_estimate_is_capped_by_what_the_content_reaches() {
    let format = photocraft_color::PixelFormat::RGBA8;
    let mut s = photocraft_raster::Surface::new(format);
    s.fill_rect(Rect::new(128, 128, 384, 384), &[1.0, 0.0, 0.0, 1.0]);
    let tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * format.bytes_per_pixel() as u64;
    assert_eq!(s.tile_count(), 4, "a tile-sized square straddles four tiles");
    assert_eq!(super::shift_bytes(&s), 4 * tile, "it reaches (1 + 1) × (1 + 1) target tiles, not the nine blocks round its tiles");
}

/// `shift_smart` shifts a smart object's filter mask with it: the estimate counts it.
#[test]
fn the_estimate_counts_a_smart_filter_mask() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(!super::exceeds(&app.session.active().unwrap().doc, &[id], super::LIVE_LIMIT, false), "a small smart object");
    app.session
        .edit("paint", |doc, a| {
            let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(a.unwrap()).unwrap().content else { panic!("not a smart object") };
            let mut mask = photocraft_doc::LayerMask::hide_all();
            // Isolated tiles, each written as four by a shift: past the limit at one byte a pixel.
            let tile = mask.surface.solid_tile(&[1.0]);
            let count = (super::LIVE_LIMIT / (1 << 16) / 4 + 1) as i32;
            mask.surface.put_tiles((0..count).map(|i| (photocraft_geom::TileCoord::new(i * 10, 0), tile.clone())));
            sm.filter_mask = Some(mask);
            Ok(())
        })
        .unwrap();
    assert!(super::exceeds(&app.session.active().unwrap().doc, &[id], super::LIVE_LIMIT, false), "the filter mask moves with it");
}

#[test]
fn bounding_box_preference_turns_the_live_drag_off() {
    assert!(live(false), "by default the layer follows the pointer");
    assert!(!live(true), "with the preference on, only the outline and arrow move");
}

/// A `depth`-bit transparent document whose layer holds a red square (8..24)² and a blue one
/// (40..56)×(8..24), the red one selected; the Move tool with Auto-Select on.
fn selected_pixels(depth: u32) -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent", "depth": depth})).unwrap();
    app.sync_views();
    for (x, color) in [(40, "#0000ff"), (8, "#ff0000")] {
        app.session.execute("select.rect", json!({"x": x, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": color})).unwrap();
    }
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let id = app.session.active().unwrap().active_layer.unwrap();
    (app, id)
}

fn alpha(app: &PhotocraftApp, id: photocraft_doc::LayerId, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// ↩ through the app's key handling.
fn enter(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let input = egui::RawInput { events: vec![key], ..Default::default() };
    ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx())).textures_delta.clear();
}

/// #578: with a selection the Move tool drags the selected pixels (from anywhere, without an
/// Auto-Select pick), not the layer; ↩ drops them as one "Move" step that one undo takes back.
#[test]
fn with_a_selection_the_move_tool_moves_only_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let steps = app.session.active().unwrap().history.past_len();
        drag(&mut app, [50.0, 40.0], [60.0, 45.0], egui::Modifiers::NONE);
        assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit: floating");
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let shown_alpha = |x, y| shown.layer(id).unwrap().surface().unwrap().rgba(x, y)[3];
        assert!(shown_alpha(10, 10) == 0.0 && shown_alpha(30, 25) == 1.0, "{depth}-bit: shown moved");
        enter(&mut app);
        assert_eq!(offset(&app), None, "{depth}-bit: ↩ dropped it");
        assert_eq!(alpha(&app, id, 10, 10), 0.0, "{depth}-bit: the hole is transparent");
        assert_eq!(alpha(&app, id, 30, 25), 1.0, "{depth}-bit: the red square moved");
        assert_eq!(alpha(&app, id, 48, 16), 1.0, "{depth}-bit: the unselected blue one stayed");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(18, 13, 34, 29), "{depth}-bit");
        assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some("Move")), "{depth}-bit");
        assert!(app.session.undo());
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 25) == 0.0, "{depth}-bit: undone");
    }
}

/// ⌥ copies the selected pixels (no layer copy), ⇧ locks the drag to multiples of 45°.
#[test]
fn alt_copies_and_shift_constrains_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let layers = app.session.active().unwrap().doc.layers.len();
        drag(&mut app, [16.0, 16.0], [30.0, 28.0], egui::Modifiers::ALT | egui::Modifiers::SHIFT);
        assert_eq!(offset(&app), Some((13, 13)), "{depth}-bit: 45°");
        app.run("select.drop", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "{depth}-bit: no layer copy");
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 35, 35) == 1.0, "{depth}-bit: original and copy");
        assert!(app.session.undo());
        assert_eq!(alpha(&app, id, 35, 35), 0.0, "{depth}-bit: undone");
    }
}

/// #7538: a loose marquee round the red square shrinks to it as the drag starts: the canvas
/// outlines the moved pixels while they float, and the dropped selection is theirs. Alt-drag
/// (a copy) too.
#[test]
fn dragging_selected_pixels_shrinks_the_ants_to_them() {
    for depth in [8, 16, 32] {
        for m in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
            let (mut app, _) = selected_pixels(depth);
            app.session.execute("select.rect", json!({"x": 2, "y": 2, "width": 30, "height": 30})).unwrap();
            drag(&mut app, [16.0, 16.0], [26.0, 21.0], m);
            assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit, {m:?}: floating");
            let ants = crate::canvas::floating_outline(&app, 0).map(|s| s.content_bounds());
            assert_eq!(ants, Some(Rect::new(8, 8, 24, 24)), "{depth}-bit, {m:?}: the ants hug the red square");
            assert!(crate::canvas::floating_outline(&app, 1).is_none(), "only the active document's");
            app.run("select.drop", json!({})).unwrap();
            let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
            assert_eq!(sel, Some(Rect::new(18, 13, 34, 29)), "{depth}-bit, {m:?}: dropped");
            assert!(crate::canvas::floating_outline(&app, 0).is_none(), "nothing floats");
        }
    }
}

/// ⌘-drag with a painting tool drags the selected pixels as the Move tool does, also when ⌘
/// comes with the event (control channel, MCP) rather than as the held-key override.
#[test]
fn command_drag_with_a_painting_tool_moves_only_the_selected_pixels() {
    let (mut app, id) = selected_pixels(8);
    app.ui.tool = Tool::Brush;
    app.ui.tool_options.move_auto_select = false;
    let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((10, 5)), "the selected pixels float");
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((20, 10)), "a second drag moves the same floating piece");
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 40, 30) == 1.0, "the red square moved");
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the unselected blue one stayed");
}

/// A click moves nothing and leaves nothing floating (Undo isn't spent putting it back).
#[test]
fn a_click_with_a_selection_moves_nothing() {
    let (mut app, id) = selected_pixels(8);
    let steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, [16.0, 16.0], [16.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 48, 16) == 1.0);
}

#[test]
fn arrow_keys_nudge_the_selected_pixels() {
    let (mut app, id) = selected_pixels(16);
    crate::move_mods::nudge(&mut app, 10.0, 0.0, false);
    crate::move_mods::nudge(&mut app, 0.0, 1.0, false);
    assert_eq!(offset(&app), Some((10, 1)));
    // ⌥ leaves the piece where it is and nudges a copy of it.
    crate::move_mods::nudge(&mut app, 1.0, 0.0, true);
    assert_eq!(offset(&app), Some((1, 0)));
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 18, 20) == 1.0 && alpha(&app, id, 34, 20) == 1.0);
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the layer itself didn't move");
}

/// Without floatable pixels (a position-locked layer) the Move tool behaves as without a
/// selection: it moves the layer, which the lock refuses.
#[test]
fn a_layer_that_cannot_float_moves_whole_as_before() {
    let (mut app, id) = selected_pixels(8);
    app.session.execute("layer.lockLayers", json!({"position": true})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 16) == 0.0, "nothing moved");
    // Without a selection the layer moves whole.
    app.session.execute("layer.lockLayers", json!({"position": false})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 58, 16) == 1.0, "the whole layer moved");
}

#[test]
fn floating_selection_does_not_snap_to_its_own_starting_outline_or_preview_at_zero() {
    let (mut app, _id) = selected_pixels(8);
    app.ui.extras.snap = true;
    app.ui.view.snap_to.document_bounds = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "no preview composite at offset (0, 0) before the pointer moves");
    // A small 2 px move within the snap radius must move immediately instead of snapping back to (0, 0).
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 18.0, y: 17.0, pressure: 1.0 }, m);
    assert_eq!(super::floating_offset(&app), Some((2, 1)), "small drag moves without snapping to own selection bounds");
    assert!(super::display_doc(&mut app, 0).is_some(), "preview is shown once offset is non-zero");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 18.0, y: 17.0 }, m);
    assert_eq!(offset(&app), Some((2, 1)));
}
