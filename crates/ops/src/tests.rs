use super::*;
use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};

fn base() -> Document {
    Document::with_background("h", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::WHITE)
}

/// Apply an edit the way the engine does: snapshot, mutate a clone, record.
fn edit(h: &mut History, cur: &mut Arc<Document>, label: &str, f: impl FnOnce(&mut Document)) {
    let before = cur.clone();
    let mut d = (**cur).clone();
    f(&mut d);
    *cur = Arc::new(d);
    h.record(label, before, LayerTarget::default());
}

#[test]
fn undo_redo_cycle() {
    let mut h = History::default();
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "New Layer", |d| {
        d.insert_above(None, Layer::raster("L", d.pixel_format()));
    });
    assert_eq!(cur.layers.len(), 2);
    assert_eq!(h.undo_label(), Some("New Layer"));
    cur = h.undo(cur).unwrap().0;
    assert_eq!(cur.layers.len(), 1);
    assert_eq!(h.redo_label(), Some("New Layer"));
    cur = h.redo(cur).unwrap().0;
    assert_eq!(cur.layers.len(), 2);
    assert!(h.redo(cur.clone()).is_none());
}

#[test]
fn undo_and_redo_return_the_layers_each_state_targeted_when_created() {
    let mut h = History::default();
    let open = Arc::new(base());
    let bg = open.layers[0].id;
    let target = |id: LayerId| LayerTarget { active: Some(id), selected: vec![id] };
    h.set_current_layers(target(bg));
    // New Layer targets the new layer; the background is then selected (not a step) and filled.
    let mut d = (*open).clone();
    let new = d.insert_above(None, Layer::raster("L", d.pixel_format()));
    let layered = Arc::new(d);
    h.record("New Layer", open, target(new));
    let mut d = (*layered).clone();
    d.name = "filled".into();
    h.record("Fill", layered, target(bg));
    let (d, layers) = h.undo(Arc::new(d)).unwrap();
    assert_eq!(layers, target(new), "New Layer's state targeted the new layer when created");
    let (d, layers) = h.undo(d).unwrap();
    assert_eq!(layers, target(bg), "the opened document's target");
    let (d, layers) = h.redo(d).unwrap();
    assert_eq!(layers, target(new));
    let (_, layers) = h.redo(d).unwrap();
    assert_eq!(layers, target(bg), "Fill's state targeted the background");
}

#[test]
fn state_accessor_matches_entries() {
    let mut h = History::default();
    let mut cur = Arc::new(base());
    let open = cur.clone();
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert_eq!(h.entries().len(), h.past_len() + 1);
    assert!(Arc::ptr_eq(&h.state(0).unwrap(), &open));
    assert_eq!(h.state(1).unwrap().name, "a");
    assert!(h.state(2).is_none(), "the current document is not held");
    cur = h.undo(cur).unwrap().0;
    h.undo(cur).unwrap();
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["A", "B"], "next redo first");
}

/// The History Brush source is a state, not an index: it stays on its state through undo,
/// redo and older states falling off, is kept when it would fall off itself, and is gone
/// (no source) once a new step discards it.
#[test]
fn brush_source_follows_its_state() {
    let mut h = History::new(3);
    let mut cur = Arc::new(base());
    assert_eq!(h.brush_source(), Some(0), "default: the oldest state");
    assert!(!h.set_brush_source(1), "no such row yet");
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert!(h.set_brush_source(1));
    assert_eq!(h.row_doc(1, &cur).unwrap().name, "a");
    // Undo / redo: the rows stay put, and A's state is still row 1.
    cur = h.undo(cur).unwrap().0;
    assert_eq!((h.brush_source(), h.row_doc(1, &cur).unwrap().name.as_str()), (Some(1), "a"));
    assert_eq!(h.row_doc(2, &cur).unwrap().name, "b", "a redo row");
    cur = h.redo(cur).unwrap().0;
    // Open falls off the 3-state limit: A's state moves up to row 0 and stays the source.
    edit(&mut h, &mut cur, "C", |d| d.name = "c".into());
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert_eq!(h.entries(), ["A", "B", "C", "D"]);
    assert_eq!(h.brush_source(), Some(0));
    // A would fall off next: it is kept, and the states after it fall off instead.
    edit(&mut h, &mut cur, "E", |d| d.name = "e".into());
    edit(&mut h, &mut cur, "F", |d| d.name = "f".into());
    assert_eq!(h.entries(), ["A", "D", "E", "F"]);
    assert_eq!((h.brush_source(), h.row_doc(0, &cur).unwrap().name.as_str()), (Some(0), "a"));
    // A redo state as the source, discarded by a new step: no source until one is set.
    cur = h.undo(cur).unwrap().0;
    assert!(h.set_brush_source(3), "the greyed F row");
    assert_eq!((h.brush_source(), h.row_doc(3, &cur).unwrap().name.as_str()), (Some(3), "f"));
    assert!(h.row_doc(4, &cur).is_none() && h.row_id(4).is_none());
    edit(&mut h, &mut cur, "G", |d| d.name = "g".into());
    assert_eq!(h.brush_source(), None);
    // The current state as the source keeps it once it becomes a past state; once it is no
    // longer the source, it falls off as usual.
    assert!(h.set_brush_source(3));
    edit(&mut h, &mut cur, "H", |d| d.name = "h".into());
    assert_eq!((h.brush_source(), h.row_doc(2, &cur).unwrap().name.as_str()), (Some(2), "g"));
    assert!(h.set_brush_source(1));
    edit(&mut h, &mut cur, "I", |d| d.name = "i".into());
    assert_eq!(h.entries(), ["E", "G", "H", "I"]);
}

#[test]
fn byte_budget_keeps_the_brush_source() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    for i in 0..5 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    assert!(h.set_brush_source(1));
    h.max_bytes = 1;
    let dropped = h.trim(&cur);
    assert_eq!(h.entries(), ["paint 0", "paint 3", "paint 4"], "the source is kept with the last step");
    assert_eq!(dropped, 3);
    assert_eq!(h.brush_source(), Some(0));
}

/// The pinned source counts against the byte budget: with room for three states, the
/// source and the two newest are kept, never more than the budget (#3536 review).
#[test]
fn byte_budget_counts_the_brush_source_first() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    let tile = h.pixel_bytes(&cur);
    assert!(tile > 0);
    for i in 0..5 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    // Every state holds one tile of its own; the source is the oldest (Open).
    assert!(h.set_brush_source(0));
    h.max_bytes = tile * 4;
    assert_eq!(h.trim(&cur), 2);
    assert_eq!(h.entries(), ["Open", "paint 2", "paint 3", "paint 4"]);
    assert!(h.pixel_bytes(&cur) <= h.max_bytes, "{} > {}", h.pixel_bytes(&cur), h.max_bytes);
    assert_eq!(h.trim(&cur), 0, "it fits now");
    // A new step trims to the budget again, still keeping the source.
    edit(&mut h, &mut cur, "paint 5", |d| {
        let id = d.layers[0].id;
        d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[1.0, 0.0, 0.0, 1.0]);
    });
    assert_eq!(h.trim(&cur), 1);
    assert_eq!(h.entries(), ["Open", "paint 3", "paint 4", "paint 5"]);
    assert!(h.pixel_bytes(&cur) <= h.max_bytes);
    assert_eq!(h.brush_source(), Some(0));
}

