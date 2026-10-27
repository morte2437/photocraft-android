use super::*;

fn session() -> Session {
    session_depth(8)
}

fn session_depth(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// The content bounds of the active layer's pixels.
fn active_bounds(s: &Session) -> Rect {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

// ---- Memory budget (#3773, #7807). Tests run with a 20 MB budget (2²⁰ px of 32-bit CMYK with
// alpha), so nothing here comes near the real one. ----

/// Memory Usage's default, in bytes.
const DEFAULT_BUDGET: u64 = (crate::prefs::DEFAULT_MEMORY_USAGE_MB as u64) << 20;

/// Scales of 10⁷ and past are refused by the real budget too, whatever the axis or sign.
#[test]
fn huge_transform_scales_exceed_the_production_budget_before_clipping() {
    let src = Rect::new(0, 0, 24, 16);
    for scale in [1e7, 1e28, 1e306] {
        for (sx, sy) in [(scale, 1.0), (1.0, scale), (-scale, 1.0)] {
            let h = Homography([sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0]);
            assert!(check_budget_with_limit(src, &h, "layer", PixelFormat::RGBA8, DEFAULT_BUDGET).is_err(), "{sx} x {sy} must be refused");
        }
    }
}

/// A 240000 px wide stretch, placed far from the canvas but within the warp's reach, fits.
#[test]
fn an_ordinary_transform_remains_within_the_production_budget() {
    let src = Rect::new(0, 0, 24, 16);
    let h = Homography([10_000.0, 0.0, 800_000.0, 0.0, 1.0, -900_000.0, 0.0, 0.0, 1.0]);
    assert!(check_budget_with_limit(src, &h, "layer", PixelFormat::RGBA8, DEFAULT_BUDGET).is_ok());
}

/// The warp places pixels only within ±2²⁰ px ([`WARP_REACH`]): a result that fits Memory
/// Usage but would reach past it, by its size or by where it lands, is refused, not clipped.
#[test]
fn a_result_past_the_warps_reach_is_refused_not_clipped() {
    let src = Rect::new(0, 0, 24, 16);
    let check = |h: [f64; 9]| check_budget_with_limit(src, &Homography(h), "layer", PixelFormat::RGBA8, DEFAULT_BUDGET);
    // 2000016 x 18 px, 138 MB of 8-bit RGBA: from x = 0 it would be cut at 2²⁰; centred it fits.
    let err = check([83_334.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap_err().to_string();
    assert!(err.contains("would reach past ±1048576 px") && err.ends_with("the document was not changed"), "{err}");
    assert!(check([83_334.0, 0.0, -1_000_000.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_ok());
    // A small layer moved past the reach, either way on either axis.
    for (dx, dy) in [(1_100_000.0, 0.0), (-1_100_000.0, 0.0), (0.0, 1_100_000.0), (0.0, -1_100_000.0)] {
        assert!(check([1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0]).is_err(), "{dx}, {dy}");
    }
    // Through the command: nothing moves, nothing is lost.
    let mut s = session();
    let st = s.active().unwrap();
    let (before, steps) = (st.doc.clone(), st.history.past_len());
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 2_000_000, 0]})).unwrap_err().to_string();
    assert!(err.contains("the transformed layer \"Layer 1\" would reach past ±1048576 px"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before && st.history.past_len() == steps, "the document is untouched");
}

/// #7807: the stitched layer's 90 % and 15°, a 37708 x 35259 px result that the old 2³⁰ px limit
/// refused, fits the default Memory Usage at 8 bits (5072 MB); at 16 bits it needs Memory Usage
/// raised, and the message says so.
#[test]
fn a_gigapixel_layer_transforms_within_memory_usage() {
    let src = Rect::new(0, 0, 35_020, 31_171);
    let r = [0.0, 0.0, 35_020.0, 31_171.0];
    let h = relative_matrix(&json!({"scaleX": 90, "scaleY": 90, "rotate": 15}), r).unwrap().unwrap();
    assert_eq!(warp_output_size(src, &h), Some((37_708, 35_259)), "the report's result, past the old limit");
    assert!(check_budget_with_limit(src, &h, "layer \"Layer 1\"", PixelFormat::RGBA8, DEFAULT_BUDGET).is_ok());
    let err = check_budget_with_limit(src, &h, "layer \"Layer 1\"", PixelFormat::RGBA16, DEFAULT_BUDGET).unwrap_err().to_string();
    assert!(err.contains("(10144 MB), more than the 8192 MB Memory Usage allows (Preferences › Performance)"), "{err}");
    assert!(check_budget_with_limit(src, &h, "layer", PixelFormat::RGBA16, 2 * DEFAULT_BUDGET).is_ok());
    // A format without alpha is priced as the warp makes it, with alpha.
    let rgb8 = PixelFormat::new(PixelFormat::RGBA8.mode, PixelFormat::RGBA8.sample, false);
    assert!(check_budget_with_limit(src, &h, "layer", rgb8, 5071 << 20).is_err());
}

/// A 16 x 16 document whose active layer is a 1 x 8 column at x = 10.
fn column() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 2, 11, 10), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Stretch x by `k` about x = 10 (the column's left edge).
fn stretch(k: f64) -> Value {
    json!({"matrix": [k, 0, 0, 1, -10.0 * k, 0], "interpolation": "nearest"})
}

/// A stretch past the budget is refused with the limit named, and the document, its revision
/// and History stay as they were; within the budget the same stretch applies unclipped.
#[test]
fn a_scale_past_the_pixel_budget_is_refused_and_changes_nothing() {
    let mut s = column();
    let st = s.active().unwrap();
    let (before, revision, steps) = (st.doc.clone(), st.revision, st.history.past_len());
    // 1000000 x 10 px of 8-bit RGBA output: about twice the test budget.
    let err = s.execute("edit.transform", stretch(1e6)).unwrap_err().to_string();
    assert!(err.starts_with("too large to transform: the transformed layer \"Layer 1\""), "{err}");
    assert!(
        err.contains("would be 1000002 x 10 px (39 MB), more than the 20 MB Memory Usage allows") && err.ends_with("the document was not changed"),
        "{err}"
    );
    // Nothing was allocated, so nothing ran short (#7807).
    assert!(!err.contains("not enough memory"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before, "the document is untouched");
    assert_eq!((st.revision, st.history.past_len()), (revision, steps));
    // Within the budget the same stretch still works, nothing clipped.
    s.execute("edit.transform", stretch(1e4)).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(0, 2, 10_000, 10));
}

#[test]
fn a_group_with_one_layer_past_the_budget_is_not_half_transformed() {
    let mut s = column();
    let wide = s.active().unwrap().active_layer.unwrap();
    // Over the column: a 1 px dot, which fits the budget stretched by the same amount.
    s.execute("layer.new.layer", json!({})).unwrap();
    let dot = s.active().unwrap().active_layer.unwrap();
    s.edit("paint", |doc, _| {
        doc.layer_mut(dot).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 12, 11, 13), &[0.0, 1.0, 0.0, 1.0]);
        // Widen the column to 8 px (x 2..10), so it alone needs 800002 x 10 px (31 MB).
        doc.layer_mut(wide).unwrap().surface_mut().unwrap().fill_rect(Rect::new(2, 2, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": wide.0})).unwrap();
    s.execute("layer.select", json!({"layer": dot.0, "mode": "add"})).unwrap();
    let group = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    let st = s.active().unwrap();
    assert!(matches!(&st.doc.layer(group).unwrap().content, LayerContent::Group(g) if g.children.len() == 2));
    let (before, revision) = (st.doc.clone(), st.revision);
    let k = 1e5;
    assert!(s.execute("edit.transform", json!({"layer": group.0, "matrix": [k, 0, 0, 1, -10.0 * k, 0]})).is_err());
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision, "nothing changed");
    // The layer-level check runs before any child is warped, so even the layer itself
    // (not just the edit's copy of the document) is left as it was.
    let mut l = st.doc.layer(group).unwrap().clone();
    let h = Homography([k, 0.0, -10.0 * k, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert!(transform_layer(None, Locks::default(), &mut l, &h, None, Interp::Nearest).is_err());
    assert!(&l == st.doc.layer(group).unwrap(), "no child was transformed");
}

/// Only the result's size is priced: a small layer moved 900000 px away (within the warp's
/// reach) and back keeps every pixel.
#[test]
fn moving_a_small_layer_far_off_the_canvas_is_within_budget() {
    let mut s = session();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 900_000, -900_000]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(900_010, -899_990, 900_030, -899_980));
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, -900_000, 900_000]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "and back, every pixel kept");
}

/// Each priced at its own format, as the warp makes it: an 8-bit outline, mask or channel with
/// alpha is 2 bytes a pixel, so a canvas-high column of it stretched 10⁶ times (34 MB) is over
/// the test budget.
#[test]
fn selections_masks_and_channels_have_the_same_budget() {
    // The selection outline of a layer transform.
    let mut s = column();
    s.execute("select.rect", json!({"x": 10, "y": 0, "width": 1, "height": 16})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let err = s.execute("edit.transform", stretch(1e6)).unwrap_err().to_string();
    assert!(err.contains("transformed selection would be"), "{err}");
    // Transform Selection.
    let err = s.execute("select.transformSelection", stretch(1e6)).unwrap_err().to_string();
    assert!(err.contains("transformed selection would be"), "{err}");
    assert!(s.active().unwrap().doc == before);
    // A targeted alpha channel.
    let mut s = column();
    s.execute("channel.new", json!({})).unwrap();
    s.edit("paint channel", |doc, _| {
        doc.channels[0].surface.fill_rect(Rect::new(10, 0, 11, 16), &[1.0]);
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    let mut p = stretch(1e6);
    p["target"] = json!({"channel": 0});
    let err = s.execute("edit.transform", p).unwrap_err().to_string();
    assert!(err.contains("transformed mask or channel would be"), "{err}");
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn scale_via_quad_and_undo() {
    let mut s = session();
    s.execute("edit.transform", json!({"quad": [[10, 10], [50, 10], [50, 30], [10, 30]]})).unwrap();
    let b = active_bounds(&s);
    assert!(b.width().abs_diff(40) <= 2 && b.height().abs_diff(20) <= 2, "{b:?}");
    s.undo();
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20));
}

#[test]
fn affine_matrix_and_perspective() {
    let mut s = session();
    // Translate by (40, 50).
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
    // Perspective: top edge narrower than the bottom.
    s.execute("edit.transform", json!({"quad": [[55, 60], [65, 60], [75, 70], [45, 70]]})).unwrap();
    let b = active_bounds(&s);
    assert!(b.x0 <= 46 && b.x1 >= 74, "{b:?}");
    assert!(s.execute("edit.transform", json!({"quad": [[0, 0], [0, 0], [0, 0], [0, 0]]})).is_err());
}

#[test]
fn background_becomes_layer_0() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    s.execute("edit.transform", json!({"quad": [[0, 0], [20, 0], [20, 20], [0, 20]]})).unwrap();
    let st = s.active().unwrap();
    let l = &st.doc.layers[0];
    assert_eq!(l.name, "Layer 0");
    assert!(l.surface().unwrap().format().alpha);
    assert_eq!(l.surface().unwrap().pixel(30, 30)[3], 0.0, "revealed area is transparent");
}

/// `locked_for_transform` (what Free Transform asks before its box opens, #2585) agrees with
/// what `edit.transform` refuses at commit: a position lock, its own or a group's; never the
/// Background's (it becomes Layer 0).
#[test]
fn locked_for_transform_matches_the_commit() {
    let quad = json!({"quad": [[10, 10], [50, 10], [50, 30], [10, 30]]});
    let lock = |s: &mut Session, id: LayerId, on: bool| {
        s.edit("lock", |doc, _| {
            doc.layer_mut(id).unwrap().locks.position = on;
            Ok(())
        })
        .unwrap();
    };
    let mut s = session();
    let layer = s.active().unwrap().active_layer.unwrap();
    lock(&mut s, layer, true);
    assert!(locked_for_transform(&s.active().unwrap().doc, layer));
    assert!(s.execute("edit.transform", quad.clone()).is_err());
    lock(&mut s, layer, false);
    let group = LayerId(s.execute("layer.groupLayers", json!({"layer": layer.0})).unwrap()["layer"].as_u64().unwrap());
    lock(&mut s, group, true);
    s.execute("layer.select", json!({"layer": layer.0})).unwrap();
    assert!(locked_for_transform(&s.active().unwrap().doc, layer), "the group's lock reaches the layer");
    assert!(s.execute("edit.transform", quad.clone()).is_err());
    let bg = s.active().unwrap().doc.layers[0].id;
    assert!(!locked_for_transform(&s.active().unwrap().doc, bg), "the Background transforms");
}

#[test]
fn with_selection_only_selected_pixels_move() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_eq!(surf.pixel(15, 15)[3], 0.0, "lifted area is cleared");
    assert_eq!(surf.pixel(25, 15)[3], 1.0, "unselected part stays");
    assert_eq!(surf.pixel(15, 65)[3], 1.0, "moved pixels land");
    let sel = st.doc.selection.as_ref().unwrap().content_bounds();
    assert_eq!((sel.y0, sel.y1), (60, 70), "selection moves too");
}

#[test]
fn with_selection_bounds_drop_the_vacated_area() {
    // #1097: moving every pixel through a selection left transparent but coloured pixels
    // behind, so the layer bounds were the union of the old and new positions.
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 50, 0]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(60, 10, 80, 20));
}

