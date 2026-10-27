//! Arrow-key nudges that follow each other are one History state (#2259).
//!
//! Tapping or holding an arrow key nudges the selected layers (or the selection outline) one
//! pixel at a time. One state per pixel buries the History panel and makes one Undo take back a
//! single pixel, so a run of nudges of the same thing shares one `coalesce` key (the engine folds
//! edits with the same key into one step, see `Session::execute`) and Undo takes back the run.
//!
//! A run lasts while each nudge comes within the Tools › Nudge Bundle Pause of the one before it
//! (longer than a key's auto-repeat delay, so a held key is one run) and nothing else happened
//! to the document in between: another edit, an Undo or Redo, a different layer selected or a
//! save each end it. Tools › Bundle Arrow-Key Nudges off gives every press its own state.
//!
//! [`NudgeBundle`] only decides which run a nudge belongs to; it reads the clock from the key
//! handlers ([`NudgeBundle::now`]) and the document from the caller, so it is tested without egui.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use photocraft_doc::DocId;
use photocraft_engine::DocState;

/// What a run of nudges needs to find unchanged to go on: the document, and its revisions (every
/// edit, Undo, Redo and layer selection moves `revision`, a save moves `saved_revision`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    doc: DocId,
    revision: u64,
    saved_revision: u64,
}

impl Seen {
    pub(crate) fn of(st: &DocState) -> Self {
        Self { doc: st.doc.id, revision: st.revision, saved_revision: st.saved_revision }
    }
}

/// The latest nudge: the run it belongs to, what it nudged, when, and the document it left.
#[derive(Clone, Debug)]
struct Last {
    run: u64,
    what: String,
    at: f64,
    left: Seen,
}

/// The run the arrow-key nudges belong to (one per app; a nudge always goes to the active
/// document, whose state it checks).
#[derive(Clone, Debug, Default)]
pub struct NudgeBundle {
    /// egui time (seconds) of the key press being handled; the arrow-key handlers set it.
    pub(crate) now: f64,
    /// Runs started so far; a run's number is part of its `coalesce` key.
    started: u64,
    last: Option<Last>,
}

impl NudgeBundle {
    /// The run a nudge of `what` now joins: the latest one when it was `what` too, came at most
    /// `pause` seconds ago and left the document as `seen`; else a new run.
    pub(crate) fn join(&mut self, what: &str, seen: Seen, pause: f64) -> u64 {
        if let Some(l) = &self.last
            && l.what == what
            && l.left == seen
            && self.now >= l.at
            && self.now - l.at <= pause
        {
            return l.run;
        }
        self.started += 1;
        self.started
    }

    /// The nudge of `run` ran and left the document as `left`.
    pub(crate) fn record(&mut self, run: u64, what: &str, left: Seen) {
        self.last = Some(Last { run, what: what.to_string(), at: self.now, left });
    }

    /// The next nudge starts a new run.
    pub(crate) fn reset(&mut self) {
        self.last = None;
    }
}

/// The longest pause (seconds) between two nudges of one run, or `None` when nudges aren't
/// bundled (Tools › Bundle Arrow-Key Nudges).
pub(crate) fn pause(app: &PhotocraftApp) -> Option<f64> {
    let tools = &app.session.prefs().tools;
    tools.bundle_nudges.then(|| f64::from(tools.nudge_bundle_pause_ms) / 1000.0)
}

/// Run the nudge command `id` (`what` names the thing it nudges: "layers", "selection") as part
/// of the current run of nudges, or on its own when nudges aren't bundled.
pub(crate) fn run(app: &mut PhotocraftApp, what: &str, id: &str, mut params: Value) -> Result<Value, String> {
    let Some(pause) = pause(app) else {
        app.nudge_bundle.reset();
        return app.run(id, params);
    };
    let Some(st) = app.session.active() else { return app.run(id, params) };
    // The same keys nudge whatever is selected: another selection is another run.
    let what = format!("{what}:{:?}:{:?}", st.active_layer, st.selected_layers);
    let run = app.nudge_bundle.join(&what, Seen::of(st), pause);
    if let Some(object) = params.as_object_mut() {
        object.insert("coalesce".into(), json!(format!("nudge:{run}")));
    }
    let result = app.run(id, params);
    match (&result, app.session.active()) {
        (Ok(_), Some(st)) => app.nudge_bundle.record(run, &what, Seen::of(st)),
        _ => app.nudge_bundle.reset(),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(revision: u64) -> Seen {
        Seen { doc: DocId(1), revision, saved_revision: 0 }
    }

    fn at(b: &mut NudgeBundle, t: f64) {
        b.now = t;
    }

    #[test]
    fn nudges_within_the_pause_share_a_run() {
        let mut b = NudgeBundle::default();
        at(&mut b, 10.0);
        let first = b.join("layers", seen(1), 1.0);
        b.record(first, "layers", seen(2));
        at(&mut b, 10.9);
        assert_eq!(b.join("layers", seen(2), 1.0), first, "0.9 s later is the same run");
        b.record(first, "layers", seen(3));
        // The pause counts from the latest nudge, so a long run goes on while the keys do.
        at(&mut b, 11.8);
        assert_eq!(b.join("layers", seen(3), 1.0), first);
        b.record(first, "layers", seen(4));
        at(&mut b, 12.9);
        assert_ne!(b.join("layers", seen(4), 1.0), first, "1.1 s of quiet ends the run");
    }

    #[test]
    fn anything_else_ends_the_run() {
        let mut b = NudgeBundle::default();
        at(&mut b, 1.0);
        let run = b.join("layers", seen(1), 1.0);
        b.record(run, "layers", seen(2));
        let new = |b: &mut NudgeBundle, what: &str, s: Seen| b.join(what, s, 1.0) != run;
        assert!(new(&mut b, "layers", seen(3)), "an edit, Undo or Redo moved the revision");
        assert!(new(&mut b, "selection", seen(2)), "another thing is nudged");
        assert!(new(&mut b, "layers", Seen { doc: DocId(2), ..seen(2) }), "another document");
        assert!(new(&mut b, "layers", Seen { saved_revision: 2, ..seen(2) }), "a save keeps the saved state a step of its own");
        assert!(!new(&mut b, "layers", seen(2)), "nothing happened: the same run");
        // The clock never runs backwards in a real session; if it did, that is not a run.
        at(&mut b, 0.5);
        assert!(new(&mut b, "layers", seen(2)));
    }

    #[test]
    fn reset_ends_the_run() {
        let mut b = NudgeBundle::default();
        let run = b.join("layers", seen(1), 1.0);
        b.record(run, "layers", seen(2));
        b.reset();
        assert_ne!(b.join("layers", seen(2), 1.0), run);
    }
}
