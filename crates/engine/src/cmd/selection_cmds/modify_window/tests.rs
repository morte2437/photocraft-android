use super::*;
use crate::Session;
use crate::selection_cmds::{crop_mask, pad_mask, touches_edge};
use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Size;
use photocraft_raster::Surface;
use serde_json::json;

fn document(w: u32, h: u32, selection: Surface) -> Document {
    let mut doc = Document::new("modify", Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    doc.selection = Some(selection);
    doc
}

fn full_canvas_reference(doc: &Document, op: &str, radius: f32, at_bounds: bool) -> Option<Surface> {
    let canvas = doc.bounds();
    let m = sel::mask_from_surface(doc.selection.as_ref(), canvas);
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let reach = if op == "feather" { sel::feather_reach(radius) } else { radius.ceil() as usize };
    let pad = if touches_edge(&m, w, h) { reach + 2 } else { 0 };
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let padded = if pad == 0 { m } else { pad_mask(&m, w, h, pad, op == "border" || at_bounds) };
    let out = match op {
        "expand" => sel::expand(&padded, pw, ph, radius),
        "contract" => sel::contract(&padded, pw, ph, radius),
        "border" => sel::border(&padded, pw, ph, radius),
        "smooth" => sel::smooth(&padded, pw, ph, radius),
        "feather" => sel::feather(&padded, pw, ph, radius),
        _ => panic!("unknown modify operation: {op}"),
    };
    let out = if pad == 0 { out } else { crop_mask(&out, pw, pad, w, h) };
    sel::combine(None, &out, canvas, SelectionMode::Replace)
}

#[test]
fn sparse_36mp_command_only_materializes_the_selection_halo() {
    let bounds = Rect::new(3000, 3000, 3050, 3050);
    let selection = sel::mask_to_surface(&vec![1.0; 50 * 50], bounds);
    let doc = document(6000, 6000, selection);
    assert_eq!(area(&doc, 5.0), Rect::new(2994, 2994, 3056, 3056));
    assert_eq!(area(&doc, 5.0).size().area(), 62 * 62);
    assert_eq!(area(&doc, sel::feather_reach(5.0) as f32), Rect::new(2984, 2984, 3066, 3066));
    assert_eq!(area(&doc, sel::feather_reach(5.0) as f32).size().area(), 82 * 82);
    for op in ["expand", "contract", "border", "smooth", "feather"] {
        let mut session = Session::new();
        session.add_document(doc.clone(), None);
        session.execute(&format!("select.modify.{op}"), json!({"radius": 5})).unwrap();
        assert!(session.active().unwrap().doc.selection.is_some());
    }
}

#[test]
fn cropped_commands_match_full_canvas_reads_padding_and_writeback() {
    let canvas = Rect::new(0, 0, 75, 61);
    for case in 0..8 {
        let values: Vec<f32> = (0..75 * 61)
            .map(|i| {
                let (x, y) = (i % 75, i / 75);
                match case {
                    0 => 0.0,
                    1 => 1.0,
                    2 if (28..39).contains(&x) && (22..33).contains(&y) => 1.0,
                    3 if x < 12 && (15..35).contains(&y) => 1.0,
                    4 if x >= 63 && y >= 48 => 1.0,
                    5 if (25..45).contains(&x) && (18..38).contains(&y) && !(30..35).contains(&x) => (i % 255 + 1) as f32 / 255.0,
                    _ => 0.0,
                }
            })
            .collect();
        let mut selection = sel::mask_to_surface(&values, canvas);
        if case == 6 {
            // Implicit full canvas with a hole: content_bounds only describes the hole.
            selection = Surface::with_default(PixelFormat::GRAY8, &[1.0]);
            selection.write_pixel(35, 30, &[0.0]);
        } else if case == 7 {
            selection.write_pixel(-10, -10, &[1.0]);
        }
        let doc = document(75, 61, selection);
        for radius in [0.1, 1.0, 1.5, 5.0, 9.5, 25.0] {
            for op in ["expand", "contract", "border", "smooth", "feather"] {
                if radius < 1.0 && op != "feather" {
                    continue;
                }
                for at_bounds in [false, true] {
                    let expected = full_canvas_reference(&doc, op, radius, at_bounds);
                    let mut session = Session::new();
                    session.add_document(doc.clone(), None);
                    session.execute(&format!("select.modify.{op}"), json!({"radius": radius, "applyAtCanvasBounds": at_bounds})).unwrap();
                    let actual = &session.active().unwrap().doc.selection;
                    assert_eq!(actual.is_some(), expected.is_some(), "case {case} {op} radius {radius} bounds {at_bounds}");
                    assert_eq!(
                        sel::mask_from_surface(actual.as_ref(), canvas),
                        sel::mask_from_surface(expected.as_ref(), canvas),
                        "case {case} {op} radius {radius} bounds {at_bounds}"
                    );
                }
            }
        }
    }
}

#[test]
fn cropped_blurs_preserve_soft_masks_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let format = PixelFormat { sample: depth, ..PixelFormat::GRAY8 };
        for default in [0.0, 0.25] {
            let mut selection = Surface::with_default(format, &[default]);
            for y in 21..39 {
                for x in 28..47 {
                    selection.write_pixel(x, y, &[((x * 13 + y * 7) % 17) as f32 / 16.0]);
                }
            }
            let mut doc = Document::new("soft modify", Size::new(83, 67), ColorMode::Rgb, depth);
            doc.selection = Some(selection);
            for op in ["smooth", "feather"] {
                for radius in [1.0, 2.5, 9.5] {
                    for at_bounds in [false, true] {
                        let expected = full_canvas_reference(&doc, op, radius, at_bounds);
                        let mut session = Session::new();
                        session.add_document(doc.clone(), None);
                        session.execute(&format!("select.modify.{op}"), json!({"radius": radius, "applyAtCanvasBounds": at_bounds})).unwrap();
                        assert_eq!(
                            sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), doc.bounds()),
                            sel::mask_from_surface(expected.as_ref(), doc.bounds()),
                            "{depth:?} default {default} {op} radius {radius} bounds {at_bounds}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn cropped_blurs_preserve_single_pixel_and_thin_canvases() {
    for (w, h) in [(1, 1), (1, 19), (23, 1)] {
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.write_pixel(w as i32 / 2, h as i32 / 2, &[1.0]);
        let doc = document(w, h, selection);
        for op in ["smooth", "feather"] {
            for radius in [1.0, 9.5] {
                for at_bounds in [false, true] {
                    let expected = full_canvas_reference(&doc, op, radius, at_bounds);
                    let mut session = Session::new();
                    session.add_document(doc.clone(), None);
                    session.execute(&format!("select.modify.{op}"), json!({"radius": radius, "applyAtCanvasBounds": at_bounds})).unwrap();
                    assert_eq!(
                        sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), doc.bounds()),
                        sel::mask_from_surface(expected.as_ref(), doc.bounds()),
                        "{w}x{h} {op} radius {radius} bounds {at_bounds}"
                    );
                }
            }
        }
    }
}

#[test]
fn cropped_replacement_keeps_undo_and_redo() {
    let canvas = Rect::new(0, 0, 75, 61);
    let selection = sel::mask_to_surface(&vec![1.0; 10 * 10], Rect::new(30, 25, 40, 35));
    let doc = document(75, 61, selection);
    let before = sel::mask_from_surface(doc.selection.as_ref(), canvas);
    for op in ["expand", "smooth", "feather"] {
        let mut session = Session::new();
        session.add_document(doc.clone(), None);
        session.execute(&format!("select.modify.{op}"), json!({"radius": 5})).unwrap();
        let after = sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), canvas);
        assert_ne!(before, after, "{op}");
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), canvas), before, "{op}");
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), canvas), after, "{op}");
    }
}

