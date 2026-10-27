use super::*;

#[test]
fn merge_with_background_preserves_background() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    let bg_id = s.active().unwrap().doc.layers[0].id;
    assert!(s.active().unwrap().doc.is_background(bg_id));
    let l1 = LayerId(s.execute("layer.new.layer", json!({"name": "Generative Fill"})).unwrap()["layer"].as_u64().unwrap());
    set_selection(&mut s, vec![bg_id, l1], Some(l1), Some(bg_id)).unwrap();
    let res = s.execute("layer.mergeLayers", json!({})).unwrap();
    let mid = LayerId(res["layer"].as_u64().unwrap());
    assert_eq!(mid, bg_id);
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert!(doc.is_background(mid));
    assert_eq!(doc.layers[0].name, "Background");
}

#[test]
fn duplicates_are_numbered_not_stacked() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 20})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Layer 1"})).unwrap();
    let name = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    };
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(name(&s), "Layer 1 copy");
    // Duplicating the copy (the active layer) numbers it rather than adding "copy" again.
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(name(&s), "Layer 1 copy 2");
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(name(&s), "Layer 1 copy 3");
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.copy_name("Layer 1"), "Layer 1 copy 4");
    // Names that merely contain "copy" keep it.
    assert_eq!(doc.copy_name("Copywriting"), "Copywriting copy");
    assert_eq!(doc.copy_name("A copyedit"), "A copyedit copy");
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// New pixel layer with an opaque red rectangle.
fn rect_layer(s: &mut Session, r: Rect) -> LayerId {
    let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    id
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    layer_bounds(doc(s).layer(id).unwrap()).unwrap()
}

fn sel(s: &Session) -> Vec<LayerId> {
    selected(s)
}

fn damage(s: &Session) -> Option<Rect> {
    s.active().unwrap().last_damage
}

/// A Photoshop action's nudge in points moves whole pixels at the document's resolution:
/// 0.36 pt is 1 px at 200 ppi and 1.5 → 2 px at 300 ppi.
#[test]
fn translate_in_points_follows_the_resolution() {
    for (res, moved) in [(200, 1), (300, 2), (72, 0)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 80, "resolution": res})).unwrap();
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        s.execute("layer.translate", json!({"dx": 0.36, "dy": -0.36, "units": "pt"})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(10 + moved, 10 - moved, 20 + moved, 20 - moved), "{res} ppi");
    }
    let mut s = session(8);
    rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    for bad in [json!({"dx": 1, "units": "in"}), json!({"dx": "x", "units": "pt"}), json!({"dx": 1e300, "units": "pt"})] {
        assert!(s.execute("layer.translate", bad.clone()).is_err(), "{bad}");
    }
}

/// `remove` deselects a selected layer, leaves an unselected one alone and never empties the
/// selection.
#[test]
fn select_remove_takes_a_layer_out_of_the_selection() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    let b = rect_layer(&mut s, Rect::new(30, 10, 40, 20));
    let c = rect_layer(&mut s, Rect::new(50, 10, 60, 20));
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": c.0, "mode": "remove"})).unwrap();
    assert_eq!(sel(&s).len(), 2, "an unselected layer stays out");
    s.execute("layer.select", json!({"layer": b.0, "mode": "remove"})).unwrap();
    assert_eq!(sel(&s), vec![a]);
    s.execute("layer.select", json!({"layer": a.0, "mode": "remove"})).unwrap();
    assert_eq!(sel(&s), vec![a], "the last selected layer stays");
}

/// A mask with its chain off stays put while the layer moves; a linked one moves along.
#[test]
fn translate_leaves_an_unlinked_mask_in_place() {
    for depth in [8, 16] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.edit("mask", |doc, _| {
            doc.layer_mut(a).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(12, 12, 18, 18), &[1.0]);
            Ok(())
        })
        .unwrap();
        let mval = |s: &Session, x, y| doc(s).layer(a).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0);
        s.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
        s.execute("layer.translate", json!({"dx": 30, "dy": 0})).unwrap();
        assert!(mval(&s, 14, 14) > 0.9, "{depth}-bit: unlinked mask stays");
        assert!(mval(&s, 44, 14) < 0.1);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("layer.layerMask.linked", json!({"linked": true})).unwrap();
        s.execute("layer.translate", json!({"dx": 30, "dy": 0})).unwrap();
        assert!(mval(&s, 44, 14) > 0.9, "{depth}-bit: linked mask moves");
    }
}

/// Aimed at the mask, a move shifts an unlinked mask alone and leaves a linked one with its layer.
#[test]
fn translate_with_the_mask_targeted_moves_an_unlinked_mask_alone() {
    for depth in [8, 16] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.edit("mask", |doc, _| {
            doc.layer_mut(a).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(12, 12, 18, 18), &[1.0]);
            Ok(())
        })
        .unwrap();
        let mval = |s: &Session, x, y| doc(s).layer(a).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0);
        // Linked: the layer and its mask move together.
        s.execute("layer.translate", json!({"dx": 30, "dy": 0, "target": "mask"})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(40, 10, 50, 20), "{depth}-bit");
        assert!(mval(&s, 44, 14) > 0.9);
        s.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
        s.execute("layer.translate", json!({"dx": -30, "dy": 0, "target": "mask"})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(40, 10, 50, 20), "{depth}-bit: the layer stays");
        assert!(mval(&s, 14, 14) > 0.9, "the mask moved");
        assert!(mval(&s, 44, 14) < 0.1);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(mval(&s, 44, 14) > 0.9, "undo puts the mask back");
    }
}

#[test]
fn moves_and_layer_props_report_only_their_area() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.translate", json!({"dx": 5, "dy": -3})).unwrap();
    assert_eq!(damage(&s), Some(Rect::new(10, 7, 25, 20)), "where the layer was and is");
    s.execute("layer.setProps", json!({"layer": a.0, "visible": false})).unwrap();
    assert_eq!(damage(&s), Some(Rect::new(15, 7, 25, 17)));
    s.execute("layer.setProps", json!({"layer": a.0, "opacity": 0.5, "blend": "Multiply"})).unwrap();
    assert_eq!(damage(&s), Some(Rect::new(15, 7, 25, 17)));
    // Clipping changes reach the layers around: everything.
    s.execute("layer.setProps", json!({"layer": a.0, "clipped": true})).unwrap();
    assert_eq!(damage(&s), None);
    // An adjustment layer changes everything beneath it.
    let adj = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": adj, "visible": false})).unwrap();
    assert_eq!(damage(&s), None);
    // The Background (opaque everywhere): everything.
    let bg = doc(&s).layers[0].id;
    s.execute("layer.setProps", json!({"layer": bg.0, "opacity": 0.5})).unwrap();
    let all = doc(&s).bounds();
    assert!(damage(&s).is_none_or(|r| r.contains_rect(&all)), "{:?}", damage(&s));
    // Bad params still fail cleanly.
    assert!(s.execute("layer.setProps", json!({"layer": u64::MAX, "visible": true})).is_err());
    assert!(s.execute("layer.translate", json!({"layer": u64::MAX, "dx": 1})).is_err());
}

/// #1771: a new empty layer or group changes no pixels, so it (and undoing or redoing it)
/// must not make the canvas recomposite every layer of the document.
#[test]
fn new_empty_layers_and_groups_damage_nothing() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        let composite = |s: &Session| photocraft_compose::render(doc(s), doc(s).bounds());
        let shown = composite(&s);
        s.execute("layer.new.layer", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY), "{depth}-bit new layer");
        assert_eq!(composite(&s), shown, "an empty layer changes no pixels");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY), "{depth}-bit undo New Layer");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY), "{depth}-bit redo New Layer");
        s.execute("layer.new.group", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY), "{depth}-bit new group");
        // Inside a group too.
        let child = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let g = s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.select", json!({"layer": child})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY), "{depth}-bit new layer in a group");
        assert!(doc(&s).layer(LayerId(g)).and_then(Layer::children).is_some_and(|c| c.len() == 2));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::EMPTY));
        // Undoing a step that painted still reports that area, not nothing.
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.translate", json!({"dx": 5, "dy": 0})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(Rect::new(10, 10, 25, 20)));
    }
}

