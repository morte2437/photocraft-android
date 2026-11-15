//! #11821: the Brush Preset picker shows the tip's angle/roundness widget at its top left, the
//! Brush Settings panel's own widget, and its drags go through `tools.setBrush` as one journaled
//! change each; Escape reverts them as it does a Size or Hardness edit.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_sections::{EllipseGrab, ellipse_drag, ellipse_grab, roundness_handles};
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0, "angle": 0.0, "roundness": 1.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let c = h.state().last_canvas_rect.center();
    let ctx = h.ctx.clone();
    crate::brush_picker::open(&mut h.state_mut().ui, &ctx, [c.x, c.y]);
    h.run_steps(3);
    h
}

/// The widget's centre on screen. It is the top block's first item, 84 × 84, and the Size column
/// starts one item spacing right of it, its first row (24 pt tall, the label centred in it) level
/// with the widget's top.
fn widget_centre(h: &Harness<'static, PhotocraftApp>) -> Pos2 {
    use egui_kittest::kittest::Queryable;
    let picker = h.ctx.memory(|m| m.area_rect(crate::brush_picker::picker_id())).expect("the picker is open");
    let label = h.query_all_by_label(crate::i18n::t("Size")).find(|n| picker.contains(n.rect().center())).expect("the picker's Size label").rect();
    let spacing = h.ctx.global_style().spacing.item_spacing.x;
    Pos2::new(label.left() - spacing - 42.0, label.center().y - 12.0 + 42.0)
}

fn brush(h: &Harness<'static, PhotocraftApp>) -> (f32, f32) {
    let b = &h.state().session.tools.brush;
    (b.angle, b.roundness)
}

/// A press at `from`, moves through `path` and the release at its end, a frame each.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, path: &[Pos2]) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for p in path {
        h.event(Event::PointerMoved(*p));
        h.run_steps(1);
    }
    let end = path.last().copied().unwrap_or(from);
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn the_widget_sits_at_the_pickers_top_left_left_of_size() {
    use egui_kittest::kittest::Queryable;
    let h = harness();
    let c = widget_centre(&h);
    // Its top left is the picker's content corner: inside the popup frame's margin and stroke.
    let picker = h.ctx.memory(|m| m.area_rect(crate::brush_picker::picker_id())).expect("the picker is open");
    let style = h.ctx.global_style();
    let inset =
        vec2(f32::from(style.spacing.menu_margin.left), f32::from(style.spacing.menu_margin.top)) + egui::Vec2::splat(style.visuals.window_stroke().width);
    let corner = c - vec2(42.0, 42.0);
    assert!((corner - (picker.min + inset)).length() <= 2.0, "the widget at the top left: {corner:?} vs {:?}", picker.min + inset);
    // The Size field is right of it, in the same top block.
    let size = h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the Size field").rect();
    assert!(size.left() > c.x + 42.0 && size.top() < c.y + 42.0, "{size:?} vs centre {c:?}");
}

#[test]
fn dragging_the_arrow_sets_the_angle_as_the_panel_widget_does() {
    let mut h = harness();
    let c = widget_centre(&h);
    let (angle, roundness) = brush(&h);
    // On the arrow (angle 0 points right), away from the roundness handles.
    let from = c + vec2(28.0, 0.0);
    assert_eq!(ellipse_grab(from - c, angle, roundness), EllipseGrab::Angle);
    let to = c + vec2(-20.0, -20.0);
    let journal = h.state().session.journal.len();
    drag(&mut h, from, &[c + vec2(10.0, -25.0), to]);
    let want = ellipse_drag(EllipseGrab::Angle, to - c, angle, roundness);
    assert_eq!(brush(&h), want, "the panel widget's values for the same drag");
    assert_eq!(want.0, 135.0);
    let added: Vec<String> = h.state().session.journal[journal..].iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(added, ["tools.setBrush"], "one journaled change for the drag");
}

#[test]
fn dragging_a_handle_sets_the_roundness_as_the_panel_widget_does() {
    let mut h = harness();
    let c = widget_centre(&h);
    let (angle, roundness) = brush(&h);
    let handle = c + roundness_handles(angle, roundness)[0];
    assert_eq!(ellipse_grab(handle - c, angle, roundness), EllipseGrab::Roundness);
    let to = c + vec2(3.0, -12.0);
    // The grab is judged where the drag starts: the first move stays within the handle's reach.
    drag(&mut h, handle, &[handle + vec2(0.0, 6.5), c + vec2(2.0, -20.0), to]);
    let want = ellipse_drag(EllipseGrab::Roundness, to - c, angle, roundness);
    assert_eq!(brush(&h), want);
    assert!((want.1 - 12.0 / 34.0).abs() < 1e-4, "{want:?}");
    // The Brush Settings panel reads the same brush.
    assert_eq!(h.state().session.tools.brush.roundness, want.1);
}

#[test]
fn escape_reverts_an_angle_edit_as_it_does_a_size_edit() {
    let mut h = harness();
    let c = widget_centre(&h);
    let opened = brush(&h);
    drag(&mut h, c + vec2(28.0, 0.0), &[c + vec2(20.0, -15.0), c + vec2(0.0, -30.0)]);
    assert_eq!(brush(&h).0, 90.0);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(brush(&h), opened, "Escape goes back to the values the picker opened with");
    assert_eq!(h.state().ui.brush_picker, None, "and closes it");
}

#[test]
fn enter_keeps_an_angle_edit_and_closes() {
    let mut h = harness();
    let c = widget_centre(&h);
    drag(&mut h, c + vec2(28.0, 0.0), &[c + vec2(20.0, 15.0), c + vec2(0.0, 30.0)]);
    assert_eq!(brush(&h).0, -90.0);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(brush(&h).0, -90.0);
    assert_eq!(h.state().ui.brush_picker, None);
}

#[test]
fn non_finite_pointer_positions_leave_the_brush_unchanged() {
    let mut h = harness();
    let c = widget_centre(&h);
    let before = brush(&h);
    let handle = c + roundness_handles(before.0, before.1)[0];
    h.event(Event::PointerMoved(handle));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    // The pointer reports non-finite positions while the handle is held. (egui's own debug checks
    // panic once a drag is under way at such a position, or on a release there, so the test stops
    // here.)
    for p in [Pos2::new(f32::NAN, c.y), Pos2::new(c.x, f32::INFINITY)] {
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
        let now = brush(&h);
        assert!(now.0.is_finite() && now.1.is_finite(), "{p:?}: {now:?}");
        assert_eq!(now, before, "{p:?}");
    }
}
