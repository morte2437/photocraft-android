//! The remaining panel commands compose with the same definitions, journal and ATN boundary.
use photocraft_engine::{Session, actions_cmds::Action, actions_edit_cmds, atn_cmds, jobs::Started};
use serde_json::json;

/// Tiny real document with one empty action keeps recording and background jobs observable.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":40,"height":30})).unwrap();
    s.actions.list.push(Action::new("A", vec![]));
    s
}

/// A painted stripe gives both the remove tool and blur a real input before recording starts.
fn stroke(s: &mut Session, color: &str) {
    s.execute("paint.stroke", json!({"points":[[8,15],[32,15]],"size":3,"color":color})).unwrap();
}

/// Mode changes flush under the previous mode without altering painting or ordinary recording.
#[test]
fn tool_recording_is_opt_in_and_toggles_do_not_reclassify_pending_steps() {
    let mut s = session();
    s.execute("actions.record", json!({"action":"A"})).unwrap();
    stroke(&mut s, "#ff0000");
    s.execute("select.all", json!({})).unwrap();
    let original = s.actions.clone();
    assert!(s.execute("actions.allowToolRecording", json!({"enabled":"yes"})).is_err());
    assert_eq!(s.actions, original, "invalid mode must not flush pending work");
    s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
    stroke(&mut s, "#00ff00");
    s.execute("actions.allowToolRecording", json!({"enabled":false})).unwrap();
    stroke(&mut s, "#0000ff");
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["select.all", "paint.stroke", "select.deselect"]);
    assert_eq!(s.actions.list[0].steps[1].1["color"], "#00ff00");
    assert_eq!(s.journal.iter().filter(|s| s.0 == "paint.stroke").count(), 3, "all actual painting remains journaled");
    assert_eq!(s.execute("actions.list", json!({})).unwrap()["toolRecording"], false);
}

/// A stroke started while off never joins a later enabled recording; ordinary jobs keep their owner.
#[test]
fn background_stroke_ownership_does_not_suppress_ordinary_filter_jobs() {
    for command in ["paint.remove", "filter.blur.gaussianBlur"] {
        let mut s = session();
        stroke(&mut s, "#bb3300");
        s.execute("actions.record", json!({"action":"A"})).unwrap();
        let params = if command == "paint.remove" { json!({"points":[[20,15],[21,15]],"size":3}) } else { json!({"radius":1}) };
        let Started::Job(job) = s.start(command, params).unwrap() else { panic!("expected the real background job: {command}") };
        let changed = s.execute("actions.allowToolRecording", json!({"enabled":true}));
        if command == "paint.remove" {
            assert!(changed.is_err(), "the in-flight stroke retains the mode it started under");
            assert!(!s.actions.record_tools);
        } else {
            changed.unwrap();
        }
        s.wait_job(job).unwrap();
        if command == "paint.remove" {
            s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
        }
        s.execute("actions.stop", json!({})).unwrap();
        let ids = s.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>();
        if command == "paint.remove" {
            assert!(ids.is_empty());
        } else {
            assert_eq!(ids, [command]);
        }
    }
}

/// A Remove job started outside the current take neither blocks its toggle nor joins its steps.
#[test]
fn background_strokes_from_outside_the_current_take_do_not_block_the_toggle() {
    for previous_take in [false, true] {
        let mut s = session();
        stroke(&mut s, "#bb3300");
        if previous_take {
            s.execute("actions.record", json!({"action":"A"})).unwrap();
            s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap();
        }
        let Started::Job(job) = s.start("paint.remove", json!({"points":[[20,15],[21,15]],"size":3})).unwrap() else {
            panic!("expected the real Remove job");
        };
        if previous_take {
            s.execute("actions.stop", json!({})).unwrap();
            s.execute("actions.allowToolRecording", json!({"enabled":false})).unwrap();
        }
        s.execute("actions.record", json!({"action":"A"})).unwrap();
        assert_eq!(s.execute("actions.allowToolRecording", json!({"enabled":true})).unwrap()["enabled"], true);
        s.wait_job(job).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert!(s.actions.list[0].steps.is_empty());
        assert_eq!(s.journal.iter().filter(|step| step.0 == "paint.remove").count(), 1);
    }
}

