//! #13870: drawing a frame shows its width and height beside the pointer, live, as the marquee
//! does, and the readout goes away on release or Escape.
use super::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Frame;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [300.0, 200.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
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
}

fn to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(Event::ModifiersChanged(m));
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
}

fn button(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, down: bool, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(2);
}

/// The readout's W and H values, if it shows: the painted rows `W:` value, `H:` value.
fn readout(h: &Harness<'static, PhotocraftApp>) -> Option<(String, String)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    h.output().shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    let w = texts.iter().position(|t| t == "W:")?;
    match texts.get(w..w + 4)? {
        [_, wv, hl, hv] if hl == "H:" => Some((wv.clone(), hv.clone())),
        _ => None,
    }
}

fn frames(app: &PhotocraftApp) -> Vec<[f64; 6]> {
    let st = app.session.active().unwrap();
    st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|f| f.transform.m)).collect()
}

#[test]
fn the_readout_follows_the_drag_and_goes_on_release() {
    let mut h = harness();
    let m = Modifiers::NONE;
    to(&mut h, 50.0, 50.0, m);
    assert_eq!(readout(&h), None, "no readout before a drag");
    button(&mut h, 50.0, 50.0, true, m);
    to(&mut h, 90.0, 80.0, m);
    assert_eq!(readout(&h), Some(("40 px".into(), "30 px".into())));
    to(&mut h, 130.0, 110.0, m);
    assert_eq!(readout(&h), Some(("80 px".into(), "60 px".into())), "updated live");
    button(&mut h, 130.0, 110.0, false, m);
    h.run_steps(2);
    assert_eq!(readout(&h), None, "gone on release");
    assert_eq!(frames(h.state()), vec![[80., 0., 0., 60., 50., 50.]], "the readout was the frame's size");
}

#[test]
fn shift_and_alt_read_the_size_the_release_makes() {
    let mut h = harness();
    let shift = Modifiers::SHIFT;
    to(&mut h, 50.0, 50.0, Modifiers::NONE);
    button(&mut h, 50.0, 50.0, true, Modifiers::NONE);
    to(&mut h, 130.0, 110.0, shift);
    let shown = readout(&h).expect("a readout");
    button(&mut h, 130.0, 110.0, false, shift);
    let f = frames(h.state())[0];
    assert_eq!(shown, (format!("{} px", f[0]), format!("{} px", f[3])), "Shift: {f:?}");
    assert_eq!(f[0], f[3], "a square");

    let mut h = harness();
    let alt = Modifiers::ALT;
    to(&mut h, 200.0, 200.0, Modifiers::NONE);
    button(&mut h, 200.0, 200.0, true, Modifiers::NONE);
    to(&mut h, 240.0, 220.0, alt);
    assert_eq!(readout(&h), Some(("80 px".into(), "40 px".into())), "Alt: twice the pointer's offset");
    button(&mut h, 240.0, 220.0, false, alt);
    assert_eq!(frames(h.state()), vec![[80., 0., 0., 40., 160., 180.]]);
}

#[test]
fn the_readout_goes_when_escape_ends_the_drag() {
    let mut h = harness();
    let m = Modifiers::NONE;
    to(&mut h, 50.0, 50.0, m);
    button(&mut h, 50.0, 50.0, true, m);
    to(&mut h, 130.0, 110.0, m);
    assert!(readout(&h).is_some());
    h.event(Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.run_steps(3);
    assert_eq!(readout(&h), None);
}

#[test]
fn show_transformation_values_off_hides_it_as_for_the_other_readouts() {
    let mut h = harness();
    h.state_mut().session.prefs.edit(|p| p.tools.show_transformation_values = false);
    let m = Modifiers::NONE;
    to(&mut h, 50.0, 50.0, m);
    button(&mut h, 50.0, 50.0, true, m);
    to(&mut h, 130.0, 110.0, m);
    assert_eq!(readout(&h), None);
}