#[test]
fn new_layers_that_change_clipping_recomposite_everything() {
    let mut s = session(8);
    let base = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    let clipped = rect_layer(&mut s, Rect::new(0, 0, 100, 80));
    s.edit("green", |doc, _| {
        doc.layer_mut(clipped).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 100, 80), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.setProps", json!({"layer": clipped.0, "clipped": true})).unwrap();
    // New Layer between a clipping base and its clipped layer joins their clip group (#2203),
    // so it draws nothing; an empty layer there that stayed unclipped would become their base.
    s.execute("layer.select", json!({"layer": base.0})).unwrap();
    let shown = photocraft_compose::render(doc(&s), doc(&s).bounds());
    let before = doc(&s).clone();
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(damage(&s), Some(Rect::EMPTY));
    assert_eq!(photocraft_compose::render(doc(&s), doc(&s).bounds()), shown);
    let mut split = before.clone();
    split.insert_above(Some(base), Layer::raster("split", split.pixel_format()));
    assert_ne!(photocraft_compose::render(&split, split.bounds()), shown);
    assert_eq!(step_damage(&before, &split), None);
    assert_eq!(step_damage(&split, &before), None);
    s.execute("edit.undo", json!({})).unwrap();
    // A step that adds a layer with pixels (Layer via Copy, paste…) changes where it draws.
    let before = doc(&s).clone();
    let mut after = before.clone();
    let mut l = Layer::raster("painted", after.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
    after.insert_above(None, l);
    assert_eq!(step_damage(&before, &after), Some(Rect::new(1, 1, 2, 2)));
    assert_eq!(step_damage(&after, &before), Some(Rect::new(1, 1, 2, 2)));
    // Unless it becomes the base of clipped layers above it.
    let mut based = before.clone();
    let mut l = Layer::raster("base", based.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(1, 1, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
    let under = based.layers.iter().position(|l| l.id == clipped).unwrap();
    based.layers.insert(under, l);
    assert_eq!(step_damage(&before, &based), None);
    assert_eq!(step_damage(&based, &before), None);
    // An adjustment reaches every pixel below it.
    let mut adjusted = before.clone();
    adjusted.insert_above(None, Layer::new("inv", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    assert_eq!(step_damage(&before, &adjusted), None);
    assert_eq!(step_damage(&adjusted, &before), None);
    // Empty layers draw nothing, in any blend mode.
    let mut styled = before.clone();
    let mut l = Layer::raster("styled", styled.pixel_format());
    l.blend = photocraft_doc::BlendMode::Multiply;
    styled.insert_above(None, l);
    assert_eq!(step_damage(&before, &styled), Some(Rect::EMPTY));
    let mut plain = before.clone();
    plain.insert_above(None, Layer::raster("plain", plain.pixel_format()));
    assert_eq!(step_damage(&before, &plain), Some(Rect::EMPTY));
    assert_eq!(step_damage(&plain, &before), Some(Rect::EMPTY));
}

/// #6535: hiding or showing an adjustment clipped to a layer recomposites only that base's
/// area, also on undo and redo and inside a group, and recompositing just that area gives
/// the same canvas as a full recomposite.
#[test]
fn toggling_a_clipped_adjustment_damages_only_its_base() {
    let all = |s: &Session| photocraft_compose::render(doc(s), doc(s).bounds());
    // Recomposite only the reported damage over the previous canvas, as the view does.
    let patched = |s: &Session, prev: &photocraft_compose::Buffer| {
        let r = damage(s).expect("bounded damage");
        let mut out = prev.clone();
        if r.is_empty() {
            return out;
        }
        let part = photocraft_compose::render(doc(s), r);
        let w = out.rect.width() as usize;
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let i = (y - out.rect.y0) as usize * w + (x - out.rect.x0) as usize;
                let k = (y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize;
                out.px[i] = part.px[k];
            }
        }
        out
    };
    for grouped in [false, true] {
        let mut s = session(16);
        let base_rect = Rect::new(10, 10, 30, 25);
        let base = rect_layer(&mut s, base_rect);
        let adj = LayerId(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.createClippingMask", json!({"layer": adj.0})).unwrap();
        assert!(doc(&s).layer(adj).unwrap().clipped);
        if grouped {
            s.execute("layer.select", json!({"layer": base.0})).unwrap();
            s.execute("layer.select", json!({"layer": adj.0, "mode": "toggle"})).unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
            assert!(doc(&s).path_of(adj).unwrap().len() == 2, "the pair sits in a group");
        }
        let shown = all(&s);
        s.execute("layer.hideLayers", json!({"layer": adj.0})).unwrap();
        assert!(!doc(&s).layer(adj).unwrap().visible);
        assert_eq!(damage(&s), Some(base_rect), "grouped: {grouped}");
        let hidden = all(&s);
        assert_ne!(hidden, shown);
        assert_eq!(patched(&s, &shown), hidden);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(base_rect));
        assert_eq!(patched(&s, &hidden), shown);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(damage(&s), Some(base_rect));
        assert_eq!(patched(&s, &shown), hidden);
        s.execute("layer.showLayers", json!({"layer": adj.0})).unwrap();
        assert_eq!(damage(&s), Some(base_rect));
        assert_eq!(patched(&s, &hidden), shown);
        // layer.setProps reports its own damage.
        s.execute("layer.setProps", json!({"layer": adj.0, "visible": false})).unwrap();
        assert_eq!(damage(&s), Some(base_rect));
        assert_eq!(patched(&s, &shown), hidden);
        // Unclipped, the adjustment still reaches the whole canvas.
        s.execute("layer.setProps", json!({"layer": adj.0, "visible": true})).unwrap();
        s.execute("layer.releaseClippingMask", json!({"layer": adj.0})).unwrap();
        s.execute("layer.hideLayers", json!({"layer": adj.0})).unwrap();
        assert_eq!(damage(&s), None);
    }
}

/// #6535: hiding a clipped adjustment inside a pass-through group that is itself a clip base
/// can switch the group between isolated and pass-through rendering, so the step damages the
/// whole group, on undo, redo and when the adjustment is deleted too.
#[test]
fn clipped_adjustment_in_pass_through_clip_base_damages_its_group() {
    use photocraft_doc::{Adjustment, BlendMode};
    let solid = |name: &str, r: Rect, blend: BlendMode| {
        let mut l = Layer::raster(name, PixelFormat::RGBA8);
        l.surface_mut().unwrap().fill_rect(r, &[0.5, 0.5, 0.5, 1.0]);
        l.blend = blend;
        l
    };
    let mut before = Document::with_background(
        "t",
        photocraft_geom::Size::new(10, 8),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_color::Color::WHITE,
    );
    before.layers.push(solid("bg", before.bounds(), BlendMode::Normal));
    let mut a = Layer::new("A", LayerContent::Adjustment(Adjustment::Invert));
    a.clipped = true;
    let mut g = Layer::group("G", vec![solid("P", Rect::new(2, 1, 4, 3), BlendMode::Normal), a, solid("Q", Rect::new(6, 4, 8, 6), BlendMode::Multiply)]);
    g.blend = BlendMode::PassThrough;
    before.layers.push(g);
    let mut x = solid("X", Rect::new(2, 1, 3, 2), BlendMode::Normal);
    x.clipped = true;
    before.layers.push(x);
    let mut after = before.clone();
    let LayerContent::Group(g) = &mut after.layers[2].content else { panic!("group") };
    g.children[1].visible = false;
    let group = Some(Rect::new(2, 1, 8, 6));
    assert_eq!(step_damage(&before, &after), group);
    assert_eq!(step_damage(&after, &before), group);
    let mut deleted = before.clone();
    let LayerContent::Group(g) = &mut deleted.layers[2].content else { panic!("group") };
    g.children.remove(1);
    assert_eq!(step_damage(&before, &deleted), group);
    let LayerContent::Group(g) = &before.layers[2].content else { panic!("group") };
    let ids = [g.children[1].id];
    assert_eq!(layers_damage(&before, &after, &ids), group);
    // Without the layer clipped to the group it stays pass-through: just A's base.
    before.layers.truncate(3);
    after.layers.truncate(3);
    assert_eq!(step_damage(&before, &after), Some(Rect::new(2, 1, 4, 3)));
}

/// #2883 (P8/P9): a duplicate shares the source's tiles (no pixel copy) and recomposites only
/// where it draws; undo brings back exactly the old pixels. A copy that becomes the clipping
/// base of the layers above still recomposites everything.
#[test]
fn duplicate_shares_tiles_and_damages_only_the_copy() {
    let render = |s: &Session| photocraft_compose::render(doc(s), doc(s).bounds());
    let tiles =
        |s: &Session, id: LayerId| doc(s).layer(id).unwrap().surface().unwrap().tiles().map(|(c, t)| (*c, std::sync::Arc::as_ptr(t))).collect::<Vec<_>>();
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        s.execute("layer.setProps", json!({"layer": a.0, "opacity": 0.5})).unwrap();
        let shown = render(&s);
        let copy = LayerId(s.execute("layer.duplicate", json!({"layer": a.0})).unwrap()["layer"].as_u64().unwrap());
        assert_eq!(damage(&s), Some(Rect::new(10, 10, 20, 20)), "{depth}-bit: only where the copy draws");
        assert!(!tiles(&s, a).is_empty());
        assert_eq!(tiles(&s, copy), tiles(&s, a), "{depth}-bit: the copy shares the source's tiles");
        let now = render(&s);
        assert_ne!(now, shown, "the half-transparent copy darkens its area");
        let w = shown.rect.width() as usize;
        for (i, (p, q)) in shown.px.iter().zip(&now.px).enumerate() {
            if !Rect::new(10, 10, 20, 20).contains((i % w) as i32, (i / w) as i32) {
                assert_eq!(p, q, "{depth}-bit: pixel {i} outside the damage changed");
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert!(doc(&s).layer(copy).is_none());
        assert_eq!(render(&s), shown);
        // A clipping base's copy goes between it and its clipped layer and becomes their base.
        let clipped = rect_layer(&mut s, Rect::new(0, 0, 50, 50));
        s.execute("layer.setProps", json!({"layer": clipped.0, "clipped": true})).unwrap();
        s.execute("layer.duplicate", json!({"layer": a.0})).unwrap();
        assert_eq!(damage(&s), None, "{depth}-bit: a new clipping base");
    }
}

/// The composite of `after` equals `before`'s outside `damage`.
fn unchanged_outside(before: &Document, after: &Document, damage: Rect) -> bool {
    let (x, y) = (photocraft_compose::render(before, before.bounds()), photocraft_compose::render(after, after.bounds()));
    let w = before.bounds().width() as i32;
    x.px.iter().zip(&y.px).enumerate().all(|(i, (p, q))| p == q || damage.contains(i as i32 % w, i as i32 / w))
}

/// Duplicate (one layer or several), paste and delete, and their undo and redo, recomposite
/// only where the layers draw; undoing a duplicate used to recomposite the whole document.
#[test]
fn duplicate_paste_and_delete_report_where_the_layer_draws() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    let full = doc(&s).bounds();
    let step = |s: &mut Session, id: &str, p: serde_json::Value| {
        let before = doc(s).clone();
        s.execute(id, p).unwrap();
        let d = damage(s).unwrap_or_else(|| panic!("{id}: unbounded damage"));
        assert!(d.width() < full.width(), "{id}: {d:?}");
        assert!(unchanged_outside(&before, doc(s), d), "{id}: pixels changed outside {d:?}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(damage(s), Some(d), "undo {id}");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(damage(s), Some(d), "redo {id}");
    };
    step(&mut s, "layer.duplicate", json!({"layer": a.0}));
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("select.rect", json!({"x": 12, "y": 12, "width": 4, "height": 4})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    step(&mut s, "edit.paste", json!({}));
    // Duplicating several selected layers reports where the copies draw, too.
    let b = rect_layer(&mut s, Rect::new(30, 30, 40, 40));
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    step(&mut s, "layer.duplicate", json!({}));
    assert_eq!(damage(&s), Some(Rect::new(10, 10, 40, 40)), "both copies, nothing else");
    step(&mut s, "layer.delete", json!({"layer": a.0}));
}

#[test]
fn the_move_preview_matches_the_move() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    let before = s.active().unwrap().doc.clone();
    let ids = move_targets(&before, &[a]);
    let shown = moved(&before, &ids, 7, 4).unwrap();
    s.execute("layer.translate", json!({"layer": a.0, "dx": 7, "dy": 4})).unwrap();
    assert_eq!(layer_bounds(shown.layer(a).unwrap()), Some(bounds(&s, a)));
    // Locked layers don't move in the preview either.
    let mut locked = (*before).clone();
    locked.layer_mut(a).unwrap().locks.position = true;
    assert!(moved(&locked, &ids, 1, 1).is_err());
    assert!(moved(&before, &[LayerId(u64::MAX)], 1, 1).is_err());
}

#[test]
fn move_stroked_shape_across_canvas_edge_matches_fresh_render() {
    let mut s = session(8);
    let id = LayerId(
        s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 30, 30], "stroke": {"width": 4, "color": "#ff0000"}})).unwrap()["layer"]
            .as_u64()
            .unwrap(),
    );
    let before = doc(&s).clone();
    let ids = move_targets(&before, &[id]);

    // Drag the stroked shape across the left canvas boundary:
    let moved_doc = moved(&before, &ids, -25, 0).unwrap();
    let moved_layer = moved_doc.layer(id).unwrap();
    let LayerContent::Shape(sh) = &moved_layer.content else { panic!("not a shape") };
    let cached = sh.cache.as_ref().expect("cached render");

    let mut fresh_sh = sh.clone();
    crate::vector_cmds::refresh_shape(&moved_doc, &mut fresh_sh);
    let fresh = fresh_sh.cache.as_ref().expect("fresh render");

    assert_eq!(cached, fresh, "dragged shape cached render must match fresh render across canvas edge");

    // Drag it back inside the canvas:
    let moved_back = moved(&moved_doc, &ids, 30, 0).unwrap();
    let back_layer = moved_back.layer(id).unwrap();
    let LayerContent::Shape(back_sh) = &back_layer.content else { panic!("not a shape") };
    let back_cached = back_sh.cache.as_ref().expect("cached render");

    let mut fresh_back = back_sh.clone();
    crate::vector_cmds::refresh_shape(&moved_back, &mut fresh_back);
    let fresh_back_render = fresh_back.cache.as_ref().expect("fresh render");

    assert_eq!(back_cached, fresh_back_render, "cached render must match fresh render when dragged back");
}

fn select_all(s: &mut Session, ids: &[LayerId]) {
    s.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "toggle"})).unwrap();
    }
}

