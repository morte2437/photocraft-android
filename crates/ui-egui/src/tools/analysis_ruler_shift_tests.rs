//! Shift constrains the Ruler to 45° steps without making it longer (#11985): on a diagonal step
//! both legs take min(|dx|, |dy|), on a 0° or 90° step the dominant component stays and the other
//! is 0; the same for an endpoint moved around the other one and for a protractor's second arm.

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use photocraft_engine::doc::Ruler;

fn ruler_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Ruler;
    app
}

/// A drag from `from` to `to` with `m` held during the move.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, m: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn ruler(app: &PhotocraftApp) -> Ruler {
    app.session.active().unwrap().doc.measurement.ruler.expect("a ruler")
}

fn info(app: &PhotocraftApp) -> Value {
    photocraft_engine::analysis_cmds::ruler_info(&ruler(app), &Default::default())
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn drawing_with_shift_keeps_the_shorter_leg_on_diagonals_and_the_dominant_one_on_axes() {
    // (pointer, constrained end, L1 when measured), all from 100,100.
    for (to, end, l1) in [
        ([182.0, 200.0], [182.0, 182.0], Some(115.97)),
        ([222.0, 220.0], [220.0, 220.0], None),
        ([400.0, 130.0], [400.0, 100.0], None),
        // Signs are kept: dx −82, dy −100.
        ([18.0, 0.0], [18.0, 18.0], None),
    ] {
        let mut app = ruler_app();
        drag(&mut app, [100.0, 100.0], to, Modifiers::NONE, Modifiers::SHIFT);
        let r = ruler(&app);
        assert_eq!(r.start, [100.0, 100.0]);
        assert!(near(r.end[0], end[0]) && near(r.end[1], end[1]), "to {to:?}: end {:?}, expected {end:?}", r.end);
        if let Some(l1) = l1 {
            let i = info(&app);
            assert!(near(i["w"].as_f64().unwrap().abs(), 82.0) && near(i["h"].as_f64().unwrap().abs(), 82.0), "{i}");
            assert!(near(i["l1"].as_f64().unwrap(), l1), "{i}");
        }
        assert_eq!(app.session.active().unwrap().history.past_len(), 0, "no history step");
    }
}

#[test]
fn moving_an_endpoint_with_shift_snaps_around_the_other_one() {
    // The end point, around the start.
    let mut app = ruler_app();
    drag(&mut app, [100.0, 100.0], [300.0, 250.0], Modifiers::NONE, Modifiers::NONE);
    drag(&mut app, [300.0, 250.0], [382.0, 160.0], Modifiers::NONE, Modifiers::SHIFT);
    let r = ruler(&app);
    assert_eq!((r.start, r.end), ([100.0, 100.0], [382.0, 100.0]), "W 282, H 0");
    // The start point, around the end.
    let mut app = ruler_app();
    drag(&mut app, [100.0, 100.0], [400.0, 300.0], Modifiers::NONE, Modifiers::NONE);
    drag(&mut app, [100.0, 100.0], [118.0, 240.0], Modifiers::NONE, Modifiers::SHIFT);
    let r = ruler(&app);
    assert_eq!((r.start, r.end), ([118.0, 300.0], [400.0, 300.0]), "W 282, H 0");
}

#[test]
fn a_protractor_arm_with_shift_keeps_the_shorter_leg() {
    let mut app = ruler_app();
    drag(&mut app, [400.0, 300.0], [600.0, 300.0], Modifiers::NONE, Modifiers::NONE);
    // Shift+Alt-drag from the vertex: pointer at +62, −80.
    drag(&mut app, [400.0, 300.0], [462.0, 220.0], Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT);
    let arm = ruler(&app).protractor.expect("a protractor");
    assert!(near(arm[0], 462.0) && near(arm[1], 238.0), "{arm:?}");
    let i = info(&app);
    assert!(near(i["l2"].as_f64().unwrap(), 87.68), "{i}");
    // Shift-drag that arm end to dx 122, dy −170 from the vertex.
    drag(&mut app, [462.0, 238.0], [522.0, 130.0], Modifiers::NONE, Modifiers::SHIFT);
    let arm = ruler(&app).protractor.unwrap();
    assert!(near(arm[0], 522.0) && near(arm[1], 178.0), "{arm:?}");
    assert!(near(info(&app)["l2"].as_f64().unwrap(), 172.53));
}

#[test]
fn without_shift_the_endpoint_follows_the_pointer() {
    let mut app = ruler_app();
    drag(&mut app, [100.0, 100.0], [182.0, 200.0], Modifiers::NONE, Modifiers::NONE);
    assert_eq!(ruler(&app).end, [182.0, 200.0]);
}
