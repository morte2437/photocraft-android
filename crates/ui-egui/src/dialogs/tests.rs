use super::*;

/// A filter dialog over a 256×256 layer, with the preview the canvas computed for it on screen.
fn filter_dialog_with_preview(background: bool) -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = background;
    app.run("file.new", serde_json::json!({"width": 256, "height": 256, "background": "#3366cc"})).unwrap();
    let id = open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur");
    let st = app.session.active().unwrap();
    let result = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "filter.blur.gaussianBlur", &serde_json::json!({"radius": 4.0}), 1);
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        dialog: id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: serde_json::json!({"radius": 4.0}),
        k: 1,
    };
    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key, result: result.map(std::sync::Arc::new), committing: false });
    (app, id)
}

#[test]
fn a_filters_preview_stays_on_screen_until_its_job_lands() {
    // OK dropped the preview and the filter ran as a background job, so the canvas flashed
    // the unfiltered image until the job finished.
    let (mut app, id) = filter_dialog_with_preview(true);
    let revision = app.session.active().unwrap().revision;
    confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let job = app.session.job_on(doc).expect("the blur runs as a background job").id;
    assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some((1, _, [256, 256])))), "the preview is shown");
    // The job lands: the document changes and the preview goes.
    app.session.wait_job(job).unwrap();
    assert_ne!(app.session.active().unwrap().revision, revision);
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
    assert!(app.filter_preview.is_none());

    // A cancelled job leaves the document as it was, so its preview goes too.
    let (mut app, id) = filter_dialog_with_preview(true);
    confirm(&mut app, id).unwrap();
    let doc = app.session.active().unwrap().doc.id;
    let job = app.session.job_on(doc).unwrap().id;
    assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some(_))));
    assert!(app.session.cancel_job(job));
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
    assert!(app.filter_preview.is_none());

    // Inline (no background jobs): the filter has already landed, nothing is held.
    let (mut app, id) = filter_dialog_with_preview(false);
    confirm(&mut app, id).unwrap();
    assert!(app.filter_preview.is_none());
    assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), None);
}

/// #5496: a pointer click on the title bar's close box does what Cancel does: the dialog
/// closes, the live preview goes and nothing reaches the document or the journal. On macOS
/// dialogs have no close box.
#[test]
fn the_title_bar_close_box_cancels_like_cancel() {
    use egui_kittest::kittest::Queryable;
    let close = |button: &str| {
        let (mut app, _) = filter_dialog_with_preview(false);
        let revision = app.session.active().unwrap().revision;
        let journal = app.session.journal.len();
        // The fonts are set up after the harness's first frame: the dialog appears after it.
        let dialogs = std::mem::take(&mut app.ui.dialogs);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.dialogs = dialogs;
        h.run_steps(4);
        assert!(h.state().filter_preview.is_some(), "the preview is on screen");
        if !CLOSE_BOX {
            assert_eq!(h.query_all_by_label("Close").count(), 0, "no close box on macOS");
        }
        let target = h.query_by_label(button).map(|n| n.rect().center())?;
        h.hover_at(target);
        h.drag_at(target);
        h.drop_at(target);
        h.run_steps(3);
        let app = h.state();
        Some((
            app.ui.dialogs.is_empty(),
            app.filter_preview.is_none(),
            app.session.active().unwrap().revision == revision,
            app.session.journal.len() == journal,
        ))
    };
    let cancel = close("Cancel");
    assert_eq!(cancel, Some((true, true, true, true)), "Cancel closes, drops the preview and changes nothing");
    if CLOSE_BOX {
        assert_eq!(close("Close"), cancel, "the × is Cancel");
    }
}

