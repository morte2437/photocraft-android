//! Hidden-tick and synthetic-input lifecycle regressions.
//!
//! The host runs `App::logic` without a UI pass for hidden/occluded windows; eframe preserves the
//! raw input appended by `raw_input_hook` until a real pass consumes it. These tests exercise that
//! order without a renderer: at most one synthetic batch may be in flight, a pass that returns
//! early must not lose it (and must not deliver it twice), and a batch queued for one document
//! must not be replayed into another.

use crate::{PhotocraftApp, Services};
use eframe::App as _;
use egui::{Context, Event, Key, Modifiers, RawInput};
use photocraft_engine::Session;
use serde_json::json;

fn app_with_doc() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).expect("a document");
    app
}

fn press(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn release(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE }
}

fn hook(app: &mut PhotocraftApp, ctx: &Context) -> Vec<Event> {
    let mut raw = RawInput::default();
    app.inject_synthetic(ctx, &mut raw);
    raw.events
}

/// Hidden logic ticks still receive the current viewport's native geometry. The default ROOT
/// entry has no geometry, so it cannot represent a window with a capturable size.
fn host_input(viewport: egui::ViewportId) -> RawInput {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let mut raw = RawInput { viewport_id: viewport, screen_rect: Some(rect), focused: true, ..Default::default() };
    raw.viewports
        .insert(viewport, egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(1.0), minimized: Some(false), ..Default::default() });
    raw
}

#[test]
fn one_batch_at_a_time_across_hidden_ticks() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::V), release(Key::V), press(Key::W), release(Key::W)]);
    // The first hidden tick injects the first press/release pair.
    let first = hook(&mut app, &ctx);
    assert_eq!(first.len(), 2, "one step: the press and its release");
    // Further hidden ticks must not drain the next step into eframe's preserved buffer.
    assert!(hook(&mut app, &ctx).is_empty());
    assert!(hook(&mut app, &ctx).is_empty());
    // A real UI pass consumes the batch; only then may the next step start.
    app.begin_synthetic_pass(&ctx);
    app.finish_synthetic_pass();
    let next = hook(&mut app, &ctx);
    assert_eq!(next.len(), 2, "the second step starts after the pass");
}

#[test]
fn an_early_returning_pass_reinjects_the_batch_exactly_once() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::A)]);
    assert_eq!(hook(&mut app, &ctx).len(), 1);
    // A pass began (egui consumed the events), then returned before handling input: fonts not
    // ready. Keep the same batch and put it back on the next hook.
    app.begin_synthetic_pass(&ctx);
    app.defer_synthetic_pass();
    let again = hook(&mut app, &ctx);
    assert_eq!(again.len(), 1, "the same step is delivered again");
    assert!(matches!(again[0], Event::Key { key: Key::A, pressed: true, .. }));
    // A second hidden tick before the UI pass must not append the step a second time.
    assert!(hook(&mut app, &ctx).is_empty(), "the reinjection is consumed once");
    app.begin_synthetic_pass(&ctx);
    app.finish_synthetic_pass();
    assert!(hook(&mut app, &ctx).is_empty(), "then it is delivered once and done");
}

#[test]
fn a_document_change_before_injection_drops_the_batch_and_replies() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::V), release(Key::V)]);
    assert!(!hook(&mut app, &ctx).is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    app.input_waiters.push(tx);
    // The document changed while the events were still in flight: the next hook sees it before
    // the host consumes them, so it withdraws the batch and fails the waiting reply.
    app.run("file.new", json!({"width": 16, "height": 16})).expect("a second document");
    assert!(hook(&mut app, &ctx).is_empty(), "nothing is delivered to the new document");
    assert!(app.synthetic_batch.is_none(), "the stale batch is dropped");
    let reply = rx.recv().expect("the waiting control request gets an answer");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("document or focus changed")));
}

