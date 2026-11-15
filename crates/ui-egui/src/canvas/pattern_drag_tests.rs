//! Canvas gesture tests for the linked Pattern Fill drag: ownership between canvases, the release
//! frame, previews and the commit they match.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use photocraft_doc::{Document, Fill, LayerContent, LayerId, Pattern};
use serde_json::json;

use super::{DialogCanvasInput, ViewXform, handle_dialog_canvas_input};
use crate::PhotocraftApp;
use crate::state::{Tool, View};

/// What else holds the pointer in a test frame.
#[derive(Clone, Copy, Default)]
struct FrameOpts {
    middle: bool,
    space_pan: bool,
    popup: bool,
}

/// One canvas's dialog input for the frame `ctx` is running, showing document `idx`.
fn canvas_input(ctx: &egui::Context, app: &mut PhotocraftApp, view: &mut View, xf: &ViewXform, (idx, primary): (usize, bool), opts: FrameOpts) {
    let rect = xf.rect;
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), rect);
    let free_hover = crate::dialogs::free_pointer_over(ctx, rect).is_some();
    handle_dialog_canvas_input(
        app,
        DialogCanvasInput {
            ctx,
            view,
            xf,
            rect,
            painter: &painter,
            ppp: 1.0,
            primary,
            middle: opts.middle,
            space_pan: opts.space_pan,
            temporary: None,
            free_hover,
            img_rect: rect,
            idx,
            aspect: xf.aspect,
        },
    );
}

/// One frame of the active document's canvas receiving `events`.
fn step(ctx: &egui::Context, app: &mut PhotocraftApp, view: &mut View, xf: ViewXform, primary: bool, events: Vec<Event>, opts: FrameOpts) {
    ctx.begin_pass(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))), focused: true, events, ..Default::default() });
    if opts.popup {
        egui::Popup::open_id(ctx, egui::Id::unique("pattern-drag-popup"));
    }
    let idx = app.session.active_index().unwrap_or(0);
    canvas_input(ctx, app, view, &xf, (idx, primary), opts);
    ctx.end_pass().textures_delta.clear();
}

/// An active canvas and an inactive one in the same frame, as Window › Arrange lays them out: the
/// inactive cell shows the other of two documents.
fn step_two(
    ctx: &egui::Context,
    app: &mut PhotocraftApp,
    active: (&mut View, ViewXform),
    inactive: (&mut View, ViewXform),
    events: Vec<Event>,
    opts: FrameOpts,
) {
    ctx.begin_pass(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))), focused: true, events, ..Default::default() });
    let active_idx = app.session.active_index().unwrap_or(0);
    let (view, xf) = active;
    canvas_input(ctx, app, view, &xf, (active_idx, true), opts);
    let (view, xf) = inactive;
    canvas_input(ctx, app, view, &xf, (1 - active_idx.min(1), false), opts);
    ctx.end_pass().textures_delta.clear();
}

/// A primary-button press at `p`.
fn press_at(p: Pos2) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }
}

/// A primary-button release at `p`.
fn release_at(p: Pos2) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }
}

/// A pointer move to `p`.
fn moved(p: Pos2) -> Event {
    Event::PointerMoved(p)
}

/// The event sequences a drag repeats, on one canvas the test owns.
struct Drag<'a> {
    ctx: egui::Context,
    app: &'a mut PhotocraftApp,
    view: &'a mut View,
    xf: ViewXform,
    dialog: u64,
}

impl<'a> Drag<'a> {
    fn new(ctx: &egui::Context, app: &'a mut PhotocraftApp, view: &'a mut View, xf: ViewXform, dialog: u64) -> Self {
        Self { ctx: ctx.clone(), app, view, xf, dialog }
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.frame_opts(events, FrameOpts::default());
    }

    fn frame_opts(&mut self, events: Vec<Event>, opts: FrameOpts) {
        let ctx = self.ctx.clone();
        step(&ctx, self.app, self.view, self.xf, true, events, opts);
    }

