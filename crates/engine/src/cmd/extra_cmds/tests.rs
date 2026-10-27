use super::*;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn equalize_average_and_clouds_edit_the_targeted_channel() {
    // #935: these four ran through `map_pixels`, which always rewrote the active layer, so
    // with an alpha channel or the Quick Mask targeted they changed the layer and left the
    // channel alone.
    let cmds = ["image.adjustments.equalize", "filter.blur.average", "filter.render.clouds", "filter.render.differenceClouds"];
    for depth in [8, 16, 32] {
        for target in ["quickMask", "alpha", "mask"] {
            for cmd in cmds {
                let mut s = session(depth);
                s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
                s.tools.background = [1.0, 1.0, 1.0, 1.0];
                s.execute("edit.fill", json!({"contents": "color", "color": "#808080"})).unwrap();
                let p = match target {
                    "quickMask" => {
                        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                        json!({"seed": 1})
                    }
                    "alpha" => {
                        s.execute("channel.new", json!({"fill": "white"})).unwrap();
                        s.execute("channel.target", json!({"channel": 0})).unwrap();
                        json!({"seed": 1})
                    }
                    _ => {
                        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                        json!({"seed": 1, "target": "mask"})
                    }
                };
                // Two levels in the target, so Equalize and Average have something to change.
                let edit_target = json!({ "target": if target == "alpha" { json!({"channel": 0}) } else { json!(target) } });
                s.edit("levels", |doc, active| {
                    let (surf, _) = crate::channel_cmds::target_surface(doc, *active, &edit_target).unwrap();
                    surf.fill_rect(Rect::new(0, 0, 20, 30), &[0.4]);
                    surf.fill_rect(Rect::new(20, 0, 40, 30), &[0.5]);
                    Ok(())
                })
                .unwrap();
                let full = Rect::new(0, 0, 40, 30);
                let read = |s: &Session| {
                    let d = doc(s);
                    let layer = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
                    let t = match target {
                        "quickMask" => d.quick_mask.as_ref().unwrap().surface.read_region(full),
                        "alpha" => d.channels[0].surface.read_region(full),
                        _ => layer.mask.as_ref().unwrap().surface.read_region(full),
                    };
                    (layer.surface().unwrap().read_region(full), t)
                };
                let ((layer0, target0), past) = (read(&s), s.active().unwrap().history.past_len());
                s.execute(cmd, p).unwrap();
                let (layer1, target1) = read(&s);
                let case = format!("{cmd} on {target} at {depth}");
                assert_eq!(layer1, layer0, "{case}: the layer is untouched");
                assert_ne!(target1, target0, "{case}: the target changed");
                assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{case}");
                s.undo();
                assert_eq!(read(&s).1, target0, "{case}: undo restores the target");
            }
        }
    }
}

fn active(s: &Session) -> &Layer {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap()
}

fn pixel(s: &Session, x: i32, y: i32) -> Vec<f32> {
    active(s).surface().unwrap().pixel(x, y)
}

fn paint_square(s: &mut Session, r: Rect, c: [f32; 4]) {
    s.edit("paint", |doc, a| {
        doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(r, &c);
        Ok(())
    })
    .unwrap();
}

#[test]
fn stroke_locations_at_several_depths() {
    for depth in [8, 16, 32] {
        for (loc, inside, outside) in [("inside", 1.0, 0.0), ("outside", 0.0, 1.0), ("center", 1.0, 1.0)] {
            let mut s = session(depth);
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
            s.execute("edit.stroke", json!({"width": 2, "color": "#ff0000", "location": loc})).unwrap();
            assert!((pixel(&s, 10, 15)[3] - inside).abs() < 0.02, "{depth} {loc} inside {:?}", pixel(&s, 10, 15));
            assert!((pixel(&s, 9, 15)[3] - outside).abs() < 0.02, "{depth} {loc} outside {:?}", pixel(&s, 9, 15));
            assert_eq!(pixel(&s, 15, 15)[3], 0.0, "centre untouched");
        }
    }
    // Without a selection the layer's opaque edge is stroked.
    let mut s = session(8);
    paint_square(&mut s, Rect::new(10, 10, 20, 20), [0.0, 0.0, 1.0, 1.0]);
    s.execute("edit.stroke", json!({"width": 3, "color": "#00ff00", "location": "outside"})).unwrap();
    assert_eq!(pixel(&s, 8, 15), vec![0.0, 1.0, 0.0, 1.0]);
    assert_eq!(pixel(&s, 15, 15), vec![0.0, 0.0, 1.0, 1.0]);
}