#[test]
fn a_document_change_inside_the_pass_is_delivered_and_not_reported_as_cancelled() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::V), release(Key::V)]);
    assert!(!hook(&mut app, &ctx).is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    app.input_waiters.push(tx);
    // `logic` opens another document after the hook consumed the events into this pass. They
    // cannot be withdrawn now. Treat the batch as delivered (automation-gated) rather than
    // failing the reply while the input still lands.
    app.run("file.new", json!({"width": 16, "height": 16})).expect("a second document");
    app.begin_synthetic_pass(&ctx);
    assert!(app.automation_input, "the delivered batch keeps the automation policy");
    assert!(app.synthetic_batch.is_some(), "the pass still owns the batch until it finishes");
    assert!(rx.try_recv().is_err(), "no cancellation reply for input that was delivered");
    app.finish_synthetic_pass();
    assert!(app.synthetic_batch.is_none());
}

#[test]
fn a_preserved_batch_is_withdrawn_from_the_raw_input_on_a_document_change() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::V), release(Key::V)]);
    let first = hook(&mut app, &ctx);
    assert_eq!(first.len(), 2);
    // A hidden logic tick preserved the events for the next pass; meanwhile the document changed.
    app.run("file.new", json!({"width": 16, "height": 16})).expect("a second document");
    let mut preserved = RawInput { events: first, ..Default::default() };
    app.inject_synthetic(&ctx, &mut preserved);
    assert!(preserved.events.is_empty(), "the stale events are withdrawn from preserved input");
    assert!(app.synthetic_batch.is_none());
}

#[test]
fn an_edit_in_the_same_document_does_not_cancel_the_queued_batch() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::L), release(Key::L)]);
    assert!(!hook(&mut app, &ctx).is_empty());
    let (tx, rx) = std::sync::mpsc::channel();
    app.input_waiters.push(tx);
    // A command edits the same document while the step is in flight: the revision changes, the
    // document activation does not, so the step is not cancelled.
    app.run("layer.new.layer", json!({"name": "queued"})).expect("an edit");
    let mut preserved = RawInput::default();
    app.inject_synthetic(&ctx, &mut preserved);
    assert!(app.synthetic_batch.is_some(), "the batch for this document survives an edit");
    assert!(rx.try_recv().is_err(), "no cancellation reply");
    app.begin_synthetic_pass(&ctx);
    app.finish_synthetic_pass();
}

/// A hidden host runs `logic` without a UI pass. Both a capture that is waiting for its first
/// geometry and a queued synthetic step must ask the host for a paint, or the window stays
/// hidden forever and the control request times out.
#[test]
fn a_logic_tick_requests_a_hidden_paint_for_waiting_captures_and_queued_input() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    let mut frame = eframe::Frame::_new_kittest();
    let (tx, _rx) = std::sync::mpsc::channel();
    let document = app.document_identity();
    app.captures.admit(90, None, tx, document, 0.0, crate::screenshot::now()).expect("parked before the first pass");
    let out = ctx.run_logic(&host_input(egui::ViewportId::ROOT), |ctx| app.logic(ctx, &mut frame));
    let paints = |out: &egui::LogicOutput| {
        out.viewport_commands.get(&egui::ViewportId::ROOT).is_some_and(|commands| commands.contains(&egui::ViewportCommand::RequestPaintWhileHidden))
    };
    assert!(paints(&out), "a capture waiting for geometry keeps the hidden window painting");

    app.synthetic.extend([press(Key::V), release(Key::V)]);
    let mut raw = host_input(egui::ViewportId::ROOT);
    app.inject_synthetic(&ctx, &mut raw);
    let out = ctx.run_logic(&raw, |ctx| app.logic(ctx, &mut frame));
    assert!(paints(&out), "queued synthetic input keeps the hidden window painting");
}

#[test]
fn a_finished_pass_does_not_replay_the_batch() {
    let ctx = Context::default();
    let mut app = app_with_doc();
    app.synthetic.extend([press(Key::B), release(Key::B)]);
    assert_eq!(hook(&mut app, &ctx).len(), 2);
    app.begin_synthetic_pass(&ctx);
    app.finish_synthetic_pass();
    // A second pass in the same frame (a popup sizing pass) must see no events from us.
    let mut second_pass = RawInput::default();
    app.inject_synthetic(&ctx, &mut second_pass);
    assert!(second_pass.events.is_empty());
}