#[test]
fn type_layers_transform_as_vectors() {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi", "size": 20})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let w0 = active_bounds(&s).width();
    s.execute("edit.transform", json!({"layer": id, "matrix": [2, 0, 0, 2, -10, -50]})).unwrap();
    let w1 = active_bounds(&s).width();
    assert!(w1 as f32 > w0 as f32 * 1.8, "{w0} → {w1}");
    let st = s.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!() };
    assert!((t.transform.m[0] - 2.0).abs() < 1e-9);
    // Perspective on type is refused, like Photoshop.
    assert!(s.execute("edit.transform", json!({"layer": id, "quad": [[0, 0], [10, 0], [12, 10], [-2, 10]]})).is_err());
}

#[test]
fn shapes_move_and_transform_as_vectors() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
    s.execute("layer.translate", json!({"dx": 30, "dy": 5})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(40, 15, 80, 35));
    s.execute("edit.transform", json!({"matrix": [2, 0, 0, 2, -40, -15]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(40, 15, 120, 55));
    // Distort maps the path and keeps a live shape (#3009): the rectangle's corners go to the quad.
    let quad = [[40.0, 15.0], [120.0, 15.0], [130.0, 55.0], [30.0, 55.0]];
    s.execute("edit.transform", json!({ "quad": quad })).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("still a shape layer") };
    let mut anchors: Vec<[f64; 2]> = sh.path.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    let mut want = quad.to_vec();
    anchors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    want.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(anchors.iter().zip(&want).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6), "{anchors:?}");
    assert!(sh.live.is_none(), "the live rectangle is now a path");
    assert_eq!(active_bounds(&s), Rect::new(30, 15, 130, 55));
    // Warp bends the path too: the arc adds anchors along the curved edges.
    s.execute("edit.transform.warp", json!({"style": "arc", "bend": 50})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("still a shape layer") };
    assert!(sh.path.subpaths[0].knots.len() > 4, "{}", sh.path.subpaths[0].knots.len());
    assert!(!active_bounds(&s).is_empty());
}