/// Photoshop's Edit › Stroke on a selection (checked by hand): Center puts ceil(w/2) px inside
/// and floor(w/2) outside (1 px is all inside), fractional widths round up, and every stroked
/// pixel is fully opaque, on every edge, horizontal ones included. Inside and Outside are
/// w px on their side.
#[test]
fn stroke_widths_are_whole_opaque_pixels_like_photoshop() {
    // Selection x 10..30, y 10..30; the edge pixels inside are 10 and 29 on each axis.
    let band = |loc: &str, w: i32| -> (i32, i32) {
        match loc {
            "inside" => (w, 0),
            "outside" => (0, w),
            _ => ((w + 1) / 2, w / 2),
        }
    };
    let mut wrong = Vec::new();
    for depth in [8, 16, 32] {
        for loc in ["center", "inside", "outside"] {
            for (param, w) in [(json!(1), 1), (json!(2), 2), (json!(3), 3), (json!(4), 4), (json!(5), 5), (json!(0.5), 1), (json!(2.5), 3)] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 50, "height": 50, "depth": depth})).unwrap();
                s.execute("layer.new.layer", json!({})).unwrap();
                s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                s.execute("edit.stroke", json!({"width": param, "color": "#ff0000", "location": loc})).unwrap();
                let (inside, outside) = band(loc, w);
                // (edge pixel inside, step inward) for left, right, top and bottom; the other
                // coordinate is the middle of the edge.
                for (name, first_in, inward, horizontal) in [("left", 10, 1, false), ("right", 29, -1, false), ("top", 10, 1, true), ("bottom", 29, -1, true)] {
                    for k in -7..7 {
                        // k ≥ 0: k px inside from the edge; k < 0: -k px outside.
                        let c = first_in + k * inward;
                        let (x, y) = if horizontal { (20, c) } else { (c, 20) };
                        let want = if (k >= 0 && k < inside) || (k < 0 && -k <= outside) { 1.0 } else { 0.0 };
                        let a = pixel(&s, x, y)[3];
                        if (a - want).abs() > 0.004 {
                            wrong.push(format!("{depth}-bit {loc} width {param} {name} edge, {k:+} px: alpha {a}, want {want}"));
                        }
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} pixels differ from Photoshop:\n{}", wrong.len(), wrong.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// Photoshop's 6 px Outside stroke of a 14 px circle selection made without anti-aliasing,
/// read pixel by pixel from Photoshop (`#` stroked, `.` not; the hole is the selection). A
/// hard selection gives a hard stroke: a pixel is stroked when its centre is less than
/// width + 1 from a selected pixel's centre (no partial pixels on the curve).
const PS_CIRCLE_OUTSIDE_6: [&str; 31] = [
    "..................................",
    "..................................",
    "...........############...........",
    "..........##############..........",
    ".........################.........",
    "........##################........",
    ".......####################.......",
    "......######################......",
    ".....#########......#########.....",
    "....#########........#########....",
    "....########..........########....",
    "....#######............#######....",
    "....######..............######....",
    "....######..............######....",
    "....######..............######....",
    "....######..............######....",
    "....######..............######....",
    "....######..............######....",
    "....#######............#######....",
    "....########..........########....",
    "....#########........#########....",
    ".....#########......#########.....",
    "......######################......",
    ".......####################.......",
    "........##################........",
    ".........################.........",
    "..........##############..........",
    "...........############...........",
    "..................................",
    "..................................",
    "..................................",
];

/// Photoshop's 3 px Inside stroke of the same 14 px hard circle (the selection is the ring
/// plus its hole). Inside, a pixel is stroked when its centre is less than width + ½ from an
/// unselected pixel's: that offset is the one fitting this, the 5 px and the 6 px samples
/// (any value from +0.41 to +0.60), so 6 px fills this circle (its deepest pixel is 6.4 px in).
const PS_CIRCLE_INSIDE_3: [&str; 17] = [
    "...................",
    "......######.......",
    ".....########......",
    "....##########.....",
    "...####....####....",
    "..####......####...",
    "..###........###...",
    "..###........###...",
    "..###........###...",
    "..###........###...",
    "..####......####...",
    "...####....####....",
    "....##########.....",
    ".....########......",
    "......######.......",
    "...................",
    "...................",
];

/// The selection of a Photoshop sample grid: an Outside stroke's hole, or an Inside stroke's
/// ring and hole.
fn grid_selection(rows: &[&str], outside: bool) -> Vec<f32> {
    let (w, h) = (rows[0].len(), rows.len());
    let mut mask = vec![0.0f32; w * h];
    for (y, row) in rows.iter().enumerate() {
        let b = row.as_bytes();
        if let (Some(first), Some(last)) = (b.iter().position(|c| *c == b'#'), b.iter().rposition(|c| *c == b'#')) {
            for x in first..=last {
                if !outside || b[x] == b'.' {
                    mask[y * w + x] = 1.0;
                }
            }
        }
    }
    mask
}

/// Strokes `mask` as the selection of a `w` × `h` layer and returns its alpha per pixel.
fn stroke_alpha(mask: &[f32], w: usize, h: usize, depth: u32, params: Value) -> Vec<f32> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("select", |doc, _| {
        doc.selection = Some(sel::mask_to_surface(mask, doc.bounds()));
        Ok(())
    })
    .unwrap();
    s.execute("edit.stroke", params).unwrap();
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pixel(&s, x as i32, y as i32)[3]).collect()
}

