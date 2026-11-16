//! #13867: holding Space while a frame is drawn moves the frame being drawn, as the marquee does;
//! after Space is released the drag sizes again from the moved start.
use super::*;
use crate::canvas::tool_event;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

fn entries(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

fn frames(app: &PhotocraftApp) -> Vec<[f64; 6]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.transform.m)).collect()
}

fn mv(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}

/// The measured sequence: press at (50,50), drag to (122,120), hold Space and move +50,+30,
/// release Space, release the button at (202,180).
#[test]
fn the_measured_sequence_makes_100_202_by_80_180_in_one_step() {
    let mut app = app();
    let before = entries(&app);
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 122., 120., m);
    assert!(set_space(&mut app, true), "a frame is being drawn");
    mv(&mut app, 147., 135., m);
    mv(&mut app, 172., 150., m);
    set_space(&mut app, false);
    mv(&mut app, 202., 180., m);
    tool_event(&mut app, ToolEvent::Up { x: 202., y: 180. }, m);
    assert_eq!(frames(&app), vec![[102., 0., 0., 100., 100., 80.]], "100..202 x 80..180");
    let after = entries(&app);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().map(String::as_str), Some("New Frame"));
}

#[test]
fn released_with_space_down_the_frame_keeps_its_size() {
    let mut app = app();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 150., 110., m);
    set_space(&mut app, true);
    mv(&mut app, 200., 150., m);
    tool_event(&mut app, ToolEvent::Up { x: 230., y: 170. }, m);
    set_space(&mut app, false);
    assert_eq!(frames(&app), vec![[100., 0., 0., 60., 130., 110.]], "the 100 x 60 frame moved by (80, 60)");
}

#[test]
fn space_moves_a_square_and_a_centred_frame_too() {
    // Shift: a square from the moved start.
    let mut app = app();
    let shift = Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, shift);
    mv(&mut app, 130., 110., shift);
    set_space(&mut app, true);
    mv(&mut app, 150., 120., shift);
    set_space(&mut app, false);
    tool_event(&mut app, ToolEvent::Up { x: 150., y: 120. }, shift);
    let moved = frames(&app);
    // The same as a Shift drag from the moved start (70,60) to the release (150,120).
    let mut plain = self::app();
    tool_event(&mut plain, ToolEvent::Down { x: 70., y: 60., pressure: 1.0 }, shift);
    mv(&mut plain, 150., 120., shift);
    tool_event(&mut plain, ToolEvent::Up { x: 150., y: 120. }, shift);
    assert_eq!(moved, frames(&plain));
    assert_eq!(moved, vec![[60., 0., 0., 60., 70., 60.]], "a square from (70,60)");
    // Alt: centred on the moved centre.
    let mut app = self::app();
    let alt = Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 200., y: 200., pressure: 1.0 }, alt);
    mv(&mut app, 240., 220., alt);
    set_space(&mut app, true);
    mv(&mut app, 260., 230., alt);
    set_space(&mut app, false);
    tool_event(&mut app, ToolEvent::Up { x: 260., y: 230. }, alt);
    assert_eq!(frames(&app), vec![[80., 0., 0., 40., 180., 190.]], "80 x 40 around (220,210)");
}

#[test]
fn without_space_today_s_drag_stays() {
    let mut app = app();
    let m = Modifiers::NONE;
    assert!(!set_space(&mut app, false), "no frame being drawn");
    tool_event(&mut app, ToolEvent::Down { x: 50., y: 50., pressure: 1.0 }, m);
    mv(&mut app, 122., 120., m);
    mv(&mut app, 202., 180., m);
    tool_event(&mut app, ToolEvent::Up { x: 202., y: 180. }, m);
    assert_eq!(frames(&app), vec![[152., 0., 0., 130., 50., 50.]]);
}

/// Through the real canvas: Space during the drag repositions instead of panning with the Hand.
#[test]
fn space_during_a_frame_drag_is_not_the_hand_tool() {
    let mut a = app();
    a.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        a,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [300.0, 200.0];
    v.fit_pending = false;
    h.run_steps(2);
    let screen = |h: &Harness<'static, PhotocraftApp>, x: f32, y: f32| -> Pos2 {
        let app = h.state();
        let v = &app.ui.views[0];
        let xf = crate::canvas::ViewXform {
            rect: crate::rulers::content_rect(app, app.last_canvas_rect),
            zoom: v.zoom,
            center: v.center,
            flip: app.ui.view.flip_horizontal,
            rotation: v.rotation,
            aspect: app.ui.view.display_aspect(),
        };
        xf.to_screen(x, y)
    };
    let to = |h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32| {
        let p = screen(h, x, y);
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
    };
    let space = |h: &mut Harness<'static, PhotocraftApp>, down: bool| {
        h.event(Event::Key { key: Key::Space, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    };
    to(&mut h, 50.0, 50.0);
    let p = screen(&h, 50.0, 50.0);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    to(&mut h, 90.0, 80.0);
    to(&mut h, 122.0, 120.0);
    space(&mut h, true);
    to(&mut h, 172.0, 150.0);
    space(&mut h, false);
    to(&mut h, 202.0, 180.0);
    let center = h.state().ui.views[0].center;
    let p = screen(&h, 202.0, 180.0);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].center, center, "Space didn't pan");
    assert_eq!(h.state().ui.tool, Tool::Frame);
    let f = frames(h.state());
    assert_eq!(f.len(), 1, "{f:?}");
    let [w, _, _, ht, x, y] = f[0];
    assert!((x - 100.).abs() <= 1. && (y - 80.).abs() <= 1. && (x + w - 202.).abs() <= 1. && (y + ht - 180.).abs() <= 1., "{:?}", f[0]);
}
