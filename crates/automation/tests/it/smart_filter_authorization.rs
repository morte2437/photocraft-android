//! Stored smart-filter parameters must not grant automation ambient filesystem access.
use std::path::Path;
use std::sync::Arc;

#[path = "../support/smart_filter_fixture.rs"]
mod fixture;

use fixture::{Fixture, assert_denied, cube, path_params};
use photocraft_automation::{Headless, workspace::authorize_desktop_engine_step};
use photocraft_doc::LayerContent;
use photocraft_engine::{Session, actions_cmds::Action, jobs::Started, smart_cmds};
use serde_json::json;

#[test]
fn parameter_edits_reject_outside_images_and_luts_without_changing_the_document() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (mut h, layer) = f.setup(lookup);
        let before = h.render_png(None, 64).unwrap();
        let doc = h.session.active().unwrap().doc.clone();
        let journal = h.session.journal.len();
        for name in if lookup { ["black.cube", "white.cube"] } else { ["black.png", "white.png"] } {
            let params = path_params(lookup, &f.outside(name));
            let id = if lookup { "image.adjustments.colorLookup" } else { "filter.distort.displace" };
            assert_denied(h.session.execute(id, params.clone()));
            assert_denied(h.handle("engine.execute", json!({"command": "layer.smartFilter.setParams", "params": {"layer": layer.0, "params": params}})));
            assert!(Arc::ptr_eq(&doc, &h.session.active().unwrap().doc));
            assert_eq!(h.session.journal.len(), journal);
            assert_eq!(h.render_png(None, 64).unwrap(), before);
        }
        // Safe numeric and embedded-data edits remain usable.
        let safe = if lookup { json!({"data": cube(0), "fileName": "embedded.cube"}) } else { json!({"horizontal": 10}) };
        h.session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "params": safe})).unwrap();
    }
}

#[test]
fn action_playback_cannot_hide_a_smart_filter_path_edit() {
    let f = Fixture::new();
    let (mut h, layer) = f.setup(false);
    let doc = h.session.active().unwrap().doc.clone();
    h.session.actions.list.push(Action::new(
        "edit filter",
        vec![("layer.smartFilter.setParams".into(), json!({"layer": layer.0, "params": path_params(false, &f.outside("white.png"))}))],
    ));
    let result = h.session.execute("actions.play", json!({"action": "edit filter"})).unwrap();
    assert!(result["failed"]["error"].as_str().unwrap().contains("ambient filesystem"), "{result}");
    assert!(Arc::ptr_eq(&doc, &h.session.active().unwrap().doc));
}

#[test]
fn loaded_filters_are_checked_after_merging_even_when_the_edit_contains_no_path() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (_, layer) = f.saved_filter(lookup);
        let mut h = f.headless();
        h.open(Path::new("stored.pcraft")).unwrap();
        let doc = h.session.active().unwrap().doc.clone();
        assert_denied(h.session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "params": {"horizontal": 10, "dither": false}})));
        assert!(Arc::ptr_eq(&doc, &h.session.active().unwrap().doc));
        // Removing an ambient input makes the filter usable again.
        let safe = if lookup { json!({"file": "", "lut": "none"}) } else { json!({"mapPath": ""}) };
        h.session.execute("layer.smartFilter.setParams", json!({"layer": layer.0, "params": safe})).unwrap();
    }
}

#[test]
fn loaded_filters_cannot_read_on_rerender_or_reuse_a_trusted_cached_stage() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (doc, layer) = f.saved_filter(lookup);
        for warm_cache in [false, true] {
            if warm_cache {
                let mut trusted = doc.clone();
                assert!(smart_cmds::refresh(&mut trusted, layer).unwrap());
            }
            let mut h = f.headless();
            h.open(Path::new("stored.pcraft")).unwrap();
            let before = h.session.active().unwrap().doc.clone();
            assert_denied(h.session.execute("layer.smartFilter.setVisible", json!({"layer": layer.0, "visible": true})));
            assert!(Arc::ptr_eq(&before, &h.session.active().unwrap().doc));
        }
    }
}

#[test]
fn background_rerender_keeps_the_desktop_request_authority() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (doc, layer) = f.saved_filter(lookup);
        let mut s = Session::new();
        s.add_document(doc, None);
        s.authorize = Some(authorize_desktop_engine_step);
        let before = s.active().unwrap().doc.clone();
        let started = s.start("layer.smartFilter.setVisible", json!({"layer": layer.0, "visible": true})).unwrap();
        // The desktop restores UI authority as soon as the automation request returns.
        s.authorize = None;
        let Started::Job(job) = started else { panic!("expected a background render") };
        assert_denied(s.wait_job(job));
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        // The worker's restriction must not leak into subsequent trusted UI work.
        s.execute("layer.smartFilter.setVisible", json!({"layer": layer.0, "visible": true})).unwrap();
    }
}

