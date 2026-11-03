//! The History panel menu (☰, #2966): Step Forward, Step Backward and Clear History. Delete and
//! New Document are [`crate::history_cmds`]; New Snapshot and History Options live elsewhere.
//!
//! Step Forward / Step Backward move one state along the History list, like Edit › Redo / Undo
//! (which step through the list too), but report a step that isn't there as an error. Clear
//! History takes every state but the current one off the list without changing the image; it is
//! undoable: Edit › Undo puts the states back until the next step is recorded.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// `{states, current, label}`: the list length, the current state's index (0 = oldest) and its
/// name, after a History panel menu command.
fn list(s: &Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let entries = st.history.entries();
    Ok(json!({ "states": entries.len() + st.history.redo_labels().count(), "current": st.history.past_len(), "label": entries.last() }))
}

fn step(s: &mut Session, forward: bool) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let possible = if forward { st.history.can_redo() } else { st.history.can_undo() };
    if !possible {
        return Err(EngineError::Other(if forward { "no later state to step forward to" } else { "no earlier state to step back to" }.into()));
    }
    let moved = if forward { s.redo() } else { s.undo() };
    if !moved {
        return Err(EngineError::Other("the history can't step right now".into()));
    }
    list(s)
}

/// History panel › Clear History: only the current state stays in the list; the image doesn't
/// change. Edit › Undo brings the states back.
fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't change while a job runs".into()));
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.coalesce = None;
    if !st.history.clear_states() {
        return Err(EngineError::Other("the current state is the only state".into()));
    }
    // Edit › Fade blends against the previous state, which is no longer in the list.
    s.edit_state.fade = None;
    list(s)
}

fn can_step_forward(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.can_redo() { Ok(()) } else { Err("no later state".into()) }
}

fn can_step_backward(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.can_undo() { Ok(()) } else { Err("no earlier state".into()) }
}