/// Edit › Purge › Histories clears every state: a source set on one of them goes back to the
/// default (the oldest state held) instead of leaving no source (#3536 review).
#[test]
fn clear_resets_the_brush_source() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert!(h.set_brush_source(1));
    h.clear();
    assert_eq!(h.brush_source(), Some(0));
    assert_eq!(h.row_doc(0, &cur).unwrap().name, "b");
}

/// History panel › Clear History leaves a usable (default) source, and its undo restores the
/// source set on a past state.
#[test]
fn clear_states_resets_the_brush_source_until_undo() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert!(h.set_brush_source(1));
    assert!(h.clear_states());
    assert_eq!(h.brush_source(), Some(0));
    assert_eq!(h.row_doc(0, &cur).unwrap().name, "b");
    cur = h.undo(cur).unwrap().0;
    assert_eq!((h.brush_source(), h.row_doc(1, &cur).unwrap().name.as_str()), (Some(1), "a"));
}

/// The byte budget keeps the source set before Clear History among the cleared states, so
/// Undo Clear History brings it back as the History Brush's source.
#[test]
fn byte_budget_keeps_the_brush_source_of_the_cleared_states() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    for i in 0..5 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    assert!(h.set_brush_source(1));
    let source = h.state(1).unwrap();
    assert!(h.clear_states());
    h.max_bytes = 1;
    assert_eq!(h.trim(&cur), 3);
    assert_counter(&h, &cur);
    assert!(h.is_cleared() && h.past_len() == 0, "the list stays cleared");
    cur = h.undo(cur).unwrap().0;
    assert_eq!(h.entries(), ["paint 0", "paint 3", "paint 4"], "the source is kept with the last step");
    assert_eq!(h.brush_source(), Some(0));
    assert!(Arc::ptr_eq(&h.row_doc(0, &cur).unwrap(), &source), "the History Brush paints from the same state");
}

/// When the History Brush's source is the newest past state, it is the state the byte budget
/// always keeps, and no older state is kept beside it.
#[test]
fn byte_budget_counts_a_newest_brush_source_as_the_kept_state() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    for i in 0..5 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    assert!(h.set_brush_source(4));
    let source = h.state(4).unwrap();
    h.max_bytes = 1;
    assert_eq!(h.trim(&cur), 4);
    assert_counter(&h, &cur);
    assert_eq!(h.past_len(), 1);
    assert_eq!(h.brush_source(), Some(0));
    assert!(Arc::ptr_eq(&h.row_doc(0, &cur).unwrap(), &source));
}

#[test]
fn new_edit_clears_redo() {
    let mut h = History::default();
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    cur = h.undo(cur).unwrap().0;
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert!(!h.can_redo());
    assert_eq!(h.entries(), vec!["Open".to_string(), "B".to_string()]);
}

