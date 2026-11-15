//! #9329 PR A: a pixel change damages only the tiles that changed, and command rects are hints
//! checked against them.

use super::*;
use photocraft_geom::TILE_SIZE;

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s
}

fn doc(s: &Session) -> std::sync::Arc<Document> {
    s.active().unwrap().doc.clone()
}

fn damage(s: &Session) -> Option<Rect> {
    s.active().unwrap().last_damage
}

/// The composite of `after` equals `before`'s outside `damage`.
fn unchanged_outside(before: &Document, after: &Document, damage: Rect) -> bool {
    let (x, y) = (photocraft_compose::render(before, before.bounds()), photocraft_compose::render(after, after.bounds()));
    let w = before.bounds().width() as i32;
    x.px.iter().zip(&y.px).enumerate().all(|(i, (p, q))| p == q || damage.contains(i as i32 % w, i as i32 / w))
}

/// The tiles `r` overlaps, as one rect.
fn tiles_of(r: Rect) -> Rect {
    let down = |v: i32| v.div_euclid(TILE_SIZE) * TILE_SIZE;
    Rect::new(down(r.x0), down(r.y0), down(r.x1 - 1) + TILE_SIZE, down(r.y1 - 1) + TILE_SIZE)
}

/// A new pixel layer with `rects` filled opaque, through one plain edit.
fn layer_with(s: &mut Session, rects: &[(Rect, [f32; 4])]) -> LayerId {
    let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        for (r, px) in rects {
            surf.fill_rect(*r, px);
        }
        Ok(())
    })
    .unwrap();
    id
}

fn full_layer(s: &mut Session) -> LayerId {
    let canvas = doc(s).bounds();
    layer_with(s, &[(canvas, [0.4, 0.5, 0.6, 1.0])])
}

/// One plain edit of layer `id`'s pixels, returning the documents on either side.
fn paint(s: &mut Session, id: LayerId, r: Rect, px: [f32; 4]) -> (std::sync::Arc<Document>, std::sync::Arc<Document>) {
    let before = doc(s);
    s.edit("paint", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(r, &px);
        Ok(())
    })
    .unwrap();
    (before, doc(s))
}

#[test]
fn a_fill_on_a_selection_damages_only_its_tiles() {
    let mut s = session(1024, 768);
    full_layer(&mut s);
    let sel = Rect::new(300, 300, 440, 410);
    s.execute("select.rect", json!({"x": sel.x0, "y": sel.y0, "width": sel.width(), "height": sel.height()})).unwrap();
    let before = doc(&s);
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let d = damage(&s).expect("a bounded fill");
    assert!(!d.is_empty() && tiles_of(sel).contains_rect(&d), "{d:?}");
    assert!(d.size().area() < doc(&s).bounds().size().area() / 4, "{d:?}");
    assert!(unchanged_outside(&before, &doc(&s), d));
}

#[test]
fn undo_of_a_brush_dab_on_a_full_layer_redraws_only_its_tiles() {
    let mut s = session(1024, 768);
    full_layer(&mut s);
    let four_tiles = 4 * (TILE_SIZE as u64).pow(2);
    let before = doc(&s);
    s.execute("paint.stroke", json!({"points": [[500, 400]], "size": 60, "color": "#ff0000"})).unwrap();
    let painted = doc(&s);
    let d = damage(&s).expect("the dab");
    assert!(!d.is_empty() && d.size().area() <= four_tiles, "dab {d:?}");
    assert!(unchanged_outside(&before, &painted, d));
    assert!(s.undo());
    let d = damage(&s).expect("undo is bounded");
    assert!(!d.is_empty() && d.size().area() <= four_tiles, "undo {d:?}");
    assert!(unchanged_outside(&painted, &doc(&s), d));
    assert!(s.redo());
    let d = damage(&s).expect("redo is bounded");
    assert!(!d.is_empty() && d.size().area() <= four_tiles, "redo {d:?}");
    assert!(unchanged_outside(&before, &doc(&s), d));
}

