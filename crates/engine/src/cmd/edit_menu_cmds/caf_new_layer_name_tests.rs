//! #14192: Content-Aware Fill's New Layer output is named like the Duplicate Layer output,
//! "<layer> copy" (`Document::copy_name`), and keeps its bounds, stacking and active layer.

use super::*;

/// A 64 x 48 document with a striped Background and a selection over a red blob.
fn caf_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8, "background": "white"})).unwrap();
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in (0..64).step_by(8) {
            surf.fill_rect(Rect::new(x, 0, x + 3, 48), &[0.0, 0.0, 0.0, 1.0]);
        }
        surf.fill_rect(Rect::new(28, 18, 36, 28), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 26, "y": 16, "width": 12, "height": 14})).unwrap();
    s
}

fn names(s: &Session) -> Vec<String> {
    s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

const NEW: fn() -> Value = || json!({"output": "new", "sampling": "rectangular", "margin": 12, "colorAdaptation": "none"});

fn layer_id(r: &Value) -> LayerId {
    LayerId(r["layer"].as_u64().unwrap())
}

fn name_of(s: &Session, id: LayerId) -> String {
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

#[test]
fn new_layer_output_on_the_background_is_named_background_copy() {
    let mut s = caf_session();
    let base = s.active().unwrap().active_layer.unwrap();
    let before = names(&s);
    let nid = layer_id(&s.execute("edit.contentAwareFill", NEW()).unwrap());
    assert_eq!(name_of(&s, nid), "Background copy");
    // Stacking: directly above the source, and it is the active layer.
    assert_eq!(s.active().unwrap().active_layer, Some(nid));
    assert_ne!(nid, base);
    let after = names(&s);
    assert_eq!(after.len(), before.len() + 1);
    let pos = |n: &str| after.iter().position(|x| x == n).unwrap();
    assert_eq!(pos("Background copy").abs_diff(pos("Background")), 1);
    // Bounds: only the fill, inside the selection's window.
    let b = s.active().unwrap().doc.layer(nid).unwrap().surface().unwrap().content_bounds();
    assert!(Rect::new(26, 16, 38, 30).contains_rect(&b), "{b:?}");
}

#[test]
fn new_layer_output_follows_the_source_layers_name() {
    let mut s = caf_session();
    let r = s.execute("layer.new.layer", json!({"name": "Fill"})).unwrap();
    let fill = layer_id(&r);
    let mut p = NEW();
    p["layer"] = json!(fill.0);
    let nid = layer_id(&s.execute("edit.contentAwareFill", p).unwrap());
    assert_eq!(name_of(&s, nid), "Fill copy");
}

#[test]
fn an_existing_copy_gives_the_next_copy_name_like_the_duplicate_output() {
    let mut s = caf_session();
    let bg = s.active().unwrap().active_layer.unwrap();
    let first = layer_id(&s.execute("edit.contentAwareFill", NEW()).unwrap());
    assert_eq!(name_of(&s, first), "Background copy");
    // The source is the Background again; "Background copy" is taken.
    let expected = s.active().unwrap().doc.copy_name("Background");
    assert_eq!(expected, "Background copy 2");
    let mut p = NEW();
    p["layer"] = json!(bg.0);
    let second = layer_id(&s.execute("edit.contentAwareFill", p).unwrap());
    assert_eq!(name_of(&s, second), expected);
    // The Duplicate Layer output on the same state gives the same name.
    s.undo();
    let r = s
        .execute("edit.contentAwareFill", json!({"layer": bg.0, "output": "duplicate", "sampling": "custom", "area": [0, 0, 20, 48], "mirror": true}))
        .unwrap();
    assert_eq!(name_of(&s, layer_id(&r)), expected);
}
