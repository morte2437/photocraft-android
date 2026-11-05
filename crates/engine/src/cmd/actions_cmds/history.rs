//! Consolidate document history after commands finish their own atomic transactions.
use super::{EngineError, Result, Session};
use photocraft_doc::{DocId, Document};
use photocraft_ops::{History, LayerTarget};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug)]
struct BeforeDocument {
    id: DocId,
    doc: Arc<Document>,
    history: History,
    layers: LayerTarget,
    coalesce: Option<String>,
    saved_revision: u64,
    clean: bool,
}

impl BeforeDocument {
    /// Capture the complete document boundary, including clean state and targeted layers.
    fn capture(st: &crate::DocState) -> Self {
        Self {
            id: st.doc.id,
            doc: st.doc.clone(),
            history: st.history.clone(),
            layers: st.layer_target(),
            coalesce: st.coalesce.clone(),
            saved_revision: st.saved_revision,
            clean: !st.is_dirty(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ActionHistory {
    pub name: String,
    external: bool,
    before: Vec<BeforeDocument>,
}

/// The outer play owns one lazy scope; nested plays share its document snapshots.
pub(super) fn begin(s: &mut Session, name: &str) {
    if s.action_history.is_none() && s.prefs().actions.create_single_history_state {
        s.action_history = Some(ActionHistory { name: name.into(), external: false, before: Vec::new() });
    }
}

/// Capture each document lazily, before its first ordinary or compound edit. Commands retain
/// their own records, purge and rollback semantics until the outer action finishes.
pub(crate) fn before_edit(s: &mut Session) {
    let Some(group) = &s.action_history else { return };
    let Some(st) = s.active() else { return };
    let previous = group.before.iter().position(|before| before.id == st.doc.id);
    if previous.is_some_and(|i| !st.history.is_cleared() && !st.history.was_purged_since(&group.before[i].history)) {
        return;
    }
    let before = BeforeDocument::capture(st);
    if let Some(group) = &mut s.action_history {
        match previous {
            Some(i) => group.before[i] = before,
            None => group.before.push(before),
        }
    }
    if let Some(st) = s.active_mut() {
        st.coalesce = None;
        st.history.begin_visibility_capture(&[]);
    }
}

/// Rows visible just before the user deletes history, distinct from internal compound purges.
pub(crate) fn history_rows(s: &Session) -> Vec<u64> {
    if s.action_history.is_none() {
        return Vec::new();
    }
    let Some(st) = s.active() else { return Vec::new() };
    st.history.row_ids().collect()
}

/// Apply only the user's actual row removals to the existing action boundary immediately.
pub(crate) fn reconcile_history_removal(s: &mut Session, rows: &[u64]) {
    let Some(st) = s.active() else { return };
    let Some(group) = &s.action_history else { return };
    let Some(index) = group.before.iter().position(|before| before.id == st.doc.id) else { return };
    let prior = &group.before[index].history;
    let after: HashSet<_> = st.history.row_ids().collect();
    let baseline: HashSet<_> = prior.row_ids().collect();
    let removed: Vec<_> = rows.iter().copied().filter(|id| !after.contains(id) && baseline.contains(id)).collect();
    if removed.is_empty() {
        return;
    }
    if prior.row_id(prior.past_len()).is_some_and(|id| removed.contains(&id)) {
        // The original pixels themselves were deleted. Only the post-delete document may
        // start a later grouped suffix; its accumulator excludes edits before that boundary.
        let mut before = BeforeDocument::capture(st);
        before.history.end_visibility_capture();
        if let Some(group) = &mut s.action_history {
            group.before[index] = before;
        }
        if let Some(st) = s.active_mut() {
            st.coalesce = None;
            st.history.begin_visibility_capture(&[]);
        }
    } else if let Some(group) = &mut s.action_history {
        let history = &mut group.before[index].history;
        for id in removed {
            let Some(row) = history.row_of(id) else { continue };
            let current = history.past_len();
            if row < current {
                history.remove(row);
            } else if row > current {
                history.remove_later(row - current - 1);
            }
        }
    }
}

/// Fold ordinary edits while retaining real Clear History and irreversible purge boundaries.
fn consolidate(s: &mut Session, group: &ActionHistory) {
    for before in &group.before {
        let Some(st) = s.docs.iter_mut().find(|st| st.doc.id == before.id) else { continue };
        if st.history.was_purged_since(&before.history) {
            st.history.end_visibility_capture();
            continue;
        }
        let mut history = before.history.clone();
        // Discarded command-local rows may have been visible while a dialog was open. Their
        // identifiers must not be allocated to a different state after consolidation.
        history.reserve_ids_from(&st.history);
        history.max_states = st.history.max_states;
        history.max_bytes = st.history.max_bytes;
        history.non_linear = st.history.non_linear;
        if st.doc.as_ref() == before.doc.as_ref() {
            st.doc = before.doc.clone();
            st.coalesce = before.coalesce.clone();
            if before.clean && st.saved_revision == before.saved_revision {
                st.saved_revision = st.revision;
            }
        } else {
            let visibility = st.history.captured_visibility().unwrap_or(st.history.current_visibility()).to_vec();
            history.set_current_layers(before.layers.clone());
            history.record(format!("Action: {}", group.name), before.doc.clone(), st.layer_target());
            history.add_current_visibility(&visibility);
            st.coalesce = None;
        }
        history.preserve_action_clear(&st.history);
        if let Some(source) = history.preserve_action_brush_source(&st.history) {
            let (source, restore_on_clear_undo) = match source {
                photocraft_ops::ActionBrushSource::Current(source) => (source, false),
                photocraft_ops::ActionBrushSource::Cleared(source) => (source, true),
            };
            // The selected row will no longer exist, but the History Brush must still paint its
            // exact pixels. Snapshots already retain source pixels without adding an Undo row.
            let id = st.next_snapshot;
            st.next_snapshot = id.saturating_add(1);
            st.snapshots.push(crate::history_cmds::Snapshot { id, name: format!("{}: {}", group.name, source.label), doc: source.doc, layers: source.layers });
            if restore_on_clear_undo {
                history.set_cleared_brush_snapshot(id);
            } else {
                history.set_action_brush_snapshot(id);
            }
        }
        st.history = history;
        st.trim_history();
    }
}

/// Publish all edited documents once the outer play, including its failure prefix, ends.
pub(super) fn finish(s: &mut Session) {
    if let Some(group) = s.action_history.take() {
        consolidate(s, &group);
    }
}

/// Batch owns its scope across nested plays without adding an artificial playback depth.
pub(crate) fn begin_document_history(s: &mut Session, name: &str) {
    begin(s, name);
    if let Some(group) = &mut s.action_history {
        group.external = true;
    }
}

/// Finalize the per-document scope explicitly owned by Batch, including a completed prefix on failure.
pub(crate) fn finish_document_history(s: &mut Session) {
    finish(s);
}

/// Finish an ordinary outer playback without closing a scope owned by the surrounding Batch.
pub(super) fn finish_playback(s: &mut Session) {
    if s.action_history.as_ref().is_none_or(|group| !group.external) {
        finish(s);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct HistoryBinding {
    past: usize,
    rows: Vec<(Option<u64>, String)>,
    source: Option<usize>,
    source_snapshot: Option<u64>,
    cleared: bool,
}

impl HistoryBinding {
    /// Rows and source identity detect history changes that leave document pixels untouched.
    fn of(history: &History) -> Self {
        let labels = history.entries().into_iter().chain(history.redo_labels().map(str::to_string));
        Self {
            past: history.past_len(),
            rows: labels.enumerate().map(|(i, label)| (history.row_id(i), label)).collect(),
            source: history.brush_source(),
            source_snapshot: history.brush_source_snapshot(),
            cleared: history.is_cleared(),
        }
    }
}

#[derive(Clone, Debug)]
struct PausedDocument {
    id: DocId,
    doc: Arc<Document>,
    history: HistoryBinding,
    source_snapshot: Option<u64>,
}

/// Exact document/history binding of a dialog continuation. Never stored in dialog JSON.
#[derive(Clone, Debug)]
pub struct ActionHistoryContinuation {
    _policy_lease: Arc<()>,
    /// Set by the first successful `resume`, shared by every clone: a kept clone can't reopen a
    /// scope that already ran (and may have finished).
    resumed: Arc<AtomicBool>,
    group: Option<ActionHistory>,
    active: Option<DocId>,
    documents: Vec<PausedDocument>,
}

impl ActionHistoryContinuation {
    /// Resume before the dialog's edit; intervening edits must never join the original play.
    pub fn resume(&self, s: &mut Session) -> Result<()> {
        if s.action_history.is_some() || s.active().map(|st| st.doc.id) != self.active {
            return Err(EngineError::Other("the paused action's document is no longer current".into()));
        }
        for before in &self.documents {
            let Some(st) = s.docs.iter().find(|st| st.doc.id == before.id) else {
                return Err(EngineError::Other("a paused action's document was closed".into()));
            };
            if !Arc::ptr_eq(&st.doc, &before.doc) || HistoryBinding::of(&st.history) != before.history || st.source_snapshot != before.source_snapshot {
                return Err(EngineError::Other("the paused action's document or history changed".into()));
            }
        }
        if self.resumed.swap(true, Ordering::SeqCst) {
            return Err(EngineError::Other("the paused action was already resumed".into()));
        }
        if let Some(group) = &self.group {
            for before in &group.before {
                if let Some(st) = s.docs.iter_mut().find(|st| st.doc.id == before.id) {
                    let prefix = st.history.current_visibility().to_vec();
                    st.history.begin_visibility_capture(&prefix);
                }
            }
        }
        s.action_history = self.group.clone();
        Ok(())
    }

    /// Rebind a completed nonmodal dialog edit, leaving open-dialog time outside its scope.
    /// Only the scope resumed from this continuation may be taken; a parent remains untouched.
    pub fn suspend_after_edit(&self, s: &mut Session) -> Result<Self> {
        if s.actions.playing != 0 || s.active().map(|st| st.doc.id) != self.active {
            return Err(EngineError::Other("the dialog's action scope is no longer current".into()));
        }
        let active = self.documents.iter().find(|doc| Some(doc.id) == self.active);
        let at_dialog_boundary = |before: &BeforeDocument| active.is_some_and(|doc| before.id == doc.id && Arc::ptr_eq(&before.doc, &doc.doc));
        let owned = match (&self.group, &s.action_history) {
            (None, None) => true,
            (Some(expected), Some(current)) => {
                expected.name == current.name
                    && expected.external == current.external
                    && current.before.len() >= expected.before.len()
                    && current.before.len() <= expected.before.len().saturating_add(1)
                    && expected
                        .before
                        .iter()
                        .zip(&current.before)
                        .all(|(before, now)| before.id == now.id && (Arc::ptr_eq(&before.doc, &now.doc) || at_dialog_boundary(now)))
                    && current.before.iter().skip(expected.before.len()).all(at_dialog_boundary)
            }
            _ => false,
        };
        if !owned {
            return Err(EngineError::Other("the dialog no longer owns the active action scope".into()));
        }
        Ok(Self::capture(s))
    }

    /// Publish the prefix and capture the same document/history/source binding for both callers.
    fn capture(s: &mut Session) -> Self {
        s.paused_action_history.retain(|lease| lease.strong_count() != 0);
        let policy_lease = Arc::new(());
        s.paused_action_history.push(Arc::downgrade(&policy_lease));
        let group = s.action_history.take();
        if let Some(group) = &group {
            consolidate(s, group);
        }
        let active = s.active().map(|st| st.doc.id);
        let documents = s
            .docs
            .iter()
            .filter(|st| Some(st.doc.id) == active || group.as_ref().is_some_and(|g| g.before.iter().any(|b| b.id == st.doc.id)))
            .map(|st| PausedDocument { id: st.doc.id, doc: st.doc.clone(), history: HistoryBinding::of(&st.history), source_snapshot: st.source_snapshot })
            .collect();
        Self { _policy_lease: policy_lease, resumed: Arc::new(AtomicBool::new(false)), group, active, documents }
    }
}

/// Publish the completed prefix, then release the active scope while a dialog is open.
pub fn suspend_history(s: &mut Session) -> Option<ActionHistoryContinuation> {
    if s.actions.playing != 1 {
        return None;
    }
    Some(ActionHistoryContinuation::capture(s))
}

/// End a dialog scope if its command or continuation fails before playback starts.
pub fn finish_pending_history(s: &mut Session) {
    if s.actions.playing == 0 {
        finish_playback(s);
    }
}

#[cfg(test)]
mod tests {
    /// Batch 16 review: a kept clone can't reopen a scope its continuation already resumed and
    /// finished, even with the document and history unchanged.
    #[test]
    fn a_continuation_resumes_once() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("actions.setHistory", json!({"singleState":true})).unwrap();
        begin_document_history(&mut s, "Spell");
        let continuation = ActionHistoryContinuation::capture(&mut s);
        let kept = continuation.clone();
        continuation.resume(&mut s).unwrap();
        finish_document_history(&mut s);
        assert!(kept.resume(&mut s).is_err(), "the clone shares the resumed mark");
        assert!(s.action_history.is_none());
    }

    use super::*;
    use serde_json::json;

    #[test]
    /// Keep the policy fixed until every clone of a paused continuation releases its lease, in either history mode.
    fn history_policy_stays_fixed_until_all_paused_continuations_are_released() {
        for single in [false, true] {
            let mut s = Session::new();
            s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
            begin_document_history(&mut s, "Paused");
            let continuation = ActionHistoryContinuation::capture(&mut s);
            let copy = continuation.clone();
            assert!(s.execute("actions.setHistory", json!({"singleState":!single})).is_err());
            assert_eq!(s.prefs().actions.create_single_history_state, single);
            continuation.resume(&mut s).unwrap();
            finish_document_history(&mut s);
            drop(continuation);
            assert!(s.execute("actions.setHistory", json!({"singleState":!single})).is_err());
            drop(copy);
            s.execute("actions.setHistory", json!({"singleState":!single})).unwrap();
            assert_eq!(s.prefs().actions.create_single_history_state, !single);
        }
    }

    #[test]
    /// Generic preference set/reset routes must preserve the same paused-policy guard as actions.setHistory.
    fn generic_preference_updates_cannot_change_a_paused_history_policy() {
        for single in [false, true] {
            let mut s = Session::new();
            s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
            begin_document_history(&mut s, "Paused");
            let continuation = ActionHistoryContinuation::capture(&mut s);
            for params in [
                json!({"path":"actions.createSingleHistoryState","value":!single}),
                json!({"path":"actions","value":{"createSingleHistoryState":!single}}),
                json!({"path":"","value":{"actions":{"createSingleHistoryState":!single}}}),
                json!({"path":"colorSettings.intent","value":"perceptual","values":{"actions.createSingleHistoryState":!single}}),
            ] {
                let before = s.prefs().clone();
                let color = s.color.settings.clone();
                assert!(s.execute("prefs.set", params).is_err());
                assert_eq!(s.prefs(), &before);
                assert_eq!(s.color.settings, color);
            }
            for params in [json!({"path":"actions.createSingleHistoryState"}), json!({"path":"actions"}), json!({})] {
                assert_eq!(s.execute("prefs.reset", params).is_err(), !single);
                assert_eq!(s.prefs().actions.create_single_history_state, single);
            }
            s.execute("prefs.set", json!({"path":"actions.createSingleHistoryState","value":single})).unwrap();
            s.execute("prefs.set", json!({"path":"performance.historyStates","value":17})).unwrap();
            s.execute("prefs.reset", json!({"path":"cursors"})).unwrap();
            drop(continuation);
            s.execute("prefs.set", json!({"path":"actions.createSingleHistoryState","value":!single})).unwrap();
        }
    }

    #[test]
    /// Rebinding a dialog retains its first edit while respecting an accepted Clear History boundary.
    fn idle_dialog_rebind_keeps_first_edit_and_clear_boundaries() {
        for single in [false, true] {
            for clear in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width":4,"height":4})).unwrap();
                s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
                begin_document_history(&mut s, "Spell");
                if clear {
                    s.execute("layer.renameLayer", json!({"name":"Prefix"})).unwrap();
                    s.execute("history.clear", json!({})).unwrap();
                }
                let before = s.active().unwrap().doc.clone();
                let mut continuation = ActionHistoryContinuation::capture(&mut s);
                for name in ["First correction", "Second correction"] {
                    continuation.resume(&mut s).unwrap();
                    s.execute("layer.renameLayer", json!({"name":name})).unwrap();
                    continuation = continuation.suspend_after_edit(&mut s).unwrap();
                    assert!(s.action_history.is_none(), "idle time must not own a history scope");
                    assert_eq!(s.actions.playing, 0);
                }
                continuation.resume(&mut s).unwrap();
                finish_document_history(&mut s);
                let after = s.active().unwrap().doc.clone();
                assert!(s.undo());
                if single {
                    assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
                } else {
                    assert_ne!(s.active().unwrap().doc.as_ref(), before.as_ref());
                }
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            }
        }
    }

    #[test]
    /// A dialog continuation cannot adopt another caller's active grouping scope.
    fn idle_dialog_rebind_never_takes_an_unrelated_parent_scope() {
        for single in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":4})).unwrap();
            s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
            begin_document_history(&mut s, "Spell");
            let continuation = ActionHistoryContinuation::capture(&mut s);
            // An independently owned parent scope must never be taken, even for a paused
            // per-step action whose policy is now protected from changes.
            s.action_history = Some(ActionHistory { name: "Parent".into(), external: true, before: Vec::new() });
            let before = s.active().unwrap().doc.clone();
            let rows = s.active().unwrap().history.entries();
            assert!(continuation.suspend_after_edit(&mut s).is_err());
            assert!(suspend_history(&mut s).is_none(), "ordinary suspension still requires playing == 1");
            assert_eq!(s.action_history.as_ref().unwrap().name, "Parent");
            assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().history.entries(), rows);
            finish_document_history(&mut s);
        }
    }
}