    fn move_to(&mut self, p: Pos2) {
        self.frame(vec![moved(p)]);
    }

    fn press(&mut self, p: Pos2) {
        self.frame(vec![press_at(p)]);
    }

    fn release(&mut self, p: Pos2) {
        self.frame(vec![release_at(p)]);
    }

    fn phase(&self) -> [f64; 2] {
        dialog_phase(self.app, self.dialog)
    }
}

/// The phase the dialog shows.
fn dialog_phase(app: &PhotocraftApp, dialog: u64) -> [f64; 2] {
    let a = app.ui.dialogs.iter().find(|d| d.id == dialog).expect("dialog").fields["phase"].as_array().expect("phase");
    [a[0].as_f64().expect("x"), a[1].as_f64().expect("y")]
}

/// Asserts two phases agree within float noise.
fn assert_phase(got: [f64; 2], want: [f64; 2], why: &str) {
    assert!((got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3, "{why}: {got:?} != {want:?}");
}

/// Whether the dialog still holds a private gesture waiting for its release.
fn drag_stored(app: &PhotocraftApp, dialog: u64) -> bool {
    let f = &app.ui.dialogs.iter().find(|d| d.id == dialog).expect("dialog").fields;
    // The test canvas: the root viewport showing the active document, outside any document window.
    let owner = super::pattern_drag_owner(egui::ViewportId::ROOT, app.session.active_index().unwrap_or(0), None);
    crate::pattern_fill_ui::drag_target(app, f).and_then(|target| crate::pattern_fill_ui::drag_last(f, &target.owned_by(owner))).is_some()
}

/// A document with a Pattern Fill dialog open: editing an existing linked fill, or creating one.
fn pattern_drag_app(depth: u64, create: bool) -> (PhotocraftApp, u64, Option<u64>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    let layer = if create {
        None
    } else {
        let id = app
            .run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard", "phase": [2.5, -3.25], "scale": 137.5, "angle": 45}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        Some(id)
    };
    let dialog = if create { crate::pattern_fill_ui::open_new(&mut app).unwrap() } else { crate::pattern_fill_ui::open(&mut app).unwrap() };
    (app, dialog, layer)
}

#[test]
fn a_linked_pattern_fill_drag_moves_in_document_space_at_any_view() {
    let cases = [(0.25f32, 0.0f32, false), (1.0, 0.0, false), (4.0, 0.0, false), (1.0, 30.0, true)];
    for create in [false, true] {
        for depth in [8, 16, 32] {
            for (zoom, rotation, flip) in cases {
                let (mut app, dialog, layer) = pattern_drag_app(depth, create);
                let before = app.session.active().unwrap().doc.clone();
                let count = app.session.active().unwrap().history.entries().len();
                let ctx = egui::Context::default();
                let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
                let mut view = View { zoom, center: [20.0, 15.0], ..Default::default() };
                let xf = ViewXform { rect, zoom, center: [20.0, 15.0], flip, rotation, aspect: 1.0 };
                let start = xf.to_screen(20.0, 15.0);
                let delta = vec2(7.0, -4.0);
                let end = start + xf.map_vec(delta);
                let expected = if create { [7.0, -4.0] } else { [9.5, -7.25] };
                {
                    let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
                    d.move_to(start);
                    d.press(start);
                    for f in [0.3, 0.7, 1.0] {
                        d.move_to(start + xf.map_vec(delta * f));
                        assert!(
                            std::sync::Arc::ptr_eq(&before, &d.app.session.active().unwrap().doc),
                            "create {create}, depth {depth}, {zoom}x, {rotation}°, flip {flip}: previewing does not edit"
                        );
                    }
                    d.release(end);
                    assert_phase(d.phase(), expected, &format!("create {create}, depth {depth}, {zoom}x, {rotation}°, flip {flip}"));
                }
                let (shown, _) = crate::pattern_fill_ui::display_doc(&mut app, 0).unwrap();
                if let Some(layer) = layer {
                    let LayerContent::Fill(Fill::Pattern { phase, .. }) = &shown.layer(LayerId(layer)).unwrap().content else { panic!() };
                    assert!(
                        (phase.0 - expected[0] as f32).abs() < 1e-3 && (phase.1 - expected[1] as f32).abs() < 1e-3,
                        "the preview shows the drag: {phase:?}"
                    );
                } else {
                    assert_eq!(shown.layers.len(), before.layers.len() + 1, "the preview shows the new layer");
                    let LayerContent::Fill(Fill::Pattern { phase, .. }) = &shown.layers.last().unwrap().content else { panic!() };
                    assert!((phase.0 - expected[0] as f32).abs() < 1e-3 && (phase.1 - expected[1] as f32).abs() < 1e-3, "the preview shows the drag");
                }
                let confirmed = crate::dialogs::confirm(&mut app, dialog).unwrap();
                let committed = app.session.active().unwrap().doc.clone();
                if let Some(layer) = layer {
                    assert_eq!(committed.layer(LayerId(layer)).unwrap().content, shown.layer(LayerId(layer)).unwrap().content, "OK commits the preview");
                    let LayerContent::Fill(Fill::Pattern { scale, angle, .. }) = &committed.layer(LayerId(layer)).unwrap().content else { panic!() };
                    assert!((*scale - 1.375).abs() < 1e-6 && (*angle - 45.0).abs() < 1e-6, "the drag changed only the phase");
                } else {
                    let new = confirmed["layer"].as_u64().unwrap();
                    assert_eq!(committed.layers.len(), before.layers.len() + 1);
                    let LayerContent::Fill(Fill::Pattern { phase, .. }) = &committed.layer(LayerId(new)).unwrap().content else { panic!() };
                    assert!((phase.0 - expected[0] as f32).abs() < 1e-3 && (phase.1 - expected[1] as f32).abs() < 1e-3);
                }
                assert_eq!(app.session.active().unwrap().history.entries().len(), count + 1, "one History step");
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(*app.session.active().unwrap().doc, *before, "undo restores the pre-drag document");
                app.run("edit.redo", json!({})).unwrap();
                assert_eq!(*app.session.active().unwrap().doc, *committed, "redo restores the dragged document");
            }
        }
    }
}

#[test]
fn a_pattern_drag_back_or_a_press_alone_is_no_history_step_and_cancel_discards() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(20.0, 15.0);
    let delta = xf.map_vec(vec2(5.0, 3.0));
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.entries().len();
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.move_to(start + delta);
        assert_phase(d.phase(), [7.5, -0.25], "the drag moved the pattern");
        d.move_to(start);
        d.release(start);
        assert_phase(d.phase(), [2.5, -3.25], "a drag back restores the phase");
    }
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), count, "dragging back is no History step");
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // A press alone changes nothing.
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.release(start);
    }
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // Cancel after a drag discards the preview.
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.move_to(start + delta);
        d.release(start + delta);
    }
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_some());
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_none());
    // Releasing outside keeps the last preview; a new press starts from it, not from the old origin.
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    let outside = rect.max + vec2(50.0, 50.0);
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.move_to(start + delta);
        d.move_to(outside);
        assert_phase(d.phase(), [7.5, -0.25], "leaving the canvas keeps the last preview");
        d.release(outside);
        d.move_to(start);
        assert_phase(d.phase(), [7.5, -0.25], "a release outside ends the gesture");
        d.press(start);
        d.move_to(start + delta);
        d.release(start + delta);
        assert_phase(d.phase(), [12.5, 2.75], "a new press starts from the phase shown");
    }
    crate::dialogs::cancel(&mut app, dialog).unwrap();
}