/// Open, A, B, C, then two steps back: A is current, B and C are redo states.
fn stepped_back(non_linear: bool) -> (History, Arc<Document>) {
    let mut h = History { non_linear, ..History::default() };
    let mut cur = Arc::new(base());
    for n in ["A", "B", "C"] {
        edit(&mut h, &mut cur, n, |d| d.name = n.to_lowercase());
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    assert_eq!(cur.name, "a");
    (h, cur)
}

#[test]
fn non_linear_history_keeps_later_states_and_appends_the_new_step() {
    let (mut h, mut cur) = stepped_back(true);
    edit(&mut h, &mut cur, "D", |d| d.name = format!("{}+d", d.name));
    assert_eq!(h.entries(), ["Open", "A", "B", "C", "D"], "B and C are kept, D goes to the end");
    assert!(!h.can_redo());
    assert_eq!(cur.name, "a+d", "D was made from the state selected (A), not from C");
    // Stepping back walks the list: C, B, A, Open, each with its own document.
    let mut names = Vec::new();
    while let Some((d, _)) = h.undo(cur.clone()) {
        cur = d;
        names.push(cur.name.clone());
    }
    assert_eq!(names, ["c", "b", "a", "h"]);
    // Linear history (the default) discards them.
    let (mut h, mut cur) = stepped_back(false);
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert_eq!(h.entries(), ["Open", "A", "D"]);
}

#[test]
fn non_linear_steps_respect_the_state_limit_and_the_tile_counter() {
    let mut h = History::new(3);
    h.non_linear = true;
    h.max_bytes = usize::MAX;
    let mut cur = Arc::new(base());
    for i in 0..4 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
        h.trim(&cur);
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    edit(&mut h, &mut cur, "branch", |d| d.name = "branch".into());
    assert_eq!(h.past_len(), 3, "the oldest states beyond the limit go");
    assert_eq!(h.entries(), ["paint 1", "paint 2", "paint 3", "branch"]);
    assert_counter(&h, &cur);
}

#[test]
fn remove_deletes_one_past_state_and_drop_current_keeps_the_redo_states() {
    let (mut h, _) = stepped_back(true);
    assert!(!h.remove(0), "the oldest state stays");
    assert!(!h.remove(1), "the current state (A) is not held");
    assert!(!h.remove(99));
    let (mut h, mut cur) = stepped_back(true);
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert!(h.remove(1));
    assert_eq!(h.entries(), ["Open", "B", "C", "D"]);
    assert_eq!(h.state(1).unwrap().name, "b", "each state keeps its own document");
    // Deleting the current state: the one before becomes current, the later ones stay.
    let (mut h, _) = stepped_back(true);
    let (d, _) = h.drop_current().unwrap();
    assert_eq!(d.name, "h");
    assert_eq!(h.entries(), ["Open"]);
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["B", "C"]);
    assert!(h.drop_current().is_none(), "nothing before Open");
    // The tile counter follows both.
    let mut h = History::new(50);
    h.non_linear = true;
    h.max_bytes = usize::MAX;
    let mut cur = Arc::new(base());
    for i in 0..4 {
        edit(&mut h, &mut cur, "Paint", |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
        h.trim(&cur);
    }
    cur = h.undo(cur).unwrap().0;
    assert!(h.remove(2));
    assert_counter(&h, &cur);
    cur = h.drop_current().unwrap().0;
    assert_counter(&h, &cur);
}

#[test]
fn remove_later_deletes_one_redo_state() {
    let mut h = History::new(50);
    h.non_linear = true;
    h.max_bytes = usize::MAX;
    let mut cur = Arc::new(base());
    for (i, n) in ["A", "B", "C"].into_iter().enumerate() {
        edit(&mut h, &mut cur, n, |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
        h.trim(&cur);
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    assert!(!h.remove_later(2) && !h.remove_later(usize::MAX));
    assert!(h.remove_later(0), "B");
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["C"]);
    assert_counter(&h, &cur);
    let (c, _) = h.redo(cur).unwrap();
    assert_eq!(h.entries(), ["Open", "A", "C"]);
    assert_counter(&h, &c);
}

/// Each state keeps the layers whose visibility its own step changed; when a state goes
/// (Purge, a non-linear Delete) or a step is appended after the kept later states, the step
/// that now follows covers the changes of the steps it now spans.
#[test]
fn step_visibility_follows_steps_and_spans_removed_ones() {
    let vis = |h: &History| {
        let mut v: Vec<u64> = h.current_visibility().iter().map(|l| l.0).collect();
        v.sort_unstable();
        v
    };
    // Open, then steps A, B, C, D changing layers 1, 2, 3, 4.
    let steps = |n: usize, non_linear: bool| {
        let mut h = History { non_linear, ..History::default() };
        let mut cur = Arc::new(base());
        for (i, label) in ["A", "B", "C", "D"].into_iter().take(n).enumerate() {
            edit(&mut h, &mut cur, label, |d| d.name = label.to_lowercase());
            h.add_current_visibility(&[LayerId(i as u64 + 1)]);
        }
        (h, cur)
    };
    let (mut h, cur) = steps(3, false);
    assert_eq!(vis(&h), [3]);
    let (cur, _) = h.undo(cur).unwrap();
    assert_eq!(vis(&h), [2]);
    h.redo(cur).unwrap();
    assert_eq!(vis(&h), [3]);
    // Purge: C now starts from A.
    h.purge_last();
    assert_eq!(vis(&h), [2, 3]);
    // Non-linear Delete of B (between A and C): C now follows A.
    let (mut h, cur) = steps(4, true);
    assert!(h.remove(2));
    h.undo(cur).unwrap();
    assert_eq!(vis(&h), [2, 3]);
    // Of the later B (A current): C follows A.
    let (mut h, mut cur) = steps(3, true);
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    assert!(h.remove_later(0));
    h.redo(cur).unwrap();
    assert_eq!(vis(&h), [2, 3]);
    // Of the current B (C later): A is current, C follows it.
    let (mut h, cur) = steps(3, true);
    h.undo(cur).unwrap();
    let cur = h.drop_current().unwrap().0;
    assert_eq!(vis(&h), [1]);
    h.redo(cur).unwrap();
    assert_eq!(vis(&h), [2, 3]);
    // A step after the kept later states B and C (made from A) follows C in the list.
    let (mut h, mut cur) = steps(3, true);
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    h.add_current_visibility(&[LayerId(4)]);
    assert_eq!(vis(&h), [2, 3, 4]);
    h.rollback().unwrap();
    assert_eq!(vis(&h), [1], "rollback restores A's");
    h.clear();
    assert_eq!(vis(&h), Vec::<u64>::new());
}

#[test]
fn rollback_takes_the_last_record_back_exactly_in_both_modes() {
    // Non-linear, with redo states: the moved states go back to the redo side and the
    // document the step replaced (A) is current again, not the last former redo state (C).
    let (mut h, mut cur) = stepped_back(true);
    let a = cur.clone();
    let entries = h.entries();
    let redo: Vec<String> = h.redo_labels().map(str::to_string).collect();
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert_eq!(h.recorded_before().map(|d| d.name.clone()).as_deref(), Some("a"));
    let (d, _) = h.rollback().unwrap();
    assert!(Arc::ptr_eq(&d, &a));
    assert_eq!(h.entries(), entries);
    assert_eq!(h.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);
    assert!(h.rollback().is_none(), "only once");
    // Redoing still walks the kept states.
    let (b, _) = h.redo(d).unwrap();
    assert_eq!(b.name, "b");
    // Linear: the same as an undo whose redo state is dropped.
    let (mut h, mut cur) = stepped_back(false);
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert_eq!(h.rollback().unwrap().0.name, "a");
    assert_eq!(h.entries(), ["Open", "A"]);
    assert!(!h.can_redo());
    // Anything else since the record: no rollback (the caller undoes instead).
    let (mut h, mut cur) = stepped_back(true);
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    cur = h.undo(cur).unwrap().0;
    assert!(h.recorded_before().is_none() && h.rollback().is_none());
    h.redo(cur).unwrap();
    assert!(h.rollback().is_none());
    // The tile counter follows.
    let mut h = History::new(50);
    h.non_linear = true;
    h.max_bytes = usize::MAX;
    let mut cur = Arc::new(base());
    for i in 0..3 {
        edit(&mut h, &mut cur, "Paint", |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
        h.trim(&cur);
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    edit(&mut h, &mut cur, "Paint", |d| {
        d.layers[0].surface_mut().unwrap().write_pixel(2, 2, &[1.0, 0.0, 0.0, 1.0]);
    });
    cur = h.rollback().unwrap().0;
    assert_counter(&h, &cur);
}

/// A failed compound command whose first step moved more later states than the state limit
/// leaves room for (or whose step the byte budget trimmed for) is still taken back exactly:
/// the states the limit dropped for it come back, as in the list before the command.
#[test]
fn rollback_brings_back_the_states_the_limit_dropped_for_the_step() {
    for max in 1..=4 {
        let (mut h, mut cur) = stepped_back(true);
        h.max_states = max;
        let a = cur.clone();
        edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
        assert!(h.past_len() <= max);
        assert_eq!(h.recorded_before().map(|d| d.name.clone()).as_deref(), Some("a"), "max {max}");
        let (d, _) = h.rollback().unwrap();
        assert!(Arc::ptr_eq(&d, &a), "max {max}: back on A, not on an Undo of C");
        assert_eq!(h.entries(), ["Open", "A"], "max {max}");
        assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["B", "C"], "max {max}");
        assert!(h.rollback().is_none());
    }
    // Once anything else changed the list, the dropped states stay dropped.
    let (mut h, mut cur) = stepped_back(true);
    h.max_states = 2;
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    cur = h.undo(cur).unwrap().0;
    h.redo(cur).unwrap();
    assert!(h.rollback().is_none());
    assert_eq!(h.entries(), ["B", "C", "D"]);
    // The byte budget: the trim after the step keeps only its newest state; the rollback
    // brings back the rest and the tile counter follows.
    let mut h = History { non_linear: true, ..History::default() };
    let mut cur = Arc::new(base());
    for (i, n) in ["A", "B", "C"].into_iter().enumerate() {
        edit(&mut h, &mut cur, n, |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    h.max_bytes = 1;
    edit(&mut h, &mut cur, "D", |d| {
        d.layers[0].surface_mut().unwrap().write_pixel(2, 2, &[1.0, 0.0, 0.0, 1.0]);
    });
    assert!(h.trim(&cur) > 0);
    assert_eq!(h.entries(), ["C", "D"]);
    assert_counter(&h, &cur);
    cur = h.rollback().unwrap().0;
    assert_eq!(h.entries(), ["Open", "A"]);
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["B", "C"]);
    assert_counter(&h, &cur);
}

/// The History Brush's source never falls off, so the limit and the byte budget drop states
/// behind it, not at the front; a rollback still puts each back where it was.
#[test]
fn rollback_puts_states_back_behind_the_pinned_brush_source() {
    let (mut h, mut cur) = stepped_back(true);
    assert!(h.set_brush_source(0));
    h.max_states = 2;
    let a = cur.clone();
    edit(&mut h, &mut cur, "D", |d| d.name = "d".into());
    assert_eq!(h.entries(), ["Open", "C", "D"], "A and B go, the source (Open) stays");
    let (d, _) = h.rollback().unwrap();
    assert!(Arc::ptr_eq(&d, &a));
    assert_eq!(h.entries(), ["Open", "A"]);
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["B", "C"]);
    assert_eq!(h.brush_source(), Some(0));
    assert_eq!(h.state(0).unwrap().name, "h");
    // The byte budget, with the source on the oldest state.
    let mut h = History { non_linear: true, ..History::default() };
    let mut cur = Arc::new(base());
    for (i, n) in ["A", "B", "C"].into_iter().enumerate() {
        edit(&mut h, &mut cur, n, |d| {
            d.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    cur = h.undo(cur).unwrap().0;
    cur = h.undo(cur).unwrap().0;
    assert!(h.set_brush_source(0));
    h.max_bytes = 1;
    edit(&mut h, &mut cur, "D", |d| {
        d.layers[0].surface_mut().unwrap().write_pixel(2, 2, &[1.0, 0.0, 0.0, 1.0]);
    });
    assert!(h.trim(&cur) > 0);
    assert_eq!(h.entries(), ["Open", "C", "D"]);
    cur = h.rollback().unwrap().0;
    assert_eq!(cur.name, "h");
    assert_eq!(h.entries(), ["Open", "A"]);
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["B", "C"]);
    assert_eq!(h.state(0).unwrap().name, "h");
    assert_counter(&h, &cur);
}

#[test]
fn max_states_bound() {
    let mut h = History::new(3);
    let mut cur = Arc::new(base());
    for i in 0..10 {
        edit(&mut h, &mut cur, &format!("step {i}"), |d| d.name = format!("{i}"));
    }
    let mut n = 0;
    while let Some((d, _)) = h.undo(cur.clone()) {
        cur = d;
        n += 1;
    }
    assert_eq!(n, 3);
    assert_eq!(cur.name, "6");
}

#[test]
fn byte_budget_drops_oldest_states_but_keeps_one() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    let tile = 256 * 256 * 4;
    // Each step repaints the background's tile: one unique tile per state.
    for i in 0..6 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    assert_eq!(h.trim(&cur), 0, "unlimited by default");
    assert_eq!(h.past_len(), 6);
    // The current tile plus three history tiles fit.
    h.max_bytes = 4 * tile;
    assert_eq!(h.trim(&cur), 3);
    assert_eq!(h.past_len(), 3);
    assert_eq!(h.unique_bytes(&cur), 3 * tile);
    assert_eq!(h.pixel_bytes(&cur), 4 * tile, "the current tile plus history's");
    assert_eq!(h.entries()[0], "paint 2");
    // A budget smaller than one state still keeps the last step undoable.
    h.max_bytes = 1;
    h.trim(&cur);
    assert_eq!(h.past_len(), 1);
    assert!(h.undo(cur.clone()).is_some());
}

/// Snapshot-only tiles count against the budget (#4530); tiles a snapshot shares with the
/// document or a state count once, and a purge frees only what no snapshot keeps.
#[test]
fn snapshot_tiles_count_against_the_budget() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    let tile = 256 * 256 * 4;
    let paint = |d: &mut Document, v: f32| {
        let id = d.layers[0].id;
        d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[v, 0.0, 0.0, 1.0]);
    };
    // A snapshot holding a tile nothing else does, then one sharing the first state's tile.
    let mut own = (*cur).clone();
    paint(&mut own, 0.9);
    edit(&mut h, &mut cur, "paint 0", |d| paint(d, 0.1));
    let shared = h.state(0).unwrap();
    for i in 1..6 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| paint(d, i as f32 / 8.0));
    }
    let snaps = [&own, &*shared];
    assert_eq!(h.extra_bytes(&cur, &snaps), tile, "the shared tile is history's");
    assert_eq!(h.extra_bytes(&cur, &[]), 0);
    assert_eq!(h.unique_bytes(&cur), 6 * tile);
    assert_eq!(h.unique_bytes_besides(&cur, &snaps), 5 * tile, "clearing keeps the shared tile");
    // Both snapshot tiles stay whatever is trimmed (the shared one too), so beside the
    // current tile they leave room for two history tiles; without them, four would fit.
    h.max_bytes = 5 * tile;
    assert_eq!(h.trim_with(&cur, &snaps), 4);
    assert_eq!(h.past_len(), 2);
    assert_eq!(h.pixel_bytes(&cur) + h.extra_bytes(&cur, &snaps), 5 * tile);
    // Still keeps the last step undoable.
    h.max_bytes = 1;
    h.trim_with(&cur, &snaps);
    assert_eq!(h.past_len(), 1);
}

