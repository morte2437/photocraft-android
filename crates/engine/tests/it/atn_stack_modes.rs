//! The measured Smart Object Stack Mode events retain their descriptor and native behavior.

use photocraft_doc::{Group, Layer, LayerContent, StackMode};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_geom::Rect;
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, UnicodeString, Value as D},
};
use serde_json::json;

const APPLY: &str = "applyImageStackPluginRenderer";
const CLEAR: &str = "imageStackConvertSmartObject";
const ORIGIN: &str = "_atnStackDescriptor";

/// Independent pairs from the measured table in docs/atn.md.
const MODES: &[(&str, &str, StackMode)] = &[
    ("rang", "Range", StackMode::Range),
    ("entr", "Entropy", StackMode::Entropy),
    ("kurt", "Kurtosis", StackMode::Kurtosis),
    ("maxx", "Maximum", StackMode::Maximum),
    ("medn", "Median", StackMode::Median),
    ("minn", "Minimum", StackMode::Minimum),
    ("avrg", "Mean", StackMode::Mean),
    ("skew", "Skewness", StackMode::Skewness),
    ("stdv", "Standard Deviation", StackMode::StandardDeviation),
    ("summ", "Summation", StackMode::Summation),
    ("vari", "Variance", StackMode::Variance),
];

fn descriptor(code: &str, name: &str) -> Descriptor {
    Descriptor::new("null")
        .with("imageStackPlugin", D::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new(code) }))
        .with("Nm  ", D::Text(UnicodeString::new_nul(name)))
}

fn item(event: &str, descriptor: Option<Descriptor>) -> AtnItem {
    AtnItem { event: AtnEvent::name(event), descriptor, ..Default::default() }
}

fn load(item: AtnItem) -> atn_cmds::Imported {
    let file = AtnFile {
        name: UnicodeString::new_nul("Stacks"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Mode"), items: vec![item], ..Default::default() }],
        ..Default::default()
    };
    atn_cmds::import(&atn::write(&file).unwrap()).unwrap()
}

fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Stacks", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions.remove(0).items.remove(0)
}

/// Exercise actual load/save bytes for all plugin classes and the separate None event.
#[test]
fn measured_stack_modes_and_none_import_export_and_reload() {
    for &(code, name, mode) in MODES {
        let original = item(APPLY, Some(descriptor(code, name)));
        let imported = load(original.clone());
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps, vec![(format!("layer.smartObjects.stackMode.{}", mode.id()), json!({}))]);
        let out = saved(&imported.actions[0]);
        assert_eq!(out.event, original.event);
        assert_eq!(out.descriptor, original.descriptor);
        assert_eq!(load(out).actions[0].steps, imported.actions[0].steps);
    }
    for original in [item(CLEAR, None), item(CLEAR, Some(Descriptor::new("null")))] {
        let imported = load(original.clone());
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].0, "layer.smartObjects.stackMode.none");
        let out = saved(&imported.actions[0]);
        assert_eq!(out.event, original.event);
        assert_eq!(out.descriptor, original.descriptor);
        assert_eq!(load(out).actions[0].steps, imported.actions[0].steps);
    }
    let native = Action::new("Mode", vec![("layer.smartObjects.stackMode.none".into(), json!({}))]);
    let out = saved(&native);
    assert_eq!(out.event, AtnEvent::name(CLEAR));
    assert!(out.descriptor.is_none());
}

/// Localized names and noncanonical wire metadata still replay the class-selected mode.
#[test]
fn localized_stack_descriptors_keep_original_metadata_and_flags() {
    for name in [UnicodeString::new("Moyenne"), UnicodeString::new_nul("平均值"), UnicodeString(vec![0xd800, 0, 0])] {
        let mut d = descriptor("avrg", "Mean");
        d.name = UnicodeString::new_nul("Recorded stack");
        d.class_id = Id::Str(b"null".to_vec());
        d.items[1].1 = D::Text(name);
        d.items[1].0 = Id::Str(b"Nm  ".to_vec());
        d.items.reverse();
        let mut original = item(APPLY, Some(d));
        original.enabled = false;
        original.with_dialog = true;
        original.dialog_options = 2;
        let imported = load(original.clone());
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].0, "layer.smartObjects.stackMode.mean");
        let out = saved(&imported.actions[0]);
        assert_eq!(out.descriptor, original.descriptor);
        assert!(!out.enabled);
        assert!(out.with_dialog);
        assert_eq!(out.dialog_options, 2);
    }
    let mut d = Descriptor::new("null");
    d.name = UnicodeString(vec![0xd800, 0]);
    d.class_id = Id::Str(b"null".to_vec());
    let imported = load(item(CLEAR, Some(d.clone())));
    assert!(imported.unsupported.is_empty());
    assert_eq!(saved(&imported.actions[0]).descriptor, Some(d));
}

fn stack(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":3,"height":2,"depth":depth})).unwrap();
    s.edit("Fixture", |doc, active| {
        let fmt = doc.pixel_format();
        let children = [0.2, 0.2, 0.8]
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                let mut layer = Layer::raster(format!("Frame {index}"), fmt);
                let pixel = photocraft_raster::from_rgba(&fmt, [value, value, value, 1.0]);
                layer.surface_mut().unwrap().write_region(Rect::new(0, 0, 3, 2), &pixel.repeat(6));
                layer
            })
            .collect();
        let group = Layer::new("Stack", LayerContent::Group(Group { children, expanded: true, artboard: None, frame_stroke: None }));
        let smart = photocraft_engine::smart_cmds::layer_to_smart(doc, &group)?;
        *active = Some(smart.id);
        doc.layers = vec![smart];
        Ok(())
    })
    .unwrap();
    s
}

