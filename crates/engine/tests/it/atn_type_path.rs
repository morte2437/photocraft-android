//! Recorded Type > Create Work Path preserves its descriptor and uses native glyph outlines.
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds::{export, import},
};
use serde_json::{Value, json};

const COMMAND: &str = "type.createWorkPath";
const SOURCE: &str = "_atnTypePathDescriptor";

fn class(id: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(id) }
}

/// Build the measured descriptor independently of the production converter.
fn descriptor() -> Descriptor {
    Descriptor::new("null")
        .with("null", D::Reference(vec![ReferenceItem::Class(class("Path"))]))
        .with("From", D::Reference(vec![ReferenceItem::Enumerated { class: class("TxLr"), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]))
}

fn item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::Code(*b"Mk  "), dictionary: b"Make".to_vec(), descriptor: Some(d), ..Default::default() }
}

/// Exercise the public byte importer rather than invoking the converter directly.
fn load(item: AtnItem) -> photocraft_engine::atn_cmds::Imported {
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Paths"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Path"), items: vec![item], ..Default::default() }],
        ..Default::default()
    })
    .unwrap();
    import(&bytes).unwrap()
}

/// Parse public export bytes so assertions inspect the actual saved ATN item.
fn saved(action: &Action) -> AtnItem {
    let bytes = export("Paths", &[action], &[], false).unwrap().0;
    atn::parse(&bytes).unwrap().actions.remove(0).items.remove(0)
}

/// Names, key order, ID encoding and dialog flags must survive both Make event aliases.
#[test]
fn descriptor_identity_flags_and_make_alias_roundtrip() {
    for named in [false, true] {
        let mut d = descriptor();
        d.name = UnicodeString::new_nul("Recorded path");
        d.class_id = Id::Str(b"null".to_vec());
        d.items.reverse();
        let mut source = item(d);
        if named {
            source.event = AtnEvent::name("make");
        }
        source.enabled = false;
        source.with_dialog = true;
        source.dialog_options = 2;
        let loaded = load(source.clone());
        assert!(loaded.unsupported.is_empty());
        assert_eq!(loaded.actions[0].steps[0].0, COMMAND);
        let result = saved(&loaded.actions[0]);
        assert_eq!(result.descriptor.unwrap().to_bytes(), source.descriptor.unwrap().to_bytes());
        assert!(!result.enabled);
        assert!(result.with_dialog);
        assert_eq!(result.dialog_options, 2);
    }
}

