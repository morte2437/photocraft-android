//! The History Brush's source (#2966): the History panel's source column, at the left of every
//! state and snapshot, sets what the History Brush (and Fill › History) restores from. Exactly one
//! source per document; by default the oldest state held (Photoshop's default is the first
//! snapshot, the document as opened, which that state is until it falls off the history).
//!
//! A state source is kept by state, not by index ([`photocraft_ops::History::set_brush_source`]),
//! so it stays on its state while older states fall off, and that state is kept when it would fall
//! off the history itself. A snapshot source is kept by snapshot id ([`HistorySource::Snapshot`]),
//! a separate namespace from the states. Once the source state or snapshot is deleted, or a redo
//! state source is discarded by a new step, there is no source until one is set, and the History
//! Brush refuses to paint without an explicit `state` or `snapshot`, as Photoshop does.
//! (Photoshop keeps a source discarded by a new step in the list; that needs states outside the
//! linear history, so here it is gone.) Edit › Purge › Histories keeps the snapshots (and a
//! snapshot source) and resets a state source to the default.
//!
//! Rows: `state` is a History panel row everywhere here (this command, `paint.historyBrush`, Fill
//! › History and `historySource` in `doc_inspect`): the states (0 = oldest), the current one, then
//! the greyed redo states, the next redo first.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

const CMD: &str = "history.setSource";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// What the History Brush paints from: a History panel state row, or a snapshot by id. The two
/// are separate, so snapshot 1 and state row 1 never stand for each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySource {
    /// A History panel row ([`photocraft_ops::History::row_id`]).
    State(usize),
    /// A History panel snapshot ([`crate::history_cmds::Snapshot::id`]).
    Snapshot(u64),
}

/// The document's History Brush source, `None` once the source state or snapshot is gone.
pub fn source(st: &DocState) -> Option<HistorySource> {
    match st.source_snapshot {
        Some(id) => st.snapshots.iter().any(|s| s.id == id).then_some(HistorySource::Snapshot(id)),
        None => st.history.brush_source().map(HistorySource::State),
    }
}

fn source_of(st: &DocState, src: HistorySource) -> Option<Arc<Document>> {
    match src {
        HistorySource::State(i) => st.history.row_doc(i, &st.doc),
        HistorySource::Snapshot(id) => st.snapshots.iter().find(|s| s.id == id).map(|s| s.doc.clone()),
    }
}

/// A `state` or `snapshot` param (`None` when neither is passed), checked against the document.
fn parse(st: &DocState, p: &Value, cmd: &str) -> Result<Option<HistorySource>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let get = |k: &str| p.get(k).filter(|v| !v.is_null());
    let index = |k: &str, v: &Value| v.as_u64().ok_or_else(|| bad(format!("`{k}` must be a non-negative integer, not {v}")));
    match (get("state"), get("snapshot")) {
        (Some(_), Some(_)) => Err(bad("pass a `state` or a `snapshot`, not both".into())),
        (Some(v), None) => {
            let rows = st.history.entries().len().saturating_add(st.history.redo_labels().count());
            let i = usize::try_from(index("state", v)?).ok().filter(|i| *i < rows);
            let i = i.ok_or_else(|| bad(format!("no history state {v}: a History panel row (0..{rows}, the greyed redo rows last)")))?;
            Ok(Some(HistorySource::State(i)))
        }
        (None, Some(v)) => {
            let id = index("snapshot", v)?;
            if !st.snapshots.iter().any(|s| s.id == id) {
                return Err(bad(format!("no snapshot {id}")));
            }
            Ok(Some(HistorySource::Snapshot(id)))
        }
        (None, None) => Ok(None),
    }
}

