use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;

fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.run_steps(40);
    for _ in 0..2 {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    }
}

fn fill(app: &PhotocraftApp, id: LayerId) -> Option<Fill> {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.fill.clone(),
        _ => None,
    }
}

/// #2812: double-clicking a shape layer's thumbnail opens the Color Picker on its fill; the
/// canvas previews the pick, Cancel changes nothing and OK is one undoable `shape.edit`.
#[test]
fn shape_thumbnail_double_click_picks_the_fill_colour() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(s.execute("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id.0).unwrap().row;
    double_click(&mut h, pos2(row.left() + 46.0, row.center().y));

    let app = h.state_mut();
    assert_eq!(app.session.active().unwrap().active_layer, Some(id));
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.entries().len();
    let dialog = app.ui.dialogs.last().filter(|d| owns(&d.fields)).expect("Color Picker opened on the shape fill");
    assert_eq!(dialog.fields["color"], "#336699");
    let dialog = dialog.id;

    // Picking previews on the canvas without touching the document; Cancel drops it.
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    let (shown, _) = display_doc(app, 0).unwrap();
    let LayerContent::Shape(sh) = &shown.layer(id).unwrap().content else { panic!() };
    assert_eq!(sh.fill, Some(Fill::Solid(Color::rgb(1.0, 136.0 / 255.0, 0.0))));
    app.ui.close_dialog(dialog);
    assert!(display_doc(app, 0).is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "Cancel leaves the shape alone");

    // OK commits the colour as one undo step.
    let dialog = open(app).unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    crate::dialogs::confirm(app, dialog).unwrap();
    assert_eq!(fill(app, id), Some(Fill::Solid(Color::rgb(1.0, 136.0 / 255.0, 0.0))));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

/// The popovers' Color Picker edits the stroke too, previews it on the canvas, and fails
/// gracefully (never panics) when its target goes stale or its fields are garbage.
#[test]
fn stroke_picker_previews_commits_and_rejects_stale_or_bad_targets() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(
        app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699", "stroke": {"width": 4, "color": [1.0, 0.0, 0.0, 0.5]}})).unwrap()["layer"]
            .as_u64()
            .unwrap(),
    );
    let stroke = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.stroke.clone(),
        _ => None,
    };
    let dialog = open_paint(&mut app, Paint::Stroke, Some(id), false, [0.0, 0.0, 0.0, 1.0]).unwrap();
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["color"], "#ff0000", "opens on the stroke colour");
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#00ff00"));
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    let LayerContent::Shape(sh) = &shown.layer(id).unwrap().content else { panic!() };
    assert_eq!(sh.stroke.as_ref().unwrap().paint, Fill::Solid(Color::rgba(0.0, 1.0, 0.0, 0.5)), "previewed with the stroke's alpha");
    assert_eq!(sh.fill, fill(&app, id), "the fill is untouched");
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let after = stroke(&app).unwrap();
    assert_eq!((after.paint, after.width), (Fill::Solid(Color::rgba(0.0, 1.0, 0.0, 0.5)), 4.0));

    // Neither a layer nor the tool options: nothing to open.
    assert!(open_paint(&mut app, Paint::Fill, None, false, [0.0; 4]).is_none());
    // A deleted layer, another document, garbage fields: errors, not panics.
    let dialog = open_paint(&mut app, Paint::Fill, Some(id), false, [0.0; 4]).unwrap();
    let mut f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    for (key, value) in [
        ("__params", json!(null)),
        ("__params", json!({"layer": "x"})),
        ("__params", json!({"layer": 9999, "document": 0, "tool": false})),
        ("color", json!(12)),
    ] {
        let mut bad = f.clone();
        bad.insert(key.into(), value);
        assert!(confirm(&mut app, &bad).is_err(), "{key}");
    }
    f.insert("color".into(), json!("#0000ff"));
    app.run("layer.delete", json!({})).unwrap();
    assert!(confirm(&mut app, &f).is_err(), "the layer is gone");
    assert!(display_doc(&mut app, 0).is_none());
}

/// A shape without a stroke, or with a 0 px one, previews the stroke the picker's OK turns on:
/// the picked colour at the enabled width, even for the colour it opened on.
#[test]
fn stroke_picker_previews_a_stroke_that_is_off_as_ok_turns_it_on() {
    for stroke in [json!(null), json!({"width": 0, "color": "#ff0000"})] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699", "stroke": stroke})).unwrap()["layer"].as_u64().unwrap());
        let dialog = open_paint(&mut app, Paint::Stroke, Some(id), false, [0.0, 0.0, 0.0, 1.0]).unwrap();
        let (shown, _) = display_doc(&mut app, 0).expect("the unchanged colour still previews the stroke OK adds");
        let LayerContent::Shape(sh) = &shown.layer(id).unwrap().content else { panic!() };
        let previewed = sh.stroke.clone().expect("a previewed stroke");
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!() };
        let committed = sh.stroke.clone().expect("OK turned the stroke on");
        assert_eq!((previewed.paint, previewed.width), (committed.paint, committed.width), "{stroke}: the preview is what OK commits");
        assert_eq!(committed.width, 3.0);
    }
}