/// #5496: the close box sits at the title band's right end, and the band stays a drag handle
/// around it: a drag that starts on the × neither moves nor closes the dialog, and the band
/// just left of the × still moves it.
#[test]
fn the_close_box_and_the_title_drag_keep_apart() {
    use egui_kittest::kittest::Queryable;
    if !CLOSE_BOX {
        return;
    }
    let mut h = dialog_harness(egui::vec2(1280.0, 800.0), "filter.blur.gaussianBlur");
    let title = h.get_by_label("Gaussian Blur").rect();
    let x = h.get_by_label("Close").rect();
    let frame = rects(&h.ctx)[0];
    assert!((x.center().y - title.center().y).abs() < 2.0 && x.left() > title.right(), "the × is on the title row: {x:?}, {title:?}");
    assert!(frame.right() - x.right() < 16.0, "at its right end: {x:?} in {frame:?}");
    let drag = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2| {
        h.hover_at(from);
        h.drag_at(from);
        h.run_steps(2);
        for i in 1..=8 {
            h.hover_at(from + (to - from) * (i as f32 / 8.0));
            h.run_steps(1);
        }
        h.drop_at(to);
        h.run_steps(3);
    };
    drag(&mut h, x.center(), x.center() + egui::vec2(-80.0, 60.0));
    assert_eq!(h.state().ui.dialogs.len(), 1, "a drag from the × doesn't close the dialog");
    assert!((h.get_by_label("Gaussian Blur").rect().min - title.min).length() < 1.0, "nor move it");
    let x = h.get_by_label("Close").rect();
    let beside = x.left_center() - egui::vec2(4.0, 0.0);
    drag(&mut h, beside, beside + egui::vec2(-120.0, 40.0));
    assert_eq!(h.state().ui.dialogs.len(), 1, "a title drag doesn't close the dialog");
    let moved = h.get_by_label("Gaussian Blur").rect().min - title.min;
    assert!((moved - egui::vec2(-120.0, 40.0)).length() < 2.0, "the band beside the × moved the dialog: {moved:?}");
}

/// #6810: dragging a dialog's title moves the dialog under the pointer, so the press origin is
/// no longer inside it; that drag must not pan the canvas or reach a canvas tool.
#[test]
fn dragging_a_dialog_title_does_not_pan_the_canvas() {
    let ctx = egui::Context::default();
    let canvas = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 800.0));
    let at_press = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(300.0, 200.0));
    let moved = egui::Rect::from_min_max(egui::pos2(550.0, 480.0), egui::pos2(750.0, 580.0));
    let run = |rect: egui::Rect, events: Vec<egui::Event>| {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(RECTS), vec![rect]));
        ctx.begin_pass(egui::RawInput { events, ..Default::default() });
        let out = (pan_delta(&ctx, canvas, true), free_press(&ctx, canvas));
        ctx.end_pass().textures_delta.clear();
        out
    };
    let press =
        egui::Event::PointerButton { pos: egui::pos2(150.0, 120.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
    let (pan, click) = run(at_press, vec![egui::Event::PointerMoved(egui::pos2(150.0, 120.0)), press]);
    assert!(pan.is_none() && click.is_none(), "the press is on the dialog");
    let (pan, click) = run(moved, vec![egui::Event::PointerMoved(egui::pos2(600.0, 500.0))]);
    assert!(pan.is_none(), "the dialog moved away from the press origin: {pan:?}");
    assert!(click.is_none(), "and the drag is not a canvas tool drag: {click:?}");
}

#[test]
fn color_picker_has_inset_content_and_actions_at_the_right_edge() {
    use egui_kittest::{Harness, kittest::Queryable};

    for size in [egui::vec2(760.0, 480.0), egui::vec2(960.0, 640.0)] {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        crate::color_picker_ui::open(h.state_mut(), "foreground");
        h.run_steps(4);
        let frame = rects(&h.ctx)[0];
        let title = h.get_by_label("Color Picker (Foreground Color)").rect();
        let ok = h.get_by_label("OK").rect();
        let cancel = h.get_by_label("Cancel").rect();
        let web = h.get_by_label("Only Web Colors").rect();
        assert!((ok.right() - title.right()).abs() < 1.0, "OK aligns with the right content edge: {ok:?} vs {title:?}");
        assert!((cancel.right() - ok.right()).abs() < 1.0);
        assert!(frame.right() - ok.right() >= 16.0, "buttons retain the window's padding");
        assert!(web.left() - frame.left() >= 16.0, "checkbox has comfortable left padding");
        assert!(frame.bottom() - web.bottom() >= 16.0, "checkbox has comfortable bottom padding");
        assert!(title.top() - frame.top() >= 16.0, "title has top padding");
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(frame), "padded picker fits {size:?}: {frame:?}");
        // A click anywhere inside the added margin must stay out of the canvas eyedropper.
        let margin = egui::pos2(frame.left() + 8.0, web.center().y);
        h.hover_at(margin);
        h.run_steps(1);
        assert!(free_pointer_over(&h.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)).is_none());
    }
}