#[test]
fn toggle_and_range_selection() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let c = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let d = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    assert_eq!(sel(&s), vec![d]);
    s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    assert_eq!(sel(&s), vec![b, d]);
    assert_eq!(s.active().unwrap().active_layer, Some(b));
    // Toggling the active layer off hands "active" to a remaining member.
    s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
    assert_eq!(sel(&s), vec![d]);
    assert_eq!(s.active().unwrap().active_layer, Some(d));
    // The last selected layer can't be toggled off.
    s.execute("layer.select", json!({"layer": d.0, "mode": "toggle"})).unwrap();
    assert_eq!(sel(&s), vec![d]);
    // Range from the anchor (a) to c.
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": c.0, "mode": "range"})).unwrap();
    assert_eq!(sel(&s), vec![a, b, c]);
    // A second ⇧-click re-ranges from the same anchor.
    s.execute("layer.select", json!({"layer": b.0, "mode": "range"})).unwrap();
    assert_eq!(sel(&s), vec![a, b]);
    assert!(s.execute("layer.select", json!({"layer": b.0, "mode": "bogus"})).is_err());
    // Selection changes are not edits.
    let history = s.active().unwrap().history.entries().len();
    s.execute("select.allLayers", json!({})).unwrap();
    assert_eq!(sel(&s), vec![a, b, c, d], "Select All Layers skips the locked Background");
    s.execute("select.deselectLayers", json!({})).unwrap();
    assert!(sel(&s).is_empty());
    assert_eq!(s.active().unwrap().history.entries().len(), history);
    // Nothing selected: a no-op, not an error (#2457).
    s.execute("select.deselectLayers", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), history);
    // An explicit layer needs no active layer.
    s.execute("layer.hideLayers", json!({"layer": a.0})).unwrap();
    assert!(!s.active().unwrap().doc.layer(a).unwrap().visible);
    s.execute("layer.renameLayer", json!({"layer": a.0, "name": "x"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap().name, "x");
    s.execute("layer.showLayers", json!({"layer": a.0})).unwrap();
    assert!(s.active().unwrap().doc.layer(a).unwrap().visible);
    assert!(s.execute("layer.hideLayers", json!({"layer": u64::MAX})).is_err());
    assert!(s.execute("layer.hideLayers", json!({})).is_err());
    assert!(!s.is_enabled("layer.align.topEdges"));
}

#[test]
fn find_layers_by_name() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    s.execute("layer.renameLayer", json!({"layer": a.0, "name": "Sky photo"})).unwrap();
    s.execute("layer.renameLayer", json!({"layer": b.0, "name": "Photo of sea"})).unwrap();
    assert_eq!(doc(&s).layer(a).unwrap().name, "Sky photo");
    let r = s.execute("select.findLayers", json!({"name": "PHOTO"})).unwrap();
    assert_eq!(r["selected"], json!([a.0, b.0]));
    assert!(s.execute("select.findLayers", json!({"name": "nothing"})).is_err());
    assert!(s.execute("layer.renameLayer", json!({})).is_err());
}

