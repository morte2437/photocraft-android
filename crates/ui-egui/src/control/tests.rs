use super::*;

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// #3674: a guessed name gets a pointer to the right one, in both directions; and every
/// method listed for those hints is served.
#[test]
fn unknown_names_suggest_the_right_method_or_command() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let mut error = |method: &str, params: Value| {
        let reply = call(&mut app, &ctx, method, params);
        assert_eq!(reply["ok"], false, "{reply}");
        reply["error"].as_str().unwrap_or_default().to_owned()
    };
    let e = error("engine.execute", json!({"command": "engine.commands"}));
    assert!(e.contains("did you mean `command.list`") && e.contains("protocol method"), "{e}");
    let e = error("engine.execute", json!({"command": "ui.inspect"}));
    assert!(e.contains("`ui.inspect` is a protocol method"), "{e}");
    let e = error("engine.execute", json!({"command": "edit.undoo"}));
    assert!(e.contains("did you mean `edit.undo`"), "{e}");
    let e = error("ui.inspct", json!({}));
    assert!(e.contains("did you mean `ui.inspect`"), "{e}");
    let e = error("edit.undo", json!({}));
    assert!(e.contains("engine.execute {\"command\": \"edit.undo\"}"), "{e}");
    for m in METHODS {
        let (req, _rx) = ControlRequest::new(*m, json!({}));
        if let Outcome::Done(v) = handle(&mut app, &ctx, &req) {
            assert!(!v.to_string().contains("unknown method"), "{m} is listed but not served: {v}");
        }
    }
}

#[test]
fn shift_right_brush_does_not_open_presets_or_paint() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = crate::state::Tool::Brush;
    let ctx = egui::Context::default();
    let before = app.session.journal.len();
    call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({"button": "right", "shift": true,
        "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "up", "x": 20, "y": 20}]}),
    );
    assert!(app.ui.brush_picker.is_none(), "Shift+right opens Mode, not Brush Presets");
    assert!(app.drag.is_none());
    assert_eq!(app.session.journal.len(), before, "opening a menu does not paint or journal");
}

#[test]
fn brush_mode_control_routes_modifiers_and_reports_choices_without_history() {
    let ctx = egui::Context::default();
    for (tool, modifiers, expected) in [
        ("brush", json!({"shift": true}), "mode"),
        ("brush", json!({}), "erase"),
        ("brush", json!({"alt": true}), "resize"),
        ("brush", json!({"shift": true, "alt": true}), "erase"),
        ("brush", json!({"shift": true, "ctrl": true}), if cfg!(target_os = "macos") { "erase" } else { "layer" }),
        ("brush", json!({"shift": true, "command": true}), "layer"),
        ("pencil", json!({"shift": true}), "preset"),
    ] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        let history = app.session.active().unwrap().history.past_len();
        call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({"tool": tool, "button": "right", "modifiers": modifiers,
            "events": [{"kind": "down", "x": 20, "y": 20}]}),
        );
        assert_eq!(app.ui.brush_mode_menu.is_some(), expected == "mode", "{tool}: {modifiers}");
        match expected {
            "mode" => {
                assert!(app.drag.is_none() && !app.secondary_erase);
                assert_eq!(app.session.active().unwrap().history.past_len(), history);
                let inspect = inspect(&app, &ctx);
                assert_eq!(inspect["brushModeMenu"]["choices"], json!(crate::brush_mode_menu::choices().iter().map(|(_, label)| *label).collect::<Vec<_>>()));
                let journal = app.session.journal.len();
                call(
                    &mut app,
                    &ctx,
                    "ui.pointer",
                    json!({"events": [
                    {"kind": "down", "x": 10, "y": 10}, {"kind": "move", "x": 30, "y": 30}, {"kind": "up", "x": 30, "y": 30}]}),
                );
                assert!(app.ui.brush_mode_menu.is_none() && app.drag.is_none());
                assert_eq!(app.session.journal.len(), journal, "control dismissal cannot paint");
            }
            "preset" => assert!(app.ui.brush_picker.is_some()),
            "layer" => assert!(app.ui.layer_menu.is_some()),
            "resize" => assert!(app.brush_resize.is_some()),
            "erase" => {
                assert!(app.drag.as_ref().is_some_and(|d| d.erase));
                assert!(app.brush_resize.is_none(), "Shift+Alt is not the resize gesture");
            }
            _ => panic!("unknown fixture route: {expected}"),
        }
    }
}

#[test]
fn ui_set_turns_the_stroke_trace_on_and_off_at_runtime() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert!(!app.stroke_trace.enabled());
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"strokeTrace": true}))["ok"], true);
    assert!(app.stroke_trace.enabled());
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"strokeTrace": "off"}))["ok"], true);
    assert!(!app.stroke_trace.enabled());
    let bad = call(&mut app, &ctx, "ui.set", json!({"strokeTrace": 3, "tool": "Pencil"}));
    assert_eq!(bad["ok"], false);
    // A file path isn't accepted over the channel (it would write outside the automation
    // write root); nothing is created.
    let path = std::env::temp_dir().join(format!("photocraft-trace-{}.csv", std::process::id()));
    let bad = call(&mut app, &ctx, "ui.set", json!({"strokeTrace": path.to_str().unwrap(), "tool": "Pencil"}));
    assert_eq!(bad["ok"], false);
    assert!(!path.exists());
    assert!(!app.stroke_trace.enabled() && app.ui.tool != crate::state::Tool::Pencil, "a rejected call applies nothing");
}

#[test]
fn perf_output_includes_slow_frames() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"], json!([]));
    app.slow_frames.set_threshold_ms(0.0);
    app.run("file.new", json!({"width": 64, "height": 32})).unwrap();
    app.finish_frame(1.0);
    let frames = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"].clone();
    let last = frames.as_array().and_then(|f| f.last()).expect("a slow frame").clone();
    assert_eq!(last["command"], "file.new", "{last}");
    for key in ["t", "frameMs", "inputMs", "commandMs", "damageMs", "compositeMs", "uploadMs", "gpuMs", "presentMs", "refresh", "refreshPx", "path"] {
        assert!(last.get(key).is_some(), "{key} in {last}");
    }
    // A nested run (`actions.play` → its view steps through `App::run`) is inside the outer
    // call's time: the frame counts the outer command once, not the steps again.
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
        name: "Zoom".into(),
        steps: vec![("view.zoomIn".into(), json!({})), ("view.zoomOut".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let played = app.run("actions.play", json!({"action": "Zoom"})).unwrap();
    assert_eq!(played["ran"], 2, "{played}");
    let outer_ms = app.perf.command_ms;
    app.finish_frame(1.0);
    let frames = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["perf"]["slowFrames"].clone();
    let last = frames.as_array().and_then(|f| f.last()).expect("a slow frame").clone();
    assert_eq!(last["command"], "actions.play", "{last}");
    let command_ms = last["commandMs"].as_f64().unwrap();
    assert!((command_ms - outer_ms).abs() < 1e-9, "the outer run's {outer_ms} ms, once: {last}");
}

#[test]
fn camera_raw_control_updates_and_commits_or_cancels_an_open_raw() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (48, 32);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let dng = spec.build();
    for finish in ["commit", "cancel"] {
        let services = crate::Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        app.open_bytes("control.dng", &dng).unwrap();
        ctx.run_ui(Default::default(), |ui| crate::camera_raw_ui::show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.camera_raw.is_some());
        // A developed raw has no embedded profile to ask about (`open_prompt`), so no profile
        // prompt stands between the open and the Camera Raw dialog.
        assert!(app.ui.dialogs.is_empty());
        let update = json!({"id":"filter.cameraRaw","params":{"ui":{"set":{"exposure":0.5}}}});
        let original = app.session.active().unwrap().doc.clone();
        let changed = call(&mut app, &ctx, "ui.menu.invoke", update.clone());
        let undone = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.cameraRaw","params":{"ui":{"undo":true}}}));
        assert_eq!(undone["result"]["params"]["exposure"], 0.0, "{undone}");
        let redone = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.cameraRaw","params":{"ui":{"redo":true}}}));
        assert_eq!(redone["result"]["params"]["exposure"], 0.5, "{redone}");
        assert_eq!(changed["ok"], true, "{changed}");
        assert_eq!(changed["result"]["params"]["exposure"], 0.5);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original), "a preview must not commit");
        assert!(!crate::menus::modal_allows(&app, "filter.cameraRaw"), "native menu clicks stay blocked");
        let finished = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.cameraRaw","params":{"ui":{finish:true}}}));
        assert_eq!(finished["ok"], true, "{finished}");
        assert!(app.camera_raw.is_none());
        if finish == "commit" {
            assert_eq!(finished["result"]["redeveloped"], true, "{finished}");
            assert_eq!(app.session.documents().len(), 1);
            let st = app.session.active().unwrap();
            assert_ne!(st.doc.id, original.id);
            assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
            assert_eq!(st.saved_revision, st.revision);
        } else {
            assert_eq!(finished["result"]["cancelled"], true);
            assert!(app.session.documents().is_empty());
        }
    }
}

#[test]
fn camera_raw_control_continuation_does_not_bypass_other_modal_or_command_gates() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#808080"})).unwrap();
    let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.cameraRaw"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let original = app.session.active().unwrap().doc.clone();
    let cancel = json!({"id":"filter.cameraRaw","params":{"ui":{"cancel":true}}});
    for request in [
        json!({"id":"file.new"}),
        json!({"id":"view.zoomIn"}),
        json!({"id":"filter.cameraRaw"}),
        json!({"id":"filter.cameraRaw","params":{"exposure":1}}),
        json!({"id":"filter.cameraRaw","params":{"ui":null}}),
        json!({"id":"filter.cameraRaw","params":{"ui":{},"smartFilter":{"layer":1,"index":0}}}),
    ] {
        let blocked = call(&mut app, &ctx, "ui.menu.invoke", request);
        assert_eq!(blocked["ok"], false, "{blocked}");
    }
    app.ui.open_dialog(DialogKind::About, Default::default());
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel.clone())["ok"], false);
    app.ui.dialogs.clear();
    assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({})));
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel.clone())["ok"], false);
    app.discard = None;
    app.ui.tool = Tool::Crop;
    app.ui.crop_rect = Some([1.0, 1.0, 10.0, 10.0]);
    assert!(crate::crop_ui::blocks(&app, "filter.cameraRaw"));
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel.clone())["ok"], false);
    app.ui.crop_rect = None;
    app.services.automation_command = Some(Box::new(|id, _| if id == "filter.cameraRaw" { Err("Camera Raw denied".into()) } else { Ok(()) }));
    for ui in [json!({"set":{"exposure":1}}), json!({"commit":true}), json!({"cancel":true})] {
        let denied = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.cameraRaw","params":{"ui":ui}}));
        assert_eq!(denied["ok"], false, "{denied}");
        assert_eq!(denied["error"], "Camera Raw denied");
    }
    assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 0.0);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
    app.services.automation_command = None;
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel)["ok"], true);
    assert!(app.camera_raw.is_none());
}

