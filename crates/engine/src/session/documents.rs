use super::history::fix_selection;
use super::*;

/// Move item `i` of `v` to position `to`, clamped to the end. Returns where it went; `None` when
/// `i` is out of range.
pub fn move_item<T>(v: &mut Vec<T>, i: usize, to: usize) -> Option<usize> {
    if i >= v.len() {
        return None;
    }
    let x = v.remove(i);
    let to = to.min(v.len());
    v.insert(to, x);
    Some(to)
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    /// Monotonic active-document generation. It changes when a different document becomes
    /// active, including a switch away and back between two observations; equal values therefore
    /// mean the document was not switched in between.
    pub fn active_generation(&self) -> u64 {
        self.active_generation
    }

    /// Bump [`Self::active_generation`] when the active document id changed. Call after every
    /// assignment to `self.active`.
    pub(crate) fn note_active_change(&mut self) {
        let id = self.active().map(|st| st.doc.id);
        if id != self.active_doc_for_generation {
            self.active_doc_for_generation = id;
            self.active_generation = self.active_generation.wrapping_add(1);
        }
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            self.active = Some(index);
            self.note_active_change();
            self.sync_tools_mask_target();
            true
        } else {
            false
        }
    }

    /// Color Sampler points dropped by the last canvas-size change (crop, Canvas Size, Image Size,
    /// rotation) since this was last called, for the status message. See [`edit`](Self::edit).
    pub fn take_samplers_removed(&mut self) -> usize {
        std::mem::take(&mut self.samplers_removed)
    }

    /// Move the Color Sampler points through `a` in the next [`Session::edit`] (crop, Canvas Size,
    /// Image Size, rotation), dropping the ones that land outside the new canvas. View state: the
    /// points stay pinned to their pixels, no history step. Cleared by the edit it is set for.
    /// The painting symmetry moves through `a` too.
    pub(crate) fn set_sampler_map(&mut self, a: photocraft_geom::Affine) {
        self.set_canvas_map(a, a);
    }

    /// [`set_sampler_map`](Self::set_sampler_map) for an edit whose sampler map differs from its
    /// geometry map (a flip or rotation maps pixel centres): `geometry` moves the painting symmetry.
    pub(crate) fn set_canvas_map(&mut self, geometry: photocraft_geom::Affine, samplers: photocraft_geom::Affine) {
        self.sampler_map = Some(samplers);
        self.symmetry_map = Some(geometry);
    }

    pub fn sync_tools_mask_target(&mut self) {
        let mask = self.active().is_some_and(|st| st.channel_view.target == crate::channel_cmds::ChannelTarget::LayerMask);
        self.tools.target_mask(mask);
    }

    /// Move the document at `from` to tab position `to` (clamped to the last), keeping the active
    /// document active. Returns its new index; `None` when `from` is out of range.
    pub fn move_document(&mut self, from: usize, to: usize) -> Option<usize> {
        let active = self.active().map(|d| d.doc.id);
        let to = move_item(&mut self.docs, from, to)?;
        self.active = active.and_then(|id| self.docs.iter().position(|d| d.doc.id == id));
        self.note_active_change();
        Some(to)
    }

    /// Add a document (from File → New, an import, etc.) and make it active.
    /// Preserve its identity unless another open document already owns it.
    /// Identity-dependent callers must read the admitted document at the returned index.
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // Persisted IDs can overlap the allocator, so check every replacement too.
        while self.docs.iter().any(|st| st.doc.id == doc.id) {
            doc.id = photocraft_doc::DocId::fresh();
        }
        let mut st = DocState::new(doc, path);
        st.history.max_states = self.prefs.get().performance.history_states.max(1) as usize;
        st.history.max_bytes = self.prefs.get().performance.history_budget_bytes();
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
        self.note_active_change();
        i
    }

    /// Holds a decoded document back from opening (see `held_opens`); returns its id.
    pub(crate) fn hold_open(&mut self, doc: Document, source_read_only: bool) -> u64 {
        self.next_held_open = self.next_held_open.wrapping_add(1);
        self.held_opens.push((self.next_held_open, doc, source_read_only));
        self.next_held_open
    }

    /// Takes a document a background open held back for the profile prompt: the document and
    /// whether its source is read-only. `None` when there is no such document (already taken).
    pub fn take_held_open(&mut self, id: u64) -> Option<(Document, bool)> {
        let i = self.held_opens.iter().position(|(h, _, _)| *h == id)?;
        let (_, doc, read_only) = self.held_opens.remove(i);
        Some((doc, read_only))
    }

    pub fn close(&mut self, index: usize) -> Option<DocState> {
        if index >= self.docs.len() {
            return None;
        }
        smart_cmds::on_close(self, index);
        if let Some(id) = self.docs.get(index).map(|d| d.doc.id) {
            self.cancel_jobs_on(id);
            self.subject_cache.forget_doc(id);
            self.object_finder.forget_doc(id);
        }
        let d = self.docs.remove(index);
        self.active = if self.docs.is_empty() {
            None
        } else {
            Some(match self.active {
                Some(active) if active > index => active - 1,
                Some(active) if active < index => active,
                _ => index.min(self.docs.len() - 1),
            })
        };
        self.note_active_change();
        Some(d)
    }
}

pub(super) fn fix_active(st: &mut DocState) {
    if st.active_layer.is_none_or(|id| st.doc.layer(id).is_none()) {
        st.active_layer = st.doc.top_layer();
    }
    fix_selection(st);
    channel_cmds::fix_view(st);
}