#[test]
fn rename_layer_keeps_surrounding_spaces() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    s.execute("layer.renameLayer", json!({"layer": a.0, "name": " black skin bw "})).unwrap();
    assert_eq!(doc(&s).layer(a).unwrap().name, " black skin bw ");
    assert!(s.execute("layer.renameLayer", json!({"layer": a.0, "name": "   "})).is_err());
    assert_eq!(doc(&s).layer(a).unwrap().name, " black skin bw ");
}

#[test]
fn align_each_edge() {
    for depth in [8, 16, 32] {
        let cases: [(&str, [Rect; 2]); 6] = [
            ("topEdges", [Rect::new(10, 5, 20, 15), Rect::new(40, 5, 50, 25)]),
            ("bottomEdges", [Rect::new(10, 20, 20, 30), Rect::new(40, 10, 50, 30)]),
            // Union 5..30: centre 17.5; half-pixel offsets round towards the top.
            ("verticalCenters", [Rect::new(10, 12, 20, 22), Rect::new(40, 7, 50, 27)]),
            ("leftEdges", [Rect::new(10, 5, 20, 15), Rect::new(10, 10, 50, 30)]),
            ("rightEdges", [Rect::new(40, 5, 50, 15), Rect::new(10, 10, 50, 30)]),
            ("horizontalCenters", [Rect::new(25, 5, 35, 15), Rect::new(10, 10, 50, 30)]),
        ];
        for (kind, want) in cases {
            let mut s = session(depth);
            let a = rect_layer(&mut s, Rect::new(10, 5, 20, 15));
            let b = rect_layer(&mut s, Rect::new(40, 10, 50, 30));
            // For the horizontal cases make b wide so it defines the target bounds.
            if !matches!(kind, "topEdges" | "bottomEdges" | "verticalCenters") {
                s.edit("wide", |doc, _| {
                    doc.layer_mut(b).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[0.0, 1.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            }
            select_all(&mut s, &[a, b]);
            s.execute(&format!("layer.align.{kind}"), json!({})).unwrap();
            assert_eq!([bounds(&s, a), bounds(&s, b)], want, "{kind} at {depth}-bit");
            // Pixels moved, not just bounds.
            let r = want[0];
            assert_eq!(doc(&s).layer(a).unwrap().surface().unwrap().pixel(r.x0, r.y0)[..4], [1.0, 0.0, 0.0, 1.0]);
            // One undo restores the original positions.
            s.undo();
            assert_eq!(bounds(&s, a), Rect::new(10, 5, 20, 15));
        }
    }
}

#[test]
fn align_single_layer_to_selection() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
        assert!(s.is_enabled("layer.align.leftEdges"), "one layer aligns to the canvas");
        s.execute("select.rect", json!({"x": 50, "y": 40, "width": 30, "height": 30})).unwrap();
        assert!(s.is_enabled("layer.align.leftEdges"));
        s.execute("layer.align.rightEdges", json!({})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(70, 10, 80, 20));
        s.execute("layer.align.verticalCenters", json!({})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(70, 50, 80, 60));
        s.execute("layer.align.topEdges", json!({"to": "canvas"})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(70, 0, 80, 10));
    }
}

#[test]
fn align_single_layer_to_canvas() {
    // Photoshop: a single selected layer with no active selection aligns to the canvas.
    let mut s = session(8); // 100 x 80
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20)); // 10 x 10
    assert!(s.is_enabled("layer.align.horizontalCenters"));
    s.execute("layer.align.horizontalCenters", json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(45, 10, 55, 20), "centred on the 100px-wide canvas");
    s.execute("layer.align.verticalCenters", json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(45, 35, 55, 45), "centred on the 80px-tall canvas");
    s.execute("layer.align.leftEdges", json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(0, 35, 10, 45), "left edge to the canvas");
}

#[test]
fn distribute_uneven_layers() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
        let b = rect_layer(&mut s, Rect::new(12, 0, 32, 10)); // 20 wide
        let c = rect_layer(&mut s, Rect::new(70, 0, 100, 10)); // 30 wide
        select_all(&mut s, &[a, b, c]);
        // Undo targets the layers its state had when it was created (#495): reselect.
        assert!(s.is_enabled("layer.distribute.leftEdges"));
        s.execute("layer.distribute.leftEdges", json!({})).unwrap();
        assert_eq!(bounds(&s, b), Rect::new(35, 0, 55, 10), "left edges 0, 35, 70");
        s.undo();
        select_all(&mut s, &[a, b, c]);
        s.execute("layer.distribute.horizontalCenters", json!({})).unwrap();
        // centres 5 and 85 → middle centre 45
        assert_eq!(bounds(&s, b), Rect::new(35, 0, 55, 10));
        s.undo();
        select_all(&mut s, &[a, b, c]);
        s.execute("layer.distribute.rightEdges", json!({})).unwrap();
        // right edges 10 and 100 → 55
        assert_eq!(bounds(&s, b), Rect::new(35, 0, 55, 10));
        s.undo();
        select_all(&mut s, &[a, b, c]);
        s.execute("layer.distribute.horizontally", json!({})).unwrap();
        // span 100, widths 60 → gaps of 20: b at 30..50
        assert_eq!(bounds(&s, b), Rect::new(30, 0, 50, 10));
        assert_eq!(bounds(&s, a), Rect::new(0, 0, 10, 10));
        assert_eq!(bounds(&s, c), Rect::new(70, 0, 100, 10));
        s.undo();
        assert_eq!(bounds(&s, b), Rect::new(12, 0, 32, 10));
    }
    // Vertical: tall layers with uneven heights.
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
    let b = rect_layer(&mut s, Rect::new(0, 50, 10, 54));
    let c = rect_layer(&mut s, Rect::new(0, 60, 10, 80));
    select_all(&mut s, &[a, b, c]);
    s.execute("layer.distribute.vertically", json!({})).unwrap();
    // span 80, heights 34 → gaps 23: b at 33..37
    assert_eq!(bounds(&s, b), Rect::new(0, 33, 10, 37));
    s.undo();
    select_all(&mut s, &[a, b, c]);
    s.execute("layer.distribute.topEdges", json!({})).unwrap();
    assert_eq!(bounds(&s, b), Rect::new(0, 30, 10, 34));
    s.undo();
    select_all(&mut s, &[a, b, c]);
    s.execute("layer.distribute.bottomEdges", json!({})).unwrap();
    assert_eq!(bounds(&s, b), Rect::new(0, 41, 10, 45));
    s.undo();
    select_all(&mut s, &[a, b, c]);
    s.execute("layer.distribute.verticalCenters", json!({})).unwrap();
    // centres 5 and 70 → 37.5, b is 4 tall → top 35.5 → rounds to 36 (35 for ties down)
    let y0 = bounds(&s, b).y0;
    assert!((35..=36).contains(&y0), "{y0}");
    // Two layers can't be distributed.
    select_all(&mut s, &[a, b]);
    assert!(!s.is_enabled("layer.distribute.leftEdges"));
}

