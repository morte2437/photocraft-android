//! #12498: an options-bar Split button places one split line, then its mode ends, as in the
//! reference. Quick splits and the menu / control-channel commands keep their behaviour.

use super::super::tests::app_with_square;
use super::*;
use egui::PointerButton;
use egui_kittest::kittest::Queryable;

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// A 400 × 300 layer in Warp with the Default grid.
fn warping() -> Harness {
    let app = app_with_square(512, photocraft_geom::Rect::new(56, 56, 456, 356));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    run(&mut h, "edit.transform.warp", json!({}));
    h
}

/// A menu / control-channel command, then a few frames.
fn run(h: &mut Harness, id: &str, params: serde_json::Value) {
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, id, params).unwrap();
    h.run_steps(3);
}

fn warp(h: &Harness) -> Option<Warp> {
    h.state().ui.transform.as_ref().unwrap().warp.clone()
}

/// The grid's horizontal knots (`vs`): one more per horizontal split line.
fn rows(h: &Harness) -> usize {
    warp(h).and_then(|w| w.mesh).map_or(2, |m| m.vs.len())
}

fn armed(h: &Harness) -> Option<SplitTool> {
    h.state().transform_preview.as_ref().unwrap().split_tool
}

fn press_button(h: &mut Harness, tip: &str) {
    h.get_by_label(tip).click();
    h.run_steps(3);
}

/// A click on the canvas at document point `p`, through the real pointer path.
fn click_doc(h: &mut Harness, p: [f32; 2], m: egui::Modifiers) {
    let at = ViewXform::active(h.state()).unwrap().to_screen(p[0], p[1]);
    h.event(egui::Event::ModifiersChanged(m));
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: m });
        h.run_steps(2);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn a_split_button_places_one_line_and_then_the_mode_ends() {
    let mut h = warping();
    press_button(&mut h, "Split Warp Horizontally");
    assert!(armed(&h) == Some(SplitTool::Horizontal), "the button arms its mode");
    let before = rows(&h);
    click_doc(&mut h, [256.0, 156.0], egui::Modifiers::NONE);
    assert_eq!(rows(&h), before + 1, "one split line at the click");
    // The bar draws the button pressed only while `split_tool` names it.
    assert!(armed(&h).is_none(), "the mode ends after one split");
    let placed = rows(&h);
    click_doc(&mut h, [256.0, 300.0], egui::Modifiers::NONE);
    assert_eq!(rows(&h), placed, "the next click is an ordinary press, not another split");
    assert!(armed(&h).is_none());
}

#[test]
fn every_split_button_is_one_shot() {
    for tip in ["Split Warp Crosswise", "Split Warp Vertically", "Split Warp Horizontally"] {
        let mut h = warping();
        press_button(&mut h, tip);
        assert!(armed(&h).is_some(), "{tip} arms its mode");
        let before = warp(&h);
        click_doc(&mut h, [200.0, 150.0], egui::Modifiers::NONE);
        assert!(warp(&h) != before, "{tip} split the grid");
        assert!(armed(&h).is_none(), "{tip} ends after one split");
    }
}

#[test]
fn a_second_press_on_the_button_still_cancels_without_a_split() {
    let mut h = warping();
    let before = warp(&h);
    press_button(&mut h, "Split Warp Vertically");
    press_button(&mut h, "Split Warp Vertically");
    assert!(armed(&h).is_none(), "the second press cancels the mode");
    assert_eq!(warp(&h), before, "nothing was split");
}

/// Not measured: a release that places no line keeps the mode armed, as today.
#[test]
fn a_release_that_places_no_line_keeps_the_mode() {
    let mut h = warping();
    run(&mut h, "edit.transform.splitWarpHorizontally", json!({"at": [256.0, 156.0]}));
    press_button(&mut h, "Split Warp Horizontally");
    let before = rows(&h);
    click_doc(&mut h, [256.0, 20.0], egui::Modifiers::NONE);
    assert_eq!(rows(&h), before, "above the grid nothing is split");
    assert!(armed(&h) == Some(SplitTool::Horizontal), "the mode stays after a miss above the grid");
    click_doc(&mut h, [256.0, 156.0], egui::Modifiers::NONE);
    assert_eq!(rows(&h), before, "the line is already there");
    assert!(armed(&h) == Some(SplitTool::Horizontal), "the mode stays after a click on an existing line");
}

#[test]
fn quick_splits_still_place_one_split_per_click() {
    let mut h = warping();
    let before = rows(&h);
    // On the Default grid a crosswise split also keeps both thirds, so it adds more than one row.
    click_doc(&mut h, [256.0, 131.0], egui::Modifiers::ALT);
    let first = rows(&h);
    assert!(first > before, "the first Option-click splits");
    click_doc(&mut h, [356.0, 281.0], egui::Modifiers::ALT);
    assert_eq!(rows(&h), first + 1, "the second Option-click adds one more line");
    click_doc(&mut h, [156.0, 206.0], egui::Modifiers::COMMAND);
    assert_eq!(rows(&h), first + 2, "a Ctrl/Cmd-click splits the same way");
    assert!(armed(&h).is_none(), "no mode is armed");
}

#[test]
fn menu_and_control_channel_splits_still_split_at_once() {
    let mut h = warping();
    let before = rows(&h);
    run(&mut h, "edit.transform.splitWarpHorizontally", json!({}));
    run(&mut h, "edit.transform.splitWarpHorizontally", json!({"at": [256.0, 300.0]}));
    assert_eq!(rows(&h), before + 2, "at the box centre, then at `at`");
    assert!(armed(&h).is_none(), "a command never arms the mode");
}
