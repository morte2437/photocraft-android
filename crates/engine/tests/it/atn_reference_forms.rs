use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::{Value, json};

fn class(name: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(name) }
}

fn ordinal(name: &str) -> ReferenceItem {
    ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new(name) }
}

fn load(s: &mut Session, reference: ReferenceItem) {
    let descriptor = Descriptor::new("null").with("null", D::Reference(vec![reference]));
    s.actions.list = vec![imported(AtnEvent::Code(*b"slct"), descriptor)];
}

fn bytes(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
    let file = AtnFile {
        name: UnicodeString::new_nul("References"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Select"),
            items: vec![AtnItem { event, descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    };
    atn::write(&file).unwrap()
}

fn imported(event: AtnEvent, descriptor: Descriptor) -> Action {
    let mut result = atn_cmds::import(&bytes(event, descriptor)).unwrap();
    assert!(result.unsupported.is_empty(), "{:?}", result.unsupported);
    result.actions.remove(0)
}

/// Check actual saved reference kinds/codes, not only a matching import/export implementation.
fn roundtrip(action: &Action, reference: &ReferenceItem, target: Option<&ReferenceItem>) {
    let (data, skipped) = atn_cmds::export("References", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&data).unwrap();
    let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
    assert_eq!(descriptor.get("null"), Some(&D::Reference(vec![reference.clone()])));
    if let Some(target) = target {
        assert_eq!(descriptor.get("T   "), Some(&D::Reference(vec![target.clone()])));
    }
    assert_eq!(atn_cmds::import(&data).unwrap().actions[0].steps, action.steps);
}

fn replay(s: &mut Session, action: &Action) {
    s.actions.list = vec![action.clone()];
    let result = s.execute("actions.play", json!({"action":"Select"})).unwrap();
    assert_eq!(result["ran"], 1, "{result}");
    assert!(result.get("failed").is_none(), "{result}");
}

fn layer(s: &Session, name: &str) -> u64 {
    s.active().unwrap().doc.walk().into_iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id.0
}

fn selection(s: &Session) -> Value {
    let st = s.active().unwrap();
    json!({"ids":st.selected_layers().iter().map(|id|id.0).collect::<Vec<_>>(),"active":st.active_layer.map(|id|id.0),"anchor":st.layer_anchor.map(|id|id.0),"revision":st.revision})
}

/// The measured stack has a collapsed group and an optional structural Background.
fn scene(background: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":if background {"white"} else {"transparent"}})).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Base"})).unwrap();
    for name in ["L1", "L2", "Gchild"] {
        s.execute("layer.new.layer", json!({"name":name})).unwrap();
    }
    s.execute("layer.groupLayers", json!({"name":"G"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"L3"})).unwrap();
    assert_eq!(s.active().unwrap().doc.background().is_some(), background);
    s
}

fn index(class_name: &str, index: u32) -> ReferenceItem {
    ReferenceItem::Index { class: class(class_name), index }
}

fn select(reference: ReferenceItem) -> Action {
    imported(AtnEvent::Code(*b"slct"), Descriptor::new("null").with("null", D::Reference(vec![reference])))
}

fn moved(reference: ReferenceItem, target: ReferenceItem) -> Action {
    imported(AtnEvent::Code(*b"move"), Descriptor::new("null").with("null", D::Reference(vec![reference])).with("T   ", D::Reference(vec![target])))
}

fn fixture() -> (Session, u64, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.execute("layer.new.layer", json!({"name": "L1"})).unwrap();
    let l2 = s.execute("layer.new.layer", json!({"name": "L2"})).unwrap()["layer"].as_u64().unwrap();
    let child = s.execute("layer.new.layer", json!({"name": "Gchild"})).unwrap()["layer"].as_u64().unwrap();
    let group = s.execute("layer.groupLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "L3"})).unwrap();
    (s, l2, child, group)
}

/// Recorded navigation enters collapsed groups; native keyboard navigation follows visible rows.
#[test]
fn recorded_forward_enters_a_collapsed_group() {
    let (mut s, l2, child, group) = fixture();
    s.execute("layer.select", json!({"layer": l2})).unwrap();
    s.execute("layer.selectAbove", json!({})).unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, group);
    s.execute("layer.select", json!({"layer": l2})).unwrap();
    load(&mut s, ordinal("Frwr"));
    s.execute("actions.play", json!({"action": "Select"})).unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, child);
}

