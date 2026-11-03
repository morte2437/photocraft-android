//! Free Transform with the whole app: picking another tool or layer, or New Layer, while the box
//! is open applies the transform as it stands, as Photoshop does (#5441, #5665, #5667).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

/// The app with a 400 × 300 document: a red 100 × 60 rectangle at (100, 100) on its own layer
/// above the Background.
fn app() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    h
}

/// Open Free Transform and move the box 50 px right, 40 px down.
fn transform_and_move(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    let t = h.state_mut().ui.transform.as_mut().expect("Free Transform is open");
    for p in &mut t.quad {
        p[0] += 50.0;
        p[1] += 40.0;
    }
    h.run_steps(2);
}

fn red_bounds(h: &Harness<'static, PhotocraftApp>) -> photocraft_geom::Rect {
    let st = h.state().session.active().unwrap();
    let top = st.doc.layers.last().unwrap();
    top.surface().unwrap().content_bounds()
}

#[test]
fn picking_another_tool_in_the_toolbar_applies_the_transform() {
    let mut h = app();
    transform_and_move(&mut h);
    let hand = h.query_by_label_contains("Hand Tool").expect("the toolbar's Hand Tool");
    hand.click();
    h.run_steps(4);
    assert!(h.state().ui.transform.is_none(), "the box closed");
    assert_eq!(red_bounds(&h), photocraft_geom::Rect::new(150, 140, 250, 200), "the move was applied, not cancelled");
}

/// The app with the red rectangle on a layer the last step made (Layer via Copy, which deselects in
/// the same step), as Place Embedded leaves it for the box `begin_placed` opens: cancelling the
/// place takes that step back.
fn placed_app() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layerViaCopy", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "Layer via Copy deselects in its own step");
    h
}

fn history(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().session.active().unwrap().history.entries().to_vec()
}

/// The box on a just-placed layer (a file dropped on the canvas), moved.
fn placed_and_moved(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::transform_tool::begin_placed(h.state_mut(), &ctx).unwrap();
    h.run_steps(2);
    let t = h.state_mut().ui.transform.as_mut().expect("Free Transform is open");
    for p in &mut t.quad {
        p[0] += 50.0;
        p[1] += 40.0;
    }
    h.run_steps(2);
}

#[test]
fn a_placed_image_is_applied_when_another_tool_is_picked() {
    let mut h = placed_app();
    placed_and_moved(&mut h);
    h.query_by_label_contains("Hand Tool").expect("the toolbar's Hand Tool").click();
    h.run_steps(4);
    assert!(h.state().ui.transform.is_none());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2, "the placed layer stays");
    assert_eq!(red_bounds(&h), photocraft_geom::Rect::new(150, 140, 250, 200));
}

#[test]
fn picking_another_layer_in_the_panel_applies_the_transform() {
    for placed in [false, true] {
        let mut h = if placed { placed_app() } else { app() };
        if placed {
            placed_and_moved(&mut h);
        } else {
            transform_and_move(&mut h);
        }
        h.get_by_label("Background").click();
        h.run_steps(4);
        assert!(h.state().ui.transform.is_none(), "placed {placed}");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2, "placed {placed}");
        assert_eq!(red_bounds(&h), photocraft_geom::Rect::new(150, 140, 250, 200), "placed {placed}");
    }
}

#[test]
fn new_layer_applies_the_transform_first() {
    for placed in [false, true] {
        let mut h = if placed { placed_app() } else { app() };
        let before = history(&h);
        if placed {
            placed_and_moved(&mut h);
        } else {
            transform_and_move(&mut h);
        }
        // The Layers panel's New Layer button (the menu item opens the New Layer dialog).
        h.get_by_label("Create a new layer").click();
        h.run_steps(4);
        assert!(h.state().ui.transform.is_none(), "placed {placed}");
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 3, "placed {placed}");
        let red = st.doc.layers[1].surface().unwrap().content_bounds();
        assert_eq!(red, photocraft_geom::Rect::new(150, 140, 250, 200), "placed {placed}: the moved image, not the original");
        // History: the transform (folded into Place Embedded for a placed layer), then New Layer.
        let after = history(&h);
        let added: Vec<&str> = after.iter().skip(before.len().saturating_sub(usize::from(placed))).map(String::as_str).collect();
        let first = if placed { photocraft_engine::file_cmds::PLACE_EMBEDDED } else { "Free Transform" };
        assert_eq!(added, [first, "New Layer"], "placed {placed}: {after:?}");
    }
}

