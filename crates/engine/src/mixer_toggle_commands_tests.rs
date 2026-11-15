//! The Mixer Brush's Auto-Load, Auto-Clean and Sample All Layers switches are commands, so
//! Keyboard Shortcuts lists them with Load and Clean and can give them a key (#11959). None has a
//! default key; each flips its `tools.brush.mixer` field, or sets it from `on`.

use crate::Session;
use crate::prefs::MIXER_BRUSH_COMMANDS;
use serde_json::{Value, json};

const SWITCHES: [(&str, &str); 3] = [
    ("brush.mixer.autoLoad", "Mixer Brush Auto-Load"),
    ("brush.mixer.autoClean", "Mixer Brush Auto-Clean"),
    ("brush.mixer.sampleAllLayers", "Mixer Brush Sample All Layers"),
];

fn values(s: &Session) -> [bool; 3] {
    let m = &s.tools.brush.mixer;
    [m.load_after_stroke, m.clean_after_stroke, m.sample_all_layers]
}

#[test]
fn each_switch_flips_its_setting_or_sets_it_from_on() {
    let mut s = Session::new();
    assert_eq!(values(&s), [true, true, false], "the options bar's defaults");
    for (i, (id, _)) in SWITCHES.iter().enumerate() {
        let before = values(&s);
        let out = s.execute(id, json!({})).unwrap();
        let mut flipped = before;
        flipped[i] = !before[i];
        assert_eq!(values(&s), flipped, "{id} flips only its own setting");
        assert_eq!(out, json!({"on": flipped[i]}));
        s.execute(id, json!({"on": true})).unwrap();
        assert!(values(&s)[i], "{id} on");
        s.execute(id, json!({"on": false})).unwrap();
        assert!(!values(&s)[i], "{id} off");
        s.execute(id, json!({"on": null})).unwrap();
        assert!(values(&s)[i], "{id}: null toggles");
    }
}

#[test]
fn wrong_types_are_errors_and_change_nothing() {
    let mut s = Session::new();
    for (id, _) in SWITCHES {
        let before = values(&s);
        for bad in [json!({"on": 1}), json!({"on": "yes"}), json!({"on": [true]})] {
            assert!(s.execute(id, bad.clone()).is_err(), "{id} {bad}");
        }
        assert_eq!(values(&s), before);
    }
}

#[test]
fn keyboard_shortcuts_list_the_five_without_keys_and_can_assign_one() {
    assert_eq!(MIXER_BRUSH_COMMANDS, ["brush.mixer.load", "brush.mixer.clean", "brush.mixer.autoLoad", "brush.mixer.autoClean", "brush.mixer.sampleAllLayers"]);
    let specs = crate::command_specs();
    let labels: Vec<&str> = MIXER_BRUSH_COMMANDS.iter().map(|id| specs.iter().find(|c| c.id == *id).unwrap_or_else(|| panic!("{id}")).label).collect();
    assert_eq!(labels, ["Load Mixer Brush", "Clean Mixer Brush", "Mixer Brush Auto-Load", "Mixer Brush Auto-Clean", "Mixer Brush Sample All Layers"]);
    for id in MIXER_BRUSH_COMMANDS {
        assert_eq!(specs.iter().find(|c| c.id == id).unwrap().shortcut, None, "{id} has no default key");
    }
    let mut s = Session::new();
    let out = s.execute("edit.keyboardShortcuts", json!({"set": {"brush.mixer.autoClean": "F9"}, "list": true})).unwrap();
    let row = |id: &str| out["commands"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap_or(Value::Null);
    assert_eq!(row("brush.mixer.autoClean")["shortcut"], "F9");
    assert_eq!(row("brush.mixer.autoClean")["default"], Value::Null);
    for (id, label) in SWITCHES {
        assert_eq!(row(id)["label"], label);
    }
}

/// A switch flipped by its key is saved like any other brush change: it moves the brush revision
/// the last-brush persistence watches (CodeRabbit review on brush_cmds.rs:825).
#[test]
fn each_switch_marks_the_brush_changed_so_it_is_saved() {
    let mut s = Session::new();
    for (id, _) in SWITCHES {
        let rev = s.tools.brush_rev;
        s.execute(id, json!({})).unwrap();
        assert_ne!(s.tools.brush_rev, rev, "{id} moves the brush revision");
    }
}