/// A gradient fill layer whose mask (a selection when it was made) frames the gradient over
/// 10..40, with the cached render a PSD import gives it: exactly the pixels the fill renders.
fn cached_gradient_fill() -> Session {
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("layer.newFillLayer.gradient", json!({"from": "#ff0000", "to": "#0000ff", "angle": 0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.edit("cache", |doc, active| {
        let (fmt, canvas) = (doc.pixel_format(), doc.bounds());
        let l = doc.layer_mut(active.unwrap()).unwrap();
        let LayerContent::Fill(f) = &l.content else { panic!("a fill layer") };
        let buf = photocraft_compose::render_fill_content(l, f, canvas, &[]);
        let mut surface = Surface::new(fmt);
        let vals: Vec<f32> = buf.px.iter().flat_map(|p| photocraft_raster::from_rgba(&fmt, *p)).collect();
        surface.write_region(canvas, &vals);
        l.fill_cache = Some(photocraft_doc::FillCache { fill: f.clone(), surface });
        Ok(())
    })
    .unwrap();
    s
}

/// How far the composite is from the composite with the fill re-rendered from its model
/// (cache dropped): the largest channel difference.
fn off_from_fresh(s: &Session) -> f32 {
    let st = s.active().unwrap();
    let mut fresh: Document = (*st.doc).clone();
    fresh.layer_mut(st.active_layer.unwrap()).unwrap().fill_cache = None;
    let (a, b) = (photocraft_compose::render(&st.doc, st.doc.bounds()).px, photocraft_compose::render(&fresh, fresh.bounds()).px);
    a.iter().zip(&b).flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs())).fold(0.0, f32::max)
}

#[test]
fn transforming_a_fill_layer_drops_its_stale_cached_render() {
    let mut s = cached_gradient_fill();
    let off = off_from_fresh(&s);
    assert!(off < 0.01, "the cache is the fill's own render: {off}");
    // The mask (and so the gradient's frame) moves 50 px right and doubles in height.
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 2, 50, 0]})).unwrap();
    let off = off_from_fresh(&s);
    assert!(off < 0.01, "the composite shows the fill laid out in its new frame, not the old render: {off}");
}