#[test]
fn full_window_workspaces_are_modal_for_menu_invocations() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#808080"})).unwrap();
    app.run("select.rect", json!({"x":8,"y":8,"width":8,"height":8})).unwrap();
    for workspace in ["filter.liquify", "edit.contentAwareFill", "filter.filterGallery", "filter.adaptiveWideAngle", "select.selectAndMask"] {
        let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id":workspace}));
        assert_eq!(opened["ok"], true, "{workspace}: {opened}");
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(workspace));
        let doc = app.session.active().unwrap().doc.id;
        for request in [
            json!({"id":"window.document.0"}),
            json!({"id":"select.all"}),
            json!({"id":"view.zoomIn"}),
            json!({"id":"filter.cameraRaw"}),
            json!({"id":"filter.liquify"}),
            json!({"id":workspace,"params":{"ui":null}}),
        ] {
            let blocked = call(&mut app, &ctx, "ui.menu.invoke", request.clone());
            assert_eq!(blocked["ok"], false, "{workspace}: {request} → {blocked}");
            assert!(!crate::menus::modal_allows(&app, request["id"].as_str().unwrap()), "{workspace}: native menu greys {request}");
        }
        let listed = call(&mut app, &ctx, "ui.menu.list", json!({}));
        let enabled = |id: &str| listed["result"].as_array().unwrap().iter().find(|it| it["id"] == id).map(|it| it["enabled"].clone());
        assert_eq!(enabled("select.all"), Some(json!(false)), "{workspace}: ui.menu.list greys what invoke refuses");
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(workspace), "{workspace} stays open with its work");
        assert_eq!(app.session.active().unwrap().doc.id, doc);
        // An open dialog still takes precedence over the workspace's own requests.
        app.ui.open_dialog(DialogKind::About, Default::default());
        let cancel = json!({"id":workspace,"params":{"ui":{"cancel":true}}});
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", cancel.clone())["ok"], false);
        app.ui.dialogs.clear();
        let cancelled = call(&mut app, &ctx, "ui.menu.invoke", cancel);
        assert_eq!(cancelled["ok"], true, "{workspace}: {cancelled}");
        assert_eq!(crate::distort_ui::modal_workspace(&app), None);
        assert!(crate::menus::modal_allows(&app, "select.all"), "{workspace}: menus are free again");
        let listed = call(&mut app, &ctx, "ui.menu.list", json!({}));
        assert!(
            listed["result"].as_array().unwrap().iter().any(|it| it["id"] == "select.all" && it["enabled"] == true),
            "{workspace}: listed as enabled again"
        );
    }
    // Programmatic edits cannot bypass the active workspace ownership gate.
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.liquify"}))["ok"], true);
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let executed = call(&mut app, &ctx, "engine.execute", json!({"command":"select.all"}));
    assert_eq!(executed["ok"], false, "{executed}");
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"filter.liquify","params":{"ui":{"cancel":true}}}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "engine.execute", json!({"command":"select.all"}))["ok"], true);
}

#[test]
fn layer_style_picker_control_binds_previews_confirms_and_cancels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    let parent = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"layerStyle","effect":"gradientOverlay"}))["result"]["dialog"].as_u64();
    let parent = parent.unwrap();
    let before = app.ui.dialogs[0].fields.clone();
    // The Gradient Overlay's colours are its stops (#5177): `stop` picks one.
    for stop in [0, 1] {
        let reply = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":stop}));
        assert_eq!(reply["ok"], true, "{reply}");
        let picker = app.ui.dialogs.last().unwrap().id;
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert!(inspected.to_string().contains("__layerStyleColor"));
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":parent}))["ok"], false);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":picker,"field":"color","value":"#123456"}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":picker}))["ok"], true);
        assert_eq!(app.ui.dialogs[0].fields["effects"][0]["params"]["stops"][stop][1], "#123456");
    }
    assert_ne!(app.ui.dialogs[0].fields, before);
    let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":"x"}));
    assert_eq!(bad["ok"], false, "{bad}");
    let bad = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":9}));
    assert_eq!(bad["ok"], false, "{bad}");
    call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"stops","stop":1}));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog":parent}))["ok"], true);
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn text_style_picker_control_uses_bound_target_and_rejects_invalid_requests_atomically() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":32,"height":32})).unwrap();
    let id = app.run("type.characterStyle.new", json!({"fromSelection":false,"attrs":{"color":"#123456"}})).unwrap()["id"].as_u64().unwrap();
    for style in [
        Value::Null,
        json!({"kind":"wrong","id":id}),
        json!({"kind":"character","id":"bad"}),
        json!({"kind":"paragraph","id":u64::MAX}),
        json!({"kind":"character","id":0}),
        json!({"kind":"character","id":id,"extra":1}),
    ] {
        let panels = app.ui.type_panels.clone();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","textStyle":style}))["ok"], false);
        assert_eq!(app.ui.type_panels, panels);
        assert!(app.ui.dialogs.is_empty());
    }
    assert_eq!(call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","textStyle":{"kind":"character","id":id},"parent":1}))["ok"], false);
    let doc = app.session.active().unwrap().doc.clone();
    let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","textStyle":{"kind":"character","id":id}}));
    assert_eq!(r["ok"], true);
    let picker = r["result"]["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialogs[0].fields["__textStyleColor"]["id"], id);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":picker,"field":"color","value":"#00ff00"}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog":picker}))["ok"], true);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
    let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"colorPicker","textStyle":{"kind":"character","id":id}}));
    let picker = r["result"]["dialog"].as_u64().unwrap();
    call(&mut app, &ctx, "ui.dialog.set", json!({"dialog":picker,"field":"color","value":"#00ff00"}));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":picker}))["ok"], true);
    assert_ne!(app.session.active().unwrap().doc.text_styles, doc.text_styles);
}

#[test]
fn layer_style_picker_control_rejects_bad_targets_without_opening_tool_picker() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    let parent = crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap();
    for params in [
        json!({"kind":"colorPicker","parent":null,"effect":"fx1","field":"color"}),
        json!({"kind":"colorPicker","parent":"bad","effect":"fx1","field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":3,"field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"missing","field":"color"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"opacity"}),
        json!({"kind":"colorPicker","parent":parent,"effect":"fx1","field":"to"}),
        json!({"kind":"colorPicker","effect":"fx1","field":"color"}),
    ] {
        let before = app.ui.dialogs.clone();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.open", params)["ok"], false);
        assert_eq!(app.ui.dialogs, before);
    }
}

#[test]
fn shape_stroke_defaults_are_drivable_and_validate_atomically() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let result =
        call(&mut app, &ctx, "ui.set", json!({"tool":"Triangle", "shapeStroke":{"width":5,"cap":"round","join":"bevel","dashes":[0,2,4,1],"dashOffset":-0.5}}));
    assert_eq!(result["ok"], true);
    assert_eq!(app.ui.tool_options.stroke_width, 5.0);
    assert_eq!(app.ui.tool_options.shape_stroke.dashes, [0.0, 2.0, 4.0, 1.0]);
    let before = app.ui.clone();
    let bad = call(&mut app, &ctx, "ui.set", json!({"tool":"Rectangle","shapeStroke":{"dashes":[0,0]}}));
    assert_eq!(bad["ok"], false);
    assert_eq!(app.ui, before);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"shapeStroke":{"colour":"#ff0000"}}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"shapeStroke":null}))["ok"], true);
    assert_eq!(app.ui.tool_options.stroke_width, 0.0);
}

#[test]
fn the_unsaved_changes_prompt_is_listed_and_answered_through_ui_dialog() {
    // #4920: the prompt shown on quit or close with unsaved changes wasn't in ui.inspect, and a
    // control client couldn't answer it.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    for _ in 0..2 {
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
    }
    let prompt = |app: &mut PhotocraftApp| {
        let v = call(app, &ctx, "ui.inspect", json!({}));
        v["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["id"] == "discard").cloned()
    };
    assert!(prompt(&mut app).is_none());
    // Closing a dirty document: listed with its choices; a bad choice and Cancel keep the document.
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    let d = prompt(&mut app).expect("the prompt is listed");
    assert_eq!(d["fields"]["command"], "file.close");
    assert_eq!(d["fields"]["choices"], json!(["save", "dontSave", "cancel"]));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "discard", "choice": "maybe"}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "discard", "field": "x", "value": 1}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "discard"}))["ok"], true);
    assert!(prompt(&mut app).is_none() && app.session.documents().len() == 2);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "discard"}))["ok"], false, "nothing left to answer");
    // Don't Save closes it.
    crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "discard", "choice": "dontSave"}))["ok"], true);
    assert_eq!(app.session.documents().len(), 1);
    // Quitting asks about each dirty document, then lets the window close.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(crate::discard_ui::intercept(&mut app, "file.exit", &Value::Null));
    let d = prompt(&mut app).expect("the quit prompt is listed");
    assert_eq!((d["fields"]["command"].as_str(), d["fields"]["remaining"].as_u64()), (Some("file.exit"), Some(2)));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "discard", "choice": "dontSave"}))["result"]["open"], true);
    assert_eq!(prompt(&mut app).unwrap()["fields"]["remaining"], 1);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "discard", "choice": "dontSave"}))["result"]["open"], false);
    assert!(app.allow_close && app.session.documents().is_empty());
}

#[test]
fn shell_dialogs_are_listed_and_driven_through_ui_dialog() {
    // #1004: Window › Workspace and View dialogs live in `ui.shell.dialog`, and ui.inspect and
    // ui.dialog.* only knew `ui.dialogs`.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let shell = |app: &mut PhotocraftApp| {
        let v = call(app, &ctx, "ui.inspect", json!({}));
        v["result"]["dialogs"].as_array().unwrap().iter().find(|d| d["id"] == "shell").cloned()
    };
    // Listed, then cancelled.
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({})).unwrap();
    let d = shell(&mut app).expect("the open shell dialog is listed");
    assert_eq!((d["kind"].as_str(), d["title"].as_str()), (Some("newWorkspace"), Some("New Workspace")));
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "shell"}))["ok"], true);
    assert!(app.ui.shell.dialog.is_none() && shell(&mut app).is_none());
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": "shell"}))["ok"], false, "nothing left to cancel");
    // A field set over control reaches OK.
    crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", json!({})).unwrap();
    assert!(app.ui.view.show.notes);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "shell", "field": "notes", "value": false}))["ok"], true);
    assert_eq!(shell(&mut app).unwrap()["fields"]["notes"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "shell"}))["ok"], true);
    assert!(!app.ui.view.show.notes && app.ui.shell.dialog.is_none());
    // An OK that fails keeps the dialog open and says why.
    crate::menus::invoke(&mut app, &ctx, "view.show.showExtrasOptions", json!({})).unwrap();
    call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": "shell", "field": "bogus", "value": true}));
    let r = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": "shell"}));
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap_or("").contains("bogus"), "{r}");
    assert!(app.ui.shell.dialog.is_some());
    // Apply is Preferences-only; numeric ids still address ordinary dialogs only.
    assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": "shell"}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": 0}))["ok"], false);
    assert!(app.ui.shell.dialog.is_some());
}

#[test]
fn opening_new_document_through_control_matches_clipboard_image_size() {
    // #2034: every way of opening New Document uses the clipboard-aware initial fields.
    let clipboard = std::sync::Arc::new(std::sync::Mutex::new(Some((100, 200, vec![255; 100 * 200 * 4]))));
    let image = clipboard.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services { clipboard_get_image: Some(Box::new(move || Ok(image.lock().unwrap().clone()))), ..Default::default() },
    );
    let ctx = egui::Context::default();
    let opened = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"newDocument"}));
    assert_eq!(opened["ok"], true);
    let id = opened["result"]["dialog"].as_u64().expect("dialog id is returned");
    let fields = &app.ui.dialog_mut(id).expect("dialog was opened").fields;
    assert_eq!((fields["width"].as_u64(), fields["height"].as_u64()), (Some(100), Some(200)));
    assert_eq!(fields["__preset"], "Clipboard");
}