/// Interleaved membership and numeric calls expose incorrect folder placement or target rewriting.
#[test]
fn set_duplication_preserves_metadata_call_targets_and_empty_folder_order() {
    let mut s = session();
    s.actions.list.clear();
    for (name, set) in [("A", "S"), ("Other", "T"), ("B", "S")] {
        let mut a = Action::new(name, vec![("actions.play".into(), json!({"action":2}))]);
        a.set = Some(set.into());
        a.color = "red".into();
        a.disabled.insert(0);
        a.dialogs.insert(0);
        s.actions.list.push(a);
    }
    s.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:A".into(), "F5".into());
    });
    let result = s.execute("actions.duplicate", json!({"set":"S"})).unwrap();
    assert_eq!(result["set"], "S copy");
    assert_eq!(actions_edit_cmds::set_names(&s), ["S", "S copy", "T"]);
    assert_eq!(s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["A", "A copy", "B copy", "Other", "B"]);
    for a in &s.actions.list {
        assert_eq!(a.steps[0].1["action"], 4);
        assert_eq!(a.color, "red");
        assert!(a.disabled.contains(&0));
        assert!(a.dialogs.contains(&0));
    }
    assert!(!s.prefs().shortcuts.contains_key("actions.play:A copy"));
    let mut empty = Session::new();
    for name in ["E", "F"] {
        empty.execute("actions.newSet", json!({"name":name})).unwrap();
    }
    empty.execute("actions.duplicate", json!({"set":"E"})).unwrap();
    assert_eq!(actions_edit_cmds::set_names(&empty), ["E", "E copy", "F"]);
    let original = empty.actions.clone();
    for params in [json!({"set":"Missing"}), json!({"set":"E","action":0}), json!({"set":false})] {
        assert!(empty.execute("actions.duplicate", params).is_err());
        assert_eq!(empty.actions, original);
    }
}

/// Menu insertion keeps descriptor absence and kind; empty descriptors and ambiguous commands differ.
#[test]
fn inserted_menu_events_roundtrip_absent_settings_and_stateless_items_execute() {
    assert!(!atn_cmds::menu_item_needs_settings("filter.noise.despeckle"));
    assert!(atn_cmds::menu_item_needs_settings("filter.blur.gaussianBlur"));
    for id in ["image.imageSize", "filter.blur.gaussianBlur", "filter.noise.despeckle"] {
        for kind in ["name", "code"] {
            let mut s = session();
            s.execute("actions.insertMenuItem", json!({"action":"A","id":id,"eventKind":kind})).unwrap();
            let bytes = atn_cmds::export("S", &[&s.actions.list[0]], &[], false).unwrap().0;
            let raw = craft_psd::atn::parse(&bytes).unwrap();
            assert!(raw.actions[0].items[0].descriptor.is_none());
            let imported = atn_cmds::import(&bytes).unwrap();
            let (step, params) = &imported.actions[0].steps[0];
            assert_eq!(step, atn_cmds::MENU_ITEM);
            assert_eq!(params["id"], id);
            assert_eq!(params["eventKind"], kind);
            let (saved, skipped) = atn_cmds::export("S", &[&imported.actions[0]], &[], false).unwrap();
            assert!(skipped.is_empty());
            let saved = craft_psd::atn::parse(&saved).unwrap();
            assert_eq!(saved.actions[0].items[0], raw.actions[0].items[0]);
            let played = s.execute("actions.play", json!({"action":"A"})).unwrap();
            assert_eq!(played.get("failed").is_none(), !atn_cmds::menu_item_needs_settings(id), "{id}: {played}");
        }
    }
    let mut s = session();
    let original = s.actions.clone();
    for params in
        [json!({"action":"A","id":"layer.setProps"}), json!({"action":"A","id":"file.open"}), json!({"action":"A","id":"image.imageSize","params":{"width":4}})]
    {
        assert!(s.execute("actions.insertMenuItem", params).is_err());
        assert_eq!(s.actions, original);
    }
    let mut raw = craft_psd::atn::AtnFile::default();
    raw.actions.push(craft_psd::atn::AtnAction {
        items: vec![craft_psd::atn::AtnItem {
            event: craft_psd::atn::AtnEvent::name("gaussianBlur"),
            descriptor: Some(craft_psd::descriptor::Descriptor::new("null")),
            ..Default::default()
        }],
        ..Default::default()
    });
    let loaded = atn_cmds::import(&craft_psd::atn::write(&raw).unwrap()).unwrap();
    assert_eq!(loaded.actions[0].steps[0].0, "filter.blur.gaussianBlur");
}

/// A full curve is snapshotted independently of later source edits and replays as the work path.
#[test]
fn insert_path_captures_cubic_geometry_without_editing_the_document() {
    let mut s = session();
    let path = json!({"fillRule":"evenodd","inverted":true,"subpaths":[{"closed":false,"op":"combine","knots":[{"anchor":[1.5,2.25],"in":[-1.0,3.0],"out":[5.0,8.0],"smooth":true},{"anchor":[20.0,18.0],"in":[12.0,5.0],"out":[21.0,19.0]}]}]});
    s.execute("path.set", json!({"name":"Curve","path":path})).unwrap();
    let expected = s.execute("path.info", json!({"name":"Curve"})).unwrap()["path"].clone();
    let revision = s.active().unwrap().revision;
    assert!(s.execute("actions.insertPath", json!({"action":"A","name":"work"})).is_err());
    assert!(s.actions.list[0].steps.is_empty());
    s.execute("actions.insertPath", json!({"action":"A","name":"Curve"})).unwrap();
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.actions.list[0].steps[0].1["path"], expected);
    s.execute("path.set", json!({"name":"Curve","path":{"subpaths":[]}})).unwrap();
    let played = s.execute("actions.play", json!({"action":"A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.execute("path.info", json!({"name":"work"})).unwrap()["path"], expected);
    assert!(atn_cmds::export("S", &[&s.actions.list[0]], &[], false).is_err(), "the measured ATN descriptor cannot represent inversion");
}
