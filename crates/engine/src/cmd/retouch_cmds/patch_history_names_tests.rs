//! History step names of the Patch Tool (#12117): "Patch Tool" for the lasso it draws with and
//! "Patch Selection" for applying a patch in every mode; the plain Lasso tool keeps "Lasso".

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s
}

fn last_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

#[test]
fn applying_a_patch_is_one_step_named_patch_selection_in_every_mode() {
    let modes = [
        json!({"offset": [30, 0]}),
        json!({"offset": [30, 0], "mode": "source"}),
        json!({"offset": [30, 0], "mode": "destination"}),
        json!({"offset": [30, 0], "contentAware": true}),
        json!({"offset": [30, 0], "mode": "destination", "contentAware": true}),
    ];
    for p in modes {
        let mut s = session();
        s.execute("select.rect", json!({"x": 16, "y": 22, "width": 20, "height": 20})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("paint.patch", p.clone()).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{p}: one step");
        assert_eq!(last_step(&s).as_deref(), Some("Patch Selection"), "{p}");
    }
}

const TRIANGLE: [[i32; 2]; 3] = [[16, 22], [36, 22], [16, 42]];

#[test]
fn the_lasso_is_named_for_the_tool_that_drew_it() {
    let mut s = session();
    s.execute("select.lasso", json!({"points": TRIANGLE})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Lasso"), "the Lasso tool sends no name");
    for mode in ["replace", "add", "subtract", "intersect"] {
        s.execute("select.lasso", json!({"points": TRIANGLE, "mode": mode, "historyName": "Patch Tool"})).unwrap();
        assert_eq!(last_step(&s).as_deref(), Some("Patch Tool"), "{mode}");
    }
    s.execute("select.lasso", json!({"points": TRIANGLE, "polygonal": true, "historyName": "Patch Tool"})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Polygonal Lasso"), "the Polygonal Lasso keeps its own name");
}

#[test]
fn a_hostile_history_name_is_refused_without_a_step() {
    let mut s = session();
    let steps = s.active().unwrap().history.past_len();
    let invisible = ["\u{200B}", " \u{200B}\u{FEFF} ", "\u{3164}", "\u{FE0F}", "\u{E0020}", "\u{00AD}", "\u{2800}"];
    let hidden = ["Patch\u{202E}looT", "\u{FEFF}Patch Tool", "Patch Tool\u{200B}", "\u{2066}Patch", "Patch\u{2028}Tool", "a\u{2029}b"];
    let ends = ["Patch Tool\n", "\r\nPatch Tool", "\tPatch", "\u{0085}Patch", "Patch\u{2028}"];
    let others = [json!(""), json!("   "), json!("x".repeat(65)), json!("a\nb"), json!(7), json!(["Patch Tool"])];
    let others = ends.iter().map(|n| json!(n)).chain(others);
    for name in invisible.iter().chain(&hidden).map(|n| json!(n)).chain(others) {
        assert!(s.execute("select.lasso", json!({"points": TRIANGLE, "historyName": name})).is_err(), "{name}");
        let polygonal = json!({"points": TRIANGLE, "polygonal": true, "historyName": name});
        assert!(s.execute("select.lasso", polygonal).is_err(), "polygonal {name}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn a_name_joining_its_letters_with_a_zero_width_joiner_is_kept() {
    let mut s = session();
    for name in ["\u{0645}\u{200C}\u{06CC}", "\u{1F469}\u{200D}\u{1F4BB}", "Patch Tool \u{2713}"] {
        s.execute("select.lasso", json!({"points": TRIANGLE, "historyName": name})).unwrap();
        assert_eq!(last_step(&s).as_deref(), Some(name));
    }
}

#[test]
fn spaces_and_joiners_at_the_ends_are_trimmed_and_only_what_is_left_counts() {
    let mut s = session();
    let padded = format!("{}{}", " ".repeat(10), "x".repeat(60));
    let joined = format!("Patch Tool{}", "\u{200D}".repeat(60));
    let cases = [
        ("Patch Tool\u{200D}", "Patch Tool"),
        (" \u{200C}Patch Tool ", "Patch Tool"),
        ("Love \u{2764}\u{FE0F}", "Love \u{2764}\u{FE0F}"),
        (joined.as_str(), "Patch Tool"),
        ("Patch Tool\u{2800}", "Patch Tool"),
        ("\u{3164}\u{2800}Patch Tool\u{FFA0} \u{115F}", "Patch Tool"),
        ("Patch Tool\u{2065}\u{FFF0}", "Patch Tool"),
        ("\u{17B4}Patch Tool", "Patch Tool"),
        ("Patch Tool \u{FE0F}", "Patch Tool"),
        ("\u{FE0F}Patch Tool", "Patch Tool"),
        ("Patch \u{2764}\u{FE0F}\u{2800}", "Patch \u{2764}\u{FE0F}"),
        ("Patch \u{1100}\u{1160}", "Patch \u{1100}\u{1160}"),
        ("\u{115F}\u{1161} Patch", "\u{115F}\u{1161} Patch"),
    ];
    for (name, stored) in cases.iter().copied().chain([(padded.as_str(), &padded[10..])]) {
        s.execute("select.lasso", json!({"points": TRIANGLE, "historyName": name})).unwrap();
        assert_eq!(last_step(&s).as_deref(), Some(stored), "{name:?}");
    }
}