/// Isolate command-window savings: both paths use the optimized algorithm kernels, while
/// the reference retains full-canvas reads/writeback. Includes the undoable edit on both.
#[test]
#[ignore = "36 MP release benchmark; run on GitHub Actions"]
fn sparse_modify_release_comparison() {
    use std::time::Instant;
    for (kind, bounds) in [("sparse", Rect::new(3000, 3000, 3050, 3050)), ("dense", Rect::new(1000, 1000, 5000, 5000)), ("full", Rect::new(0, 0, 6000, 6000))] {
        let selection = sel::mask_to_surface(&vec![1.0; bounds.size().area() as usize], bounds);
        let doc = document(6000, 6000, selection);
        let ops: &[&str] = if kind == "sparse" { &["expand", "contract", "border", "smooth", "feather"] } else { &["smooth", "feather"] };
        for radius in [3.0, 50.0] {
            for &op in ops {
                let reference = full_canvas_reference(&doc, op, radius, false);
                let expected = sel::mask_from_surface(reference.as_ref(), doc.bounds());
                let mut old_times = Vec::new();
                let mut new_times = Vec::new();
                for round in 0..3 {
                    for old in if round % 2 == 0 { [true, false] } else { [false, true] } {
                        let mut session = Session::new();
                        session.add_document(doc.clone(), None);
                        let now = Instant::now();
                        if old {
                            let result = full_canvas_reference(&doc, op, radius, false);
                            session
                                .edit("Modify selection", |doc, _| {
                                    doc.selection = result;
                                    Ok(())
                                })
                                .unwrap();
                        } else {
                            session.execute(&format!("select.modify.{op}"), json!({"radius": radius})).unwrap();
                        }
                        let elapsed = now.elapsed().as_secs_f64() * 1000.0;
                        if old {
                            old_times.push(elapsed)
                        } else {
                            new_times.push(elapsed)
                        }
                        assert_eq!(sel::mask_from_surface(session.active().unwrap().doc.selection.as_ref(), doc.bounds()), expected);
                    }
                }
                old_times.sort_by(f64::total_cmp);
                new_times.sort_by(f64::total_cmp);
                let (old, new) = (old_times[1], new_times[1]);
                println!("36 MP {kind} command {op} r={radius}: full_canvas={old:.3}ms cropped={new:.3}ms speedup={:.2}x exact=true", old / new);
            }
        }
    }
}

#[test]
fn corrupt_tile_coordinates_do_not_panic_while_finding_the_window() {
    use photocraft_geom::TileCoord;
    use std::sync::Arc;
    let mut selection = Surface::new(PixelFormat::GRAY8);
    let tile = selection.solid_tile(&[1.0]);
    selection.put_tiles([(TileCoord::new(i32::MIN, i32::MIN), Arc::clone(&tile)), (TileCoord::new(i32::MAX, i32::MAX), tile)]);
    let doc = document(75, 61, selection);
    // Without the guard, content_bounds overflows while sorting these coordinates.
    assert_eq!(area(&doc, 5.0), doc.bounds());
    for op in ["expand", "contract", "border", "smooth", "feather"] {
        let mut session = Session::new();
        session.add_document(doc.clone(), None);
        session.execute(&format!("select.modify.{op}"), json!({"radius": 5})).unwrap();
        assert!(session.active().unwrap().doc.selection.is_none());
    }
}
