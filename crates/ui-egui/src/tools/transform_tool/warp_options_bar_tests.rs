//! #12503: the Warp options bar keeps Bend, H and V to one decimal, shares Free Transform's
//! Commit and Cancel tooltips (Reset reads "Reset the warp"), and starts with Free Transform's
//! reference point check box and grid, greyed and inert; the tool preset picker is greyed too.

use super::super::tests::{bar_harness, session, type_into_bar_field_with};
use super::*;
use egui_kittest::kittest::{NodeT, Queryable};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// An Arc warp over the test box, so Bend, H and V are live.
fn arc(app: &mut PhotocraftApp) {
    let t = app.ui.transform.as_mut().unwrap();
    t.warp = Some(Warp { style: WarpStyle::Arc, bend: 50.0, ..Warp::none(t.rect) });
}

fn warp(t: &TransformSession) -> [f64; 3] {
    let w = t.warp.as_ref().unwrap();
    [w.bend, w.h_distort, w.v_distort]
}

/// Hovers `label`'s control long enough for its tooltip, and says whether `tip` is shown.
fn shows_tip(h: &mut Harness, label: &str, tip: &str) -> bool {
    h.get_by_label(label).hover();
    h.run_steps(60);
    h.query_all_by_label(tip).any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
}

#[test]
fn bend_h_and_v_store_the_one_decimal_they_show() {
    for n in 0..3 {
        let mut want = [50.0, 0.0, 0.0];
        want[n] = 12.3;
        assert_eq!(warp(&type_into_bar_field_with(arc, n, "12.345")), want, "field {n}");
    }
    // Past the range, the value is clamped as before.
    assert_eq!(warp(&type_into_bar_field_with(arc, 1, "-150")), [50.0, -100.0, 0.0]);
    let h = bar_harness(|app| {
        arc(app);
        app.ui.transform.as_mut().unwrap().warp.as_mut().unwrap().h_distort = 12.3;
    });
    assert_eq!(shown(&h), ["50.0", "12.3", "0.0"]);
    // Entering Warp from Free Transform: no bend yet.
    let mut h = bar_harness(|_| {});
    // Free Transform's X, Y, W and H keep dropping trailing zeros.
    assert_eq!(shown(&h)[..4], ["50", "25", "100", "100"]);
    h.get_by_label("Switch between free transform and warp modes").click();
    h.run_steps(4);
    assert_eq!(shown(&h), ["0.0", "0.0", "0.0"]);
}

#[test]
fn tabbing_through_keeps_a_value_it_did_not_edit() {
    let mut h = bar_harness(|app| {
        arc(app);
        app.ui.transform.as_mut().unwrap().warp.as_mut().unwrap().bend = 33.333;
    });
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run();
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(warp(h.state().ui.transform.as_ref().unwrap()), [33.333, 0.0, 0.0]);
}

#[test]
fn an_arithmetic_entry_is_stored_as_the_field_is_left() {
    assert_eq!(warp(&type_into_bar_field_with(arc, 1, "10+2")), [50.0, 12.0, 0.0]);
    // Appended to the shown 10.0, no keystroke changes the value before the field is left.
    for leave in [egui::Key::Enter, egui::Key::Tab] {
        let mut h = bar_harness(|app| {
            arc(app);
            app.ui.transform.as_mut().unwrap().warp.as_mut().unwrap().h_distort = 10.0;
        });
        h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(1).unwrap().click();
        h.run();
        h.key_press(egui::Key::End);
        h.run();
        for c in "+2.25".chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run();
        }
        h.key_press(leave);
        h.run_steps(3);
        assert_eq!(warp(h.state().ui.transform.as_ref().unwrap()), [50.0, 12.3, 0.0], "{leave:?}");
    }
}

/// The text of every number field in the bar, in order.
fn shown(h: &Harness) -> Vec<String> {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).map(|n| n.accesskit_node().value().unwrap_or_default().to_string()).collect()
}

