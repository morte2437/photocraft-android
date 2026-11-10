//! Exercise the actual editor window, including its layer and tool-preset targets.

use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn editor(layer: bool) -> (Harness<'static, PhotocraftApp>, Sink) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    let mut params = json!({"stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]], "transparency": [[0, 25], [0.5, 60], [1, 100]]});
    let sink = if layer {
        params["from"] = json!([20, 60]);
        params["to"] = json!([180, 60]);
        app.run(cmds::CREATE, params).unwrap();
        Sink::Layer(app.session.active().unwrap().active_layer.unwrap())
    } else {
        app.run("gradient.presets.select", params).unwrap();
        Sink::Preset
    };
    let mut h = Harness::builder().with_size(vec2(1000.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| editor_window(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(2);
    h.state_mut().ui.panels.gradient_editor = true;
    h.run_steps(4);
    (h, sink)
}

fn click(h: &mut Harness<PhotocraftApp>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn select(h: &mut Harness<PhotocraftApp>, location: f32, opacity: bool) {
    let rect = h.get_by_label("Gradient stops").rect();
    click(h, pos2(rect.left() + 8.0 + location * (rect.width() - 16.0), rect.top() + if opacity { 6.0 } else { 50.0 }));
}

fn type_number(h: &mut Harness<PhotocraftApp>, index: usize, value: &str, commit: bool) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(value.into()));
    h.run_steps(2);
    if commit {
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }
}

#[test]
fn top_opacity_and_bottom_color_select_their_own_fields_in_both_editor_modes() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let before = sink.fill(h.state()).unwrap();
        assert!(h.query_by_label("Color stops").is_none(), "editor has one combined strip");
        assert!(h.query_by_label("Opacity stops").is_none());
        assert!(h.get_all_by_label("Color").any(|n| n.accesskit_node().role() == egui::accesskit::Role::Button));
        select(&mut h, 0.5, true);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Opacity(1)));
        assert!(h.query_by_label("Color").is_none(), "opacity selection hides color controls");
        select(&mut h, 0.5, false);
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Color(1)));
        assert!(h.get_all_by_label("Color").any(|n| n.accesskit_node().role() == egui::accesskit::Role::Button));
        assert_eq!(sink.fill(h.state()).unwrap(), before, "selecting stops never edits the gradient");
    }
}

#[test]
fn selected_editor_opacity_commits_once_and_delete_preserves_minimum() {
    let (mut h, sink) = editor(true);
    select(&mut h, 0.5, true);
    let before = sink.fill(h.state()).unwrap();
    let history = h.state().session.active().unwrap().history.past_len();
    type_number(&mut h, 0, "35", false);
    assert_eq!(sink.fill(h.state()).unwrap(), before, "typing previews until committed");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let after = sink.fill(h.state()).unwrap();
    assert!(matches!(&after, Fill::Gradient { opacity_stops, .. } if (opacity_stops[1].1 - 0.35).abs() < 1e-5));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    h.get_by_label("Delete").click();
    h.run_steps(3);
    let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
    assert_eq!(stops.len(), 3);
    assert_eq!(opacity_stops.len(), 2);
    select(&mut h, 0.0, true);
    assert!(h.get_by_label("Delete").accesskit_node().is_disabled());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), after);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), before);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(sink.fill(h.state()).unwrap(), after);
}

#[test]
fn editor_location_follows_reordered_color_and_opens_existing_picker() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        select(&mut h, 0.0, false);
        type_number(&mut h, 0, "70", true);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Color(1)));
        assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { stops, .. } if (stops[1].0 - 0.7).abs() < 1e-5));
        h.get_all_by_label("Color").find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).unwrap().click();
        h.run_steps(3);
        assert!(!h.state().ui.dialogs.is_empty());
    }
}

#[test]
fn editor_opacity_location_changes_only_opacity_in_both_modes() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let Fill::Gradient { stops: before, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        select(&mut h, 0.0, true);
        type_number(&mut h, 1, "70", true);
        let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        assert_eq!(stops, before, "opacity location must not move a color stop");
        assert!((opacity_stops[1].0 - 0.7).abs() < 1e-5);
        let key = strip_key(h.state(), sink).with("selection");
        assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(key)), Some(Marker::Opacity(1)));
    }
}

