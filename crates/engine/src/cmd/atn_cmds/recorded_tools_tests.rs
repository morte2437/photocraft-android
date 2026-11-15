//! #10710: Actions steps recorded with the Gradient tool and Create Droplet, as the issue's table
//! gives them (measured in `docs/atn.md` › "Measured through dialogs and recordings"; once mapped,
//! the events are listed with the supported events instead).

use super::*;

fn action(items: Vec<AtnItem>) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Recorded"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    })
    .unwrap()
}

fn step(event: &[u8; 4], label: &str, d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::Code(*event), dictionary: label.as_bytes().to_vec(), descriptor: Some(d), ..Default::default() }
}

fn saved(s: &mut Session) -> Vec<AtnItem> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items
}

/// The gradient object the Gradient tool records: dither, classic interpolation, angle, type,
/// scale and offset in percent, then the two-stop gradient.
fn tool_gradient(angle: f64, scale: f64, offset: (f64, f64)) -> Descriptor {
    let stop = |hex: &str, at: i32| {
        D::Descriptor(
            desc("Clrt")
                .with("Clr ", D::Descriptor(color_desc(hex).unwrap()))
                .with("Type", enm("Clry", "UsrS"))
                .with("Lctn", D::Integer(at))
                .with("Mdpn", D::Integer(50)),
        )
    };
    let trn = |at: i32| D::Descriptor(desc("TrnS").with("Opct", pct(100.0)).with("Lctn", D::Integer(at)).with("Mdpn", D::Integer(50)));
    let grad = desc("Grdn")
        .with("Nm  ", text("Custom"))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(4096.0))
        .with("Clrs", D::List(vec![stop("#102030", 0), stop("#f0e0d0", 4096)]))
        .with("Trns", D::List(vec![trn(0), trn(4096)]));
    desc("gradientLayer")
        .with("Dthr", D::Boolean(true))
        .with("gradientsInterpolationMethod", enm("gradientInterpolationMethodType", "Gcls"))
        .with("Angl", ang(angle))
        .with("Type", enm("GrdT", "Lnr "))
        .with("Scl ", pct(scale))
        .with("Ofst", D::Descriptor(desc("Pnt ").with("Hrzn", pct(offset.0)).with("Vrtc", pct(offset.1))))
        .with("Grad", D::Descriptor(grad))
}

/// The Gradient tool (default mode) records a Make of a gradient fill layer at the first click and
/// a Set with the finished drag (`Algn` false). Both load as their commands, play, and save back
/// as recorded.
#[test]
fn gradient_tool_steps_load_play_and_save_back() {
    let make = desc("null")
        .with("null", class_ref("contentLayer"))
        .with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(tool_gradient(90.0, 100.0, (0.0, 0.0))))));
    let set = desc("null")
        .with("null", target_ref("contentLayer"))
        .with("T   ", D::Descriptor(tool_gradient(-35.0, 140.0, (12.5, -8.0)).with("Algn", D::Boolean(false))));
    let items = vec![step(b"Mk  ", "Make", make.clone()), step(b"setd", "Set", set.clone())];
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&action(items))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let ids: Vec<&str> = s.actions.list[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["layer.newFillLayer.gradient", "gradient.fill.set"]);
    let set_params = &s.actions.list[0].steps[1].1;
    assert_eq!(
        (set_params["angle"].as_f64(), set_params["scale"].as_f64(), set_params["align"].as_bool()),
        (Some(-35.0), Some(140.0), Some(false)),
        "{set_params}"
    );

    s.execute("file.new", json!({"width": 20, "height": 12})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(played["ran"], 2, "{played}");

    let back = saved(&mut s);
    assert_eq!(back.len(), 2);
    assert_eq!((back[0].event.id(), back[0].descriptor.as_ref()), ("Mk  ".to_string(), Some(&make)));
    assert_eq!((back[1].event.id(), back[1].descriptor.as_ref()), ("setd".to_string(), Some(&set)));
}

/// A Create Droplet step as the reference application records it with a Folder destination and
/// Override Action "Save As" on.
fn droplet(file: &str, output: &str, extra: &[(&str, D)]) -> Descriptor {
    let reference = D::Reference(vec![
        ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("Sharpen") },
        ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul("Tools") },
    ]);
    let mut d = desc("null").with("In  ", path_value(file)).with("Fldr", D::Boolean(false)).with("Usng", reference);
    for (k, v) in extra.iter().take_while(|(k, _)| *k == "OvrO") {
        d = d.with(k, v.clone());
    }
    d = d.with("T   ", path_value(output)).with("Ovrd", D::Boolean(true));
    for (k, v) in extra.iter().filter(|(k, _)| *k != "OvrO") {
        d = d.with(k, v.clone());
    }
    d
}

#[test]
fn unresolved_droplet_identity_cannot_export_a_fallback_caption() {
    let mut params = json!({"path": "C:/work/Sharpen.pcdroplet", "action": "Sharpen", "set": "Tools", "output": "C:/work/out"});
    assert!(export_item("file.automate.createDroplet", &params, "Tools").is_some());
    params["actionId"] = Value::Null;
    assert!(export_item("file.automate.createDroplet", &params, "Tools").is_none());
}