#[test]
fn a_hard_circle_strokes_inside_like_photoshop() {
    let rows = PS_CIRCLE_INSIDE_3;
    let (w, h) = (rows[0].len(), rows.len());
    let mask = grid_selection(&rows, false);
    for depth in [8, 16, 32] {
        let got = stroke_alpha(&mask, w, h, depth, json!({"width": 3, "color": "#000000", "location": "inside"}));
        let wrong: Vec<String> = rows
            .iter()
            .enumerate()
            .flat_map(|(y, row)| row.bytes().enumerate().map(move |(x, c)| (x, y, c)))
            .filter_map(|(x, y, c)| {
                let want = if c == b'#' { 1.0 } else { 0.0 };
                let a = got[y * w + x];
                ((a - want).abs() > 0.004).then(|| format!("({x}, {y}): {a}, Photoshop {want}"))
            })
            .collect();
        assert!(wrong.is_empty(), "{depth}-bit: {} pixels differ from Photoshop: {wrong:?}", wrong.len());
        // Checked in Photoshop: a 5 px Inside stroke leaves a plus-shaped hole of 12 pixels in
        // the middle (rows 2, 4, 4, 2 wide), and 6 px fills the whole circle.
        let hole = [(8, 6), (9, 6), (7, 7), (8, 7), (9, 7), (10, 7), (7, 8), (8, 8), (9, 8), (10, 8), (8, 9), (9, 9)];
        let five = stroke_alpha(&mask, w, h, depth, json!({"width": 5, "color": "#000000", "location": "inside"}));
        for (i, m) in mask.iter().enumerate() {
            let stroked = *m > 0.0 && !hole.contains(&(i % w, i / w));
            assert_eq!(five[i] > 0.996, stroked, "{depth}-bit 5 px Inside at ({}, {})", i % w, i / w);
        }
        let full = stroke_alpha(&mask, w, h, depth, json!({"width": 6, "color": "#000000", "location": "inside"}));
        for (i, m) in mask.iter().enumerate() {
            assert_eq!(full[i] > 0.996, *m > 0.0, "{depth}-bit 6 px Inside at ({}, {})", i % w, i / w);
        }
    }
}

#[test]
fn a_hard_circle_strokes_like_photoshop() {
    let rows = PS_CIRCLE_OUTSIDE_6;
    let (w, h) = (rows[0].len(), rows.len());
    // The selection: the `.` cells enclosed by the ring on each row.
    let mask = grid_selection(&rows, true);
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("select", |doc, _| {
            doc.selection = Some(sel::mask_to_surface(&mask, doc.bounds()));
            Ok(())
        })
        .unwrap();
        s.execute("edit.stroke", json!({"width": 6, "color": "#000000", "location": "outside"})).unwrap();
        let mut wrong = Vec::new();
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                let want = if c == b'#' { 1.0 } else { 0.0 };
                let a = pixel(&s, x as i32, y as i32)[3];
                if (a - want).abs() > 0.004 {
                    wrong.push(format!("({x}, {y}): {a}, Photoshop {want}"));
                }
            }
        }
        assert!(wrong.is_empty(), "{depth}-bit: {} pixels differ from Photoshop: {wrong:?}", wrong.len());
    }
}

/// Select All, then Stroke › Inside draws a border around the canvas (a common Photoshop
/// technique): the canvas edge is a selection edge like any other, on all four sides.
#[test]
fn stroking_a_selection_at_the_canvas_edge_draws_that_edge() {
    for depth in [8, 16, 32] {
        for (loc, w, inside) in [("inside", 3, 3), ("center", 3, 2), ("center", 1, 1)] {
            let mut s = session(depth);
            s.execute("select.all", json!({})).unwrap();
            s.execute("edit.stroke", json!({"width": w, "color": "#ff0000", "location": loc})).unwrap();
            // 40 × 30 canvas: k px in from each side, in the middle of the side.
            for name in ["left", "right", "top", "bottom"] {
                for k in 0..6 {
                    let (x, y) = match name {
                        "left" => (k, 15),
                        "right" => (39 - k, 15),
                        "top" => (20, k),
                        _ => (20, 29 - k),
                    };
                    let want = if k < inside { 1.0 } else { 0.0 };
                    let a = pixel(&s, x, y)[3];
                    assert!((a - want).abs() < 0.004, "{depth}-bit {loc} {w} px, {name} side, {k} px in: alpha {a}, want {want}");
                }
            }
        }
    }
}