/// Indexes count group ends; recorded navigation visits collapsed children and retains its form.
#[test]
fn indexes_and_ordinals_use_the_recorded_stack_with_or_without_background() {
    for background in [false, true] {
        let mut s = scene(background);
        let first = u32::from(!background);
        let expected = [Some("Base"), Some("L1"), Some("L2"), None, Some("Gchild"), Some("G"), Some("L3")];
        for (offset, name) in expected.into_iter().enumerate() {
            s.execute("layer.select", json!({"layer":layer(&s,"L2")})).unwrap();
            let reference = index("Lyr ", first + offset as u32);
            let action = select(reference.clone());
            roundtrip(&action, &reference, None);
            replay(&mut s, &action);
            assert_eq!(s.active().unwrap().active_layer.unwrap().0, layer(&s, name.unwrap_or("L2")));
        }
        for (code, expected) in [("Trgt", "L2"), ("Frwr", "Gchild"), ("Bckw", "L1"), ("Frnt", "L3"), ("Back", "Base")] {
            s.execute("layer.select", json!({"layer":layer(&s,"L2")})).unwrap();
            let reference = ordinal(code);
            let action = select(reference.clone());
            roundtrip(&action, &reference, None);
            replay(&mut s, &action);
            assert_eq!(s.active().unwrap().active_layer.unwrap().0, layer(&s, expected));
        }
        for invalid in [first + 7, u32::MAX].into_iter().chain((!background).then_some(0)) {
            let action = select(index("Lyr ", invalid));
            let before = selection(&s);
            assert!(s.execute(&action.steps[0].0, action.steps[0].1.clone()).is_err());
            assert_eq!(selection(&s), before);
        }
    }
}

/// An end marker is a true no-op, including multi-selection, anchor and add/remove modifiers.
#[test]
fn group_end_does_not_replace_the_current_selection() {
    let mut s = scene(true);
    s.execute("layer.select", json!({"layer":layer(&s,"L1")})).unwrap();
    s.execute("layer.select", json!({"layer":layer(&s,"L2"),"mode":"add"})).unwrap();
    for modifier in [None, Some("addToSelection"), Some("removeFromSelection"), Some("addToSelectionContinuous")] {
        let mut d = Descriptor::new("null").with("null", D::Reference(vec![index("Lyr ", 3)]));
        if let Some(modifier) = modifier {
            d = d.with("selectionModifier", D::Enumerated { type_id: Id::new("selectionModifierType"), value: Id::new(modifier) });
        }
        let action = imported(AtnEvent::Code(*b"slct"), d);
        let before = selection(&s);
        replay(&mut s, &action);
        assert_eq!(selection(&s), before);
    }
    for extras in [json!({"name":"L1"}), json!({"occurrence":0}), json!({"mode":"invalid"})] {
        let mut params = json!({"layer":{"index":3}});
        params.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());
        let before = selection(&s);
        assert!(s.execute("layer.select", params).is_err());
        assert_eq!(selection(&s), before);
    }
}

