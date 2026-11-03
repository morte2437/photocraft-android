//! Recorded CAF settings use the native operation; unsupported descriptors stay whole.
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds::{export, import},
};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use serde_json::{Value, json};

const CMD: &str = "edit.contentAwareFill";

fn enm(key: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(key), value: Id::new(value) }
}

fn descriptor() -> Descriptor {
    Descriptor::new("null")
        .with("cafSamplingRegion", enm("cafSamplingRegion", "cafSamplingRegionRectangular"))
        .with("cafSampleAllLayers", D::Boolean(false))
        .with("cafColorAdaptationLevel", enm("cafColorAdaptationLevel", "cafColorAdaptationDefault"))
        .with("cafRotationAmount", enm("cafRotationAmount", "cafRotationAmountNone"))
        .with("cafScale", D::Boolean(false))
        .with("cafMirror", D::Boolean(false))
        .with("cafOutput", enm("cafOutput", "cafOutputToNewLayer"))
}

fn params() -> Value {
    json!({"sampling":"rectangular","sampleAllLayers":false,"colorAdaptation":"default",
        "rotationAdaptation":"none","scale":false,"mirror":false,"output":"new"})
}

fn item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::name("cafWorkspace"), dictionary: b"Content-Aware Fill".to_vec(), descriptor: Some(d), ..Default::default() }
}

fn file(item: AtnItem) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item], ..Default::default() }],
        ..Default::default()
    }
}

fn load(d: Descriptor) -> Action {
    let loaded = import(&atn::write(&file(item(d))).unwrap()).unwrap();
    assert!(loaded.unsupported.is_empty());
    loaded.actions.into_iter().next().unwrap()
}

fn exported(p: Value) -> Option<AtnItem> {
    let a = Action::new("A", vec![(CMD.into(), p)]);
    let bytes = export("S", &[&a], &[], false).ok()?.0;
    atn::parse(&bytes).ok()?.actions.into_iter().next()?.items.into_iter().next()
}

/// Each field retains its native meaning, recorded type and position, including false booleans.
#[test]
fn caf_settings_roundtrip_seven_fields_and_boolean_choices() {
    let base = descriptor();
    let mut cases = Vec::new();
    for bits in 0..8 {
        let mut d = base.clone();
        let mut p = params();
        for (bit, index, param) in [(0, 1, "sampleAllLayers"), (1, 4, "scale"), (2, 5, "mirror")] {
            let on = bits & (1 << bit) != 0;
            d.items[index].1 = D::Boolean(on);
            p[param] = json!(on);
        }
        cases.push((d, p));
    }
    let mut current = base.clone();
    current.items[6].1 = enm("cafOutput", "cafOutputToCurrentLayer");
    let mut p = params();
    p["output"] = json!("current");
    cases.push((current, p));
    for (d, p) in cases {
        let action = load(d.clone());
        assert_eq!(action.steps, vec![(CMD.into(), p.clone())]);
        assert_eq!(exported(p).unwrap(), item(d.clone()));
        let bytes = export("S", &[&action], &[], false).unwrap().0;
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], item(d));
    }
}

/// A noncanonical source order is preservation data, and cannot overwrite an edited option.
#[test]
fn caf_source_order_and_flags_survive_without_reviving_stale_settings() {
    let mut d = descriptor();
    d.items.reverse();
    let mut source = item(d);
    source.enabled = false;
    source.with_dialog = true;
    source.dialog_options = 2;
    let imported = import(&atn::write(&file(source.clone())).unwrap()).unwrap();
    assert!(imported.unsupported.is_empty());
    let a = &imported.actions[0];
    assert!(a.steps[0].1.get("_atnCafDescriptor").is_some());
    let bytes = export("S", &[a], &[], false).unwrap().0;
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], source);
    let mut edited = a.steps[0].1.clone();
    edited["mirror"] = json!(true);
    assert!(exported(edited).is_none());
    let mut damaged = a.steps[0].1.clone();
    damaged["_atnCafDescriptor"] = json!("not base64");
    assert!(exported(damaged).is_none());
}