/// The damage `note_damage` used to report after a move: both extents of the moved layer.
#[test]
fn a_move_still_damages_both_extents() {
    let mut s = session(1024, 768);
    let r = Rect::new(100, 100, 200, 200);
    let id = layer_with(&mut s, &[(r, [1.0, 0.0, 0.0, 1.0])]);
    let before = doc(&s);
    s.execute("layer.translate", json!({"layer": id.0, "dx": 300, "dy": 50})).unwrap();
    let after = doc(&s);
    let canvas = after.bounds();
    let extents = |d: &Document| photocraft_compose::change_bounds(d.layer(id).unwrap(), canvas).unwrap();
    let both = extents(&before).union(&extents(&after));
    assert_eq!(both, r.union(&r.translate(300, 50)));
    assert_eq!(damage(&s), Some(both));
    assert!(unchanged_outside(&before, &after, both));
}

#[test]
fn set_props_damage_is_unchanged_without_note_damage() {
    let r = Rect::new(100, 100, 200, 200);
    for props in [json!({"opacity": 0.5}), json!({"fill": 0.5}), json!({"blend": "multiply"}), json!({"visible": false})] {
        let mut s = session(1024, 768);
        let id = layer_with(&mut s, &[(r, [1.0, 0.0, 0.0, 0.8])]);
        let before = doc(&s);
        let mut p = props.clone();
        p["layer"] = json!(id.0);
        s.execute("layer.setProps", p).unwrap();
        assert_eq!(damage(&s), Some(r), "{props}");
        assert!(unchanged_outside(&before, &doc(&s), r), "{props}");
    }
}