#[test]
fn a_pattern_drag_ends_on_a_popup_nested_dialog_or_unlink_and_pan_wins() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(20.0, 15.0);
    let step = |d: f32| xf.map_vec(vec2(d, 0.0));
    let mut drag = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
    drag.move_to(start);
    drag.press(start);
    drag.move_to(start + step(5.0));
    assert_phase(drag.phase(), [7.5, -3.25], "the drag began");
    // A popup takes the pointer; the stored origin is over, and the same press never restarts.
    drag.frame_opts(vec![moved(start + step(10.0))], FrameOpts { popup: true, ..Default::default() });
    assert_phase(drag.phase(), [7.5, -3.25], "a popup stops the drag");
    egui::Popup::close_all(&ctx);
    drag.move_to(start + step(15.0));
    assert_phase(drag.phase(), [7.5, -3.25], "closing the popup does not restart a held press");
    drag.release(start + step(15.0));
    // A new press moves from the phase shown.
    drag.move_to(start);
    drag.press(start);
    drag.move_to(start + step(2.0));
    assert_phase(drag.phase(), [9.5, -3.25], "a new press starts from the shown phase");
    drag.release(start + step(2.0));
    // A dialog above takes the pointer; closing it under the same press does not restart the drag.
    drag.move_to(start);
    drag.press(start);
    drag.move_to(start + step(3.0));
    let nested = drag.app.ui.open_dialog(crate::state::DialogKind::Command, serde_json::Map::new());
    drag.move_to(start + step(8.0));
    assert_phase(drag.phase(), [12.5, -3.25], "a nested dialog stops the drag");
    let _ = drag.app.ui.close_dialog(nested);
    drag.move_to(start + step(12.0));
    assert_phase(drag.phase(), [12.5, -3.25], "closing the nested dialog does not restart a held press");
    drag.release(start + step(12.0));
    // Unlinking under the press ends the gesture; relinking does not restart it.
    drag.move_to(start);
    drag.press(start);
    drag.move_to(start + step(4.0));
    drag.app.ui.dialog_mut(dialog).unwrap().fields.insert("link".into(), json!(false));
    drag.move_to(start + step(8.0));
    assert_phase(drag.phase(), [16.5, -3.25], "unlinking stops the drag");
    drag.app.ui.dialog_mut(dialog).unwrap().fields.insert("link".into(), json!(true));
    drag.move_to(start + step(12.0));
    assert_phase(drag.phase(), [16.5, -3.25], "relinking does not restart a held press");
    drag.release(start + step(12.0));
    // With the Hand tool a linked drag moves the pattern, not the view.
    drag.app.ui.tool = Tool::Hand;
    let center = drag.view.center;
    let phase = drag.phase();
    drag.move_to(start);
    drag.press(start);
    drag.move_to(start + step(5.0));
    drag.release(start + step(5.0));
    assert_phase(drag.phase(), [phase[0] + 5.0, phase[1]], "the Hand tool moves the pattern");
    assert_eq!(drag.view.center, center, "the Hand tool does not pan");
    // Space and the middle button still pan; the pattern stays.
    let phase = drag.phase();
    let center = drag.view.center;
    drag.move_to(start);
    drag.frame_opts(vec![press_at(start)], FrameOpts { space_pan: true, ..Default::default() });
    drag.frame_opts(vec![moved(start + step(6.0))], FrameOpts { space_pan: true, ..Default::default() });
    drag.frame_opts(vec![release_at(start + step(6.0))], FrameOpts { space_pan: true, ..Default::default() });
    assert_phase(drag.phase(), phase, "Space pans instead");
    assert_ne!(drag.view.center, center);
    let center = drag.view.center;
    drag.move_to(start);
    drag.frame_opts(vec![press_at(start)], FrameOpts { middle: true, ..Default::default() });
    drag.frame_opts(vec![moved(start + step(6.0))], FrameOpts { middle: true, ..Default::default() });
    drag.frame_opts(vec![release_at(start + step(6.0))], FrameOpts { middle: true, ..Default::default() });
    assert_phase(drag.phase(), phase, "the middle button pans instead");
    assert_ne!(drag.view.center, center);
    crate::dialogs::cancel(drag.app, dialog).unwrap();
}

