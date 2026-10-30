use serde_json::json;

use super::*;

fn session() -> Session {
    Session::new()
}

#[test]
fn starring_toggles_and_on_sets_a_favourite() {
    let mut s = session();
    s.execute("type.font.favorite", json!({"family": "Arial"})).unwrap();
    s.execute("type.font.favorite", json!({"family": "Bebas Neue"})).unwrap();
    assert_eq!(favorite_fonts(&s), ["Arial", "Bebas Neue"]);
    // Toggling again removes it; `on` sets it either way.
    let r = s.execute("type.font.favorite", json!({"family": "Arial"})).unwrap();
    assert_eq!(r["favorite"], false);
    s.execute("type.font.favorite", json!({"family": "Bebas Neue", "on": true})).unwrap();
    assert_eq!(favorite_fonts(&s), ["Bebas Neue"], "on: true doesn't duplicate");
    s.execute("type.font.favorite", json!({"family": "Bebas Neue", "on": false})).unwrap();
    assert!(favorite_fonts(&s).is_empty());
}

#[test]
fn resetting_the_type_preferences_keeps_the_favourites() {
    let mut s = session();
    s.execute("type.font.favorite", json!({"family": "Bebas Neue"})).unwrap();
    s.execute("prefs.set", json!({"path": "type.smartQuotes", "value": false})).unwrap();
    s.execute("prefs.reset", json!({"path": "type"})).unwrap();
    assert!(s.prefs().type_.smart_quotes, "the Type settings are reset");
    assert_eq!(favorite_fonts(&s), ["Bebas Neue"], "the favourites are the user's library, not a setting");
    // Asked for by name, or with everything, they go.
    s.execute("prefs.reset", json!({"path": "type.favoriteFonts"})).unwrap();
    assert!(favorite_fonts(&s).is_empty());
    s.execute("type.font.favorite", json!({"family": "Bebas Neue"})).unwrap();
    s.execute("prefs.reset", json!({})).unwrap();
    assert!(favorite_fonts(&s).is_empty());
}

#[test]
fn the_library_survives_a_preferences_round_trip_and_hostile_lists_are_tidied() {
    let mut s = session();
    s.execute("type.font.favorite", json!({"family": "Arial"})).unwrap();
    let saved = serde_json::to_value(s.prefs()).unwrap();
    assert_eq!(saved["type"]["favoriteFonts"], json!(["Arial"]));
    let mut t = session();
    let prefs = serde_json::from_value(saved).unwrap();
    t.prefs.edit(|p| *p = prefs);
    let lib = t.execute("type.font.library", json!({})).unwrap();
    assert_eq!(lib, json!({"favorites": ["Arial"]}));
    // Duplicates, blanks and an oversized list from a hand-edited file are tidied on read.
    t.prefs.edit(|p| p.type_.favorite_fonts = (0..5000).map(|i| format!("F{}", i % 1200)).chain([String::new()]).collect());
    assert_eq!(favorite_fonts(&t).len(), MAX_FAVORITE_FONTS);
}

#[test]
fn bad_params_are_errors_and_change_nothing() {
    let mut s = session();
    s.execute("type.font.favorite", json!({"family": "Arial"})).unwrap();
    for (id, p) in [
        ("type.font.favorite", json!({})),
        ("type.font.favorite", json!({"family": 42})),
        ("type.font.favorite", json!({"family": "   "})),
        ("type.font.favorite", json!({"family": "a\u{0}b"})),
        ("type.font.favorite", json!({"family": "x".repeat(300)})),
        ("type.font.favorite", json!({"family": "Arial", "on": "yes"})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
    assert_eq!(favorite_fonts(&s), ["Arial"]);
}

#[test]
fn font_library_edits_are_not_recorded_into_actions() {
    // Playing an action back must not rewrite the user's favourite fonts.
    for id in ["type.font.favorite", "type.font.library"] {
        assert!(!crate::actions_cmds::replayable(id), "{id}");
    }
}
