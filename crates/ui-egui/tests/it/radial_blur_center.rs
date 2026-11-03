//! Radial Blur's Zoom diagram shares the dialog's centre coordinates with its sliders,
//! numeric fields, automation, preview and final command. Uses the real dialog through
//! the public app/control APIs, like custom_print_paper.rs; no GPU adapter is needed.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2, accesskit::Role, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Outcome, handle},
    dialogs, filter_dialog, i18n,
    theme::ThemeKind,
};
use serde_json::{Map, Value, json};

const COMMAND: &str = "filter.blur.radialBlur";

fn harness() -> Harness<'static, PhotocraftApp> {
    themed_harness(ThemeKind::Pro, vec2(1000.0, 800.0))
}

fn themed_harness(theme: ThemeKind, size: Vec2) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("paint.stroke", json!({"points": [[8, 8, 1]], "size": 4, "color": "#ff8000"})).unwrap();
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app| {
            let _language = i18n::language_scope(i18n::Lang::EN);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    control(&mut h, "ui.menu.invoke", json!({"id": COMMAND}));
    let id = h.state().ui.dialogs.last().unwrap().id;
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "method", "value": "zoom"}));
    h
}

fn control(h: &mut Harness<'_, PhotocraftApp>, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (request, _rx) = ControlRequest::new(method, params);
    let result = handle(h.state_mut(), &ctx, &request);
    assert!(matches!(result, Outcome::Done(v) if v["ok"] == true), "{method} succeeds");
    h.run_steps(3);
}

fn fields<'a>(h: &'a Harness<'_, PhotocraftApp>) -> &'a Map<String, Value> {
    &h.state().ui.dialogs.last().unwrap().fields
}

fn center(h: &Harness<'_, PhotocraftApp>) -> [f64; 2] {
    [fields(h)["centerX"].as_f64().unwrap(), fields(h)["centerY"].as_f64().unwrap()]
}

fn diagram(h: &Harness<'_, PhotocraftApp>) -> Rect {
    h.get_by_role_and_label(Role::Slider, "Center").rect()
}

fn pointer(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn click(h: &mut Harness<'_, PhotocraftApp>, x: f32, y: f32) {
    let plot = diagram(h).shrink(4.0);
    let pos = plot.min + vec2(x, y) * plot.size();
    pointer(h, pos, true);
    pointer(h, pos, false);
    h.run_steps(2);
}

fn marker(shape: &Shape, rect: Rect) -> Option<Pos2> {
    match shape {
        Shape::Circle(c) if c.radius == 3.0 && rect.contains(c.center) => Some(c.center),
        Shape::Vec(shapes) => shapes.iter().find_map(|s| marker(s, rect)),
        _ => None,
    }
}

fn assert_marker(h: &Harness<'_, PhotocraftApp>, x: f32, y: f32) {
    let rect = diagram(h);
    let plot = rect.shrink(4.0);
    let expected = plot.min + vec2(x, y) * plot.size();
    let actual = h.output().shapes.iter().find_map(|s| marker(&s.shape, rect)).expect("painted centre marker");
    assert!(actual.distance(expected) < 0.1, "marker at {actual:?}, expected {expected:?}");
}

#[test]
fn clicks_and_drags_update_both_coordinates_and_clamp_at_the_edges() {
    let mut h = harness();
    click(&mut h, 0.25, 0.75);
    assert_eq!(center(&h), [0.25, 0.75]);
    assert_marker(&h, 0.25, 0.75);
    let plot = diagram(&h).shrink(4.0);
    pointer(&mut h, plot.min + plot.size() * 0.5, true);
    assert_eq!(center(&h), [0.5, 0.5], "the press itself moves the centre");
    for (point, expected) in [(vec2(0.8, 0.2), [0.8, 0.2]), (vec2(1.4, -0.4), [1.0, 0.0]), (vec2(-0.4, 1.4), [0.0, 1.0])] {
        h.event(Event::PointerMoved(plot.min + plot.size() * point));
        h.step();
        assert_eq!(center(&h), expected, "updates during the drag, before release");
        assert_marker(&h, expected[0] as f32, expected[1] as f32);
    }
    pointer(&mut h, plot.left_bottom(), false);
    h.run_steps(2);
    assert_eq!(center(&h), [0.0, 1.0]);
}

#[test]
fn slider_and_numeric_edits_move_the_diagram() {
    let mut h = harness();
    let slider = h.get_by_role_and_label(Role::Slider, "Center X").rect();
    let at = |fraction| pos2(slider.left() + 7.0 + (slider.width() - 14.0) * fraction, slider.center().y);
    pointer(&mut h, at(0.15), true);
    h.event(Event::PointerMoved(at(0.8)));
    h.step();
    pointer(&mut h, at(0.8), false);
    h.run_steps(2);
    assert_eq!(center(&h), [0.8, 0.5]);
    assert_marker(&h, 0.8, 0.5);
    h.get_all_by_role(Role::SpinButton).nth(2).expect("Center Y numeric field").click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("0.23".into()));
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert_eq!(center(&h), [0.8, 0.23]);
    assert_marker(&h, 0.8, 0.23);
}