#[test]
fn expired_queued_edit_does_not_run_and_a_live_retry_runs_once() {
    use std::time::{Duration, Instant};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let before = app.session.active().unwrap();
    let layers = before.doc.layers.len();
    let steps = before.history.past_len();
    let (tx, rx) = std::sync::mpsc::channel();
    app = app.with_control(rx);
    let (mut expired, expired_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
    expired.deadline = Some(Instant::now() - Duration::from_secs(1));
    tx.send(expired).unwrap();
    let (mut retry, retry_reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
    retry.deadline = Some(Instant::now() + Duration::from_secs(60));
    tx.send(retry).unwrap();
    app.drain_control(&egui::Context::default());
    let expired_error = expired_reply.try_recv().unwrap()["error"].as_str().unwrap().to_string();
    assert_eq!(retry_reply.try_recv().unwrap()["ok"], true);
    let after = app.session.active().unwrap();
    assert_eq!(after.doc.layers.len(), layers + 1, "only the live retry adds a layer");
    assert_eq!(after.history.past_len(), steps + 1, "the expired edit adds no undo step");
    // #3662: a caller must be able to tell this reply, safe to re-send, from the
    // transport's, sent while the command may still apply.
    assert!(expired_error.starts_with("timeout"), "machines match on the prefix: {expired_error}");
    assert!(expired_error.contains("not run"), "the expired request says it never ran: {expired_error}");
    let transport_error = super::timeout_error();
    assert!(!transport_error.contains("not run"), "the transport can't know it didn't run: {transport_error}");
    assert!(transport_error.contains("may still apply"), "the transport warns against a blind re-send: {transport_error}");
}

#[test]
fn a_transport_that_gives_up_learns_whether_the_command_can_still_run() {
    // #3662: the transport's clock runs out no later than the UI thread's, so it is the
    // one that answers; it must still tell a request that will never run from one that may.
    use std::time::{Duration, Instant};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let layers = app.session.active().unwrap().doc.layers.len();
    let (tx, rx) = std::sync::mpsc::channel();
    app = app.with_control(rx);
    let ctx = egui::Context::default();
    let request = |deadline: Instant| {
        let (mut req, reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
        req.deadline = Some(deadline);
        (req.dispatch.clone(), req, reply)
    };

    let (dispatch, queued, _) = request(Instant::now() + Duration::from_secs(60));
    tx.send(queued).unwrap();
    assert_eq!(dispatch.give_up(), super::expired_error(), "still queued: it will not run");
    app.drain_control(&ctx);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "a request given up on never runs");

    let (dispatch, expired, _) = request(Instant::now() - Duration::from_secs(1));
    tx.send(expired).unwrap();
    app.drain_control(&ctx);
    assert_eq!(dispatch.give_up(), super::expired_error(), "rejected by the UI thread: it did not run");

    let (dispatch, live, reply) = request(Instant::now() + Duration::from_secs(60));
    tx.send(live).unwrap();
    app.drain_control(&ctx);
    assert_eq!(reply.try_recv().unwrap()["ok"], true);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(dispatch.give_up(), super::timeout_error(), "dispatched: it may still apply");
}

#[test]
fn timeout_errors_name_the_likely_causes() {
    // #2489: the reply must tell "wait" from "check the machine", and its number
    // must come from the deadline it reports.
    let error = super::timeout_error();
    assert!(error.starts_with("timeout"), "machines match on the prefix");
    assert!(error.contains(&format!("{}s", super::REPLY_DEADLINE.as_secs())), "the text matches the deadline: {error}");
    assert!(error.contains("may still be running"), "the wait-it-out cause");
    assert!(error.contains("display asleep"), "the check-the-display cause");
    assert!(error.contains("frame"), "the frame-loop cause");
}

#[test]
fn deadline_free_requests_run_even_after_the_receiver_is_dropped() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let layers = app.session.active().unwrap().doc.layers.len();
    let (tx, rx) = std::sync::mpsc::channel();
    app = app.with_control(rx);
    let (req, reply) = ControlRequest::new("engine.execute", json!({"command": "layer.new.layer"}));
    assert!(req.deadline.is_none());
    drop(reply);
    tx.send(req).unwrap();
    app.drain_control(&egui::Context::default());
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn right_pointer_opens_agent_visible_selection_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    let result = call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({
            "tool": "RectMarquee", "button": "secondary",
            "events": [{"kind": "down", "x": 4, "y": 4}, {"kind": "up", "x": 4, "y": 4}]
        }),
    );
    assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    assert!(entries.iter().any(|entry| entry.get("id") == Some(&json!("select.inverse")) && entry.get("enabled") == Some(&json!(true))));
    assert!(app.session.active().unwrap().doc.selection.is_some(), "right-click must not edit selection");
}

#[test]
fn agent_right_click_with_the_zoom_tool_opens_and_runs_the_view_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let result =
        call(&mut app, &ctx, "ui.pointer", json!({"tool": "Zoom", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]}));
    assert_eq!(result["ok"], true);
    assert_eq!(app.ui.canvas_tool_menu.as_ref().and_then(|m| m.anchor), Some([8.0, 8.0]));
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    let ids: Vec<_> = entries.iter().filter_map(|row| row.get("id").and_then(Value::as_str)).collect();
    assert_eq!(ids, ["view.fitOnScreen", "view.fillScreen", "view.actualPixels", "view.twoHundredPercent", "view.printSize", "view.zoomIn", "view.zoomOut"]);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"select.all"}))["ok"], false);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"view.twoHundredPercent"}))["ok"], true);
    assert_eq!(app.ui.views[0].zoom, 2.0);
    assert!(app.ui.canvas_tool_menu.is_none());
}

/// #6561: an agent's right-click on a pending crop opens the crop menu, as on the canvas.
#[test]
fn agent_right_click_on_a_pending_crop_opens_and_runs_the_crop_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Crop;
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.extras.snap = false;
    crate::crop_ui::ensure_frame(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 4.0, y: 4.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 16.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 28.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 28.0, y: 20.0 }, egui::Modifiers::NONE);
    let frame = app.ui.crop_rect.unwrap();
    let result = call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({"tool": "Crop", "button": "secondary", "events": [{"kind":"down","x":16,"y":12},{"kind":"up","x":16,"y":12}]}),
    );
    assert_eq!(result["ok"], true);
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(inspected.pointer("/result/canvasToolMenu/crop"), Some(&json!(true)));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    assert!(entries.iter().any(|row| row.get("id") == Some(&json!("crop.commit")) && row.get("enabled") == Some(&json!(true))));
    assert_eq!(app.ui.crop_rect, Some(frame), "the right-click leaves the frame alone");
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"crop.commit"}))["ok"], true);
    let size = app.session.active().unwrap().doc.size;
    assert_eq!([f64::from(size.width), f64::from(size.height)], [frame[2] - frame[0], frame[3] - frame[1]], "cropped to the frame");
}

/// An agent's right-click while its own crop frame drag is still down opens no menu over it;
/// after the drag's release it does.
#[test]
fn agent_right_click_during_its_crop_drag_opens_no_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Crop;
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.extras.snap = false;
    crate::crop_ui::ensure_frame(&mut app);
    let pointer = |app: &mut PhotocraftApp, button: &str, events: Value| {
        assert_eq!(call(app, &ctx, "ui.pointer", json!({"tool": "Crop", "button": button, "events": events}))["ok"], true);
    };
    // The first drag on the default frame (nothing pending yet): no pasteboard menu outside the image.
    pointer(&mut app, "left", json!([{"kind":"down","x":4,"y":4},{"kind":"move","x":16,"y":12},{"kind":"move","x":28,"y":20}]));
    assert!(app.crop.drag.is_some() && !crate::crop_ui::pending(&app), "the first frame is still being drawn");
    pointer(&mut app, "secondary", json!([{"kind":"down","x":-8,"y":-8},{"kind":"up","x":-8,"y":-8}]));
    assert!(app.ui.canvas_tool_menu.is_none(), "no pasteboard menu over the first drag");
    pointer(&mut app, "left", json!([{"kind":"up","x":28,"y":20}]));
    // A second drag, on the drawn frame, still down.
    pointer(&mut app, "left", json!([{"kind":"down","x":16,"y":12},{"kind":"move","x":18,"y":14}]));
    assert!(app.crop.drag.is_some() && crate::crop_ui::pending(&app), "the frame drag is still down");
    let right_click = json!([{"kind":"down","x":16,"y":12},{"kind":"up","x":16,"y":12}]);
    pointer(&mut app, "secondary", right_click.clone());
    assert!(app.ui.canvas_tool_menu.is_none(), "no menu over the drag");
    pointer(&mut app, "left", json!([{"kind":"up","x":18,"y":14}]));
    pointer(&mut app, "secondary", right_click.clone());
    assert!(app.ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()), "after the release it opens");
    // Nor over a Straighten line still being drawn.
    app.ui.canvas_tool_menu = None;
    crate::crop_straighten::toggle(&mut app);
    pointer(&mut app, "left", json!([{"kind":"down","x":6,"y":6},{"kind":"move","x":20,"y":8}]));
    assert!(app.crop.straighten.line.is_some(), "the Straighten line is still down");
    pointer(&mut app, "secondary", right_click);
    assert!(app.ui.canvas_tool_menu.is_none(), "no menu over the Straighten line");
}

#[test]
fn agent_can_inspect_and_choose_pen_make_selection_from_full_context_menu() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[2,2],[20,2],[20,20]]}]}})).unwrap();
    let expected_pos = screen_point(&app, 8.0, 8.0);
    let result = call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({
            "tool": "Pen", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]
        }),
    );
    assert_eq!(result["ok"], true);
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(app.ui.canvas_tool_menu.as_ref().unwrap().pos, expected_pos);
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
    assert_eq!(entries.iter().filter(|row| row.get("separator") == Some(&json!(true))).count(), 9);
    assert!(entries.iter().any(|row| row.get("id") == Some(&json!("path.toSelection")) && row.get("enabled") == Some(&json!(true))));
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"file.new"}))["ok"], false);
    let chosen = call(&mut app, &ctx, "ui.context.choose", json!({"id":"path.toSelection"}));
    assert_eq!(chosen["ok"], true);
    let dialog = chosen["result"]["dialog"].as_u64().unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn ui_inspect_reports_the_view_preferences() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let before = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(before.pointer("/result/view/show/selection_edges"), Some(&json!(true)));
    assert_eq!(before.pointer("/result/view/screen_mode"), Some(&json!("standard")));
    assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "view.show.selectionEdges"}))["ok"], true);
    let after = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(after.pointer("/result/view/show/selection_edges"), Some(&json!(false)));
    assert_eq!(after["result"]["view"], json!(app.ui.view));
}

#[test]
fn engine_execute_runs_with_defaults_but_menu_invoke_opens_the_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let rev = app.session.active().unwrap().revision;
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "filter.blur.gaussianBlur"}));
    assert!(!r.to_string().contains("\"dialog\""), "engine.execute opened a dialog: {r}");
    assert!(app.session.active().unwrap().revision > rev, "engine.execute didn't run the filter");
    let rev = app.session.active().unwrap().revision;
    let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
    assert!(r.to_string().contains("dialog"), "ui.menu.invoke should open the dialog: {r}");
    assert_eq!(app.session.active().unwrap().revision, rev, "opening a dialog must not edit the document");
}

#[test]
fn menu_invoke_cannot_edit_document_behind_an_open_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();

    let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageSize"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let dialog = opened["result"]["dialog"].as_u64().unwrap();
    let revision = app.session.active().unwrap().revision;
    let before = call(&mut app, &ctx, "ui.inspect", json!({}));
    let fields = before["result"]["dialogs"][0]["fields"].clone();

    for params in [json!({"id": "image.imageRotation.90cw"}), json!({"id": "image.imageRotation.90cw", "params": {}})] {
        let blocked = call(&mut app, &ctx, "ui.menu.invoke", params);
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(app.session.active().unwrap().revision, revision);
        let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
        assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
    }

    // View navigation stays permitted by the same policy as native menus and shortcuts.
    assert!(crate::menus::modal_allows(&app, "view.zoomIn"));
    assert!(!crate::menus::modal_allows(&app, "image.imageRotation.90cw"));

    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    let rotated = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.imageRotation.90cw"}));
    assert_eq!(rotated["ok"], true, "{rotated}");
    assert!(app.session.active().unwrap().revision > revision);
}

