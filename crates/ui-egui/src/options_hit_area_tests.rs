//! #5052: the actual Brush options caption and field form one scrub target.
use crate::PhotocraftApp;
use egui::{Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = crate::state::Tool::Brush;
    app.session.tools.brush.hardness = 0.5;
    let mut h = Harness::builder().with_size(vec2(1400.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn geometry(h: &Harness<'_, PhotocraftApp>) -> (Rect, Rect) {
    // The visible caption, not the number field it names: kittest returns the labelled control
    // for `get_by_label`, whose rect would put the drag inside the numeric editor.
    let label = h
        .query_all(egui_kittest::kittest::by().label("Hardness").include_labels())
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .expect("the Hardness caption")
        .rect();
    let field = h
        .get_all_by_role(egui::accesskit::Role::SpinButton)
        .map(|n| n.rect())
        .filter(|r| r.left() >= label.right())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    (label, field)
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, dx: f32, mods: Modifiers) {
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    for i in 1..=4 {
        h.event(egui::Event::ModifiersChanged(mods));
        h.hover_at(at + vec2(dx * i as f32 / 4.0, 0.0));
        h.run();
    }
    h.drop_at(at + vec2(dx, 0.0));
    h.run_steps(2);
}

#[test]
fn options_hardness_label_drag_cannot_copy_selected_caption() {
    let mut h = harness();
    let (l, _) = geometry(&h);
    drag(&mut h, pos2(l.left() + 2.0, l.center().y), 20.0, Modifiers::NONE);
    assert!(!h.ctx.plugin::<egui::text_selection::LabelSelectionState>().lock().has_selection(), "scrubbing Hardness must not create label text selection");
    h.event(egui::Event::Copy);
    h.step();
    assert!(
        !h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s.contains("Hard"))),
        "a scrub caption must not select text for copying"
    );
    assert!(h.state().session.tools.brush.hardness > 0.5);
}

#[test]
fn options_hardness_caption_upper_and_lower_margins_scrub() {
    for dy in [-10.0, 10.0] {
        let mut h = harness();
        let (l, _) = geometry(&h);
        drag(&mut h, l.center() + vec2(0.0, dy), 12.0, Modifiers::NONE);
        assert!((h.state().session.tools.brush.hardness - 0.62).abs() < 0.0001, "24pt caption margin dy={dy}: {}", h.state().session.tools.brush.hardness);
        assert_eq!(h.state().session.journal.len(), 1, "one command for the whole gesture");
    }
}

#[test]
fn options_hardness_gap_and_suffix_scrub() {
    for suffix in [false, true] {
        let mut h = harness();
        let (l, f) = geometry(&h);
        let x = if suffix { f.right() + 8.0 } else { (l.right() + f.left() - 4.0) / 2.0 };
        drag(&mut h, pos2(x, l.center().y), 12.0, Modifiers::NONE);
        assert!((h.state().session.tools.brush.hardness - 0.62).abs() < 0.0001, "new region suffix={suffix}: {}", h.state().session.tools.brush.hardness);
        assert_eq!(h.state().session.journal.len(), 1);
    }
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run_steps(2);
}

#[test]
fn options_numeric_editing_and_focused_text_selection_keep_priority() {
    let mut h = harness();
    let (_, f) = geometry(&h);
    click(&mut h, f.center());
    let id = h.ctx.memory(|m| m.focused()).expect("click focuses the numeric editor");
    drag(&mut h, pos2(f.left() + 2.0, f.center().y), f.width() - 4.0, Modifiers::NONE);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(id), "text drag keeps numeric focus");
    assert_eq!(h.state().session.tools.brush.hardness, 0.5, "selecting digits must not scrub");
    let state = egui::TextEdit::load_state(&h.ctx, id).expect("focused text state");
    assert!(!state.cursor.char_range().unwrap().is_empty(), "digit selection still works");
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("25+5".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!((h.state().session.tools.brush.hardness - 0.3).abs() < 0.0001);
}

#[test]
fn options_expanded_hit_click_does_not_edit_or_request_focus() {
    let mut h = harness();
    let (_, f) = geometry(&h);
    click(&mut h, f.center());
    let before_focus = h.ctx.memory(|m| m.focused()).expect("numeric editor focus");
    let before_journal = h.state().session.journal.len();
    let (l, _) = geometry(&h);
    click(&mut h, l.center() + vec2(0.0, 10.0));
    assert_eq!(h.state().session.tools.brush.hardness, 0.5);
    println!(
        "focus_before={before_focus:?} focus_after={:?} value={} journal_before={before_journal} journal_after={}",
        h.ctx.memory(|m| m.focused()),
        h.state().session.tools.brush.hardness,
        h.state().session.journal.len()
    );
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(before_focus), "matches actual main: background retains numeric focus without acquiring it");
    assert_eq!(h.state().session.journal.len(), before_journal);
}