/// Compare enabled canonical and retained-descriptor playback with native output at all depths.
/// Glyph bounds distinguish the older active text layer from the newest layer independently.
#[test]
fn playback_matches_native_path_and_preserves_text_with_undo_redo() {
    for noncanonical in [false, true] {
        for depth in [8, 16, 32] {
            let mut recorded = Session::new();
            recorded.execute("file.new", json!({"width":160,"height":100,"depth":depth})).unwrap();
            let text = recorded.execute("type.create", json!({"x":90,"y":80,"text":"ill","size":30})).unwrap();
            let text = LayerId(text["layer"].as_u64().unwrap());
            let other = recorded.execute("type.create", json!({"x":8,"y":35,"text":"o","size":24})).unwrap();
            let other = LayerId(other["layer"].as_u64().unwrap());
            assert_eq!(recorded.active().unwrap().active_layer, Some(other));
            recorded.select_layer(text).unwrap();
            let before = recorded.active().unwrap().doc.clone();
            let glyphs = before.layer(text).unwrap().surface().unwrap().content_bounds();
            let other_glyphs = before.layer(other).unwrap().surface().unwrap().content_bounds();
            let past = recorded.active().unwrap().history.past_len();
            // Obtain the native result on this very document, then restore the starting state.
            recorded.execute(COMMAND, json!({})).unwrap();
            let native = recorded.active().unwrap().doc.clone();
            assert!(!native.work_path.as_ref().unwrap().subpaths.is_empty());
            assert!(recorded.undo());
            assert_eq!(recorded.active().unwrap().doc.as_ref(), before.as_ref());
            let mut d = descriptor();
            if noncanonical {
                d.items.reverse();
                d.name = UnicodeString::new_nul("Recorded path");
            }
            recorded.actions.list = load(item(d)).actions;
            if noncanonical {
                assert!(recorded.actions.list[0].steps[0].1.get(SOURCE).is_some());
            }
            let result = recorded.execute("actions.play", json!({"action":0})).unwrap();
            assert_eq!(result["ran"], 1);
            assert!(result.get("failed").is_none(), "{result}");
            let after = recorded.active().unwrap().doc.clone();
            assert_eq!(after.as_ref(), native.as_ref());
            // Rendered glyph bounds are an independent oracle for the implicit active target.
            let (x0, y0, x1, y1) = after.work_path.as_ref().unwrap().control_bounds().unwrap();
            for (path, ink) in [(x0, glyphs.x0), (y0, glyphs.y0), (x1, glyphs.x1), (y1, glyphs.y1)] {
                assert!((path - f64::from(ink)).abs() < 3.0, "path {path}, active glyph bound {ink}");
            }
            assert!(x0 > f64::from(other_glyphs.x1), "the other layer's glyphs must not supply the work path");
            assert!(matches!(after.layer(text).unwrap().content, LayerContent::Text(_)));
            assert_eq!(recorded.active().unwrap().history.past_len(), past + 1);
            assert!(recorded.undo());
            assert_eq!(recorded.active().unwrap().doc.as_ref(), before.as_ref());
            assert!(recorded.redo());
            assert_eq!(recorded.active().unwrap().doc.as_ref(), after.as_ref());
        }
    }
}

/// Unknown keys and unsupported reference chains remain verbatim kept steps, never guessed mappings.
#[test]
fn unsupported_path_forms_are_kept_and_save_without_loss() {
    let original = descriptor();
    let mut malformed = vec![
        original.clone().with("extra", D::Boolean(true)),
        original.clone().with("From", D::Boolean(false)),
        Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Class(class("Path"))])),
    ];
    for from in [
        D::Reference(vec![ReferenceItem::Name { class: class("TxLr"), name: UnicodeString::new_nul("Text") }]),
        D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]),
        D::Reference(vec![ReferenceItem::Enumerated { class: class("TxLr"), type_id: Id::new("Ordn"), value: Id::new("Al  ") }]),
        D::Reference(vec![ReferenceItem::Class(class("TxLr"))]),
    ] {
        let mut d = original.clone();
        d.items.iter_mut().find(|(k, _)| k.is("From")).unwrap().1 = from;
        malformed.push(d);
    }
    for d in malformed {
        let source = item(d);
        let loaded = load(source.clone());
        assert_eq!(loaded.unsupported.len(), 1);
        assert_eq!(loaded.actions[0].steps[0].0, "actions.photoshopStep");
        assert_eq!(saved(&loaded.actions[0]), source);
    }
}

/// A missing document or non-text receiver must fail without changing document or history.
#[test]
fn non_text_and_no_document_playback_fail_without_edits() {
    for document in [false, true] {
        let mut s = Session::new();
        if document {
            s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        }
        let before = s.active().map(|st| (st.doc.clone(), st.history.past_len()));
        s.actions.list = load(item(descriptor())).actions;
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result.get("failed").is_some(), "{result}");
        assert_eq!(s.active().map(|st| (st.doc.clone(), st.history.past_len())), before);
    }
}

/// Explicit native targets and corrupt wire metadata cannot export as the implicit measured form.
#[test]
fn native_export_refuses_explicit_targets_and_damaged_metadata() {
    for params in [json!({"layer":1}), json!({"layers":[1]}), json!({(SOURCE):"bad base64"}), json!({(SOURCE):Value::Null})] {
        let action = Action::new("Path", vec![(COMMAND.into(), params)]);
        assert!(export("Paths", &[&action], &[], false).is_err());
    }
    let action = Action::new("Path", vec![(COMMAND.into(), json!({}))]);
    assert_eq!(saved(&action).descriptor.unwrap(), descriptor());
}
