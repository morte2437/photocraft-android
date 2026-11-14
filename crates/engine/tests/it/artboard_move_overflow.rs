//! Cross-board reparenting must either preserve board-relative pixels or refuse atomically.
use photocraft_doc::LayerId;
use photocraft_engine::{Session, layer_multi_cmds::preview_move};
use photocraft_geom::Rect;
use serde_json::{Value, json};

fn move_into(layer: LayerId, board: LayerId) -> Value {
    json!({"layer": layer.0, "target": board.0, "position": "into"})
}

/// Fill only the original 8x8 canvas, then create valid boards using public commands.
/// Even when their span caps the canvas metadata, the raster remains tiny and sparse.
fn two_boards(a: [i32; 2], b: [i32; 2]) -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    let red = LayerId(s.execute("layer.new.layer", json!({"name": "Red"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": [1, 0, 0, 1]})).unwrap();
    // Positive board first: creating the negative one must not rebase either board.
    let board_b = LayerId(s.execute("layer.new.artboard", json!({"name": "B", "rect": [b[0], b[1], 8, 8]})).unwrap()["layer"].as_u64().unwrap());
    let board_a = LayerId(s.execute("layer.new.artboard", json!({"name": "A", "rect": [a[0], a[1], 8, 8]})).unwrap()["layer"].as_u64().unwrap());
    for (id, [x, y]) in [(board_a, a), (board_b, b)] {
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().artboard().unwrap().rect, Rect::new(x, y, x + 8, y + 8));
    }
    // Joining a board from outside every board intentionally keeps canvas position.
    s.execute("layer.moveTo", move_into(red, board_a)).unwrap();
    s.execute("layer.translate", json!({"layer": red.0, "dx": a[0], "dy": a[1]})).unwrap();
    assert_eq!(s.active().unwrap().doc.artboard_of(red), Some(board_a));
    assert_pixels(&s, red, a);
    (s, red, board_a, board_b)
}

fn assert_pixels(s: &Session, layer: LayerId, [x, y]: [i32; 2]) {
    let surface = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
    assert_eq!(surface.content_bounds(), Rect::new(x, y, x + 8, y + 8));
    assert_eq!(surface.rgba(x + 3, y + 3), [1.0, 0.0, 0.0, 1.0]);
}

/// Both origins and all pixel endpoints fit i32, but the required +3,000,000,000
/// translation does not. A failed drop must not commit just the reparenting.
#[test]
fn unrepresentable_cross_board_move_is_refused_without_mutation() {
    let (mut s, red, _a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
    s.execute("layer.select", json!({"layer": red.0})).unwrap();
    let before = s.active().unwrap().clone();
    let params = move_into(red, b);
    let result = s.execute("layer.moveTo", params.clone());
    assert!(result.is_err(), "an unrepresentable required board offset must refuse the move: {result:?}");
    let after = s.active().unwrap();
    assert_eq!(after.doc, before.doc, "parent, pixels and the rest of the document stay unchanged");
    assert_eq!(after.active_layer, before.active_layer);
    assert_eq!(after.selected_layers, before.selected_layers);
    assert_eq!(after.layer_anchor, before.layer_anchor);
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.history.entries(), before.history.entries());
    assert_eq!(after.history.past_len(), before.history.past_len());
    assert_eq!(after.history.redo_labels().collect::<Vec<_>>(), before.history.redo_labels().collect::<Vec<_>>());
    assert!(preview_move(&before.doc, &params, true).is_none(), "the refused drop has no placement preview");
}

#[test]
fn ordinary_cross_board_move_preserves_pixels_preview_and_undo() {
    let (mut s, red, a, b) = two_boards([0, 0], [32, 16]);
    let before = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    let params = move_into(red, b);
    let preview = preview_move(&before, &params, true).expect("ordinary move has a preview");
    assert_eq!(s.active().unwrap().doc, before, "preview leaves the source alone");
    s.execute("layer.moveTo", params).unwrap();
    assert_eq!(*s.active().unwrap().doc, preview);
    assert_eq!(s.active().unwrap().doc.artboard_of(red), Some(b));
    assert_pixels(&s, red, [32, 16]);
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().doc.artboard_of(red), Some(a));
    assert!(s.redo());
    assert_pixels(&s, red, [32, 16]);
}

/// A lock on either side pins canvas position, so even distant boards require no shift.
#[test]
fn locked_cross_board_moves_still_reparent_without_translation() {
    for owner in ["layer", "source", "destination"] {
        for lock in ["position", "all"] {
            let (mut s, red, a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
            let locked = match owner {
                "layer" => red,
                "source" => a,
                _ => b,
            };
            s.execute("layer.select", json!({"layer": locked.0})).unwrap();
            let mut locks = json!({});
            locks[lock] = json!(true);
            s.execute("layer.lockLayers", locks).unwrap();
            let params = move_into(red, b);
            let preview = preview_move(&s.active().unwrap().doc, &params, true).expect("pinned move has a preview");
            s.execute("layer.moveTo", params).unwrap_or_else(|e| panic!("{owner}/{lock}: {e}"));
            assert_eq!(*s.active().unwrap().doc, preview, "{owner}/{lock}");
            assert_eq!(s.active().unwrap().doc.artboard_of(red), Some(b), "{owner}/{lock}");
            assert_pixels(&s, red, [-1_500_000_000, 0]);
        }
    }
}

/// Same-board moves and moves that leave or enter an artboard retain canvas pixels.
#[test]
fn same_board_and_moves_without_two_boards_keep_canvas_position() {
    let (mut s, red, a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
    for (params, parent) in [(move_into(red, a), Some(a)), (json!({"layer": red.0, "target": b.0, "position": "above"}), None), (move_into(red, b), Some(b))] {
        let preview = preview_move(&s.active().unwrap().doc, &params, true).expect("no-shift move has a preview");
        s.execute("layer.moveTo", params).unwrap();
        assert_eq!(*s.active().unwrap().doc, preview);
        assert_eq!(s.active().unwrap().doc.artboard_of(red), parent);
        assert_pixels(&s, red, [-1_500_000_000, 0]);
    }
}

/// Publicly created Adjustment/Fill content, with the nearest optional positioned state.
fn nonpixel_layer(s: &mut Session, board: LayerId, kind: &str, state: &str) -> LayerId {
    let command = if kind == "adjustment" { "layer.newAdjustmentLayer.invert" } else { "layer.newFillLayer.solidColor" };
    let params = if kind == "adjustment" { json!({}) } else { json!({"color": "#ff0000"}) };
    let id = LayerId(s.execute(command, params).unwrap()["layer"].as_u64().unwrap());
    s.edit("Set move fixture state", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        layer.mask = None;
        layer.vector_mask = None;
        layer.effects.reference = None;
        match state {
            "linked mask" | "unlinked mask" => {
                let mut mask = photocraft_doc::LayerMask::hide_all();
                mask.surface.fill_rect(Rect::new(1, 2, 3, 4), &[1.0]);
                mask.linked = state == "linked mask";
                layer.mask = Some(mask);
            }
            "linked vector" | "unlinked vector" => {
                let path = photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(1.0, 2.0), (3.0, 2.0), (3.0, 4.0)])]);
                let mut mask = photocraft_doc::VectorMask::new(path);
                mask.linked = state == "linked vector";
                layer.vector_mask = Some(mask);
            }
            "reference" => layer.effects.reference = Some((1.0, 2.0)),
            "none" => {}
            _ => panic!("unknown state"),
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.moveTo", move_into(id, board)).unwrap();
    id
}

/// Adjustment/Fill layers with no positioned state or only unlinked masks can move across distant boards while retaining layer state and undo.
fn unpositioned_move(kind: &str) {
    for state in ["none", "unlinked mask", "unlinked vector"] {
        let (mut s, _red, a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
        let id = nonpixel_layer(&mut s, a, kind, state);
        let before = s.active().unwrap().doc.clone();
        let shifted = photocraft_engine::layer_multi_cmds::moved(&before, &[id], 5, 7).unwrap();
        assert_eq!(shifted.layer(id), before.layer(id), "{kind}/{state}: no state moves");
        let params = move_into(id, b);
        s.execute("layer.moveTo", params.clone()).unwrap_or_else(|e| panic!("{kind}/{state}: unpositioned layer must reparent: {e}"));
        let preview = preview_move(&before, &params, true).expect("unpositioned move has a preview");
        assert_eq!(*s.active().unwrap().doc, preview);
        assert_eq!(s.active().unwrap().doc.artboard_of(id), Some(b));
        assert_eq!(s.active().unwrap().doc.layer(id), before.layer(id), "{kind}/{state}");
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn unpositioned_adjustment_can_reparent_between_distant_boards() {
    unpositioned_move("adjustment");
}

#[test]
fn unpositioned_fill_can_reparent_between_distant_boards() {
    unpositioned_move("fill");
}

/// Linked pixel/vector masks and effects references require translation, so overflow refuses before document, history, or selection is committed.
#[test]
fn positioned_adjustment_and_fill_overflow_still_refuses_atomically() {
    for kind in ["adjustment", "fill"] {
        for state in ["linked mask", "linked vector", "reference"] {
            let (mut s, _red, a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
            let id = nonpixel_layer(&mut s, a, kind, state);
            let before = s.active().unwrap().clone();
            let shifted = photocraft_engine::layer_multi_cmds::moved(&before.doc, &[id], 5, 7).unwrap();
            assert_ne!(shifted.layer(id), before.doc.layer(id), "{kind}/{state}: positioned state moves");
            let params = move_into(id, b);
            assert!(s.execute("layer.moveTo", params.clone()).is_err(), "{kind}/{state}");
            let after = s.active().unwrap();
            assert_eq!(after.doc, before.doc);
            assert_eq!(after.active_layer, before.active_layer);
            assert_eq!(after.selected_layers, before.selected_layers);
            assert_eq!(after.layer_anchor, before.layer_anchor);
            assert_eq!(after.revision, before.revision);
            assert_eq!(after.history.entries(), before.history.entries());
            assert!(preview_move(&before.doc, &params, true).is_none());
        }
    }
}

/// An ordinary cross-board move and preview match normal translation for each tested nonpixel state.
#[test]
fn representable_nonpixel_moves_match_the_existing_translation_path() {
    for kind in ["adjustment", "fill"] {
        for state in ["none", "unlinked mask", "linked mask", "unlinked vector", "linked vector", "reference"] {
            let (mut s, _red, a, b) = two_boards([0, 0], [32, 16]);
            let id = nonpixel_layer(&mut s, a, kind, state);
            let before = s.active().unwrap().doc.clone();
            let shifted = photocraft_engine::layer_multi_cmds::moved(&before, &[id], 32, 16).unwrap();
            let params = move_into(id, b);
            let preview = preview_move(&before, &params, true).unwrap();
            s.execute("layer.moveTo", params).unwrap();
            assert_eq!(*s.active().unwrap().doc, preview, "{kind}/{state}");
            assert_eq!(s.active().unwrap().doc.layer(id), shifted.layer(id), "{kind}/{state}");
        }
    }
}

#[test]
fn cross_board_copy_positions_the_landed_copy_and_keeps_the_source() {
    let (mut s, red, a, b) = two_boards([0, 0], [32, 16]);
    let before = s.active().unwrap().doc.clone();
    let mut params = move_into(red, b);
    params["copy"] = json!(true);
    let preview = preview_move(&before, &params, true).unwrap();
    let copy = LayerId(s.execute("layer.moveTo", params).unwrap()["layer"].as_u64().unwrap());
    assert_ne!(copy, red);
    assert_eq!(s.active().unwrap().doc.artboard_of(copy), Some(b));
    assert_eq!(s.active().unwrap().doc.artboard_of(red), Some(a));
    assert_eq!(s.active().unwrap().doc.layer(red), before.layer(red));
    assert_pixels(&s, copy, [32, 16]);
    assert_pixels(&s, red, [0, 0]);
    // Preview allocates its own copy ID; compare its placed pixels rather than those IDs.
    let preview_copy = preview.layer(b).unwrap().children().unwrap().last().unwrap();
    assert_eq!(preview_copy.surface(), s.active().unwrap().doc.layer(copy).unwrap().surface());
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn overflowing_cross_board_copy_refuses_without_leaving_a_copy() {
    let (mut s, red, _a, b) = two_boards([-1_500_000_000, 0], [1_500_000_000, 0]);
    let before = s.active().unwrap().clone();
    let mut params = move_into(red, b);
    params["copy"] = json!(true);
    assert!(s.execute("layer.moveTo", params.clone()).is_err());
    let after = s.active().unwrap();
    assert_eq!(after.doc, before.doc);
    assert_eq!(after.selected_layers, before.selected_layers);
    assert_eq!(after.active_layer, before.active_layer);
    assert_eq!(after.history.entries(), before.history.entries());
    assert_eq!(after.revision, before.revision);
    assert!(preview_move(&before.doc, &params, true).is_none());
}