#[test]
fn history_shares_untouched_tiles() {
    let mut h = History::default();
    let mut cur = Arc::new(base());
    assert_eq!(h.unique_bytes(&cur), 0);
    // rename only: no pixel changes, history holds no unique tiles
    edit(&mut h, &mut cur, "Rename", |d| d.name = "x".into());
    assert_eq!(h.unique_bytes(&cur), 0);
    // paint one pixel: exactly one tile becomes unique to history
    edit(&mut h, &mut cur, "Paint", |d| {
        let id = d.layers[0].id;
        d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[0.0, 0.0, 0.0, 1.0]);
    });
    assert_eq!(h.unique_bytes(&cur), 256 * 256 * 4);
}

#[test]
fn fill_caches_and_smart_filter_masks_count_toward_history_bytes() {
    use photocraft_doc::{Affine, Fill, FillCache, LayerContent, LayerMask, PixelFormat, SmartObject, SmartSource, Surface};
    let mut doc = Document::new("fills", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    let mut fill = Layer::new("Fill", LayerContent::Fill(Fill::Solid(Color::WHITE)));
    let mut cache = Surface::new(PixelFormat::GRAY8);
    cache.write_pixel(1, 1, &[1.0]);
    fill.fill_cache = Some(FillCache { fill: Fill::Solid(Color::WHITE), surface: cache });
    doc.insert_above(None, fill);
    let mut smart_obj = SmartObject::new(SmartSource::Linked { path: "a.png".into() }, Affine::IDENTITY, None);
    let mut mask = LayerMask::reveal_all();
    mask.surface.write_pixel(1, 1, &[0.5]);
    smart_obj.filter_mask = Some(mask);
    doc.insert_above(None, Layer::new("Smart", LayerContent::Smart(smart_obj)));
    let tile = 256 * 256;
    let mut cur = Arc::new(doc);
    let mut h = History { max_bytes: usize::MAX, ..History::default() };
    assert_eq!(h.pixel_bytes(&cur), 2 * tile, "fill cache and smart filter mask are counted");
    edit(&mut h, &mut cur, "Rename", |doc| doc.name = "renamed".into());
    assert_eq!(h.pixel_bytes(&cur), 2 * tile, "shared tiles count once");
    edit(&mut h, &mut cur, "Fill cache", |doc| {
        let id = doc.layers[0].id;
        if let Some(cache) = doc.layer_mut(id).and_then(|l| l.fill_cache.as_mut()) {
            cache.surface.write_pixel(1, 1, &[0.0]);
        }
    });
    assert_eq!(h.pixel_bytes(&cur), 3 * tile, "the copied fill cache tile is history-only");
    assert_eq!(h.unique_bytes(&cur), tile);
    h.trim(&cur);
    assert_counter(&h, &cur);
    h.max_bytes = 2 * tile;
    assert_eq!(h.trim(&cur), 1, "the copied fill cache tile pushes history over budget");
    assert_eq!(h.pixel_bytes(&cur), 3 * tile, "the most recent undo state is always kept");
    assert_counter(&h, &cur);
}

const TILE: usize = 256 * 256 * 4;
const SOURCE: usize = 1_000_000;

fn embedded(len: usize) -> photocraft_doc::SmartSource {
    photocraft_doc::SmartSource::Embedded { file_name: "source.pcraft".into(), bytes: Arc::new(vec![7; len]) }
}

/// `base()` with one smart object holding `source`; returns the document and the layer's id.
fn smart_document(source: photocraft_doc::SmartSource) -> (Document, photocraft_doc::LayerId) {
    use photocraft_doc::{Affine, LayerContent, SmartObject};
    let mut doc = base();
    let layer = Layer::new("Smart", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, None)));
    let id = layer.id;
    doc.insert_above(None, layer);
    (doc, id)
}