#[test]
fn engine_execute_cannot_change_document_behind_adjustment_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 60, "height": 40, "background": "#3366cc"})).unwrap();
    let opened = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "image.adjustments.hueSaturation"}));
    assert_eq!(opened["ok"], true, "{opened}");
    let dialog = opened["result"]["dialog"].as_u64().unwrap();
    let revision = app.session.active().unwrap().revision;
    let fields = call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["dialogs"][0]["fields"].clone();

    for method in ["engine.execute", "ui.menu.invoke"] {
        let blocked = call(&mut app, &ctx, method, json!({"command": "image.adjustments.invert"}));
        assert_eq!(blocked["ok"], false, "{method}: {blocked}");
        assert_eq!(app.session.active().unwrap().revision, revision, "{method} changed the document");
        let snapshot = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(snapshot["result"]["dialogs"][0]["id"], dialog);
        assert_eq!(snapshot["result"]["dialogs"][0]["fields"], fields);
    }

    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "engine.execute", json!({"command": "image.adjustments.invert"}))["ok"], true);
    assert!(app.session.active().unwrap().revision > revision);
}

#[test]
fn ui_set_rejects_unknown_fields_before_changing_anything() {
    use crate::theme::ThemeKind;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.set_theme(&ctx, ThemeKind::StudioLight);
    // `dark` was advertised to MCP clients but never read; a typo looked like success too.
    for params in [json!({"dark": true}), json!({"thme": "classic"}), json!({"tool": "move", "dark": true})] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert!(r["error"].as_str().unwrap().contains("unknown field"), "{r}");
    }
    assert_eq!(app.ui.theme, ThemeKind::StudioLight);
    assert_eq!(app.ui.tool, Tool::Brush, "a rejected call applies none of its fields");
    // Every field the method reads gets past the check (a bad value is its own error).
    for field in UI_SET_FIELDS {
        let r = call(&mut app, &ctx, "ui.set", json!({ field: null }));
        assert!(!r.to_string().contains("unknown field"), "{field}: {r}");
    }
    assert_eq!(call(&mut app, &ctx, "ui.set", Value::Null)["ok"], true);
}

#[test]
fn ui_set_gradient_blend_mode_validates_and_updates_options() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let good = call(&mut app, &ctx, "ui.set", json!({"tool": "gradient", "gradientBlendMode": "Difference", "gradientClassic": true}));
    assert_eq!(good["ok"], true, "{good}");
    assert_eq!(app.ui.tool_options.gradient_blend_mode, photocraft_color::BlendMode::Difference);
    assert!(app.ui.tool_options.gradient_classic);
    let bad = call(&mut app, &ctx, "ui.set", json!({"gradientBlendMode": "nonsense", "gradientClassic": false}));
    assert_eq!(bad["ok"], false, "{bad}");
    assert!(app.ui.tool_options.gradient_classic, "invalid mode must not change options");
}

/// #5048: the Layers panel filter is drivable; a bad field changes nothing.
#[test]
fn ui_set_drives_the_layers_panel_filter() {
    use crate::layer_filter_ui::FilterBy;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let ok = call(&mut app, &ctx, "ui.set", json!({"layerFilter": {"by": "name", "name": "ink"}}));
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!((app.ui.layer_filter_by, app.ui.layer_filter_name.as_str()), (FilterBy::Name, "ink"));
    let ok = call(&mut app, &ctx, "ui.set", json!({"layerFilter": {"by": "mode", "mode": "Pass Through", "kinds": ["type", "pixel", "type"]}}));
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!((app.ui.layer_filter_by, app.ui.layer_filter_mode), (FilterBy::Mode, photocraft_color::BlendMode::PassThrough));
    assert_eq!(app.ui.layer_filter, ["type", "pixel"], "kinds are kept once each");
    assert_eq!(app.ui.layer_filter_name, "ink", "fields left out keep their value");
    for bad in [
        json!({"layerFilter": "name"}),
        json!({"layerFilter": {"by": "colour"}}),
        json!({"layerFilter": {"kinds": ["pixels"]}}),
        json!({"layerFilter": {"kinds": "pixel"}}),
        json!({"layerFilter": {"name": 3}}),
        json!({"layerFilter": {"mode": "Nonsense"}}),
        json!({"layerFilter": {"by": "kind", "extra": true}}),
        json!({"layerFilter": {"by": "kind"}, "tileGrid": "yes"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad} → {r}");
        assert_eq!(app.ui.layer_filter_by, FilterBy::Mode, "{bad} changed nothing");
    }
}

#[test]
fn ui_set_toggles_the_developer_tile_grid() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert!(!app.ui.view.tile_grid, "off by default");
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tileGrid": true}))["ok"], true);
    assert!(app.ui.view.tile_grid);
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["view"]["tile_grid"], true);
    // Not a View › Show item: Extras doesn't hide it, and Show Extras Options doesn't list it.
    crate::menus::invoke(&mut app, &ctx, "view.extras", json!({})).unwrap();
    assert!(!app.ui.view.extras && app.ui.view.tile_grid);
    assert!(serde_json::to_value(app.ui.view.show).unwrap().get("tile_grid").is_none());
    let bad = call(&mut app, &ctx, "ui.set", json!({"tileGrid": "yes"}));
    assert_eq!(bad["ok"], false, "{bad}");
    assert!(app.ui.view.tile_grid, "a rejected value changes nothing");
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tileGrid": false}))["ok"], true);
    assert!(!app.ui.view.tile_grid);
}

#[test]
fn ui_set_drives_the_eyedropper_options() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let good = call(
        &mut app,
        &ctx,
        "ui.set",
        json!({"tool": "eyedropper", "eyedropperSampleSize": 11, "eyedropperSample": "currentAndBelow", "eyedropperRing": false}),
    );
    assert_eq!(good["ok"], true, "{good}");
    let o = &app.ui.tool_options;
    assert_eq!((o.eyedropper_size, o.eyedropper_sample.as_str(), o.eyedropper_ring), (11, "currentAndBelow", false));
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": "point"}))["ok"], true);
    assert_eq!(app.ui.tool_options.eyedropper_size, 1);
    for bad in [
        json!({"eyedropperSampleSize": 4}),
        json!({"eyedropperSampleSize": "big"}),
        json!({"eyedropperSample": "below"}),
        json!({"eyedropperSample": 2}),
        json!({"eyedropperRing": "yes"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    // A rejected call applies none of its fields.
    let r = call(&mut app, &ctx, "ui.set", json!({"eyedropperSampleSize": 101, "eyedropperSample": "nope"}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(app.ui.tool_options.eyedropper_size, 1);
}

/// #1046: the Color Sampler tool is selectable over the control channel and reported as active.
#[test]
fn ui_set_selects_the_color_sampler_tool() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "colorSampler"}))["ok"], true);
    assert_eq!(app.ui.tool, Tool::ColorSampler);
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["tool"], "ColorSampler");
    // The `I` key group cycles through the eyedropper tools, so the sampler is on the same key.
    assert_eq!(Tool::ColorSampler.key(), 'I');
}

/// #3823: the Crop tool's Fill over the control channel, reported by `ui.inspect`.
#[test]
fn ui_set_drives_the_crop_fill() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "crop", "cropFill": "contentAware"}))["ok"], true);
    assert!(app.ui.tool_options.crop_content_aware);
    assert_eq!(call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"]["crop_content_aware"], true);
    for bad in [json!("generativeExpand"), json!(true), json!(1)] {
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropFill": bad}))["ok"], false, "{bad}");
        assert!(app.ui.tool_options.crop_content_aware, "{bad}: unchanged");
    }
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropFill": "background"}))["ok"], true);
    assert!(!app.ui.tool_options.crop_content_aware);
}

/// #1397: the Pen's mode and Path Operation over the control channel, reported by `ui.inspect`.
#[test]
fn ui_set_drives_the_pen_path_operation() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "pen", "penMode": "shape", "penPathOp": "subtract"}))["ok"], true);
    let o = &call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"];
    assert_eq!((o["vector_mode"].clone(), o["pen_path_op"].clone()), (json!("shape"), json!("subtract")));
    for bad in [json!({"penPathOp": "melt"}), json!({"penPathOp": 1}), json!({"penMode": "pixels"}), json!({"penMode": "path", "penPathOp": "xor"})] {
        assert_eq!(call(&mut app, &ctx, "ui.set", bad.clone())["ok"], false, "{bad}");
        assert_eq!((app.ui.tool_options.vector_mode.as_str(), app.ui.tool_options.pen_path_op.as_str()), ("shape", "subtract"), "{bad}: unchanged");
    }
}

/// #1919: the Crop tool's overlay menu over the control channel, reported by `ui.inspect`.
#[test]
fn ui_set_drives_the_crop_overlay_options() {
    use crate::crop_overlay::{CropOverlay, OverlayShow};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let good = call(&mut app, &ctx, "ui.set", json!({"tool": "crop", "cropOverlay": "goldenSpiral", "cropOverlayShow": "always", "cropOverlayOrientation": 2}));
    assert_eq!(good["ok"], true, "{good}");
    let o = &app.ui.tool_options;
    assert_eq!((o.crop_overlay, o.crop_overlay_show, o.crop_overlay_orientation), (CropOverlay::GoldenSpiral, OverlayShow::Always, 2));
    let seen = call(&mut app, &ctx, "ui.inspect", json!({}));
    let t = &seen["result"]["toolOptions"];
    assert_eq!(
        (t["crop_overlay"].as_str(), t["crop_overlay_show"].as_str(), t["crop_overlay_orientation"].as_u64()),
        (Some("goldenSpiral"), Some("always"), Some(2))
    );
    for bad in [
        json!({"cropOverlay": "spiral"}),
        json!({"cropOverlay": 1}),
        json!({"cropOverlayShow": "sometimes"}),
        json!({"cropOverlayOrientation": 4}),
        json!({"cropOverlayOrientation": -1}),
        json!({"cropOverlayOrientation": "1"}),
        json!({"cropOverlay": "grid", "cropOverlayShow": "nope"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(app.ui.tool_options.crop_overlay, CropOverlay::GoldenSpiral, "a rejected call applies none of its fields");
}

/// #1919: the Crop tool's gear menu (Show Cropped Area, crop shield) over the control channel.
#[test]
fn ui_set_drives_the_crop_shield_options() {
    use crate::crop_shield::{CropShield, ShieldColor};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let good = call(&mut app, &ctx, "ui.set", json!({"cropShield": {"color": "custom", "custom_color": [255, 0, 0], "opacity": 40}}));
    assert_eq!(good["ok"], true, "{good}");
    let want = CropShield { color: ShieldColor::Custom, custom_color: [255, 0, 0], opacity: 40.0, ..Default::default() };
    assert_eq!(app.ui.tool_options.crop_shield, want);
    // A patch: the other fields stay.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"cropShield": {"show_cropped_area": false}}))["ok"], true);
    assert_eq!(app.ui.tool_options.crop_shield, CropShield { show_cropped_area: false, ..want.clone() });
    let t = &call(&mut app, &ctx, "ui.inspect", json!({}))["result"]["toolOptions"]["crop_shield"];
    assert_eq!((t["color"].as_str(), t["opacity"].as_f64(), t["show_cropped_area"].as_bool()), (Some("custom"), Some(40.0), Some(false)));
    for bad in [
        json!({"cropShield": true}),
        json!({"cropShield": {"opacity": 101}}),
        json!({"cropShield": {"opacity": -1}}),
        json!({"cropShield": {"opacity": "50"}}),
        json!({"cropShield": {"color": "red"}}),
        json!({"cropShield": {"custom_color": [256, 0, 0]}}),
        json!({"cropShield": {"shield": true}}),
        json!({"cropShield": {"enabled": false}, "cropOverlay": "nope"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
    assert_eq!(app.ui.tool_options.crop_shield, CropShield { show_cropped_area: false, ..want }, "a rejected call applies none of its fields");
}

#[test]
fn ui_set_opens_the_brush_preset_picker_and_sets_its_cards() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let list = &app.ui.brush_picker_list;
    assert!(list.show_name && list.show_stroke && list.show_tip, "every card part is on by default");
    let r = call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushPicker": [120, 80], "brushPickerStroke": false}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]));
    assert!(!app.ui.brush_picker_list.show_stroke);
    for bad in [
        json!({"brushPicker": [1]}),
        json!({"brushPicker": "here"}),
        json!({"brushPickerStroke": "nope"}),
        json!({"brushPickerName": false, "brushPickerStroke": false, "brushPickerTip": false}),
    ] {
        assert_eq!(call(&mut app, &ctx, "ui.set", bad.clone())["ok"], false, "{bad}");
    }
    assert_eq!(app.ui.brush_picker, Some([120.0, 80.0]), "a bad value leaves the picker alone");
    assert!(app.ui.brush_picker_list.show_name, "a rejected call leaves the card parts alone");
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
    assert_eq!(app.ui.brush_picker, None);
}

#[test]
fn ui_inspect_tracks_brush_picker_open_and_closed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 128, "height": 128})).unwrap();

    let closed = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(closed["result"]["brushPicker"].is_null());

    // A right-click on the canvas with Brush opens the same picker as the options bar.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}))["ok"], true);
    let click = call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({
            "button": "right",
            "events": [{"kind": "down", "x": 64, "y": 64}, {"kind": "up", "x": 64, "y": 64}]
        }),
    );
    assert_eq!(click["ok"], true, "{click}");
    let open = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(open["result"]["brushPicker"]["pos"].is_array(), "{open}");
    assert_eq!(open["result"]["brushPicker"]["list"]["showName"], true);
    assert_eq!(open["result"]["brushPicker"]["list"]["showStroke"], true);
    assert_eq!(open["result"]["brushPicker"]["list"]["showTip"], true);

    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPickerTip": false}))["ok"], true);
    let changed = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(changed["result"]["brushPicker"]["list"]["showTip"], false);

    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushPicker": null}))["ok"], true);
    let closed_again = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert!(closed_again["result"]["brushPicker"].is_null());
}

