use super::documents::fix_active;
use super::{Arc, DocState, Document, LayerId, LayerTarget, Session, channel_cmds, layer_multi_cmds};

impl Session {
    pub fn undo(&mut self) -> bool {
        // A background job is computing from the current state: it must not move under it.
        if self.active_job().is_some() {
            return false;
        }
        let keep_visibility = !self.visibility_undoable();
        let Some(st) = self.active_mut() else { return false };
        st.coalesce = None;
        // The step being undone: the layers whose visibility it changed get it back.
        let changed = st.history.current_visibility().to_vec();
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
                if keep_visibility {
                    carry_visibility(&st.doc, &mut d, &changed);
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
        let keep_visibility = !self.visibility_undoable();
        let Some(st) = self.active_mut() else { return false };
        st.coalesce = None;
        match st.history.redo(st.doc.clone()) {
            Some((mut d, layers)) => {
                if d.name != st.doc.name {
                    Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
                }
                if keep_visibility {
                    // The step being redone sets the visibility it changed again.
                    let changed = st.history.current_visibility().to_vec();
                    carry_visibility(&st.doc, &mut d, &changed);
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

impl Session {
    /// Take back the step a compound command just recorded, after its later part failed: the
    /// document before the command is current again, nothing is left to redo, and in non-linear
    /// history the later states that step moved stay where they were (an undo would land on the
    /// last of them). Falls back to Undo plus forgetting the redo states when the history
    /// changed since that step. Returns whether a step was taken back.
    pub fn rollback(&mut self) -> bool {
        if self.active_job().is_some() {
            return false;
        }
        let keep_visibility = !self.visibility_undoable();
        let Some(st) = self.active_mut() else { return false };
        st.coalesce = None;
        // The step taken back (and the later states it moved, in non-linear history): the layers
        // whose visibility it changed get it back, the unrecorded changes stay, as for Undo.
        let changed = st.history.current_visibility().to_vec();
        let Some((mut d, layers)) = st.history.rollback() else {
            let undone = self.undo();
            if undone && let Some(st) = self.active_mut() {
                st.history.clear_redo();
            }
            return undone;
        };
        if d.name != st.doc.name {
            Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
        }
        if keep_visibility {
            carry_visibility(&st.doc, &mut d, &changed);
        }
        let damage = layer_multi_cmds::step_damage(&d, &st.doc);
        let before = std::mem::replace(&mut st.doc, d);
        // Taking back a selection-only step keeps a clean document clean, as Undo does (#6865).
        let keep_clean = st.keeps_clean(&before);
        restore_target(st, layers);
        st.bump_revision(keep_clean);
        st.last_damage = damage;
        true
    }
}

/// History Options › Make Layer Visibility Changes Undoable off: showing or hiding a layer is not
/// a history state, so stepping through history must not bring the old visibility back either
/// (history states hold whole documents). Gives every layer of `to` that `from` also has the
/// visibility it has in `from`, except the `stepped` layers: a recorded step between the two
/// states changed their visibility (Apply Layer Comp, a snapshot, a visibility change recorded
/// while the option was on), so they keep the visibility `to` holds. Layers only one of them has
/// keep theirs. Copies `to` only when something differs.
pub(crate) fn carry_visibility(from: &Document, to: &mut Arc<Document>, stepped: &[LayerId]) {
    let visible: std::collections::HashMap<LayerId, bool> = from.walk().into_iter().map(|(_, _, l)| (l.id, l.visible)).collect();
    let changes: Vec<(LayerId, bool)> = to
        .walk()
        .into_iter()
        .filter(|(_, _, l)| !stepped.contains(&l.id))
        .filter_map(|(_, _, l)| visible.get(&l.id).filter(|v| **v != l.visible).map(|v| (l.id, *v)))
        .collect();
    if changes.is_empty() {
        return;
    }
    let d = Arc::make_mut(to);
    for (id, v) in changes {
        if let Some(l) = d.layer_mut(id) {
            l.visible = v;
        }
    }
}

/// The layers (in both documents) whose visibility differs between `before` and `after`: what a
/// recorded step changed, for [`photocraft_ops::History::add_current_visibility`].
pub(crate) fn visibility_changes(before: &Document, after: &Document) -> Vec<LayerId> {
    let visible: std::collections::HashMap<LayerId, bool> = before.walk().into_iter().map(|(_, _, l)| (l.id, l.visible)).collect();
    after.walk().into_iter().filter(|(_, _, l)| visible.get(&l.id).is_some_and(|v| *v != l.visible)).map(|(_, _, l)| l.id).collect()
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