/// Missing defaults, malformed/repeated fields and unrepresented workspace state stay complete.
#[test]
fn caf_unrepresented_descriptors_are_kept_and_never_partially_play() {
    let base = descriptor();
    let mut cases = vec![Descriptor::new("null"), base.clone().with("extra", D::Boolean(true))];
    for i in 0..base.items.len() {
        let mut missing = base.clone();
        missing.items.remove(i);
        cases.push(missing);
        let mut repeated = base.clone();
        repeated.items.push(repeated.items[i].clone());
        cases.push(repeated);
        let mut wrong = base.clone();
        wrong.items[i].1 = D::Integer(0);
        cases.push(wrong);
    }
    for (i, key, value) in [
        (0, "cafSamplingRegion", "cafSamplingRegionAuto"),
        (0, "cafSamplingRegion", "cafSamplingRegionCustom"),
        (2, "cafColorAdaptationLevel", "cafColorAdaptationNone"),
        (2, "cafColorAdaptationLevel", "cafColorAdaptationHigh"),
        (2, "cafColorAdaptationLevel", "cafColorAdaptationVeryHigh"),
        (2, "cafColorAdaptationLevel", "unknown"),
        (3, "cafRotationAmount", "cafRotationAmountLow"),
        (3, "cafRotationAmount", "cafRotationAmountMedium"),
        (3, "cafRotationAmount", "cafRotationAmountHigh"),
        (3, "cafRotationAmount", "cafRotationAmountFull"),
        (3, "cafRotationAmount", "unknown"),
        (6, "cafOutput", "cafOutputToDuplicateLayer"),
    ] {
        let mut d = base.clone();
        d.items[i].1 = enm(key, value);
        cases.push(d);
    }
    let mut wrong_class = base.clone();
    wrong_class.items[0].1 = enm("unrelatedEnum", "cafSamplingRegionRectangular");
    cases.push(wrong_class);
    for d in cases {
        let source = item(d);
        let imported = import(&atn::write(&file(source.clone())).unwrap()).unwrap();
        assert_eq!(imported.unsupported.len(), 1);
        assert_eq!(imported.actions[0].steps[0].0, "actions.photoshopStep");
        let saved = export("S", &[&imported.actions[0]], &[], false).unwrap().0;
        assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0], source);
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        s.actions.list.extend(imported.actions);
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result.get("failed").is_some());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), past);
    }
}

/// Known native defaults become explicit values; unsupported defaults/options cannot disappear.
#[test]
fn caf_native_exports_preserve_effective_defaults_and_refuse_unrepresented_options() {
    for p in [json!({"sampling":"rectangular","output":"new"}), json!({"sampling":"rectangular","output":"new","seed":1,"target":"pixels"}), params()] {
        let a = load(exported(p).unwrap().descriptor.unwrap());
        assert_eq!(a.steps, vec![(CMD.into(), params())]);
    }
    let mut current = params();
    current["output"] = json!("current");
    for p in [json!({"sampling":"rectangular"}), current.clone()] {
        let a = load(exported(p).unwrap().descriptor.unwrap());
        assert_eq!(a.steps, vec![(CMD.into(), current.clone())]);
    }
    for (key, value) in [
        ("sampling", json!("auto")),
        ("sampling", json!("custom")),
        ("sampling", json!("unknown")),
        ("output", json!("duplicate")),
        ("colorAdaptation", json!("none")),
        ("colorAdaptation", json!("high")),
        ("colorAdaptation", json!("veryHigh")),
        ("rotationAdaptation", json!("low")),
        ("rotationAdaptation", json!("medium")),
        ("rotationAdaptation", json!("high")),
        ("rotationAdaptation", json!("full")),
        ("sampleAllLayers", json!(0)),
        ("scale", Value::Null),
        ("mirror", json!("true")),
        ("seed", json!(2)),
        ("layer", json!(1)),
        ("margin", json!(10)),
        ("area", json!([0, 0, 4, 4])),
        ("channel", json!(0)),
        ("samplingStrokes", json!([])),
        ("fillEdits", json!([])),
        ("target", json!("mask")),
        ("_atnCafDescriptor", Value::Null),
    ] {
        let mut p = params();
        p[key] = value;
        assert!(exported(p.clone()).is_none(), "{p}");
    }
    for p in [json!({}), json!({"output":"new"}), json!([])] {
        assert!(exported(p.clone()).is_none(), "unrepresented native default: {p}");
    }
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"white"})).unwrap();
    s.execute("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x":3,"y":3,"width":2,"height":2})).unwrap();
    s.execute("edit.fill", json!({"contents":"foreground"})).unwrap();
    s
}

