use super::*;

/// PhotoCraft's own roundtrip accepts a combined Set, but Photoshop applies only its rename.
#[test]
fn compound_properties_export_as_separate_sets_with_rename_last() {
    for target in [None, Some("Original")] {
        let mut params = json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"});
        if let Some(name) = target {
            params["layer"] = json!(name);
        }
        let action = Action::new("Properties", vec![("layer.setProps".into(), params.clone())]);
        let (bytes, skipped) = export("Set", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        let items = &file.actions[0].items;
        assert_eq!(items.len(), 3);
        let expected = [("Opct", pct(37.0)), ("Md  ", enm("BlnM", "Mltp")), ("Nm  ", text("Renamed"))];
        for (item, (key, value)) in items.iter().zip(expected) {
            assert_eq!(item.event.id(), "setd");
            let descriptor = item.descriptor.as_ref().unwrap();
            assert_eq!(descriptor.get("null"), Some(&layer_ref(&params).unwrap()));
            let properties = get_obj(descriptor, "T   ").unwrap();
            assert!(properties.class_id.is("Lyr "));
            assert_eq!(properties.items.len(), 1, "Photoshop must receive each property independently");
            assert_eq!(properties.get(key), Some(&value));
        }
    }
}

#[test]
fn expanded_properties_keep_flags_and_original_skip_indices() {
    let mut action = Action::new(
        "Flags",
        vec![
            ("layer.setProps".into(), json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"})),
            ("filter.blur.gaussianBlur".into(), json!({"radius": 2})),
            ("filter.render.clouds".into(), json!({})),
        ],
    );
    action.disabled.insert(0);
    action.dialogs.insert(0);
    let (bytes, skipped) = export("Set", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Flags".into(), step: 2, id: "filter.render.clouds".into() }]);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert_eq!(items.len(), 4);
    for item in &items[..3] {
        assert!(!item.enabled);
        assert!(item.with_dialog);
        assert_eq!(item.dialog_options, 2);
    }
    assert_eq!(items[3].event.id(), "GsnB");
    assert!(items[3].enabled);
    assert!(!items[3].with_dialog);
    assert_eq!(items[3].dialog_options, 0);
    let back = import(&bytes).unwrap();
    assert_eq!(back.actions[0].disabled.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(back.actions[0].dialogs.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn invalid_compound_properties_fail_or_skip_the_whole_source_step() {
    let action = Action::new(
        "Atomic",
        vec![
            ("filter.blur.gaussianBlur".into(), json!({"radius": 2})),
            ("layer.setProps".into(), json!({"name": "Renamed", "opacity": 0.37, "blend": "invalid blend"})),
            ("select.inverse".into(), json!({})),
        ],
    );
    let error = export("Set", &[&action], &[], false).unwrap_err();
    assert!(error.contains("step 2"), "{error}");
    let (bytes, skipped) = export("Set", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Atomic".into(), step: 1, id: "layer.setProps".into() }]);
    let file = atn::parse(&bytes).unwrap();
    let events: Vec<_> = file.actions[0].items.iter().map(|item| item.event.id()).collect();
    assert_eq!(events, ["GsnB", "Invs"]);
}

#[test]
fn single_layer_properties_keep_their_existing_descriptor() {
    for params in [json!({"name": "Renamed"}), json!({"opacity": 0.37}), json!({"blend": "Multiply"}), json!({"fill": 0.5}), json!({"visible": false})] {
        let action = Action::new("Single", vec![("layer.setProps".into(), params.clone())]);
        let expected = export_item("layer.setProps", &params, "Set").unwrap();
        let (bytes, skipped) = export("Set", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        assert_eq!(file.actions[0].items.len(), 1);
        assert_eq!(file.actions[0].items[0].event, expected.event);
        assert_eq!(file.actions[0].items[0].descriptor, expected.descriptor);
    }
}

#[test]
fn fill_and_visibility_also_export_separately_before_rename() {
    let action = Action::new("Other properties", vec![("layer.setProps".into(), json!({"name": "Renamed", "fill": 0.5, "visible": false}))]);
    let (bytes, _) = export("Set", &[&action], &[], false).unwrap();
    let file = atn::parse(&bytes).unwrap();
    let keys: Vec<_> = file.actions[0]
        .items
        .iter()
        .map(|item| {
            let properties = get_obj(item.descriptor.as_ref().unwrap(), "T   ").unwrap();
            assert_eq!(properties.items.len(), 1);
            trimmed(&properties.items[0].0)
        })
        .collect();
    assert_eq!(keys, ["fillOpacity", "Vsbl", "Nm"]);
}

#[test]
fn expanded_properties_replay_like_direct_edits_at_every_depth() {
    for depth in [8, 16, 32] {
        for named in [false, true] {
            let prepare = || {
                let mut session = Session::new();
                session.execute("file.new", json!({"width": 2, "height": 2, "depth": depth})).unwrap();
                session.execute("layer.new.layer", json!({"name": "Original"})).unwrap();
                session.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
                if !named {
                    session.execute("layer.select", json!({"layer": "Original"})).unwrap();
                }
                session
            };
            let mut params = json!({"name": "Renamed", "opacity": 0.37, "blend": "Multiply"});
            if named {
                params["layer"] = json!("Original");
            }
            let mut direct = prepare();
            direct.execute("layer.setProps", params.clone()).unwrap();
            let action = Action::new("Properties", vec![("layer.setProps".into(), params)]);
            let (bytes, _) = export("Set", &[&action], &[], false).unwrap();
            let mut replay = prepare();
            replay.actions.list = import(&bytes).unwrap().actions;
            let result = replay.execute("actions.play", json!({"action": "Properties"})).unwrap();
            assert!(result["failed"].is_null(), "{result}");
            assert_eq!(result["ran"], 3);
            let state = |session: &Session| {
                session
                    .active()
                    .unwrap()
                    .doc
                    .walk()
                    .iter()
                    .map(|(_, _, layer)| (layer.name.clone(), layer.opacity, layer.blend, layer.visible))
                    .collect::<Vec<_>>()
            };
            assert_eq!(state(&direct), state(&replay), "depth={depth}, named={named}");
        }
    }
}
