//! Crop tool: the contextual task bar under the crop frame (#10890).
//!
//! Measured in the reference application: the bar is centred under the frame with its top 11 px
//! below the frame's bottom edge and follows the frame; dragged by its grip it stays where it was
//! dropped; Reset bar position brings it back under the frame, Pin bar position keeps it where it
//! is; Hide bar hides it and unticks Window › Contextual Task Bar; Cancel and Done show only
//! while a crop is pending and do what the options bar's cancel and commit do.
//!
//! The whole app runs in an egui_kittest harness (no GPU needed); the bar's own buttons are
//! pressed with pointer events, so a click must reach the bar and not the canvas under it.

use egui::{Modifiers, PointerButton, Pos2, Rect, Vec2, pos2, vec2};
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::crop_ui::{corners, pending};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// The bar's top below the frame's bottom edge, measured in the reference application (screen px
/// at 100 %, points here).
const GAP: f32 = 11.0;

/// The app in English with a 400 × 300 white document at 100 %, the Crop tool picked in Classic
/// Mode (the frame moves over a fixed image, so the frame's screen place is easy to predict).
fn harness() -> Harness {
    let mut h = Harness::builder().with_size(vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        // English labels, whatever the system language.
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool_options.crop_shield.classic_mode = true;
        app
    });
    h.run_steps(4);
    control(&mut h, "ui.set", json!({"tool": "crop", "zoom": 1.0, "center": [200.0, 150.0]}));
    h.run_steps(6);
    h
}

/// Runs one control-channel request, which must succeed, then lets the app draw.
fn control(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let Outcome::Done(v) = handle(h.state_mut(), &ctx, &req) else { panic!("{method} {params}: not done") };
    assert_eq!(v["ok"], true, "{method} {params}: {v}");
    h.run_steps(6);
    v
}

/// Draws a new crop frame from `a` to `b` (document px), as an agent does.
fn draw_frame(h: &mut Harness, a: [f64; 2], b: [f64; 2]) {
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    control(h, "ui.pointer", json!({"events": events}));
}

/// The crop frame's screen bounds, as the canvas maps it.
fn frame_on_screen(h: &Harness) -> Rect {
    let app = h.state();
    let xf = ViewXform::active(app).expect("a view");
    let r = app.ui.crop_rect.expect("a crop frame");
    Rect::from_points(&corners(r, app.ui.crop_angle).map(|[x, y]| xf.to_screen(x as f32, y as f32)))
}

/// The task bar's screen rect, as `ui.inspect` reports it.
fn bar(h: &Harness) -> Option<Rect> {
    h.state().ui.task_bar_shown.map(|[x0, y0, x1, y1]| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)))
}

fn assert_under_frame(h: &Harness, what: &str) {
    let (f, b) = (frame_on_screen(h), bar(h).unwrap_or_else(|| panic!("{what}: no bar")));
    assert!((b.top() - (f.bottom() + GAP)).abs() < 1.0, "{what}: bar top {} vs frame bottom {} + {GAP}", b.top(), f.bottom());
    assert!((b.center().x - f.center().x).abs() < 1.0, "{what}: bar centre {} vs frame centre {}", b.center().x, f.center().x);
}