fn replace_source(doc: &mut Document, id: photocraft_doc::LayerId, source: photocraft_doc::SmartSource) {
    if let Some(photocraft_doc::LayerContent::Smart(sm)) = doc.layer_mut(id).map(|l| &mut l.content) {
        sm.source = source;
    }
}

#[test]
fn embedded_smart_object_contents_count_toward_history_bytes() {
    let (with, _) = smart_document(embedded(SOURCE));
    let without = base();
    let mut h = History::default();
    assert_eq!(h.pixel_bytes(&with), h.pixel_bytes(&without) + SOURCE, "uncounted history walks the document");
    assert_eq!(h.pixel_bytes(&without), TILE);
    // With a budget the counter keeps the figure incrementally.
    let mut cur = Arc::new(with.clone());
    h.max_bytes = usize::MAX;
    edit(&mut h, &mut cur, "Rename", |d| d.name = "renamed".into());
    edit(&mut h, &mut cur, "Rename again", |d| d.name = "renamed again".into());
    h.trim(&cur);
    assert_eq!(h.pixel_bytes(&cur), TILE + SOURCE);
    assert_counter(&h, &cur);
    // A linked source holds no bytes of its own.
    let (linked, _) = smart_document(photocraft_doc::SmartSource::Linked { path: "a.png".into() });
    assert_eq!(History::default().pixel_bytes(&linked), TILE);
}

#[test]
fn states_sharing_one_embedded_source_count_it_once() {
    let (doc, _) = smart_document(embedded(SOURCE));
    let mut cur = Arc::new(doc);
    let mut h = History { max_bytes: usize::MAX, ..History::default() };
    for i in 0..10 {
        edit(&mut h, &mut cur, &format!("Rename {i}"), |d| d.name = format!("renamed {i}"));
    }
    h.trim(&cur);
    assert_eq!(h.past_len(), 10);
    assert_eq!(h.pixel_bytes(&cur), TILE + SOURCE);
    assert_eq!(h.unique_bytes(&cur), 0);
    assert_counter(&h, &cur);
    // A duplicated layer shares the same bytes too.
    edit(&mut h, &mut cur, "Duplicate", |d| {
        if let Some(copy) = d.layers.last().cloned() {
            d.insert_above(None, copy);
        }
    });
    h.trim(&cur);
    assert_eq!(h.pixel_bytes(&cur), TILE + SOURCE);
    assert_counter(&h, &cur);
}

#[test]
fn a_replaced_embedded_source_counts_until_the_state_holding_it_is_trimmed() {
    const REPLACED: usize = 400_000;
    let (doc, id) = smart_document(embedded(SOURCE));
    let mut cur = Arc::new(doc);
    let mut h = History { max_bytes: usize::MAX, ..History::default() };
    edit(&mut h, &mut cur, "Replace Contents", |d| replace_source(d, id, embedded(REPLACED)));
    assert_eq!(h.pixel_bytes(&cur), TILE + SOURCE + REPLACED, "the state before still holds the old contents");
    assert_eq!(h.unique_bytes(&cur), SOURCE);
    edit(&mut h, &mut cur, "Rename", |d| d.name = "renamed".into());
    h.trim(&cur);
    assert_counter(&h, &cur);
    h.max_bytes = TILE + REPLACED;
    assert_eq!(h.trim(&cur), 1, "dropping the oldest state releases the old contents");
    assert_eq!(h.pixel_bytes(&cur), TILE + REPLACED);
    assert_counter(&h, &cur);
}