#[test]
fn stroke_uses_fill_blending_and_preserves_transparency_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        paint_square(&mut s, Rect::new(10, 10, 20, 20), [0.5, 0.5, 0.5, 1.0]);
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();

        let past = s.active().unwrap().history.past_len();
        s.execute(
            "edit.stroke",
            json!({"width": 2, "color": "#ffffff", "location": "inside", "mode": "multiply", "opacity": 100, "preserveTransparency": true}),
        )
        .unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "Stroke is one history step");
        let edge = pixel(&s, 10, 15);
        assert!((edge[0] - 0.5).abs() < 0.015, "{depth}-bit Multiply must preserve gray: {edge:?}");
        assert_eq!(edge[3], 1.0);

        s.execute("edit.stroke", json!({"width": 2, "color": "#ffffff", "location": "outside", "preserveTransparency": true})).unwrap();
        assert_eq!(pixel(&s, 9, 15)[3], 0.0, "{depth}-bit transparency lock prevents new outer pixels");

        s.execute("edit.stroke", json!({"width": 2, "color": "#ffffff", "location": "outside", "opacity": 50})).unwrap();
        let out = pixel(&s, 9, 15);
        assert!((out[3] - 0.5).abs() < 0.015, "{depth}-bit 50% stroke opacity: {out:?}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&s, 9, 15)[3], 0.0);
    }
}

