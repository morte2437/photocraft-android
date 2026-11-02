use super::*;

fn names(s: &Session, g: &str) -> Vec<String> {
    s.tools.presets.iter().filter(|x| x.group == g).map(|x| x.name.clone()).collect()
}

fn with_groups() -> Session {
    let mut s = Session::new();
    for (n, g) in [("A1", "Alpha"), ("A2", "Alpha"), ("A3", "Alpha"), ("B1", "Beta"), ("B2", "Beta")] {
        s.tools.presets.push(BrushPreset {
            name: n.into(),
            brush: Default::default(),
            builtin: false,
            group: g.into(),
            folder: Vec::new(),
            captures: Default::default(),
        });
    }
    s
}

#[test]
fn move_within_and_between_groups() {
    let mut s = with_groups();
    let r = s.execute("brush.presets.move", json!({"name": "A3", "before": "A1"})).unwrap();
    assert_eq!(r, json!({"name": "A3", "group": "Alpha", "folder": [], "index": 0}));
    assert_eq!(names(&s, "Alpha"), ["A3", "A1", "A2"]);
    s.execute("brush.presets.move", json!({"name": "A3", "index": 99})).unwrap();
    assert_eq!(names(&s, "Alpha"), ["A1", "A2", "A3"]);
    s.execute("brush.presets.move", json!({"name": "A1", "index": 1})).unwrap();
    assert_eq!(names(&s, "Alpha"), ["A2", "A1", "A3"]);
    // Into another group, before a member (the group comes from `before`).
    s.execute("brush.presets.move", json!({"name": "A1", "before": "B2"})).unwrap();
    assert_eq!((names(&s, "Alpha"), names(&s, "Beta")), (vec!["A2".to_string(), "A3".into()], vec!["B1".to_string(), "A1".into(), "B2".into()]));
    // Into a group by name (appended), and into a new group.
    s.execute("brush.presets.move", json!({"name": "A2", "group": "Beta"})).unwrap();
    assert_eq!(names(&s, "Beta"), ["B1", "A1", "B2", "A2"]);
    s.execute("brush.presets.move", json!({"name": "A3", "group": "Gamma"})).unwrap();
    assert_eq!(group_order(&s.tools.presets).last().map(String::as_str), Some("Gamma"));
    assert!(names(&s, "Alpha").is_empty());
    // A built-in moved to another group becomes the user's.
    let b = s.tools.presets.iter().find(|x| x.builtin).unwrap().name.clone();
    s.execute("brush.presets.move", json!({"name": b, "group": "Beta", "index": 0})).unwrap();
    let moved = photocraft_paint::presets::find(&s.tools.presets, &b).unwrap();
    assert!(!moved.builtin && moved.group == "Beta");
    assert_eq!(names(&s, "Beta")[0], b);
}

#[test]
fn groups_move_rename_and_delete() {
    let mut s = with_groups();
    let order = group_order(&s.tools.presets);
    let n = order.len();
    assert_eq!(&order[n - 2..], ["Alpha", "Beta"]);
    let r = s.execute("brush.presets.moveGroup", json!({"group": "Beta", "index": 0})).unwrap();
    assert_eq!(r["index"], 0);
    assert_eq!(group_order(&s.tools.presets)[0], "Beta");
    assert_eq!(names(&s, "Beta"), ["B1", "B2"], "a group moves as a block, in order");
    s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "before": "Beta"})).unwrap();
    assert_eq!(&group_order(&s.tools.presets)[..2], ["Alpha", "Beta"]);
    s.execute("brush.presets.moveGroup", json!({"group": "Alpha"})).unwrap();
    assert_eq!(group_order(&s.tools.presets).last().map(String::as_str), Some("Alpha"));
    s.execute("brush.presets.renameGroup", json!({"group": "Alpha", "newName": "First"})).unwrap();
    assert_eq!(names(&s, "First"), ["A1", "A2", "A3"]);
    assert!(s.execute("brush.presets.renameGroup", json!({"group": "First", "newName": "Beta"})).is_err());
    let r = s.execute("brush.presets.deleteGroup", json!({"group": "First"})).unwrap();
    assert_eq!(r["deleted"], 3);
    assert!(names(&s, "First").is_empty());
}

