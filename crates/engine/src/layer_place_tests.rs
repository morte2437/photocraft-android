use photocraft_doc::{Document, Layer, LayerContent, LayerId, MAX_GROUP_DEPTH};
use serde_json::{Value, json};

use super::*;
use crate::Session;

/// Background, then `A` and `B` (`B` active).
fn session() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let a = id(s.execute("layer.new.layer", json!({"name": "A"})).unwrap());
    let b = id(s.execute("layer.new.layer", json!({"name": "B"})).unwrap());
    (s, a, b)
}

fn id(v: Value) -> LayerId {
    LayerId(v["layer"].as_u64().unwrap())
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn top(s: &Session) -> Vec<LayerId> {
    doc(s).layers.iter().map(|l| l.id).collect()
}

fn children(s: &Session, group: LayerId) -> Vec<LayerId> {
    doc(s).layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect()
}

fn set_expanded(s: &mut Session, group: LayerId, expanded: bool) {
    s.execute("layer.setExpanded", json!({"layer": group.0, "expanded": expanded})).unwrap();
}

fn select(s: &mut Session, layer: LayerId) {
    s.execute("layer.select", json!({"layer": layer.0})).unwrap();
}

#[test]
fn a_new_layer_goes_inside_an_open_active_group_and_above_a_closed_one() {
    let (mut s, a, b) = session();
    select(&mut s, a);
    let g = id(s.execute("layer.groupLayers", json!({})).unwrap());
    // A group made from layers starts closed: the new layer goes above it.
    let above = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(top(&s)[1..], [g, above, b]);
    assert_eq!(children(&s, g), vec![a]);
    // Opened, it takes the new layer as its top layer.
    select(&mut s, g);
    set_expanded(&mut s, g, true);
    let inside = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![a, inside]);
    // A new empty group is open, so the next new layer goes into it (New Group, then New Layer).
    s.execute("select.deselectLayers", json!({})).unwrap();
    let empty = id(s.execute("layer.new.group", json!({})).unwrap());
    let first = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(children(&s, empty), vec![first]);
    // The same goes for the other new layers: adjustment, fill, type and shape layers.
    select(&mut s, empty);
    for (cmd, p) in [
        ("layer.newAdjustmentLayer.invert", json!({})),
        ("layer.newFillLayer.solidColor", json!({"color": "#ff0000"})),
        ("type.create", json!({"x": 2, "y": 10, "text": "Hi"})),
        ("shape.create", json!({"kind": "rect", "rect": [1, 1, 5, 5]})),
    ] {
        select(&mut s, empty);
        let l = id(s.execute(cmd, p).unwrap());
        assert_eq!(children(&s, empty).last(), Some(&l), "{cmd} goes inside the open group");
    }
}

#[test]
fn artboards_and_locked_groups_take_new_layers() {
    let (mut s, _, b) = session();
    let board = id(s.execute("layer.new.artboard", json!({"rect": [0, 0, 16, 16]})).unwrap());
    set_expanded(&mut s, board, false);
    select(&mut s, board);
    let l = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(doc(&s).artboard_of(l), Some(board), "an artboard takes the layer even when closed");
    // Photoshop puts a new layer inside an open group locked with Lock All too; the layer is
    // locked through the group.
    select(&mut s, b);
    let g = id(s.execute("layer.new.group", json!({})).unwrap());
    s.execute("layer.setProps", json!({"layer": g.0, "locks": {"all": true}})).unwrap();
    select(&mut s, g);
    let inside = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![inside]);
    assert!(doc(&s).effective_locks(inside).all);
}

#[test]
fn below_puts_the_new_layer_under_the_active_layer() {
    let (mut s, a, b) = session();
    let bg = top(&s)[0];
    let l = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert_eq!(top(&s), vec![bg, a, l, b]);
    // Never under the Background, and never inside an open group.
    select(&mut s, bg);
    let l2 = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert_eq!(top(&s)[..2], [bg, l2]);
    let g = id(s.execute("layer.new.group", json!({})).unwrap());
    let l3 = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert!(children(&s, g).is_empty());
    let root = top(&s);
    assert_eq!(root.iter().position(|x| *x == l3).unwrap() + 1, root.iter().position(|x| *x == g).unwrap());
    assert!(s.execute("layer.new.layer", json!({"position": "yes"})).is_err());
}

