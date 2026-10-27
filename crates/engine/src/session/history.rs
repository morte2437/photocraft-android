use super::documents::fix_active;
use super::*;

impl Session {
    pub fn undo(&mut self) -> bool {
        // A background job is computing from the current state: it must not move under it.
        if self.active_job().is_some() {
            return false;
        }
        let Some(st) = self.active_mut() else { return false };
        st.coalesce = None;
        match st.history.undo(st.doc.clone()) {
            // Undo Clear History: the list comes back, the document is the same one. Not an
            // edit: keep a clean document clean (the revision still moves for the panels).
            Some((d, layers)) if Arc::ptr_eq(&d, &st.doc) => {
                restore_target(st, layers);
                let clean = !st.is_dirty();
                st.revision += 1;
                if clean {
                    st.saved_revision = st.revision;
                }
                st.last_damage = Some(photocraft_geom::Rect::EMPTY);
                true
            }
            Some((mut d, layers)) => {
                // Save As changes file identity outside history, just like the saved path.
                if d.name != st.doc.name {
                    Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
                }
                // Pixels this step can have touched, so the canvas recomposites only that
                // (it recomposited everything before).
                let damage = layer_multi_cmds::step_damage(&d, &st.doc);
                let before = std::mem::replace(&mut st.doc, d);
                let keep_clean = st.keeps_clean(&before);
                restore_target(st, layers);
                st.bump_revision(keep_clean);
                st.last_damage = damage;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        if self.active_job().is_some() {
            return false;
        }
        let Some(st) = self.active_mut() else { return false };
        st.coalesce = None;
        match st.history.redo(st.doc.clone()) {
            Some((mut d, layers)) => {
                if d.name != st.doc.name {
                    Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
                }
                let damage = layer_multi_cmds::step_damage(&st.doc, &d);
                let before = std::mem::replace(&mut st.doc, d);
                let keep_clean = st.keeps_clean(&before);
                restore_target(st, layers);
                st.bump_revision(keep_clean);
                st.last_damage = damage;
                true
            }
            None => false,
        }
    }
}

/// Undo / redo: target the layers the restored state targeted, as far as they still exist.
pub(crate) fn restore_target(st: &mut DocState, layers: LayerTarget) {
    // Deselect Layers is a valid history target, not a missing layer to replace with the top
    // layer. An edit after deselection must undo back to no active or selected layers (#3042).
    if layers.active.is_none() && layers.selected.is_empty() {
        st.active_layer = None;
        st.selected_layers.clear();
        st.layer_anchor = None;
        channel_cmds::fix_view(st);
        return;
    }
    if layers.active.is_some_and(|id| st.doc.layer(id).is_some()) {
        st.active_layer = layers.active;
        st.selected_layers = layers.selected;
    }
    fix_active(st);
}

/// Keep the layer selection valid after the document or the active layer changed: drop deleted
/// layers, and collapse to the active layer when a command made a non-member active.
pub(crate) fn fix_selection(st: &mut DocState) {
    let Some(active) = st.active_layer else {
        st.selected_layers.clear();
        return;
    };
    if !st.selected_layers.contains(&active) {
        st.selected_layers = vec![active];
    } else {
        let live: std::collections::HashSet<LayerId> = st.doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        st.selected_layers.retain(|id| live.contains(id));
        let mut seen = std::collections::HashSet::new();
        st.selected_layers.retain(|id| seen.insert(*id));
    }
    if st.layer_anchor.is_some_and(|a| !st.selected_layers.contains(&a)) {
        st.layer_anchor = Some(active);
    }
}