#[test]
fn warping_a_fill_layer_drops_its_stale_cached_render() {
    let mut s = cached_gradient_fill();
    s.execute("edit.transform.warp", json!({"style": "arc", "bend": 50})).unwrap();
    let off = off_from_fresh(&s);
    assert!(off < 0.01, "the composite shows the fill laid out in its new frame, not the old render: {off}");
}

#[test]
fn type_moves_with_the_move_command() {
    let mut s = session();
    s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi", "size": 20})).unwrap();
    let b0 = active_bounds(&s);
    s.execute("layer.translate", json!({"dx": 20, "dy": 10})).unwrap();
    let b1 = active_bounds(&s);
    assert_eq!((b1.x0 - b0.x0, b1.y0 - b0.y0), (20, 10));
}

// ---- Layer masks and targets (#205) ----

/// `session()` plus a reveal-all mask with `hide` painted black (linked unless `linked` is false).
fn masked(hide: &[Rect], linked: bool) -> Session {
    masked_depth(hide, linked, 8)
}

fn masked_depth(hide: &[Rect], linked: bool, depth: u32) -> Session {
    let mut s = session_depth(depth);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.edit("mask", |doc, active| {
        let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
        for r in hide {
            m.surface.fill_rect(*r, &[0.0]);
        }
        m.linked = linked;
        Ok(())
    })
    .unwrap();
    s
}

fn mask_at(s: &Session, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
}

fn alpha_at(s: &Session, x: i32, y: i32) -> f32 {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
}

#[test]
fn linked_raster_mask_moves_with_the_pixels() {
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], true);
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
    assert!(mask_at(&s, 55, 65) < 0.01, "the hidden half moved with the pixels");
    assert!(mask_at(&s, 65, 65) > 0.99);
    assert!(mask_at(&s, 15, 15) > 0.99, "nothing hidden left behind");
}

#[test]
fn linked_mask_with_a_selection_moves_only_the_selected_region() {
    // The selection covers the left half of the red rect; the mask hides its left quarter
    // (selected) and its right quarter (not selected).
    let mut s = masked(&[Rect::new(10, 10, 15, 20), Rect::new(25, 10, 30, 20)], true);
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
    // Moved pixels and their mask line up.
    assert_eq!(alpha_at(&s, 12, 65), 1.0);
    assert!(mask_at(&s, 12, 65) < 0.01, "selected hidden quarter moved");
    assert!(mask_at(&s, 17, 65) > 0.99, "selected visible quarter moved");
    // The vacated area reads as revealed (default), with transparent pixels.
    assert_eq!(alpha_at(&s, 12, 15), 0.0);
    assert!(mask_at(&s, 12, 15) > 0.99);
    // The unselected part of the mask stays, aligned with the unselected pixels.
    assert_eq!(alpha_at(&s, 27, 15), 1.0);
    assert!(mask_at(&s, 27, 15) < 0.01, "unselected mask stays put");
    assert!(mask_at(&s, 27, 65) > 0.99, "and isn't copied along");
    // Every pixel's mask value is where it was relative to the pixel.
    for (x, y) in [(11, 61), (14, 69), (15, 61), (19, 69)] {
        let want = if x < 15 { 0.0 } else { 1.0 };
        assert!((mask_at(&s, x, y) - want).abs() < 0.01, "({x},{y})");
    }
}