#[test]
fn embedded_contents_push_old_states_out_of_the_budget() {
    let (doc, id) = smart_document(embedded(SOURCE));
    let mut cur = Arc::new(doc);
    let mut h = History::new(50);
    for i in 0..5 {
        // Each step swaps in contents of its own; only the tile is shared between states.
        edit(&mut h, &mut cur, &format!("Replace {i}"), |d| replace_source(d, id, embedded(SOURCE)));
    }
    assert_eq!(h.trim(&cur), 0, "unlimited by default");
    assert_eq!(h.pixel_bytes(&cur), TILE + 6 * SOURCE);
    // The current contents and the two newest states fit.
    h.max_bytes = TILE + 3 * SOURCE;
    assert_eq!(h.trim(&cur), 3);
    assert_eq!(h.past_len(), 2);
    assert_eq!(h.pixel_bytes(&cur), TILE + 3 * SOURCE);
    assert_counter(&h, &cur);
}

#[test]
fn selection_tiles_count_current_and_history_without_double_counting() {
    use photocraft_doc::{PixelFormat, Surface};
    for depth in SampleType::ALL {
        let mut doc = Document::new("selection", Size::new(300, 64), ColorMode::Rgb, depth);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        selection.write_pixel(1, 1, &[1.0]);
        selection.write_pixel(257, 1, &[1.0]);
        doc.selection = Some(selection);
        let mut cur = Arc::new(doc);
        let tile = match depth {
            SampleType::U8 => 65_536,
            SampleType::U16 => 131_072,
            SampleType::F32 => 262_144,
        };
        let mut h = History { max_bytes: usize::MAX, ..History::default() };
        assert_eq!(h.pixel_bytes(&cur), 2 * tile, "current selection counts without any history");
        assert_eq!(h.unique_bytes(&cur), 0);
        edit(&mut h, &mut cur, "Rename", |doc| doc.name = "renamed".into());
        assert_eq!(h.pixel_bytes(&cur), 2 * tile, "the shared selection tiles count once");
        edit(&mut h, &mut cur, "Selection", |doc| {
            doc.selection.as_mut().unwrap().write_pixel(1, 1, &[0.5]);
        });
        assert_eq!(h.pixel_bytes(&cur), 3 * tile, "only the changed selection tile is copied");
        assert_eq!(h.unique_bytes(&cur), tile);
        h.trim(&cur);
        assert_counter(&h, &cur);
        let selected_value = cur.selection.as_ref().unwrap().sample_channel(1, 1, 0);
        edit(&mut h, &mut cur, "Deselect", |doc| doc.selection = None);
        assert_eq!(h.pixel_bytes(&cur), 3 * tile, "deselecting leaves the previous masks in history");
        assert_eq!(h.unique_bytes(&cur), 3 * tile);
        assert_counter(&h, &cur);
        cur = h.undo(cur).unwrap().0;
        assert_eq!(cur.selection.as_ref().unwrap().sample_channel(1, 1, 0), selected_value);
        assert_eq!(h.pixel_bytes(&cur), 3 * tile);
        assert_eq!(h.unique_bytes(&cur), tile);
        assert_counter(&h, &cur);
        // Coalescing can change the current mask without recording another state.
        Arc::make_mut(&mut cur).selection.as_mut().unwrap().write_pixel(257, 1, &[0.25]);
        assert_eq!(h.pixel_bytes(&cur), 4 * tile);
        assert_eq!(h.trim(&cur), 0);
        assert_counter(&h, &cur);
        cur = h.redo(cur).unwrap().0;
        assert!(cur.selection.is_none());
        assert_eq!(h.pixel_bytes(&cur), 4 * tile);
        assert_counter(&h, &cur);
    }
}

#[test]
fn selection_byte_budget_drops_old_masks_and_keeps_the_last_undo() {
    use photocraft_doc::{PixelFormat, Surface};
    for depth in SampleType::ALL {
        let mut doc = Document::new("selection", Size::new(64, 64), ColorMode::Rgb, depth);
        doc.selection = Some(Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false)));
        let mut cur = Arc::new(doc);
        let tile = 256 * 256 * depth.bytes();
        let mut h = History::default();
        for i in 1..=6 {
            edit(&mut h, &mut cur, "Selection", |doc| {
                doc.selection.as_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0]);
            });
        }
        assert_eq!(h.pixel_bytes(&cur), 6 * tile);
        h.max_bytes = 3 * tile;
        assert_eq!(h.trim(&cur), 4);
        assert_eq!(h.past_len(), 2);
        assert_eq!(h.pixel_bytes(&cur), 3 * tile);
        assert_eq!(h.unique_bytes(&cur), 2 * tile);
        assert_counter(&h, &cur);
        h.max_bytes = 1;
        assert_eq!(h.trim(&cur), 1);
        assert_eq!(h.past_len(), 1, "the most recent undo survives even below one mask's size");
        assert_eq!(h.pixel_bytes(&cur), 2 * tile);
        cur = h.undo(cur).unwrap().0;
        assert_counter(&h, &cur);
    }
}

fn assert_counter(h: &History, current: &Document) {
    let counter = h.tiles.as_ref().expect("budget initializes the counter");
    let total = tile_bytes(current, &mut HashSet::new()).saturating_add(h.unique_bytes(current));
    assert_eq!(counter.bytes_with(current), total as u128);
    assert_eq!(h.pixel_bytes(current), total);
    let mut seen = HashSet::new();
    let retained: usize = h.held_states().map(|s| tile_bytes(&s.doc, &mut seen)).sum();
    let empty = Document::new("empty", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    assert_eq!(counter.bytes_with(&empty), retained as u128);
}

fn video_document(depth: SampleType, frames: usize) -> Document {
    let mut doc = Document::new("video", Size::new(64, 64), ColorMode::Rgb, depth);
    let mut layer = Layer::raster("Video", doc.pixel_format());
    let footage: Vec<_> = (0..frames)
        .map(|i| {
            let mut surface = photocraft_doc::Surface::new(doc.pixel_format());
            surface.write_pixel(1, 1, &[i as f32 / frames as f32, 0.0, 0.0, 1.0]);
            surface
        })
        .collect();
    layer.content = photocraft_doc::LayerContent::Raster(footage[0].clone());
    layer.video = Some(photocraft_doc::VideoData::new(footage, 24.0));
    doc.layers.push(layer);
    doc
}

#[test]
fn video_memory_counts_originals_and_deduplicates_displayed_frames() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = video_document(depth, 3);
        let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1.bytes().len();
        let video = doc.layers[0].video.as_mut().unwrap();
        video.original_frames = Some(video.frames.clone());
        // Original footage and the displayed frame alias the stack, except this painted frame.
        video.frames[2].write_pixel(1, 1, &[0.0, 1.0, 0.0, 1.0]);
        let mut cur = Arc::new(doc);
        let mut h = History { max_bytes: usize::MAX, ..History::default() };
        assert_eq!(h.pixel_bytes(&cur), 4 * tile);
        for name in ["A", "B"] {
            edit(&mut h, &mut cur, "Rename", |doc| doc.name = name.into());
            h.trim(&cur);
        }
        assert_counter(&h, &cur);
        assert_eq!(h.pixel_bytes(&cur), 4 * tile);
        assert_eq!(h.unique_bytes(&cur), 0);
        // Rasterizing keeps the displayed frame; the rest of the clip survives only in undo.
        edit(&mut h, &mut cur, "Rasterize", |doc| doc.layers[0].video = None);
        assert_eq!(h.unique_bytes(&cur), 3 * tile);
        assert_eq!(h.pixel_bytes(&cur), 4 * tile);
        assert_counter(&h, &cur);
        cur = h.undo(cur).unwrap().0;
        assert_eq!(h.unique_bytes(&cur), 0);
        assert_counter(&h, &cur);
        cur = h.redo(cur).unwrap().0;
        assert_eq!(h.unique_bytes(&cur), 3 * tile);
        assert_counter(&h, &cur);
        h.clear();
        assert_eq!(h.pixel_bytes(&cur), tile);
    }
}