/// The document the History Brush and Fill › History paint from: the `state` (History panel row)
/// or `snapshot` passed, else the source set in the History panel. A source of another canvas
/// size is refused rather than painting shifted pixels (after Crop, Canvas Size or Image Size):
/// Photoshop refuses too, "the current canvas size does not match that of the history state"
/// (its message as users report it; Adobe's help doesn't document the case).
pub(crate) fn paint_source(st: &DocState, p: &Value, cmd: &str) -> Result<Arc<Document>> {
    let doc = match parse(st, p, cmd)? {
        Some(src) => source_of(st, src).ok_or_else(|| EngineError::Other("that history state or snapshot is gone".into()))?,
        None => source(st)
            .and_then(|src| source_of(st, src))
            .ok_or_else(|| EngineError::Other("the History Brush source was deleted: set a new source in the History panel".into()))?,
    };
    if doc.size != st.doc.size {
        return Err(EngineError::Other(format!(
            "the current canvas size ({}×{}) does not match that of the History Brush source ({}×{}): set a source of this size in the History panel",
            st.doc.size.width, st.doc.size.height, doc.size.width, doc.size.height
        )));
    }
    Ok(doc)
}

/// `history.setSource {state}` or `{snapshot}`: make a History panel row or a snapshot the History
/// Brush's source.
fn set_source(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    match parse(st, p, CMD)? {
        None => return Err(bad("pass the `state` (a History panel row, 0 = oldest) or the `snapshot` id to paint from")),
        Some(HistorySource::State(i)) => {
            if !st.history.set_brush_source(i) {
                return Err(bad(format!("no history state {i}")));
            }
            st.source_snapshot = None;
        }
        Some(HistorySource::Snapshot(id)) => {
            // No state is the source now, so none is kept past its time.
            st.history.reset_brush_source();
            st.source_snapshot = Some(id);
        }
    }
    Ok(source_json(st))
}

