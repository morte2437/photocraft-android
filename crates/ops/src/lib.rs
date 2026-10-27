//! Undo/redo history.
//!
//! Because pixel tiles are `Arc`-shared copy-on-write (see `photocraft-raster`), a full
//! [`Document`] clone costs O(layers + tiles) pointer copies, not pixel copies. History therefore
//! stores whole-document snapshots per transaction, which is simple, obviously correct, and the
//! same approach Photoshop's History panel exposes to users (one state per step).
//!
//! Memory is bounded by `max_states` plus an approximate byte budget. The current document's
//! tiles and tiles only history holds count against it, including selection masks.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use photocraft_doc::{Document, LayerId};

mod tiles;

/// The layers a state targeted: the active ("key") layer and every selected layer. Undo and redo
/// bring them back with the state's document, as the reference app does. Selecting layers is not
/// a step of its own, so the session updates the current state's target with
/// [`History::set_current_layers`] just before recording the next step (#1356).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerTarget {
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

#[derive(Clone, Debug)]
pub struct HistoryState {
    pub label: String,
    pub doc: Arc<Document>,
    /// The layers targeted when this state was created.
    pub layers: LayerTarget,
    /// Stable while the state is held: indices shift when older states fall off, ids don't.
    pub id: u64,
}

#[derive(Clone, Debug)]
pub struct History {
    /// Past states; the last one is the state *before* the current document.
    undo: VecDeque<HistoryState>,
    redo: Vec<HistoryState>,
    /// Lazily initialized when a byte budget is used; unlimited histories pay no counting cost.
    tiles: Option<tiles::Tiles>,
    pub max_states: usize,
    /// Pixel memory budget in bytes for the current document plus the tiles only history holds
    /// (0 = unlimited). [`History::trim`] drops the oldest states beyond it.
    pub max_bytes: usize,
    /// Label of the step that produced the current document.
    current_label: String,
    /// The layers targeted when the current document was created (opened or edited).
    current_layers: LayerTarget,
    /// The states History panel › Clear History took off the list, kept so the next undo puts
    /// them back (Clear History is itself undoable). Any new step drops them.
    cleared: Option<Box<Cleared>>,
    /// [`HistoryState::id`] of the current state.
    current_id: u64,
    /// The id the next state gets: never reused, so a discarded state's id names no other.
    next_id: u64,
    /// The History Brush's source state, set in the History panel (`None`: the default).
    brush_source: Option<u64>,
}

/// The undo and redo states [`History::clear_states`] removed, and the History Brush source set
/// before it.
#[derive(Clone, Debug)]
struct Cleared {
    undo: VecDeque<HistoryState>,
    redo: Vec<HistoryState>,
    brush_source: Option<u64>,
}

/// The undo label of the step [`History::clear_states`] adds (Edit › Undo Clear History).
pub const CLEAR_HISTORY_LABEL: &str = "Clear History";

impl Default for History {
    fn default() -> Self {
        Self::new(50)
    }
}