#[test]
fn a_pattern_drag_previews_without_editing_masks_selection_or_the_tool() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": 16})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 20, "height": 10})).unwrap();
    let id = app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap();
    // A mask on the non-active Background must not change.
    let background = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    app.run("layer.layerMask.fromTransparency", json!({})).unwrap();
    app.run("layer.select", json!({"layer": id})).unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 8})).unwrap();
    app.ui.tool = Tool::Brush;
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = app.session.active().unwrap().history.entries().len();
    let revision = app.session.active().unwrap().revision;
    let active_layer = app.session.active().unwrap().active_layer;
    let fill_mask = before.layer(LayerId(id)).unwrap().mask.clone();
    let background_mask = before.layer(background).unwrap().mask.clone();
    let selection = before.selection.clone();
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(20.0, 15.0);
    let delta = xf.map_vec(vec2(4.0, 6.0));
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.move_to(start + delta);
        d.release(start + delta);
    }
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "previewing does not edit the document");
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.entries().len(), count);
    assert_eq!(st.doc.layer(LayerId(id)).unwrap().mask, fill_mask);
    assert_eq!(st.doc.layer(background).unwrap().mask, background_mask);
    assert_eq!(st.doc.selection, selection);
    assert_eq!(st.active_layer, active_layer);
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(crate::pattern_fill_ui::display_doc(&mut app, 0).is_some());
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries().len(), count + 1, "OK is one History step");
    assert_eq!(st.doc.layer(LayerId(id)).unwrap().mask, fill_mask);
    assert_eq!(st.doc.layer(background).unwrap().mask, background_mask);
    assert_eq!(st.doc.selection, selection);
    assert_eq!(st.active_layer, active_layer);
    assert_eq!(app.ui.tool, Tool::Brush);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().mask, fill_mask);
}