fn button(h: &mut Harness, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A real click at `at`: hover, press, release.
fn click_at(h: &mut Harness, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    button(h, at, true);
    button(h, at, false);
    h.run_steps(4);
}

/// The centre of the widget labelled `label` (which must exist).
fn widget(h: &Harness, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

fn has(h: &Harness, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// Opens the bar's ⋯ menu and picks `entry`.
fn menu(h: &mut Harness, entry: &str) {
    let more = widget(h, "More options");
    click_at(h, more);
    let at = widget(h, entry);
    click_at(h, at);
}

/// Drags the bar by its grip by `d` screen points.
fn drag_grip(h: &mut Harness, d: Vec2) {
    let from = widget(h, "Move bar");
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    button(h, from, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(from + d * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    button(h, from + d, false);
    h.run_steps(4);
}

fn window_item_checked(h: &mut Harness) -> Option<bool> {
    let items = control(h, "ui.menu.list", json!({}));
    let item = items["result"].as_array()?.iter().find(|i| i["id"] == "window.panel.contextualTaskBar")?.clone();
    item["checked"].as_bool()
}

#[test]
fn sits_centred_eleven_points_under_the_frame_and_follows_it() {
    let mut h = harness();
    // The untouched default frame (the whole canvas).
    assert_under_frame(&h, "default frame");
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    assert_under_frame(&h, "drawn frame");
    // Moved: the bar comes along.
    let events = json!([
        {"kind": "down", "x": 150.0, "y": 110.0},
        {"kind": "move", "x": 210.0, "y": 150.0},
        {"kind": "up", "x": 210.0, "y": 150.0},
    ]);
    control(&mut h, "ui.pointer", json!({"events": events}));
    assert_eq!(h.state().ui.crop_rect, Some([100.0, 70.0, 320.0, 240.0]), "the frame moved");
    assert_under_frame(&h, "moved frame");
    let v = control(&mut h, "ui.inspect", json!({}));
    assert_eq!(v["result"]["taskBar"]["visible"], true);
    assert_eq!(v["result"]["taskBar"]["enabled"], true);
    assert_eq!(v["result"]["taskBar"]["pinned"], false);
}

#[test]
fn shows_only_with_the_crop_tool() {
    let mut h = harness();
    assert!(bar(&h).is_some());
    control(&mut h, "ui.set", json!({"tool": "brush"}));
    assert!(bar(&h).is_none(), "another tool hides the bar");
    assert!(!has(&h, "More options"));
    // No document: no canvas, no bar (and `ui.inspect` doesn't report the last one drawn).
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    assert!(bar(&h).is_some());
    h.state_mut().run("file.close", json!({})).expect("close");
    h.run_steps(6);
    assert!(h.state().session.active().is_none());
    assert!(bar(&h).is_none(), "closing the document clears the bar");
}

#[test]
fn a_dragged_bar_stays_put_and_reset_brings_it_back() {
    let mut h = harness();
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    let before = bar(&h).expect("bar");
    drag_grip(&mut h, vec2(-60.0, -120.0));
    let dropped = bar(&h).expect("bar");
    assert!((dropped.min - (before.min + vec2(-60.0, -120.0))).length() < 1.5, "{before:?} dragged to {dropped:?}");
    assert!(h.state().ui.panels.task_bar.at.is_some());
    assert_eq!(h.state().ui.crop_rect, Some([40.0, 30.0, 260.0, 200.0]), "dragging the bar doesn't touch the frame");
    // A new frame no longer moves it; its left edge stays.
    draw_frame(&mut h, [300.0, 200.0], [120.0, 60.0]);
    assert_eq!(bar(&h).expect("bar").left_top(), dropped.left_top());
    menu(&mut h, "Reset bar position");
    assert_eq!(h.state().ui.panels.task_bar, Default::default());
    assert_under_frame(&h, "after Reset bar position");
}

#[test]
fn pin_keeps_the_bar_where_it_is() {
    let mut h = harness();
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    menu(&mut h, "Pin bar position");
    let pinned = bar(&h).expect("bar");
    assert!(h.state().ui.panels.task_bar.pinned);
    draw_frame(&mut h, [300.0, 200.0], [120.0, 60.0]);
    assert_eq!(bar(&h).expect("bar").left_top(), pinned.left_top(), "pinned: the frame changed, the bar didn't move");
    // Pinning over the control channel freezes it the same way.
    menu(&mut h, "Reset bar position");
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": true, "at": null}}}));
    let here = bar(&h).expect("bar");
    draw_frame(&mut h, [20.0, 20.0], [100.0, 80.0]);
    assert_eq!(bar(&h).expect("bar").left_top(), here.left_top());
}

/// Runs one control-channel request without drawing after it.
fn request(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let Outcome::Done(v) = handle(h.state_mut(), &ctx, &req) else { panic!("{method} {params}: not done") };
    v
}

#[test]
fn ui_set_changes_only_the_place_fields_it_names() {
    let mut h = harness();
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    drag_grip(&mut h, vec2(-60.0, -120.0));
    let dropped = bar(&h).expect("bar");
    let at = h.state().ui.panels.task_bar.at;
    // Pinning keeps the dragged place: the bar doesn't jump back under the frame.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": true}}}));
    assert_eq!(h.state().ui.panels.task_bar.at, at);
    assert!(h.state().ui.panels.task_bar.pinned);
    assert_eq!(bar(&h).expect("bar").left_top(), dropped.left_top());
    // Placing it keeps the pin.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"at": [12.0, 34.0]}}}));
    assert_eq!(h.state().ui.panels.task_bar, photocraft_ui_egui::state::TaskBarPlace { pinned: true, at: Some([12.0, 34.0]) });
    // A field the bar doesn't have is refused, and nothing changes.
    let v = request(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": false, "nope": 1}}}));
    assert_eq!(v["ok"], false, "{v}");
    assert!(h.state().ui.panels.task_bar.pinned);
    // Reset bar position over the control channel.
    control(&mut h, "ui.set", json!({"panels": {"task_bar": {"pinned": false, "at": null}}}));
    assert_eq!(h.state().ui.panels.task_bar, Default::default());
    assert_under_frame(&h, "after the reset");
}

#[test]
fn the_bars_preset_dropdown_is_the_options_bars() {
    let mut h = harness();
    let b = bar(&h).expect("bar");
    assert_eq!(h.state().ui.crop_rect, Some([0.0, 0.0, 400.0, 300.0]), "the untouched frame is the canvas");
    // Both bars show the Ratio dropdown: open the task bar's.
    let ratio = h.get_all_by_value("Ratio").map(|n| n.rect().center()).find(|p| b.contains(*p)).expect("the task bar's Ratio dropdown");
    click_at(&mut h, ratio);
    h.get_by_label("1 : 1 (Square)").click();
    h.run_steps(6);
    let [x0, y0, x1, y1] = h.state().ui.crop_rect.expect("a frame");
    assert!(((x1 - x0) - (y1 - y0)).abs() < 1e-6, "a square frame: {:?}", h.state().ui.crop_rect);
    assert_eq!(h.get_all_by_value("1 : 1 (Square)").count(), 2, "the options bar shows the same preset");
}

#[test]
fn inspect_reports_a_bar_hidden_before_the_next_draw_as_hidden() {
    let mut h = harness();
    assert!(bar(&h).is_some());
    for hide in [json!({"panels": {"context_task_bar": false}}), json!({"tool": "brush"})] {
        let v = request(&mut h, "ui.set", hide.clone());
        assert_eq!(v["ok"], true, "{v}");
        let v = request(&mut h, "ui.inspect", json!({}));
        assert_eq!(v["result"]["taskBar"]["visible"], false, "{hide}");
        assert_eq!(v["result"]["taskBar"]["rect"], Value::Null, "{hide}");
        control(&mut h, "ui.set", json!({"tool": "crop", "panels": {"context_task_bar": true}}));
        assert!(bar(&h).is_some());
    }
}

#[test]
fn with_tiled_documents_the_bar_sits_on_the_active_tile() {
    let mut h = harness();
    h.state_mut().run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
    h.run_steps(4);
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.arrange.twoUpVertical", json!({})).expect("2-up");
    h.run_steps(6);
    // Each tile active in turn (side by side, the first one left): the first tile is drawn before
    // the second, so a bar drawn by every tile would end up on the second one both times.
    let mut centres = Vec::new();
    for idx in [0, 1] {
        assert!(h.state_mut().session.set_active(idx));
        h.run_steps(8);
        assert_eq!(h.state().session.active_index(), Some(idx));
        centres.push(bar(&h).unwrap_or_else(|| panic!("document {idx}: no bar")).center().x);
    }
    // The documents are the same size at the same zoom: the bars are one tile width apart.
    assert!(centres[1] - centres[0] > 200.0, "the bar follows the active tile: {centres:?}");
    // A click into the first tile activates its document while the frame is drawn: the second
    // tile, drawn after it as the active one, must not put the new document's bar on itself.
    let y = bar(&h).expect("bar").top() - 120.0;
    let at = pos2(centres[0], y);
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(2);
    button(&mut h, at, true);
    assert_eq!(h.state().session.active_index(), Some(0), "the click activates the first tile");
    if let Some(b) = bar(&h) {
        assert!((b.center().x - centres[0]).abs() < 100.0, "the bar is on the first tile, not {} (the second)", b.center().x);
    }
    button(&mut h, at, false);
}

#[test]
fn hide_bar_and_the_window_toggle() {
    let mut h = harness();
    assert_eq!(window_item_checked(&mut h), Some(true));
    menu(&mut h, "Hide bar");
    assert!(!h.state().ui.panels.context_task_bar);
    assert!(bar(&h).is_none());
    assert_eq!(window_item_checked(&mut h), Some(false), "Hide bar unticks Window › Contextual Task Bar");
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.panel.contextualTaskBar", json!({})).expect("toggle");
    h.run_steps(6);
    assert!(bar(&h).is_some(), "Window › Contextual Task Bar shows it again");
    assert_eq!(window_item_checked(&mut h), Some(true));
    control(&mut h, "ui.set", json!({"panels": {"context_task_bar": false}}));
    assert!(bar(&h).is_none(), "ui.set hides it");
}

#[test]
fn cancel_and_done_only_while_a_crop_is_pending() {
    let mut h = harness();
    assert!(!pending(h.state()));
    assert!(has(&h, "More options") && !has(&h, "Done") && !has(&h, "Cancel"), "the untouched frame offers no Cancel / Done");
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    assert!(pending(h.state()));
    assert!(has(&h, "Done") && has(&h, "Cancel"));
    let narrow = bar(&h).expect("bar");
    assert!(narrow.width() > 0.0);
    // Cancel: the frame goes, as with the options bar's cancel.
    let at = widget(&h, "Cancel");
    click_at(&mut h, at);
    assert!(!pending(h.state()));
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((400, 300)));
    // Done commits the crop.
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    let at = widget(&h, "Done");
    click_at(&mut h, at);
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((220, 170)), "Done crops");
}

/// The active document's stored zoom.
fn zoom(h: &Harness) -> f32 {
    let app = h.state();
    app.session.active_index().and_then(|i| app.ui.views.get(i)).map(|v| v.zoom).expect("a view")
}

/// Done, like the options bar's commit, fits the cropped document on screen. Fit never zooms past
/// 100 %, so the crop starts at 50 %: the 220 × 170 result fits at 100 %. The bar draws inside the
/// canvas pass, whose write-back of its own camera used to drop that fit.
#[test]
fn done_fits_the_cropped_document() {
    let mut h = harness();
    control(&mut h, "ui.set", json!({"zoom": 0.5, "center": [200.0, 150.0]}));
    assert!((zoom(&h) - 0.5).abs() < 1e-4);
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    let at = widget(&h, "Done");
    click_at(&mut h, at);
    assert_eq!(h.state().session.active().map(|s| (s.doc.size.width, s.doc.size.height)), Some((220, 170)), "Done crops");
    assert!((zoom(&h) - 1.0).abs() < 1e-4, "Done fits the cropped document: zoom {}", zoom(&h));
    let app = h.state();
    let v = app.session.active_index().and_then(|i| app.ui.views.get(i)).expect("a view");
    assert!(!v.fit_pending && (v.center[0] - 110.0).abs() < 0.5 && (v.center[1] - 85.0).abs() < 0.5, "fitted and centred: {:?}", v.center);
}

#[test]
fn the_bar_grows_for_cancel_and_done() {
    let mut h = harness();
    let idle = bar(&h).expect("bar").width();
    draw_frame(&mut h, [40.0, 30.0], [260.0, 200.0]);
    h.run_steps(4);
    let busy = bar(&h).expect("bar").width();
    assert!(busy > idle + 40.0, "idle {idle}, pending {busy}");
}

#[test]
fn straighten_is_the_options_bar_straighten() {
    let mut h = harness();
    assert!(!photocraft_ui_egui::crop_straighten::mode(h.state()));
    let at = widget(&h, "Straighten");
    click_at(&mut h, at);
    assert!(photocraft_ui_egui::crop_straighten::mode(h.state()), "armed, as by the options bar's button");
    assert_eq!(h.state().ui.crop_rect, Some([0.0, 0.0, 400.0, 300.0]), "the click didn't reach the canvas");
}

#[test]
fn hide_and_place_are_kept_with_the_panel_layout() {
    let mut h = harness();
    // A layout saved before the bar existed: the bar is on and follows the frame.
    let mut old = serde_json::to_value(&h.state().ui.panels).expect("panels");
    old.as_object_mut().map(|o| (o.remove("context_task_bar"), o.remove("task_bar")));
    photocraft_ui_egui::dock::apply(h.state_mut(), &json!({"panels": old}));
    assert!(h.state().ui.panels.context_task_bar);
    assert_eq!(h.state().ui.panels.task_bar, Default::default());
    // What a restart restores.
    let saved = json!({"panels": {"context_task_bar": false, "task_bar": {"pinned": true, "at": [12.0, 34.0]}}});
    let mut panels = serde_json::to_value(&h.state().ui.panels).expect("panels");
    panels.as_object_mut().expect("object").extend(saved["panels"].as_object().expect("object").clone());
    photocraft_ui_egui::dock::apply(h.state_mut(), &json!({"panels": panels}));
    let p = &h.state().ui.panels;
    assert!(!p.context_task_bar);
    assert!(p.task_bar.pinned);
    assert_eq!(p.task_bar.at, Some([12.0, 34.0]));
}
