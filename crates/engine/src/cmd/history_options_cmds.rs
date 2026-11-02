//! History panel › History Options… (#2966): Allow Non-Linear History and Make Layer Visibility
//! Changes Undoable. The values are preferences (`historyOptions.*`, also `prefs.set`), applied
//! by [`Session::apply_prefs`] and [`Session::edit_visibility`]. Without params the command
//! reports them; the GUI opens its dialog instead and runs it with the dialog's values on OK.
//!
//! Photoshop's three snapshot options (Automatically Create First Snapshot, Automatically
//! Create New Snapshot When Saving, Show New Snapshot Dialog by Default) come with #3634, which
//! makes the snapshots act on them.

use serde_json::{Map, Value};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "history.options";

/// The option keys, as in `prefs.get historyOptions` and the command's params.
pub const OPTIONS: [&str; 2] = ["allowNonLinearHistory", "makeLayerVisibilityChangesUndoable"];

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The current History Options as `{allowNonLinearHistory, makeLayerVisibilityChangesUndoable}`.
pub fn current(s: &Session) -> Value {
    let o = &s.prefs().history_options;
    serde_json::json!({
        "allowNonLinearHistory": o.allow_non_linear_history,
        "makeLayerVisibilityChangesUndoable": o.make_layer_visibility_changes_undoable,
    })
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let given: Map<String, Value> = match p {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(bad("params must be an object")),
    };
    // Validate everything before changing anything.
    let mut set = Vec::new();
    for (k, v) in &given {
        if !OPTIONS.contains(&k.as_str()) {
            return Err(bad(format!("unknown option `{k}` (expected {})", OPTIONS.join(", "))));
        }
        let b = v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false")))?;
        set.push((k.clone(), b));
    }
    if !set.is_empty() {
        s.edit_prefs(|prefs| {
            for (k, b) in &set {
                match k.as_str() {
                    "allowNonLinearHistory" => prefs.history_options.allow_non_linear_history = *b,
                    _ => prefs.history_options.make_layer_visibility_changes_undoable = *b,
                }
            }
        });
    }
    Ok(current(s))
}

