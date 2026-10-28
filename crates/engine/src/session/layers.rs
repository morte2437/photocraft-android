use super::history::fix_selection;
use super::{Arc, EngineError, LayerId, Result, Session, channel_cmds, type_cmds};

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
    pub fn refresh_type_layers(&mut self, layers: &[(photocraft_doc::DocId, LayerId)]) -> Vec<(photocraft_doc::DocId, LayerId)> {
        let mut busy = Vec::new();
        let mut docs: Vec<photocraft_doc::DocId> = Vec::new();
        for (d, _) in layers {
            if !docs.contains(d) {
                docs.push(*d);
            }
        }
        for doc_id in docs {
            let mine = layers.iter().filter(|(d, _)| *d == doc_id);
            if self.job_on(doc_id).is_some() {
                busy.extend(mine);
                continue;
            }
            let Some(st) = self.docs.iter_mut().find(|st| st.doc.id == doc_id) else { continue };
            let snapshot = st.doc.clone();
            let mut doc = (*snapshot).clone();
            let mut changed = false;
            for (_, id) in mine {
                if let Some(photocraft_doc::Layer { content: photocraft_doc::LayerContent::Text(t), .. }) = doc.layer_mut(*id) {
                    type_cmds::refresh(&snapshot, t);
                    changed = true;
                }
            }
            if changed {
                let clean = st.saved_revision == st.revision;
                st.doc = Arc::new(doc);
                st.revision += 1;
                if clean {
                    st.saved_revision = st.revision;
                }
                st.last_damage = None;
            }
        }
        busy
    }
}