#[test]
fn a_release_applies_its_last_motion_and_starts_no_held_drag() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(20.0, 15.0);
    let at = |dx: f32| start + xf.map_vec(vec2(dx, 0.0));
    let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
    // A move callback stopped at +3, then the press was released at +7 without another move.
    d.move_to(start);
    d.press(start);
    d.move_to(at(3.0));
    assert_phase(d.phase(), [5.5, -3.25], "the last preview is at +3");
    d.release(at(7.0));
    assert_phase(d.phase(), [9.5, -3.25], "the release applies its own +7, not +3");
    // PointerMoved and the release in one frame still carry the final position.
    d.move_to(start);
    d.press(start);
    d.frame(vec![moved(at(4.0)), release_at(at(4.0))]);
    assert_phase(d.phase(), [13.5, -3.25], "a move and release in one frame land at +4");
    // A released press is over: moving on without a new press does not move the pattern.
    d.move_to(at(12.0));
    assert_phase(d.phase(), [13.5, -3.25], "no held gesture after release");
    // A new press starts from the phase shown.
    d.press(at(0.0));
    d.move_to(at(2.0));
    d.release(at(2.0));
    assert_phase(d.phase(), [15.5, -3.25], "a new press starts from the shown phase");
    // Releasing outside keeps the last in-canvas preview, then the gesture is over.
    d.press(at(0.0));
    d.move_to(at(3.0));
    d.move_to(rect.max + vec2(20.0, 20.0));
    d.release(rect.max + vec2(20.0, 20.0));
    d.move_to(at(9.0));
    assert_phase(d.phase(), [18.5, -3.25], "a release outside keeps the last valid preview");
    // The release event's own position still applies when the pointer leaves the window with it.
    d.press(at(0.0));
    d.move_to(at(3.0));
    d.frame(vec![release_at(at(7.0)), Event::PointerGone]);
    assert_phase(d.phase(), [25.5, -3.25], "a release and PointerGone land at the release position");
    assert!(!drag_stored(d.app, dialog), "the gesture is over");
    // An outside release is not saved by moving back inside in the same frame.
    d.press(at(0.0));
    d.move_to(at(2.0));
    d.frame(vec![release_at(rect.max + vec2(20.0, 20.0)), moved(at(9.0))]);
    assert_phase(d.phase(), [27.5, -3.25], "an outside release ignores the hover that follows");
    assert!(!drag_stored(d.app, dialog));
}

