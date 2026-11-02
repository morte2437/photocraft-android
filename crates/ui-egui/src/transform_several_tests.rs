//! Free Transform with several layers selected (#6501): ⌘T frames them all and the box moves
//! every one of them, type and pixels alike, in one history step.

use photocraft_doc::{LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;

/// A 200 × 200 document: "Paint" with a red block at x 10..50, y 10..30, and a type layer over it.
/// Both are selected with Paint active, as after ⌘-clicking Paint in the Layers panel (the
/// reporter's steps).
fn app() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let text = LayerId(app.run("type.create", json!({"x": 80, "y": 120, "text": "Hi", "size": 30})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    app.run("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(paint));
    (app, paint, text)
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let st = app.session.active().unwrap();
    photocraft_engine::transform_cmds::transform_bounds(&st.doc, st.doc.layer(id).unwrap())
}

fn free_transform(app: &mut PhotocraftApp) {
    crate::menus::invoke(app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
}

#[test]
fn free_transform_frames_every_selected_layer_and_moves_them_all() {
    let (mut app, paint, text) = app();
    let (p0, t0) = (bounds(&app, paint), bounds(&app, text));
    free_transform(&mut app);
    let t = app.ui.transform.as_ref().unwrap();
    let both = p0.union(&t0);
    assert_eq!(t.rect, [both.x0, both.y0, both.x1, both.y1].map(f64::from), "the box frames both layers");
    assert_eq!((t.layer, t.layers.clone()), (paint.0, vec![paint.0, text.0]));
    // Both move with the box: neither stays behind on the canvas under it.
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(!pv.doc.layer(paint).unwrap().visible && !pv.doc.layer(text).unwrap().visible);
    let steps = app.session.active().unwrap().history.past_len();
    // A quarter turn about the box's centre.
    let r = t.rect;
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [cx - (y - cy), cy + (x - cx)]);
    }
    crate::transform_tool::commit(&mut app);
    assert_eq!(app.ui.status, "");
    assert!(app.ui.transform.is_none());
    let (p1, t1) = (bounds(&app, paint), bounds(&app, text));
    assert!(p1.width().abs_diff(p0.height()) <= 1 && p1.height().abs_diff(p0.width()) <= 1, "Paint turned: {p0:?} -> {p1:?}");
    assert!(t1.width().abs_diff(t0.height()) <= 2 && t1.height().abs_diff(t0.width()) <= 2, "the type turned: {t0:?} -> {t1:?}");
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.layer(text).unwrap().content, LayerContent::Text(_)), "still editable type");
    assert_eq!(st.history.past_len(), steps + 1, "one history step");
    assert_eq!(st.selected_layers(), [paint, text], "both stay selected");
}

#[test]
fn with_a_selection_the_pixels_are_lifted_and_the_type_moves_whole() {
    let (mut app, paint, text) = app();
    let t0 = bounds(&app, text);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 200})).unwrap();
    free_transform(&mut app);
    let pv = app.transform_preview.as_ref().unwrap();
    let under = pv.doc.layer(paint).unwrap();
    assert!(under.visible, "Paint stays, less the selected pixels");
    assert_eq!(under.surface().unwrap().content_bounds(), Rect::new(30, 10, 50, 30));
    assert!(!pv.doc.layer(text).unwrap().visible, "the type moves whole (#2630)");
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x, y + 100.0]);
    }
    crate::transform_tool::commit(&mut app);
    assert_eq!(app.ui.status, "");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(paint).unwrap().surface().unwrap().content_bounds(), Rect::new(10, 10, 50, 130));
    assert_eq!(bounds(&app, text), t0.translate(0, 100));
}

#[test]
fn cancel_leaves_every_layer_as_it_was() {
    let (mut app, paint, text) = app();
    let before = app.session.active().unwrap().doc.clone();
    free_transform(&mut app);
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    crate::transform_tool::cancel(&mut app);
    assert!(app.ui.transform.is_none());
    let st = app.session.active().unwrap();
    assert!(st.doc == before);
    assert_eq!(st.selected_layers(), [paint, text]);
}