#[test]
fn an_untouched_placed_image_stays_when_another_tool_or_layer_is_picked() {
    for via_layer in [false, true] {
        let mut h = placed_app();
        let ctx = h.ctx.clone();
        crate::transform_tool::begin_placed(h.state_mut(), &ctx).unwrap();
        h.run_steps(2);
        if via_layer {
            let bg = h.state().session.active().unwrap().doc.layers[0].id;
            h.state_mut().run("layer.select", json!({"layer": bg.0})).unwrap();
        } else {
            h.query_by_label_contains("Hand Tool").expect("Hand Tool").click();
        }
        h.run_steps(4);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 2, "via layer {via_layer}: the placed image stays");
        assert_eq!(red_bounds(&h), photocraft_geom::Rect::new(100, 100, 200, 160));
    }
}

/// The placed fixture is a real place: cancelling the box takes the layer back, so the tests
/// above would see a switch that cancelled it.
#[test]
fn cancelling_the_placed_fixture_removes_its_layer() {
    let mut h = placed_app();
    placed_and_moved(&mut h);
    crate::transform_tool::cancel(h.state_mut());
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
}

/// A new-layer command that can't run (Artboard from Group with a pixel layer active) leaves the box and the
/// placed layer as they were.
#[test]
fn a_new_layer_command_that_cant_run_leaves_the_box_open() {
    let mut h = placed_app();
    placed_and_moved(&mut h);
    let before = history(&h);
    assert!(h.state_mut().run("layer.new.artboardFromGroup", json!({})).is_err());
    h.run_steps(2);
    assert!(h.state().ui.transform.is_some(), "the box stays open");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2, "the placed layer stays");
    assert_eq!(history(&h), before);
}

/// A transform the engine refuses (a result far over the pixel budget, #3773) keeps the box open
/// with the placed layer, and the new layer isn't made: the refusal comes back.
#[test]
fn a_refused_transform_keeps_the_placed_layer_and_refuses_the_new_layer() {
    let mut h = placed_app();
    placed_and_moved(&mut h);
    let t = h.state_mut().ui.transform.as_mut().unwrap();
    t.quad = [[0.0, 0.0], [4.0e6, 0.0], [4.0e6, 4.0e6], [0.0, 4.0e6]];
    h.run_steps(2);
    let before = history(&h);
    let r = h.state_mut().run("layer.new.layer", json!({}));
    h.run_steps(2);
    assert!(r.is_err(), "{r:?}");
    assert!(h.state().ui.transform.is_some(), "the box stays open");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2, "the placed layer stays, no new layer");
    assert_eq!(history(&h), before);
}

/// New Layer with bad params, while a moved box is open, is refused before the transform is
/// applied: the box, the layer and History stay as they were.
#[test]
fn new_layer_with_bad_params_leaves_the_box_open() {
    for bad in [json!({"mode": "Bogus"}), json!({"fillNeutral": true}), json!({"position": "x"}), json!([1])] {
        let mut h = app();
        transform_and_move(&mut h);
        let before = history(&h);
        assert!(h.state_mut().run("layer.new.layer", bad.clone()).is_err(), "{bad}");
        h.run_steps(2);
        assert!(h.state().ui.transform.is_some(), "{bad}: the box stays open");
        assert_eq!(history(&h), before, "{bad}: nothing applied");
        assert_eq!(red_bounds(&h), photocraft_geom::Rect::new(100, 100, 200, 160), "{bad}");
    }
}