#[test]
fn ui_set_applies_all_fields_or_none() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let before = app.ui.tool;
    let panels_before = serde_json::to_value(&app.ui.panels).unwrap_or_default();
    // A late invalid field must not leave the earlier valid one applied - including the
    // journaled brush command, which is validated up front for exactly this reason.
    for params in [json!({"tool": "move", "theme": "nope"}), json!({"tool": "move", "brushSize": 1e300}), json!({"tool": "move", "colorPanel": {"nope": true}})]
    {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(app.ui.tool, before, "{params}: a rejected call applies none of its fields");
    }
    // Wrong types and unknown nested keys are errors, not silent no-ops with ok:true.
    for params in [
        json!({"panels": "x"}),
        json!({"panels": {"nope": true}}),
        json!({"dockTabs": {"nope": 1}}),
        json!({"dock": {"nope": 1}}),
        json!({"center": [1]}),
        json!({"center": [1, 2, 3]}),
        json!({"center": "here"}),
        json!({"colorPanel": "x"}),
        json!({"selectionMode": "add"}),
    ] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
    }
    assert_eq!(serde_json::to_value(&app.ui.panels).unwrap_or_default(), panels_before, "nothing was applied");
    // Non-numeric brushSize keeps the API's historical silent no-op (see the dispatch test).
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"brushSize": "large"}))["ok"], true);
}

#[test]
fn ui_set_rejects_view_numbers_beyond_f32_range() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let r = call(&mut app, &ctx, "ui.set", json!({"rotation": 30, "zoom": 2, "center": [3, 4], "dockWidth": 300}));
    assert_eq!(r["ok"], true, "{r}");
    let dock = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<f32>(crate::panels::dock_width_id()));
    let (rotation, zoom, center) = (app.ui.views[0].rotation, app.ui.views[0].zoom, app.ui.views[0].center);
    // Finite in f64 but infinite as f32: an error, not a silent reset to 0°, 100% or the minimum dock width.
    for params in [json!({"rotation": 1e39}), json!({"zoom": 1e39}), json!({"center": [1e39, 0]}), json!({"center": [0, -1e39]}), json!({"dockWidth": 1e39})] {
        let r = call(&mut app, &ctx, "ui.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(app.ui.views[0].rotation, rotation, "{params}");
        assert_eq!(app.ui.views[0].zoom, zoom, "{params}");
        assert_eq!(app.ui.views[0].center, center, "{params}");
        assert_eq!(dock(&ctx), Some(300.0), "{params}");
    }
    // Huge but representable still clamps to the top zoom level.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"zoom": 1e38}))["ok"], true);
    assert!(app.ui.views[0].zoom.is_finite() && app.ui.views[0].zoom > zoom, "{}", app.ui.views[0].zoom);
}

#[test]
fn ui_set_theme_keeps_explicit_dock_tab_indices() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    for theme in ["adwaitaDark", "pro"] {
        let r = call(&mut app, &ctx, "ui.set", json!({"theme": theme, "dockTabs": {"color": 0}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.ui.dock_tabs.color, 0, "{theme}: explicit tab index changed");
    }
}

#[test]
fn ui_set_color_panel_picks_the_edited_chip() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": true}}))["ok"], true);
    assert!(app.ui.color_panel.background);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": "yes"}}))["ok"], false);
    assert!(app.ui.color_panel.background, "a bad value changes nothing");
}

#[test]
fn ui_set_color_panel_mode() {
    use crate::state::ColorPanelMode;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HueCube);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"mode": "rgbSliders"}}))["ok"], true);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::RgbSliders);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"mode": "hsbSliders"}}))["ok"], true);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HsbSliders);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"mode": "nope"}}))["ok"], false);
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HsbSliders);
}

#[test]
fn ui_set_unknown_theme_error_names_every_theme_and_each_name_works() {
    use crate::theme::ThemeKind;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let r = call(&mut app, &ctx, "ui.set", json!({"theme": "nope"}));
    assert_eq!(r["ok"], false);
    let error = r["error"].as_str().unwrap();
    for kind in ThemeKind::ALL {
        assert!(error.contains(kind.id()), "{error} lacks {}", kind.id());
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": kind.id()}))["ok"], true);
        assert_eq!(app.ui.theme, kind);
        assert_eq!(ThemeKind::from_name(kind.id()), Some(kind));
    }
}

#[test]
fn ui_window_open_rejects_an_unknown_document_index() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    // An out-of-range index must error instead of silently switching the active document
    // and opening the window for whatever is active now.
    let before = app.session.active_index();
    let r = call(&mut app, &ctx, "ui.window.open", json!({"document": 99}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("no document 99"), "{r}");
    assert_eq!(app.session.active_index(), before, "the active document is untouched");
    // A valid index still opens the window.
    assert_eq!(call(&mut app, &ctx, "ui.window.open", json!({"document": 0}))["ok"], true);
}

#[test]
fn ui_set_brush_size_dispatches_a_journaled_brush_command() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();

    let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": 42.5}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.session.tools.brush.size, 42.5);
    let (id, params) = app.session.journal.last().cloned().expect("brush change is journaled");
    assert_eq!(id, "tools.setBrush");
    assert_eq!(params, json!({"brush": {"size": 42.5}}));

    // The control API has historically ignored non-numeric optional values.
    for params in [json!({}), json!({"brushSize": null}), json!({"brushSize": "large"}), json!({"brushSize": true})] {
        let journal_len = app.session.journal.len();
        let r = call(&mut app, &ctx, "ui.set", params);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        assert_eq!(app.session.journal.len(), journal_len);
    }

    // Each tool keeps its own brush (#218), so `tool` + `brushSize` in one call sets the new
    // tool's size rather than the one it was carrying.
    call(&mut app, &ctx, "ui.set", json!({"tool": "eraser", "brushSize": 12.0}));
    assert_eq!(app.session.tools.brush.size, 12.0, "the Eraser's own size");
    call(&mut app, &ctx, "ui.set", json!({"tool": "brush"}));
    assert_eq!(app.session.tools.brush.size, 42.5, "the Brush gets its own back");
    call(&mut app, &ctx, "ui.set", json!({"tool": "eraser"}));
    assert_eq!(app.session.tools.brush.size, 12.0);
    call(&mut app, &ctx, "ui.set", json!({"tool": "brush", "brushSize": 42.5}));

    // Values that cannot be represented by BrushSettings must report the command error and
    // leave both the brush and journal unchanged instead of mutating tool state directly.
    let journal_len = app.session.journal.len();
    let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": f64::MAX}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().is_some(), "{r}");
    assert_eq!(app.session.tools.brush.size, 42.5);
    assert_eq!(app.session.journal.len(), journal_len);
}

#[test]
fn preferences_apply_is_available_over_control_without_closing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.preferences.interface"}));
    let id = r["result"]["dialog"].as_u64().unwrap();
    let mut values = app.ui.dialog_mut(id).unwrap().fields["values"].clone();
    values["interface"]["uiScale"] = json!("200");
    assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
    assert!(app.ui.dialog_mut(id).is_some());
    assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
    assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
    for params in [json!({}), json!({"dialog": "invalid"}), json!({"dialog": u64::MAX})] {
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", params)["ok"], false);
    }
}

#[test]
fn preferences_apply_respects_the_automation_command_policy() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| if id == "prefs.set" { Err("preference changes denied".into()) } else { Ok(()) })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    let before = app.session.prefs().to_json();
    let id = crate::prefs_ui::open_preferences(&mut app, "interface");
    app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["uiScale"] = json!("200");
    let r = call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}));
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"], "preference changes denied");
    assert_eq!(app.session.prefs().to_json(), before);
    assert!(app.ui.dialog_mut(id).is_some());
}

#[test]
fn synthetic_shortcuts_use_the_automation_command_policy() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| if id.starts_with("file.") { Err("filesystem command denied".into()) } else { Ok(()) })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.automation_input = true;
    let error = crate::menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err();
    assert_eq!(error, "filesystem command denied");
}

/// `app.save` takes the Targa bits per pixel (`tgaBits`) and never stops at Targa Options.
#[test]
fn automation_save_passes_targa_bits_without_asking() {
    use std::cell::RefCell;
    use std::rc::Rc;
    // The bits each export was asked for, and each path written.
    let asked: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w) = (asked.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push(s.tga_bits.map(photocraft_io::TgaBits::bits));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in [json!({"path": "a.tga"}), json!({"path": "b.tga", "tgaBits": 24}), json!({"path": "c.tga", "tgaBits": 32})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.targa_options.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [None, Some(24), Some(32)]);
    assert_eq!(*written.borrow(), ["a.tga", "b.tga", "c.tga"]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "d.tga", "tgaBits": 16}));
    assert!(r["error"].as_str().unwrap().contains("tgaBits"), "{r}");
    assert_eq!(written.borrow().len(), 3, "a bad tgaBits writes nothing");
}

/// `app.save` takes JPEG Options (`quality` 0–12, `jpegFormat`) and never stops at the prompt;
/// a value it can't read is an error that writes nothing.
#[test]
fn automation_save_passes_jpeg_options_without_asking() {
    use photocraft_codecs::JpegFormat;
    use std::cell::RefCell;
    use std::rc::Rc;
    // The JPEG quality (1–100) and format each export was asked for.
    type Asked = Rc<RefCell<Vec<(Option<u8>, JpegFormat)>>>;
    let asked: Asked = Rc::default();
    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let (a, w) = (asked.clone(), written.clone());
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push((s.jpeg_quality, s.jpeg_format));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in
        [json!({"path": "a.jpg"}), json!({"path": "b.jpg", "quality": 12, "jpegFormat": "progressive"}), json!({"path": "c.jpg", "jpegFormat": "optimized"})]
    {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.jpeg_options.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [(None, JpegFormat::Baseline), (Some(100), JpegFormat::Progressive), (None, JpegFormat::Optimized)]);
    for bad in [
        json!({"path": "d.jpg", "quality": 13}),
        json!({"path": "d.jpg", "quality": "high"}),
        json!({"path": "d.jpg", "jpegFormat": "fast"}),
        json!({"path": "d.jpg", "jpegFormat": 2}),
    ] {
        let r = call(&mut app, &ctx, "app.save", bad.clone());
        assert!(r["error"].is_string(), "{bad}: {r}");
    }
    assert_eq!(written.borrow().len(), 3, "a bad JPEG option writes nothing");
}