#[test]
fn layer_style_title_gutters_can_start_drags() {
    use egui_kittest::{Harness, kittest::Queryable};

    for point in 0..5 {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        h.run_steps(3);
        let before = h.get_by_label("Layer Style").rect();
        let from = match point {
            0 => egui::pos2(before.left() - 5.0, before.bottom() - 4.0),
            1 => egui::pos2(before.right() + 5.0, before.bottom() - 4.0),
            2 => egui::pos2(before.center().x, before.bottom() + 3.0),
            3 => egui::pos2(before.center().x, before.top() - 5.0),
            _ => egui::pos2(before.center().x, before.bottom() - 3.0),
        };
        h.hover_at(from);
        h.drag_at(from);
        h.run_steps(2);
        for i in 1..=8 {
            h.hover_at(from + egui::vec2(64.0, -32.0) * (i as f32 / 8.0));
            h.run_steps(1);
        }
        h.drop_at(from + egui::vec2(64.0, -32.0));
        h.run_steps(3);
        let delta = h.get_by_label("Layer Style").rect().min - before.min;
        assert!((delta - egui::vec2(64.0, -32.0)).length() < 2.0, "point {point}: moved {delta:?}");
        assert_eq!(h.state().ui.dialogs.len(), 1);
    }
}

#[test]
fn dragging_the_title_bar_moves_the_dialog() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
    harness.run_steps(3);
    let before = harness.get_by_label("Layer Style").rect();

    // Grab the title text itself: it must move the dialog, not select the text.
    let from = before.center();
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run_steps(2);
    for i in 1..=10 {
        harness.hover_at(from + egui::vec2(-12.0, 8.0) * i as f32);
        harness.run_steps(1);
    }
    harness.drop_at(from + egui::vec2(-120.0, 80.0));
    harness.run_steps(3);

    let moved = harness.get_by_label("Layer Style").rect().min - before.min;
    assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
    assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
}

/// #4872: an alert (Error, About) closes with ↩ as with OK, but not on the ↩ that raised it.
#[test]
fn enter_closes_an_alert_but_not_the_enter_that_opened_it() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    let alert = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>| {
        h.state_mut().ui.open_dialog(DialogKind::Error, json!({"message": crate::move_lock::MESSAGE}).as_object().cloned().unwrap_or_default())
    };
    // Raised in the frame its own ↩ arrives: that ↩ leaves it open.
    alert(&mut h);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs.len(), 1, "the ↩ that raised the alert doesn't close it");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "↩ is OK");
    // A new alert starts over, and Esc still closes it.
    alert(&mut h);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "Esc too");
}

fn dialog_harness(size: egui::Vec2, command: &str) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    crate::filter_dialog::open(h.state_mut(), command).unwrap();
    h.run_steps(4);
    h
}