/// History panel › Delete in non-linear history: delete state `i` (numbered as the History panel
/// lists them, `current` = the current state's index, the later dimmed states after it) and
/// nothing else. Deleting the current state steps back to the one before it; the later states
/// stay.
pub(crate) fn delete_state(s: &mut Session, i: usize, current: usize) -> Result<()> {
    if i == 0 {
        return Err(EngineError::BadParams { cmd: "history.deleteState".into(), msg: "the oldest state can't be deleted".into() });
    }
    // A background job is computing from the current state: it must not move under it.
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't step back right now".into()));
    }
    let keep_visibility = !s.visibility_undoable();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.coalesce = None;
    if i != current {
        let removed = match i.checked_sub(current.saturating_add(1)) {
            Some(j) => st.history.remove_later(j),
            None => st.history.remove(i),
        };
        if !removed {
            return Err(EngineError::Other(format!("state {i} is not held")));
        }
        // The document is unchanged, but the History panel's rows are not: a view-only change
        // that keeps a clean document clean.
        let clean = st.saved_revision == st.revision;
        st.revision += 1;
        st.view_revision += 1;
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(photocraft_geom::Rect::EMPTY);
        return Ok(());
    }
    // The step that made the current state: the layers whose visibility it changed get it back.
    let changed = st.history.current_visibility().to_vec();
    let (mut d, layers) = st.history.drop_current().ok_or_else(|| EngineError::Other("the history can't step back right now".into()))?;
    // As Undo does: Save As changes file identity outside history.
    if d.name != st.doc.name {
        std::sync::Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
    }
    if keep_visibility {
        crate::carry_visibility(&st.doc, &mut d, &changed);
    }
    let damage = crate::layer_multi_cmds::step_damage(&d, &st.doc);
    st.doc = d;
    crate::restore_target(st, layers);
    st.revision += 1;
    st.last_damage = damage;
    Ok(())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "History Options…",
        menu: &[],
        shortcut: None,
        params: r##"{"allowNonLinearHistory":bool?,"makeLayerVisibilityChangesUndoable":bool?} → both values (no params: report them). Allow Non-Linear History: an edit after stepping back keeps the later states and is appended at the end; deleting a state deletes only it (default off). Make Layer Visibility Changes Undoable: showing or hiding a layer is a history state (default on). Also `prefs.set historyOptions.*`"##,
        enabled: always,
        run: options,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entries(s: &Session) -> Vec<String> {
        s.active().unwrap().history.entries()
    }

    /// A document with a layer above the background: entries Open, New Layer.
    fn doc() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        (s, id)
    }

    #[test]
    fn defaults_match_photoshop_and_round_trip_through_prefs() {
        let mut s = Session::new();
        assert_eq!(s.execute(CMD, json!({})).unwrap(), json!({"allowNonLinearHistory": false, "makeLayerVisibilityChangesUndoable": true}));
        assert_eq!(s.execute("prefs.get", json!({"path": "historyOptions"})).unwrap(), current(&s));
        let r = s.execute(CMD, json!({"allowNonLinearHistory": true, "makeLayerVisibilityChangesUndoable": false})).unwrap();
        assert_eq!(r, json!({"allowNonLinearHistory": true, "makeLayerVisibilityChangesUndoable": false}));
        assert_eq!(s.execute("prefs.get", json!({"path": "historyOptions.allowNonLinearHistory"})).unwrap(), json!(true));
        // Saved and restored with the other preferences.
        let saved = s.prefs_to_json();
        let mut t = Session::new();
        t.load_prefs_json(&saved).unwrap();
        assert_eq!(current(&t), r);
        // prefs.set and prefs.reset reach them too.
        t.execute("prefs.set", json!({"path": "historyOptions.allowNonLinearHistory", "value": false})).unwrap();
        assert_eq!(t.execute(CMD, Value::Null).unwrap()["allowNonLinearHistory"], json!(false));
        t.execute("prefs.reset", json!({"path": "historyOptions"})).unwrap();
        assert_eq!(current(&t), current(&Session::new()));
        // Older preference files without the section load with the defaults.
        t.load_prefs_json("{}").unwrap();
        assert_eq!(current(&t), current(&Session::new()));
    }

    #[test]
    fn bad_params_are_refused_and_change_nothing() {
        let mut s = Session::new();
        let before = current(&s);
        for p in [
            json!({"allowNonLinearHistory": "yes"}),
            json!({"allowNonLinearHistory": 1}),
            json!({"makeLayerVisibilityChangesUndoable": null}),
            json!({"allowNonLinearHistory": true, "snapshots": true}),
            json!([true]),
            json!("x"),
            json!(3),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
            assert_eq!(current(&s), before, "{p} changed nothing");
        }
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.allowNonLinearHistory", "value": "on"})).is_err());
        assert_eq!(current(&s), before);
    }

    #[test]
    fn layer_visibility_is_a_history_state_only_when_undoable() {
        // On (the default): the eye, Hide/Show Layers and ⌥-click each add a state.
        let (mut s, id) = doc();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert_eq!(entries(&s).last().map(String::as_str), Some("Layer Visibility"));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().visible);

        // Off: the document changes (and is modified) but History doesn't.
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        let before = entries(&s);
        let rev = s.active().unwrap().revision;
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("layer.showLayers", json!({"layer": id})).unwrap();
        s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
        s.execute("layer.showOnly", json!({"layer": id})).unwrap();
        s.execute("layer.showOnly", json!({"layer": id})).unwrap();
        // Hide Layers toggles (⌘,), so show the layer first for it to hide.
        s.execute("layer.showLayers", json!({"layer": id})).unwrap();
        s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
        assert_eq!(entries(&s), before, "no visibility state was recorded");
        let st = s.active().unwrap();
        assert!(!st.doc.layer(photocraft_doc::LayerId(id)).unwrap().visible, "the change still applies");
        assert!(st.revision > rev && st.is_dirty());
        // Other properties still record a state.
        s.execute("layer.setProps", json!({"layer": id, "opacity": 0.5})).unwrap();
        assert_eq!(entries(&s).len(), before.len() + 1);
        // An unrecorded change leaves the redo states alone.
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        assert!(s.active().unwrap().history.can_redo());
    }

    fn visible(s: &Session, id: u64) -> bool {
        s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().visible
    }

    fn red(s: &Session, id: u64) -> bool {
        let st = s.active().unwrap();
        st.doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().pixel(1, 1)[0] > 0.5
    }

    /// Off, showing or hiding a layer is not a state, so stepping through history (Undo, Redo, a
    /// History row) leaves the visibility as the user set it, as Photoshop does; history states
    /// hold whole documents and would otherwise bring the old visibility back.
    #[test]
    fn stepping_through_history_keeps_unrecorded_visibility() {
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert!(s.execute("edit.undo", json!({})).unwrap().as_bool().unwrap());
        assert!(!red(&s, id), "the fill is undone");
        assert!(!visible(&s, id), "the layer stays hidden");
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("edit.redo", json!({})).unwrap();
        assert!(red(&s, id) && visible(&s, id), "redo brings the fill back, not the hidden layer");
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        // History rows: New, New Layer, Fill.
        s.execute("history.selectState", json!({"state": 1})).unwrap();
        assert!(!red(&s, id) && !visible(&s, id));
        s.execute("history.selectState", json!({"state": 2})).unwrap();
        assert!(red(&s, id) && !visible(&s, id));

        // On (the default): Undo takes the hide back first, then the fill.
        let (mut s, id) = doc();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(red(&s, id) && visible(&s, id));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(!red(&s, id) && visible(&s, id));
        s.execute("history.selectState", json!({"state": 3})).unwrap();
        assert!(red(&s, id) && !visible(&s, id), "the hide is a state of its own");
    }

    /// Off, only the unrecorded visibility changes carry across history: a recorded step that
    /// changed visibility (Apply Layer Comp) gives it back when undone and sets it again when
    /// redone, whether by Undo / Redo, a History row or deleting the current state.
    #[test]
    fn stepping_over_a_recorded_visibility_change_restores_it() {
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        // A comp with the layer hidden, then the eye shows it again (no state) and a fill.
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("layerComp.new", json!({"name": "Hidden"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layerComp.apply", json!({"comp": "Hidden"})).unwrap();
        assert!(red(&s, id) && !visible(&s, id), "the comp hides the layer");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(red(&s, id) && visible(&s, id), "undoing Apply Layer Comp shows the layer again");
        // The eye hides it (no state): undoing the fill keeps that.
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(!red(&s, id) && !visible(&s, id), "the unrecorded hide carries across the fill's undo");
        s.execute("edit.redo", json!({})).unwrap();
        assert!(red(&s, id) && !visible(&s, id));
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("edit.redo", json!({})).unwrap();
        assert!(red(&s, id) && !visible(&s, id), "redoing the comp hides the layer again");

        // History rows: New, New Layer, New Layer Comp, Fill, Apply Layer Comp.
        assert_eq!(entries(&s).len(), 5);
        s.execute("history.selectState", json!({"state": 2})).unwrap();
        assert!(!red(&s, id) && visible(&s, id), "jumping back over the comp shows the layer again");
        s.execute("history.selectState", json!({"state": 4})).unwrap();
        assert!(red(&s, id) && !visible(&s, id), "jumping forward over the comp hides it again");

        // Non-linear Delete of the current state (the comp) steps back and shows the layer too.
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        s.execute("history.deleteState", json!({})).unwrap();
        assert!(red(&s, id) && visible(&s, id), "deleting the comp's state shows the layer again");

        // Non-linear Delete of a past state: the step after it now starts from the state before
        // it, so undoing that step also gives back the visibility the deleted step changed.
        s.execute("layerComp.apply", json!({"comp": "Hidden"})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let comp = entries(&s).len() - 2;
        s.execute("history.deleteState", json!({"state": comp})).unwrap();
        assert!(!visible(&s, id));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(red(&s, id) && visible(&s, id), "back at the red fill, where the layer was shown");
    }

    #[test]
    fn non_linear_history_keeps_later_states_and_delete_removes_only_one() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        for c in ["#ff0000", "#00ff00", "#0000ff"] {
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        let filled = entries(&s);
        assert_eq!(filled.len(), 4);
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let after = entries(&s);
        assert_eq!(after.len(), 5, "the two later fills are kept: {after:?}");
        assert_eq!(after[..4], filled[..]);
        // Delete the second fill only: the states after it stay.
        s.execute("history.deleteState", json!({"state": 2})).unwrap();
        let mut expected = after.clone();
        expected.remove(2);
        assert_eq!(entries(&s), expected);
        // Deleting the current state steps back to the one before; nothing else goes.
        let doc_before = s.active().unwrap().history.state(expected.len() - 2).unwrap();
        s.execute("history.deleteState", json!({})).unwrap();
        assert_eq!(entries(&s), expected[..expected.len() - 1].to_vec());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &doc_before));
        // After stepping back, deleting the current state keeps the later (redo) states.
        s.execute("edit.undo", json!({})).unwrap();
        let redo: Vec<String> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
        s.execute("history.deleteState", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);

        // Turning it off applies to open documents: the next edit after stepping back discards.
        s.execute(CMD, json!({"allowNonLinearHistory": false})).unwrap();
        assert!(!s.active().unwrap().history.non_linear);
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        assert!(!s.active().unwrap().history.can_redo());
        // New documents pick the option up too.
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(s.active().unwrap().history.non_linear);
    }

    /// With the oldest state current, non-linear Delete is still enabled for a later (dimmed)
    /// state (the History panel's trash and its drop target ask for it); linear history has
    /// nothing to delete there.
    #[test]
    fn non_linear_delete_is_enabled_while_the_oldest_state_is_current() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        for c in ["#ff0000", "#00ff00"] {
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(!s.is_enabled("history.deleteState"), "linear: only the oldest state is left to delete");
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        assert!(s.is_enabled("history.deleteState"));
        let doc = s.active().unwrap().doc.clone();
        s.execute("history.deleteState", json!({"state": 1})).unwrap();
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&st.doc, &doc), "the document is unchanged");
        assert_eq!(st.history.redo_labels().count(), 1, "only the first fill went");
        assert!(s.execute("history.deleteState", json!({})).is_err(), "the oldest state itself stays");
        s.execute("history.deleteState", json!({"state": 1})).unwrap();
        assert!(!s.is_enabled("history.deleteState"), "nothing left but the oldest state");
    }

    /// Non-linear Delete of a past or later state leaves the document as it is, so a saved
    /// (clean) document stays clean; deleting the current state steps back and modifies it.
    #[test]
    fn non_linear_delete_of_another_state_keeps_a_clean_document_clean() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        for c in ["#ff0000", "#00ff00", "#0000ff", "#808080"] {
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        s.execute("edit.undo", json!({})).unwrap();
        // States: New, Fill 1-3 (3 current), the dimmed Fill 4. As if just saved.
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let rev = st.revision;
        s.execute("history.deleteState", json!({"state": 1})).unwrap();
        let st = s.active().unwrap();
        assert!(!st.is_dirty(), "deleting a past state left the saved document unmodified");
        assert!(st.revision > rev, "the History panel still sees the change");
        s.execute("history.deleteState", json!({"state": 3})).unwrap();
        assert!(!s.active().unwrap().is_dirty(), "deleting a later state left it unmodified too");
        assert_eq!(s.active().unwrap().history.redo_labels().count(), 0);
        // A modified document stays modified.
        s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
        s.execute("history.deleteState", json!({"state": 1})).unwrap();
        assert!(s.active().unwrap().is_dirty());
        // The current state: the document steps back, which is a change.
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        s.execute("history.deleteState", json!({})).unwrap();
        assert!(s.active().unwrap().is_dirty());
    }

    /// With the option off, a recorded step that changes visibility along with other properties
    /// (layer.setProps visible + opacity is one Layer Properties state) is undone and redone as a
    /// whole: the carried-over visibility skips the layers the step itself changed.
    #[test]
    fn undoing_a_mixed_visibility_and_opacity_change_undoes_both() {
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        let n = entries(&s).len();
        s.execute("layer.setProps", json!({"layer": id, "visible": false, "opacity": 0.5})).unwrap();
        assert_eq!(entries(&s).len(), n + 1, "the mixed change is a state");
        let opacity = |s: &Session| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().opacity;
        s.execute("edit.undo", json!({})).unwrap();
        assert!(visible(&s, id) && (opacity(&s) - 1.0).abs() < 1e-6, "undo shows the layer again and restores its opacity");
        s.execute("edit.redo", json!({})).unwrap();
        assert!(!visible(&s, id) && (opacity(&s) - 0.5).abs() < 1e-6, "redo hides it again");
    }

    /// Non-linear Delete reaches a later (dimmed) state too, and deletes only it; linear history
    /// still refuses one past the current state.
    #[test]
    fn non_linear_delete_removes_a_later_state() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        for c in ["#ff0000", "#00ff00", "#0000ff"] {
            s.execute("edit.fill", json!({"color": c})).unwrap();
        }
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.execute("history.deleteState", json!({"state": 2})).is_err(), "linear: not a deletable state");
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let mut expected = entries(&s);
        let mut redo: Vec<String> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
        // Entries: New, Fill (current), then the dimmed Fill 2 (state 2) and Fill 3 (state 3).
        s.execute("history.deleteState", json!({"state": 2})).unwrap();
        redo.remove(0);
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&st.doc, &doc), "the document is unchanged");
        assert_eq!(st.history.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);
        assert_eq!(entries(&s), expected);
        assert!(s.execute("history.deleteState", json!({"state": 4})).is_err(), "past the last state");
        s.execute("edit.redo", json!({})).unwrap();
        expected.push(redo[0].clone());
        assert_eq!(entries(&s), expected);
        let px = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
        assert!(px[2] > 0.5 && px[1] < 0.5, "redo reaches the blue fill: {px:?}");
    }

    /// Off, taking a step back with `Session::rollback` (a cancelled Free Transform of a made
    /// layer, an abandoned Alt-drag copy, a compound command whose later part failed) keeps the
    /// unrecorded visibility as Undo does, and gives back the visibility the step itself changed,
    /// whether it takes the step back exactly, falls back to Undo or lands on the oldest state.
    #[test]
    fn rollback_keeps_unrecorded_visibility() {
        // Exactly: the fill is taken back, the eye's hide stays.
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert!(s.rollback());
        assert!(!red(&s, id), "the fill is taken back");
        assert!(!visible(&s, id), "the unrecorded hide stays");
        assert!(!s.active().unwrap().history.can_redo(), "nothing to redo");

        // Landing on the oldest state (the New Layer step taken back): the hidden background stays
        // hidden.
        let (mut s, _) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id.0;
        s.execute("layer.setProps", json!({"layer": bg, "visible": false})).unwrap();
        assert!(s.rollback());
        assert_eq!(s.active().unwrap().doc.layers.len(), 1, "the new layer is taken back");
        assert!(!visible(&s, bg), "the unrecorded hide stays on the oldest state");
        assert!(!s.rollback(), "nothing left to take back");
        assert!(!visible(&s, bg));

        // A step that changed visibility (Apply Layer Comp) gives it back.
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("layerComp.new", json!({"name": "Hidden"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("layerComp.apply", json!({"comp": "Hidden"})).unwrap();
        assert!(!visible(&s, id));
        assert!(s.rollback());
        assert!(visible(&s, id), "taking back Apply Layer Comp shows the layer again");

        // Falling back to Undo (the history changed since the fill: undo, redo).
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.redo", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert!(s.rollback());
        assert!(!red(&s, id) && !visible(&s, id), "the fallback Undo keeps the hide too");
        assert!(!s.active().unwrap().history.can_redo());

        // Non-linear: a step recorded after stepping back, taken back exactly, keeps the hide and
        // gives the later state back to redo.
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false, "allowNonLinearHistory": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert!(s.rollback());
        assert!(!red(&s, id) && !visible(&s, id), "back before the invert, still hidden");
        assert!(s.active().unwrap().history.can_redo(), "the fill is redoable again");

        // On (the default) the eye records a state of its own, which a rollback takes back.
        let (mut s, id) = doc();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert!(s.rollback());
        assert!(red(&s, id) && visible(&s, id), "the hide is taken back, the fill stays");
    }

    /// Non-linear, a step that moves more later states than the History States limit leaves room
    /// for is still taken back exactly by a rollback (a compound command whose later part failed):
    /// the document before it comes back with the full list, not an Undo onto the last later
    /// state that also drops the rest.
    #[test]
    fn rollback_past_the_state_limit_restores_the_list() {
        let (mut s, id) = doc();
        s.execute(CMD, json!({"allowNonLinearHistory": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let before = entries(&s);
        let redo: Vec<String> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
        assert_eq!(redo.len(), 2);
        s.active_mut().unwrap().history.max_states = 2;
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        assert!(s.rollback());
        assert_eq!(entries(&s), before);
        assert_eq!(s.active().unwrap().history.redo_labels().map(str::to_string).collect::<Vec<_>>(), redo);
        assert!(!red(&s, id), "back on the New Layer state, not on the inverted fill");
        s.execute("edit.redo", json!({})).unwrap();
        assert!(red(&s, id), "the red fill is still redoable");
    }
}