#[test]
fn a_mask_edit_damages_its_tiles_grown_by_the_feather() {
    let mut s = session(1024, 768);
    let id = full_layer(&mut s);
    let feather = 10.0;
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask { feather, ..photocraft_doc::LayerMask::reveal_all() });
        Ok(())
    })
    .unwrap();
    let before = doc(&s);
    let hole = Rect::new(600, 400, 610, 410);
    s.edit("mask paint", |doc, _| {
        doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface.fill_rect(hole, &[0.0]);
        Ok(())
    })
    .unwrap();
    let after = doc(&s);
    let d = step_damage(&before, &after).unwrap();
    let grown = photocraft_vector::feather::grow(tiles_of(hole), photocraft_vector::feather::feather_sigma(feather));
    assert_eq!(d, grown.intersect(&after.bounds()));
    assert!(d.contains_rect(&tiles_of(hole)) && d != after.bounds(), "{d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn tile_damage_grows_by_the_layer_effect_margin() {
    let mut s = session(1024, 768);
    let (a, b) = (Rect::new(50, 50, 150, 150), Rect::new(600, 400, 700, 500));
    let id = layer_with(&mut s, &[(a, [1.0, 0.0, 0.0, 1.0]), (b, [1.0, 0.0, 0.0, 1.0])]);
    s.edit("fx", |doc, _| {
        doc.layer_mut(id).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let m = photocraft_compose::effects::margin(doc(&s).layer(id).unwrap());
    assert!(m > 0);
    let (before, after) = paint(&mut s, id, Rect::new(640, 440, 660, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = step_damage(&before, &after).unwrap();
    let whole = photocraft_compose::change_bounds(after.layer(id).unwrap(), after.bounds()).unwrap();
    assert_eq!(d, tiles_of(b).inflate(m).intersect(&whole));
    assert!(d.x0 < tiles_of(b).x0 && d.y0 < tiles_of(b).y0, "the shadow reaches past the tile: {d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

/// The default drop shadow, `distance` px away and `size` px soft.
fn drop_shadow(distance: f32, size: f32) -> photocraft_doc::Effect {
    match photocraft_doc::Effect::default_drop_shadow() {
        photocraft_doc::Effect::DropShadow(sh) => photocraft_doc::Effect::DropShadow(photocraft_doc::Shadow { distance, size, ..sh }),
        e => e,
    }
}

/// A styled layer inside a styled group: the group's shadow is cast by the child's shadow too, so
/// a dab's change reaches as far as both shadows together, not just the farther one.
#[test]
fn a_dab_inside_a_group_with_a_drop_shadow_damages_the_shadow() {
    let mut s = session(1024, 768);
    let child = layer_with(&mut s, &[(Rect::new(200, 400, 300, 500), [1.0, 0.0, 0.0, 1.0])]);
    s.execute("layer.select", json!({"layer": child.0})).unwrap();
    let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("fx", |doc, _| {
        for id in [child, g] {
            doc.layer_mut(id).unwrap().effects.items.push(drop_shadow(40.0, 2.0));
        }
        Ok(())
    })
    .unwrap();
    assert!(photocraft_compose::effects::margin(doc(&s).layer(g).unwrap()) > 40);
    // A brush dab (a hinted edit) in empty space left of a tile's right edge. Each shadow falls
    // about 20 px right and 35 px down, so the group's shadow of the child's shadow lands about
    // 40 px right and 70 px down, in the next tile and past either shadow's reach alone.
    s.execute("layer.select", json!({"layer": child.0})).unwrap();
    let before = doc(&s);
    s.execute("paint.stroke", json!({"points": [[TILE_SIZE - 30, 300]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": child.0})).unwrap();
    assert_eq!(s.damage_misses, 0);
    let d = damage(&s).expect("a bounded dab");
    assert!(d.x0 < TILE_SIZE && d.x1 > TILE_SIZE, "{d:?}");
    assert!(unchanged_outside(&before, &doc(&s), d), "the group's shadow changed outside {d:?}");
}

/// A layer the compositor doesn't draw adds nothing to a dab elsewhere: neither a hidden layer's
/// effects nor those of layers inside a hidden group grow the hint.
#[test]
fn a_brush_hint_ignores_the_effects_of_hidden_layers() {
    let mut s = session(1024, 768);
    let plain = full_layer(&mut s);
    let hidden = layer_with(&mut s, &[(Rect::new(800, 600, 900, 700), [1.0, 0.0, 0.0, 1.0])]);
    let child = layer_with(&mut s, &[(Rect::new(50, 600, 150, 700), [0.0, 1.0, 0.0, 1.0])]);
    s.execute("layer.select", json!({"layer": child.0})).unwrap();
    let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("fx", |doc, _| {
        for id in [hidden, child] {
            let l = doc.layer_mut(id).unwrap();
            l.effects.items.push(drop_shadow(40.0, 40.0));
        }
        doc.layer_mut(hidden).unwrap().visible = false;
        doc.layer_mut(g).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    assert!(photocraft_compose::effects::margin(doc(&s).layer(hidden).unwrap()) > 40);
    // A dab in the middle of a tile, far from every tile edge.
    s.execute("layer.select", json!({"layer": plain.0})).unwrap();
    let before = doc(&s);
    s.execute("paint.stroke", json!({"points": [[384, 384]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": plain.0})).unwrap();
    assert_eq!(s.damage_misses, 0);
    let d = damage(&s).expect("a bounded dab");
    assert!(d.contains(384, 384) && d.width() <= 20 && d.height() <= 20, "grown by a hidden layer: {d:?}");
    assert!(unchanged_outside(&before, &doc(&s), d));
}

#[test]
fn painting_a_clip_base_under_a_styled_clipped_layer_is_covered() {
    let mut s = session(1024, 768);
    let base = layer_with(&mut s, &[(Rect::new(500, 300, 800, 600), [0.0, 1.0, 0.0, 1.0])]);
    let top = full_layer(&mut s);
    s.edit("clip", |doc, _| {
        let l = doc.layer_mut(top).unwrap();
        l.clipped = true;
        l.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let (before, after) = paint(&mut s, base, Rect::new(300, 200, 340, 240), [0.0, 0.0, 1.0, 1.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
    let (before, after) = paint(&mut s, base, Rect::new(640, 440, 660, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
    // At a tile's edge too: the clipped layer's shadow is built from its own shape, and the base
    // only masks it pixel by pixel, so a base edit changes nothing past its own tiles.
    let (before, after) = paint(&mut s, base, Rect::new(3 * TILE_SIZE - 6, 440, 3 * TILE_SIZE, 460), [0.0, 0.0, 0.0, 0.0]);
    let d = damage(&s).expect("a bounded base edit");
    assert!(d.x1 <= 3 * TILE_SIZE, "{d:?}");
    assert!(unchanged_outside(&before, &after, d), "the clipped layer changed outside {d:?}");
}

/// #4053: effects laid out over the layer's frame repaint it everywhere when it grows.
#[test]
fn a_frame_spanning_effect_still_redraws_its_whole_frame() {
    use photocraft_doc::{Effect, FxCommon, Gradient};
    let mut s = session(1024, 768);
    let id = layer_with(&mut s, &[(Rect::new(100, 100, 700, 200), [1.0, 0.0, 0.0, 1.0])]);
    s.edit("fx", |doc, _| {
        doc.layer_mut(id).unwrap().effects.items.push(Effect::GradientOverlay {
            common: FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
            gradient: Gradient::default(),
            dither: false,
            interpolation: None,
        });
        Ok(())
    })
    .unwrap();
    let (before, after) = paint(&mut s, id, Rect::new(700, 100, 800, 200), [0.0, 1.0, 0.0, 1.0]);
    let d = step_damage(&before, &after).unwrap();
    let extents = |d: &Document| photocraft_compose::change_bounds(d.layer(id).unwrap(), d.bounds()).unwrap();
    let whole = extents(&before).union(&extents(&after));
    assert!(whole.contains_rect(&Rect::new(100, 100, 800, 200)), "{whole:?}");
    assert_eq!(d, whole);
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn refreshing_type_layers_damages_only_those_layers() {
    let mut s = session(1500, 1000);
    let one = LayerId(s.execute("type.create", json!({"x": 100, "y": 100, "text": "One", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    let two = LayerId(s.execute("type.create", json!({"x": 1000, "y": 800, "text": "Two", "size": 20})).unwrap()["layer"].as_u64().unwrap());
    let before = doc(&s);
    let busy = s.refresh_type_layers(&[crate::TypeLayerRef { doc: before.id, layer: one, first_snapshot: false }]);
    assert!(busy.is_empty());
    let after = doc(&s);
    let d = damage(&s).expect("a bounded refresh");
    let bounds = |d: &Document, id: LayerId| photocraft_compose::change_bounds(d.layer(id).unwrap(), d.bounds()).unwrap();
    assert!(bounds(&before, one).union(&bounds(&after, one)).contains_rect(&d), "{d:?}");
    assert!(d.intersect(&bounds(&after, two)).is_empty(), "{d:?}");
    assert!(unchanged_outside(&before, &after, d));
}

#[test]
fn a_hint_that_misses_a_changed_tile_is_counted() {
    let mut s = session(1024, 768);
    let id = full_layer(&mut s);
    paint(&mut s, id, Rect::new(600, 400, 620, 420), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.damage_misses, 0);
    s.hint_damage(Rect::new(0, 0, 10, 10));
    assert_eq!(s.damage_misses, 1);
    assert_eq!(damage(&s), Some(Rect::EMPTY));
    // A hint covering the change is not a miss.
    paint(&mut s, id, Rect::new(600, 400, 620, 420), [0.0, 1.0, 0.0, 1.0]);
    s.hint_damage(Rect::new(600, 400, 620, 420));
    assert_eq!(s.damage_misses, 1);
    assert_eq!(damage(&s), Some(Rect::new(600, 400, 620, 420)));
}

/// A brush hint is the bare dab; on a feathered mask the dab changes a soft ring around it, and
/// the hint must not cut that off.
#[test]
fn a_brush_hint_on_a_feathered_mask_keeps_the_ring() {
    let mut s = session(1024, 768);
    let id = full_layer(&mut s);
    s.edit("mask", |doc, _| {
        doc.layer_mut(id).unwrap().mask = Some(photocraft_doc::LayerMask { feather: 10.0, ..photocraft_doc::LayerMask::reveal_all() });
        Ok(())
    })
    .unwrap();
    let before = doc(&s);
    s.execute("paint.stroke", json!({"points": [[500, 400]], "size": 10, "hardness": 1.0, "color": "#000000", "target": "mask", "layer": id.0})).unwrap();
    assert_eq!(s.damage_misses, 0);
    let d = damage(&s).expect("a bounded dab");
    assert!(unchanged_outside(&before, &doc(&s), d), "the feathered mask changed outside {d:?}");
}

/// The same for a layer's effects: the dab's drop shadow lands outside the dab.
#[test]
fn a_brush_hint_on_a_layer_with_a_drop_shadow_keeps_the_shadow() {
    let mut s = session(1024, 768);
    let id = layer_with(&mut s, &[(Rect::new(600, 400, 700, 500), [1.0, 0.0, 0.0, 1.0])]);
    s.edit("fx", |doc, _| {
        doc.layer_mut(id).unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let before = doc(&s);
    s.execute("paint.stroke", json!({"points": [[300, 300]], "size": 10, "hardness": 1.0, "color": "#0000ff", "layer": id.0})).unwrap();
    assert_eq!(s.damage_misses, 0);
    let d = damage(&s).expect("a bounded dab");
    assert!(unchanged_outside(&before, &doc(&s), d), "the shadow changed outside {d:?}");
}

#[test]
fn painting_command_hints_cover_their_changed_tiles() {
    let mut s = session(600, 400);
    let mut rects = Vec::new();
    for by in 0..20 {
        for bx in 0..30 {
            let v = ((bx * 7 + by * 13) % 17) as f32 / 17.0;
            rects.push((Rect::from_xywh(bx * 20, by * 20, 20, 20), [v, 1.0 - v, 0.5 * v + 0.2, 1.0]));
        }
    }
    rects.push((Rect::new(430, 280, 470, 320), [0.1, 0.1, 0.1, 1.0]));
    for y in 292..309 {
        for x in 442..459 {
            if (x - 450) * (x - 450) + (y - 300) * (y - 300) <= 64 {
                rects.push((Rect::from_xywh(x, y, 1, 1), [0.9, 0.05, 0.05, 1.0]));
            }
        }
    }
    layer_with(&mut s, &rects);
    let ty = s.execute("type.create", json!({"x": 40, "y": 380, "text": "Hi", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    let mut steps: Vec<(&str, serde_json::Value, Option<Rect>)> = vec![
        ("paint.stroke", json!({"points": [[100, 100], [300, 120]], "size": 20, "color": "#2040ff"}), None),
        ("paint.backgroundEraser", json!({"points": [[120, 300], [200, 320]], "size": 20}), None),
        ("paint.cloneStamp", json!({"points": [[400, 100]], "offset": [-100, 0], "size": 30, "hardness": 100}), None),
        ("paint.healingBrush", json!({"points": [[420, 200]], "source": [300, 200], "size": 30, "hardness": 100}), None),
        ("paint.redEye", json!({"x": 450, "y": 300}), None),
        ("paint.patch", json!({"offset": [60, 0]}), Some(Rect::new(60, 250, 100, 290))),
        ("paint.remove", json!({"points": [[500, 100], [540, 140]], "size": 24}), None),
        ("paint.contentAwareMove", json!({"offset": [100, -50], "structure": 3, "color": 4}), Some(Rect::new(300, 300, 340, 340))),
        ("paint.patternStamp", json!({"points": [[200, 200]], "size": 16, "hardness": 100, "pattern": "Checkerboard", "scale": 100, "aligned": true}), None),
    ];
    steps.push(("type.setStyle", json!({"layer": ty, "size": 40}), None));
    for (cmd, p, sel) in steps {
        if let Some(r) = sel {
            s.execute("select.rect", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()})).unwrap();
        } else if doc(&s).selection.is_some() {
            s.execute("select.deselect", json!({})).unwrap();
        }
        if cmd != "type.setStyle" {
            let layer = doc(&s).layers.iter().find(|l| l.surface().is_some() && !doc(&s).is_background(l.id)).unwrap().id;
            s.select_layer(layer).unwrap();
        }
        let before = doc(&s);
        s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        assert_eq!(s.damage_misses, 0, "{cmd}: its hint missed a changed tile");
        assert!(!same_pixels(&before, &doc(&s)), "{cmd} changed nothing");
        let d = damage(&s).unwrap_or_else(|| doc(&s).bounds());
        assert!(unchanged_outside(&before, &doc(&s), d), "{cmd}: pixels changed outside {d:?}");
    }
}

// PR C (#9329): damage spans a command, not a revision.

/// The damage since the command began: measured from the revision before it, bounded, and
/// covering every pixel the command changed.
fn command_damage(s: &mut Session, cmd: &str, p: serde_json::Value) -> (std::sync::Arc<Document>, Rect) {
    let (before, rev) = (doc(s), s.active().unwrap().revision);
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
    let st = s.active().unwrap();
    assert!(st.revision - rev >= 2, "{cmd}: several revisions ({} → {})", rev, st.revision);
    assert_eq!(st.damage_from(), rev, "{cmd}: measured from before the command");
    let d = st.last_damage.unwrap_or_else(|| panic!("{cmd}: bounded damage"));
    assert!(unchanged_outside(&before, &st.doc, d), "{cmd}: pixels changed outside {d:?}");
    (before, d)
}

#[test]
fn paste_into_reports_damage_since_the_command_began() {
    let mut s = session(1024, 768);
    layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0]), (Rect::new(40, 40, 120, 120), [0.9, 0.1, 0.1, 1.0])]);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 100, "height": 100})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 700, "y": 500, "width": 120, "height": 90})).unwrap();
    let (_, d) = command_damage(&mut s, "edit.pasteSpecial.pasteInto", json!({}));
    assert!(!d.intersect(&Rect::new(700, 500, 820, 590)).is_empty(), "the pasted area: {d:?}");
    assert!(u64::from(d.width()) * u64::from(d.height()) * 4 < 1024 * 768, "the pasted area, not the canvas: {d:?}");
}

#[test]
fn stroke_path_damage_spans_every_subpath() {
    let mut s = session(1024, 768);
    layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.5, 0.5, 0.5, 1.0])]);
    let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20], [80, 80]]}, {"knots": [[800, 600], [900, 600], [900, 700]]}]});
    s.execute("path.set", json!({ "path": path })).unwrap();
    // A tool with its own `paint.*` command runs once per subpath, one revision each.
    let (_, d) = command_damage(&mut s, "path.stroke", json!({"tool": "burn", "size": 16}));
    assert!(d.contains(50, 20) && d.contains(850, 600), "both subpaths: {d:?}");
    assert_eq!(s.whole_redraws, 0);
}

/// Pattern Stamp copies its pattern into the document on first use: a pattern no layer
/// refers to draws nothing, so a multi-subpath Stroke Path stays bounded.
#[test]
fn a_new_unreferenced_pattern_keeps_stroke_path_bounded() {
    let mut s = session(1024, 768);
    layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.5, 0.5, 0.5, 1.0])]);
    let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20], [80, 80]]}, {"knots": [[800, 600], [900, 600], [900, 700]]}]});
    s.execute("path.set", json!({ "path": path })).unwrap();
    let n = doc(&s).patterns.len();
    let p = json!({"tool": "patternStamp", "size": 16, "hardness": 100, "pattern": "Checkerboard", "scale": 100, "aligned": true});
    let (_, d) = command_damage(&mut s, "path.stroke", p);
    assert!(doc(&s).patterns.len() > n, "the stamp added its pattern");
    assert!(d.contains(50, 20) && d.contains(850, 600), "both subpaths: {d:?}");
    assert_eq!(s.whole_redraws, 0);
}

/// A pattern that an existing layer refers to (here a fill layer whose pattern was missing)
/// changes that layer's pixels when it is added: not bounded by the tile diff.
#[test]
fn a_new_pattern_a_layer_refers_to_is_unbounded() {
    let mut s = session(400, 300);
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})).unwrap();
    // Without its pixel cache, so both sides render the fill from the library.
    let mut with = (*doc(&s)).clone();
    with.layers.iter_mut().for_each(|l| l.fill_cache = None);
    let mut without = with.clone();
    without.patterns.clear();
    assert!(!same_pixels(&without, &with), "the fill layer draws only with its pattern");
    assert_eq!(step_damage(&without, &with), None);
    // Unchanged libraries, and an unreferenced addition, stay pixel-equal.
    assert_eq!(step_damage(&with, &with), Some(Rect::EMPTY));
    let plain = doc(&session(400, 300));
    let mut more = (*plain).clone();
    more.patterns = with.patterns.clone();
    assert_eq!(step_damage(&plain, &more), Some(Rect::EMPTY));
    assert!(unchanged_outside(&plain, &more, Rect::EMPTY));
}

