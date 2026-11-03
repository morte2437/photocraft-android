use super::*;

/// A 24×16 document with three layers, "Test 0" to "Test 2", above the Background; the last one is active.
fn new_session() -> (Session, Vec<LayerId>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 16})).unwrap();
    let ids = (0..3).map(|n| LayerId(s.execute("layer.new.layer", json!({"name": format!("Test {n}")})).unwrap()["layer"].as_u64().unwrap())).collect();
    (s, ids)
}

/// Set one Layers panel option in the preferences, as the Panel Options dialog does.
fn option(s: &mut Session, key: &str, enabled: bool) {
    s.prefs.edit(|p| {
        let val = p.dialogs.entry("ui.layersPanelOptions".into()).or_insert_with(|| json!({}));
        if let Some(o) = val.as_object_mut() {
            o.insert(key.to_owned(), json!(enabled));
        }
    });
}

/// The drag preview equals the committed `layer.moveTo` result and never touches the document or its history.
#[test]
fn preview_matches_real_move_and_never_edits_document_or_history() {
    let (mut s, ids) = new_session();
    let params = json!({"layer": ids[0].0, "target": ids[2].0, "position": "above"});
    let before = s.active().unwrap().doc.clone();
    let undo_before = s.active().unwrap().history.entries().len();
    let candidate = preview_move(&before, &params, true).unwrap();
    assert_eq!(*before, *s.active().unwrap().doc);
    assert_eq!(s.active().unwrap().history.entries().len(), undo_before);
    s.execute("layer.moveTo", params).unwrap();
    assert_eq!(candidate, *s.active().unwrap().doc, "visual preview must match committed layer order");
}

/// A previewed ⌥-drag copy names its layers like the committed copy, with and without the "copy" suffix option.
#[test]
fn preview_copy_and_no_copy_suffix_option_match_committed_layers() {
    let (mut s, ids) = new_session();
    option(&mut s, "addCopyToCopiedLayersAndGroups", false);
    let params = json!({"layer": ids[0].0, "target": ids[2].0, "position": "below", "copy": true});
    let before = s.active().unwrap().doc.clone();
    let candidate = preview_move(&before, &params, false).unwrap();
    s.execute("layer.moveTo", params).unwrap();
    let actual = &s.active().unwrap().doc;
    assert_eq!(candidate.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), actual.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>());
    assert_eq!(actual.layer_count(), before.layer_count() + 1);
    assert_eq!(actual.layers.iter().filter(|l| l.name == "Test 0").count(), 2);
    let d = s.execute("layer.duplicate", json!({"layer": ids[1].0})).unwrap();
    let duplicate = LayerId(d["layer"].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(duplicate).unwrap().name, "Test 1");
    option(&mut s, "addCopyToCopiedLayersAndGroups", true);
    let d = s.execute("layer.duplicate", json!({"layer": ids[1].0})).unwrap();
    assert!(s.active().unwrap().doc.layer(LayerId(d["layer"].as_u64().unwrap())).unwrap().name.contains("copy"));
}

/// Drops the command refuses (or ignores) give no preview and leave the document alone; a multi-layer preview follows the committed stack order.
#[test]
fn preview_rejects_invalid_drop_unchanged() {
    let (mut s, ids) = new_session();
    let before = s.active().unwrap().doc.clone();
    // Dropping a layer on itself is a no-op for the command, the others are errors.
    for (p, command_rejects) in [
        (json!({"layer": ids[1].0, "target": ids[1].0, "position": "above"}), false),
        (json!({"layer": ids[1].0, "target": 999999, "position": "below"}), true),
        (json!({"layer": ids[1].0, "target": ids[2].0, "position": "into"}), true),
        (json!({"layer": ids[1].0, "target": ids[2].0, "position": "invalid"}), true),
    ] {
        assert!(preview_move(&before, &p, true).is_none(), "{p}");
        let result = s.execute("layer.moveTo", p.clone());
        if command_rejects {
            assert!(result.is_err(), "{p}");
        } else {
            assert_eq!(result.unwrap(), Value::Null, "{p}");
        }
        assert_eq!(*before, *s.active().unwrap().doc, "{p}");
    }
    // Multi-layer preview and commit follow the same stack order regardless of params order.
    let p = json!({"layers": [ids[2].0, ids[0].0], "target": ids[1].0, "position": "below"});
    let candidate = preview_move(&before, &p, true).unwrap();
    s.execute("layer.moveTo", p).unwrap();
    assert_eq!(candidate, *s.active().unwrap().doc);
}