#[test]
fn unlinked_mask_stays_put() {
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
    assert!(mask_at(&s, 15, 15) < 0.01);
    assert!(mask_at(&s, 55, 65) > 0.99);
    // With a selection too.
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50]})).unwrap();
    assert!(mask_at(&s, 15, 15) < 0.01 && mask_at(&s, 15, 65) > 0.99);
}

#[test]
fn targeted_unlinked_mask_transforms_alone() {
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": "mask"})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "pixels untouched");
    assert!(mask_at(&s, 55, 65) < 0.01, "mask moved");
    assert!(mask_at(&s, 15, 15) > 0.99, "vacated area reads as the default");
    s.undo();
    assert!(mask_at(&s, 15, 15) < 0.01, "one undo step");
    // With a selection, only its part of the mask.
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 10})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 50], "target": "mask"})).unwrap();
    assert!(mask_at(&s, 12, 65) < 0.01 && mask_at(&s, 12, 15) > 0.99);
    assert!(mask_at(&s, 17, 15) < 0.01, "unselected mask stays");
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "pixels untouched");
}

#[test]
fn targeted_linked_mask_moves_with_its_layer() {
    // Photoshop: a linked mask can't move without its layer.
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], true);
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": "mask"})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(50, 60, 70, 70));
    assert!(mask_at(&s, 55, 65) < 0.01);
}

#[test]
fn targeted_alpha_channel_transforms_alone() {
    let mut s = session();
    s.execute("channel.new", json!({})).unwrap();
    s.edit("paint channel", |doc, _| {
        doc.channels[0].surface.fill_rect(Rect::new(10, 10, 20, 20), &[1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 40, 50], "target": {"channel": 0}})).unwrap();
    let st = s.active().unwrap();
    let ch = &st.doc.channels[0].surface;
    assert!(ch.sample_channel(55, 65, 0) > 0.99 && ch.sample_channel(15, 15, 0) < 0.01);
    assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 20), "layer untouched");
}

/// The Channels panel targets an alpha channel: Edit › Transform › Flip Horizontal and a
/// 50% `edit.transform` (no `target` param) work on the channel, not the layer (BUG-207-4).
#[test]
fn transforms_follow_the_targeted_alpha_channel() {
    let mut s = session();
    s.execute("channel.new", json!({})).unwrap();
    s.edit("paint channel", |doc, _| {
        let ch = &mut doc.channels[0].surface;
        ch.fill_rect(Rect::new(10, 10, 20, 20), &[1.0]);
        ch.fill_rect(Rect::new(10, 10, 12, 20), &[0.5]);
        Ok(())
    })
    .unwrap();
    s.execute("channel.target", json!({"channel": 0})).unwrap();
    let layer = |s: &Session| crate::active_layer_of(s).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 100, 100));
    let (pixels, past) = (layer(&s), s.active().unwrap().history.past_len());
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    // 8-bit channel: 0.5 is stored as 128/255.
    let ch = |s: &Session, x, y| (s.active().unwrap().doc.channels[0].surface.sample_channel(x, y, 0) * 10.0).round() / 10.0;
    assert_eq!((ch(&s, 19, 15), ch(&s, 10, 15)), (0.5, 1.0), "the channel flips within its frame");
    assert!(layer(&s) == pixels, "the layer is untouched");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one undo step");
    s.execute("edit.transform", json!({"matrix": [0.5, 0, 0, 0.5, 0, 0], "interpolation": "nearest"})).unwrap();
    assert_eq!((ch(&s, 7, 7), ch(&s, 15, 15)), (1.0, 0.0), "the channel scales to 50%");
    assert!(layer(&s) == pixels, "the layer is untouched");
    s.undo();
    s.undo();
    assert_eq!((ch(&s, 10, 15), ch(&s, 19, 15)), (0.5, 1.0));
}

#[test]
fn transform_targets_fail_gracefully() {
    let mut s = session();
    let m = json!([1, 0, 0, 1, 5, 5]);
    assert!(s.execute("edit.transform", json!({"matrix": m, "target": "mask"})).is_err(), "no mask");
    assert!(s.execute("edit.transform", json!({"matrix": m, "target": {"channel": 9}})).is_err());
    assert!(s.execute("edit.transform", json!({"matrix": m, "target": "quickMask"})).is_err());
    // Unknown target shapes fall back to the pixels.
    s.execute("edit.transform", json!({"matrix": m, "target": 7})).unwrap();
    s.execute("edit.transform", json!({"matrix": m, "target": {"channel": "x"}})).unwrap();
    // Selections that miss the mask entirely transform nothing, without panicking.
    let mut s = masked(&[Rect::new(10, 10, 20, 20)], false);
    s.execute("select.rect", json!({"x": 80, "y": 80, "width": 5, "height": 5})).unwrap();
    let _ = s.execute("edit.transform", json!({"matrix": [1e9, 0, 0, 1e9, 0, 0], "target": "mask"}));
    let _ = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 1e12, 0], "target": "mask"}));
}