/// An appended pattern sharing its name with one a layer already resolves by id loses to that id
/// match (`pattern::find` tries ids first), so the layer draws as before: bounded, not a whole
/// redraw. A cross-document layer copy can bring such a pattern along.
#[test]
fn a_same_named_new_pattern_a_reference_still_resolves_past_is_bounded() {
    let mut s = session(400, 300);
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})).unwrap();
    let mut with = (*doc(&s)).clone();
    with.layers.iter_mut().for_each(|l| l.fill_cache = None);
    let refers_by_id = with.layers.iter().any(|l| matches!(&l.content, LayerContent::Fill(photocraft_doc::Fill::Pattern { id, .. }) if !id.is_empty()));
    assert!(refers_by_id, "the fill layer refers to its pattern by id");
    let mut twin = with.patterns[0].clone();
    twin.id = "another-id".into();
    twin.surface.fill_rect(Rect::new(0, 0, twin.width as i32, twin.height as i32), &[1.0, 0.0, 1.0, 1.0]);
    let mut more = with.clone();
    more.patterns.push(twin);
    assert_eq!(step_damage(&with, &more), Some(Rect::EMPTY));
    assert!(unchanged_outside(&with, &more, Rect::EMPTY));
}

#[test]
fn a_failed_compound_reports_no_change() {
    let mut s = session(600, 400);
    let id = layer_with(&mut s, &[(Rect::new(0, 0, 50, 50), [1.0, 0.0, 0.0, 1.0])]);
    let (before, rev) = (doc(&s), s.active().unwrap().revision);
    let r: Result<()> = crate::analysis_cmds::compound(&mut s, "Both", |s| {
        paint(s, id, Rect::new(300, 200, 340, 240), [0.0, 1.0, 0.0, 1.0]);
        paint(s, id, Rect::new(10, 300, 40, 340), [0.0, 0.0, 1.0, 1.0]);
        Err(EngineError::Other("the second step failed".into()))
    });
    assert!(r.is_err());
    // What `Session::dispatch` does after any inline command.
    s.note_command_damage("test.compound", &before, rev);
    let st = s.active().unwrap();
    assert!(same_pixels(&before, &st.doc), "rolled back");
    assert_eq!((st.damage_from(), st.last_damage), (rev, Some(Rect::EMPTY)), "nothing changed since it began");
    assert_eq!(s.whole_redraws, 0);
}

