//! An error notice's message can be copied (#7807): text dragged across in it is copied by ⌘C,
//! as egui-winit sends it (`Event::Copy`) and as the Mac menu's key equivalent sends it (a ⌘C
//! press), instead of Edit › Copy copying the layer; the drag never reaches the canvas. A plain
//! click on the text leaves ⌘C to the document. The whole app (eframe harness) with the app's own
//! `raw_input_hook`, which kittest doesn't run, so the clipboard keys are mapped as on the desktop.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The refusal the report quotes, as the notice shows it.
const MESSAGE: &str = "Free Transform: not enough memory for transforming pixels (the transformed layer \"Layer 1\" would be 37708 x 35259 px, more than the 1073741824 px limit); the document was not changed";

/// The whole app with a filled layer on a 2000 x 1500 document, the Move tool, and the refusal in
/// an error notice.
fn app_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(4);
    crate::notices::error(h.state_mut(), MESSAGE.to_string());
    h.run_steps(3);
    h
}

/// The notice's message (the status bar shows it too, on one line: the notice wraps it).
fn notice_text(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    let rects: Vec<egui::Rect> = h.get_all_by_label(MESSAGE).map(|n| n.rect()).collect();
    rects.into_iter().max_by(|a, b| a.height().total_cmp(&b.height())).unwrap()
}

/// The document's layers (bounds) and History length.
fn document(h: &Harness<'static, PhotocraftApp>) -> (Vec<Option<photocraft_geom::Rect>>, usize) {
    let st = h.state().session.active().unwrap();
    (st.doc.walk().iter().map(|(_, _, l)| l.surface().map(|s| s.content_bounds())).collect(), st.history.past_len())
}

/// One frame with `events`, through the app's `raw_input_hook` as on the desktop; the text it
/// copied.
fn frame(h: &mut Harness<'static, PhotocraftApp>, events: Vec<egui::Event>) -> Vec<String> {
    let ctx = h.ctx.clone();
    let mut raw = h.input_mut().clone();
    raw.events.extend(events);
    eframe::App::raw_input_hook(h.state_mut(), &ctx, &mut raw);
    *h.input_mut() = raw;
    h.step();
    h.output().platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect()
}

/// The primary button pressed or released at `p`, then a frame without events.
fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool) {
    frame(h, vec![egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    frame(h, vec![]);
}

/// A primary-button drag from `from` to `to` in ten moves.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    frame(h, vec![egui::Event::PointerMoved(from)]);
    button(h, from, true);
    for i in 1..=10 {
        frame(h, vec![egui::Event::PointerMoved(from + (to - from) * (i as f32 / 10.0))]);
    }
    button(h, to, false);
}

/// A ⌘C press, as the Mac menu's key equivalent sends it.
fn command_c() -> egui::Event {
    egui::Event::Key { key: egui::Key::C, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }
}

/// Text dragged across in the notice is what ⌘C copies, as a Copy event or a ⌘C press; the drag
/// and the copy leave the layers, History and the session clipboard alone.
#[test]
fn copy_takes_the_text_dragged_across_in_an_error_notice_not_the_layer() {
    for (how, copy) in [("Event::Copy", egui::Event::Copy), ("a ⌘C press", command_c())] {
        let mut h = app_harness();
        let before = document(&h);
        let r = notice_text(&h);
        drag(&mut h, r.left_top() + vec2(2.0, 6.0), r.right_bottom() - vec2(2.0, 6.0));
        assert_eq!(document(&h), before, "{how}: the drag across the notice stays off the canvas");
        let copied = frame(&mut h, vec![copy]);
        frame(&mut h, vec![]);
        assert!(copied.iter().any(|t| t.contains("1073741824 px limit")), "{how}: the selected text is copied: {copied:?}");
        assert!(h.state().session.clipboard.is_none(), "{how}: Edit › Copy didn't copy the layer");
        assert_eq!(document(&h), before, "{how}");
    }
}

/// The selection outlives the pointer: after the drag the pointer moves off the notice and over
/// the canvas without a press, and ⌘C still copies the text, not the layer.
#[test]
fn copy_takes_the_selected_text_after_the_pointer_leaves_the_notice() {
    for (how, copy) in [("Event::Copy", egui::Event::Copy), ("a ⌘C press", command_c())] {
        let mut h = app_harness();
        let before = document(&h);
        let r = notice_text(&h);
        drag(&mut h, r.left_top() + vec2(2.0, 6.0), r.right_bottom() - vec2(2.0, 6.0));
        let canvas = Pos2::new(500.0, 400.0);
        assert!(!r.contains(canvas));
        for i in 1..=10 {
            frame(&mut h, vec![egui::Event::PointerMoved(r.right_bottom() + (canvas - r.right_bottom()) * (i as f32 / 10.0))]);
        }
        let copied = frame(&mut h, vec![copy]);
        frame(&mut h, vec![]);
        assert!(copied.iter().any(|t| t.contains("1073741824 px limit")), "{how}: the selected text is copied: {copied:?}");
        assert!(h.state().session.clipboard.is_none(), "{how}: Edit › Copy didn't copy the layer");
        assert_eq!(document(&h), before, "{how}");
    }
}

/// A plain click on the notice's text selects nothing, so ⌘C stays Edit › Copy and copies the
/// layer.
#[test]
fn a_plain_click_on_a_notice_leaves_copy_to_the_document() {
    let mut h = app_harness();
    let p = notice_text(&h).center();
    frame(&mut h, vec![egui::Event::PointerMoved(p)]);
    button(&mut h, p, true);
    button(&mut h, p, false);
    let copied = frame(&mut h, vec![egui::Event::Copy]);
    frame(&mut h, vec![]);
    assert!(copied.is_empty(), "{copied:?}");
    assert!(h.state().session.clipboard.is_some(), "Edit › Copy copies the layer");
}