/// A 16×16 document holding `spec` (bottom to top, `*` = clipped) above its Background. Every
/// layer is a translucent, differently placed rectangle, so the stack order and the clip
/// groups both show in the composite.
fn clip_stack(spec: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    for (n, name) in spec.split_whitespace().enumerate() {
        let id = LayerId(s.execute("layer.new.layer", json!({"name": name.trim_end_matches('*')})).unwrap()["layer"].as_u64().unwrap());
        let n = n as i32;
        let colour = [(n % 3) as f32 * 0.5, ((n + 1) % 3) as f32 * 0.5, ((n + 2) % 3) as f32 * 0.5, 0.6];
        s.edit("paint", |doc, _| {
            let layer = doc.layer_mut(id).unwrap();
            layer.clipped = name.ends_with('*');
            layer.surface_mut().unwrap().fill_rect(Rect::new(n * 2, 0, n * 2 + 10, 16), &colour);
            Ok(())
        })
        .unwrap();
    }
    s
}

/// Every layer of `doc` from the bottom, `*` marking the clipped ones.
fn stack_text(doc: &Document) -> String {
    doc.layers.iter().map(|l| format!("{}{}", l.name, if l.clipped { "*" } else { "" })).collect::<Vec<_>>().join(" ")
}

/// The id of the layer called `name`, searching the whole layer tree.
fn id_of(s: &Session, name: &str) -> u64 {
    s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id.0
}

/// The preview of `params` and the committed `layer.moveTo` show the same stack (names, clip
/// flags) and the same composite, and both are `want`.
fn assert_preview_is_commit(mut s: Session, params: Value, want: &str) {
    let before = s.active().unwrap().doc.clone();
    let preview = preview_move(&before, &params, true).expect("the drop has a preview");
    s.execute("layer.moveTo", params.clone()).unwrap();
    let committed = &s.active().unwrap().doc;
    assert_eq!(stack_text(&preview), want, "preview of {params}");
    assert_eq!(stack_text(committed), want, "commit of {params}");
    assert_eq!(photocraft_compose::flatten(&preview).px, photocraft_compose::flatten(committed).px, "composite of {params}");
}

/// Previewed moves join and release clipping groups exactly as the committed reorder does, copies included.
#[test]
fn preview_follows_clip_groups_like_the_committed_reorder() {
    // The fixtures of `clip_group::reorder_joins_and_leaves_groups`.
    for (spec, layers, target, pos, want) in [
        // A layer dropped into a clip group joins it.
        ("B C1* C2* U", vec!["U"], "C1", "above", "Background B C1* U* C2*"),
        ("B C1* C2* U V", vec!["U", "V"], "C1", "above", "Background B C1* U* V* C2*"),
        // A clipped layer dropped off its group is released.
        ("B C1* C2* U", vec!["C1"], "U", "above", "Background B C2* U C1"),
        // A base moved away releases the layers clipped to it.
        ("B C1* C2* U", vec!["B"], "U", "above", "Background C1 C2 U B"),
        // A base moved up into its own group joins it, and the layers it left are released.
        ("B C1* C2*", vec!["B"], "C1", "above", "Background C1 B* C2"),
        ("X B C1* C2* U", vec!["B", "C1"], "U", "above", "Background X C2 U B C1*"),
    ] {
        let s = clip_stack(spec);
        let ids: Vec<u64> = layers.iter().map(|n| id_of(&s, n)).collect();
        let t = id_of(&s, target);
        assert_preview_is_commit(s, json!({"layers": ids, "target": t, "position": pos}), want);
    }
    // An option-drag copy joins the clip group it lands in too.
    let s = clip_stack("B C1* C2* U");
    let (u, c1) = (id_of(&s, "U"), id_of(&s, "C1"));
    assert_preview_is_commit(s, json!({"layer": u, "target": c1, "position": "above", "copy": true}), "Background B C1* U copy* C2* U");
}