impl History {
    pub fn new(max_states: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            tiles: None,
            max_states: max_states.max(1),
            max_bytes: 0,
            current_label: "Open".into(),
            current_layers: LayerTarget::default(),
            cleared: None,
            current_id: 0,
            next_id: 1,
            brush_source: None,
        }
    }

    /// Record that `before` was replaced by a new current document via step `label`, which left
    /// `layers` targeted.
    pub fn record(&mut self, label: impl Into<String>, before: Arc<Document>, layers: LayerTarget) {
        if self.max_bytes == 0 {
            self.tiles = None;
        }
        // The tile counter also counted the cleared states this step forgets: rebuild it lazily.
        if self.cleared.take().is_some() {
            self.tiles = None;
        }
        if let Some(tiles) = &mut self.tiles {
            tiles.add(&before);
        }
        let prev_label = std::mem::replace(&mut self.current_label, label.into());
        let prev_layers = std::mem::replace(&mut self.current_layers, layers);
        let prev_id = std::mem::replace(&mut self.current_id, self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.undo.push_back(HistoryState { label: prev_label, doc: before, layers: prev_layers, id: prev_id });
        self.clear_redo();
        while self.undo.len() > self.max_states {
            // The History Brush's source doesn't fall off (as in Photoshop): the next oldest does.
            let pinned = self.max_states > 1 && self.undo.front().is_some_and(|s| Some(s.id) == self.brush_source);
            if let Some(state) = self.undo.remove(usize::from(pinned))
                && let Some(tiles) = &mut self.tiles
            {
                tiles.remove(&state.doc);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.cleared.is_some()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        if self.cleared.is_some() {
            return Some(CLEAR_HISTORY_LABEL);
        }
        self.can_undo().then_some(self.current_label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
    /// Labels of the redo states, the next redo first (the History panel's greyed rows).
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }

    /// Set the layers the current document targets from its creation on: when it is first
    /// opened, or when its step coalesces or changes the selection after recording.
    pub fn set_current_layers(&mut self, layers: LayerTarget) {
        self.current_layers = layers;
    }

    /// Undo: `current` becomes redoable; returns the document to make current and the layers it
    /// targeted when it was created.
    pub fn undo(&mut self, current: Arc<Document>) -> Option<(Arc<Document>, LayerTarget)> {
        // Undo Clear History: the cleared states come back; the document doesn't change.
        if let Some(cleared) = self.cleared.take() {
            let Cleared { undo, redo, brush_source } = *cleared;
            self.undo = undo;
            self.redo = redo;
            self.brush_source = brush_source;
            self.tiles = None;
            return Some((current, self.current_layers.clone()));
        }
        let prev = self.undo.pop_back()?;
        if let Some(tiles) = &mut self.tiles {
            tiles.remove(&prev.doc);
            tiles.add(&current);
        }
        let label = std::mem::replace(&mut self.current_label, prev.label);
        let layers = std::mem::replace(&mut self.current_layers, prev.layers.clone());
        let id = std::mem::replace(&mut self.current_id, prev.id);
        self.redo.push(HistoryState { label, doc: current, layers, id });
        Some((prev.doc, prev.layers))
    }

    /// Redo: the mirror of [`Self::undo`].
    pub fn redo(&mut self, current: Arc<Document>) -> Option<(Arc<Document>, LayerTarget)> {
        let next = self.redo.pop()?;
        if let Some(tiles) = &mut self.tiles {
            tiles.remove(&next.doc);
            tiles.add(&current);
        }
        let label = std::mem::replace(&mut self.current_label, next.label);
        let layers = std::mem::replace(&mut self.current_layers, next.layers.clone());
        let id = std::mem::replace(&mut self.current_id, next.id);
        self.undo.push_back(HistoryState { label, doc: current, layers, id });
        Some((next.doc, next.layers))
    }

    /// Entries for a History panel: past labels oldest→newest, then the current label.
    pub fn entries(&self) -> Vec<String> {
        self.undo.iter().map(|s| s.label.clone()).chain(std::iter::once(self.current_label.clone())).collect()
    }

    /// Document of past entry `i`, indexed like [`History::entries`] (0 = oldest). The last entry is
    /// the current document, which the history does not hold, so it (and any index past it) is `None`.
    pub fn state(&self, i: usize) -> Option<Arc<Document>> {
        self.undo.get(i).map(|s| s.doc.clone())
    }

    /// Id of History panel row `i`: the [`History::entries`] (0 = oldest, then the current state),
    /// then the redo states, the next redo first ([`History::redo_labels`]). `None` past the end.
    pub fn row_id(&self, i: usize) -> Option<u64> {
        let past = self.undo.len();
        match i.checked_sub(past) {
            None => self.undo.get(i).map(|s| s.id),
            Some(0) => Some(self.current_id),
            Some(k) => self.redo.iter().rev().nth(k - 1).map(|s| s.id),
        }
    }

    /// The History panel row of the state with id `id`, if it is still held.
    pub fn row_of(&self, id: u64) -> Option<usize> {
        let past = self.undo.len();
        if id == self.current_id {
            return Some(past);
        }
        if let Some(i) = self.undo.iter().position(|s| s.id == id) {
            return Some(i);
        }
        self.redo.iter().rev().position(|s| s.id == id).map(|k| past + 1 + k)
    }

    /// Document of History panel row `i` (see [`History::row_id`]); `current` is the current
    /// document, which the history does not hold.
    pub fn row_doc(&self, i: usize, current: &Arc<Document>) -> Option<Arc<Document>> {
        let past = self.undo.len();
        match i.checked_sub(past) {
            None => self.state(i),
            Some(0) => Some(current.clone()),
            Some(k) => self.redo.iter().rev().nth(k - 1).map(|s| s.doc.clone()),
        }
    }

    /// Make row `i` the History Brush's source (the History panel's source column). It stays on
    /// that state while older states fall off, and the state itself is kept when it would fall off
    /// the history (by count or memory); `false` if there is no such row.
    pub fn set_brush_source(&mut self, i: usize) -> bool {
        let Some(id) = self.row_id(i) else { return false };
        self.brush_source = Some(id);
        true
    }

    /// The History Brush's source row: the oldest state held by default, else the state set with
    /// [`History::set_brush_source`]. `None` once that state is gone (deleted, or discarded by a
    /// new step after stepping back): as in Photoshop, there is then no source until one is set.
    pub fn brush_source(&self) -> Option<usize> {
        match self.brush_source {
            None => Some(0),
            Some(id) => self.row_of(id),
        }
    }

    /// Number of past states (entries before the current one).
    pub fn past_len(&self) -> usize {
        self.undo.len()
    }

    /// Rename the current step (e.g. after folding several steps into one with [`Self::purge_last`]).
    pub fn set_current_label(&mut self, label: impl Into<String>) {
        self.current_label = label.into();
    }

    /// Forget the most recent undo state and every redo state (Edit › Purge › Undo): the
    /// last step can no longer be undone and its pixels are released.
    pub fn purge_last(&mut self) -> bool {
        self.clear_redo();
        if self.cleared.take().is_some() {
            self.tiles = None;
            return true;
        }
        let Some(state) = self.undo.pop_back() else { return false };
        if let Some(tiles) = &mut self.tiles {
            tiles.remove(&state.doc);
        }
        true
    }

    /// Forget the redo states, e.g. after undoing half of a compound step that failed.
    pub fn clear_redo(&mut self) {
        if let Some(tiles) = &mut self.tiles {
            for state in &self.redo {
                tiles.remove(&state.doc);
            }
        }
        self.redo.clear();
    }

    /// Forget every past and redo state (Edit › Purge › Histories). The History Brush's source
    /// goes back to the default, the oldest state held (now the current one): a source set on a
    /// state the purge removed would otherwise leave no source.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.tiles = None;
        self.cleared = None;
        self.reset_brush_source();
    }

    /// Make the History Brush's source the default again (the oldest state held), pinning no state.
    pub fn reset_brush_source(&mut self) {
        self.brush_source = None;
    }

    /// History panel › Clear History: take every state but the current one off the list without
    /// changing the document. Unlike [`Self::clear`] (Edit › Purge › Histories) it is undoable:
    /// the next [`Self::undo`] puts the states back, and the next recorded step forgets them.
    /// The History Brush's source goes back to the default until that undo, as in [`Self::clear`].
    /// Returns false (and changes nothing) when the current state is the only one.
    pub fn clear_states(&mut self) -> bool {
        if self.undo.is_empty() && self.redo.is_empty() {
            return false;
        }
        let undo = std::mem::take(&mut self.undo);
        let redo = std::mem::take(&mut self.redo);
        let brush_source = self.brush_source.take();
        self.cleared = Some(Box::new(Cleared { undo, redo, brush_source }));
        self.tiles = None;
        true
    }

    /// Whether the last change to the list was Clear History (so undo restores the states).
    pub fn is_cleared(&self) -> bool {
        self.cleared.is_some()
    }

    /// The undo and redo states history holds: the cleared ones while Clear History can still be
    /// undone (the list is then empty), so memory figures and the byte budget count them.
    fn held(&self) -> (&VecDeque<HistoryState>, &Vec<HistoryState>) {
        match &self.cleared {
            Some(c) => (&c.undo, &c.redo),
            None => (&self.undo, &self.redo),
        }
    }

    /// Every state history holds, cleared ones included (see [`Self::held`]).
    fn held_states(&self) -> impl Iterator<Item = &HistoryState> {
        let (undo, redo) = self.held();
        undo.iter().chain(redo.iter())
    }

    /// Approximate unique pixel bytes held by history (tiles not shared with `current`).
    pub fn unique_bytes(&self, current: &Document) -> usize {
        let mut seen = HashSet::new();
        tile_bytes(current, &mut seen);
        self.held_states().map(|s| tile_bytes(&s.doc, &mut seen)).sum()
    }

    /// Pixel bytes of `current` plus the tiles only history holds (what [`History::trim`]
    /// bounds by [`History::max_bytes`]).
    pub fn pixel_bytes(&self, current: &Document) -> usize {
        if let Some(tiles) = &self.tiles {
            return usize::try_from(tiles.bytes_with(current)).unwrap_or(usize::MAX);
        }
        let mut seen = HashSet::new();
        let own = tile_bytes(current, &mut seen);
        self.held_states().fold(own, |n, s| n.saturating_add(tile_bytes(&s.doc, &mut seen)))
    }

    /// Bytes of the tiles only `extra` documents hold (History panel snapshots, #4530): not
    /// `current`'s and not any state's. 0 without `extra`, without walking the states.
    pub fn extra_bytes(&self, current: &Document, extra: &[&Document]) -> usize {
        if extra.is_empty() {
            return 0;
        }
        let mut seen = HashSet::new();
        tile_bytes(current, &mut seen);
        for s in self.held_states() {
            tile_bytes(&s.doc, &mut seen);
        }
        extra.iter().fold(0, |n, d| n.saturating_add(tile_bytes(d, &mut seen)))
    }

    /// [`History::unique_bytes`] less the tiles `keep` documents also hold: what clearing the
    /// history frees while they stay (Edit › Purge › Histories keeps the snapshots).
    pub fn unique_bytes_besides(&self, current: &Document, keep: &[&Document]) -> usize {
        let mut seen = HashSet::new();
        tile_bytes(current, &mut seen);
        for d in keep {
            tile_bytes(d, &mut seen);
        }
        self.held_states().fold(0, |n, s| n.saturating_add(tile_bytes(&s.doc, &mut seen)))
    }

    /// Keep pixel memory within [`History::max_bytes`]: the current document's tiles plus the
    /// tiles only history holds (newest states first). The oldest undo states that don't fit are
    /// dropped; the most recent one is always kept so the last step can be undone. The History
    /// Brush's source state ([`History::set_brush_source`]) is never dropped: its unique tiles are
    /// counted first, and the newest states fill what remains. Returns how many states were
    /// dropped.
    pub fn trim(&mut self, current: &Document) -> usize {
        self.trim_with(current, &[])
    }

    /// [`History::trim`] counting the tiles only `pinned` documents hold (History panel
    /// snapshots, which outlive the states) against the budget too, so they can't take memory
    /// beyond it unseen (#4530). The most recent state is still always kept.
    pub fn trim_with(&mut self, current: &Document, pinned: &[&Document]) -> usize {
        // While Clear History can be undone, the cleared states are what history holds: the
        // oldest of them go first, as the list's would.
        let (undo, redo) = match &mut self.cleared {
            Some(c) => (&mut c.undo, &c.redo),
            None => (&mut self.undo, &self.redo),
        };
        if self.max_bytes == 0 || undo.len() <= 1 {
            return 0;
        }
        // An upper bound of what only `pinned` holds (some of it may be in states too): if the
        // states fit beside it, they fit.
        let pinned_bytes = if pinned.is_empty() {
            0
        } else {
            let mut seen = HashSet::new();
            tile_bytes(current, &mut seen);
            pinned.iter().fold(0usize, |n, d| n.saturating_add(tile_bytes(d, &mut seen)))
        };
        let tiles = self.tiles.get_or_insert_with(|| {
            let mut tiles = tiles::Tiles::default();
            for state in undo.iter().chain(redo) {
                tiles.add(&state.doc);
            }
            tiles
        });
        if tiles.fits(current, self.max_bytes.saturating_sub(pinned_bytes)) {
            return 0;
        }
        let mut seen = HashSet::new();
        let mut total = tile_bytes(current, &mut seen);
        for d in pinned {
            total = total.saturating_add(tile_bytes(d, &mut seen));
        }
        for s in redo.iter().rev() {
            total = total.saturating_add(tile_bytes(&s.doc, &mut seen));
        }
        // The History Brush's source is kept whatever its age, so its own tiles come first; the
        // newest states then fill what remains of the budget (the most recent one always).
        let source = self.brush_source.and_then(|id| undo.iter().position(|s| s.id == id));
        if let Some(state) = source.and_then(|i| undo.get(i)) {
            total = total.saturating_add(tile_bytes(&state.doc, &mut seen));
        }
        let mut keep = 0;
        for (i, s) in undo.iter().enumerate().rev() {
            if Some(i) == source {
                continue;
            }
            total = total.saturating_add(tile_bytes(&s.doc, &mut seen));
            if total > self.max_bytes && keep >= 1 {
                break;
            }
            keep += 1;
        }
        let mut kept = VecDeque::with_capacity(keep + 1);
        let mut dropped = 0;
        for (i, state) in std::mem::take(undo).into_iter().enumerate().rev() {
            if Some(i) == source || kept.len() < keep + usize::from(source.is_some_and(|j| j > i)) {
                kept.push_front(state);
            } else {
                tiles.remove(&state.doc);
                dropped += 1;
            }
        }
        *undo = kept;
        dropped
    }
}

/// Bytes of the pixel tiles of `doc` (layers, masks, fill caches, smart filter masks, alpha channels, selection) not already in `seen`.
fn tile_bytes(doc: &Document, seen: &mut HashSet<usize>) -> usize {
    let mut n = 0usize;
    tiles::surfaces(doc, |surface| {
        for (_, tile) in surface.tiles() {
            if seen.insert(Arc::as_ptr(tile) as usize) {
                n = n.saturating_add(tile.bytes().len());
            }
        }
    });
    n
}

#[cfg(test)]
mod tests;