#[test]
fn preset_editor_opacity_fields_do_not_edit_document_history() {
    let (mut h, sink) = editor(false);
    let before = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    select(&mut h, 0.5, true);
    type_number(&mut h, 0, "35", true);
    assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { opacity_stops, .. } if (opacity_stops[1].1 - 0.35).abs() < 1e-5));
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert!(matches!(sink.fill(h.state()).unwrap(), Fill::Gradient { stops, opacity_stops, .. } if stops.len() == 3 && opacity_stops.len() == 2));
    assert_eq!(h.state().session.active().unwrap().doc, before);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

/// Press at `points[0]`, move through the rest, then release at the last one; `during` runs while
/// the button is still down at the last point.
fn drag_through(h: &mut Harness<PhotocraftApp>, points: &[Pos2], during: impl FnOnce(&mut Harness<PhotocraftApp>)) {
    let (Some(&from), Some(&to)) = (points.first(), points.last()) else { return };
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for &p in &points[1..] {
        // Two small steps per leg, as a real pointer reports.
        let last = h.ctx.input(|i| i.pointer.latest_pos()).unwrap_or(p);
        h.event(egui::Event::PointerMoved(last + (p - last) * 0.5));
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
    }
    during(h);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// A stop marker's centre on the strip: colour stops under the bar, opacity stops above it.
fn marker(h: &Harness<PhotocraftApp>, location: f32, opacity: bool) -> Pos2 {
    let rect = h.get_by_label("Gradient stops").rect();
    pos2(rect.left() + 8.0 + location * (rect.width() - 16.0), rect.top() + if opacity { 6.0 } else { 50.0 })
}

fn counts(h: &Harness<PhotocraftApp>, sink: Sink) -> (usize, usize) {
    let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
    (stops.len(), opacity_stops.len())
}

/// The strip as drawn while the pointer is down: the pending edit applied to the gradient.
fn shown(h: &Harness<PhotocraftApp>, sink: Sink) -> (usize, usize) {
    let f = sink.fill(h.state()).unwrap();
    let pending = match sink {
        Sink::Layer(_) => h.state().gradient.panel.as_ref().map(|(.., p)| p.clone()),
        Sink::Preset => h.state().gradient.preset_stop.clone(),
    };
    let (fg, bg) = (h.state().session.tools.foreground, h.state().session.tools.background);
    let f = pending.and_then(|p| cmds::apply_stop(&f, &p, fg, bg).ok()).unwrap_or(f);
    let Fill::Gradient { stops, opacity_stops, .. } = f else { panic!() };
    (stops.len(), opacity_stops.len())
}

/// #10742: a colour stop dragged down off the bar (by about a marker's height) is removed on
/// release; while it is off the bar the strip already shows it gone; one undo step restores it.
#[test]
fn a_color_stop_dragged_down_off_the_bar_is_removed() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let history = h.state().session.active().unwrap().history.past_len();
        let before = sink.fill(h.state()).unwrap();
        let from = marker(&h, 0.5, false);
        drag_through(&mut h, &[from, from + vec2(0.0, 22.0)], |h| {
            assert_eq!(shown(h, sink), (2, 3), "layer {layer}: the dragged-off stop is gone from the preview");
            assert_eq!(sink.fill(h.state()).unwrap(), before, "layer {layer}: nothing is committed before release");
        });
        let Fill::Gradient { stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        assert_eq!(stops.iter().map(|s| s.0).collect::<Vec<_>>(), [0.0, 1.0], "layer {layer}: the middle stop is removed");
        assert_eq!(counts(&h, sink), (2, 3), "layer {layer}: opacity stops are untouched");
        if layer {
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1, "one undo step");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(sink.fill(h.state()).unwrap(), before);
        }
    }
}

/// Dragging a stop off the bar and back onto it before releasing cancels the removal.
#[test]
fn dragging_a_stop_back_onto_the_bar_cancels_the_removal() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        for opacity in [false, true] {
            let from = marker(&h, 0.5, opacity);
            let away = from + vec2(0.0, if opacity { -30.0 } else { 30.0 });
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(egui::Event::PointerMoved(from + (away - from) * 0.5));
            h.run_steps(1);
            h.event(egui::Event::PointerMoved(away));
            h.run_steps(1);
            assert_eq!(shown(&h, sink), if opacity { (3, 2) } else { (2, 3) }, "layer {layer}: off the bar the stop is gone");
            // Back on the bar before release: the stop returns and stays where it was.
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            assert_eq!(shown(&h, sink), (3, 3), "layer {layer}: back on the bar, the stop is shown");
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(3);
            let Fill::Gradient { stops, opacity_stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
            let locations: Vec<f32> = if opacity { opacity_stops.iter().map(|s| s.0).collect() } else { stops.iter().map(|s| s.0).collect() };
            assert_eq!((stops.len(), opacity_stops.len()), (3, 3), "layer {layer} opacity {opacity}: nothing removed");
            assert!((locations[1] - 0.5).abs() < 1e-3, "layer {layer} opacity {opacity}: {locations:?}");
        }
    }
}