#[test]
fn stroke_rejects_invalid_blending_options_without_modifying_the_document() {
    let mut s = session(8);
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let past = s.active().unwrap().history.past_len();
    for params in [
        json!({"mode": "unknown-mode"}),
        json!({"mode": 42}),
        json!({"mode": "passThrough"}),
        json!({"opacity": "lots"}),
        json!({"preserveTransparency": "true"}),
    ] {
        assert!(s.execute("edit.stroke", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "{params} should not create a history step");
    }
}

#[test]
fn transform_presets_are_exact() {
    let mut s = session(8);
    paint_square(&mut s, Rect::new(10, 10, 20, 14), [1.0, 0.0, 0.0, 1.0]);
    paint_square(&mut s, Rect::new(10, 10, 11, 11), [0.0, 1.0, 0.0, 1.0]);
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(10, 10, 20, 14));
    assert_eq!(pixel(&s, 19, 10), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("edit.transform.flipVertical", json!({})).unwrap();
    assert_eq!(pixel(&s, 19, 13), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("edit.transform.rotate180", json!({})).unwrap();
    assert_eq!(pixel(&s, 10, 10), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("edit.transform.rotate90Cw", json!({})).unwrap();
    let b = active(&s).surface().unwrap().content_bounds();
    assert_eq!((b.width(), b.height()), (4, 10));
    // Top-left corner goes to the top-right after a clockwise turn.
    assert_eq!(pixel(&s, b.x1 - 1, b.y0), vec![0.0, 1.0, 0.0, 1.0]);
    s.execute("edit.transform.rotate90Ccw", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(10, 10, 20, 14));
    assert_eq!(pixel(&s, 10, 10), vec![0.0, 1.0, 0.0, 1.0]);
}

/// #6928: with a selection, the presets flip and turn about the selection's own bounds, as
/// in Photoshop: Select All mirrors across the canvas, a marquee across its own centre.
/// Without a selection they keep turning about the content (above).
#[test]
fn transform_presets_turn_about_the_selection() {
    let mut s = session(8);
    paint_square(&mut s, Rect::new(10, 10, 20, 14), [1.0, 0.0, 0.0, 1.0]);
    paint_square(&mut s, Rect::new(10, 10, 11, 11), [0.0, 1.0, 0.0, 1.0]);
    let green = vec![0.0, 1.0, 0.0, 1.0];
    // Select All on the 40 × 30 canvas: across the canvas's centre lines.
    s.execute("select.all", json!({})).unwrap();
    let past = s.active().unwrap().history.past_len();
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(20, 10, 30, 14));
    assert_eq!(pixel(&s, 29, 10), green);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one undo step");
    s.execute("edit.transform.flipVertical", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(20, 16, 30, 20));
    assert_eq!(pixel(&s, 29, 19), green);
    s.execute("edit.transform.rotate180", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(10, 10, 20, 14));
    assert_eq!(pixel(&s, 10, 10), green);
    // A marquee (0, 0)–(24, 30) holding the whole square: across x = 12.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 24, "height": 30})).unwrap();
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(4, 10, 14, 14));
    assert_eq!(pixel(&s, 13, 10), green);
    // Undo puts it back where it was.
    s.undo();
    assert_eq!(active(&s).surface().unwrap().content_bounds(), Rect::new(10, 10, 20, 14));
    // A targeted alpha channel turns about the selection's bounds too.
    s.execute("channel.new", json!({})).unwrap();
    s.edit("paint channel", |doc, _| {
        doc.channels[0].surface.fill_rect(Rect::new(10, 10, 20, 20), &[1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("channel.target", json!({"channel": 0})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.channels[0].surface.content_bounds(), Rect::new(20, 10, 30, 20));
}

#[test]
fn reselect_restores_the_last_selection() {
    let mut s = session(8);
    assert!(!s.is_enabled("select.reselect"));
    s.execute("select.rect", json!({"x": 2, "y": 3, "width": 5, "height": 6})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert!(doc(&s).selection.is_none());
    s.execute("select.reselect", json!({})).unwrap();
    assert_eq!(doc(&s).selection.as_ref().unwrap().content_bounds(), Rect::new(2, 3, 7, 9));
    assert!(!s.is_enabled("select.reselect"), "nothing to reselect while a selection exists");
}

#[test]
fn equalize_spreads_values() {
    for depth in [8, 16] {
        let mut s = session(depth);
        paint_square(&mut s, Rect::new(0, 0, 40, 15), [0.4, 0.4, 0.4, 1.0]);
        paint_square(&mut s, Rect::new(0, 15, 40, 30), [0.5, 0.5, 0.5, 1.0]);
        s.execute("image.adjustments.equalize", json!({})).unwrap();
        let (dark, light) = (pixel(&s, 1, 1)[0], pixel(&s, 1, 20)[0]);
        assert!(dark < 0.05 && light > 0.95, "{depth}: {dark} {light}");
    }
}

/// A selection masks Clouds but doesn't change their size: inside it they are the canvas's
/// clouds, as when Clouds sized itself by the document (before it became a filter).
#[test]
fn clouds_in_a_selection_are_the_canvas_clouds() {
    // Big enough that the canvas (cloud size 100 px) and the selection (16 px) size differently.
    let big = |depth: u32| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    };
    for depth in [8, 16, 32] {
        let mut whole = big(depth);
        whole.execute("filter.render.clouds", json!({"seed": 4})).unwrap();
        let mut s = big(depth);
        s.execute("select.rect", json!({"x": 50, "y": 40, "width": 60, "height": 50})).unwrap();
        s.execute("filter.render.clouds", json!({"seed": 4})).unwrap();
        for (x, y) in [(50, 40), (75, 60), (100, 85), (109, 89)] {
            let (a, b) = (pixel(&s, x, y), pixel(&whole, x, y));
            assert!((0..4).all(|c| (a[c] - b[c]).abs() < 1e-3), "depth {depth} at {x},{y}: {a:?} vs {b:?}");
        }
        assert_eq!(pixel(&s, 300, 200)[3], 0.0, "outside the selection the layer stays empty");
    }
}

#[test]
fn average_and_clouds() {
    let mut s = session(8);
    paint_square(&mut s, Rect::new(0, 0, 20, 30), [1.0, 0.0, 0.0, 1.0]);
    paint_square(&mut s, Rect::new(20, 0, 40, 30), [0.0, 0.0, 1.0, 1.0]);
    s.execute("filter.blur.average", json!({})).unwrap();
    let p = pixel(&s, 5, 5);
    assert!((p[0] - 0.5).abs() < 0.01 && (p[2] - 0.5).abs() < 0.01, "{p:?}");
    s.execute("filter.render.clouds", json!({"seed": 3})).unwrap();
    let vals: Vec<f32> = (0..40).map(|x| pixel(&s, x, 12)[0]).collect();
    let (lo, hi) = vals.iter().fold((1.0f32, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(hi - lo > 0.05, "clouds vary: {lo}..{hi}");
    assert!(vals.windows(2).all(|w| (w[0] - w[1]).abs() < 0.2), "clouds are smooth");
    // Difference Clouds over itself with the same seed cancels to black.
    s.execute("filter.render.differenceClouds", json!({"seed": 3})).unwrap();
    assert!(pixel(&s, 7, 7)[..3].iter().all(|v| *v < 0.01), "{:?}", pixel(&s, 7, 7));
}

#[test]
fn paste_into_masks_to_the_selection() {
    let mut s = session(8);
    paint_square(&mut s, Rect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 1.0]);
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    s.execute("select.rect", json!({"x": 20, "y": 10, "width": 8, "height": 8})).unwrap();
    s.execute("edit.pasteSpecial.pasteInto", json!({})).unwrap();
    let l = active(&s);
    let m = l.mask.as_ref().unwrap();
    assert!(!m.linked);
    assert_eq!(m.surface.content_bounds(), Rect::new(20, 10, 28, 18));
    assert_eq!(m.value(0, 0), 0.0);
    assert!(doc(&s).selection.is_none());
    assert_eq!(s.active().unwrap().history.past_len(), steps + 2, "select + one Paste Into step");
    // Paste Outside reveals everything but the selection.
    s.execute("select.rect", json!({"x": 20, "y": 10, "width": 8, "height": 8})).unwrap();
    s.execute("edit.pasteSpecial.pasteOutside", json!({})).unwrap();
    let m = active(&s).mask.as_ref().unwrap();
    assert_eq!((m.value(0, 0), m.value(22, 12)), (1.0, 0.0));
}

#[test]
fn option_click_eye_shows_one_layer_then_restores() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let ids: Vec<LayerId> = doc(&s).layers.iter().map(|l| l.id).collect();
    // Background, Layer 1 (hidden beforehand), Layer 2.
    s.execute("layer.hideLayers", json!({"layer": ids[1].0})).unwrap();
    let vis = |s: &Session| doc(s).layers.iter().map(|l| l.visible).collect::<Vec<_>>();
    assert_eq!(s.execute("layer.showOnly", json!({"layer": ids[1].0})).unwrap()["shownAlone"], true);
    assert_eq!(vis(&s), [false, true, false]);
    // Another eye moves the solo; the original snapshot survives.
    s.execute("layer.showOnly", json!({"layer": ids[2].0})).unwrap();
    assert_eq!(vis(&s), [false, false, true]);
    assert_eq!(s.execute("layer.showOnly", json!({"layer": ids[2].0})).unwrap()["shownAlone"], false);
    assert_eq!(vis(&s), [true, false, true], "restored, with Layer 1 still hidden");
    // One history step each way.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(vis(&s), [false, false, true]);
}

#[test]
fn show_only_keeps_groups_and_their_contents() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let inner = active(&s).id;
    let group = s.execute("layer.groupLayers", json!({"layer": inner.0})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    let shown = |s: &Session| doc(s).walk().iter().map(|(_, _, l)| l.visible).collect::<Vec<_>>();
    // Background, Layer 1, Group 1, Layer 2 (inside the group).
    s.execute("layer.showOnly", json!({"layer": inner.0})).unwrap();
    assert_eq!(shown(&s), [false, false, true, true], "the enclosing group stays visible");
    // An undo ends the solo, so the next ⌥-click solos again from the current state.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("layer.showOnly", json!({"layer": inner.0})).unwrap()["shownAlone"], true);
    s.execute("edit.undo", json!({})).unwrap();
    // Showing a group alone leaves its contents as they are.
    s.execute("layer.setProps", json!({"layer": inner.0, "visible": false})).unwrap();
    s.execute("layer.showOnly", json!({"layer": group.0})).unwrap();
    assert_eq!(shown(&s), [false, false, true, false]);
    s.execute("layer.showOnly", json!({"layer": group.0})).unwrap();
    assert_eq!(shown(&s), [true, true, true, false]);
}

#[test]
fn show_only_without_a_snapshot_shows_every_layer() {
    let mut s = session(16);
    let bg = doc(&s).layers[0].id;
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    // Layer 1 is already the only visible layer: ⌥-clicking it shows everything.
    assert_eq!(s.execute("layer.showOnly", json!({})).unwrap()["shownAlone"], false);
    assert!(doc(&s).layers.iter().all(|l| l.visible));
    assert!(s.execute("layer.showOnly", json!({"layer": u64::MAX})).is_err());
    assert!(Session::new().execute("layer.showOnly", json!({})).is_err());
}

#[test]
fn a_mask_turns_the_background_into_a_layer() {
    for (cmd, needs_selection) in [
        ("layer.layerMask.revealAll", false),
        ("layer.layerMask.hideAll", false),
        ("layer.layerMask.revealSelection", true),
        ("layer.layerMask.hideSelection", true),
    ] {
        let mut s = session(8);
        let bg = doc(&s).layers[0].id;
        if needs_selection {
            s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        }
        s.execute(cmd, json!({"layer": bg.0})).unwrap();
        let l = &doc(&s).layers[0];
        assert!(l.mask.is_some() && l.name == "Layer 0" && !l.locks.transparency && !l.locks.position, "{cmd}");
        assert!(!s.is_enabled("layer.new.layerFromBackground"), "{cmd}");
        // One step: undo brings the locked Background back, without a mask.
        assert!(s.undo());
        let l = &doc(&s).layers[0];
        assert!(l.mask.is_none() && l.name == "Background" && l.locks.transparency, "{cmd}");
    }
    // Other layers keep their name and locks.
    let mut s = session(8);
    let top = doc(&s).layers[1].id;
    s.execute("layer.setProps", json!({"layer": top.0, "locks": {"position": true}})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({"layer": top.0})).unwrap();
    let l = doc(&s).layer(top).unwrap();
    assert!(l.mask.is_some() && l.name != "Layer 0" && l.locks.position);
    assert_eq!(doc(&s).layers[0].name, "Background");
}

#[test]
fn hide_layers_toggles_once_every_target_is_hidden() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let b = active(&s).id;
    let visible = |s: &Session| doc(s).layer(b).unwrap().visible;
    // Select › All Layers leaves out the Background, as in Photoshop.
    let all = |s: &Session| crate::layer_multi_cmds::selected(s).iter().map(|id| doc(s).layer(*id).unwrap().visible).collect::<Vec<_>>();
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(!visible(&s));
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(visible(&s), "⌘, again shows it");
    // Several selected: any one still shown means hide them all; then they all come back.
    s.execute("layer.hideLayers", json!({"layer": b.0})).unwrap();
    s.execute("select.allLayers", json!({})).unwrap();
    assert!(all(&s).len() > 1);
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(all(&s).iter().all(|v| !v));
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(all(&s).iter().all(|v| *v));
}

#[test]
fn slash_toggles_lock_transparent_pixels_but_not_on_the_background() {
    let mut s = session(8);
    let bg = doc(&s).layers[0].id;
    assert!(doc(&s).is_background(bg));
    s.execute("layer.new.layer", json!({})).unwrap();
    let top = active(&s).id;
    let locked = |s: &Session, id: LayerId| doc(s).layer(id).unwrap().locks.transparency;
    assert_eq!(s.execute("layer.toggleLockTransparency", json!({})).unwrap()["locked"], true);
    assert!(locked(&s, top));
    s.execute("layer.toggleLockTransparency", json!({})).unwrap();
    assert!(!locked(&s, top));
    s.undo();
    assert!(locked(&s, top), "one undoable step each");
    let bg_locks = doc(&s).layer(bg).unwrap().locks;
    assert!(s.execute("layer.toggleLockTransparency", json!({"layer": bg.0})).is_err());
    assert_eq!(doc(&s).layer(bg).unwrap().locks, bg_locks, "the Background is left alone");
    assert!(s.execute("layer.toggleLockTransparency", json!({"layer": u64::MAX})).is_err());
}

#[test]
fn layer_utilities() {
    let mut s = session(8);
    // Layer from Background.
    assert!(s.is_enabled("layer.new.layerFromBackground"));
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    assert_eq!(doc(&s).layers[0].name, "Layer 0");
    assert!(!doc(&s).layers[0].locks.transparency);
    assert!(!s.is_enabled("layer.new.layerFromBackground"));
    // Copy / paste style and hide all effects.
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({})).unwrap();
    let top = doc(&s).layers[1].id;
    s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": top.0})).unwrap();
    assert_eq!(doc(&s).layers[1].effects.items.len(), 1);
    s.execute("layer.layerStyle.hideAllEffects", json!({})).unwrap();
    assert!(doc(&s).walk().iter().all(|(_, _, l)| l.effects.items.is_empty() || !l.effects.enabled));
    // Mask toggles.
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("layer.layerMask.enabled", json!({})).unwrap();
    assert!(!active(&s).mask.as_ref().unwrap().enabled);
    s.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
    assert!(!active(&s).mask.as_ref().unwrap().linked);
    // Hide, delete hidden, delete empty.
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(!active(&s).visible);
    assert_eq!(s.execute("layer.delete.hiddenLayers", json!({})).unwrap()["deleted"], 1);
    s.execute("layer.new.layer", json!({})).unwrap();
    let n = doc(&s).layer_count();
    assert_eq!(s.execute("file.scripts.deleteAllEmptyLayers", json!({})).unwrap()["deleted"], 1);
    assert_eq!(doc(&s).layer_count(), n - 1);
}

#[test]
fn ungroup_restores_children_in_order() {
    let mut s = session(8);
    let a = active(&s).id;
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert!(active(&s).is_group());
    s.execute("layer.ungroupLayers", json!({})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(a));
    assert!(doc(&s).layers.iter().all(|l| !l.is_group()));
    assert_eq!(doc(&s).layers.len(), 2);
}

#[test]
fn reveal_all_grows_the_canvas() {
    let mut s = session(8);
    s.execute("layer.translate", json!({"dx": 0, "dy": 0})).ok();
    paint_square(&mut s, Rect::new(-5, 0, 10, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(reveal_all_bounds(doc(&s)), Rect::new(-5, 0, 40, 30));
    let steps = s.active().unwrap().history.past_len();
    s.execute("image.revealAll", json!({})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (45, 30));
    let history = &s.active().unwrap().history;
    assert_eq!(history.past_len(), steps + 1, "one step");
    assert_eq!(history.entries().last().map(String::as_str), Some("Reveal All"), "named as in Photoshop, not Crop");
    assert_eq!(s.execute("image.revealAll", json!({})).unwrap()["changed"], false);
}

#[test]
fn reveal_all_skips_erased_pixels() {
    // #9379: the Eraser only clears alpha, so erased pixels keep their colour bytes; Reveal
    // All grew the canvas to them although nothing there is visible.
    // No Background: its off-canvas pixels are opaque and would be revealed anyway.
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "transparent"})).unwrap();
        paint_square(&mut s, Rect::new(0, 0, 40, 30), [1.0, 0.0, 0.0, 1.0]);
        let erase = json!({"points": [[32, -10], [32, 40]], "size": 28, "erase": true, "brush": {"hardness": 1.0}});
        s.execute("paint.stroke", erase).unwrap();
        assert_eq!(pixel(&s, 30, 15)[3], 0.0, "{depth}: erased");
        s.execute("image.canvasSize", json!({"width": 20, "height": 30, "anchor": "topLeft"})).unwrap();
        assert_eq!(s.execute("image.revealAll", json!({})).unwrap()["changed"], false, "{depth}");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (20, 30), "{depth}");
        // A pixel that is only partly erased is still there to reveal.
        paint_square(&mut s, Rect::new(25, 5, 26, 6), [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(s.execute("image.revealAll", json!({})).unwrap()["changed"], true, "{depth}");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (26, 30), "{depth}");
    }
}

#[test]
fn rasterize_fill_and_all_in_one_step() {
    let mut s = session(8);
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.rasterize.fillContent", json!({})).unwrap();
    assert!(matches!(active(&s).content, LayerContent::Raster(_)));
    assert_eq!(pixel(&s, 3, 3), vec![0.0, 0.0, 1.0, 1.0]);
    assert!(s.execute("layer.rasterize.type", json!({})).is_err());
    let steps = s.active().unwrap().history.past_len();
    assert_eq!(s.execute("layer.rasterize.allLayers", json!({})).unwrap()["rasterized"], 1);
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert!(doc(&s).walk().iter().all(|(_, _, l)| !matches!(l.content, LayerContent::Fill(_))));
}

#[test]
fn rasterize_smart_object_layer() {
    let mut s = session(8);
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    let px_before = pixel(&s, 5, 5);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s).content, LayerContent::Smart(_)));
    s.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s).content, LayerContent::Raster(_)));
    let px_after = pixel(&s, 5, 5);
    assert_eq!(px_after, px_before);

    // Case 2: New layer above background
    let mut s2 = session(8);
    s2.execute("layer.new.layer", json!({})).unwrap();
    s2.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s2.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    s2.execute("select.deselect", json!({})).unwrap();
    let px_mid = pixel(&s2, 20, 20);
    assert_eq!(px_mid[1], 1.0); // green
    s2.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s2).content, LayerContent::Smart(_)));
    s2.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s2).content, LayerContent::Raster(_)));
    let px_after_rast = pixel(&s2, 20, 20);
    assert_eq!(px_after_rast, px_mid);

    // Case 3: Shape layer converted to smart object then rasterized
    let mut s3 = session(8);
    s3.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#0000ff"})).unwrap();
    let px_shape = pixel(&s3, 20, 20);
    s3.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(matches!(active(&s3).content, LayerContent::Smart(_)));
    s3.execute("layer.rasterize.layer", json!({})).unwrap();
    assert!(matches!(active(&s3).content, LayerContent::Raster(_)));
    let px_after_shape = pixel(&s3, 20, 20);
    assert_eq!(px_after_shape, px_shape);

    // Case 4: Transformed smart object
    let mut s4 = session(8);
    s4.execute("layer.new.layer", json!({})).unwrap();
    s4.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s4.execute("edit.fill", json!({"contents": "color", "color": "#ffff00"})).unwrap();
    s4.execute("select.deselect", json!({})).unwrap();
    s4.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    // Move smart object by 10, 10
    s4.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 10]})).unwrap();
    s4.execute("layer.rasterize.smartObject", json!({})).unwrap();
    assert!(matches!(active(&s4).content, LayerContent::Raster(_)));
    assert_eq!(pixel(&s4, 30, 30), vec![1.0, 1.0, 0.0, 1.0]);
}