#[test]
fn select_then_edit_in_one_command_is_bounded() {
    let mut s = session(1024, 768);
    let id = layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0])]);
    let steps = vec![
        ("layer.select".to_string(), json!({"layer": id.0})),
        ("select.rect".to_string(), json!({"x": 300, "y": 300, "width": 50, "height": 40})),
        ("edit.fill".to_string(), json!({"color": "#ff0000"})),
        ("select.deselect".to_string(), json!({})),
    ];
    s.actions.list.push(crate::actions_cmds::Action::new("A", steps));
    let (_, d) = command_damage(&mut s, "actions.play", json!({"action": "A"}));
    assert!(d.contains(300, 300) && d.contains(349, 339), "{d:?}");
    assert!(u64::from(d.width()) * u64::from(d.height()) * 4 < 1024 * 768, "the filled tiles, not the canvas: {d:?}");
    assert_eq!(s.whole_redraws, 0);
}

#[test]
fn whole_document_redraws_of_small_edits_are_counted() {
    let mut s = session(1000, 1000);
    let id = layer_with(&mut s, &[(Rect::new(0, 0, 1000, 1000), [0.2, 0.4, 0.6, 1.0])]);
    // A synthetic command that paints 1 % and reports "everything".
    let rev = s.active().unwrap().revision;
    let (before, _) = paint(&mut s, id, Rect::new(10, 10, 110, 110), [1.0, 0.0, 0.0, 1.0]);
    s.active_mut().unwrap().last_damage = None;
    s.note_command_damage("test.wholeRedraw", &before, rev);
    assert_eq!(s.whole_redraws, 1);
    // A canvas-size change really does redraw everything: not counted.
    s.execute("image.canvasSize", json!({"width": 1200, "height": 900})).unwrap();
    assert_eq!(s.active().unwrap().last_damage, None);
    assert_eq!(s.whole_redraws, 1);
}