/// #3152: Motion Blur's angle is a dial, as in Photoshop. Clicking or dragging it points the
/// blur; the field still takes the whole -360..360 range.
#[test]
fn motion_blur_angle_dial_sets_the_angle_by_click_and_drag() {
    use egui_kittest::kittest::Queryable;
    let mut h = dialog_harness(egui::vec2(1280.0, 800.0), "filter.blur.motionBlur");
    let angle = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.state().ui.dialogs.last().and_then(|d| d.fields["angle"].as_f64());
    let centre = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.get_by_role_and_label(egui::accesskit::Role::Slider, "Angle").rect().center();
    let press = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, pos: egui::Pos2, pressed: bool| {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    };
    assert_eq!(angle(&h), Some(0.0));
    // The dial's field is the dialog's first number, so it opens focused (#1757).
    let first = h.get_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap();
    let distance = h.query_all_by_label("Distance").next().expect("the Distance field").rect();
    assert!(first.is_focused() && first.rect().center().y < distance.center().y, "the Angle field has focus");
    // Click above the centre: straight up is 90°.
    let up = centre(&h) + egui::vec2(0.0, -15.0);
    h.event(egui::Event::PointerMoved(up));
    press(&mut h, up, true);
    h.step();
    press(&mut h, up, false);
    h.run_steps(3);
    assert_eq!(angle(&h), Some(90.0));
    // Drag from the right (0°) round to the lower left (-135°).
    let c = centre(&h);
    let from = c + egui::vec2(15.0, 0.0);
    h.event(egui::Event::PointerMoved(from));
    press(&mut h, from, true);
    h.step();
    for (k, to) in [egui::vec2(12.0, 12.0), egui::vec2(0.0, 15.0), egui::vec2(-12.0, 12.0)].into_iter().enumerate() {
        h.event(egui::Event::PointerMoved(c + to));
        h.step();
        assert_eq!(angle(&h), Some([-45.0, -90.0, -135.0][k]), "dragged to {to:?}");
    }
    press(&mut h, c + egui::vec2(-12.0, 12.0), false);
    h.run_steps(3);
    assert_eq!(angle(&h), Some(-135.0));
    // A remembered or typed angle past half a turn is kept, not clamped to the dial's ±180°.
    if let Some(d) = h.state_mut().ui.dialogs.last_mut() {
        d.fields.insert("angle".into(), json!(270.0));
    }
    h.run_steps(3);
    assert_eq!(angle(&h), Some(270.0));
}

#[test]
fn a_tall_filter_dialog_keeps_its_title_and_buttons_in_a_small_window() {
    // Filter › Render › Flame was taller than a 1000×560 window: OK and Cancel were cut off.
    use egui_kittest::kittest::Queryable;
    let size = egui::vec2(1000.0, 560.0);
    let h = dialog_harness(size, "filter.render.flame");
    for label in ["Flame", "OK", "Cancel"] {
        let r = h.get_by_label(label).rect();
        assert!(r.top() >= 0.0 && r.bottom() <= size.y, "{label} at {r:?} is outside the {size:?} window");
    }
}