#[test]
fn merge_keeps_pixels_outside_the_canvas() {
    // #5067: the merged layer was cropped to the 100x80 canvas.
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(-30, -20, 10, 10));
    let b = rect_layer(&mut s, Rect::new(90, 70, 140, 120));
    select_all(&mut s, &[a, b]);
    let m = LayerId(s.execute("layer.mergeLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    let ml = doc(&s).layer(m).unwrap();
    assert_eq!(layer_bounds(ml).unwrap(), Rect::new(-30, -20, 140, 120));
    assert_eq!(ml.surface().unwrap().pixel(-10, -5)[3], 1.0);
    assert_eq!(ml.surface().unwrap().pixel(130, 110)[3], 1.0);
}

#[test]
fn merge_down_and_merge_group_keep_pixels_outside_the_canvas() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(-30, -20, 10, 10));
    let b = rect_layer(&mut s, Rect::new(90, 70, 140, 120));
    s.execute("layer.mergeDown", json!({"layer": b.0})).unwrap();
    assert_eq!(layer_bounds(doc(&s).layer(a).unwrap()).unwrap(), Rect::new(-30, -20, 140, 120));

    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(-30, -20, 10, 10));
    let b = rect_layer(&mut s, Rect::new(90, 70, 140, 120));
    select_all(&mut s, &[a, b]);
    let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.mergeGroup", json!({"layer": g.0})).unwrap();
    assert_eq!(layer_bounds(doc(&s).layer(g).unwrap()).unwrap(), Rect::new(-30, -20, 140, 120));
}

#[test]
fn merge_selected_into_top_most() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
        let b = rect_layer(&mut s, Rect::new(20, 0, 30, 10));
        let c = rect_layer(&mut s, Rect::new(40, 0, 50, 10));
        s.execute("layer.renameLayer", json!({"layer": c.0, "name": "Top"})).unwrap();
        select_all(&mut s, &[a, c]);
        let before = doc(&s).layer_count();
        let m = LayerId(s.execute("layer.mergeLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        let d = doc(&s);
        assert_eq!(d.layer_count(), before - 1);
        let ml = d.layer(m).unwrap();
        assert_eq!(ml.name, "Top");
        assert_eq!(layer_bounds(ml).unwrap(), Rect::new(0, 0, 50, 10));
        assert_eq!(ml.surface().unwrap().pixel(5, 5)[..4], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(ml.surface().unwrap().pixel(25, 5)[3], 0.0, "b was not merged");
        assert!(d.layer(b).is_some() && d.layer(a).is_none());
        assert_eq!(d.layers.last().unwrap().id, m, "merged layer sits where the top-most was");
        assert_eq!(sel(&s), vec![m]);
        s.undo();
        assert_eq!(doc(&s).layer_count(), before);
        assert_eq!(sel(&s), vec![a, c], "undo selects the layers that were merged");
        // One layer selected: Merge Down.
        s.execute("layer.select", json!({"layer": c.0})).unwrap();
        s.execute("layer.mergeLayers", json!({})).unwrap();
        assert_eq!(doc(&s).layer_count(), before - 1);
    }
}

#[test]
fn merging_group_children_preserves_their_composite() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let lower = rect_layer(&mut s, Rect::new(10, 12, 40, 42));
        let upper = rect_layer(&mut s, Rect::new(25, 20, 55, 50));
        select_all(&mut s, &[lower, upper]);
        let group = LayerId(s.execute("layer.groupLayers", json!({"name": "Merge test group"})).unwrap()["layer"].as_u64().unwrap());
        let doc_before = doc(&s);
        let expected = photocraft_compose::render(doc_before, doc_before.bounds());
        let children = doc_before.layer(group).unwrap().children().unwrap().iter().map(|layer| layer.id).collect::<Vec<_>>();

        select_all(&mut s, &children);
        s.execute("layer.mergeLayers", json!({})).unwrap();
        let doc_after = doc(&s);
        let actual = photocraft_compose::render(doc_after, doc_after.bounds());
        assert_eq!(actual, expected, "depth {depth}: group merge changed rendered pixels");
        assert_eq!(doc_after.layer(group).unwrap().children().unwrap().len(), 1);
    }
}

#[test]
fn merging_keeps_layer_effects_outside_the_pixels() {
    // A drop shadow reaches past the layer's pixels; the merge must render that far.
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let lower = rect_layer(&mut s, Rect::new(10, 10, 30, 30));
        let upper = rect_layer(&mut s, Rect::new(40, 20, 60, 40));
        s.execute("layer.layerStyle.dropShadow", json!({"layer": upper.0, "blend": "normal", "distance": 12, "size": 4})).unwrap();
        let before = photocraft_compose::render(doc(&s), doc(&s).bounds());
        select_all(&mut s, &[lower, upper]);
        s.execute("layer.mergeLayers", json!({})).unwrap();
        let after = photocraft_compose::render(doc(&s), doc(&s).bounds());
        let worst = before.px.iter().zip(&after.px).map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
        assert!(worst <= 1.5 / 255.0, "depth {depth}: merge layers dropped the shadow (off by {worst})");
    }
}

#[test]
fn merging_a_group_down_preserves_the_composite() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let lower = rect_layer(&mut s, Rect::new(10, 12, 50, 52));
        let child_a = rect_layer(&mut s, Rect::new(24, 20, 64, 60));
        let child_b = rect_layer(&mut s, Rect::new(35, 28, 75, 68));
        select_all(&mut s, &[child_a, child_b]);
        let group = LayerId(s.execute("layer.groupLayers", json!({"name": "Merge down group"})).unwrap()["layer"].as_u64().unwrap());
        let doc_before = doc(&s);
        let expected = photocraft_compose::render(doc_before, doc_before.bounds());

        s.execute("layer.mergeDown", json!({"layer": group.0})).unwrap();
        let doc_after = doc(&s);
        let actual = photocraft_compose::render(doc_after, doc_after.bounds());
        assert_eq!(actual, expected, "depth {depth}: group merge down changed rendered pixels");
        assert!(doc_after.layer(lower).is_some());
        assert!(doc_after.layer(group).is_none());
    }
}

#[test]
fn a_locked_group_locks_its_contents() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let g = s.execute("layer.new.groupFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": g, "locked": true})).unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    for (cmd, p) in [
        ("layer.translate", json!({"dx": 1})),
        ("paint.stroke", json!({"points": [[2, 2], [4, 4]]})),
        ("edit.fill", json!({"color": "#00ff00"})),
        ("edit.transform.flipHorizontal", json!({})),
    ] {
        assert!(s.execute(cmd, p).is_err(), "{cmd} edited a layer in a locked group");
    }
    // Unlocking the group releases its contents.
    s.execute("layer.setProps", json!({"layer": g, "locked": false})).unwrap();
    s.execute("layer.translate", json!({"dx": 1})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(1, 0, 6, 5));
}