/// Native and imported fills produce the same real pixels, source/selection state and Undo/Redo.
#[test]
fn caf_action_matches_native_pixels_and_undo_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let before = s.active().unwrap().doc.clone();
        let source = s.active().unwrap().active_layer.unwrap();
        let past = s.active().unwrap().history.past_len();
        s.actions.list.push(load(descriptor()));
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert!(result.get("failed").is_none(), "{depth}: {result}");
        assert_eq!(result["ran"], 1);
        let after = s.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), before.layers.len() + 1);
        assert_eq!(after.selection, before.selection);
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(after.layer(source), before.layer(source), "new output keeps its source unchanged");
        let id = s.active().unwrap().active_layer.unwrap();
        let pixels = after.layer(id).unwrap().surface().unwrap();
        let center = pixels.rgba(3, 3);
        assert!(center.iter().all(|v| (*v - 1.0).abs() < 0.01), "{depth}: hole was not filled: {center:?}");
        assert_eq!(pixels.rgba(0, 0)[3], 0.0, "new output is transparent outside the selection");
        let mut native = session(depth);
        native.execute(CMD, params()).unwrap();
        let expected = &native.active().unwrap().doc;
        assert_eq!(after.layers.len(), expected.layers.len());
        for (a, b) in after.layers.iter().zip(&expected.layers) {
            let (a, b) = (a.surface().unwrap(), b.surface().unwrap());
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(a.rgba(x, y), b.rgba(x, y), "{depth}: pixel{x},{y}");
                }
            }
        }
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc, after);

        // Current Layer keeps one layer and matches the ordinary native operation as well.
        let mut current = session(depth);
        let mut p = params();
        p["output"] = json!("current");
        current.execute(CMD, p).unwrap();
        assert_eq!(current.active().unwrap().doc.layers.len(), 1);
        let center = current.active().unwrap().doc.layers[0].surface().unwrap().rgba(3, 3);
        assert!(center.iter().all(|v| (*v - 1.0).abs() < 0.01), "{depth}: native Current Layer control failed: {center:?}");
        let mut recorded = session(depth);
        let before = recorded.active().unwrap().doc.clone();
        let source = recorded.active().unwrap().active_layer.unwrap();
        let past = recorded.active().unwrap().history.past_len();
        let mut d = descriptor();
        d.items[6].1 = enm("cafOutput", "cafOutputToCurrentLayer");
        recorded.actions.list.push(load(d));
        let result = recorded.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none(), "{depth}: {result}");
        let after = recorded.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), before.layers.len());
        assert_eq!(after.selection, before.selection);
        assert_eq!(recorded.active().unwrap().active_layer, Some(source));
        assert_eq!(recorded.active().unwrap().history.past_len(), past + 1);
        let expected = current.active().unwrap().doc.layers[0].surface().unwrap();
        let actual = after.layer(source).unwrap().surface().unwrap();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(actual.rgba(x, y), expected.rgba(x, y), "{depth}: Current Layer pixel{x},{y}");
            }
        }
        assert!(recorded.undo());
        assert_eq!(recorded.active().unwrap().doc, before);
        assert!(recorded.redo());
        assert_eq!(recorded.active().unwrap().doc, after);
    }
}

/// The adapter cannot manufacture a selection or turn a refused native operation into an edit.
#[test]
fn caf_action_without_selection_fails_without_an_edit() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    s.actions.list.push(load(descriptor()));
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert_eq!(result["ran"], 0);
    assert!(result.get("failed").is_some());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), past);
}
