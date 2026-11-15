//! History panel › History Options… (#2966), Photoshop's five checkboxes: Automatically Create
//! First Snapshot, Automatically Create New Snapshot When Saving, Allow Non-Linear History, Show
//! New Snapshot Dialog by Default and Make Layer Visibility Changes Undoable. The values are
//! preferences (`historyOptions.*`, also `prefs.set`), applied by [`Session::add_document`],
//! [`snapshot_on_save`], [`Session::apply_prefs`], [`Session::edit_visibility`] and the GUI's
//! camera button. Without params the command reports them; the GUI opens its dialog instead and
//! runs it with the dialog's values on OK.

use serde_json::{Map, Value};

use crate::commands::CommandSpec;
use crate::history_cmds::Snapshot;
use crate::prefs::HistoryOptions;
use crate::{EngineError, Result, Session};

const CMD: &str = "history.options";

/// The option keys, as in `prefs.get historyOptions` and the command's params, in the order of
/// Photoshop's dialog.
pub const OPTIONS: [&str; 5] = [
    "automaticallyCreateFirstSnapshot",
    "automaticallyCreateNewSnapshotWhenSaving",
    "allowNonLinearHistory",
    "showNewSnapshotDialogByDefault",
    "makeLayerVisibilityChangesUndoable",
];

/// The option named `key` (one of [`OPTIONS`]).
fn option_mut<'a>(o: &'a mut HistoryOptions, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "automaticallyCreateFirstSnapshot" => &mut o.automatically_create_first_snapshot,
        "automaticallyCreateNewSnapshotWhenSaving" => &mut o.automatically_create_new_snapshot_when_saving,
        "allowNonLinearHistory" => &mut o.allow_non_linear_history,
        "showNewSnapshotDialogByDefault" => &mut o.show_new_snapshot_dialog_by_default,
        "makeLayerVisibilityChangesUndoable" => &mut o.make_layer_visibility_changes_undoable,
        _ => return None,
    })
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The current History Options, keyed by [`OPTIONS`].
pub fn current(s: &Session) -> Value {
    let o = &s.prefs().history_options;
    serde_json::json!({
        "automaticallyCreateFirstSnapshot": o.automatically_create_first_snapshot,
        "automaticallyCreateNewSnapshotWhenSaving": o.automatically_create_new_snapshot_when_saving,
        "allowNonLinearHistory": o.allow_non_linear_history,
        "showNewSnapshotDialogByDefault": o.show_new_snapshot_dialog_by_default,
        "makeLayerVisibilityChangesUndoable": o.make_layer_visibility_changes_undoable,
    })
}

/// History Options › Show New Snapshot Dialog by Default: whether a plain click on the History
/// panel's camera button opens New Snapshot.
pub fn show_new_snapshot_dialog(s: &Session) -> bool {
    s.prefs().history_options.show_new_snapshot_dialog_by_default
}

/// History Options › Automatically Create New Snapshot When Saving: after the active document
/// was saved to its own file (not a copy or an export), add a snapshot of it named with the local
/// time of the save, as Photoshop does. Not a history state. Returns the new snapshot's id, or
/// `None` when the option is off, no document is active, or the document is unchanged since the
/// snapshot the previous save added (and that snapshot is still there), so saving a clean
/// document again doesn't add a duplicate row. Shells call it where they record a document save.
///
/// Memory: like every History panel snapshot, each one pins a copy of the document for the
/// session (only the tiles that differ from the document and the other snapshots cost memory).
/// [`crate::DocState::trim_history`] counts those tiles against the History & Cache budget by
/// dropping the oldest history states, but never drops a snapshot itself: many saves of a
/// changing document can hold more than the budget until snapshots are deleted or the document
/// is closed.
pub fn snapshot_on_save(s: &mut Session) -> Option<u64> {
    if !s.prefs().history_options.automatically_create_new_snapshot_when_saving {
        return None;
    }
    let st = s.active_mut()?;
    // Saved again without a change: the previous save's snapshot already shows this document.
    if let Some(prev) = st.save_snapshot
        && st.snapshots.iter().any(|snap| snap.id == prev && std::sync::Arc::ptr_eq(&snap.doc, &st.doc))
    {
        return None;
    }
    let name = save_snapshot_name().unwrap_or_else(|| crate::history_cmds::next_snapshot_name(st));
    let id = st.next_snapshot;
    st.next_snapshot = id.saturating_add(1);
    let (doc, layers) = (st.doc.clone(), st.layer_target());
    // Like the camera button: the document and its saved state are unchanged.
    st.snapshots.push(Snapshot { id, name, doc, layers });
    st.save_snapshot = Some(id);
    // The snapshot's tiles count against the history budget.
    st.trim_history();
    Some(id)
}