/// `app.save` takes the DDS options (`ddsCompression`, `ddsMipmaps`) and never stops at DDS
/// Options.
#[test]
fn automation_save_passes_dds_options_without_asking() {
    use photocraft_io::{DdsCompression, DdsOptions};
    use std::cell::RefCell;
    use std::rc::Rc;
    let asked: Rc<RefCell<Vec<DdsOptions>>> = Rc::default();
    let a = asked.clone();
    let services = crate::Services {
        export: Some(Box::new(move |_d: &photocraft_doc::Document, _p: &str, s: &crate::ExportSettings| {
            a.borrow_mut().push(s.dds);
            Ok((b"out".to_vec(), Vec::new()))
        })),
        automation_write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for params in [json!({"path": "a.dds"}), json!({"path": "b.dds", "ddsCompression": "bc1", "ddsMipmaps": true})] {
        let r = call(&mut app, &ctx, "app.save", params.clone());
        assert_eq!(r["result"]["path"], params["path"], "{r}");
        assert!(app.dds_options.is_none(), "automation saves never prompt");
    }
    assert_eq!(*asked.borrow(), [DdsOptions::default(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: true }]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "c.dds", "ddsCompression": "bc9"}));
    assert!(r["error"].as_str().unwrap().contains("ddsCompression"), "{r}");
    assert_eq!(asked.borrow().len(), 2, "a bad ddsCompression writes nothing");
}

#[test]
fn automation_open_and_save_use_the_roots_and_reply_with_warnings() {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::Size;
    use std::cell::RefCell;
    use std::rc::Rc;

    // Without granted roots both fail closed, even with an exporter and an ambient writer.
    let services = crate::Services {
        export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
    assert!(r.to_string().contains("automation read authority is not configured"), "{r}");
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
    assert!(r.to_string().contains("automation write authority is not configured"), "{r}");

    let written: Rc<RefCell<Vec<String>>> = Rc::default();
    let w = written.clone();
    let services = crate::Services {
        import: Some(Box::new(|name: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), vec!["Adjustment layer flattened".to_string()]))
        })),
        export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), vec!["Layers were flattened".to_string()])))),
        automation_read: Some(Box::new(|path: &str| Ok((path.rsplit('/').next().unwrap_or(path).to_string(), b"x".to_vec())))),
        automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
    assert_eq!(r["result"]["warnings"], json!(["Adjustment layer flattened"]), "{r}");
    assert_eq!(r["result"]["name"], "warn.psd");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("in/warn.psd"));
    assert_eq!(app.ui.recent_files, vec!["in/warn.psd".to_string()]);
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
    assert_eq!(r["result"], json!({"path": "out.png", "warnings": ["Layers were flattened"]}), "{r}");
    assert_eq!(*written.borrow(), vec!["out.png".to_string()]);
    // A flat export is a copy: the document keeps its name and file (#2579).
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("warn.psd", Some("in/warn.psd")));
    // Without `path`, only a layered file is written back, like File › Save (#416).
    call(&mut app, &ctx, "app.open", json!({"path": "in/flat.jpg"}));
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
    assert_eq!(written.borrow().len(), 1, "nothing written over the JPEG");
    call(&mut app, &ctx, "app.open", json!({"path": "in/layered.psd"}));
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert_eq!(r["result"]["path"], "in/layered.psd", "{r}");
    assert_eq!(written.borrow().last().map(String::as_str), Some("in/layered.psd"));
    // A template opens untitled, so a save without `path` never writes over it.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "in/card.psdt"}));
    assert_eq!(r["result"]["name"], "Untitled-1", "{r}");
    assert_eq!(app.session.active().unwrap().path, None);
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
}

fn deny_ambient_file(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
    if id.starts_with("file.") && id != "file.new" {
        Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
    } else {
        Ok(())
    }
}

/// An app whose automation gates refuse file commands, with a document open, and the paths
/// its writer was asked to write.
fn gated_app_recording_writes() -> (PhotocraftApp, egui::Context, std::rc::Rc<std::cell::RefCell<Vec<String>>>, u64) {
    let written: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::default();
    let w = written.clone();
    let services = crate::Services {
        automation_authorize: Some(deny_ambient_file),
        automation_command: Some(Box::new(|id, params| deny_ambient_file(id, params).map_err(|e| e.to_string()))),
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let doc = app.session.active().unwrap().doc.id.0;
    (app, egui::Context::default(), written, doc)
}

fn outside_target() -> String {
    std::env::temp_dir().join("outside").join("x.png").to_string_lossy().into_owned()
}

/// `ui.dialog.set` and `ui.dialog.open` never add a marker that picks a dialog's OK handler:
/// a View › Zoom In dialog can't be made to confirm as the disk-change prompt's save.
#[test]
fn automation_cannot_add_a_marker_that_reroutes_a_dialog() {
    let (mut app, ctx, written, doc) = gated_app_recording_writes();
    let target = outside_target();
    let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "command", "command": "view.zoomIn"}));
    let dialog = r["result"]["dialog"].clone();
    for (field, value) in [("document", json!(doc)), ("path", json!(target)), ("flat", json!(false))] {
        let r = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": field, "value": value}));
        assert!(r.get("error").is_none(), "{r}");
    }
    let injected = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": "__diskChanged", "value": true}));
    let replace = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": dialog, "field": "__replaceSaved", "value": true}));
    assert!(replace["error"].as_str().is_some_and(|e| e.contains("set only by the dialog")), "{replace}");
    call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": dialog}));
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(injected["error"].as_str().is_some_and(|e| e.contains("set only by the dialog")), "{injected}");
    let r = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "about", "fields": {"__export": true}}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("set only by the dialog")), "{r}");
}

/// A handler that writes files itself (the disk-change prompt's save) is judged as the file
/// command it stands for, through a control request or synthetic input alike.
#[test]
fn automation_confirming_a_file_writing_dialog_is_judged_as_its_file_command() {
    let (mut app, ctx, written, doc) = gated_app_recording_writes();
    let target = outside_target();
    // A real disk-change prompt keeps its own fields settable, but its save is a file command.
    let fields = json!({"__diskChanged": true, "__label": "Save", "message": "", "document": doc, "path": target, "flat": false});
    let prompt = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap());
    let r = call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": prompt, "field": "__diskChanged", "value": true}));
    assert!(r.get("error").is_none(), "{r}");
    let r = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": prompt}));
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(r["error"].as_str().is_some_and(|e| e.contains("file.save")), "{r}");
    // Synthetic input (Enter or OK) reaches the same handler without the control pre-check.
    let synthetic = app.ui.open_dialog(DialogKind::Command, fields.as_object().cloned().unwrap());
    app.automation_input = true;
    let r = crate::dialogs::confirm(&mut app, synthetic);
    app.automation_input = false;
    assert!(written.borrow().is_empty(), "nothing is written: {:?}", written.borrow());
    assert!(r.is_err_and(|e| e.contains("file.save")));
}