#[test]
fn atomic_press_release_sequences_keep_their_own_travel() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(20.0, 15.0);
    let at = |dx: f32, dy: f32| start + xf.map_vec(vec2(dx, dy));
    let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
    // Two taps in consecutive frames, with no idle frame: neither reuses the other's press.
    d.move_to(start);
    d.frame(vec![press_at(at(0.0, 0.0)), release_at(at(0.0, 0.0))]);
    assert_phase(d.phase(), [2.5, -3.25], "a tap moves nothing");
    assert!(!drag_stored(d.app, dialog), "a tap leaves no gesture");
    d.frame(vec![press_at(at(20.0, 10.0)), release_at(at(20.0, 10.0))]);
    assert_phase(d.phase(), [2.5, -3.25], "the next tap does not reuse the first press");
    assert!(!drag_stored(d.app, dialog));
    // A press, move and release in one frame apply exactly that press's travel and end.
    d.frame(vec![press_at(at(0.0, 0.0)), moved(at(6.0, -3.0)), release_at(at(6.0, -3.0))]);
    assert_phase(d.phase(), [8.5, -6.25], "one frame's drag applies its own +6, -3");
    assert!(!drag_stored(d.app, dialog), "and ends with the release");
    // Two atomic drags in one frame keep independent deltas.
    d.frame(vec![
        press_at(at(0.0, 0.0)),
        moved(at(4.0, 0.0)),
        release_at(at(4.0, 0.0)),
        press_at(at(20.0, 10.0)),
        moved(at(23.0, 10.0)),
        release_at(at(23.0, 10.0)),
    ]);
    assert_phase(d.phase(), [15.5, -6.25], "the second drag adds only its own +3");
    assert!(!drag_stored(d.app, dialog));
    // A release followed by a new press in one frame ends the old gesture and keeps the new one.
    d.press(at(0.0, 0.0));
    d.move_to(at(2.0, 0.0));
    assert_phase(d.phase(), [17.5, -6.25], "the held drag previewed +2");
    d.frame(vec![release_at(at(5.0, 0.0)), press_at(at(30.0, 0.0)), moved(at(33.0, 0.0))]);
    assert_phase(d.phase(), [23.5, -6.25], "the release adds +3 and the new press its own +3");
    assert!(drag_stored(d.app, dialog), "the new press is still held");
    d.move_to(at(35.0, 0.0));
    d.release(at(35.0, 0.0));
    assert_phase(d.phase(), [25.5, -6.25], "the new gesture continues and ends on its release");
    assert!(!drag_stored(d.app, dialog));
}