#[test]
fn no_command_redraws_the_whole_document_for_a_small_edit() {
    let mut s = session(1024, 768);
    let base = layer_with(&mut s, &[(Rect::new(0, 0, 1024, 768), [0.2, 0.4, 0.6, 1.0])]);
    let small = layer_with(&mut s, &[(Rect::new(100, 100, 160, 160), [0.9, 0.1, 0.1, 1.0])]);
    let other = layer_with(&mut s, &[(Rect::new(130, 130, 190, 190), [0.1, 0.9, 0.1, 1.0])]);
    let path = json!({"subpaths": [{"knots": [[20, 20], [80, 20]]}, {"knots": [[800, 600], [900, 600]]}]});
    let steps: Vec<(&str, serde_json::Value)> = vec![
        ("layer.select", json!({"layer": base.0})),
        ("select.rect", json!({"x": 300, "y": 300, "width": 50, "height": 40})),
        ("edit.fill", json!({"color": "#ff0000"})),
        ("filter.blur.gaussianBlur", json!({"radius": 2})),
        ("edit.copy", json!({})),
        ("select.rect", json!({"x": 600, "y": 400, "width": 80, "height": 60})),
        ("edit.pasteSpecial.pasteInto", json!({})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("layer.select", json!({"layer": base.0})),
        ("paint.stroke", json!({"points": [[100, 400], [200, 420]], "size": 20, "color": "#2040ff"})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("path.set", json!({ "path": path })),
        ("path.stroke", json!({"tool": "pencil", "size": 4, "color": "#0000ff", "opacity": 100})),
        ("path.stroke", json!({"tool": "burn", "size": 16})),
        ("layer.select", json!({"layer": small.0})),
        ("layer.setProps", json!({"layer": small.0, "opacity": 0.5})),
        ("layer.setProps", json!({"layer": small.0, "visible": false})),
        ("layer.setProps", json!({"layer": small.0, "visible": true})),
        ("layer.translate", json!({"layer": small.0, "dx": 20, "dy": 10})),
        // Show Only hides the full-canvas base: a large change, so not counted either way.
        ("layer.showOnly", json!({"layer": small.0})),
        ("layer.showOnly", json!({"layer": small.0})),
        ("layer.arrange.bringForward", json!({})),
        ("layer.arrange.sendBackward", json!({})),
        ("layer.setProps", json!({"layer": other.0, "clipped": true})),
        ("layer.new.layer", json!({})),
    ];
    for (cmd, p) in steps {
        s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        assert_eq!(s.whole_redraws, 0, "{cmd}: redrew the whole document for a small edit");
    }
}