/// Compare the adapter with existing native commands, including pixel output and one Undo/Redo.
#[test]
fn stack_recordings_play_native_modes_and_undo_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = stack(depth);
        let modes = MODES.iter().map(|&(code, _, mode)| (item(APPLY, Some(descriptor(code, "Recorded name"))), Some(mode)));
        let clear_forms = [None, Some(Descriptor::new("null"))].into_iter().map(|d| (item(CLEAR, d), None));
        for (source, mode) in modes.chain(clear_forms) {
            let starting = if mode == Some(StackMode::Maximum) { "none" } else { "maximum" };
            s.execute(&format!("layer.smartObjects.stackMode.{starting}"), json!({})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let before_steps = s.active().unwrap().history.past_len();
            let command = format!("layer.smartObjects.stackMode.{}", mode.map_or("none", StackMode::id));
            s.execute(&command, json!({})).unwrap();
            let expected = s.active().unwrap().doc.clone();
            s.execute("edit.undo", json!({})).unwrap();
            let imported = load(source);
            s.actions.list = imported.actions;
            let result = s.execute("actions.play", json!({"action":"Mode","set":"Stacks"})).unwrap();
            assert_eq!(result["ran"], 1, "{result}");
            let actual = s.active().unwrap();
            assert_eq!(actual.doc.layers, expected.layers, "{command} at {depth}-bit");
            assert_eq!(actual.history.past_len(), before_steps + 1);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers, expected.layers);
        }
    }
}

/// Unrepresented types, duplicate keys and extra settings stay complete kept steps.
#[test]
fn unsupported_stack_recordings_save_whole_and_refuse_playback() {
    let canonical = descriptor("medn", "Median");
    let mut duplicate = canonical.clone();
    duplicate.items.push(duplicate.items[0].clone());
    let mut wrong_name = canonical.clone();
    wrong_name.items[1].1 = D::Integer(5);
    let mut global_class = canonical.clone();
    let D::Class(class) = global_class.items[0].1.clone() else { unreachable!() };
    global_class.items[0].1 = D::GlobalClass(class);
    let extra = canonical.clone().with("unknown", D::Boolean(true));
    for original in [
        item(APPLY, Some(duplicate)),
        item(APPLY, Some(wrong_name)),
        item(APPLY, Some(global_class)),
        item(APPLY, Some(extra)),
        item(APPLY, Some(descriptor("nope", "Median"))),
        item(APPLY, None),
        item(CLEAR, Some(canonical)),
        item(CLEAR, Some(Descriptor::new("nope"))),
    ] {
        let imported = load(original.clone());
        assert_eq!(imported.unsupported.len(), 1);
        assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let out = saved(&imported.actions[0]);
        assert_eq!(out.event, original.event);
        assert_eq!(out.descriptor, original.descriptor);
        let mut s = stack(8);
        s.execute("layer.smartObjects.stackMode.maximum", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        s.actions.list = imported.actions;
        let result = s.execute("actions.play", json!({"action":"Mode","set":"Stacks"})).unwrap();
        assert!(result.get("failed").is_some(), "{result}");
        assert_eq!(s.active().unwrap().doc.layers, before.layers);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

/// Native mode edits replace the old class; explicit targets and damaged origin metadata refuse export.
#[test]
fn stack_mode_edits_and_unrepresented_native_parameters_are_safe() {
    let imported = load(item(APPLY, Some(descriptor("medn", "Médiane"))));
    let mut action = imported.actions[0].clone();
    action.steps[0].0 = "layer.smartObjects.stackMode.maximum".into();
    assert_eq!(saved(&action).descriptor, Some(descriptor("maxx", "Maximum")));

    action.steps[0].0 = "layer.smartObjects.stackMode.none".into();
    let cleared = saved(&action);
    assert_eq!(cleared.event, AtnEvent::name(CLEAR));
    assert_eq!(cleared.descriptor, Some(Descriptor::new("null")));

    let mut action = load(item(CLEAR, Some(Descriptor::new("null")))).actions.remove(0);
    action.steps[0].0 = "layer.smartObjects.stackMode.mean".into();
    let applied = saved(&action);
    assert_eq!(applied.event, AtnEvent::name(APPLY));
    assert_eq!(applied.descriptor, Some(descriptor("avrg", "Mean")));

    let unknown_empty = photocraft_engine::paint::tile::b64_encode(&Descriptor::new("nope").to_bytes());
    let unknown_class = photocraft_engine::paint::tile::b64_encode(&descriptor("nope", "Mean").to_bytes());
    for command in ["layer.smartObjects.stackMode.mean", "layer.smartObjects.stackMode.none"] {
        for params in [json!(null), json!([]), json!({"layer":12}), json!({"layer":"Named"}), json!({"target":"mask"}), json!({"unknown":true})] {
            let action = Action::new("Mode", vec![(command.into(), params)]);
            assert!(atn_cmds::export("Stacks", &[&action], &[], false).is_err());
        }
        for origin in [json!(false), json!("!invalid-base64"), json!(""), json!(unknown_class), json!(unknown_empty)] {
            let mut action = Action::new("Mode", vec![(command.into(), json!({}))]);
            action.steps[0].1[ORIGIN] = origin;
            assert!(atn_cmds::export("Stacks", &[&action], &[], false).is_err());
        }
    }
}