#[test]
fn warp_is_greyed_and_distort_too_while_type_is_among_them() {
    let (mut app, paint, text) = app();
    for id in ["edit.freeTransform", "edit.transform.scale", "edit.transform.rotate", "edit.transform.skew", "edit.transform.flipHorizontal"] {
        assert!(crate::menus::is_enabled(&app, id), "{id}");
    }
    for id in ["edit.transform.warp", "edit.transform.distort", "edit.transform.perspective"] {
        assert!(!crate::menus::is_enabled(&app, id), "{id} is greyed");
    }
    assert_eq!(crate::transform_tool::begin_warp(&mut app, &egui::Context::default()), Err(crate::transform_tool::WARP_ONE_LAYER.into()));
    assert!(app.ui.transform.is_none(), "no box is left open");
    // In an open box too.
    free_transform(&mut app);
    for id in ["edit.transform.warp", "edit.transform.distort", "edit.transform.perspective"] {
        assert!(!crate::menus::is_enabled(&app, id), "{id} is greyed in the box");
    }
    crate::transform_tool::cancel(&mut app);
    // Two pixel layers distort.
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    app.run("layer.rasterize.type", json!({})).unwrap();
    app.run("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    assert!(crate::menus::is_enabled(&app, "edit.transform.distort") && crate::menus::is_enabled(&app, "edit.transform.perspective"));
    assert!(!crate::menus::is_enabled(&app, "edit.transform.warp"));
}

#[test]
fn the_warp_button_keeps_the_box_when_the_active_layer_is_type() {
    let (mut app, paint, text) = app();
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(text));
    free_transform(&mut app);
    let before = app.ui.transform.as_ref().map(|t| (t.rect, t.quad));
    crate::transform_tool::enter_warp(&mut app, &egui::Context::default());
    let t = app.ui.transform.as_ref().unwrap();
    assert!(t.warp.is_none(), "no warp on several layers");
    assert_eq!(Some((t.rect, t.quad)), before);
}

#[test]
fn a_locked_layer_among_them_refuses_before_the_box_opens() {
    let (mut app, paint, text) = app();
    app.run("layer.setProps", json!({"layer": text.0, "locks": {"position": true}})).unwrap();
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    app.run("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({}));
    assert_eq!(r, Err(crate::transform_tool::LOCKED.into()));
    assert!(app.ui.transform.is_none());
}

#[test]
fn with_the_background_active_the_box_moves_the_other_selected_layers() {
    let (mut app, paint, text) = app();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(bg));
    assert!(crate::menus::is_enabled(&app, "edit.freeTransform"), "not greyed for the Background");
    free_transform(&mut app);
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!((t.layer, t.layers.clone()), (bg.0, vec![paint.0, text.0]));
    let p0 = bounds(&app, paint);
    if let Some(t) = app.ui.transform.as_mut() {
        // Nearest neighbour: no resampling fringe at the block's edge.
        t.interpolation = "nearest".into();
        t.quad = t.quad.map(|[x, y]| [x + 5.0, y]);
    }
    crate::transform_tool::commit(&mut app);
    assert_eq!(app.ui.status, "");
    let st = app.session.active().unwrap();
    assert!(st.doc.is_background(bg), "the Background is left alone");
    assert_eq!(bounds(&app, paint), p0.translate(5, 0));
}

#[test]
fn picking_another_layer_applies_the_box_to_all_of_them() {
    let (mut app, paint, text) = app();
    let (p0, t0) = (bounds(&app, paint), bounds(&app, text));
    free_transform(&mut app);
    if let Some(t) = app.ui.transform.as_mut() {
        t.interpolation = "nearest".into();
        t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    }
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(bounds(&app, paint), p0.translate(10, 0));
    assert_eq!(bounds(&app, text), t0.translate(10, 0));
}

/// Paint with a 6 px outside stroke, then both layers selected again (Paint active).
fn stroked() -> (PhotocraftApp, LayerId, LayerId) {
    let (mut app, paint, text) = app();
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("layer.layerStyle.stroke", json!({"size": 6, "color": "#00ff00", "position": "outside"})).unwrap();
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    app.run("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    (app, paint, text)
}

#[test]
fn styles_past_the_frame_move_with_the_box() {
    let (mut app, _, _) = stroked();
    free_transform(&mut app);
    // The frame is the content's (the block starts at x 10); its stroke reaches 6 px past it.
    assert_eq!(app.ui.transform.as_ref().unwrap().rect[0], 10.0, "the handles stay on the content");
    let (stroke, quad) = crate::transform_tool::moving_texel(&app, [6, 20]);
    assert!(stroke.is_some_and(|c| c.a() > 200 && c.g() > 200), "the stroke left of the block is in the moving pixels: {stroke:?}");
    assert!(quad[0][0] <= 4.0, "and drawn there: {quad:?}");
    // Moving the box carries the texture's own rect with it.
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 50.0, y + 5.0]);
    }
    let (_, moved) = crate::transform_tool::moving_texel(&app, [6, 20]);
    for (a, b) in quad.iter().zip(&moved) {
        assert!((b[0] - a[0] - 50.0).abs() < 1e-9 && (b[1] - a[1] - 5.0).abs() < 1e-9, "{quad:?} -> {moved:?}");
    }
    // A quarter turn maps the stroke's corner where the frame's map takes it.
    let r = app.ui.transform.as_ref().unwrap().rect;
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = crate::transform_tool::corners(r).map(|[x, y]| [cx - (y - cy), cy + (x - cx)]);
    }
    let (_, turned) = crate::transform_tool::moving_texel(&app, [6, 20]);
    let [x0, y0] = quad[0];
    assert!((turned[0][0] - (cx - (y0 - cy))).abs() < 1e-6 && (turned[0][1] - (cy + (x0 - cx))).abs() < 1e-6, "{turned:?}");
}