/// The last two colour stops stay: dragging one off the bar leaves it where it is.
#[test]
fn the_last_two_color_stops_cannot_be_dragged_off() {
    let (mut h, sink) = editor(true);
    let from = marker(&h, 0.5, false);
    drag_through(&mut h, &[from, from + vec2(0.0, 60.0)], |_| {});
    assert_eq!(counts(&h, sink), (2, 3));
    let after = sink.fill(h.state()).unwrap();
    for location in [0.0, 1.0] {
        let from = marker(&h, location, false);
        drag_through(&mut h, &[from, from + vec2(0.0, 60.0)], |h| assert_eq!(shown(h, sink).0, 2, "the preview keeps two stops"));
        assert_eq!(sink.fill(h.state()).unwrap(), after, "two colour stops always remain");
    }
}

/// An opacity stop dragged up off the bar is removed while two remain.
#[test]
fn an_opacity_stop_dragged_up_off_the_bar_is_removed() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let from = marker(&h, 0.5, true);
        drag_through(&mut h, &[from, from - vec2(0.0, 30.0)], |h| assert_eq!(shown(h, sink), (3, 2), "layer {layer}"));
        let Fill::Gradient { opacity_stops, stops, .. } = sink.fill(h.state()).unwrap() else { panic!() };
        assert_eq!(opacity_stops.iter().map(|s| s.0).collect::<Vec<_>>(), [0.0, 1.0], "layer {layer}");
        assert_eq!(stops.len(), 3, "layer {layer}: colour stops are untouched");
        // Two opacity stops remain: the next one dragged up stays.
        let before = sink.fill(h.state()).unwrap();
        let from = marker(&h, 0.0, true);
        drag_through(&mut h, &[from, from - vec2(0.0, 30.0)], |_| {});
        assert_eq!(sink.fill(h.state()).unwrap(), before, "layer {layer}: two opacity stops always remain");
    }
}

/// Either way off the bar takes a stop off, as in the reference application: an opacity stop
/// dragged down past the colour stops and a colour stop dragged up past the opacity stops are
/// removed too.
#[test]
fn a_stop_dragged_the_other_way_comes_off_too() {
    for layer in [true, false] {
        for opacity in [true, false] {
            let (mut h, sink) = editor(layer);
            let from = marker(&h, 0.5, opacity);
            let to = from + vec2(0.0, if opacity { 70.0 } else { -70.0 });
            let gone = if opacity { (3, 2) } else { (2, 3) };
            drag_through(&mut h, &[from, to], |h| assert_eq!(shown(h, sink), gone, "layer {layer} opacity {opacity}: gone from the preview"));
            assert_eq!(counts(&h, sink), gone, "layer {layer} opacity {opacity}: the stop is removed");
        }
    }
}

/// Press on a stop, drag a little along the bar, then move to each of `then` in its own frame;
/// the last move and the release arrive in the same frame, as a quick flick does.
fn flick(h: &mut Harness<PhotocraftApp>, from: Pos2, then: &[Pos2]) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
    h.run_steps(1);
    let Some((&last, moves)) = then.split_last() else { return };
    for &p in moves {
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
    }
    // `Harness::event` runs one frame per event, so both go straight into the next frame.
    let input = &mut h.input_mut().events;
    input.push(egui::Event::PointerMoved(last));
    input.push(egui::Event::PointerButton { pos: last, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// The release frame counts: a stop flicked off the bar and released in the same frame is
/// removed, and one brought back onto the bar in the release frame is kept.
#[test]
fn the_release_frame_decides_whether_the_stop_comes_off() {
    for layer in [true, false] {
        let (mut h, sink) = editor(layer);
        let from = marker(&h, 0.5, false);
        let away = from + vec2(10.0, 40.0);
        flick(&mut h, from, &[away, from]);
        assert_eq!(counts(&h, sink), (3, 3), "layer {layer}: back on the bar at release, the stop is kept");
        let from = marker(&h, 0.5, false);
        flick(&mut h, from, &[from + vec2(10.0, 40.0)]);
        assert_eq!(counts(&h, sink), (2, 3), "layer {layer}: released off the bar in the same frame, the stop is removed");
    }
}
