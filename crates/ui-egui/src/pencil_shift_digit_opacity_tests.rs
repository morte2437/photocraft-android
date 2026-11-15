//! #11967: the Pencil has no Flow, so ⇧ plus a digit sets its Opacity as a plain digit does
//! (⇧3 = 30%, ⇧0 = 100%, two quick ⇧ digits an exact value). The Brush's ⇧ digits still set Flow,
//! swapped with Opacity while airbrush build-up is on.

use serde_json::json;

use crate::PhotocraftApp;
use crate::opacity_keys::press;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

/// The brush settings' (opacity, flow) in percent.
fn brush(app: &PhotocraftApp) -> (f32, f32) {
    let b = &app.session.tools.brush;
    ((b.opacity * 100.0).round(), (b.flow * 100.0).round())
}

#[test]
fn shift_digits_set_the_pencils_opacity() {
    let mut a = app(Tool::Pencil);
    let flow = brush(&a).1;
    press(&mut a, 3, true, 0.0);
    assert_eq!(brush(&a), (30.0, flow), "⇧3");
    press(&mut a, 0, true, 5000.0);
    assert_eq!(brush(&a), (100.0, flow), "⇧0");
    for d in 1..=9u8 {
        press(&mut a, d, true, 10000.0 * f64::from(d));
        assert_eq!(brush(&a), (f32::from(d) * 10.0, flow), "⇧{d}");
    }
}

#[test]
fn two_quick_shift_digits_set_an_exact_opacity() {
    let mut a = app(Tool::Pencil);
    press(&mut a, 4, true, 0.0);
    press(&mut a, 5, true, 300.0);
    assert_eq!(brush(&a).0, 45.0);
    press(&mut a, 0, true, 5000.0);
    press(&mut a, 7, true, 5300.0);
    assert_eq!(brush(&a).0, 7.0);
    // As a plain `00`, which is 100 % for the Pencil (#11812). Joined, typed slowly or with the
    // second press ignored, ⇧0 ⇧0 ends at 100 % either way.
    press(&mut a, 0, true, 10000.0);
    press(&mut a, 0, true, 10300.0);
    assert_eq!(brush(&a).0, 100.0);
}

/// No digit, plain or ⇧, ever changes the Flow while the Pencil is active, build-up on or off.
#[test]
fn the_pencils_flow_stays_untouched() {
    for build_up in [false, true] {
        let mut a = app(Tool::Pencil);
        a.run("tools.setBrush", json!({"brush": {"flow": 0.42}})).unwrap();
        a.session.tools.brush.build_up = build_up;
        for (i, (d, shift)) in [(3, true), (6, false), (0, true), (8, false)].into_iter().enumerate() {
            press(&mut a, d, shift, 10000.0 * i as f64);
            assert_eq!(brush(&a).1, 42.0, "build-up {build_up}, digit {d}, ⇧ {shift}");
            assert_eq!(a.session.tools.brush.build_up, build_up, "the case under test holds");
        }
        assert_eq!(brush(&a).0, 80.0, "build-up {build_up}: the last digit set the Opacity");
    }
}

#[test]
fn the_brushs_shift_digits_still_set_flow() {
    let mut a = app(Tool::Brush);
    press(&mut a, 3, true, 0.0);
    assert_eq!(brush(&a), (100.0, 30.0), "⇧3 is Flow");
    // With airbrush build-up the roles swap: ⇧ digits set Opacity, plain ones Flow.
    a.session.tools.brush.build_up = true;
    press(&mut a, 6, true, 5000.0);
    press(&mut a, 2, false, 10000.0);
    assert_eq!(brush(&a), (60.0, 20.0));
}