/// Move indexes name pre-move boundaries; the two self boundaries preserve selection/history.
#[test]
fn indexed_moves_follow_the_measured_boundary_table() {
    for target in 0..=6 {
        let mut s = scene(true);
        let reference = index("Lyr ", 2);
        let destination = index("Lyr ", target);
        let action = moved(reference.clone(), destination.clone());
        roundtrip(&action, &reference, Some(&destination));
        let before = selection(&s);
        if target == 0 {
            let doc = s.active().unwrap().doc.clone();
            assert!(s.execute(&action.steps[0].0, action.steps[0].1.clone()).is_err());
            assert_eq!(s.active().unwrap().doc, doc);
            assert_eq!(selection(&s), before);
            continue;
        }
        replay(&mut s, &action);
        let expected = match target {
            1 => vec!["Base", "L2", "L1", "G", "L3"],
            2 | 3 => vec!["Base", "L1", "L2", "G", "L3"],
            4 | 5 => vec!["Base", "L1", "G", "L3"],
            _ => vec!["Base", "L1", "G", "L2", "L3"],
        };
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), expected, "target {target}");
        let children = doc.layer(photocraft_doc::LayerId(layer(&s, "G"))).unwrap().children().unwrap();
        assert_eq!(
            children.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            match target {
                4 => vec!["L2", "Gchild"],
                5 => vec!["Gchild", "L2"],
                _ => vec!["Gchild"],
            }
        );
        if matches!(target, 2 | 3) {
            assert_eq!(selection(&s), before);
        }
    }
    let mut s = scene(false);
    let action = moved(index("Lyr ", 3), index("Lyr ", 0));
    replay(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.layers[0].name, "L2");
    let mut s = scene(true);
    for invalid in [7, u32::MAX] {
        let action = moved(index("Lyr ", 2), index("Lyr ", invalid));
        let doc = s.active().unwrap().doc.clone();
        let state = selection(&s);
        assert!(s.execute(&action.steps[0].0, action.steps[0].1.clone()).is_err());
        assert_eq!(s.active().unwrap().doc, doc);
        assert_eq!(selection(&s), state);
    }
}