#[test]
fn options_added_regions_preserve_modifiers_and_numeric_drag_rate() {
    for (mods, dx, want) in [(Modifiers::SHIFT, 3.0, 0.8), (Modifiers::ALT, 20.0, 0.52)] {
        let mut h = harness();
        let (l, f) = geometry(&h);
        drag(&mut h, pos2(f.right() + 8.0, l.center().y), dx, mods);
        assert!((h.state().session.tools.brush.hardness - want).abs() < 0.0001);
    }
    let mut h = harness();
    let (_, f) = geometry(&h);
    drag(&mut h, f.center(), 20.0, Modifiers::NONE);
    assert!((h.state().session.tools.brush.hardness - 0.6).abs() < 0.0001, "numeric drag retains its own rate");
}

#[test]
fn options_disabled_eraser_caption_and_added_regions_do_nothing() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::Eraser;
    h.state_mut().ui.tool_options.eraser_mode = "block".into();
    h.run_steps(3);
    let (l, f) = geometry(&h);
    for at in [l.center(), pos2(f.right() + 8.0, l.center().y), l.center() + vec2(0.0, 10.0)] {
        drag(&mut h, at, 12.0, Modifiers::NONE);
        assert_eq!(h.state().session.tools.brush.hardness, 0.5);
        assert!(h.state().session.journal.is_empty());
    }
}

#[test]
fn options_field_outer_top_and_bottom_margins_scrub() {
    for top in [true, false] {
        let mut h = harness();
        let (_, f) = geometry(&h);
        let y = if top { f.top() - 0.5 } else { f.bottom() + 0.5 };
        drag(&mut h, pos2(f.center().x, y), 12.0, Modifiers::NONE);
        assert!((h.state().session.tools.brush.hardness - 0.62).abs() < 0.0001, "field margin top={top}");
        assert_eq!(h.state().session.journal.len(), 1);
    }
}

#[test]
fn options_gesture_keeps_its_start_owner_when_crossing_regions() {
    let mut h = harness();
    let (l, _) = geometry(&h);
    drag(&mut h, pos2(l.right() - 2.0, l.center().y), 24.0, Modifiers::NONE);
    assert!((h.state().session.tools.brush.hardness - 0.74).abs() < 0.0001);
    assert_eq!(h.state().session.journal.len(), 1);
    let mut h = harness();
    let (_, f) = geometry(&h);
    drag(&mut h, pos2(f.left() + 2.0, f.center().y), -30.0, Modifiers::NONE);
    assert!((h.state().session.tools.brush.hardness - 0.35).abs() < 0.0001);
    assert_eq!(h.state().session.journal.len(), 1);
}

#[test]
fn options_tab_moves_from_hardness_to_opacity_without_a_caption_stop() {
    let mut h = harness();
    let (_, f) = geometry(&h);
    click(&mut h, f.center());
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.hardness, 0.5);
    assert!((h.state().session.tools.brush.opacity - 0.99).abs() < 0.0001);
}

// Regression test for #3425: the options bar drew its own 1 px top and bottom separator inside
// the panel, on top of the panel's own border, so the borders doubled (three lines, not one).
#[test]
fn options_bar_has_a_single_full_width_border() {
    for kind in [crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Classic] {
        let mut h = harness();
        PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(4);
        let separator = crate::theme::Tokens::get(&h.ctx).separator;
        let borders: Vec<_> = h
            .output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == separator && (points[0].y - points[1].y).abs() < 0.01 && (points[1].x - points[0].x).abs() > 400.0 =>
                {
                    Some(*points)
                }
                _ => None,
            })
            .collect();
        assert_eq!(borders.len(), 1, "{kind:?}: the options bar has exactly one border, the panel's (its own would double it): {borders:?}");
    }
}
