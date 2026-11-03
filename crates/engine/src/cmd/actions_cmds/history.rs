//! Consolidate document history after commands finish their own atomic transactions.
use super::{EngineError, Result, Session};
use photocraft_doc::{DocId, Document};
use photocraft_ops::{History, LayerTarget};
use std::sync::Arc;

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
    let before = BeforeDocument {
        id: st.doc.id,
        doc: st.doc.clone(),
        history: st.history.clone(),
        layers: st.layer_target(),
        coalesce: st.coalesce.clone(),
        saved_revision: st.saved_revision,
        clean: !st.is_dirty(),
    };
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
        if st.history.is_cleared() {
            history.clear_states();
        }
        if let Some(source) = history.preserve_action_brush_source(&st.history) {
            // The selected row will no longer exist, but the History Brush must still paint its
            // exact pixels. Snapshots already retain source pixels without adding an Undo row.
            let id = st.next_snapshot;
            st.next_snapshot = id.saturating_add(1);
            st.snapshots.push(crate::history_cmds::Snapshot { id, name: format!("{}: {}", group.name, source.label), doc: source.doc, layers: source.layers });
            st.source_snapshot = Some(id);
            history.reset_brush_source();
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

pub(crate) fn finish_document_history(s: &mut Session) {
    finish(s);
}

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
}

/// Publish the completed prefix, then release the active scope while a dialog is open.
pub fn suspend_history(s: &mut Session) -> Option<ActionHistoryContinuation> {
    if s.actions.playing != 1 {
        return None;
    }
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
    Some(ActionHistoryContinuation { group, active, documents })
}

/// End a dialog scope if its command or continuation fails before playback starts.
pub fn finish_pending_history(s: &mut Session) {
    if s.actions.playing == 0 {
        finish_playback(s);
    }
}