/// The local time as `HH:MM:SS`, the name of a snapshot made on save; `None` without a clock
/// (the web build), where the snapshot gets the camera button's "Snapshot N" instead.
fn save_snapshot_name() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(chrono::Local::now().format("%H:%M:%S").to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
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
        if option_mut(&mut HistoryOptions::default(), k).is_none() {
            return Err(bad(format!("unknown option `{k}` (expected {})", OPTIONS.join(", "))));
        }
        let b = v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false")))?;
        set.push((k.clone(), b));
    }
    if !set.is_empty() {
        s.edit_prefs(|prefs| {
            for (k, b) in &set {
                if let Some(o) = option_mut(&mut prefs.history_options, k) {
                    *o = *b;
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
        params: r##"{"automaticallyCreateFirstSnapshot":bool?,"automaticallyCreateNewSnapshotWhenSaving":bool?,"allowNonLinearHistory":bool?,"showNewSnapshotDialogByDefault":bool?,"makeLayerVisibilityChangesUndoable":bool?} → all five values (no params: report them). Automatically Create First Snapshot: a new or opened document starts with a snapshot of itself, named after it (default on). Automatically Create New Snapshot When Saving: each save of the document (not a copy) adds a snapshot named with the time of the save (default off). Allow Non-Linear History: an edit after stepping back keeps the later states and is appended at the end; deleting a state deletes only it (default off). Show New Snapshot Dialog by Default: a click on the History panel's camera button opens New Snapshot, not only ⌥-click (default off). Make Layer Visibility Changes Undoable: showing or hiding a layer is a history state (default on). Also `prefs.set historyOptions.*`"##,
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
        let defaults = json!({
            "automaticallyCreateFirstSnapshot": true,
            "automaticallyCreateNewSnapshotWhenSaving": false,
            "allowNonLinearHistory": false,
            "showNewSnapshotDialogByDefault": false,
            "makeLayerVisibilityChangesUndoable": true,
        });
        assert_eq!(s.execute(CMD, json!({})).unwrap(), defaults);
        assert_eq!(s.execute("prefs.get", json!({"path": "historyOptions"})).unwrap(), current(&s));
        let flipped: Map<String, Value> = defaults.as_object().unwrap().iter().map(|(k, v)| (k.clone(), json!(!v.as_bool().unwrap()))).collect();
        let r = s.execute(CMD, Value::Object(flipped.clone())).unwrap();
        assert_eq!(r, Value::Object(flipped));
        for key in OPTIONS {
            assert_eq!(s.execute("prefs.get", json!({"path": format!("historyOptions.{key}")})).unwrap(), r[key], "{key}");
        }
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
            json!({"automaticallyCreateFirstSnapshot": "true"}),
            json!({"automaticallyCreateNewSnapshotWhenSaving": 0}),
            json!({"showNewSnapshotDialogByDefault": [true]}),
            json!({"showNewSnapshotDialogByDefault": true, "showNewSnapshotDialog": true}),
            json!({"AutomaticallyCreateFirstSnapshot": false}),
            json!([true]),
            json!("x"),
            json!(3),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
            assert_eq!(current(&s), before, "{p} changed nothing");
        }
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.allowNonLinearHistory", "value": "on"})).is_err());
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.automaticallyCreateFirstSnapshot", "value": 1})).is_err());
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.showNewSnapshotDialogByDefault", "value": null})).is_err());
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

    fn snapshot_names(s: &Session) -> Vec<String> {
        s.active().unwrap().snapshots.iter().map(|snap| snap.name.clone()).collect()
    }

    /// PNG bytes of a small image, to open.
    fn png() -> Vec<u8> {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        crate::file_cmds::encode(&s.active().unwrap().doc, "a.png", None::<f64>).unwrap().0
    }

    /// Automatically Create First Snapshot (checked against Photoshop): on (the default), a new or
    /// opened document starts with a snapshot named after it; off, it starts with none and the
    /// camera's first snapshot is still "Snapshot 1".
    #[test]
    fn first_snapshot_follows_the_option_on_new_and_open() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let name = s.active().unwrap().doc.name.clone();
        assert_eq!(snapshot_names(&s), [name]);
        crate::file_cmds::open_bytes_as(&mut s, "photo.png", &png(), None, None).unwrap();
        assert_eq!(snapshot_names(&s), ["photo.png"], "named after the file, as Photoshop does");

        s.execute(CMD, json!({"automaticallyCreateFirstSnapshot": false})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(snapshot_names(&s).is_empty());
        assert_eq!(entries(&s).len(), 1, "the opening state is still there");
        crate::file_cmds::open_bytes_as(&mut s, "photo.png", &png(), None, None).unwrap();
        assert!(snapshot_names(&s).is_empty());
        assert_eq!(s.execute("history.newSnapshot", json!({})).unwrap(), json!({"snapshot": 1, "name": "Snapshot 1"}));
        assert_eq!(crate::inspect::document(s.active().unwrap())["snapshots"], json!([{"id": 1, "name": "Snapshot 1"}]));
        // Documents opened earlier keep theirs: the option applies when a document opens.
        s.set_active(0);
        assert_eq!(snapshot_names(&s).len(), 1);
    }

    /// Automatically Create New Snapshot When Saving: off (the default), a save adds nothing; on,
    /// each save adds a snapshot of the document as saved, named with the time, and not a
    /// history state.
    #[test]
    fn a_save_adds_a_snapshot_only_with_the_option() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(snapshot_on_save(&mut s), None);
        assert_eq!(snapshot_names(&s).len(), 1, "only the first snapshot");

        s.execute(CMD, json!({"automaticallyCreateNewSnapshotWhenSaving": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.active_mut().unwrap().saved_to("saved.psd".into());
        let states = entries(&s);
        let id = snapshot_on_save(&mut s).unwrap();
        let st = s.active().unwrap();
        let snap = st.snapshots.last().unwrap();
        assert_eq!(snap.id, id);
        assert!(std::sync::Arc::ptr_eq(&snap.doc, &st.doc), "the document as saved");
        // HH:MM:SS, the local time of the save.
        let parts: Vec<&str> = snap.name.split(':').collect();
        assert!(parts.len() == 3 && parts.iter().all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit())), "{}", snap.name);
        assert_eq!(entries(&s), states, "not a history state");
        assert!(!s.active().unwrap().is_dirty(), "the saved document stays clean");
        // Each save adds one; selecting it brings that save back.
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let second = snapshot_on_save(&mut s).unwrap();
        assert_eq!(snapshot_names(&s).len(), 3);
        s.execute("history.selectSnapshot", json!({"snapshot": id})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers[0].surface().unwrap().rgba(1, 1), [1.0, 0.0, 0.0, 1.0]);
        assert!(second > id);
        // No document: nothing to do, no panic.
        let mut empty = Session::new();
        empty.execute(CMD, json!({"automaticallyCreateNewSnapshotWhenSaving": true})).unwrap();
        assert_eq!(snapshot_on_save(&mut empty), None);
    }

    /// Saving an unchanged document again (⌘S on a clean document) adds no duplicate row; a
    /// change, or deleting the previous save's snapshot, makes the next save add one again.
    #[test]
    fn saving_an_unchanged_document_adds_no_duplicate_snapshot() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute(CMD, json!({"automaticallyCreateNewSnapshotWhenSaving": true})).unwrap();
        let first = snapshot_on_save(&mut s).unwrap();
        assert_eq!(snapshot_on_save(&mut s), None, "unchanged since the last save snapshot");
        assert_eq!(snapshot_names(&s).len(), 2, "the first snapshot and one save snapshot");
        // A view-only change (no document change) doesn't count either.
        s.active_mut().unwrap().revision += 1;
        assert_eq!(snapshot_on_save(&mut s), None);
        // An edit does.
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let second = snapshot_on_save(&mut s).unwrap();
        assert!(second > first);
        assert_eq!(snapshot_on_save(&mut s), None);
        // The previous save's snapshot deleted: the next save shows the document again.
        s.execute("history.deleteSnapshot", json!({"snapshot": second})).unwrap();
        assert!(snapshot_on_save(&mut s).is_some());
        assert_eq!(snapshot_names(&s).len(), 3);
    }

    #[test]
    fn show_new_snapshot_dialog_reads_the_option() {
        let mut s = Session::new();
        assert!(!show_new_snapshot_dialog(&s));
        s.execute("prefs.set", json!({"path": "historyOptions.showNewSnapshotDialogByDefault", "value": true})).unwrap();
        assert!(show_new_snapshot_dialog(&s));
        assert_eq!(crate::inspect::session(&s)["historyOptions"]["showNewSnapshotDialogByDefault"], json!(true));
    }

    /// The first snapshot and a save snapshot are recorded steps like any other: with Make Layer
    /// Visibility Changes Undoable off, selecting one that shows a layer the eye hid, then Undo,
    /// hides the layer again, and Redo shows it (the step changed it, #3535's rule).
    #[test]
    fn selecting_a_snapshot_undoes_its_own_visibility_change() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        // Red, not the white document as opened (`red` only reads the red channel).
        let red = |s: &Session| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().pixel(1, 1)[1] < 0.5;
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false, "automaticallyCreateNewSnapshotWhenSaving": true})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.active_mut().unwrap().saved_to("saved.psd".into());
        let saved = snapshot_on_save(&mut s).unwrap();
        // The eye hides the layer: no state.
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        for (snap, is_red) in [(0, false), (saved, true)] {
            s.execute("history.selectSnapshot", json!({"snapshot": snap})).unwrap();
            assert!(visible(&s, id) && red(&s) == is_red, "snapshot {snap} shows the layer as it was");
            s.execute("edit.undo", json!({})).unwrap();
            assert!(!visible(&s, id) && red(&s), "undoing snapshot {snap} hides the layer again");
            s.execute("edit.redo", json!({})).unwrap();
            assert!(visible(&s, id) && red(&s) == is_red, "redoing snapshot {snap} shows it");
            s.execute("edit.undo", json!({})).unwrap();
        }
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