/// Create Droplet loads as `file.automate.createDroplet`, plays (writes the droplet with the
/// action's steps) and saves back as recorded.
#[test]
fn create_droplet_step_loads_plays_and_saves_back() {
    let dir = std::env::temp_dir().join(format!("pc-10710-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Sharpen.pcdroplet").to_string_lossy().into_owned();
    let out = dir.join("out").to_string_lossy().into_owned();
    let recorded = droplet(&file, &out, &[("OvrO", D::Boolean(true))]);
    let mut s = Session::new();
    // The action the droplet runs.
    s.actions.list.push(crate::actions_cmds::Action::new("Sharpen", vec![("filter.sharpen.sharpen".into(), json!({}))]));
    s.actions.list[0].set = Some("Tools".into());
    let loaded = s
        .execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&action(vec![step(b"CrtD", "Create Droplet", recorded.clone())]))}))
        .unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let a = s.actions.list.iter().find(|a| a.name == "A").unwrap();
    assert_eq!(
        a.steps,
        [("file.automate.createDroplet".to_string(), json!({"path": file, "action": "Sharpen", "set": "Tools", "output": out, "overrideOpen": true}))]
    );
    let r = s.execute("file.automate.createDroplet", a.steps[0].1.clone()).unwrap();
    assert_eq!(r["path"], json!(file));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(written["action"]["steps"], json!([["filter.sharpen.sharpen", {}]]));
    assert_eq!((written["options"]["output"].as_str(), written["options"]["overrideOpen"].as_bool()), (Some(out.as_str()), Some(true)));
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    let back = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
    let item = back.actions.iter().find(|a| a.name.to_string_lossy() == "A").unwrap().items[0].clone();
    assert_eq!((item.event.id(), item.descriptor), ("CrtD".to_string(), Some(recorded)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the droplet cannot do keeps the Photoshop step: Include All Subfolders, Save and Close,
/// no destination, Save As steps left to save where recorded, suppressed dialogs, custom naming
/// and an error log.
#[test]
fn create_droplet_steps_the_droplet_cannot_play_are_kept() {
    let kept = |d: Descriptor| matches!(steps_droplet::droplet_from(&d), Mapped::Unsupported { harmless: false });
    let base = || droplet("/d/S.pcdroplet", "/d/out", &[]);
    let replaced = |key: &str, v: D| {
        let mut d = base();
        d.items.retain(|(k, _)| !k.is(key));
        d.with(key, v)
    };
    assert!(!kept(base()), "the plain form maps");
    assert!(kept(replaced("Fldr", D::Boolean(true))));
    assert!(kept(replaced("Ovrd", D::Boolean(false))));
    let mut no_destination = base();
    no_destination.items.retain(|(k, _)| !k.is("T   "));
    assert!(kept(no_destination.clone()));
    assert!(kept(no_destination.with("SvAn", D::Boolean(true))));
    assert!(kept(base().with("suppressOpenOptions", D::Boolean(true))));
    assert!(kept(base().with("fileNamingRules", D::Descriptor(desc("fileNamingRules")))));
    assert!(kept(base().with("Log ", path_value("/d/logs"))));
}

/// Settings the event has no key for refuse the step on save rather than replay differently.
#[test]
fn create_droplet_steps_that_cannot_be_written_are_refused() {
    let p = json!({"path": "/d/S.pcdroplet", "action": "Sharpen", "output": "/d/out"});
    assert!(export_item("file.automate.createDroplet", &p, "S").is_some());
    for extra in [json!({"format": "png"}), json!({"quality": 8}), json!({"steps": [["filter.sharpen.sharpen", {}]]}), json!({"name": "Other"})] {
        let mut q = p.clone();
        q.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(export_item("file.automate.createDroplet", &q, "S").is_none(), "{q}");
    }
    let mut no_output = p.clone();
    no_output.as_object_mut().unwrap().remove("output");
    assert!(export_item("file.automate.createDroplet", &no_output, "S").is_none());
}

/// Loading a set whose action clashes with one already in the panel renames the loaded copy; its
/// Create Droplet steps follow the rename, as Batch steps do, whether the reference names the
/// action alone or the action in the file's own set (CodeRabbit review on steps_droplet.rs:59).
#[test]
fn create_droplet_steps_follow_a_renamed_loaded_action() {
    let with_ref = |items: Vec<ReferenceItem>| {
        let mut d = droplet("/d/S.pcdroplet", "/d/out", &[]);
        d.items.retain(|(k, _)| !k.is("Usng"));
        d.with("Usng", D::Reference(items))
    };
    let actn = || ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("Sharpen") };
    let own_set = ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul("Recorded") };
    let file = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Recorded"),
        actions: vec![
            AtnAction { name: UnicodeString::new_nul("Sharpen"), ..Default::default() },
            AtnAction {
                name: UnicodeString::new_nul("A"),
                items: vec![step(b"CrtD", "Create Droplet", with_ref(vec![actn()])), step(b"CrtD", "Create Droplet", with_ref(vec![actn(), own_set]))],
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .unwrap();
    let mut s = Session::new();
    s.actions.list.push(crate::actions_cmds::Action::new("Sharpen", vec![]));
    s.actions.list[0].set = Some("Tools".into());
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&file), "set": "Loaded"})).unwrap();
    assert_eq!(r["actions"], json!(["Sharpen 2", "A"]), "{r}");
    let a = s.actions.list.iter().find(|a| a.name == "A").unwrap();
    assert_eq!(a.steps[0].1["action"], "Sharpen 2", "{:?}", a.steps[0]);
    assert_eq!((&a.steps[1].1["action"], &a.steps[1].1["set"]), (&json!("Sharpen 2"), &json!("Loaded")), "{:?}", a.steps[1]);
}