#[test]
fn a_locked_group_keeps_a_grouped_background_locked() {
    let mut s = session(8);
    let bg = doc(&s).layers[0].id;
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    let g = s.execute("layer.new.groupFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": g, "locked": true})).unwrap();
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    for (cmd, p) in [
        ("edit.transform.flipHorizontal", json!({})),
        ("edit.transform.warp", json!({"style": "flag", "bend": 50})),
        ("edit.puppetWarp", json!({"pins": [{"src": [30, 25], "dst": [36, 29]}]})),
    ] {
        let e = s.execute(cmd, p).unwrap_err().to_string();
        assert!(e.contains("locked"), "{cmd}: {e}");
        assert_eq!(doc(&s).layer(bg).unwrap().name, "Background", "{cmd}");
    }
}

#[test]
fn group_from_layers_preserves_order() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let c = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let d = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[c, a]);
    let g = LayerId(s.execute("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap()["layer"].as_u64().unwrap());
    let docu = doc(&s);
    let gl = docu.layer(g).unwrap();
    assert_eq!(gl.name, "Pair");
    assert!(matches!(&gl.content, LayerContent::Group(gr) if !gr.expanded), "created collapsed");
    assert_eq!(gl.children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![a, c]);
    // Root (bottom-to-top): Background, b, Pair (where c was), d.
    assert_eq!(docu.layers.iter().map(|l| l.id).skip(1).collect::<Vec<_>>(), vec![b, g, d]);
    assert_eq!(sel(&s), vec![g]);
    s.undo();
    assert_eq!(doc(&s).layers.len(), 5);
    // ⌘G with several selected groups them too.
    select_all(&mut s, &[b, d]);
    let g = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(doc(&s).layers.len(), 4);
    // Both create the group collapsed, as Photoshop does (#5571).
    assert!(matches!(&doc(&s).layer(g).unwrap().content, LayerContent::Group(gr) if !gr.expanded));
}

#[test]
fn grouping_is_capped_at_the_document_nesting_limit() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[a]);
    for _ in 0..photocraft_doc::MAX_GROUP_DEPTH {
        s.execute("layer.groupLayers", json!({})).unwrap();
    }
    assert_eq!(doc(&s).max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
    // One more wrapping group would pass the cap: rejected, document untouched.
    let err = s.execute("layer.groupLayers", json!({})).unwrap_err();
    assert!(err.to_string().contains("deeper than 100"), "{err}");
    assert_eq!(doc(&s).max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH);
    // The rejected call recorded no history step: undo still lands one grouping earlier.
    s.undo();
    assert_eq!(doc(&s).max_group_depth(), photocraft_doc::MAX_GROUP_DEPTH - 1);
}

#[test]
fn select_linked_layers_excludes_an_unlinked_active_layer() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
        let b = rect_layer(&mut s, Rect::new(20, 20, 30, 30));
        let other = rect_layer(&mut s, Rect::new(50, 50, 60, 60));
        select_all(&mut s, &[a, b]);
        s.execute("layer.linkLayers", json!({})).unwrap();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": other.0, "mode": "toggle"})).unwrap();
        assert_eq!(sel(&s), vec![a, other]);
        assert_eq!(s.active().unwrap().active_layer, Some(other));
        let past = s.active().unwrap().history.past_len();
        let dirty = s.active().unwrap().is_dirty();

        let result = s.execute("layer.selectLinkedLayers", json!({})).unwrap();
        assert_eq!(sel(&s), vec![a, b]);
        assert_eq!(result["selected"], sel(&s).len());
        assert_eq!(s.active().unwrap().active_layer, Some(b));
        assert_eq!(s.active().unwrap().layer_anchor, Some(b));
        assert_eq!(s.active().unwrap().history.past_len(), past);
        assert_eq!(s.active().unwrap().is_dirty(), dirty);

        // A following multi-layer delete must leave the unrelated layer intact.
        s.execute("layer.delete", json!({})).unwrap();
        assert!(doc(&s).layer(a).is_none() && doc(&s).layer(b).is_none());
        assert!(doc(&s).layer(other).is_some());
        s.undo();
        assert!(doc(&s).layer(a).is_some() && doc(&s).layer(b).is_some());
        assert!(doc(&s).layer(other).is_some());
    }
}

#[test]
fn select_linked_layers_preserves_a_linked_active_layer() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
        let b = rect_layer(&mut s, Rect::new(20, 20, 30, 30));
        select_all(&mut s, &[a, b]);
        s.execute("layer.linkLayers", json!({})).unwrap();
        for active in [a, b] {
            s.execute("layer.select", json!({"layer": active.0})).unwrap();
            let st = s.active_mut().unwrap();
            st.saved_revision = st.revision;
            let past = st.history.past_len();
            for _ in 0..2 {
                let result = s.execute("layer.selectLinkedLayers", json!({})).unwrap();
                assert_eq!(sel(&s), vec![a, b]);
                assert_eq!(result["selected"], sel(&s).len());
                assert_eq!(s.active().unwrap().active_layer, Some(active));
                assert_eq!(s.active().unwrap().layer_anchor, Some(active));
                assert_eq!(s.active().unwrap().history.past_len(), past);
                assert!(!s.active().unwrap().is_dirty());
            }
        }
    }
}

#[test]
fn select_linked_layers_requires_a_linked_selection() {
    let mut s = Session::new();
    assert!(!s.is_enabled("layer.selectLinkedLayers"));
    assert!(s.execute("layer.selectLinkedLayers", json!({})).is_err());
    let mut s = session(8);
    let before = sel(&s);
    assert!(!s.is_enabled("layer.selectLinkedLayers"));
    assert!(s.execute("layer.selectLinkedLayers", json!({})).is_err());
    assert_eq!(sel(&s), before);
}

#[test]
fn translate_rejects_offsets_that_would_wrap() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[a]);
    // 2^32 + 50 wrapped to `dx = 50` and 3e9 to a negative offset through `as i32`.
    for dx in [4_294_967_346_i64, 3_000_000_000_i64] {
        let err = s.execute("layer.translate", json!({"dx": dx, "dy": 0})).unwrap_err();
        assert!(err.to_string().contains("32-bit"), "{err}");
    }
    assert_eq!(bounds(&s, a), Rect::new(0, 0, 5, 5), "the layer never moved");
    // Large in-range offsets still work.
    s.execute("layer.translate", json!({"dx": -200_000, "dy": 200_000})).unwrap();
}

#[test]
fn link_layers_never_reuses_a_group_when_the_largest_id_is_u64_max() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
    let b = rect_layer(&mut s, Rect::new(20, 20, 30, 30));
    let c = rect_layer(&mut s, Rect::new(50, 50, 60, 60));
    let d = rect_layer(&mut s, Rect::new(70, 70, 80, 80));
    // A stored document can carry any id; `max + 1` once overflowed here.
    let st = s.active_mut().unwrap();
    for id in [c, d] {
        std::sync::Arc::make_mut(&mut st.doc).layer_mut(id).unwrap().link_group = Some(u64::MAX);
    }
    select_all(&mut s, &[a, b]);
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    let g = doc(&s).layer(a).unwrap().link_group;
    assert_eq!(g, Some(1));
    assert_eq!(doc(&s).layer(b).unwrap().link_group, g);
    for id in [c, d] {
        assert_eq!(doc(&s).layer(id).unwrap().link_group, Some(u64::MAX), "the existing group is untouched");
    }
}