/// Copying the locked Background in a preview gives the same unlocked copy, joining the clip group it lands in, as the command.
#[test]
fn preview_copies_the_background_like_the_committed_copy() {
    let mut s = clip_stack("A B*");
    let (bg, a) = (s.active().unwrap().doc.background().unwrap().id, id_of(&s, "A"));
    let before = s.active().unwrap().doc.clone();
    assert_ne!(before.layer(bg).unwrap().locks, Default::default(), "the Background is locked");
    let p = json!({"layer": bg.0, "target": a, "position": "above", "copy": true});
    let preview = preview_move(&before, &p, true).unwrap();
    s.execute("layer.moveTo", p).unwrap();
    let committed = &s.active().unwrap().doc;
    // The copy is an ordinary, unlocked layer that joins the clip group it lands in.
    assert_eq!(stack_text(committed), "Background A Background copy* B*");
    assert_eq!(stack_text(&preview), stack_text(committed));
    for doc in [&preview, committed] {
        assert_eq!(doc.layers.get(2).unwrap().locks, Default::default());
        assert!(doc.layers.first().unwrap().locks != Default::default());
    }
    assert_eq!(photocraft_compose::flatten(&preview).px, photocraft_compose::flatten(committed).px);
}

/// A drop that would move the Background off the bottom has no preview, as the command refuses it.
#[test]
fn preview_refuses_what_would_leave_the_background_off_the_bottom() {
    let mut s = clip_stack("A B");
    let (bg, a) = (s.active().unwrap().doc.background().unwrap().id, id_of(&s, "A"));
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({"layer": a, "target": bg.0, "position": "below"}),
        json!({"layer": bg.0, "target": bg.0, "position": "below", "copy": true}),
        json!({"layer": a, "target": bg.0, "position": "below", "copy": true}),
    ] {
        assert!(preview_move(&before, &p, true).is_none(), "{p}");
        assert!(s.execute("layer.moveTo", p.clone()).is_err(), "{p}");
        assert_eq!(*before, *s.active().unwrap().doc, "{p}");
    }
}

/// The UI runs commands through `start`; Expand New Effects applies there as well as to `execute`.
#[test]
fn expand_new_effects_applies_to_ui_start_dispatch_too() {
    let (mut s, ids) = new_session();
    option(&mut s, "expandNewEffects", false);
    let target = ids[0];
    let started = s.start("layer.layerStyle.dropShadow", json!({"layer": target.0})).unwrap();
    assert!(matches!(started, crate::jobs::Started::Done(_)));
    assert!(s.active().unwrap().fx_collapsed.contains(&target));
}

/// New Solid Color and Gradient fill layers get the default mask only while Use Default Masks on Fill Layers is on.
#[test]
fn fill_layers_respect_use_default_masks_option() {
    let (mut s, _) = new_session();
    option(&mut s, "useDefaultMasksOnFillLayers", false);
    let new = s.execute("layer.newFillLayer.solidColor", json!({"color": "#112233"})).unwrap();
    let id = LayerId(new["layer"].as_u64().unwrap());
    assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none());
    option(&mut s, "useDefaultMasksOnFillLayers", true);
    let new = s.execute("layer.newFillLayer.gradient", json!({})).unwrap();
    let id = LayerId(new["layer"].as_u64().unwrap());
    assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_some());
}

