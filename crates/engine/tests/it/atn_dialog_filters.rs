use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use serde_json::{Value, json};

fn encoded(code: &[u8; 4], d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Dialog"),
            items: vec![AtnItem { event: AtnEvent::Code(*code), descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Pair measured descriptors with native params, including signed Fibers seed bits.
fn cases() -> Vec<([u8; 4], Descriptor, &'static str, Value)> {
    let kernel: Vec<i32> = (0..25).map(|i| if i == 12 { 1 } else { 0 }).collect();
    let emboss: Vec<i32> = (0..25)
        .map(|i| match i {
            11 => -1,
            13 => 1,
            _ => 0,
        })
        .collect();
    vec![
        (
            *b"Cstm",
            Descriptor::new("null")
                .with("Scl ", D::Integer(1))
                .with("Ofst", D::Integer(17))
                .with("Mtrx", D::List(kernel.iter().copied().map(D::Integer).collect())),
            "filter.other.custom",
            json!({"kernel":kernel,"scale":1,"offset":17}),
        ),
        (
            *b"Cstm",
            Descriptor::new("null")
                .with("Scl ", D::Integer(1))
                .with("Ofst", D::Integer(17))
                .with("Mtrx", D::List(emboss.iter().copied().map(D::Integer).collect())),
            "filter.other.custom",
            json!({"kernel":emboss,"scale":1,"offset":17}),
        ),
        (
            *b"Fbrs",
            Descriptor::new("null").with("Vrnc", D::Integer(64)).with("Strg", D::Integer(1)).with("RndS", D::Integer(-1)),
            "filter.render.fibers",
            json!({"variance":64,"strength":1,"seed":u32::MAX}),
        ),
    ]
}

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":7,"height":5,"depth":depth,"background":"#4080c0"})).unwrap();
    s.execute("tools.setColors", json!({"foreground":"#cc2233","background":"#112288"})).unwrap();
    s
}

/// Compare imported actions with native execution across depths, then verify descriptor save and Undo.
#[test]
fn custom_and_fibers_recordings_round_trip_replay_native_settings_and_undo() {
    for (code, d, id, p) in cases() {
        let imported = atn_cmds::import(&encoded(&code, d.clone())).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].0, id);
        let (out, skipped) = atn_cmds::export("Dialog", &[&imported.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
        for depth in [8, 16, 32] {
            let mut native = fixture(depth);
            native.execute(id, p.clone()).unwrap();
            let mut s = fixture(depth);
            let before = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
            s.actions.list = imported.actions.clone();
            assert_eq!(s.execute("actions.play", json!({"action":"Dialog"})).unwrap()["ran"], 1);
            assert_eq!(
                photocraft_compose::flatten(&s.active().unwrap().doc).px,
                photocraft_compose::flatten(&native.active().unwrap().doc).px,
                "{id} {depth}-bit"
            );
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap(), before);
        }
    }
}

/// A missing Fibers seed is captured once, then replay and Save Actions reuse the same bits.
#[test]
fn fibers_missing_seed_is_stable_across_playback_and_save() {
    let d = Descriptor::new("null").with("Vrnc", D::Integer(16)).with("Strg", D::Integer(4));
    let imported = atn_cmds::import(&encoded(b"Fbrs", d)).unwrap();
    assert!(imported.unsupported.is_empty());
    let p = &imported.actions[0].steps[0].1;
    let seed = i32::try_from(p["seed"].as_i64().unwrap()).unwrap();
    let (out, skipped) = atn_cmds::export("Dialog", &[&imported.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor.as_ref().unwrap().get("RndS"), Some(&D::Integer(seed)));
    let saved = atn_cmds::import(&out).unwrap();
    assert_eq!(saved.actions[0].steps[0].1, *p);
    let render = |actions| {
        let mut s = fixture(8);
        s.actions.list = actions;
        s.execute("actions.play", json!({"action":"Dialog"})).unwrap();
        photocraft_compose::flatten(&s.active().unwrap().doc).px.clone()
    };
    assert_eq!(render(imported.actions.clone()), render(saved.actions));
}

/// Default materialization remains native; unrepresented metadata stays on the preserved path.
#[test]
fn fibers_preserves_noncanonical_descriptors_without_rejecting_missing_defaults() {
    for d in [Descriptor::new("null"), Descriptor::new("null").with("RndS", D::Integer(-1))] {
        let imported = atn_cmds::import(&encoded(b"Fbrs", d)).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].0, "filter.render.fibers");
        let (out, skipped) = atn_cmds::export("Dialog", &[&imported.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        let saved = atn_cmds::import(&out).unwrap();
        assert_eq!(saved.actions[0].steps, imported.actions[0].steps);
    }
    let canonical = Descriptor::new("null").with("Vrnc", D::Integer(16)).with("Strg", D::Integer(4)).with("RndS", D::Integer(7));
    let mut named = canonical.clone();
    named.name = UnicodeString(vec![0xd800, 0]);
    let mut class = canonical.clone();
    class.class_id = Id::Str(b"null".to_vec());
    let mut ordered = canonical.clone();
    ordered.items.swap(0, 1);
    let mut key = canonical;
    key.items[0].0 = Id::Str(b"Vrnc".to_vec());
    for d in [named, class, ordered, key] {
        let imported = atn_cmds::import(&encoded(b"Fbrs", d.clone())).unwrap();
        assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let (out, _) = atn_cmds::export("Dialog", &[&imported.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
    }
}

/// Malformed and unsupported forms remain complete steps instead of being partially interpreted.
#[test]
fn dialog_filters_keep_unrepresented_types_ranges_and_fields_whole() {
    for (code, d, _, _) in cases() {
        let changes = if code == *b"Cstm" {
            vec![("Scl ", D::Integer(0)), ("Ofst", D::Integer(10000)), ("Mtrx", D::List(vec![D::Integer(0); 24])), ("Scl ", D::Double(1.5))]
        } else {
            vec![("Vrnc", D::Integer(0)), ("Strg", D::Integer(65)), ("RndS", D::Double(1.0))]
        };
        for (key, value) in changes.into_iter().chain([("unknown", D::Boolean(true))]) {
            let mut odd = d.clone();
            if let Some((_, slot)) = odd.items.iter_mut().find(|(k, _)| k.is(key)) {
                *slot = value;
            } else {
                odd.items.push((Id::new(key), value));
            }
            let a = atn_cmds::import(&encoded(&code, odd.clone())).unwrap();
            assert_eq!(a.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
            let (out, _) = atn_cmds::export("Dialog", &[&a.actions[0]], &[], false).unwrap();
            assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(odd));
        }
    }
    for (code, d, _, _) in cases() {
        let a = atn_cmds::import(&encoded(&code, d)).unwrap();
        let key = if code == *b"Cstm" { "scale" } else { "variance" };
        for value in [json!(0), json!(10000), json!(1.5)] {
            let mut edited = a.actions[0].clone();
            edited.steps[0].1[key] = value;
            assert!(atn_cmds::export("Dialog", &[&edited], &[], false).is_err());
        }
    }
}