#[test]
fn keyboard_steps_and_automation_share_the_diagram_coordinates() {
    let mut h = harness();
    click(&mut h, 1.0, 0.0);
    h.key_press(Key::ArrowRight);
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(center(&h), [1.0, 0.0]);
    h.key_press(Key::ArrowLeft);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(center(&h), [0.99, 0.01]);
    let id = h.state().ui.dialogs.last().unwrap().id;
    for (field, value) in [("centerX", 0.12), ("centerY", 0.87)] {
        control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    }
    h.run_steps(3);
    assert_marker(&h, 0.12, 0.87);
}

#[test]
fn only_zoom_has_the_diagram_and_switching_methods_keeps_the_center() {
    let mut h = harness();
    click(&mut h, 0.2, 0.7);
    for method in ["spin", "zoom"] {
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("method".into(), json!(method));
        h.run_steps(3);
        assert_eq!(h.query_by_role_and_label(Role::Slider, "Center").is_some(), method == "zoom");
        let [x, y] = center(&h);
        assert!((x - 0.2).abs() < 1e-6 && (y - 0.7).abs() < 1e-6);
        assert_eq!(h.get_all_by_role(Role::SpinButton).count(), 3, "no duplicated centre controls");
    }
    let id = h.state().ui.dialogs.last().unwrap().id;
    control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    control(&mut h, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
    assert!(h.query_by_role_and_label(Role::Slider, "Center").is_none(), "other filters keep their existing controls");
}

#[test]
fn editing_the_diagram_is_only_a_preview_until_confirmed() {
    let mut h = harness();
    let revision = h.state().session.active().unwrap().revision;
    click(&mut h, 0.25, 0.75);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    let before = h.state().session.active().unwrap().doc.clone();
    let params = filter_dialog::params_of(fields(&h));
    let active_layer = h.state().session.active().unwrap().active_layer;
    let expected = filter_dialog::preview_document(&before, active_layer, COMMAND, &params, 1).unwrap();
    let id = h.state().ui.dialogs.last().unwrap().id;
    control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
    let actual = &h.state().session.active().unwrap().doc;
    assert_eq!(photocraft_compose::render(actual, actual.bounds()), photocraft_compose::render(&expected, expected.bounds()));
    assert!(h.state().session.active().unwrap().revision > revision);
    control(&mut h, "ui.menu.invoke", json!({"id": COMMAND}));
    let id = h.state().ui.dialogs.last().unwrap().id;
    let revision = h.state().session.active().unwrap().revision;
    click(&mut h, 0.8, 0.2);
    control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
}

#[test]
fn hostile_coordinates_are_bounded_before_drawing() {
    let mut h = harness();
    for (x, y, expected) in [(json!(1e100), json!(-1e100), [1.0, 0.0]), (json!("invalid"), Value::Null, [0.5, 0.5])] {
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        fields.insert("centerX".into(), x);
        fields.insert("centerY".into(), y);
        h.run_steps(3);
        assert_eq!(center(&h), expected);
        assert_marker(&h, expected[0] as f32, expected[1] as f32);
    }
}

#[test]
fn diagram_and_sliders_fit_every_theme_and_amount_changes_the_rays() {
    for theme in ThemeKind::ALL {
        for width in [560.0, 1000.0] {
            let mut h = themed_harness(theme, vec2(width, 800.0));
            click(&mut h, 0.25, 0.75);
            let diagram = h.get_by_role_and_label(Role::Slider, "Center").rect();
            let viewport = Rect::from_min_size(Pos2::ZERO, vec2(width, 800.0));
            assert!(viewport.contains_rect(diagram), "{theme:?}: the diagram fits at {width} pt");
            for label in ["Center X", "Center Y"] {
                let slider = h.get_by_role_and_label(Role::Slider, label).rect();
                assert!(slider.left() > diagram.right(), "{theme:?}: {label} is beside the diagram");
                assert!(viewport.contains_rect(slider), "{theme:?}: {label} fits at {width} pt");
            }
            let rays = |h: &Harness<'_, PhotocraftApp>| {
                h.output()
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        Shape::LineSegment { points, .. } if points.iter().all(|p| diagram.contains(*p)) => Some(*points),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let before = rays(&h);
            assert!(!before.is_empty(), "the diagram paints the streaks");
            let id = h.state().ui.dialogs.last().unwrap().id;
            control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "amount", "value": 75}));
            assert_ne!(before, rays(&h), "Amount changes the schematic streaks");
            assert_eq!(center(&h), [0.25, 0.75]);
        }
    }
}