/// A target-layer move keeps the complete selected source set and remains one undoable edit.
#[test]
fn indexed_move_preserves_multiple_selected_sources() {
    let mut s = scene(true);
    s.execute("layer.select", json!({"layer":layer(&s,"L1")})).unwrap();
    s.execute("layer.select", json!({"layer":layer(&s,"L2"),"mode":"add"})).unwrap();
    let before = selection(&s);
    let action = moved(ordinal("Trgt"), index("Lyr ", 6));
    roundtrip(&action, &ordinal("Trgt"), Some(&index("Lyr ", 6)));
    replay(&mut s, &action);
    let after = selection(&s);
    for key in ["ids", "active", "anchor"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(s.active().unwrap().doc.layers.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["Base", "G", "L1", "L2", "L3"]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["Base", "L1", "L2", "G", "L3"]);
}

/// Nested empty groups have distinct end/header boundaries, and cycle refusals are atomic.
#[test]
fn nested_empty_group_boundaries_reparent_at_the_correct_level() {
    for target in [1, 2, 3] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
        s.execute("layer.new.group", json!({"name":"Inner"})).unwrap();
        s.execute("layer.groupLayers", json!({"name":"Outer"})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Top"})).unwrap();
        let outer = photocraft_doc::LayerId(layer(&s, "Outer"));
        let inner = photocraft_doc::LayerId(layer(&s, "Inner"));
        let action = moved(index("Lyr ", 5), index("Lyr ", target));
        replay(&mut s, &action);
        let doc = &s.active().unwrap().doc;
        let parent = doc.path_of(photocraft_doc::LayerId(layer(&s, "Top"))).unwrap();
        assert_eq!(
            parent.len(),
            match target {
                1 => 1,
                2 => 2,
                _ => 3,
            }
        );
        assert_eq!(doc.layer(outer).unwrap().children().unwrap().len(), if target == 2 { 2 } else { 1 });
        assert_eq!(doc.layer(inner).unwrap().children().unwrap().len(), usize::from(target == 3));
        let action = moved(index("Lyr ", 5), index("Lyr ", 4));
        let before = doc.clone();
        assert!(s.execute(&action.steps[0].0, action.steps[0].1.clone()).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

/// Typed document indexes use opening order while numeric document indexes still address tabs.
#[test]
fn document_indexes_survive_tab_moves_close_and_reopen_with_old_ids() {
    let mut s = Session::new();
    for name in ["A", "B", "C"] {
        s.execute("file.new", json!({"width":2,"height":2,"name":name})).unwrap();
    }
    let reopened = (*s.documents()[0].doc).clone();
    s.execute("document.move", json!({"document":2,"to":0})).unwrap();
    s.execute("document.activate", json!({"document":0})).unwrap();
    assert_eq!(s.active().unwrap().doc.name, "C");
    let reference = index("Dcmn", 2);
    let action = select(reference.clone());
    roundtrip(&action, &reference, None);
    replay(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.name, "B");
    s.execute("file.close", json!({"document":1})).unwrap();
    s.add_document(reopened, None);
    for (n, expected) in [(1, "B"), (2, "C"), (3, "A")] {
        replay(&mut s, &select(index("Dcmn", n)));
        assert_eq!(s.active().unwrap().doc.name, expected);
    }
    for invalid in [0, u32::MAX, 4] {
        let before = s.active_index();
        let action = select(index("Dcmn", invalid));
        assert!(s.execute(&action.steps[0].0, action.steps[0].1.clone()).is_err());
        assert_eq!(s.active_index(), before);
    }
}

fn comp_scene() -> (Session, u32, u32) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":2,"height":2})).unwrap();
    let dead = s.execute("layerComp.new", json!({"name":"Deleted"})).unwrap()["comp"].as_u64().unwrap();
    s.execute("layer.hideLayers", json!({})).unwrap();
    let first = s.execute("layerComp.new", json!({"name":"First"})).unwrap()["comp"].as_u64().unwrap() as u32;
    s.execute("layer.showLayers", json!({})).unwrap();
    let second = s.execute("layerComp.new", json!({"name":"Second"})).unwrap()["comp"].as_u64().unwrap() as u32;
    s.execute("layerComp.delete", json!({"comp":dead})).unwrap();
    (s, first, second)
}

/// Comp ids, panel indexes and unusual measured ordinals stay distinct after reordering.
#[test]
fn comp_forms_keep_their_wire_kind_and_resolve_at_playback() {
    let (mut s, first, second) = comp_scene();
    for reordered in [false, true] {
        if reordered {
            s.execute("layerComp.reorder", json!({"comp":second,"index":0})).unwrap();
        }
        let panel_first = if reordered { second } else { first };
        let panel_second = if reordered { first } else { second };
        let current = panel_second;
        let mut cases = vec![
            (index("compsClass", 1), panel_first),
            (index("compsClass", 2), panel_second),
            (ReferenceItem::Identifier { class: class("compsClass"), id: first }, first),
        ];
        for (code, expected) in [("Trgt", current), ("Nxt ", current), ("Prvs", current), ("Frst", panel_first), ("Lst ", panel_first), ("Al  ", panel_first)] {
            cases.push((
                ReferenceItem::Enumerated { class: class("compsClass"), type_id: Id::new("Ordn"), value: Id::Code(code.as_bytes().try_into().unwrap()) },
                expected,
            ));
        }
        for (reference, expected) in cases {
            s.execute("layerComp.apply", json!({"comp":current})).unwrap();
            s.execute(if current == second { "layer.hideLayers" } else { "layer.showLayers" }, json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.last_applied_comp, Some(current));
            let action = imported(AtnEvent::name("applyComp"), Descriptor::new("null").with("null", D::Reference(vec![reference.clone()])));
            roundtrip(&action, &reference, None);
            replay(&mut s, &action);
            assert_eq!(s.active().unwrap().doc.last_applied_comp, Some(expected));
            assert_eq!(s.active().unwrap().doc.layers[0].visible, expected == second);
        }
    }
}

/// Invalid comp indexes succeed without edits; unmeasured layer/comp reference forms stay kept.
#[test]
fn invalid_comp_indexes_and_unsupported_references_preserve_state() {
    let (mut s, _, _) = comp_scene();
    for invalid in [0, 3, u32::MAX] {
        let reference = index("compsClass", invalid);
        let action = imported(AtnEvent::name("applyComp"), Descriptor::new("null").with("null", D::Reference(vec![reference.clone()])));
        roundtrip(&action, &reference, None);
        let before = s.active().unwrap().doc.clone();
        let state = selection(&s);
        replay(&mut s, &action);
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(selection(&s), state);
    }
    let before = s.active().unwrap().doc.clone();
    s.execute("layerComp.apply", json!({"comp":{"index":-1}})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.execute("layerComp.apply", json!({"comp":{"id":u32::MAX}})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    let negative = Action::new("Select", vec![("layerComp.apply".into(), json!({"comp":{"index":-1}}))]);
    let (saved, _) = atn_cmds::export("References", &[&negative], &[], false).unwrap();
    assert_eq!(
        atn::parse(&saved).unwrap().actions[0].items[0].descriptor.as_ref().unwrap().get("null"),
        Some(&D::Reference(vec![index("compsClass", u32::MAX)]))
    );
    for (event, reference) in [
        (AtnEvent::Code(*b"slct"), ReferenceItem::Identifier { class: class("Lyr "), id: 2 }),
        (AtnEvent::Code(*b"slct"), ReferenceItem::Offset { class: class("Lyr "), offset: 1 }),
        (AtnEvent::name("applyComp"), ReferenceItem::Offset { class: class("compsClass"), offset: 0 }),
        (AtnEvent::name("applyComp"), ReferenceItem::Enumerated { class: class("compsClass"), type_id: Id::new("wrong"), value: Id::new("Trgt") }),
        (AtnEvent::Code(*b"slct"), ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("wrong"), value: Id::new("Frwr") }),
    ] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![reference]));
        let data = bytes(event, d.clone());
        let result = atn_cmds::import(&data).unwrap();
        assert_eq!(result.unsupported.len(), 1);
        assert_eq!(result.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let (saved, _) = atn_cmds::export("References", &[&result.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0].descriptor, Some(d));
    }
    for offset in [-1, 0, 1] {
        let d = Descriptor::new("null")
            .with("null", D::Reference(vec![ordinal("Trgt")]))
            .with("T   ", D::Reference(vec![ReferenceItem::Offset { class: class("Lyr "), offset }]));
        assert_eq!(atn_cmds::import(&bytes(AtnEvent::Code(*b"move"), d)).unwrap().unsupported.len(), 1);
    }
}

/// Reject integers outside the ATN reference format before they can enter a recording.
#[test]
fn unrepresentable_comp_indexes_are_rejected_before_recording() {
    let (mut s, _, _) = comp_scene();
    for valid in [json!(i32::MIN), json!(u32::MAX)] {
        s.execute("layerComp.apply", json!({"comp":{"index":valid}})).unwrap();
    }
    let before = s.active().unwrap().doc.clone();
    let state = selection(&s);
    let history = s.active().unwrap().history.past_len();
    s.execute("actions.record", json!({"name":"Index bounds"})).unwrap();
    let mut accepted = Vec::new();
    for invalid in [json!(u64::from(u32::MAX) + 1), json!(u64::MAX), json!(i64::from(i32::MIN) - 1), json!(i64::MIN)] {
        if s.execute("layerComp.apply", json!({"comp":{"index":invalid}})).is_ok() {
            accepted.push(invalid);
        }
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(selection(&s), state);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    s.execute("actions.stop", json!({})).unwrap();
    assert!(accepted.is_empty(), "unrepresentable indexes were accepted: {accepted:?}");
    assert!(s.actions.list.last().unwrap().steps.is_empty());
}

/// Valid extreme indexes survive recording and export as ATN Index references.
#[test]
fn accepted_comp_boundary_indexes_record_and_export() {
    let (mut s, _, _) = comp_scene();
    s.execute("actions.record", json!({"name":"Boundary indexes"})).unwrap();
    for valid in [json!(i32::MIN), json!(u32::MAX)] {
        s.execute("layerComp.apply", json!({"comp":{"index":valid}})).unwrap();
    }
    s.execute("actions.stop", json!({})).unwrap();
    let action = s.actions.list.last().unwrap();
    assert_eq!(action.steps.len(), 2);
    let (saved, skipped) = atn_cmds::export("References", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let parsed = atn::parse(&saved).unwrap();
    assert_eq!(parsed.actions[0].items.len(), 2);
    for (item, expected) in parsed.actions[0].items.iter().zip([i32::MIN as u32, u32::MAX]) {
        assert_eq!(item.descriptor.as_ref().unwrap().get("null"), Some(&D::Reference(vec![index("compsClass", expected)])));
    }
}

/// A recorded invalid index is a no-op even with zero comps; ordinary Apply stays disabled.
#[test]
fn empty_comp_list_index_is_noop() {
    let mut s = Session::new();
    assert!(!s.is_enabled_with("layerComp.apply", &json!({"comp":{"index":0}})));
    s.execute("file.new", json!({"width":2,"height":2})).unwrap();
    assert!(s.active().unwrap().doc.layer_comps.is_empty());
    assert!(!s.is_enabled("layerComp.apply"));
    for invalid in [0, 1, u32::MAX] {
        let reference = index("compsClass", invalid);
        let action = imported(AtnEvent::name("applyComp"), Descriptor::new("null").with("null", D::Reference(vec![reference.clone()])));
        roundtrip(&action, &reference, None);
        let before = s.active().unwrap().doc.clone();
        let state = selection(&s);
        replay(&mut s, &action);
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(selection(&s), state);
    }
    s.execute("layerComp.apply", json!({"comp":{"index":-1}})).unwrap();
    assert!(!s.is_enabled("layerComp.apply"));
    for params in [json!({}), json!({"comp":{"id":1}}), json!({"comp":{"index":false}})] {
        assert!(s.execute("layerComp.apply", params).is_err());
    }
}

/// A group-end index names no layer, so a command acting on a layer fails and changes nothing
/// rather than falling back to the selected layers.
#[test]
fn group_end_index_never_targets_the_selection() {
    let mut s = scene(true);
    s.execute("layer.select", json!({"layer":layer(&s,"L1")})).unwrap();
    s.execute("layer.select", json!({"layer":layer(&s,"L2"),"mode":"add"})).unwrap();
    let past = s.active().unwrap().history.past_len();
    let doc = s.active().unwrap().doc.clone();
    assert!(s.execute("layer.translate", json!({"layer":{"index":3},"dx":1,"dy":0})).is_err());
    let offset = Descriptor::new("Ofst").with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: 1.0 }).with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: 0.0 });
    let action =
        imported(AtnEvent::Code(*b"move"), Descriptor::new("null").with("null", D::Reference(vec![index("Lyr ", 3)])).with("T   ", D::Descriptor(offset)));
    s.actions.list = vec![action];
    let result = s.execute("actions.play", json!({"action":"Select"})).unwrap();
    assert!(result.get("failed").is_some(), "{result}");
    assert_eq!(s.active().unwrap().history.past_len(), past);
    assert_eq!(s.active().unwrap().doc, doc);
}

/// A relative ordinal is resolved once: the precondition checked on the named layer sees that
/// layer, not the one past it (here a type layer that would refuse a colour-channel edit).
#[test]
fn forward_ordinal_precondition_checks_the_resolved_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"transparent"})).unwrap();
    s.execute("layer.renameLayer", json!({"name":"A"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"B"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#000000"})).unwrap();
    s.execute("type.create", json!({"text":"C","size":2})).unwrap();
    s.execute("layer.renameLayer", json!({"name":"C"})).unwrap();
    s.execute("layer.select", json!({"layer":layer(&s,"A")})).unwrap();
    s.execute("channel.target", json!({"colors":[0]})).unwrap();
    let b = photocraft_doc::LayerId(layer(&s, "B"));
    let before = s.active().unwrap().doc.layer(b).cloned();
    s.execute("edit.clear", json!({"layer":{"ordinal":"Frwr"}})).unwrap();
    assert_ne!(s.active().unwrap().doc.layer(b).cloned(), before);
}
