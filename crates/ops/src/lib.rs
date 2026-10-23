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
mod tests {
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
}