#[test]
fn rename_checks_names() {
    let mut s = with_groups();
    s.execute("brush.presets.rename", json!({"name": "a1", "newName": "Alpha One"})).unwrap();
    assert!(photocraft_paint::presets::find(&s.tools.presets, "Alpha One").is_some());
    assert!(s.execute("brush.presets.rename", json!({"name": "A2", "newName": "alpha one"})).is_err());
    // Case-only renames of itself are fine.
    s.execute("brush.presets.rename", json!({"name": "Alpha One", "newName": "ALPHA ONE"})).unwrap();
    let b = s.tools.presets.iter().find(|x| x.builtin).unwrap().name.clone();
    s.execute("brush.presets.rename", json!({"name": b, "newName": "My Round"})).unwrap();
    assert!(!photocraft_paint::presets::find(&s.tools.presets, "My Round").unwrap().builtin);
}

#[test]
fn bad_params_fail_without_changes() {
    let mut s = with_groups();
    let before = s.tools.presets.clone();
    let long = "x".repeat(MAX_NAME + 1);
    for (id, p) in [
        ("brush.presets.move", json!({})),
        ("brush.presets.move", json!({"name": "Nope"})),
        ("brush.presets.move", json!({"name": "A1", "before": "A1"})),
        ("brush.presets.move", json!({"name": "A1", "before": "Nope"})),
        ("brush.presets.move", json!({"name": "A1", "group": "Alpha", "before": "B1"})),
        ("brush.presets.move", json!({"name": "A1", "index": -1})),
        ("brush.presets.move", json!({"name": "A1", "index": "first"})),
        ("brush.presets.move", json!({"name": "A1", "group": 7})),
        ("brush.presets.move", json!({"name": "A1", "group": long})),
        ("brush.presets.rename", json!({"name": "A1"})),
        ("brush.presets.rename", json!({"name": "A1", "newName": "  "})),
        ("brush.presets.rename", json!({"name": "A1", "newName": long})),
        ("brush.presets.moveGroup", json!({"group": "Nope"})),
        ("brush.presets.moveGroup", json!({"group": "Alpha", "before": "Alpha"})),
        ("brush.presets.moveGroup", json!({"group": "Alpha", "index": 1.5})),
        ("brush.presets.renameGroup", json!({"group": "Alpha"})),
        ("brush.presets.deleteGroup", json!({"group": null})),
        ("brush.presets.deleteGroup", json!({"group": "Nope"})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
    assert_eq!(s.tools.presets, before);
    // Huge indices clamp to the end.
    s.execute("brush.presets.move", json!({"name": "A1", "index": u64::MAX})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Alpha", "index": u64::MAX})).unwrap();
}

/// Group "Set": S0 at its top level, folders Ink (I1, I2; subfolder Fine: F1) and Dry (D1).
fn with_folders() -> Session {
    let mut s = Session::new();
    for (n, f) in [("S0", &[][..]), ("I1", &["Ink"][..]), ("F1", &["Ink", "Fine"][..]), ("I2", &["Ink"][..]), ("D1", &["Dry"][..])] {
        let folder = f.iter().map(|x| x.to_string()).collect();
        s.tools.presets.push(BrushPreset {
            name: n.into(),
            brush: Default::default(),
            builtin: false,
            group: "Set".into(),
            folder,
            captures: Default::default(),
        });
    }
    s
}

fn folder_of(s: &Session, n: &str) -> Vec<String> {
    photocraft_paint::presets::find(&s.tools.presets, n).unwrap().folder.clone()
}

#[test]
fn presets_move_between_folders() {
    let mut s = with_folders();
    // `before` brings its folder.
    let r = s.execute("brush.presets.move", json!({"name": "D1", "before": "I2"})).unwrap();
    assert_eq!(r["folder"], json!(["Ink"]));
    assert_eq!(folder_of(&s, "D1"), ["Ink"]);
    // An explicit folder, appended at the end of it.
    s.execute("brush.presets.move", json!({"name": "S0", "folder": ["Ink", "Fine"]})).unwrap();
    assert_eq!(folder_of(&s, "S0"), ["Ink", "Fine"]);
    let pos = |n: &str| s.tools.presets.iter().position(|x| x.name == n).unwrap();
    assert_eq!(pos("S0"), pos("F1") + 1);
    // Within its group without a folder keeps its folder; to another group without one lands
    // at that group's top level; `folder: []` is the group's top level.
    s.execute("brush.presets.move", json!({"name": "S0", "index": 0})).unwrap();
    assert_eq!(folder_of(&s, "S0"), ["Ink", "Fine"]);
    s.execute("brush.presets.move", json!({"name": "S0", "group": "Other"})).unwrap();
    assert!(folder_of(&s, "S0").is_empty());
    s.execute("brush.presets.move", json!({"name": "I1", "folder": []})).unwrap();
    assert!(folder_of(&s, "I1").is_empty());
    // `before` in another folder than the one named.
    assert!(s.execute("brush.presets.move", json!({"name": "I1", "folder": ["Dry"], "before": "I2"})).is_err());
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink"]);
}

#[test]
fn folders_move_rename_and_delete() {
    let mut s = with_folders();
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink", "Dry"]);
    // Move Dry before Ink: the whole folder moves as a block.
    let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "before": "Ink"})).unwrap();
    assert_eq!(r, json!({"group": "Set", "folder": ["Dry"], "index": 0}));
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Dry", "Ink"]);
    s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"]})).unwrap();
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Ink", "Dry"]);
    s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "index": 0})).unwrap();
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Dry", "Ink"]);
    let ink: Vec<&str> = s.tools.presets.iter().filter(|x| x.folder.first().is_some_and(|f| f == "Ink")).map(|x| x.name.as_str()).collect();
    assert_eq!(ink, ["I1", "F1", "I2"], "presets keep their order inside a moved folder");
    // Rename a nested folder: every preset below it follows.
    let r = s.execute("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink"], "newName": "Inks"})).unwrap();
    assert_eq!(r["folder"], json!(["Inks"]));
    assert_eq!(folder_of(&s, "F1"), ["Inks", "Fine"]);
    assert!(s.execute("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Inks"], "newName": "Dry"})).is_err());
    // Renaming the group keeps the folders.
    s.execute("brush.presets.renameGroup", json!({"group": "Set", "newName": "Kit"})).unwrap();
    assert_eq!(folder_of(&s, "F1"), ["Inks", "Fine"]);
    // Delete a nested folder: only the presets in it (and below) go.
    let r = s.execute("brush.presets.deleteGroup", json!({"group": "Kit", "folder": ["Inks"]})).unwrap();
    assert_eq!(r["deleted"], 3);
    assert!(photocraft_paint::presets::find(&s.tools.presets, "S0").is_some());
    assert!(photocraft_paint::presets::find(&s.tools.presets, "D1").is_some());
}

