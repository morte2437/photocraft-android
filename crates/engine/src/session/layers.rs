use super::history::fix_selection;
use super::{Arc, EngineError, LayerId, Result, Session, channel_cmds, layer_multi_cmds, type_cmds};
use crate::TypeLayerRef;

impl Session {
    /// Replace the active document's active layer without creating a history step.
    pub fn select_layer(&mut self, id: LayerId) -> Result<()> {
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        if st.doc.layer(id).is_none() {
            return Err(EngineError::NoLayer(id));
        }
        st.active_layer = Some(id);
        st.selected_layers = vec![id];
        st.layer_anchor = Some(id);
        channel_cmds::fix_view(st);
        // Selecting a layer is not an edit: keep a clean document clean.
        let clean = st.saved_revision == st.revision;
        st.revision += 1;
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        Ok(())
    }

    /// Put back a layer selection (active layer, selected layers, anchor) that a step moved, such
    /// as the undo ending an empty Type Mask, which restores the selection from before it. `None`
    /// is no layer selected. Selected layers that are gone are dropped; an active layer that is
    /// gone is an error and changes nothing. Like [`Self::select_layer`], not an edit.
    pub fn restore_layer_selection(&mut self, active: Option<LayerId>, selected: &[LayerId], anchor: Option<LayerId>) -> Result<()> {
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        if let Some(id) = active
            && st.doc.layer(id).is_none()
        {
            return Err(EngineError::NoLayer(id));
        }
        st.active_layer = active;
        st.selected_layers = selected.to_vec();
        st.layer_anchor = anchor.filter(|_| active.is_some());
        fix_selection(st);
        channel_cmds::fix_view(st);
        let clean = st.saved_revision == st.revision;
        st.revision += 1;
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        Ok(())
    }

    /// Re-renders the pixels of type layers whose font arrived after they were drawn (the web
    /// build fetches served fonts on demand, `photocraft_text::served`). The layers' model is
    /// unchanged, so this is no history step and leaves a clean document clean. Unknown documents
    /// and layers are skipped. A document a background job is computing from must not move under
    /// it: its layers are returned, to try again later.
    pub fn refresh_type_layers(&mut self, layers: &[TypeLayerRef]) -> Vec<TypeLayerRef> {
        let mut busy = Vec::new();
        let mut docs: Vec<photocraft_doc::DocId> = Vec::new();
        for l in layers {
            if !docs.contains(&l.doc) {
                docs.push(l.doc);
            }
        }
        for doc_id in docs {
            if self.job_on(doc_id).is_some() {
                busy.extend(layers.iter().filter(|l| l.doc == doc_id));
                continue;
            }
            let ids = |first: bool| -> Vec<LayerId> { layers.iter().filter(|l| l.doc == doc_id && l.first_snapshot == first).map(|l| l.layer).collect() };
            let Some(st) = self.docs.iter_mut().find(|st| st.doc.id == doc_id) else { continue };
            // The first snapshot is the document as opened (#2966, #6087): it was taken before the
            // font arrived, so its type layers get the same refresh, or returning to it brings the
            // fallback-font pixels back. While it still is the open document, it shares the
            // refreshed one; once edited, it is refreshed on its own, layers the edit removed too.
            let first_is_doc = st.snapshots.iter().any(|s| s.id == 0 && Arc::ptr_eq(&s.doc, &st.doc));
            if !first_is_doc
                && let Some(first) = st.snapshots.iter_mut().find(|s| s.id == 0)
                && let Some(doc) = refresh_text_layers(&first.doc, &ids(true))
            {
                first.doc = Arc::new(doc);
            }
            // While shared, a layer picked in the snapshot is a layer of the open document too
            // (a document busy when its font arrived can be back at the snapshot by the retry).
            let mut open = ids(false);
            if first_is_doc {
                for id in ids(true) {
                    if !open.contains(&id) {
                        open.push(id);
                    }
                }
            }
            let Some(doc) = refresh_text_layers(&st.doc, &open) else { continue };
            let doc = Arc::new(doc);
            if first_is_doc && let Some(first) = st.snapshots.iter_mut().find(|s| s.id == 0) {
                first.doc = doc.clone();
            }
            let clean = st.saved_revision == st.revision;
            let before = std::mem::replace(&mut st.doc, doc);
            st.revision += 1;
            if clean {
                st.saved_revision = st.revision;
            }
            // Only the refreshed layers' caches changed: their changed tiles.
            st.last_damage = layer_multi_cmds::step_damage(&before, &st.doc);
        }
        busy
    }
}

/// `doc` with its type layers among `ids` re-rendered (`Session::refresh_type_layers`), or `None`
/// when none of them is a type layer of `doc`.
fn refresh_text_layers(doc: &Arc<photocraft_doc::Document>, ids: &[LayerId]) -> Option<photocraft_doc::Document> {
    let mut out = (**doc).clone();
    let mut changed = false;
    for id in ids {
        if let Some(photocraft_doc::Layer { content: photocraft_doc::LayerContent::Text(t), .. }) = out.layer_mut(*id) {
            type_cmds::refresh(doc, t);
            changed = true;
        }
    }
    changed.then_some(out)
}