#[test]
fn video_memory_budget_evicts_deleted_off_playhead_frames() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut cur = Arc::new(video_document(depth, 6));
        let tile = cur.layers[0].surface().unwrap().tiles().next().unwrap().1.bytes().len();
        let mut h = History::default();
        for _ in 0..4 {
            edit(&mut h, &mut cur, "Delete Frame", |doc| {
                doc.layers[0].video.as_mut().unwrap().frames.remove(1);
            });
        }
        assert_eq!(h.pixel_bytes(&cur), 6 * tile);
        assert_eq!(h.unique_bytes(&cur), 4 * tile);
        h.max_bytes = 3 * tile;
        assert_eq!(h.trim(&cur), 3);
        assert_eq!(h.past_len(), 1);
        assert_eq!(h.pixel_bytes(&cur), 3 * tile);
        assert_eq!(h.unique_bytes(&cur), tile);
        assert_counter(&h, &cur);
        cur = h.undo(cur).unwrap().0;
        assert_eq!(cur.layers[0].video.as_ref().unwrap().frames.len(), 3);
        assert_counter(&h, &cur);
        cur = h.redo(cur).unwrap().0;
        assert_eq!(cur.layers[0].video.as_ref().unwrap().frames.len(), 2);
        assert_counter(&h, &cur);
        // A budget below the remaining clip still keeps the latest step undoable.
        h.max_bytes = 1;
        assert_eq!(h.trim(&cur), 0);
        assert!(h.can_undo());
    }
}

/// A pattern fill or layer style copies its pattern into the document, so the pattern's tiles
/// count like layer pixels, shared ones once; undoing that step leaves them to the redo states.
#[test]
fn document_patterns_count_as_document_pixels() {
    use photocraft_doc::{Pattern, Surface};
    for depth in SampleType::ALL {
        let tile = 4 * 256 * 256 * depth.bytes();
        let mut cur = Arc::new(Document::new("patterns", Size::new(64, 64), ColorMode::Rgb, depth));
        let mut h = History { max_bytes: usize::MAX, ..History::default() };
        for name in ["A", "B"] {
            edit(&mut h, &mut cur, "Rename", |doc| doc.name = name.into());
            h.trim(&cur);
        }
        let mut surface = Surface::new(cur.pixel_format());
        surface.write_pixel(1, 1, &[0.0, 0.0, 1.0, 1.0]);
        let pattern = Pattern::new("Pattern", surface, 4, 4);
        edit(&mut h, &mut cur, "Pattern Fill", |doc| doc.patterns.push(pattern));
        assert_eq!(h.pixel_bytes(&cur), tile, "the document's pattern");
        assert_eq!(h.unique_bytes(&cur), 0);
        assert_counter(&h, &cur);
        edit(&mut h, &mut cur, "Rename", |doc| doc.name = "C".into());
        assert_eq!(h.pixel_bytes(&cur), tile, "shared with history, it counts once");
        assert_counter(&h, &cur);
        for _ in 0..2 {
            cur = h.undo(cur).unwrap().0;
            assert_counter(&h, &cur);
        }
        assert!(cur.patterns.is_empty());
        assert_eq!(h.unique_bytes(&cur), tile, "only the redo states hold the pattern");
        assert_eq!(h.pixel_bytes(&cur), tile);
        cur = h.redo(cur).unwrap().0;
        assert_eq!(h.unique_bytes(&cur), 0);
        assert_counter(&h, &cur);
        h.clear();
        assert_eq!(h.pixel_bytes(&cur), tile);
    }
}

#[test]
fn counter_tracks_eviction_undo_redo_branching_purge_and_clones() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut cur = Arc::new(Document::with_background("h", Size::new(300, 64), ColorMode::Rgb, depth, Color::WHITE));
        Arc::make_mut(&mut cur).selection = Some(photocraft_doc::Surface::new(photocraft_doc::PixelFormat::GRAY8));
        let mut h = History::new(4);
        h.max_bytes = usize::MAX;
        for i in 0..8 {
            edit(&mut h, &mut cur, "Paint", |doc| {
                doc.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0, 0.0, 0.0, 1.0]);
                doc.selection.as_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0]);
            });
            h.trim(&cur);
            if i >= 1 {
                assert_counter(&h, &cur);
            }
        }
        cur = h.undo(cur).unwrap().0;
        assert_counter(&h, &cur);
        cur = h.undo(cur).unwrap().0;
        assert_counter(&h, &cur);
        cur = h.redo(cur).unwrap().0;
        assert_counter(&h, &cur);
        let copy = h.clone();
        edit(&mut h, &mut cur, "Branch", |doc| doc.name = "branch".into());
        assert!(!h.can_redo());
        assert_counter(&h, &cur);
        assert_counter(&copy, &cur);
        cur = h.undo(cur).unwrap().0;
        h.clear_redo();
        assert_counter(&h, &cur);
        h.purge_last();
        assert_counter(&h, &cur);
        h.clear();
        assert!(h.tiles.is_none());
        assert_eq!(h.pixel_bytes(&cur), tile_bytes(&cur, &mut HashSet::new()));
    }
}