#[test]
fn transferring_stored_filters_cannot_read_outside_files() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (_, from) = f.saved_filter(lookup);
        let mut h = f.headless();
        h.open(Path::new("stored.pcraft")).unwrap();
        let to = h.handle("doc.place", json!({"path": "stripes.png"})).unwrap()["layer"].as_u64().unwrap();
        let before = h.session.active().unwrap().doc.clone();
        for copy in [false, true] {
            assert_denied(h.session.execute("layer.smartFilter.transfer", json!({"from": from.0, "to": to, "copy": copy})));
            assert!(Arc::ptr_eq(&before, &h.session.active().unwrap().doc));
        }
    }
}

#[test]
fn trusted_parameter_edits_still_read_maps_and_luts_after_a_denied_request() {
    for lookup in [false, true] {
        let f = Fixture::new();
        let (mut h, layer) = f.setup(lookup);
        let edit = |path: &Path| json!({"layer": layer.0, "params": path_params(lookup, path)});
        assert_denied(h.session.execute("layer.smartFilter.setParams", edit(&f.outside("white.png"))));
        let mut trusted = Headless::trusted_local();
        trusted.session.add_document((*h.session.active().unwrap().doc).clone(), None);
        let mut renders = Vec::new();
        for name in if lookup { ["black.cube", "white.cube", "black.cube"] } else { ["black.png", "white.png", "black.png"] } {
            trusted.session.execute("layer.smartFilter.setParams", edit(&f.outside(name))).unwrap();
            renders.push(trusted.render_png(None, 64).unwrap());
        }
        assert_ne!(renders[0], renders[1], "trusted path-based filter still works");
        assert_eq!(renders[0], renders[2], "trusted rendering remains deterministic");
    }
}

#[test]
fn a_refresh_outside_command_execution_surfaces_the_denial_instead_of_rendering_unfiltered() {
    // link-watch and transform paths re-render through Session::edit, not Session::execute or
    // jobs::run; the gate must reach them and the denial must not silently pass pixels through
    // into the process-wide stack cache.
    for lookup in [false, true] {
        let f = Fixture::new();
        let (doc, layer) = f.saved_filter(lookup);
        let mut h = f.headless();
        h.open(Path::new("stored.pcraft")).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        // Session::edit is the outside-execute mutation path (an edit closure the UI or a
        // linked-file change runs); the stored filter must be denied there too.
        assert_denied(h.session.edit("Re-render", |d, _| match smart_cmds::refresh(d, layer) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }));
        assert!(Arc::ptr_eq(&before, &h.session.active().unwrap().doc));
        // A trusted session (no gate) still renders the same document.
        let mut trusted = Session::new();
        trusted.add_document(doc, None);
        let mut d = (*trusted.active().unwrap().doc).clone();
        assert!(smart_cmds::refresh(&mut d, layer).is_ok());
    }
}

/// Choosing another Displace map through `setParams` embeds it (#7423), so the new map is read
/// before the edit: a path outside the workspace is refused before that read, and the filter
/// keeps the map it had.
#[test]
fn a_new_displace_map_outside_the_workspace_is_refused_before_it_is_read() {
    let f = Fixture::new();
    let (mut h, layer) = f.setup(false);
    let params = |h: &Headless| {
        let d = h.session.active().unwrap();
        let LayerContent::Smart(sm) = &d.doc.layer(layer).unwrap().content else { panic!("smart object") };
        sm.smart_filters[0].params.clone()
    };
    let before = params(&h);
    let map = photocraft_io::smart_map::displace_embedded_map(&before).expect("the open document's map, embedded");
    assert!(map.data.iter().all(|v| *v < 0.9), "the grey map");
    let doc = h.session.active().unwrap().doc.clone();
    let edit = json!({"layer": layer.0, "params": {"mapPath": f.outside("white.png")}});
    assert_denied(h.session.execute("layer.smartFilter.setParams", edit.clone()));
    assert_denied(h.handle("engine.execute", json!({"command": "layer.smartFilter.setParams", "params": edit})));
    assert!(Arc::ptr_eq(&doc, &h.session.active().unwrap().doc));
    assert_eq!(params(&h), before, "no white map was read into the filter");
}