#[test]
fn split_gray_selected_rejects_non_gray() {
    let s = Surface::new(PixelFormat::RGBA8);
    let mut sel = Surface::new(PixelFormat::GRAY8);
    sel.fill_rect(Rect::new(0, 0, 4, 4), &[1.0]);
    assert!(matches!(split_gray_selected(&s, &sel), Ok(None)));
}

/// #2630: type can't be partly transformed, so a selection doesn't narrow the frame: Free
/// Transform (and the frame-based presets) act on the whole type layer.
#[test]
fn a_selection_does_not_narrow_a_type_layers_frame() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 120})).unwrap();
    s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 36})).unwrap();
    let whole = active_bounds(&s);
    s.execute("select.rect", json!({"x": whole.x0, "y": whole.y0, "width": whole.width() / 2, "height": whole.height()})).unwrap();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(transform_bounds(&st.doc, l), whole, "the frame is the whole type layer");
    // Corners of the frame moved 10 px right: a pure translation of the whole text.
    let q = [[whole.x0 + 10, whole.y0], [whole.x1 + 10, whole.y0], [whole.x1 + 10, whole.y1], [whole.x0 + 10, whole.y1]];
    s.execute("edit.transform", json!({"quad": q})).unwrap();
    let moved = active_bounds(&s);
    assert!(moved.x0.abs_diff(whole.x0 + 10) <= 1 && moved.width().abs_diff(whole.width()) <= 1, "{whole:?} -> {moved:?}");
    // Rotate 180° turns it about its own centre, not the selection's.
    s.execute("edit.transform.rotate180", json!({})).unwrap();
    let turned = active_bounds(&s);
    assert!(turned.x0.abs_diff(moved.x0) <= 2 && turned.x1.abs_diff(moved.x1) <= 2, "{moved:?} -> {turned:?}");
    // A pixel layer still frames only the selected pixels.
    let mut s = session();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 10})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(transform_bounds(&st.doc, st.doc.layer(st.active_layer.unwrap()).unwrap()), Rect::new(10, 10, 15, 20));
}

/// #5297: the box Free Transform opens with a selection is the selection's own rectangle,
/// not the part of it the layer fills; type still moves whole, and a selection that holds none
/// of the layer leaves nothing to transform.
#[test]
fn the_frame_is_the_selections_own_bounds() {
    let mut s = session();
    let frame = |s: &Session| {
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        (transform_frame(&st.doc, l), transform_bounds(&st.doc, l))
    };
    let content = Rect::new(10, 10, 30, 20);
    assert_eq!(frame(&s), (content, content), "no selection: the content");
    s.execute("select.rect", json!({"x": 5, "y": 5, "width": 15, "height": 40})).unwrap();
    assert_eq!(frame(&s), (Rect::new(5, 5, 20, 45), Rect::new(10, 10, 20, 20)));
    s.execute("select.rect", json!({"x": 60, "y": 60, "width": 10, "height": 10})).unwrap();
    assert!(frame(&s).0.is_empty(), "nothing of the layer is selected");

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 120})).unwrap();
    s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello", "size": 36})).unwrap();
    let whole = active_bounds(&s);
    s.execute("select.rect", json!({"x": whole.x0, "y": whole.y0, "width": whole.width() / 2, "height": whole.height()})).unwrap();
    assert_eq!(frame(&s), (whole, whole), "type moves whole");
}

#[test]
fn group_transform_ignores_selections_inside_its_children() {
    group_transform_with_selection(Rect::new(10, 10, 15, 15));
}

#[test]
fn group_transform_ignores_selections_outside_its_children() {
    group_transform_with_selection(Rect::new(70, 70, 80, 80));
}

#[test]
fn group_transform_presets_rotate_about_the_whole_group() {
    for depth in [8, 16, 32] {
        for x in [10, 70] {
            let mut s = masked_depth(&[Rect::new(10, 10, 15, 20)], true, depth);
            let child = s.active().unwrap().active_layer.unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
            s.execute("select.rect", json!({"x": x, "y": 10, "width": 5, "height": 5})).unwrap();
            s.execute("edit.transform.rotate180", json!({})).unwrap();
            let layer = s.active().unwrap().doc.layer(child).unwrap();
            assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 20), "rotate about the group's centre, not the selection's");
            let mask = &layer.mask.as_ref().unwrap().surface;
            assert_eq!(mask.sample_channel(27, 15, 0), 0.0);
            assert_eq!(mask.sample_channel(12, 15, 0), 1.0);
        }
    }
}