/// From the options bar without a selected shape, OK sets the next shape's colour and turns the
/// paint on, and edits no document.
#[test]
fn tool_only_picker_sets_the_next_shapes_colour() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool_options.shape_fill = false;
    let history = app.session.active().unwrap().history.past_len();
    let dialog = open_paint(&mut app, Paint::Fill, None, true, [0.1, 0.2, 0.3, 1.0]).unwrap();
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["__label"], "Color Picker (Fill Color)");
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.ui.tool_options.shape_fill);
    assert_eq!(color_picker_ui::hex([app.session.tools.foreground[0], app.session.tools.foreground[1], app.session.tools.foreground[2]]), "#ff8800");
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// With no document open, the options bar's Color Picker still sets the next shape's colour.
#[test]
fn tool_only_picker_works_without_a_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    for which in [Paint::Fill, Paint::Stroke] {
        let dialog = open_paint(&mut app, which, None, true, [0.1, 0.2, 0.3, 1.0]).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
    }
    assert!(app.ui.tool_options.shape_fill);
    assert_eq!(color_picker_ui::hex([app.session.tools.foreground[0], app.session.tools.foreground[1], app.session.tools.foreground[2]]), "#ff8800");
    assert_eq!(color_picker_ui::hex([app.session.tools.background[0], app.session.tools.background[1], app.session.tools.background[2]]), "#ff8800");
    // A picker for a shape layer still needs its document.
    let mut f = Map::new();
    f.insert("__params".into(), json!({"layer": 3, "document": 1, "which": "fill"}));
    f.insert("color".into(), json!("#ff8800"));
    assert_eq!(confirm(&mut app, &f).unwrap_err(), "no document open");
}

/// A locked selected shape refuses the options bar's Color Picker before anything changes: the
/// next shape's colour stays as it was, as does the document.
#[test]
fn locked_shape_refuses_the_options_bar_picker_without_changing_the_next_shapes_colour() {
    for lock in ["all", "pixels"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.lockLayers", json!({lock: true})).unwrap();
        let (fg, options) = (app.session.tools.foreground, app.ui.tool_options.clone());
        let history = app.session.active().unwrap().history.past_len();
        let dialog = open_paint(&mut app, Paint::Fill, Some(id), true, fg).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
        let f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
        assert_eq!(confirm(&mut app, &f).unwrap_err(), "shape layer is locked", "{lock}");
        assert_eq!(app.session.tools.foreground, fg, "{lock}: the next shape's colour is unchanged");
        assert_eq!(app.ui.tool_options, options);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(fill(&app, id), Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.6))));
    }
}

/// A shape edit refused for another reason (here a job holding the document) leaves the next
/// shape's colour and stroke as they were: the edit runs first, the tool defaults only after it.
#[test]
fn a_refused_shape_edit_leaves_the_next_shapes_colour_alone() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    for which in [Paint::Fill, Paint::Stroke] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
        let (tools, options) = (app.session.tools.clone(), app.ui.tool_options.clone());
        let dialog = open_paint(&mut app, which, Some(id), true, [0.0, 0.0, 0.0, 1.0]).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
        let f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
        let gate = Arc::new(AtomicBool::new(false));
        let open = gate.clone();
        let started = app
            .session
            .start_job(
                "test.hold",
                json!({}),
                "Hold",
                true,
                move |ctx| {
                    while !open.load(Ordering::Relaxed) {
                        ctx.check()?;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Ok(())
                },
                |_, ()| Ok(json!({})),
            )
            .unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("the job ran inline") };
        assert!(confirm(&mut app, &f).unwrap_err().contains("still running"), "{which:?}");
        assert_eq!(app.session.tools.foreground, tools.foreground, "{which:?}: the next shape's colours are unchanged");
        assert_eq!(app.session.tools.background, tools.background);
        assert_eq!(app.ui.tool_options, options, "{which:?}: the stroke stays off");
        gate.store(true, Ordering::Relaxed);
        app.session.cancel_job(job);
        app.session.join_cancelled_jobs();
    }
}