#[test]
fn shared_masks_channels_and_repeated_snapshot_references_count_once() {
    use photocraft_doc::{AlphaChannel, LayerMask, PixelFormat, Surface};
    let mut doc = base();
    let mut gray = Surface::new(PixelFormat::GRAY8);
    gray.write_pixel(1, 1, &[1.0]);
    doc.layers[0].mask = Some(LayerMask { surface: gray.clone(), ..LayerMask::reveal_all() });
    doc.channels.push(AlphaChannel::new("alpha", gray.clone()));
    doc.selection = Some(gray.clone());
    doc.quick_mask = Some(AlphaChannel::new("quick", gray));
    doc.layers.push(doc.layers[0].clone());
    let mut cur = Arc::new(doc);
    let bytes = tile_bytes(&cur, &mut HashSet::new());
    let mut h = History::new(50);
    // Counting current separately would double count its tiles; the exact union fits.
    h.max_bytes = bytes;
    for _ in 0..6 {
        h.record("Same", cur.clone(), LayerTarget::default());
        assert_eq!(h.trim(&cur), 0);
    }
    assert_counter(&h, &cur);
    for _ in 0..3 {
        cur = h.undo(cur).unwrap().0;
        assert_counter(&h, &cur);
    }
    h.clear_redo();
    h.purge_last();
    assert_counter(&h, &cur);
    assert_eq!(h.pixel_bytes(&cur), bytes);
}

#[test]
fn threshold_rechecks_coalesced_current_and_changed_budget() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    h.max_bytes = usize::MAX;
    for i in 0..6 {
        edit(&mut h, &mut cur, "Paint", |doc| {
            doc.layers[0].surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 10.0, 0.0, 0.0, 1.0]);
        });
        h.trim(&cur);
    }
    // A coalesced edit may change current without recording another history state.
    Arc::make_mut(&mut cur).layers[0].surface_mut().unwrap().write_pixel(300, 1, &[1.0; 4]);
    let tile = 256 * 256 * 4;
    h.max_bytes = 4 * tile;
    assert_eq!(h.trim(&cur), 4);
    assert_eq!(h.past_len(), 2);
    assert_counter(&h, &cur);
    cur = h.undo(cur).unwrap().0;
    assert_counter(&h, &cur);
    h.max_bytes = 0;
    edit(&mut h, &mut cur, "Unlimited", |doc| doc.name = "u".into());
    assert!(h.tiles.is_none());
    h.max_bytes = 1;
    h.trim(&cur);
    assert_eq!(h.past_len(), 1, "always retain the most recent undo");
    assert_counter(&h, &cur);
}

#[test]
fn transferred_tiles_cannot_make_the_budget_check_underestimate_current() {
    use photocraft_doc::{PixelFormat, Surface};
    let before = Arc::new(base());
    let mut current = (*before).clone();
    let mut large = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true));
    large.write_pixel(256, 0, &[1.0; 5]);
    current.layers[0].surface_mut().unwrap().put_tiles(large.take_tiles(photocraft_doc::Rect::from_xywh(256, 0, 1, 1)));
    let mut h = History::new(50);
    h.max_bytes = tile_bytes(&current, &mut HashSet::new()) - 1;
    h.record("A", before.clone(), LayerTarget::default());
    h.record("B", before, LayerTarget::default());
    assert_eq!(h.trim(&current), 1, "current alone exceeds the budget; only the mandatory undo survives");
    assert_counter(&h, &current);
}

#[test]
fn clear_states_keeps_the_document_and_undo_brings_the_list_back() {
    let mut h = History::new(10);
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    edit(&mut h, &mut cur, "C", |d| d.name = "c".into());
    let (d, _) = h.undo(cur.clone()).unwrap();
    cur = d;
    let entries = h.entries();
    assert_eq!(entries, ["Open", "A", "B"]);
    assert!(h.clear_states());
    assert_eq!(h.entries(), ["B"], "only the current state is left");
    assert!(!h.can_redo() && h.can_undo() && h.is_cleared());
    assert_eq!(h.undo_label(), Some(CLEAR_HISTORY_LABEL));
    assert!(!h.clear_states(), "nothing left to clear");
    assert!(h.is_cleared(), "and the cleared states are still held");
    let (d, _) = h.undo(cur.clone()).unwrap();
    assert!(Arc::ptr_eq(&d, &cur), "undoing Clear History doesn't change the document");
    assert_eq!(h.entries(), entries);
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["C"], "the redo state is back too");
    assert!(!h.is_cleared());
}

#[test]
fn cleared_states_still_count_in_the_memory_figures_and_the_budget() {
    let mut h = History::new(50);
    let mut cur = Arc::new(base());
    let tile = 256 * 256 * 4;
    for i in 0..5 {
        edit(&mut h, &mut cur, &format!("paint {i}"), |d| {
            let id = d.layers[0].id;
            d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[i as f32 / 8.0, 0.0, 0.0, 1.0]);
        });
    }
    cur = h.undo(cur).unwrap().0;
    let (unique, pixels) = (h.unique_bytes(&cur), h.pixel_bytes(&cur));
    assert_eq!(unique, 5 * tile, "four undo states and one redo state");
    assert!(h.clear_states());
    assert_eq!(h.unique_bytes(&cur), unique, "Clear History frees nothing while it can be undone");
    assert_eq!(h.pixel_bytes(&cur), pixels);
    assert_eq!(h.extra_bytes(&cur, &[&base()]), tile, "a snapshot's own tile, not the cleared states'");
    assert_eq!(h.unique_bytes_besides(&cur, &[]), unique);
    // The budget counts them too: the oldest cleared states go first.
    h.max_bytes = 4 * tile;
    assert_eq!(h.trim(&cur), 2);
    assert_counter(&h, &cur);
    assert_eq!(h.pixel_bytes(&cur), 4 * tile);
    assert!(h.is_cleared() && h.past_len() == 0, "the list stays cleared");
    h.undo(cur.clone()).unwrap();
    assert_eq!(h.entries(), ["paint 1", "paint 2", "paint 3"], "undo brings back what the budget kept");
    assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["paint 4"]);
    // A new step after Clear History forgets the cleared states, and the counter with them.
    assert!(h.clear_states());
    h.trim(&cur);
    edit(&mut h, &mut cur, "paint 5", |d| {
        let id = d.layers[0].id;
        d.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[1.0, 0.0, 0.0, 1.0]);
    });
    assert_eq!(h.unique_bytes(&cur), tile, "only the state before the new step");
    assert_eq!(h.pixel_bytes(&cur), 2 * tile);
}

#[test]
fn a_new_step_or_purge_forgets_the_cleared_states() {
    let mut h = History::new(10);
    let mut cur = Arc::new(base());
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    assert!(h.clear_states());
    edit(&mut h, &mut cur, "B", |d| d.name = "b".into());
    assert_eq!(h.entries(), ["A", "B"]);
    assert!(!h.is_cleared());
    assert!(h.undo(cur.clone()).is_some());
    assert!(!h.can_undo(), "the cleared states are gone");
    let mut h = History::new(10);
    edit(&mut h, &mut cur, "A", |d| d.name = "a".into());
    assert!(h.clear_states());
    assert!(h.purge_last());
    assert!(!h.can_undo() && !h.is_cleared());
    assert!(!h.clear_states(), "a single state can't be cleared");
}