/// The clipping rule Photoshop follows on every insert (#2203): a new layer is clipped exactly
/// when the layer just above its slot is clipped.
#[test]
fn a_new_layer_joins_the_clipping_mask_it_lands_in() {
    let (mut s, base, c1) = session();
    let c2 = id(s.execute("layer.new.layer", json!({"name": "C2"})).unwrap());
    for c in [c1, c2] {
        s.execute("layer.setProps", json!({"layer": c.0, "clipped": true})).unwrap();
    }
    let clipped = |s: &Session, l: LayerId| doc(s).layer(l).unwrap().clipped;
    // On the base: between the base and its clipped layers.
    select(&mut s, base);
    let on_base = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(clipped(&s, on_base));
    // In the middle of the chain.
    select(&mut s, c1);
    let mid = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(clipped(&s, mid));
    // Above the top clipped layer it is outside the mask.
    select(&mut s, c2);
    let above = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert!(!clipped(&s, above));
    // ⌘-click New Layer under a clipped layer joins it too; under the base it does not.
    select(&mut s, c2);
    let l = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert!(clipped(&s, l));
    select(&mut s, base);
    let l = id(s.execute("layer.new.layer", json!({"position": "below"})).unwrap());
    assert!(!clipped(&s, l));
    // A group joins as well (Photoshop clips groups too), and so do the other new layers.
    select(&mut s, base);
    let l = id(s.execute("layer.new.group", json!({})).unwrap());
    assert!(clipped(&s, l));
    select(&mut s, base);
    let l = id(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap());
    assert!(clipped(&s, l));
    // A layer made from a clipped layer (Layer via Copy) stays in its mask.
    select(&mut s, c1);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    let l = id(s.execute("layer.new.layerViaCopy", json!({})).unwrap());
    assert!(clipped(&s, l));
}

#[test]
fn a_pasted_layer_goes_into_an_open_group() {
    let (mut s, a, _) = session();
    // A whole pixel layer on the clipboard (no selection).
    select(&mut s, a);
    s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let into = id(s.execute("layer.new.group", json!({"name": "Into"})).unwrap());
    let pasted = id(s.execute("edit.paste", json!({})).unwrap());
    assert_eq!(children(&s, into), vec![pasted]);
}

/// The second and later of several new layers stack above the first, even when the first is an
/// open group: it must not swallow them.
#[test]
fn later_layers_of_a_batch_go_above_the_first() {
    let (s, _, b) = session();
    let mut d = doc(&s).clone();
    let first = insert_new(&mut d, Some(b), Layer::group("open", vec![]), Place::Auto);
    let next = Layer::raster("next", d.pixel_format());
    let second = insert_new(&mut d, Some(first), next, Place::Above);
    let root: Vec<LayerId> = d.layers.iter().map(|l| l.id).collect();
    assert_eq!(root[root.len() - 2..], [first, second]);
    assert!(d.layer(first).unwrap().children().unwrap().is_empty());
}