fn group_transform_with_selection(selection: Rect) {
    for depth in [8, 16, 32] {
        for nested in [false, true] {
            let mut s = masked_depth(&[Rect::new(10, 10, 15, 20)], true, depth);
            let child = s.active().unwrap().active_layer.unwrap();
            s.execute("layer.groupLayers", json!({})).unwrap();
            if nested {
                s.execute("layer.groupLayers", json!({})).unwrap();
            }
            s.execute("select.rect", json!({"x": selection.x0, "y": selection.y0, "width": selection.width(), "height": selection.height()})).unwrap();
            let steps = s.active().unwrap().history.entries().len();
            let result = s.execute("edit.transform", json!({"quad": [[10, 10], [50, 10], [50, 30], [10, 30]], "interpolation": "nearest"})).unwrap();
            assert_eq!(result["rect"], json!([10.0, 10.0, 30.0, 20.0]), "frame must include the whole group (nested={nested})");
            let st = s.active().unwrap();
            let layer = st.doc.layer(child).unwrap();
            assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 50, 30));
            let mask = &layer.mask.as_ref().unwrap().surface;
            assert_eq!(mask.sample_channel(19, 20, 0), 0.0, "the linked mask scales with its complete layer");
            assert_eq!(mask.sample_channel(21, 20, 0), 1.0);
            assert_eq!(st.history.entries().len(), steps + 1);
            assert!(s.undo());
            let st = s.active().unwrap();
            let layer = st.doc.layer(child).unwrap();
            assert_eq!(layer.surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 20));
            assert_eq!(layer.mask.as_ref().unwrap().surface.sample_channel(19, 15, 0), 1.0);
            assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), selection);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.layer(child).unwrap().surface().unwrap().content_bounds(), Rect::new(10, 10, 50, 30));
        }
    }
}

#[test]
fn feathered_selection_identity_transform_preserves_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    s.execute("select.modify.feather", json!({"radius": 1})).unwrap();

    let mut before_px = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            before_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
        }
    }

    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0], "interpolation": "nearest"})).unwrap();

    let mut after_px = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            after_px.push(s.execute("document.pixel", json!({"x": x, "y": y})).unwrap());
        }
    }

    assert_eq!(before_px, after_px, "feathered selection identity transform must preserve all pixels");
    let p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
    assert_eq!(p4_8, json!([1.0, 0.0, 0.0, 1.0]), "(4,8) must remain opaque red");

    // Undo restores original document.
    s.execute("edit.undo", json!({})).unwrap();
    let undo_p4_8 = s.execute("document.pixel", json!({"x": 4, "y": 8})).unwrap();
    assert_eq!(undo_p4_8, json!([1.0, 0.0, 0.0, 1.0]));

    // Locked layer still refuses transform even if identity.
    s.execute("layer.setProps", json!({"locks": {"position": true}})).unwrap();
    assert!(s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})).is_err(), "locked layer refuses identity transform");
}

/// A 1 x 8 px column on a 256 x 256 canvas (#3773).
fn column_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 256, "height": 256})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(100, 100, 101, 108), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

fn opaque_green(s: &Session, x: i32, y: i32) -> bool {
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (surf.sample_channel(x, y, 1) - 1.0).abs() < 1e-3 && (surf.sample_channel(x, y, 3) - 1.0).abs() < 1e-3
}

/// The "pixel stretch" (#3773): scales far past 1000% are taken, and the stretched column stays
/// solid along its whole length.
#[test]
fn a_one_pixel_column_stretches_5000_percent() {
    let mut s = column_session();
    s.execute("edit.transform", json!({"quad": [[100, 100], [150, 100], [150, 108], [100, 108]]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(100, 100, 150, 108));
    for x in [100, 101, 125, 149] {
        for y in 100..108 {
            assert!(opaque_green(&s, x, y), "({x}, {y})");
        }
    }
}

/// The technique as done in Photoshop: select a 1 px column of an image and stretch it across.
#[test]
fn a_selected_column_stretches_solid_over_the_image() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 256, "height": 256})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 256, 256), &[1.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(10, 0, 11, 256), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 0, "width": 1, "height": 256})).unwrap();
    s.execute("edit.transform", json!({"quad": [[10, 0], [250, 0], [250, 256], [10, 256]]})).unwrap();
    for x in [10, 11, 100, 200, 249] {
        assert!(opaque_green(&s, x, 128), "x = {x}");
    }
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert!(surf.sample_channel(250, 128, 0) > 0.999 && surf.sample_channel(9, 128, 0) > 0.999, "the image either side is untouched");
}

/// Free Transform keeps pixels moved off the canvas, as Photoshop does: an 8 x 8 layer moved
/// three canvas widths right and back comes back whole.
#[test]
fn a_layer_moved_far_off_the_canvas_and_back_keeps_its_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("paint", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..8 {
            for x in 0..8 {
                surf.fill_rect(Rect::new(20 + x, 20 + y, 21 + x, 21 + y), &[x as f32 / 8.0, y as f32 / 8.0, 0.5, 1.0]);
            }
        }
        Ok(())
    })
    .unwrap();
    let pixels = |s: &Session| {
        let st = s.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        (0..8 * 8 * 4).map(|i| surf.sample_channel(20 + i / 4 % 8, 20 + i / 32, (i % 4) as usize)).collect::<Vec<_>>()
    };
    let before = pixels(&s);
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 192, 0]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(212, 20, 220, 28));
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, -192, 0]})).unwrap();
    assert_eq!(active_bounds(&s), Rect::new(20, 20, 28, 28));
    assert_eq!(pixels(&s), before);
}