#[test]
fn a_secondary_canvas_cannot_move_or_cancel_the_active_pattern_drag() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    let id =
        app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": "Checkerboard", "phase": [2.5, -3.25]})).unwrap()["layer"].as_u64().unwrap();
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    // Window › Arrange: a second document in its own cell, the fill's document active.
    app.run("file.new", json!({"width": 30, "height": 20})).unwrap();
    assert!(app.session.set_active(0));
    let ctx = egui::Context::default();
    let rect_a = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0));
    let rect_b = Rect::from_min_size(pos2(400.0, 0.0), vec2(400.0, 600.0));
    let mut view_a = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let mut view_b = View { zoom: 1.0, center: [15.0, 10.0], ..Default::default() };
    let xf_a = ViewXform { rect: rect_a, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let xf_b = ViewXform { rect: rect_b, zoom: 1.0, center: [15.0, 10.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start_a = xf_a.to_screen(20.0, 15.0);
    let start_b = xf_b.to_screen(15.0, 10.0);
    let delta = |dx: f32| vec2(dx, 0.0);
    let before = dialog_phase(&app, dialog);
    // A press on the inactive canvas is not the active fill's gesture.
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![moved(start_b)], FrameOpts::default());
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![press_at(start_b)], FrameOpts::default());
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![moved(start_b + delta(9.0))], FrameOpts::default());
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![release_at(start_b + delta(9.0))], FrameOpts::default());
    assert_eq!(dialog_phase(&app, dialog), before, "a press on an inactive canvas cannot move the active fill");
    // The inactive canvas does not cancel the active canvas's ongoing drag either.
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![moved(start_a)], FrameOpts::default());
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![press_at(start_a)], FrameOpts::default());
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![moved(start_a + delta(5.0))], FrameOpts::default());
    assert_phase(dialog_phase(&app, dialog), [7.5, -3.25], "the active canvas starts the drag");
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![moved(start_a + delta(12.0))], FrameOpts::default());
    assert_phase(dialog_phase(&app, dialog), [14.5, -3.25], "a secondary callback keeps the active drag");
    step_two(&ctx, &mut app, (&mut view_a, xf_a), (&mut view_b, xf_b), vec![release_at(start_a + delta(12.0))], FrameOpts::default());
    assert_phase(dialog_phase(&app, dialog), [14.5, -3.25], "the active canvas finishes the drag");
    // A focused canvas whose document is not the dialog's cannot move the fill.
    assert!(app.session.set_active(1));
    step(&ctx, &mut app, &mut view_b, xf_b, true, vec![press_at(start_b)], FrameOpts::default());
    step(&ctx, &mut app, &mut view_b, xf_b, true, vec![moved(start_b + delta(9.0))], FrameOpts::default());
    step(&ctx, &mut app, &mut view_b, xf_b, true, vec![release_at(start_b + delta(9.0))], FrameOpts::default());
    assert_phase(dialog_phase(&app, dialog), [14.5, -3.25], "another document's canvas cannot move the fill");
    assert!(app.session.set_active(0));
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view_a, xf_a, dialog);
        d.move_to(start_a);
        d.press(start_a);
        d.move_to(start_a + delta(2.0));
        d.release(start_a + delta(2.0));
        assert_phase(d.phase(), [16.5, -3.25], "the active canvas still moves the fill");
    }
    let LayerContent::Fill(Fill::Pattern { phase, .. }) = &app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!() };
    assert_eq!((phase.0, phase.1), (2.5, -3.25), "the document is unchanged until OK");
    crate::dialogs::cancel(&mut app, dialog).unwrap();
}