fn can_clear(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.past_len() > 0 || st.history.can_redo() { Ok(()) } else { Err("the current state is the only state".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "history.stepForward",
            label: "Step Forward",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: makes the next History state current (Err when there is none)"##,
            enabled: can_step_forward,
            run: |s, _| step(s, true),
            journal: true,
        },
        CommandSpec {
            id: "history.stepBackward",
            label: "Step Backward",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: makes the previous History state current (Err when there is none)"##,
            enabled: can_step_backward,
            run: |s, _| step(s, false),
            journal: true,
        },
        CommandSpec {
            id: "history.clear",
            label: "Clear History",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: removes every History state but the current one without changing the image; Edit › Undo restores them"##,
            enabled: can_clear,
            run: clear,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A document with three steps after Open: entries Open, New Layer, Fill, Fill.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s
    }

    fn entries(s: &Session) -> Vec<String> {
        s.active().unwrap().history.entries()
    }

    #[test]
    fn step_backward_and_forward_move_one_state() {
        let mut s = session();
        let all = entries(&s);
        let last = s.active().unwrap().doc.clone();
        assert!(!s.is_enabled("history.stepForward"), "no later state yet");
        assert!(s.is_enabled("history.stepBackward"));
        let r = s.execute("history.stepBackward", json!({})).unwrap();
        assert_eq!((r["states"].as_u64(), r["current"].as_u64()), (Some(all.len() as u64), Some(all.len() as u64 - 2)));
        assert_eq!(r["label"], json!(all[all.len() - 2]));
        assert_eq!(entries(&s), all[..all.len() - 1].to_vec());
        assert!(s.is_enabled("history.stepForward"));
        s.execute("history.stepBackward", json!({})).unwrap();
        s.execute("history.stepForward", json!({})).unwrap();
        let r = s.execute("history.stepForward", json!({})).unwrap();
        assert_eq!(r["current"].as_u64(), Some(all.len() as u64 - 1));
        assert_eq!(entries(&s), all);
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &last), "back at the last state");
        assert!(s.execute("history.stepForward", json!({})).is_err(), "nothing to step forward to");
    }

    #[test]
    fn step_backward_stops_at_the_oldest_state() {
        let mut s = session();
        while s.is_enabled("history.stepBackward") {
            s.execute("history.stepBackward", json!({})).unwrap();
        }
        assert_eq!(entries(&s), ["Open"]);
        assert!(s.execute("history.stepBackward", json!({})).is_err());
        assert_eq!(entries(&s), ["Open"], "a refused step changes nothing");
    }

    #[test]
    fn clear_history_keeps_the_image_and_leaves_one_state() {
        let mut s = session();
        s.execute("history.stepBackward", json!({})).unwrap();
        let all = entries(&s);
        let doc = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let r = s.execute("history.clear", json!({})).unwrap();
        assert_eq!((r["states"].as_u64(), r["current"].as_u64()), (Some(1), Some(0)));
        let st = s.active().unwrap();
        assert_eq!(st.history.entries(), [all.last().unwrap().clone()], "the current state is the only one");
        assert!(!st.history.can_redo(), "redo states are cleared too");
        assert!(Arc::ptr_eq(&st.doc, &doc) && st.revision == revision, "the image doesn't change");
        assert!(!s.is_enabled("history.clear"), "nothing left to clear");
        assert!(s.execute("history.clear", json!({})).is_err());
        assert!(!s.is_enabled("history.stepForward") && !s.is_enabled("history.deleteState"));
        assert!(!s.is_enabled("edit.fade"));
    }

    #[test]
    fn undo_after_clear_history_brings_the_states_back() {
        let mut s = session();
        s.execute("history.stepBackward", json!({})).unwrap();
        let all = entries(&s);
        let redo: Vec<String> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
        let doc = s.active().unwrap().doc.clone();
        s.execute("history.clear", json!({})).unwrap();
        assert!(s.is_enabled("edit.undo") && s.is_enabled("history.stepBackward"), "Clear History is undoable");
        s.execute("edit.undo", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.history.entries(), all);
        assert_eq!(st.history.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);
        assert!(Arc::ptr_eq(&st.doc, &doc), "undoing it doesn't change the image either");
        // A step after Clear History forgets the cleared states.
        s.execute("history.clear", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        assert_eq!(entries(&s).len(), 2);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(!s.is_enabled("edit.undo"), "the cleared states are gone");
    }

    #[test]
    fn clear_history_and_its_undo_keep_a_saved_document_clean() {
        let mut s = session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        s.execute("history.clear", json!({})).unwrap();
        assert!(!s.active().unwrap().is_dirty(), "Clear History is no edit");
        let revision = s.active().unwrap().revision;
        assert!(s.execute("edit.undo", json!({})).unwrap().as_bool().unwrap());
        let st = s.active().unwrap();
        assert!(!st.is_dirty(), "nor is undoing it");
        assert!(st.revision > revision, "the panels still see the list change");
        // A modified document stays modified.
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("history.clear", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().is_dirty());
    }

    #[test]
    fn clear_history_leaves_other_documents_alone() {
        let mut s = session();
        let first = entries(&s);
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        assert_eq!(s.active_index(), Some(1));
        s.execute("history.clear", json!({})).unwrap();
        assert_eq!(entries(&s).len(), 1);
        let other = &s.documents()[0];
        assert_eq!(other.history.entries(), first, "the other document keeps its states");
        assert!(!other.history.is_cleared() && other.history.can_undo());
        assert!(s.set_active(0));
        assert!(s.is_enabled("history.clear") && s.is_enabled("history.stepBackward"));
        s.execute("history.stepBackward", json!({})).unwrap();
        assert_eq!(entries(&s).len(), first.len() - 1);
    }

    #[test]
    fn history_menu_commands_refuse_without_a_document_and_ignore_odd_params() {
        let mut s = Session::new();
        for id in ["history.stepForward", "history.stepBackward", "history.clear"] {
            assert!(!s.is_enabled(id), "{id}");
            assert!(s.execute(id, json!({})).is_err(), "{id} without a document");
        }
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        for id in ["history.stepForward", "history.stepBackward", "history.clear"] {
            assert!(!s.is_enabled(id), "{id}: only the opening state");
            for p in [json!({}), json!(null), json!([1]), json!({"state": -1}), json!("x")] {
                assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
            }
            assert_eq!(entries(&s), ["Open"]);
        }
    }
}