#[test]
fn warp_actions_have_the_free_transform_tooltips() {
    let commit = crate::i18n::fmt("Commit transform ({key})", &[("key", &crate::shortcuts::pretty("Enter"))]);
    let mut h = bar_harness(arc);
    assert!(shows_tip(&mut h, "Commit transform", &commit));
    assert!(shows_tip(&mut h, "Cancel transform", "Cancel transform (Esc)"));
    assert!(shows_tip(&mut h, "Reset warp", "Reset the warp"));
    // Free Transform keeps its own, and has no Reset.
    let mut h = bar_harness(|_| {});
    assert!(shows_tip(&mut h, "Commit transform", &commit));
    assert!(shows_tip(&mut h, "Cancel transform", "Cancel transform (Esc)"));
    assert!(h.query_by_label("Reset warp").is_none());
}

#[test]
fn warp_bar_starts_with_the_greyed_reference_point_controls() {
    let mut h = bar_harness(arc);
    let toggle = h.get_by_label("Toggle reference point");
    let node = toggle.accesskit_node();
    assert!(node.is_disabled() && node.toggled() == Some(egui::accesskit::Toggled::True), "greyed and on");
    let rect = toggle.rect();
    assert!(rect.right() < h.get_by_label("Split Warp Crosswise").rect().left(), "before Split");
    // In the slot Free Transform has it in.
    assert_eq!(rect.left(), bar_harness(|_| {}).get_by_label("Toggle reference point").rect().left());
    assert!(shows_tip(&mut h, "Toggle reference point", "Toggle reference point"));

    // The grid follows the check box; its top-left dot would move the reference point.
    let gap = h.ctx.global_style().spacing.item_spacing.x;
    let grid_top_left = egui::pos2(rect.right() + gap + 16.0 / 6.0, rect.center().y - 16.0 / 3.0);
    h.event(egui::Event::PointerMoved(grid_top_left));
    h.run_steps(60);
    assert!(h.query_all_by_label("Reference point location").any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label), "grid tooltip");

    let state = |h: &Harness| (h.state().ui.tool_options.transform_reference_point, h.state().ui.transform.as_ref().unwrap().pivot);
    let before = state(&h);
    h.get_by_label("Toggle reference point").click();
    h.run_steps(2);
    click(&mut h, grid_top_left);
    assert_eq!(state(&h), before, "greyed controls do nothing");

    // The same click in Free Transform, with the reference point shown, moves it to the corner.
    let mut h = bar_harness(|app| app.ui.tool_options.transform_reference_point = true);
    click(&mut h, grid_top_left);
    assert_eq!(state(&h), (true, [0.0, 0.0]));
}

fn click(h: &mut Harness, pos: egui::Pos2) {
    click_with(h, pos, egui::PointerButton::Primary);
}

fn click_with(h: &mut Harness, pos: egui::Pos2, button: egui::PointerButton) {
    h.event(egui::Event::PointerMoved(pos));
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() });
        h.run();
    }
    h.run_steps(2);
}

/// The whole options bar (tool preset picker included) over the test box, warping or not.
fn full_bar(warping: bool) -> Harness {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.transform = Some(session());
    if warping {
        arc(&mut app);
    }
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 300.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    h
}

#[test]
fn the_tool_preset_picker_is_greyed_only_while_warping() {
    for warping in [false, true] {
        let mut h = full_bar(warping);
        let tool = h.state().ui.tool.label();
        let picker = h.get_by_label(tool);
        assert_eq!(picker.accesskit_node().is_disabled(), warping, "warping: {warping}");
        // Its right-click menu (Reset Tool) opens only when it is live.
        let at = picker.rect().center();
        click_with(&mut h, at, egui::PointerButton::Secondary);
        assert_eq!(h.query_by_label("Reset Tool").is_none(), warping, "warping: {warping}");
    }
}