#[test]
fn bad_folder_params_fail_without_changes() {
    let mut s = with_folders();
    let before = s.tools.presets.clone();
    let long = "x".repeat(MAX_NAME + 1);
    let deep: Vec<&str> = vec!["a"; MAX_FOLDER_DEPTH + 1];
    for (id, p) in [
        ("brush.presets.move", json!({"name": "S0", "folder": "Ink"})),
        ("brush.presets.move", json!({"name": "S0", "folder": [""]})),
        ("brush.presets.move", json!({"name": "S0", "folder": [7]})),
        ("brush.presets.move", json!({"name": "S0", "folder": [long]})),
        ("brush.presets.move", json!({"name": "S0", "folder": deep})),
        ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Nope"]})),
        ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": "Ink"})),
        ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": "Nope"})),
        ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "before": 3})),
        ("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "index": -2})),
        ("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink"], "newName": ""})),
        ("brush.presets.renameGroup", json!({"group": "Set", "folder": ["Ink", "Nope"], "newName": "x"})),
        ("brush.presets.deleteGroup", json!({"group": "Set", "folder": ["Dry", "Nope"]})),
        ("brush.presets.deleteGroup", json!({"group": "Nope", "folder": ["Dry"]})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
    assert_eq!(s.tools.presets, before);
    s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink", "Fine"], "index": u64::MAX})).unwrap();
}

fn paths(s: &Session) -> Vec<String> {
    s.tools.presets.iter().filter(|x| !x.builtin).map(|x| format!("{}:{}:{}", x.group, x.folder.join("/"), x.name)).collect()
}

/// `with_folders` plus group "Other" holding O1, and folder Box (B1) in it.
fn with_two_groups() -> Session {
    let mut s = with_folders();
    for (n, f) in [("O1", &[][..]), ("B1", &["Box"][..])] {
        let folder = f.iter().map(|x| x.to_string()).collect();
        s.tools.presets.push(BrushPreset {
            name: n.into(),
            brush: Default::default(),
            builtin: false,
            group: "Other".into(),
            folder,
            captures: Default::default(),
        });
    }
    s
}

