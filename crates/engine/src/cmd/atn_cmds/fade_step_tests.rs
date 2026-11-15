use super::*;

/// One action with a single recorded Fade step.
fn fade_file(descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Fade"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Fade"), dictionary: b"Fade".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// The command, opacity and mode a recorded Fade step imports as.
fn imported(descriptor: Descriptor) -> (String, f64, String) {
    let loaded = import(&fade_file(descriptor)).unwrap();
    assert!(loaded.unsupported.is_empty());
    let (id, params) = &loaded.actions[0].steps[0];
    (id.clone(), params["opacity"].as_f64().unwrap(), params["mode"].as_str().unwrap().to_owned())
}

/// An 8 x 8 image, inverted so there is an edit to fade.
fn inverted(depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#4080c0"})).unwrap();
    s.execute("image.adjustments.invert", json!({})).unwrap();
    s
}

/// The pixel a recorded Fade step leaves after the Invert it fades.
fn played(depth: u8, descriptor: Descriptor) -> Value {
    let mut s = inverted(depth);
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&fade_file(descriptor))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    let result = s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    s.execute("document.pixel", json!({"x":3,"y":3})).unwrap()
}

/// The pixel `edit.fade` leaves when run directly with these params.
fn faded(depth: u8, params: Value) -> Value {
    let mut s = inverted(depth);
    s.execute("edit.fade", params).unwrap();
    s.execute("document.pixel", json!({"x":3,"y":3})).unwrap()
}

/// A step with no keys plays at 100 % Normal; with one key, the other takes its default.
#[test]
fn fade_step_missing_keys_take_fixed_defaults() {
    assert_eq!(imported(desc("null")), ("edit.fade".into(), 100.0, "Normal".into()));
    assert_eq!(imported(desc("null").with("Opct", pct(40.0))), ("edit.fade".into(), 40.0, "Normal".into()));
    assert_eq!(imported(desc("null").with("Md  ", enm("BlnM", "Mltp"))), ("edit.fade".into(), 100.0, "Multiply".into()));
    for depth in [8, 16, 32] {
        assert_eq!(played(depth, desc("null")), faded(depth, json!({"opacity":100,"mode":"normal"})));
        assert_eq!(played(depth, desc("null").with("Md  ", enm("BlnM", "Mltp"))), faded(depth, json!({"opacity":100,"mode":"multiply"})));
    }
}

/// Opacity less than 100 out of range wraps modulo 100 and fractions round; whole values in range
/// are unchanged, and unmeasured values further out keep the clamp.
#[test]
fn fade_step_opacity_wraps_and_rounds() {
    for (recorded, expected) in [
        (150.0, 50.0),
        (-5.0, 95.0),
        (50.5, 51.0),
        (0.0, 0.0),
        (37.0, 37.0),
        (100.0, 100.0),
        (-0.4, 0.0),
        (199.0, 99.0),
        (200.0, 100.0),
        (1e9, 100.0),
        (-100.0, 0.0),
        (-105.0, 0.0),
    ] {
        let (id, opacity, _) = imported(desc("null").with("Opct", pct(recorded)).with("Md  ", enm("BlnM", "Nrml")));
        assert_eq!(id, "edit.fade");
        assert_eq!(opacity, expected, "{recorded}");
        assert!(opacity.is_sign_positive(), "{recorded}");
    }
    for depth in [8, 16, 32] {
        let recorded = desc("null").with("Opct", pct(150.0)).with("Md  ", enm("BlnM", "Nrml"));
        assert_eq!(played(depth, recorded), faded(depth, json!({"opacity":50,"mode":"normal"})));
    }
}

/// Recording still writes both keys, so a step imported without them saves them explicitly.
#[test]
fn fade_step_saves_both_keys() {
    let loaded = import(&fade_file(desc("null"))).unwrap();
    let saved = export("Fade", &[&loaded.actions[0]], &[], false).unwrap().0;
    let d = atn::parse(&saved).unwrap().actions[0].items[0].descriptor.clone().unwrap();
    assert_eq!(d, desc("null").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml")));
}
