//! With no document open, double-clicking the empty workspace shows File › Open, as in Photoshop
//! (#5367): with Auto Show the Home Screen on (the Home screen's blank background) and off (the
//! plain workspace). A single click does nothing, and the Home screen's own buttons keep theirs.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::file_dialog::{FileDialogReply, FileDialogRequest};
use crate::{PhotocraftApp, Services};

type Requests = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

fn harness(home: bool) -> (Harness<'static, PhotocraftApp>, Requests) {
    let requests: Requests = Default::default();
    let held = requests.clone();
    let services = Services { file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.edit_prefs(|p| p.general.auto_show_home_screen = home);
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // Fonts set up after the first frame only apply from the next one.
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            app.poll_file_dialog(&ctx, None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    (h, requests)
}

fn button(h: &mut Harness<'static, PhotocraftApp>, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// `count` clicks two frames apart at `at`, after letting the clock move past earlier clicks.
fn clicks(h: &mut Harness<'static, PhotocraftApp>, at: Pos2, count: usize) {
    h.hover_at(at);
    for _ in 0..40 {
        h.step();
    }
    for _ in 0..count {
        button(h, at, true);
        h.step();
        button(h, at, false);
        h.step();
    }
    h.run_steps(3);
}

fn opens(requests: &Requests) -> usize {
    requests.borrow().iter().filter(|(r, _)| matches!(r, FileDialogRequest::Open { multiple: true, .. })).count()
}

#[test]
fn double_clicking_the_empty_workspace_shows_file_open() {
    for home in [false, true] {
        let (mut h, requests) = harness(home);
        assert!(h.state().session.documents().is_empty());
        // Well away from the Home screen's centred card.
        let blank = pos2(40.0, 760.0);
        clicks(&mut h, blank, 1);
        assert_eq!(opens(&requests), 0, "home {home}: a single click opens nothing");
        clicks(&mut h, blank, 2);
        assert_eq!(opens(&requests), 1, "home {home}: a double-click shows File › Open");
    }
}

#[test]
fn the_home_screens_own_widgets_keep_their_double_clicks() {
    let (mut h, requests) = harness(true);
    // The Home rail button: a double-click there stays on Home, not File › Open.
    let at = h.get_by_label_contains("Home").rect().center();
    clicks(&mut h, at, 2);
    assert_eq!(opens(&requests), 0, "the rail item took the double-click");
    // The New file button opens the New Document dialog, not File › Open.
    let at = h.get_by_label_contains("New file").rect().center();
    clicks(&mut h, at, 1);
    assert_eq!(opens(&requests), 0);
    assert_eq!(h.state().ui.dialogs.len(), 1, "the New Document dialog");
}