#[test]
fn undo_and_redo_report_only_the_edited_area() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    select_all(&mut s, &[a]);
    // A pixel edit on one layer is bounded by that layer's extent on both sides.
    s.edit("paint", |doc, _| {
        doc.layer_mut(a).unwrap().surface_mut().unwrap().fill_rect(Rect::new(12, 12, 14, 14), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.undo();
    assert_eq!(damage(&s), Some(Rect::new(10, 10, 20, 20)));
    s.redo();
    assert_eq!(damage(&s), Some(Rect::new(10, 10, 20, 20)));
    // A move reports where the layer was and where it landed.
    s.execute("layer.translate", json!({"dx": 5, "dy": 0})).unwrap();
    s.undo();
    assert_eq!(damage(&s), Some(Rect::new(10, 10, 25, 20)), "union of where the layer was and is");
    s.redo();
    assert_eq!(damage(&s), Some(Rect::new(10, 10, 25, 20)));
}

#[test]
fn undo_of_document_wide_steps_still_composites_everything() {
    let mut s = session(8);
    let _ = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    // A canvas change has no bounded damage.
    s.execute("image.crop", json!({"x": 0, "y": 0, "width": 30, "height": 25})).unwrap();
    assert_ne!(doc(&s).size, photocraft_doc::Size::new(100, 80));
    s.undo();
    assert_eq!(damage(&s), None, "a size change recomposites the whole canvas");
    // A layer removed and restored changes only where it draws.
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[b]);
    s.execute("layer.delete", json!({})).unwrap();
    s.undo();
    assert_eq!(damage(&s), Some(Rect::new(0, 0, 5, 5)));
}

#[test]
fn undo_of_compositor_read_document_fields_composites_everything() {
    let mut s = session(8);
    let _ = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    // Global Light changes every effect's angle across the document.
    s.execute("layer.layerStyle.globalLight", json!({"angle": 90.0})).unwrap();
    s.undo();
    assert_eq!(damage(&s), None, "a global-light change recomposites everything");
    // A mode change that keeps the pixel format (RGB → Indexed maps to RGB) neither.
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    s.undo();
    assert_eq!(damage(&s), None, "a mode change recomposites everything");
}

#[test]
fn undo_of_clipping_or_channel_exclusion_composites_everything() {
    let mut s = session(8);
    let _ = rect_layer(&mut s, Rect::new(10, 10, 20, 20));
    let b = rect_layer(&mut s, Rect::new(12, 12, 30, 30));
    select_all(&mut s, &[b]);
    // Clipping reaches the layers clipped above, outside this layer's bounds.
    s.execute("layer.setProps", json!({"clipped": true})).unwrap();
    s.undo();
    assert_eq!(damage(&s), None, "a clipping change recomposites everything");
    s.execute("layer.setProps", json!({"channels": [true, false, true]})).unwrap();
    s.undo();
    assert_eq!(damage(&s), None, "an excluded-channel change recomposites everything");
}

#[test]
fn linked_layers_move_together_and_undo() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let a = rect_layer(&mut s, Rect::new(0, 0, 10, 10));
        let b = rect_layer(&mut s, Rect::new(20, 20, 30, 30));
        let c = rect_layer(&mut s, Rect::new(50, 50, 60, 60));
        assert!(!s.is_enabled("layer.linkLayers"));
        select_all(&mut s, &[a, b]);
        assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
        let g = doc(&s).layer(a).unwrap().link_group;
        assert!(g.is_some() && doc(&s).layer(b).unwrap().link_group == g);
        // Move only a: b follows, c stays.
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.translate", json!({"dx": 5, "dy": 3})).unwrap();
        assert_eq!(bounds(&s, a), Rect::new(5, 3, 15, 13));
        assert_eq!(bounds(&s, b), Rect::new(25, 23, 35, 33));
        assert_eq!(bounds(&s, c), Rect::new(50, 50, 60, 60));
        s.undo();
        assert_eq!(bounds(&s, a), Rect::new(0, 0, 10, 10));
        assert_eq!(bounds(&s, b), Rect::new(20, 20, 30, 30));
        // Select Linked Layers from just a.
        s.execute("layer.selectLinkedLayers", json!({})).unwrap();
        assert_eq!(sel(&s), vec![a, b]);
        // Multi-selection translate moves all selected (c added, unlinked).
        s.execute("layer.select", json!({"layer": c.0, "mode": "toggle"})).unwrap();
        s.execute("layer.translate", json!({"dx": -1, "dy": 0})).unwrap();
        assert_eq!(bounds(&s, c), Rect::new(49, 50, 59, 60));
        assert_eq!(bounds(&s, a), Rect::new(-1, 0, 9, 10));
        // Toggle: linking the same group again unlinks.
        select_all(&mut s, &[a, b]);
        assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], false);
        assert!(doc(&s).layer(a).unwrap().link_group.is_none());
    }
}

/// Plain layers carrying stored link groups, as a loaded `.pcraft` or PSD can hold (#961).
fn stored_groups(s: &mut Session, groups: &[Option<u64>]) -> Vec<LayerId> {
    let ids: Vec<LayerId> = groups.iter().map(|_| rect_layer(s, Rect::new(0, 0, 4, 4))).collect();
    s.edit("load groups", |doc, _| {
        for (id, g) in ids.iter().zip(groups) {
            doc.layer_mut(*id).unwrap().link_group = *g;
        }
        Ok(())
    })
    .unwrap();
    ids
}

fn group(s: &Session, id: LayerId) -> Option<u64> {
    doc(s).layer(id).unwrap().link_group
}

/// Links `ids`, then checks they share one group no other layer uses (#961).
fn link_fresh(s: &mut Session, ids: &[LayerId]) -> u64 {
    select_all(s, ids);
    let past = s.active().unwrap().history.past_len();
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    let g = group(s, ids[0]).expect("linked");
    assert!(ids.iter().all(|id| group(s, *id) == Some(g)));
    let members = doc(s).walk().iter().filter(|(_, _, l)| l.link_group == Some(g)).count();
    assert_eq!(members, ids.len(), "group {g} is shared with unselected layers");
    g
}

#[test]
fn link_layers_beside_a_maximal_stored_group() {
    // The issue's repro: one layer stored at u64::MAX, link it with an unlinked one.
    let mut s = session(8);
    let ids = stored_groups(&mut s, &[Some(u64::MAX), None]);
    assert_ne!(link_fresh(&mut s, &ids), u64::MAX);
    // Unlink and relink the pair: with no groups left, numbering starts over at 1.
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], false);
    assert!(ids.iter().all(|id| group(&s, *id).is_none()));
    assert_eq!(link_fresh(&mut s, &ids), 1);

    // Two other layers linked beside an untouched maximal pair.
    let mut s = session(8);
    let ids = stored_groups(&mut s, &[Some(u64::MAX), Some(u64::MAX), None, None]);
    let g = link_fresh(&mut s, &ids[2..]);
    assert_ne!(g, u64::MAX);
    assert_eq!((group(&s, ids[0]), group(&s, ids[1])), (Some(u64::MAX), Some(u64::MAX)));
}

#[test]
fn unlink_a_maximal_stored_group() {
    let mut s = session(8);
    let ids = stored_groups(&mut s, &[Some(u64::MAX), Some(u64::MAX), None]);
    select_all(&mut s, &ids[..2]);
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], false);
    assert!(ids.iter().all(|id| group(&s, *id).is_none()));
    s.undo();
    assert_eq!((group(&s, ids[0]), group(&s, ids[1])), (Some(u64::MAX), Some(u64::MAX)));
    // Relinking all three gets a working group.
    link_fresh(&mut s, &ids);
}