/// A texel of the type's glyphs in the moving pixels as the box opened.
fn glyph_texel(app: &PhotocraftApp, text: LayerId) -> [i32; 2] {
    let b = bounds(app, text);
    (b.y0..b.y1)
        .flat_map(|y| (b.x0..b.x1).map(move |x| [x, y]))
        .find(|p| crate::transform_tool::moving_texel(app, *p).0.is_some_and(|c| c.a() > 200))
        .expect("the type is in the moving pixels")
}

#[test]
fn hiding_or_fading_a_moving_layer_shows_in_the_preview_at_once() {
    let (mut app, _, text) = app();
    free_transform(&mut app);
    let p = glyph_texel(&app, text);
    // Paint stays the active layer, so the box stays open.
    app.run("layer.setProps", json!({"layer": text.0, "opacity": 0.5})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_some());
    let half = crate::transform_tool::moving_texel(&app, p).0.unwrap();
    assert!((100..=155).contains(&half.a()), "half opacity: {half:?}");
    app.run("layer.setProps", json!({"layer": text.0, "visible": false})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    assert_eq!(crate::transform_tool::moving_texel(&app, p).0.map(|c| c.a()), Some(0), "a hidden layer moves nothing");
    // The commit still transforms it, hidden, like Photoshop.
    let t0 = bounds(&app, text);
    if let Some(t) = app.ui.transform.as_mut() {
        t.interpolation = "nearest".into();
        t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    }
    crate::transform_tool::commit(&mut app);
    assert_eq!(bounds(&app, text), t0.translate(10, 0));
}

#[test]
fn a_style_added_while_the_box_is_open_grows_the_moving_pixels() {
    let (mut app, paint, _) = app();
    free_transform(&mut app);
    assert!(crate::transform_tool::moving_texel(&app, [6, 20]).0.is_none(), "nothing past the block yet");
    app.run("layer.layerStyle.stroke", json!({"layer": paint.0, "size": 6, "color": "#00ff00", "position": "outside"})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    let stroke = crate::transform_tool::moving_texel(&app, [6, 20]).0;
    assert!(stroke.is_some_and(|c| c.a() > 200), "{stroke:?}");
}

fn group(app: &mut PhotocraftApp, ids: &[LayerId]) -> LayerId {
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
    LayerId(app.run("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn edits_to_the_children_of_a_moving_group_show_in_the_preview() {
    let (mut app, paint, text) = app();
    let g = group(&mut app, &[paint]);
    app.run("layer.select", json!({"layer": g.0})).unwrap();
    app.run("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    free_transform(&mut app);
    assert_eq!(app.ui.transform.as_ref().unwrap().layers, [g.0, text.0]);
    let red = |app: &PhotocraftApp| crate::transform_tool::moving_texel(app, [20, 20]).0.map(|c| c.a());
    assert_eq!(red(&app), Some(255), "the group's child is in the moving pixels");
    app.run("layer.setProps", json!({"layer": paint.0, "opacity": 0.5})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert!(red(&app).is_some_and(|a| (100..=155).contains(&a)), "the child at half opacity: {:?}", red(&app));
    app.run("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    assert_eq!(red(&app), Some(0), "a hidden child moves nothing");
}

#[test]
fn a_style_of_the_group_around_the_moving_layers_moves_with_them() {
    let (mut app, paint, text) = app();
    let g = group(&mut app, &[paint, text]);
    app.run("layer.layerStyle.stroke", json!({"layer": g.0, "size": 6, "color": "#00ff00", "position": "outside"})).unwrap();
    // The two layers in the group, not the group itself.
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    app.run("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    free_transform(&mut app);
    assert_eq!(app.ui.transform.as_ref().unwrap().layers, [paint.0, text.0]);
    let (stroke, quad) = crate::transform_tool::moving_texel(&app, [6, 20]);
    assert!(stroke.is_some_and(|c| c.a() > 200 && c.g() > 200), "the group's stroke left of the block: {stroke:?}");
    assert!(quad[0][0] <= 4.0, "{quad:?}");
}
