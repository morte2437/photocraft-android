//! Name of the Remove Tool's result layer (#12542): with Create new layer on, the first is
//! "Remove tool edits", the next "Remove tool edits 1", "Remove tool edits 2"…; placement, the
//! active layer and the pixels are as before, and with it off no layer is added.

use super::*;

/// A flat grey layer with a dark square at 40..52 × 24..36.
fn square_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 64, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if (40..52).contains(&x) && (24..36).contains(&y) { 0.05 } else { 0.5 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

const STROKE: [[i32; 2]; 2] = [[38, 30], [54, 30]];

/// One stroke with Create new layer. Sample All Layers, as a result layer filled from its own
/// transparent surroundings would leave the next stroke on it nothing to remove.
fn remove_to_new_layer(s: &mut Session) -> LayerId {
    let r = s.execute("paint.remove", json!({"points": STROKE, "size": 16, "newLayer": true, "sampleAllLayers": true})).unwrap();
    LayerId(r["layer"].as_u64().unwrap_or_else(|| panic!("no new layer: {r}")))
}

fn name(s: &Session, id: LayerId) -> String {
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

fn order(s: &Session) -> Vec<LayerId> {
    s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).collect()
}

fn rename(s: &mut Session, id: LayerId, to: &str) {
    s.edit("rename", |doc, _| {
        doc.layer_mut(id).unwrap().name = to.to_string();
        Ok(())
    })
    .unwrap();
}

#[test]
fn successive_strokes_name_remove_tool_edits_then_1_then_2_each_above_the_active_layer() {
    let mut s = square_session();
    let source = s.active().unwrap().active_layer.unwrap();
    let before = s.active().unwrap().doc.layer(source).unwrap().surface().unwrap().rgba(46, 30);
    let mut stack = vec![source];
    for want in ["Remove tool edits", "Remove tool edits 1", "Remove tool edits 2"] {
        let nid = remove_to_new_layer(&mut s);
        stack.push(nid);
        assert_eq!(name(&s, nid), want);
        assert_eq!(s.active().unwrap().active_layer, Some(nid), "{want}: the new layer is active");
        assert_eq!(order(&s), stack, "{want}: a new layer directly above the then active one");
    }
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(source).unwrap().surface().unwrap().rgba(46, 30), before, "the source layer is unchanged");
}

#[test]
fn two_strokes_without_sample_all_layers_name_remove_tool_edits_then_1() {
    let mut s = square_session();
    let source = s.active().unwrap().active_layer.unwrap();
    let mut stack = vec![source];
    for want in ["Remove tool edits", "Remove tool edits 1"] {
        let r = s.execute("paint.remove", json!({"points": STROKE, "size": 16, "newLayer": true})).unwrap();
        let nid = LayerId(r["layer"].as_u64().unwrap_or_else(|| panic!("no new layer: {r}")));
        stack.push(nid);
        assert_eq!(name(&s, nid), want);
        assert_eq!(s.active().unwrap().active_layer, Some(nid), "{want}: the new layer is active");
        assert_eq!(order(&s), stack, "{want}: a new layer directly above the then active one");
    }
}

#[test]
fn undo_frees_the_name_for_the_next_stroke() {
    let mut s = square_session();
    let source = s.active().unwrap().active_layer.unwrap();
    let nid = remove_to_new_layer(&mut s);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(nid).is_none(), "undo removes the layer");
    assert_eq!(s.active().unwrap().active_layer, Some(source));
    let again = remove_to_new_layer(&mut s);
    assert_eq!(name(&s, again), "Remove tool edits");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(name(&s, again), "Remove tool edits", "redo brings the layer back with its name");
}

#[test]
fn a_taken_name_gives_the_first_free_one() {
    let mut s = square_session();
    let source = s.active().unwrap().active_layer.unwrap();
    rename(&mut s, source, "Remove tool edits");
    let nid = remove_to_new_layer(&mut s);
    assert_eq!(name(&s, nid), "Remove tool edits 1");

    // A gap left by a renamed result layer is filled first.
    let mut s = square_session();
    let first = remove_to_new_layer(&mut s);
    let second = remove_to_new_layer(&mut s);
    assert_eq!(name(&s, second), "Remove tool edits 1");
    rename(&mut s, second, "Kept");
    let nid = remove_to_new_layer(&mut s);
    assert_eq!(name(&s, nid), "Remove tool edits 1");
    rename(&mut s, first, "Other");
    let nid = remove_to_new_layer(&mut s);
    assert_eq!(name(&s, nid), "Remove tool edits");
}

#[test]
fn without_create_new_layer_the_fill_goes_into_the_active_layer() {
    let mut s = square_session();
    let source = s.active().unwrap().active_layer.unwrap();
    let r = s.execute("paint.remove", json!({"points": STROKE, "size": 16})).unwrap();
    assert!(r.get("layer").is_none(), "{r}");
    assert_eq!(order(&s), vec![source], "no new layer");
    let d = s.active().unwrap();
    assert_eq!(d.active_layer, Some(source));
    let px = d.doc.layer(source).unwrap().surface().unwrap().rgba(46, 30);
    assert!((px[0] - 0.5).abs() < 0.03, "the square is removed in place: {px:?}");
}
