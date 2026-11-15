//! Exercise Average through the smart-filter renderer, not an imported cache.
use photocraft_doc::LayerContent;
use photocraft_engine::{Session, filters, smart_cmds};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

fn image(s: &Session) -> Surface {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match &l.content {
        LayerContent::Raster(r) => r.clone(),
        LayerContent::Smart(sm) => sm.cache.clone().unwrap(),
        _ => panic!("expected pixels"),
    }
}

fn input(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let d = s.active_mut().unwrap();
    let l = std::sync::Arc::make_mut(&mut d.doc).layer_mut(d.active_layer.unwrap()).unwrap();
    let surf = l.surface_mut().unwrap();
    for y in 0..16 {
        for x in 0..16 {
            surf.write_region(Rect::new(x, y, x + 1, y + 1), if x < 8 { &[1.0, 0.0, 0.0, 1.0] } else { &[0.0, 0.0, 1.0, 1.0] });
        }
    }
    s
}

fn purple(surf: &Surface) {
    for p in surf.read_region(Rect::new(0, 0, 16, 16)).as_chunks::<4>().0 {
        assert!((p[0] - 0.5).abs() <= 1.0 / 255.0, "{p:?}");
        assert_eq!(p[1], 0.0);
        assert!((p[2] - 0.5).abs() <= 1.0 / 255.0, "{p:?}");
        assert_eq!(p[3], 1.0);
    }
}

#[test]
fn average_stored_filter_executes() {
    for depth in [8, 16, 32] {
        let s = input(depth);
        let bounds = s.active().unwrap().doc.bounds();
        let out = filters::apply_filter_to_surface("filter.blur.average", &json!({}), &image(&s), bounds, None, bounds)
            .unwrap()
            .expect("Average must be executable, not treated as an unknown filter");
        purple(&out);
    }
}

#[test]
fn average_smart_command_rerenders_and_undoes() {
    for depth in [8, 16, 32] {
        let mut s = input(depth);
        s.execute("filter.convertForSmartFilters", json!({})).unwrap();
        let before = image(&s);
        let steps = s.active().unwrap().history.past_len();
        assert!(s.is_enabled("filter.blur.average"));
        s.execute("filter.blur.average", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        purple(&image(&s));
        assert!(s.undo());
        assert_eq!(image(&s), before);
        assert!(s.redo());
        purple(&image(&s));
        s.execute("layer.smartFilter.setVisible", json!({"index": 0, "visible": false})).unwrap();
        assert_eq!(image(&s), before);
        s.execute("layer.smartFilter.setVisible", json!({"index": 0, "visible": true})).unwrap();
        let d = s.active_mut().unwrap();
        let snapshot = d.doc.clone();
        let layer = std::sync::Arc::make_mut(&mut d.doc).layer_mut(d.active_layer.unwrap()).unwrap();
        if let LayerContent::Smart(sm) = &mut layer.content {
            assert_eq!(sm.smart_filters.len(), 1);
            assert_eq!(sm.smart_filters[0].command, "filter.blur.average");
            sm.cache = None;
        }
        smart_cmds::purge_stack_cache();
        assert!(smart_cmds::refresh_layer(&snapshot, layer).unwrap());
        purple(&image(&s));
    }
}

#[test]
fn average_rejects_bad_params_without_an_edit() {
    let mut s = input(8);
    let before = image(&s);
    let steps = s.active().unwrap().history.past_len();
    for params in [json!("bad"), json!({"unknown": 1}), json!({"layer": -1}), json!({"target": false})] {
        assert!(s.execute("filter.blur.average", params).is_err());
        assert_eq!(image(&s), before);
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }
}

/// Optional local oracle: generate a 16×16 red/blue Smart Object in Photoshop, apply Average,
/// save as PSD, then set PHOTOCRAFT_AVERAGE_PSD to that file. Never accept cached pixels as proof.
#[test]
#[ignore = "requires a locally generated Photoshop Average PSD; see docs/average-filter.md"]
fn average_photoshop_psd_rerenders_from_source() {
    let path = std::env::var("PHOTOCRAFT_AVERAGE_PSD").expect("set PHOTOCRAFT_AVERAGE_PSD");
    let bytes = std::fs::read(&path).unwrap();
    let mut doc = photocraft_io::import(&path, &bytes).unwrap().document;
    let snapshot = doc.clone();
    let ids: Vec<_> = snapshot
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(sm) if sm.smart_filters.iter().any(|f| f.command == "filter.blur.average") => Some(l.id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 1);
    let layer = doc.layer_mut(ids[0]).unwrap();
    if let LayerContent::Smart(sm) = &mut layer.content {
        sm.cache = None;
    }
    smart_cmds::purge_stack_cache();
    assert!(smart_cmds::refresh_layer(&snapshot, layer).unwrap());
    let LayerContent::Smart(sm) = &layer.content else { panic!("expected smart object") };
    purple(sm.cache.as_ref().unwrap());
}