/// The source as `doc_inspect` reports it: `{state,label}` (a History panel row) or
/// `{snapshot,name}`; `null` once its state or snapshot is gone.
pub fn source_json(st: &DocState) -> Value {
    match source(st) {
        None => Value::Null,
        Some(HistorySource::State(i)) => {
            let label = st.history.entries().into_iter().chain(st.history.redo_labels().map(str::to_string)).nth(i);
            json!({ "state": i, "label": label })
        }
        Some(HistorySource::Snapshot(id)) => {
            json!({ "snapshot": id, "name": st.snapshots.iter().find(|s| s.id == id).map(|s| s.name.clone()) })
        }
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Set History Brush Source",
        menu: &[],
        shortcut: None,
        params: r##"{"state":row} | {"snapshot":id} (state: a History panel row, 0 = oldest state, then the current one, then the greyed redo states; snapshot: a history.newSnapshot id) → {state,label} | {snapshot,name}: paint.historyBrush and Fill › History use this source when neither `state` nor `snapshot` is passed"##,
        enabled: has_doc,
        run: set_source,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white 8×8 document filled red, then green: rows Open, Fill (red), Fill (green).
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16, "background": "white"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s
    }

    fn rgb(s: &Session) -> [f32; 3] {
        let st = s.active().unwrap();
        let px = st.doc.layers[0].surface().unwrap().rgba(4, 4);
        [px[0], px[1], px[2]]
    }

    fn brush(s: &mut Session, extra: Value) {
        let mut p = json!({"points": [[4, 4]], "size": 40, "hardness": 100});
        if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        s.execute("paint.historyBrush", p).unwrap();
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn default_source_is_the_oldest_state() {
        let mut s = session();
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "Open"}));
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
    }

    #[test]
    fn the_history_brush_and_fill_paint_from_the_set_source() {
        let mut s = session();
        let r = s.execute(CMD, json!({"state": 1})).unwrap();
        assert_eq!(r, json!({"state": 1, "label": "Fill"}));
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"]["state"], 1);
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "the red state: {:?}", rgb(&s));
        // An explicit `state` still wins.
        brush(&mut s, json!({"state": 0}));
        assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
        s.execute("edit.fill", json!({"contents": "history"})).unwrap();
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "Fill › History uses the source too: {:?}", rgb(&s));
        // Setting the source is not a step.
        assert_eq!(s.active().unwrap().history.entries().len(), 6);
    }

    /// The common Photoshop workflow: apply, step back, set the source on the greyed state, paint.
    #[test]
    fn a_redo_state_can_be_the_source_until_a_new_step_discards_it() {
        let mut s = session();
        assert!(s.undo());
        s.execute(CMD, json!({"state": 2})).unwrap();
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the greyed green state: {:?}", rgb(&s));
        // Painting was a new step, which discarded the green state: no source until one is set.
        assert!(crate::inspect::document(s.active().unwrap())["historySource"].is_null());
        let before = rgb(&s);
        assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
        assert!(s.execute("edit.fill", json!({"contents": "history"})).is_err());
        assert!(close(rgb(&s), before), "nothing painted");
        brush(&mut s, json!({"state": 0}));
        assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "an explicit state still paints: {:?}", rgb(&s));
        s.execute(CMD, json!({"state": 0})).unwrap();
        brush(&mut s, json!({}));
    }

    #[test]
    fn the_source_follows_its_state_as_older_states_fall_off() {
        let mut s = session();
        s.active_mut().unwrap().history.max_states = 2;
        s.execute(CMD, json!({"state": 1})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        // Open fell off: the red state is row 0 now and still the source.
        let st = s.active().unwrap();
        assert_eq!((st.history.entries().len(), st.history.brush_source()), (3, Some(0)));
        // It would fall off next: it is kept, as in Photoshop, and paints red.
        s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
        brush(&mut s, json!({}));
        let st = s.active().unwrap();
        assert_eq!((st.history.entries()[0].as_str(), st.history.brush_source()), ("Fill", Some(0)));
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
    }

    #[test]
    fn deleting_the_source_state_leaves_no_source() {
        let mut s = session();
        s.execute(CMD, json!({"state": 2})).unwrap();
        s.execute("history.deleteState", json!({"state": 2})).unwrap();
        assert!(crate::inspect::document(s.active().unwrap())["historySource"].is_null());
        assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
        s.execute(CMD, json!({"state": 1})).unwrap();
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
    }

    /// Edit › Purge › Histories clears the states: the source goes back to the default instead of
    /// leaving every document without one (#3536 review).
    #[test]
    fn purging_histories_resets_a_state_source() {
        let mut s = session();
        s.execute(CMD, json!({"state": 1})).unwrap();
        s.execute("edit.purge.histories", json!({})).unwrap();
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"], json!({"state": 0, "label": "Fill"}));
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the only state left: {:?}", rgb(&s));
        s.execute("edit.fill", json!({"contents": "history"})).unwrap();
    }

    /// After Canvas Size (or Crop, Image Size) the old states have another size: painting from one
    /// would shift the pixels, so the History Brush and Fill › History refuse, as Photoshop does.
    #[test]
    fn a_source_of_another_canvas_size_is_refused() {
        let mut s = session();
        s.execute("image.canvasSize", json!({"width": 12, "height": 8, "anchor": "topLeft"})).unwrap();
        for cmd in ["paint.historyBrush", "edit.fill"] {
            for state in [json!(null), json!(0), json!(2)] {
                let p =
                    if cmd == "edit.fill" { json!({"contents": "history", "state": state}) } else { json!({"points": [[4, 4]], "size": 40, "state": state}) };
                let e = s.execute(cmd, p.clone()).unwrap_err().to_string();
                assert!(e.contains("canvas size"), "{cmd} {p}: {e}");
            }
        }
        assert_eq!(s.active().unwrap().history.entries().len(), 4, "nothing painted");
        // The state after the resize is the same size: it paints.
        s.execute(CMD, json!({"state": 3})).unwrap();
        brush(&mut s, json!({}));
        s.execute("edit.fill", json!({"contents": "history"})).unwrap();
        // A snapshot of the old size is refused the same way.
        let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
        s.execute("image.canvasSize", json!({"width": 8, "height": 8, "anchor": "topLeft"})).unwrap();
        s.execute(CMD, json!({"snapshot": snap})).unwrap();
        assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
    }

    /// `state` is a History panel row for the History Brush and Fill too: the greyed redo rows
    /// count, as in `history.setSource` and `historySource`.
    #[test]
    fn the_brush_and_fill_take_redo_rows() {
        let mut s = session();
        assert!(s.undo());
        brush(&mut s, json!({"state": 2}));
        assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "the greyed green state: {:?}", rgb(&s));
        let mut s = session();
        assert!(s.undo());
        s.execute("edit.fill", json!({"contents": "history", "state": 2})).unwrap();
        assert!(close(rgb(&s), [0.0, 1.0, 0.0]), "{:?}", rgb(&s));
        assert!(s.execute("edit.fill", json!({"contents": "history", "state": 4})).is_err(), "no such row");
    }

    /// A snapshot as the source (the column on snapshot rows): kept by snapshot id, apart from the
    /// state rows; deleting it leaves no source; Purge › Histories keeps it.
    #[test]
    fn a_snapshot_can_be_the_source() {
        let mut s = session();
        assert!(s.undo());
        let snap = s.execute("history.newSnapshot", json!({"name": "Red"})).unwrap()["snapshot"].as_u64().unwrap();
        assert_eq!(snap, 1, "snapshot 1, while state row 1 is the red fill too: they must not mix");
        assert!(s.redo());
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let r = s.execute(CMD, json!({"snapshot": snap})).unwrap();
        assert_eq!(r, json!({"snapshot": 1, "name": "Red"}));
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"], r);
        // No state is the source now: the default would be state 0 (white).
        assert_eq!(s.active().unwrap().history.brush_source(), Some(0));
        brush(&mut s, json!({}));
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "the snapshot paints: {:?}", rgb(&s));
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("edit.fill", json!({"contents": "history"})).unwrap();
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
        // An explicit snapshot or state still wins.
        brush(&mut s, json!({"state": 0}));
        assert!(close(rgb(&s), [1.0, 1.0, 1.0]), "{:?}", rgb(&s));
        brush(&mut s, json!({"snapshot": snap}));
        assert!(close(rgb(&s), [1.0, 0.0, 0.0]), "{:?}", rgb(&s));
        // Purging histories keeps the snapshots and so the source.
        s.execute("edit.purge.histories", json!({})).unwrap();
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"], r);
        // A state source replaces it.
        s.execute(CMD, json!({"state": 0})).unwrap();
        assert_eq!(crate::inspect::document(s.active().unwrap())["historySource"]["state"], 0);
        s.execute(CMD, json!({"snapshot": snap})).unwrap();
        // Deleting the snapshot leaves no source.
        s.execute("history.deleteSnapshot", json!({"snapshot": snap})).unwrap();
        assert!(crate::inspect::document(s.active().unwrap())["historySource"].is_null());
        assert!(s.execute("paint.historyBrush", json!({"points": [[4, 4]], "size": 40})).is_err());
        assert!(s.execute("edit.fill", json!({"contents": "history"})).is_err());
        assert!(s.execute(CMD, json!({"snapshot": snap})).is_err(), "it is gone");
    }

    #[test]
    fn bad_params_and_no_document_are_errors() {
        assert!(matches!(Session::new().execute(CMD, json!({"state": 0})), Err(EngineError::NoDocument | EngineError::Disabled(..))));
        let mut s = session();
        for p in [
            json!({}),
            json!({"state": null}),
            json!({"state": -1}),
            json!({"state": 3}),
            json!({"state": 1e300}),
            json!({"state": 1.5}),
            json!({"state": "1"}),
            json!({"state": [1]}),
            json!({"snapshot": 0}),
            json!({"snapshot": 1}),
            json!({"snapshot": -1}),
            json!({"snapshot": "Open"}),
            json!({"snapshot": "Open", "state": 1}),
            json!({"snapshot": 1, "state": 1}),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.active().unwrap().history.brush_source(), Some(0), "nothing changed");
    }
}