/// With the option off only the default reveal-all mask goes away: an active selection still
/// becomes the layer mask and an explicit path still becomes the vector mask (path first).
#[test]
fn fill_layers_without_default_masks_keep_selection_and_explicit_path_masks() {
    const KINDS: [(&str, &str); 3] = [
        ("layer.newFillLayer.solidColor", r##"{"color": "#112233"}"##),
        ("layer.newFillLayer.gradient", "{}"),
        ("layer.newFillLayer.pattern", r#"{"pattern": "Checkerboard"}"#),
    ];
    let path = json!({"subpaths": [{"closed": true, "knots": [[2, 2], [10, 2], [10, 10], [2, 10]]}]});
    let (mut s, _) = new_session();
    option(&mut s, "useDefaultMasksOnFillLayers", false);
    // Creates the fill layer; returns its raster mask and whether it has a vector mask.
    let create = |s: &mut Session, cmd: &str, params: &str, path: Option<&Value>| {
        let mut p: Value = serde_json::from_str(params).unwrap();
        if let Some(path) = path {
            p["path"] = path.clone();
        }
        let id = LayerId(s.execute(cmd, p).unwrap()["layer"].as_u64().unwrap());
        let l = s.active().unwrap().doc.layer(id).unwrap();
        (l.mask.clone(), l.vector_mask.is_some())
    };

    for (cmd, params) in KINDS {
        let (mask, vector) = create(&mut s, cmd, params, None);
        assert!(mask.is_none() && !vector, "{cmd}: no selection, no path: no mask at all");
    }

    for (cmd, params) in KINDS {
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 6})).unwrap();
        let (mask, vector) = create(&mut s, cmd, params, None);
        let mask = mask.unwrap_or_else(|| panic!("{cmd}: the selection still becomes the layer mask"));
        assert!(!vector, "{cmd}");
        assert!(mask.surface.pixel(6, 6)[0] > 0.99, "{cmd}: inside the selection reveals");
        assert!(mask.surface.pixel(20, 12)[0] < 0.01, "{cmd}: outside the selection hides");
        assert!(s.active().unwrap().doc.selection.is_none(), "{cmd}: the selection went into the mask (#8801)");
    }

    // An explicit path wins over the selection, as with the option on.
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 6})).unwrap();
    for selected in [true, false] {
        if !selected {
            s.execute("select.deselect", json!({})).unwrap();
        }
        for (cmd, params) in KINDS {
            let (mask, vector) = create(&mut s, cmd, params, Some(&path));
            assert!(vector, "{cmd}: the explicit path becomes the vector mask (selection: {selected})");
            assert!(mask.is_none(), "{cmd}: no raster mask next to the vector mask (selection: {selected})");
        }
    }
}

/// Whether new effects start expanded is view state: it adds no history step, and editing an existing effect keeps its disclosure.
#[test]
fn expand_new_effects_is_view_state_and_keeps_edit_history_clean() {
    let (mut s, ids) = new_session();
    option(&mut s, "expandNewEffects", false);
    let target = ids[0];
    let history = s.active().unwrap().history.entries().len();
    s.execute("layer.layerStyle.dropShadow", json!({"layer": target.0, "size": 2})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), history + 1);
    assert!(s.active().unwrap().fx_collapsed.contains(&target));
    option(&mut s, "expandNewEffects", true);
    let other = ids[1];
    s.execute("layer.layerStyle.stroke", json!({"layer": other.0, "size": 2})).unwrap();
    assert!(!s.active().unwrap().fx_collapsed.contains(&other));
    // Editing the existing effect must not unexpectedly change its disclosure.
    s.execute("layer.layerStyle.dropShadow", json!({"layer": target.0, "size": 5})).unwrap();
    assert!(s.active().unwrap().fx_collapsed.contains(&target));
}

/// The disclosure follows the layer the command changes, however the command names it: a
/// layer given by name is not the active one.
#[test]
fn expand_new_effects_follows_a_layer_named_by_its_name() {
    let (mut s, ids) = new_session();
    option(&mut s, "expandNewEffects", false);
    let (named, active) = (ids[0], ids[2]);
    assert_eq!(s.active().unwrap().active_layer, Some(active));
    s.execute("layer.layerStyle.dropShadow", json!({"layer": "Test 0", "size": 2})).unwrap();
    assert!(s.active().unwrap().doc.layer(named).is_some_and(|l| !l.effects.items.is_empty()));
    assert!(s.active().unwrap().fx_collapsed.contains(&named), "the named layer's new effects start collapsed");
    assert!(!s.active().unwrap().fx_collapsed.contains(&active), "the active layer is not the one changed");
}

/// Transfer Effects changes the layer it moves the effects to, so that is the layer whose
/// disclosure follows the option, not the active layer or the one the effects came from.
#[test]
fn expand_new_effects_follows_the_transfer_destination() {
    let (mut s, ids) = new_session();
    let (from, to, active) = (ids[0], ids[1], ids[2]);
    s.execute("layer.layerStyle.dropShadow", json!({"layer": from.0, "size": 2})).unwrap();
    option(&mut s, "expandNewEffects", false);
    assert_eq!(s.active().unwrap().active_layer, Some(active));
    s.execute("layer.layerStyle.transferEffects", json!({"from": from.0, "to": to.0, "copy": true})).unwrap();
    assert!(s.active().unwrap().doc.layer(to).is_some_and(|l| !l.effects.items.is_empty()));
    assert!(s.active().unwrap().fx_collapsed.contains(&to), "the destination's new effects start collapsed");
    assert!(!s.active().unwrap().fx_collapsed.contains(&active), "the active layer is not the one changed");
}