#[test]
fn folders_move_into_folders_and_groups() {
    let mut s = with_two_groups();
    // A nested folder into a sibling folder: its subtree follows, order kept.
    let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}})).unwrap();
    assert_eq!(r, json!({"group": "Set", "folder": ["Ink", "Fine", "Dry"], "index": 0}));
    assert_eq!(folder_of(&s, "D1"), ["Ink", "Fine", "Dry"]);
    // Into another group, before one of its folders.
    let r = s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Other"}, "before": "Box"})).unwrap();
    assert_eq!(r["folder"], json!(["Ink"]));
    assert_eq!(paths(&s), ["Set::S0", "Other::O1", "Other:Ink:I1", "Other:Ink/Fine:F1", "Other:Ink/Fine/Dry:D1", "Other:Ink:I2", "Other:Box:B1"]);
    // Out to the top level: the folder becomes a group of its own.
    let r = s.execute("brush.presets.moveGroup", json!({"group": "Other", "folder": ["Ink", "Fine"], "into": {}})).unwrap();
    assert_eq!(r["group"], "Fine");
    assert_eq!(folder_of(&s, "D1"), ["Dry"]);
    assert_eq!(photocraft_paint::presets::find(&s.tools.presets, "F1").unwrap().group, "Fine");
    // A whole group into another group's folder.
    s.execute("brush.presets.moveGroup", json!({"group": "Fine", "into": {"group": "Set"}})).unwrap();
    assert!(names(&s, "Fine").is_empty());
    assert_eq!(folder_of(&s, "D1"), ["Fine", "Dry"]);
    assert_eq!(subfolder_order(&s.tools.presets, "Set", &[]), ["Fine"]);
}

#[test]
fn bad_moves_into_folders_fail_without_changes() {
    let mut s = with_two_groups();
    // A chain 31 deep under Set/Deep.
    let deep: Vec<String> = std::iter::once("Deep".to_string()).chain((0..30).map(|i| format!("d{i}"))).collect();
    s.tools.presets.push(BrushPreset {
        name: "Z".into(),
        brush: Default::default(),
        builtin: false,
        group: "Set".into(),
        folder: deep.clone(),
        captures: Default::default(),
    });
    s.tools.presets.push(BrushPreset {
        name: "Y".into(),
        brush: Default::default(),
        builtin: false,
        group: "Other".into(),
        folder: vec!["Dry".into()],
        captures: Default::default(),
    });
    let before = s.tools.presets.clone();
    let deep_json = json!(deep);
    for p in [
        // Into itself or its own descendant.
        json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": ["Ink"]}}),
        json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}}),
        json!({"group": "Set", "into": {"group": "Set", "folder": ["Dry"]}}),
        json!({"group": "Set", "into": {"group": "Set"}}),
        // A group already at the top level; the ungrouped presets.
        json!({"group": "Set", "into": {}}),
        // A name clash at the destination.
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other"}}),
        json!({"group": "Other", "folder": ["Box"], "into": {"group": "Set", "folder": ["Ink"]}, "before": "Nope"}),
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Set", "folder": ["Ink"]}, "index": 0}),
        // Missing destinations and bad shapes.
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Nope"}}),
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other", "folder": ["Nope"]}}),
        json!({"group": "Set", "folder": ["Dry"], "into": {"folder": ["Box"]}}),
        json!({"group": "Set", "folder": ["Dry"], "into": "Other"}),
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": 5}}),
        json!({"group": "Set", "folder": ["Dry"], "into": {"group": "Other"}, "before": 3}),
        // Deeper than MAX_FOLDER_DEPTH once moved (Ink/Fine is 2 deep: 2 + 31 > 32).
        json!({"group": "Set", "folder": ["Deep"], "into": {"group": "Set", "folder": ["Ink", "Fine"]}}),
        json!({"group": "Set", "folder": ["Ink"], "into": {"group": "Set", "folder": deep_json}}),
    ] {
        assert!(s.execute("brush.presets.moveGroup", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.tools.presets, before);
    // Ink/Fine into Set itself: a clash with nothing, so it moves up a level.
    s.execute("brush.presets.moveGroup", json!({"group": "Set", "folder": ["Ink", "Fine"], "into": {"group": "Set"}})).unwrap();
    assert_eq!(folder_of(&s, "F1"), ["Fine"]);
    // An ungrouped preset's group can't move.
    s.tools.presets.push(BrushPreset {
        name: "U".into(),
        brush: Default::default(),
        builtin: false,
        group: String::new(),
        folder: Vec::new(),
        captures: Default::default(),
    });
    assert!(s.execute("brush.presets.moveGroup", json!({"group": "", "into": {"group": "Set"}})).is_err());
}