#[test]
fn link_layers_when_every_group_is_huge() {
    // Unselected groups at the top of the range, including the 0 a wrapped id would join.
    let top = [u64::MAX, u64::MAX - 1, u64::MAX - 2, 0, 1, 2];
    let stored: Vec<Option<u64>> = top.iter().flat_map(|g| [Some(*g), Some(*g)]).chain([None, None, None, None]).collect();
    let mut s = session(8);
    let ids = stored_groups(&mut s, &stored);
    let n = top.len() * 2;
    let g = link_fresh(&mut s, &ids[n..n + 2]);
    assert!(!top.contains(&g), "{g} collides with a stored group");
    // A second link gets yet another group, and existing groups never change.
    let h = link_fresh(&mut s, &ids[n + 2..]);
    assert!(!top.contains(&h) && h != g);
    for (i, t) in top.iter().enumerate() {
        assert_eq!((group(&s, ids[2 * i]), group(&s, ids[2 * i + 1])), (Some(*t), Some(*t)));
    }
    // Moving a newly linked layer leaves the group-0 pair alone.
    assert_eq!(with_links(doc(&s), &[ids[n]]), vec![ids[n], ids[n + 1]]);
}

#[test]
fn link_layers_near_the_maximum() {
    // max + 1 still fits: the new group is u64::MAX itself.
    let mut s = session(8);
    let ids = stored_groups(&mut s, &[Some(u64::MAX - 1), Some(u64::MAX - 1), None, None, None, None]);
    assert_eq!(link_fresh(&mut s, &ids[2..4]), u64::MAX);
    // The next link no longer fits above the maximum and takes a free id instead.
    let g = link_fresh(&mut s, &ids[4..]);
    assert!(g != u64::MAX && g != u64::MAX - 1);
}

#[test]
fn link_layers_assigns_working_groups() {
    // Control: ordinary documents keep getting max + 1, starting at 1.
    let mut s = session(8);
    let ids = stored_groups(&mut s, &[None, None, None, None]);
    assert_eq!(link_fresh(&mut s, &ids[..2]), 1);
    assert_eq!(link_fresh(&mut s, &ids[2..]), 2);
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], false);
    assert_eq!((group(&s, ids[2]), group(&s, ids[3])), (None, None));
    assert_eq!((group(&s, ids[0]), group(&s, ids[1])), (Some(1), Some(1)));
}

#[test]
fn selection_pruned_after_delete_and_undo() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let c = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[a, c]);
    // Deleting one layer explicitly prunes it from the set.
    s.execute("layer.delete", json!({"layer": a.0})).unwrap();
    assert_eq!(sel(&s), vec![c]);
    s.undo();
    assert!(sel(&s).iter().all(|id| doc(&s).layer(*id).is_some()));
    // Deleting with several selected removes them all, then selects the layer below.
    select_all(&mut s, &[a, c]);
    s.execute("layer.delete", json!({})).unwrap();
    assert!(doc(&s).layer(a).is_none() && doc(&s).layer(c).is_none());
    assert_eq!(doc(&s).layer_count(), 2);
    assert_eq!(sel(&s).len(), 1);
    assert!(s.active().unwrap().selected_layers.iter().all(|id| doc(&s).layer(*id).is_some()));
    s.undo();
    assert_eq!(doc(&s).layer_count(), 4);
    let _ = b;
    // Redo then undo again keeps the set valid.
    s.redo();
    assert!(sel(&s).iter().all(|id| doc(&s).layer(*id).is_some()));
}

#[test]
fn duplicate_hide_lock_and_reverse_selected() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let c = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[a, c]);
    let r = s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    assert_eq!(doc(&s).layer_count(), 6);
    assert_eq!(sel(&s).len(), 2);
    assert!(sel(&s).iter().all(|id| *id != a && *id != c));
    s.undo();
    select_all(&mut s, &[a, b]);
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(!doc(&s).layer(a).unwrap().visible && !doc(&s).layer(b).unwrap().visible && doc(&s).layer(c).unwrap().visible);
    s.execute("layer.showLayers", json!({})).unwrap();
    assert!(doc(&s).layer(a).unwrap().visible);
    s.execute("layer.lockLayers", json!({})).unwrap();
    assert!(doc(&s).layer(a).unwrap().locks.all && doc(&s).layer(b).unwrap().locks.all);
    assert!(s.execute("layer.translate", json!({"dx": 1})).is_err(), "locked layers don't move");
    s.execute("layer.lockLayers", json!({})).unwrap();
    assert!(!doc(&s).layer(a).unwrap().locks.all);
    s.execute("layer.lockLayers", json!({"position": true})).unwrap();
    assert!(doc(&s).layer(b).unwrap().locks.position && !doc(&s).layer(c).unwrap().locks.position);
    // Reverse a, b, c.
    select_all(&mut s, &[a, b, c]);
    s.execute("layer.arrange.reverse", json!({})).unwrap();
    assert_eq!(doc(&s).layers.iter().skip(1).map(|l| l.id).collect::<Vec<_>>(), vec![c, b, a]);
    assert_eq!(s.active().unwrap().history.entries().last().map(|e| e.as_str()), Some("Reverse"));
}

/// #4892: Lock Image Pixels includes Lock Transparent Pixels, so the two are never both on:
/// locking the pixels clears the transparency lock, which stays off while they are locked,
/// through both `layer.setProps` and `layer.lockLayers`.
#[test]
fn locking_pixels_clears_and_holds_off_the_transparency_lock() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let id = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let locks = |s: &Session| doc(s).layer(id).unwrap().locks;
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    assert!(locks(&s).transparency);
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    assert!(locks(&s).pixels && !locks(&s).transparency, "locking the pixels clears the transparency lock");
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    assert!(!locks(&s).transparency, "and it stays off while the pixels are locked");
    s.execute("layer.setProps", json!({"locks": {"transparency": true, "pixels": true}})).unwrap();
    assert!(locks(&s).pixels && !locks(&s).transparency, "both at once: the pixel lock wins");
    s.execute("layer.lockLayers", json!({"pixels": false})).unwrap();
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    assert!(locks(&s).transparency && !locks(&s).pixels, "with the pixels unlocked it can be set again");
    s.execute("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(locks(&s).pixels && !locks(&s).transparency);
    assert!(s.execute("layer.setProps", json!({"locks": {"nope": true}})).is_err());
}

/// `document.inspect` lists the selected layers and marks them in the layer list.
#[test]
fn inspect_reports_selection() {
    let mut s = session(8);
    let a = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    let b = rect_layer(&mut s, Rect::new(0, 0, 5, 5));
    select_all(&mut s, &[a, b]);
    let v = s.execute("document.inspect", json!({})).unwrap();
    assert_eq!(v["selectedLayers"], json!([a.0, b.0]));
    assert_eq!(v["layers"][0]["selected"], true);
}

/// #7538: `selection: true` moves the selection with the layers in the same step, as the Move
/// tool does with several layers selected; without it the selection stays.
#[test]
fn translate_can_carry_the_selection_in_the_same_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100, "background": "transparent"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 70, "height": 30})).unwrap();
    let sel = |s: &Session| s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds());
    let bounds = |s: &Session, id: LayerId| s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
    let steps = s.active().unwrap().history.past_len();
    s.execute("layer.translate", json!({"dx": 7, "dy": 20, "selection": true})).unwrap();
    assert_eq!((bounds(&s, a), bounds(&s, b)), (Rect::new(17, 30, 37, 50), Rect::new(57, 30, 77, 50)));
    assert_eq!(sel(&s), Some(Rect::new(12, 25, 82, 55)), "the selection came along");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
    s.undo();
    assert_eq!((sel(&s), bounds(&s, a)), (Some(Rect::new(5, 5, 75, 35)), Rect::new(10, 10, 30, 30)), "undone together");
    s.execute("layer.translate", json!({"dx": 7, "dy": 20})).unwrap();
    assert_eq!(sel(&s), Some(Rect::new(5, 5, 75, 35)), "without it the selection stays");
    let state = |s: &Session| (bounds(s, a), bounds(s, b), sel(s), s.active().unwrap().history.past_len());
    let before = state(&s);
    for dx in [1, 0] {
        assert!(s.execute("layer.translate", json!({"dx": dx, "selection": "yes"})).is_err(), "dx {dx}: a bad `selection` is an error");
        assert_eq!(state(&s), before, "dx {dx}: and changes nothing");
    }
}