#[test]
fn a_drag_moves_the_visible_pattern_not_only_its_phase() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 2, "depth": 8})).unwrap();
    // Our own 8 × 2 tile: a solid red 2 × 2 left edge, transparent elsewhere.
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.fill_rect(photocraft_geom::Rect::new(0, 0, 2, 2), &[1.0, 0.0, 0.0, 1.0]);
    let tile = Pattern::new("Shift tile", surface, 8, 2);
    let pattern = tile.id.clone();
    app.session.patterns.items.push(tile);
    app.run(photocraft_engine::pattern_fill_cmds::CREATE, json!({"pattern": pattern, "phase": [0.0, 0.0], "scale": 100, "angle": 0, "link": true})).unwrap();
    let dialog = crate::pattern_fill_ui::open(&mut app).unwrap();
    let band = |doc: &Document| {
        let buf = photocraft_compose::render(doc, photocraft_geom::Rect::new(0, 0, 8, 1));
        (0..8).filter(|x| buf.px[*x as usize][0] > 0.5 && buf.px[*x as usize][1] < 0.5).collect::<Vec<i32>>()
    };
    let before = band(&app.session.active().unwrap().doc);
    assert_eq!(before, vec![0, 1], "the tile's red half starts at the origin");
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let mut view = View { zoom: 4.0, center: [4.0, 1.0], ..Default::default() };
    let xf = ViewXform { rect, zoom: 4.0, center: [4.0, 1.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let start = xf.to_screen(4.0, 1.0);
    let end = xf.to_screen(6.0, 1.0);
    {
        let mut d = Drag::new(&ctx, &mut app, &mut view, xf, dialog);
        d.move_to(start);
        d.press(start);
        d.move_to(end);
        d.release(end);
    }
    let (shown, _) = crate::pattern_fill_ui::display_doc(&mut app, 0).unwrap();
    let moved = band(&shown);
    assert_eq!(moved, vec![2, 3], "the preview paints the red half two document pixels right");
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let committed = band(&app.session.active().unwrap().doc);
    assert_eq!(committed, moved, "OK commits the pixels the preview showed");
    assert_ne!(committed, before, "the drag is visible, not only a phase field");
}

/// A frame in which the main window's canvas and a Window › New Window view of the same document
/// both run as primary views. Each OS window has its own pointer, so each gets its own context and
/// events; the document window is drawn after the main one, as `extra_windows` does.
fn step_windows(app: &mut PhotocraftApp, main: (&egui::Context, &mut View, ViewXform, Vec<Event>), window: (&egui::Context, &mut View, ViewXform, Vec<Event>)) {
    let idx = app.session.active_index().unwrap_or(0);
    let screen = Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)));
    let (ctx, view, xf, events) = main;
    ctx.begin_pass(egui::RawInput { screen_rect: screen, focused: false, events, ..Default::default() });
    canvas_input(ctx, app, view, &xf, (idx, true), FrameOpts::default());
    ctx.end_pass().textures_delta.clear();
    let (ctx, view, xf, events) = window;
    ctx.begin_pass(egui::RawInput { screen_rect: screen, focused: true, events, ..Default::default() });
    app.drawing_window = Some(7);
    canvas_input(ctx, app, view, &xf, (idx, true), FrameOpts::default());
    app.drawing_window = None;
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn a_new_window_view_keeps_its_drag_while_the_main_canvas_is_idle() {
    let (mut app, dialog, _) = pattern_drag_app(8, false);
    let (main_ctx, window_ctx) = (egui::Context::default(), egui::Context::default());
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let xf = ViewXform { rect, zoom: 1.0, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 };
    let mut main_view = View { zoom: 1.0, center: [20.0, 15.0], ..Default::default() };
    let mut window_view = main_view.clone();
    let start = xf.to_screen(20.0, 15.0);
    let mut frame = |app: &mut PhotocraftApp, main: Vec<Event>, window: Vec<Event>| {
        step_windows(app, (&main_ctx, &mut main_view, xf, main), (&window_ctx, &mut window_view, xf, window));
    };
    frame(&mut app, vec![], vec![moved(start)]);
    frame(&mut app, vec![], vec![press_at(start)]);
    frame(&mut app, vec![], vec![moved(start + vec2(3.0, 0.0))]);
    assert_phase(dialog_phase(&app, dialog), [5.5, -3.25], "the window's drag moves the fill");
    // The main window's button is up: its canvas does not end the window's gesture.
    frame(&mut app, vec![], vec![moved(start + vec2(7.0, -4.0))]);
    assert_phase(dialog_phase(&app, dialog), [9.5, -7.25], "the drag continues while the main canvas is idle");
    // Nor does the main window losing the pointer.
    frame(&mut app, vec![Event::PointerGone], vec![moved(start + vec2(9.0, -4.0))]);
    assert_phase(dialog_phase(&app, dialog), [11.5, -7.25], "the main window losing the pointer does not end it");
    frame(&mut app, vec![], vec![release_at(start + vec2(10.0, -4.0))]);
    assert_phase(dialog_phase(&app, dialog), [12.5, -7.25], "the release lands where the window let go");
    // The gesture is over: a later move in the window moves nothing.
    frame(&mut app, vec![], vec![moved(start + vec2(20.0, 0.0))]);
    assert_phase(dialog_phase(&app, dialog), [12.5, -7.25], "a released gesture does not resume");
    crate::dialogs::cancel(&mut app, dialog).unwrap();
}