#[test]
fn app_open_opens_a_directory_pcraft_and_falls_back_for_the_zip_form() {
    use photocraft_doc::Document;
    use serde_json::json;
    let services = crate::Services {
        automation_open_pcraft: Some(Box::new(|path: &str| {
            if path == "native.pcraft" {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                let doc = Document::new("Native", photocraft_doc::Size::new(16, 12), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
                Ok(Some((name, doc)))
            } else if path == "zipped.pcraft" {
                Ok(None) // a regular file: the bytes path reads the zip form
            } else {
                Err(format!("corrupt bundle: {path}"))
            }
        })),
        import: Some(Box::new(|name: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), Vec::new()))
        })),
        automation_read: Some(Box::new(|path: &str| {
            if path == "zipped.pcraft" {
                Ok((path.rsplit('/').next().unwrap_or(path).to_string(), b"zip-bytes".to_vec()))
            } else {
                Err(format!("no such path: {path}"))
            }
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();

    let r = call(&mut app, &ctx, "app.open", json!({"path": "native.pcraft"}));
    assert_eq!(r["result"], json!({"path": "native.pcraft", "name": "native.pcraft", "warnings": []}), "{r}");
    assert_eq!(app.session.active().unwrap().doc.name, "Native");
    assert_eq!(app.ui.recent_files, vec!["native.pcraft".to_string()], "the open is recorded in recent files");
    assert_eq!(app.session.active().unwrap().path, None, "no save-back target: the automation writer replaces files, never directories");
    let r = call(&mut app, &ctx, "app.save", json!({}));
    assert!(r["error"].as_str().unwrap().contains("pass `path`"), "an in-place save is refused up front: {r}");

    // The single-file form takes the bytes path through the same `.pcraft` extension.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "zipped.pcraft"}));
    assert_eq!(r["result"]["name"], "zipped.pcraft", "{r}");
    assert_eq!(app.session.active().unwrap().doc.name, "zipped.pcraft", "the import served the zip form");

    // A real opener error propagates instead of being masked by the fallback.
    let r = call(&mut app, &ctx, "app.open", json!({"path": "gone.pcraft"}));
    assert!(r["error"].as_str().unwrap().contains("corrupt bundle"), "{r}");
}

#[test]
fn app_save_runs_as_a_background_job_and_the_final_reply_carries_the_path() {
    use photocraft_engine::automate_cmds::ScriptBinding;
    use serde_json::json;
    let saved: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let record = saved.clone();
    let services = crate::Services {
        automation_save: Some(std::sync::Arc::new(move |_doc, path: &str, _settings, _ctl| {
            record.lock().unwrap().push(path.to_string());
            Ok(vec!["Layers were flattened".to_string()])
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = true;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    // A "Save Document" binding that would leave a trace: automation saves never fire it.
    app.session.edit_prefs(|prefs| {
        prefs.script_events.enabled = true;
        prefs.script_events.bindings = vec![ScriptBinding {
            event: "saveDocument".into(),
            script: None,
            steps: Some(json!([["layer.new.layer", {"name": "From the binding"}]])),
            name: "Binding".into(),
        }];
    });
    let ctx = egui::Context::default();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.with_control(rx);
    let mut app = app;

    let (mut req, reply) = ControlRequest::new("app.save", json!({"path": "out.psd"}));
    req.deadline = None;
    tx.send(req).unwrap();
    app.drain_control(&ctx);
    // Started in the background: no reply yet, the job is on the document.
    assert!(reply.try_recv().is_err(), "the reply waits for the save job");
    assert!(app.saving(), "the save runs as a background job");
    for _ in 0..100 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.saving(), "the save finished");
    assert_eq!(*saved.lock().unwrap(), vec!["out.psd".to_string()]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out.psd"), "the save was recorded");
    assert_eq!(app.session.active().unwrap().doc.layer_count(), 1, "the saveDocument binding did not run for an automation save");
    let r = reply.recv().unwrap();
    assert_eq!(r["result"], json!({"path": "out.psd", "warnings": ["Layers were flattened"]}), "{r}");

    // An interactive save (File › Save's worker) still fires the binding.
    let save = std::sync::Arc::new(|_doc: &photocraft_doc::Document, path: &str, _settings: &crate::ExportSettings, _ctl: &crate::jobs_ui::SaveCtl| {
        let _ = path;
        Ok(Vec::new())
    }) as crate::SaveFileFn;
    crate::jobs_ui::start_save(&mut app, "user.psd".into(), crate::ExportSettings::default(), false, save, false).unwrap();
    for _ in 0..100 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.saving(), "the interactive save finished");
    assert_eq!(app.session.active().unwrap().doc.layer_count(), 2, "the binding runs for an interactive save");
}

#[test]
fn ui_inspect_replies_while_a_save_job_encodes() {
    // #2488: the control channel must keep answering while a large save runs.
    use serde_json::json;
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wait_for_gate = gate.clone();
    let saved: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let record = saved.clone();
    let services = crate::Services {
        automation_save: Some(std::sync::Arc::new(move |_doc, path: &str, _settings, ctl| {
            while !wait_for_gate.load(std::sync::atomic::Ordering::Acquire) {
                ctl.check()?;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            record.lock().unwrap().push(path.to_string());
            Ok(Vec::new())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = true;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let ctx = egui::Context::default();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.with_control(rx);
    let mut app = app;

    let (mut save_req, save_reply) = ControlRequest::new("app.save", json!({"path": "out.png", "wait": false}));
    save_req.deadline = None;
    tx.send(save_req).unwrap();
    let (mut inspect_req, inspect_reply) = ControlRequest::new("ui.inspect", json!({}));
    inspect_req.deadline = None;
    tx.send(inspect_req).unwrap();
    app.drain_control(&ctx);

    let pending = save_reply.recv_timeout(std::time::Duration::from_secs(5)).expect("the unwaited save replies at once");
    assert_eq!(pending["result"]["pending"], true, "{pending}");
    assert_eq!(pending["result"]["path"], "out.png", "{pending}");
    assert!(pending["result"]["job"].is_u64(), "the caller can poll or wait: {pending}");
    // While the save is still encoding, the channel answers other requests.
    assert!(saved.lock().unwrap().is_empty(), "the save is still encoding");
    let inspected = inspect_reply.recv_timeout(std::time::Duration::from_secs(5)).expect("ui.inspect is not blocked by the save");
    assert_eq!(inspected["ok"], true, "{inspected}");

    gate.store(true, std::sync::atomic::Ordering::Release);
    for _ in 0..500 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if !app.saving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(*saved.lock().unwrap(), vec!["out.png".to_string()], "the save completed after the gate opened");
}

fn deny_smart_object_paths(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    // The production policy lives in the automation crate. This gate exercises the control
    // session's state-derived path check without adding that dependency to the UI crate.
    if matches!(id, "layer.smartObjects.editContents" | "layer.smartObjects.convertToLayers" | "layer.smartObjects.saveContents")
        && params.get("path").and_then(Value::as_str).is_some()
    {
        Err(photocraft_engine::EngineError::Other("ambient smart-object path denied".into()))
    } else {
        Ok(())
    }
}

fn smart_control_app() -> (PhotocraftApp, egui::Context, photocraft_doc::LayerId) {
    let services = crate::Services {
        automation_authorize: Some(deny_smart_object_paths),
        automation_command: Some(Box::new(|id, params| deny_smart_object_paths(id, params).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6, "background": "transparent"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    (app, egui::Context::default(), photocraft_doc::LayerId(id))
}

#[test]
fn control_edits_nested_embedded_smart_objects_and_converts_an_explicit_target() {
    use photocraft_doc::{LayerContent, SmartSource};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    let (mut app, ctx, smart) = smart_control_app();
    // Embed this one-smart-layer document inside the outer object: two Edit Contents
    // requests must reach the raster layer, as in a nested product-photo template.
    let nested = photocraft_engine::smart_cmds::encode_source(&app.session.active().unwrap().doc).unwrap();
    app.session
        .edit("Nested fixture", |doc, _| {
            let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
            sm.source = SmartSource::Embedded { file_name: "product.pcraft".into(), bytes: Arc::new(nested) };
            Ok(())
        })
        .unwrap();
    let other = app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(app.session.active().unwrap().active_layer, Some(smart));
    for expected_document in [1, 2] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.editContents", "params": {"layer": smart.0}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["document"], expected_document, "{r}");
        assert!(app.session.active().unwrap().path.is_none(), "contents open in memory");
        assert!(app.session.authorize.is_none(), "the request gate must not remain installed");
    }
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "edit.fill", "params": {"color": "#0000ff"}}));
    assert_eq!(r["ok"], true, "{r}");
    for expected_documents in [2, 1] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.saveContents"}));
        assert_eq!(r["ok"], true, "{r}");
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "file.close"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.documents().len(), expected_documents);
    }
    assert_eq!(app.session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(other)));
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [0.0, 0.0, 1.0, 1.0]);
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "layer.smartObjects.convertToLayers", "params": {"layer": smart.0}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(other)).is_some(), "the unrelated active layer remains");
    assert!(app.session.authorize.is_none());

    let written = Rc::new(RefCell::new(Vec::new()));
    let output = written.clone();
    app.services.export =
        Some(Box::new(|doc, _, _| photocraft_engine::smart_cmds::encode_source(doc).map(|bytes| (bytes, Vec::new())).map_err(|e| e.to_string())));
    app.services.automation_write = Some(Box::new(move |path, bytes| {
        output.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    app.services.write = Some(Box::new(|_, _| Err("ambient writer used".into())));
    let r = call(&mut app, &ctx, "app.save", json!({"path": "out/template.pcraft"}));
    assert_eq!(r["ok"], true, "{r}");
    let written = written.borrow();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, "out/template.pcraft");
    let saved = photocraft_engine::smart_cmds::decode_source(&written[0].0, &written[0].1).unwrap();
    assert_eq!(photocraft_compose::flatten(&saved).px[0], [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn control_refuses_linked_smart_objects_in_commands_and_actions_but_local_editing_still_works() {
    use photocraft_doc::{LayerContent, SmartSource};
    use std::sync::Arc;

    let (mut app, ctx, smart) = smart_control_app();
    let path = std::env::temp_dir().join(format!("pc-control-smart-linked-{}.pcraft", std::process::id()));
    let st = app.session.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(smart).unwrap().content else { panic!("smart fixture") };
    let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("embedded fixture") };
    std::fs::write(&path, bytes.as_slice()).unwrap();
    app.session
        .edit("Linked fixture", |doc, _| {
            let LayerContent::Smart(sm) = &mut doc.layer_mut(smart).unwrap().content else { panic!("smart fixture") };
            sm.source = SmartSource::Linked { path: path.to_string_lossy().into_owned() };
            Ok(())
        })
        .unwrap();
    app.run("layer.new.layer", json!({"name": "Unrelated"})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active().unwrap().active_layer;
    for command in ["layer.smartObjects.editContents", "layer.smartObjects.convertToLayers"] {
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": command, "params": {"layer": smart.0}}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("ambient smart-object path denied"), "{r}");
        app.session.actions.list = vec![photocraft_engine::actions_cmds::Action {
            name: "Linked contents".into(),
            steps: vec![(command.into(), json!({"layer": smart.0}))],
            disabled: Default::default(),
            ..Default::default()
        }];
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Linked contents"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["ran"], 0, "{r}");
        assert_eq!(r["result"]["failed"]["id"], command, "{r}");
        assert_eq!(app.session.documents().len(), 1);
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before), "a refused operation must not edit the document");
        assert_eq!(app.session.active().unwrap().active_layer, active);
        assert!(app.session.authorize.is_none(), "a refused request must restore the local session");
    }
    // A synthetic thumbnail action uses the same temporary authorization as control calls.
    app.automation_input = true;
    let error = crate::menus::invoke(&mut app, &ctx, "layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap_err();
    assert!(error.contains("ambient smart-object path denied"), "{error}");
    assert!(app.session.authorize.is_none());
    app.automation_input = false;
    app.run("layer.smartObjects.editContents", json!({"layer": smart.0})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "a human can still open the existing linked file");
    assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px[0], [1.0, 0.0, 0.0, 1.0]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn actions_play_checks_each_step_over_control() {
    let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
        name: "Open".into(),
        steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let ctx = egui::Context::default();
    let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Open"}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["ran"], 0, "{r}");
    assert_eq!(r["result"]["failed"]["id"], "file.open", "{r}");
    assert!(r["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{r}");
    assert!(app.session.documents().is_empty());
    assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the request");
}

#[test]
fn actions_play_checks_each_step_for_synthetic_input() {
    let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
        name: "Open".into(),
        steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.automation_input = true;
    let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
    assert_eq!(r["ran"], 0, "{r}");
    assert_eq!(r["failed"]["id"], "file.open", "{r}");
    assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the command");

    // A local play (no automation input) still runs recorded steps.
    app.automation_input = false;
    app.session.actions.list[0].steps.remove(0);
    app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
    assert_eq!(r["ran"], 1, "{r}");
}

#[test]
fn huge_click_count_is_rejected() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    // #982: a huge count used to queue a press and a release per click before replying.
    for count in [json!(MAX_CLICKS + 1), json!(1_000_000_000u64), json!(u64::MAX)] {
        let r = call(&mut app, &ctx, "ui.click", json!({"x": 10, "y": 10, "count": count}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().unwrap().contains("`count` must be at most 256"), "{r}");
        assert!(app.synthetic.is_empty(), "a rejected click queues nothing");
    }
    // Single, double and the largest allowed clicks still queue a move plus a press and a release each.
    for (count, events) in [(None, 3), (Some(2), 5), (Some(MAX_CLICKS), 2 * MAX_CLICKS as usize + 1)] {
        let params = match count {
            Some(n) => json!({"x": 10, "y": 10, "count": n}),
            None => json!({"x": 10, "y": 10}),
        };
        let (req, _rx) = ControlRequest::new("ui.click", params);
        assert!(matches!(handle(&mut app, &ctx, &req), Outcome::AfterInput));
        assert_eq!(app.synthetic.len(), events);
        app.synthetic.clear();
    }
}

/// #2449: the error of a rejected synthetic input call, after checking it changed nothing.
fn rejected(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value, names: &str) -> String {
    let (tool, revision) = (app.ui.tool, app.session.active().map(|d| d.revision));
    let r = call(app, ctx, method, params.clone());
    assert_eq!(r["ok"], false, "{method} {params}: {r}");
    let error = r["error"].as_str().unwrap().to_string();
    assert!(error.contains(names), "{method} {params}: `{error}` should name `{names}`");
    assert!(app.synthetic.is_empty(), "{method} {params}: a rejected call queues nothing");
    assert_eq!(app.ui.tool, tool, "{method} {params}: a rejected call keeps the tool");
    assert_eq!(app.session.active().map(|d| d.revision), revision, "{method} {params}: a rejected call edits nothing");
    error
}

#[test]
fn ui_pointer_rejects_bad_params_before_acting() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let stroke = json!([{"kind": "down", "x": 5, "y": 5}, {"kind": "move", "x": 30, "y": 30}, {"kind": "up", "x": 30, "y": 30}]);
    // The same unknown tool is the same error as `ui.set`'s.
    let set = call(&mut app, &ctx, "ui.set", json!({"tool": "noSuchTool"}));
    let error = rejected(&mut app, &ctx, "ui.pointer", json!({"tool": "noSuchTool", "events": stroke}), "noSuchTool");
    assert_eq!(json!(error), set["error"]);
    let event = |extra: Value| {
        let mut e = json!({"kind": "down", "x": 5, "y": 5});
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        // A valid stroke first: a later bad event must not let the earlier ones run.
        json!({"tool": "move", "events": [stroke[0], stroke[1], stroke[2], e]})
    };
    let cases = [
        // Unknown fields, top-level, under `modifiers` and in an event.
        (json!({"tol": "move", "events": stroke}), "tol"),
        (json!({"modifiers": {"shfit": true}, "events": stroke}), "shfit"),
        (event(json!({"presure": 0.5})), "presure"),
        // Wrong JSON types.
        (json!({"tool": 3, "events": stroke}), "tool"),
        (json!({"events": "down"}), "events"),
        (json!({}), "events"),
        (json!({"events": [5]}), "events[0]"),
        (event(json!({"x": "abc"})), "x"),
        (event(json!({"y": true})), "y"),
        (event(json!({"kind": 1})), "kind"),
        (json!({"shift": "yes", "events": stroke}), "shift"),
        (json!({"modifiers": {"alt": 1}, "events": stroke}), "alt"),
        (json!({"modifiers": true, "events": stroke}), "modifiers"),
        (json!({"button": 2, "events": stroke}), "button"),
        (json!([1, 2]), "params"),
        // Values outside the accepted set.
        (json!({"button": "ritgh", "events": stroke}), "button"),
        (event(json!({"kind": "dwon"})), "kind"),
        // Numbers outside their documented range.
        (event(json!({"pressure": 99})), "pressure"),
        (event(json!({"pressure": -0.1})), "pressure"),
        (event(json!({"tiltX": 91})), "tiltX"),
        (event(json!({"tiltY": -90.5})), "tiltY"),
        (event(json!({"rotation": 361})), "rotation"),
        (event(json!({"rotation": -1})), "rotation"),
    ];
    for (params, names) in cases {
        rejected(&mut app, &ctx, "ui.pointer", params, names);
    }
    let error = rejected(&mut app, &ctx, "ui.pointer", json!({"button": "ritgh", "events": stroke}), "button");
    assert!(error.contains("right") && error.contains("secondary"), "the error lists the buttons: {error}");
    let error = rejected(&mut app, &ctx, "ui.pointer", event(json!({"kind": "dwon"})), "kind");
    assert!(error.contains("down") && error.contains("move") && error.contains("up"), "the error lists the kinds: {error}");

    // Valid calls still run: an omitted kind is a move, null is an omitted field, and the
    // documented range ends are accepted.
    let before = app.session.active().unwrap().revision;
    let pen = json!({"kind": "down", "x": 5, "y": 5, "pressure": 0, "tiltX": -90, "tiltY": 90, "rotation": 360});
    let valid = json!({"tool": "brush", "button": null, "modifiers": {"shift": false}, "events": [pen, {"x": 20, "y": 20, "pressure": 1}, {"kind": "up", "x": 20, "y": 20}]});
    let r = call(&mut app, &ctx, "ui.pointer", valid);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(app.session.active().unwrap().revision > before, "the stroke painted");
    for button in ["left", "middle", "right", "secondary"] {
        let r = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({"tool": "move", "button": button, "events": [{"kind": "down", "x": 5, "y": 5}, {"kind": "up", "x": 5, "y": 5}]}),
        );
        assert_eq!(r["ok"], true, "{button}: {r}");
    }
    assert!(app.ui.layer_menu.is_some(), "a right click with Move still opens the layer menu");
}

#[test]
fn ui_click_and_move_reject_bad_params_before_queueing_input() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let cases = [
        ("ui.click", json!({"x": 10, "y": 10, "buton": "right"}), "buton"),
        ("ui.move", json!({"x": 10, "y": 10, "count": 2}), "count"),
        ("ui.click", json!({"x": "abc", "y": 10}), "x"),
        ("ui.move", json!({"x": 10, "y": [10]}), "y"),
        ("ui.click", json!({"x": 10, "y": 10, "count": "dos"}), "count"),
        ("ui.click", json!({"x": 10, "y": 10, "count": 1.5}), "count"),
        ("ui.click", json!({"x": 10, "y": 10, "count": -1}), "count"),
        ("ui.click", json!({"x": 10, "y": 10, "button": true}), "button"),
        ("ui.click", json!({"x": 10, "y": 10, "button": "ritgh"}), "button"),
        ("ui.click", json!({"x": 10, "y": 10, "count": MAX_CLICKS + 1}), "count"),
        ("ui.click", json!("click"), "params"),
    ];
    for (method, params, names) in cases {
        rejected(&mut app, &ctx, method, params, names);
    }
    let error = rejected(&mut app, &ctx, "ui.click", json!({"x": 10, "y": 10, "button": "ritgh"}), "button");
    assert!(error.contains("left") && error.contains("right") && error.contains("middle"), "the error lists the buttons: {error}");
    // Valid calls queue what they did before; an omitted button is a left click.
    for (method, params, button, events) in [
        ("ui.click", json!({"x": 10, "y": 10}), egui::PointerButton::Primary, 3),
        ("ui.click", json!({"x": 10, "y": 10, "button": "right", "count": 2}), egui::PointerButton::Secondary, 5),
        ("ui.click", json!({"x": 10, "y": 10, "button": "secondary"}), egui::PointerButton::Secondary, 3),
        ("ui.click", json!({"x": 10.5, "y": 10, "button": "middle", "count": null}), egui::PointerButton::Middle, 3),
        ("ui.move", json!({"x": 10, "y": 10}), egui::PointerButton::Primary, 1),
    ] {
        let (req, _rx) = ControlRequest::new(method, params.clone());
        assert!(matches!(handle(&mut app, &ctx, &req), Outcome::AfterInput), "{method} {params}");
        assert_eq!(app.synthetic.len(), events, "{method} {params}");
        if let Some(egui::Event::PointerButton { button: queued, .. }) = app.synthetic.get(1) {
            assert_eq!(*queued, button, "{method} {params}");
        }
        app.synthetic.clear();
    }
}

#[test]
fn ui_key_rejects_bad_params_before_queueing_input() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let cases = [
        (json!({"key": "N", "ctl": true}), "ctl"),
        (json!({"key": "N", "modifiers": {"space": true}}), "space"),
        (json!({"key": "N", "ctrl": "yes"}), "ctrl"),
        (json!({"key": "N", "modifiers": {"shift": 1}}), "shift"),
        (json!({"key": "N", "modifiers": "ctrl"}), "modifiers"),
        (json!({"key": 5}), "key"),
        (json!({}), "key"),
        (json!({"key": "NoSuchKey"}), "NoSuchKey"),
    ];
    for (params, names) in cases {
        rejected(&mut app, &ctx, "ui.key", params, names);
    }
    for params in [json!({"key": "N", "ctrl": true}), json!({"key": "N", "shift": false, "modifiers": {"ctrl": true}})] {
        let (req, _rx) = ControlRequest::new("ui.key", params.clone());
        assert!(matches!(handle(&mut app, &ctx, &req), Outcome::AfterInput), "{params}");
        assert!(matches!(app.synthetic.first(), Some(egui::Event::Key { key: egui::Key::N, modifiers, .. }) if modifiers.ctrl), "{params}");
        app.synthetic.clear();
    }
}

#[test]
fn ui_screenshot_rejects_bad_params_before_capturing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    for (params, names) in [(json!({"focus": "yes"}), "focus"), (json!({"path": 5}), "path"), (json!({"pth": "shot.png"}), "pth")] {
        rejected(&mut app, &ctx, "ui.screenshot", params, names);
    }
    for (params, path) in [(json!({}), None), (Value::Null, None), (json!({"path": "shot.png", "focus": false}), Some("shot.png"))] {
        let (req, _rx) = ControlRequest::new("ui.screenshot", params.clone());
        let Outcome::Screenshot { path: queued, .. } = handle(&mut app, &ctx, &req) else { panic!("{params}: expected a capture") };
        assert_eq!(queued.as_deref(), path, "{params}");
    }
}

/// A key event's modifiers, as `ui.key` queued them.
fn key_modifiers(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> egui::Modifiers {
    app.synthetic.clear();
    let (req, _rx) = ControlRequest::new("ui.key", params.clone());
    assert!(matches!(handle(app, ctx, &req), Outcome::AfterInput), "{params}");
    match app.synthetic.first() {
        Some(egui::Event::Key { modifiers, .. }) => *modifiers,
        other => panic!("{params}: expected a key event, got {other:?}"),
    }
}

#[test]
fn ui_key_ctrl_reaches_shortcuts_like_a_real_ctrl_press() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let ctrl = key_modifiers(&mut app, &ctx, json!({"key":"N","ctrl":true}));
    let command = key_modifiers(&mut app, &ctx, json!({"key":"N","command":true}));
    let grouped = key_modifiers(&mut app, &ctx, json!({"key":"N","modifiers":{"ctrl":true}}));
    assert_eq!(ctrl, grouped, "a grouped flag must match the top-level one");
    if cfg!(target_os = "macos") {
        // Two different keys: Control is not ⌘, and only ⌘ matches a shortcut.
        assert!(ctrl.ctrl && !ctrl.command && !ctrl.mac_cmd, "{ctrl:?}");
        assert!(command.command && command.mac_cmd && !command.ctrl, "{command:?}");
    } else {
        // One key under two names, and shortcuts are matched on `command`, so asking for
        // either must reach them: Ctrl+N has to open New Document, not do nothing.
        assert_eq!(ctrl, command, "ctrl and command are the same key off the Mac");
        assert!(ctrl.command && ctrl.ctrl && !ctrl.mac_cmd, "{ctrl:?}");
    }
    let plain = key_modifiers(&mut app, &ctx, json!({"key":"N"}));
    assert_eq!(plain, egui::Modifiers::default(), "{plain:?}");
    let shift_alt = key_modifiers(&mut app, &ctx, json!({"key":"N","shift":true,"alt":true}));
    assert!(shift_alt.shift && shift_alt.alt && !shift_alt.command && !shift_alt.ctrl, "{shift_alt:?}");
}

/// FILE-215-3: `filterMaskTarget` on a layer without a filter mask is refused before anything
/// changes, so the layer mask stays targeted.
#[test]
fn ui_set_filter_mask_target_needs_a_smart_object_with_filters() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": true}))["ok"], true);
    let bad = call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}));
    assert_eq!(bad["ok"], false, "{bad}");
    assert!(app.ui.mask_target, "the layer mask stays targeted");
    assert_eq!(app.ui.filter_mask_target, None);
    // A smart object with smart filters takes it.
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    // Not while a single colour channel is targeted: nothing would end up targeted.
    app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Color(0);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], false);
    app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Composite;
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], true);
    assert!(app.filter_mask_targeted());
    // Untargeting it ends its view, which would otherwise keep it targeted.
    app.run("paint.stroke", json!({"points": [[4, 4]], "size": 4, "hardness": 1, "color": "#000000", "target": "filterMask"})).unwrap();
    app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": false}))["ok"], true);
    assert!(!app.filter_mask_targeted());
    assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none());
    // Targeting the pixels untargets it too.
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget": true}))["ok"], true);
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": false}))["ok"], true);
    assert!(!app.filter_mask_targeted());
    assert_eq!(crate::canvas::paint_target(&app), json!("pixels"));
}

/// FILE-215-3: with the filter mask shown (⌥-click its thumbnail), targeting the layer mask
/// ends that view, and the brush paints the layer mask.
#[test]
fn ui_set_mask_target_ends_a_filter_mask_view() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("paint.stroke", json!({"points": [[4, 4]], "size": 4, "hardness": 1, "color": "#000000", "target": "filterMask"})).unwrap();
    app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    app.sync_mask_targets();
    assert!(app.filter_mask_targeted());
    assert_eq!(call(&mut app, &ctx, "ui.set", json!({"maskTarget": true}))["ok"], true);
    assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none(), "the filter mask view ends");
    assert!(app.layer_mask_targeted());
    assert!(!app.filter_mask_targeted());
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"));
    let target = crate::canvas::paint_target(&app);
    app.run("paint.stroke", json!({"points": [[20, 20]], "size": 4, "hardness": 1, "color": "#000000", "target": target})).unwrap();
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(l.mask.as_ref().unwrap().value(20, 20), 0.0, "the stroke went into the layer mask");
    let photocraft_doc::LayerContent::Smart(sm) = &l.content else { panic!("smart object") };
    assert_eq!(sm.filter_mask.as_ref().unwrap().value(20, 20), 1.0, "not into the filter mask");
}