#[test]
fn relative_params_scale_and_rotate_about_the_frame_centre() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 30, "width": 40, "height": 20})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    // 50% about the centre (40, 40), then 10 px right: the content ends at x 40..60, y 35..45.
    let r = s.execute("edit.transform", json!({"scaleX": 50, "scaleY": 50, "dx": 10})).unwrap();
    let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
    assert_eq!(rect, [20.0, 30.0, 60.0, 50.0], "the reported frame is the source frame");
    let bounds = |s: &Session| {
        let d = s.active().unwrap();
        let b = transform_bounds(&d.doc, d.doc.layer(d.active_layer.unwrap()).unwrap());
        (b.x0, b.y0, b.x1, b.y1)
    };
    assert_eq!(bounds(&s), (40, 35, 60, 45));
    // A quarter turn about the centre (50, 40) swaps the frame's sides.
    s.execute("edit.transform", json!({"rotate": 90})).unwrap();
    assert_eq!(bounds(&s), (45, 30, 55, 50));
    for bad in [json!({"scaleX": 0}), json!({"rotate": "x"}), json!({"dx": f64::NAN})] {
        assert!(s.execute("edit.transform", bad.clone()).is_err(), "{bad}");
    }
}

/// #5846: a shape's Density and Feather render past its path (over the whole canvas below
/// 100 %), but its transform frame follows the path, as in Photoshop: the same box as the hard
/// shape for Free Transform, the Move tool's controls and snapping, never the canvas.
#[test]
fn a_soft_shapes_frame_follows_its_path() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = s.execute("shape.create", json!({"kind": "rect", "rect": [20, 30, 60, 40], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let id = photocraft_doc::LayerId(id);
    let boxes = |s: &Session| {
        let st = s.active().unwrap();
        let l = st.doc.layer(id).unwrap();
        (transform_bounds(&st.doc, l), transform_frame(&st.doc, l), crate::snap::layer_rect(&st.doc, id))
    };
    let hard = boxes(&s);
    assert_eq!(hard.0, Rect::new(20, 30, 80, 70), "the hard shape's frame");
    s.execute("layer.vectorMask.edit", json!({"density": 60, "feather": 20})).unwrap();
    {
        let st = s.active().unwrap();
        let LayerContent::Shape(sh) = &st.doc.layer(id).unwrap().content else { panic!("a shape") };
        let painted = photocraft_compose::bounds::content_bounds(sh.cache.as_ref().unwrap());
        assert_eq!(painted, st.doc.bounds(), "the soft fill renders over the whole canvas");
    }
    let soft = boxes(&s);
    assert_eq!(soft, hard, "the frame ignores Density and Feather");
    assert_ne!(soft.1, s.active().unwrap().doc.bounds());
    // Feather alone (density 100 %) leaves the frame on the path too, not grown by the blur.
    s.execute("layer.vectorMask.edit", json!({"density": 100, "feather": 20})).unwrap();
    assert_eq!(boxes(&s), hard);
    // Free Transform scales about the path's centre, not the canvas's.
    s.execute("edit.transform", json!({"layer": id.0, "scaleX": 200})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(id).unwrap().content else { panic!("a shape") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    assert_eq!((x0.round(), y0.round(), x1.round(), y1.round()), (-10.0, 30.0, 110.0, 70.0));
    assert_eq!((sh.density, sh.feather), (1.0, 20.0), "the transform keeps the soft settings");
}

/// A stroked soft shape's frame is the hard shape's rendered frame for every stroke alignment:
/// a centred stroke reaches half its width past the path, not its full width; both stop at
/// the canvas edge.
#[test]
fn a_stroked_soft_shapes_frame_matches_the_hard_shapes() {
    for (align, reach, x) in [("center", 5, 50), ("inside", 0, 50), ("outside", 10, 50), ("center", 5, -20)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        let stroke = json!({"width": 10, "color": "#ff0000", "align": align, "join": "round"});
        let id = s.execute("shape.create", json!({"kind": "rect", "rect": [x, 50, 60, 40], "fill": "#000000", "stroke": stroke})).unwrap()["layer"]
            .as_u64()
            .unwrap();
        let id = photocraft_doc::LayerId(id);
        let boxes = |s: &Session| {
            let st = s.active().unwrap();
            let l = st.doc.layer(id).unwrap();
            (transform_bounds(&st.doc, l), transform_frame(&st.doc, l), crate::snap::layer_rect(&st.doc, id))
        };
        let hard = boxes(&s);
        // Hand-computed: the path is x..x + 60 x 50..90, the stroke reaches `reach` px past it.
        assert_eq!(hard.0, Rect::new((x - reach).max(0), 50 - reach, x + 60 + reach, 90 + reach), "{align} at {x}: the hard shape's frame");
        s.execute("layer.vectorMask.edit", json!({"density": 60, "feather": 8})).unwrap();
        assert_eq!(boxes(&s), hard, "{align} at {x}: Density and Feather leave the frame");
        s.execute("layer.vectorMask.edit", json!({"density": 100, "feather": 8})).unwrap();
        assert_eq!(boxes(&s), hard, "{align} at {x}: Feather alone leaves the frame");
    }
}