#[test]
fn nesting_stops_at_the_group_depth_limit() {
    let mut doc = Document::new("deep", photocraft_doc::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    // A chain of open groups as deep as allowed, with a layer at the bottom.
    let mut inner = Layer::raster("deep", doc.pixel_format());
    let deep_id = inner.id;
    let mut ids = Vec::new();
    for i in 0..MAX_GROUP_DEPTH {
        let g = Layer::group(format!("g{i}"), vec![inner]);
        ids.push(g.id);
        inner = g;
    }
    doc.layers.push(inner);
    assert_eq!(doc.max_group_depth(), MAX_GROUP_DEPTH);
    let innermost = *ids.first().unwrap();
    // A plain layer fits beside `deep`, inside the innermost group (the depth stays the same)…
    let layer = Layer::raster("fits", doc.pixel_format());
    let l = insert_new(&mut doc, Some(innermost), layer, Place::Auto);
    assert_eq!(doc.max_group_depth(), MAX_GROUP_DEPTH);
    assert!(doc.layer(innermost).unwrap().children().unwrap().iter().any(|c| c.id == l));
    // …but a group with a layer in it would go one deeper: it goes beside the group instead.
    let nested = Layer::group("nested", vec![Layer::raster("x", doc.pixel_format())]);
    let n = insert_new(&mut doc, Some(innermost), nested, Place::Auto);
    assert_eq!(doc.max_group_depth(), MAX_GROUP_DEPTH);
    assert!(!doc.layer(innermost).unwrap().children().unwrap().iter().any(|c| c.id == n));
    assert!(doc.layer(deep_id).is_some());
}

#[test]
fn a_missing_or_stale_anchor_means_the_top_of_the_stack() {
    let (s, _, _) = session();
    let mut d = doc(&s).clone();
    for anchor in [None, Some(LayerId(u64::MAX))] {
        for place in [Place::Auto, Place::Above, Place::Below] {
            let layer = Layer::raster("x", d.pixel_format());
            let l = insert_new(&mut d, anchor, layer, place);
            assert_eq!(d.layers.last().map(|x| x.id), Some(l), "{anchor:?} {place:?}");
        }
    }
    assert!(matches!(d.layers.last().unwrap().content, LayerContent::Raster(_)));
}

/// Any lock on an open group still lets New Layer, New Group and new adjustment and fill layers in,
/// as in Photoshop (checked with Lock All). A layer that is edited right after it is made (Type,
/// Shape, Paste, Render…) goes above a group whose lock would refuse that edit,
/// so it never comes out locked.
#[test]
fn locked_groups_take_new_layers_but_not_the_ones_edited_at_once() {
    for locks in [json!({"all": true}), json!({"pixels": true}), json!({"transparency": true}), json!({"position": true})] {
        let (mut s, _, b) = session();
        select(&mut s, b);
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let g = id(s.execute("layer.new.group", json!({})).unwrap());
        s.execute("layer.setProps", json!({"layer": g.0, "locks": locks})).unwrap();
        for (cmd, p) in [
            ("layer.new.layer", json!({})),
            ("layer.new.group", json!({})),
            ("layer.newAdjustmentLayer.invert", json!({})),
            ("layer.newFillLayer.solidColor", json!({"color": "#ff0000"})),
        ] {
            select(&mut s, g);
            let l = id(s.execute(cmd, p).unwrap());
            assert_eq!(children(&s, g).last(), Some(&l), "{cmd} goes inside the group locked with {locks}");
        }
        for (cmd, p) in [
            ("type.create", json!({"x": 2, "y": 10, "text": "Hi"})),
            ("shape.create", json!({"kind": "rect", "rect": [1, 1, 5, 5]})),
            ("edit.paste", json!({})),
            ("filter.render.tree", json!({})),
            ("gradient.fill.create", json!({"from": [0, 0], "to": [16, 16]})),
        ] {
            select(&mut s, g);
            let l = id(s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd} with {locks}: {e}")));
            assert!(!children(&s, g).contains(&l), "{cmd} stays out of the group locked with {locks}");
            assert_eq!(top(&s).iter().position(|x| *x == l), top(&s).iter().position(|x| *x == g).map(|i| i + 1), "{cmd} directly above the group");
            assert!(!doc(&s).effective_locks(l).any(), "{cmd}: the new layer is not locked");
            if cmd == "filter.render.tree" {
                assert!(doc(&s).layer(l).unwrap().surface().unwrap().tile_count() > 0, "Render drew the tree with {locks}");
            }
        }
    }
    // An open group inside a locked one still takes New Layer (the lock reaches it through the outer group).
    let (mut s, _, b) = session();
    select(&mut s, b);
    let outer = id(s.execute("layer.new.group", json!({})).unwrap());
    let inner = id(s.execute("layer.new.group", json!({})).unwrap());
    assert_eq!(children(&s, outer), vec![inner]);
    s.execute("layer.setProps", json!({"layer": outer.0, "locks": {"pixels": true}})).unwrap();
    select(&mut s, inner);
    let plain = id(s.execute("layer.new.layer", json!({})).unwrap());
    assert_eq!(children(&s, inner), vec![plain]);
}

/// Stamp Visible goes directly above an open active group, never inside it: the stamp is the
/// composite already, and inside it the group's opacity, mode or mask would apply to it again.
/// With a layer inside a group active, it still lands above that layer, inside the group.
#[test]
fn stamp_visible_goes_above_an_open_group() {
    let (mut s, a, b) = session();
    select(&mut s, a);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let g = id(s.execute("layer.groupLayers", json!({})).unwrap());
    set_expanded(&mut s, g, true);
    s.execute("layer.setProps", json!({"layer": g.0, "opacity": 0.5})).unwrap();
    select(&mut s, g);
    let stamp = id(s.execute("layer.stampVisible", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![a], "the stamp stays out of the group");
    assert_eq!(top(&s)[1..], [g, stamp, b]);
    // A layer inside the group active: the stamp goes above it, inside the group.
    select(&mut s, a);
    let inside = id(s.execute("layer.stampVisible", json!({})).unwrap());
    assert_eq!(children(&s, g), vec![a, inside]);
}

/// An artboard takes a new layer whose content is centred on it, and a new empty layer; content
/// placed elsewhere (Place and Paste centre on the canvas) stays at the top level instead of
/// being clipped away by the board.
#[test]
fn an_artboard_takes_only_content_that_is_on_it() {
    let (mut s, _, _) = session();
    let board = id(s.execute("layer.new.artboard", json!({"rect": [0, 0, 8, 8]})).unwrap());
    let mut d = doc(&s).clone();
    let raster = |d: &Document, r: photocraft_geom::Rect| {
        let mut l = Layer::raster("content", d.pixel_format());
        l.surface_mut().unwrap().fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
        l
    };
    let on = raster(&d, photocraft_geom::Rect::new(1, 1, 5, 5));
    let on = insert_new(&mut d, Some(board), on, Place::Auto);
    assert_eq!(d.artboard_of(on), Some(board));
    let off = raster(&d, photocraft_geom::Rect::new(10, 10, 16, 16));
    let off = insert_new(&mut d, Some(board), off, Place::Auto);
    assert_eq!(d.artboard_of(off), None);
    let root: Vec<LayerId> = d.layers.iter().map(|l| l.id).collect();
    assert_eq!(root.iter().position(|x| *x == off), root.iter().position(|x| *x == board).map(|i| i + 1));
}

/// An artboard is never nested or clipped: a copied board goes beside the anchor's top-level
/// layer, even after a first copy went into a group, and never joins a clipping mask.
#[test]
fn artboards_stay_at_the_top_level_and_unclipped() {
    let (mut s, a, b) = session();
    let board = id(s.execute("layer.new.artboard", json!({"rect": [0, 0, 8, 8]})).unwrap());
    s.execute("layer.setProps", json!({"layer": b.0, "clipped": true})).unwrap();
    let mut d = doc(&s).clone();
    let copy = d.layer(board).unwrap().duplicate();
    // On a clipping base: at the top level above its clipping mask (B keeps A as its base), not
    // clipped.
    let c = insert_new(&mut d, Some(a), copy.duplicate(), Place::Auto);
    let root: Vec<LayerId> = d.layers.iter().map(|l| l.id).collect();
    let at = |id: LayerId| root.iter().position(|x| *x == id).unwrap();
    assert_eq!((at(b), at(c)), (at(a) + 1, at(a) + 2));
    assert!(!d.layer(c).unwrap().clipped);
    // After a layer that went into an open group: beside the group, not inside it.
    let g = insert_new(&mut d, Some(a), Layer::group("open", vec![]), Place::Above);
    let first = Layer::raster("first", d.pixel_format());
    let first = insert_new(&mut d, Some(g), first, Place::Auto);
    assert!(d.layer(g).unwrap().children().unwrap().iter().any(|l| l.id == first));
    let second = insert_new(&mut d, Some(first), copy, Place::Above);
    assert!(d.layers.iter().any(|l| l.id == second), "the board is not nested in the group");
    assert_eq!(d.artboards().len(), 3);
}

/// A copied clipped layer pasted into an empty group has nothing below it to clip to.
#[test]
fn a_clipped_layer_with_nothing_below_is_released() {
    let (s, _, b) = session();
    let mut d = doc(&s).clone();
    let g = insert_new(&mut d, Some(b), Layer::group("empty", vec![]), Place::Auto);
    let mut l = Layer::raster("was clipped", d.pixel_format());
    l.clipped = true;
    let l = insert_new(&mut d, Some(g), l, Place::Auto);
    assert!(d.layer(g).unwrap().children().unwrap().iter().any(|c| c.id == l));
    assert!(!d.layer(l).unwrap().clipped);
}