#[test]
fn a_moved_dialog_reopens_where_it_was_left() {
    use egui_kittest::kittest::Queryable;
    let size = egui::vec2(1280.0, 800.0);
    let mut h = dialog_harness(size, "filter.blur.gaussianBlur");
    let centred = h.get_by_label("Gaussian Blur").rect();
    // Drag the title bar up and to the left.
    let (from, by) = (centred.center(), egui::vec2(-240.0, -150.0));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    for k in 1..=6 {
        h.event(egui::Event::PointerMoved(from + by * (k as f32 / 6.0)));
        h.step();
    }
    h.event(egui::Event::PointerButton { pos: from + by, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    let moved = h.get_by_label("Gaussian Blur").rect();
    assert!((moved.min - (centred.min + by)).length() < 2.0, "dragged: {centred:?} → {moved:?}");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    // Reopened: where it was left.
    crate::filter_dialog::open(h.state_mut(), "filter.blur.gaussianBlur").unwrap();
    h.run_steps(4);
    let reopened = h.get_by_label("Gaussian Blur").rect();
    assert!((reopened.min - moved.min).length() < 2.0, "reopened at {reopened:?}, left at {moved:?}");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    // Another dialog keeps its own place: never moved, it opens centred.
    crate::filter_dialog::open(h.state_mut(), "filter.blur.motionBlur").unwrap();
    h.run_steps(4);
    let other = h.get_by_label("Motion Blur").rect();
    assert!(other.min.x > moved.min.x + 100.0, "Motion Blur opened centred: {other:?}");
}

/// #1757: like Photoshop, a value dialog opens on its first number with the text selected, so
/// typing replaces it and Enter applies it; it reopens with the value applied last time, and a
/// cancelled edit is not remembered.
#[test]
fn a_value_dialog_opens_on_its_first_number_selected_and_remembers_it() {
    use egui_kittest::kittest::Queryable;
    const BLUR: &str = "filter.blur.gaussianBlur";
    let mut h = dialog_harness(egui::vec2(1280.0, 800.0), BLUR);
    let radius = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.state().ui.dialogs.last().and_then(|d| d.fields["radius"].as_f64());
    assert_eq!(radius(&h), Some(1.0), "the default the first time");
    assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "the Radius field has focus");
    h.event(egui::Event::Text("5".into()));
    h.run_steps(1);
    assert_eq!(radius(&h), Some(5.0), "typing replaced the selected 1");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Enter is OK");
    assert_eq!(h.state().session.journal.last(), Some(&(BLUR.to_string(), json!({"radius": 5.0}))));

    crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
    h.run_steps(4);
    assert_eq!(radius(&h), Some(5.0), "reopens with the last value");
    assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "focused again");
    h.event(egui::Event::Text("9".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Esc is Cancel");
    crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
    h.run_steps(2);
    assert_eq!(radius(&h), Some(5.0), "a cancelled 9 is not remembered");
}

/// #4673: File › Automate › Fit Image holds two numbers and a check box, so its window is as
/// wide as that content and its buttons, not the 380-point floor of the larger dialogs.
#[test]
fn fit_image_is_as_narrow_as_its_content() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let d = crate::menus::invoke(h.state_mut(), &ctx, "file.automate.fitImage", json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    let window = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", d)))).expect("the Fit Image window");
    assert!(window.width() < 300.0, "Fit Image is {} points wide", window.width());
    for label in ["Fit Image", "Width", "Height", "OK", "Cancel"] {
        // A caption and the field it names share the label; either sits inside the window.
        let node = h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label} in the dialog"));
        assert!(window.contains_rect(node.rect()), "{label} inside the window");
    }
}

#[test]
fn dialogs_that_preview_on_the_canvas_go_to_the_bottom_on_a_phone() {
    let dialog = |kind, fields: serde_json::Value| Dialog { id: 1, kind, fields: fields.as_object().cloned().unwrap_or_default() };
    assert!(previews_on_canvas(&dialog(DialogKind::LayerStyle, json!({}))), "Layer Style");
    assert!(previews_on_canvas(&dialog(DialogKind::Command, json!({"__filter": true, "__command": "filter.blur.gaussianBlur"}))), "a filter");
    assert!(!previews_on_canvas(&dialog(DialogKind::Command, json!({"__command": "image.imageSize", "__sizing": true}))), "Image Size");
    assert!(!previews_on_canvas(&dialog(DialogKind::NewDocument, json!({}))), "New Document");
}

/// The phone layout: Preferences, the widest dialog, fits a phone held upright on every section
/// and its OK stays reachable.
#[test]
fn phone_layout_dialogs_fit_the_window() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.compact_layout = true;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 760.0));
    let mut h = egui_kittest::Harness::builder().with_size(screen.size()).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let mut wide = Vec::new();
    for (section, _) in photocraft_engine::prefs::SECTIONS {
        h.state_mut().ui.dialogs.clear();
        h.run_steps(1);
        let d = crate::prefs_ui::open_preferences(h.state_mut(), section);
        h.run_steps(4);
        let window = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", d)))).expect("the Preferences window");
        if !screen.contains_rect(window) {
            wide.push(format!("{section}: {:.0}", window.width()));
        } else {
            assert!(window.contains_rect(h.get_by_label("OK").rect()), "{section}: OK {:?} inside the window {window:?}", h.get_by_label("OK").rect());
        }
    }
    assert!(wide.is_empty(), "sections wider than the phone's {:.0}: {wide:?}", screen.width());
}
