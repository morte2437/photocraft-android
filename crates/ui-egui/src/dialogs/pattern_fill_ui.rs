//! Photoshop's Pattern Fill dialog (#2908): Layer › New Fill Layer › Pattern… and double-clicking a
//! Pattern Fill layer's thumbnail open it. Pattern (swatch picker), Snap to Origin, Angle, Scale
//! and Link with Layer, previewed live on the canvas from a document snapshot. OK runs one engine
//! command (`layer.newFillLayer.pattern` for a new layer, `layer.fill.pattern.set` for an existing
//! one), so it is one History step; Cancel changes nothing. With Link with Layer on, dragging on
//! the canvas moves the pattern. The fields are the command's params, so `ui.dialog.set` /
//! `ui.dialog.confirm` drive it like any dialog.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use photocraft_engine::pattern_fill_cmds as cmds;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const MARK: &str = "__patternFill";
/// The fields that are command params (besides the target in `__params`).
const KEYS: [&str; 5] = ["pattern", "scale", "angle", "link", "phase"];
/// The gesture's baseline and pointer, kept out of the command's params and `unchanged`.
const DRAG_KEY: &str = "__patternDrag";
/// The active layer a new fill would be inserted above, frozen when its dialog opens.
const INSERT_KEY: &str = "__patternInsert";
/// What the engine accepts for a phase coordinate (`pattern_fill_cmds`).
const MAX_PHASE: f64 = 1.0e7;

pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    dialog: u64,
    params: Value,
    shown: Arc<Document>,
    key: u64,
}

/// Is this dialog the Pattern Fill dialog?
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(MARK)
}

fn fields(command: &str, target: Value, values: Value) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!("Pattern Fill"));
    f.insert("__params".into(), target);
    for k in KEYS {
        if let Some(v) = values.get(k) {
            f.insert(k.into(), v.clone());
        }
    }
    f.insert("__orig".into(), values);
    f
}

/// Edit the active Pattern Fill layer (its thumbnail, Layer › Layer Content Options…).
pub(crate) fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let LayerContent::Fill(Fill::Pattern { scale, id, angle, link, phase, .. }) = &st.doc.layer(layer)?.content else { return None };
    let values = json!({"pattern": id, "scale": scale * 100.0, "angle": angle, "link": link, "phase": [phase.0, phase.1]});
    let f = fields(cmds::SET, json!({"layer": layer.0, "document": st.doc.id.0}), values);
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// Layer › New Fill Layer › Pattern…: the dialog in front of creating the layer, starting from
/// the Patterns panel's pattern at 100 %, 0° and linked, as in Photoshop.
pub(crate) fn open_new(app: &mut PhotocraftApp) -> Result<u64, String> {
    if let Some(Err(why)) = photocraft_engine::commands::find(cmds::CREATE).map(|c| (c.enabled)(&app.session)) {
        return Err(why);
    }
    let st = app.session.active().ok_or("no document open")?;
    let pattern = photocraft_engine::pattern_cmds::resolve_id(&app.session, "").ok_or("no patterns")?;
    let values = json!({"pattern": pattern, "scale": 100.0, "angle": 0.0, "link": true, "phase": [0.0, 0.0]});
    let mut f = fields(cmds::CREATE, json!({"document": st.doc.id.0}), values);
    // The command inserts above the active layer. Its id is frozen here (null with none, which
    // the command allows) so a later layer change cannot silently move the insertion point.
    f.insert(INSERT_KEY.into(), json!(st.active_layer.map(|id| id.0)));
    Ok(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// The command and its params: the target plus the dialog's values.
fn command(app: &PhotocraftApp, f: &Map<String, Value>) -> Option<(&'static str, Value)> {
    let cmd = match f.get("__command").and_then(Value::as_str)? {
        cmds::SET => cmds::SET,
        cmds::CREATE => cmds::CREATE,
        _ => return None,
    };
    let mut params = f.get("__params").filter(|p| p.is_object()).cloned()?;
    // An edit sends only what changed: an imported scale or angle outside the dialog's range must
    // not make an unrelated phase edit fail the strict `SET`. A new layer still sends them all.
    for k in KEYS {
        if (cmd == cmds::CREATE || f.get(k) != f.get("__orig").and_then(|o| o.get(k)))
            && let Some(v) = f.get(k)
        {
            params[k] = v.clone();
        }
    }
    // A new fill layer is masked by the path selected in the Paths panel, as when it runs.
    let params = if cmd == cmds::CREATE {
        params.as_object_mut()?.remove("document");
        crate::vector_ui::with_active_path(app, cmd, params)
    } else {
        params
    };
    Some((cmd, params))
}

/// An edit dialog whose values are still the layer's.
fn unchanged(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(cmds::SET) && KEYS.iter().all(|k| f.get(*k) == f.get("__orig").and_then(|o| o.get(*k)))
}

/// The document the dialog targets, if it is still the active one.
fn target_is_active(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    let target = f.get("__params").and_then(|p| p.get("document")).and_then(Value::as_u64);
    app.session.active().is_some_and(|st| target == Some(st.doc.id.0))
}

/// The phase the dialog shows, when it is the finite pair the engine takes.
fn shown_phase(f: &Map<String, Value>) -> Option<[f64; 2]> {
    let [x, y] = finite_pair(f.get("phase"))?;
    (x.abs() <= MAX_PHASE && y.abs() <= MAX_PHASE).then_some([x, y])
}

/// Reads a finite `[x, y]`. The drag's private travel may exceed the engine's phase bound; only
/// what it publishes is bounded.
fn finite_pair(v: Option<&Value>) -> Option<[f64; 2]> {
    let a = v?.as_array().filter(|a| a.len() == 2)?;
    let x = a.first()?.as_f64()?;
    let y = a.get(1)?.as_f64()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// A new fill inserts above this active layer, frozen when the dialog opened.
fn insertion_unchanged(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    let Some(st) = app.session.active() else { return false };
    let Some(stored) = f.get(INSERT_KEY) else { return false };
    let frozen = match stored {
        Value::Null => None,
        v => match v.as_u64() {
            Some(id) => Some(id),
            None => return false,
        },
    };
    st.active_layer.map(|id| id.0) == frozen
}

/// Whether this dialog creates a new fill layer rather than editing one.
fn is_create(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(cmds::CREATE)
}

/// The dialog's target document and insertion layer are still the ones it opened on.
fn context_ok(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    target_is_active(app, f) && (!is_create(f) || insertion_unchanged(app, f))
}

/// What a linked canvas drag moves: the Pattern Fill layer being edited, or the layer a new fill
/// would be inserted above. Captured on the press so a changed target ends the gesture instead of
/// moving a different layer, and so automation cannot redirect a held drag to a new insertion
/// point. The pattern is part of it: choosing another one starts a fresh baseline. `owner` is the
/// canvas the press landed on (see [`DragTarget::owned_by`]), so a second view of the same
/// document neither moves nor ends a gesture it does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DragTarget {
    document: u64,
    command: &'static str,
    pattern: Option<String>,
    layer: Option<u64>,
    owner: Option<u64>,
}

impl DragTarget {
    /// The same target, held by the canvas `owner` identifies.
    pub(crate) fn owned_by(mut self, owner: u64) -> Self {
        self.owner = Some(owner);
        self
    }

    /// Whether a stored gesture `state` was started on this target by this canvas.
    fn matches(&self, state: &Value) -> bool {
        state.get("document").and_then(Value::as_u64) == Some(self.document)
            && state.get("command").and_then(Value::as_str).is_some_and(|c| c == self.command)
            && state.get("pattern").and_then(Value::as_str) == self.pattern.as_deref()
            && state.get("layer").and_then(Value::as_u64) == self.layer
            && state.get("owner").and_then(Value::as_u64) == self.owner
    }

    /// Records this target and its canvas in a gesture's stored state.
    fn store(&self, state: &mut Map<String, Value>) {
        state.insert("document".into(), json!(self.document));
        state.insert("command".into(), json!(self.command));
        state.insert("pattern".into(), json!(self.pattern.as_deref()));
        state.insert("layer".into(), json!(self.layer));
        state.insert("owner".into(), json!(self.owner));
    }
}

/// The canvas holding this dialog's stored gesture: `None` when no gesture is stored, and
/// `Some(None)` for one stored without a canvas (driven directly, not from a canvas).
pub(crate) fn drag_owner(f: &Map<String, Value>) -> Option<Option<u64>> {
    let state = f.get(DRAG_KEY)?;
    Some(state.get("owner").and_then(Value::as_u64))
}

/// The target a canvas drag moves, when this is a linked Pattern Fill dialog that can move one:
/// the document is active, the phase is valid, and (for an edit) the layer still exists, is a
/// Pattern Fill and is not pixel-locked, directly or through a group. A new fill needs no existing
/// fill layer; it may be inserted above a locked one, as the command allows, but its insertion
/// layer must still be the one the dialog opened on.
pub(crate) fn drag_target(app: &PhotocraftApp, f: &Map<String, Value>) -> Option<DragTarget> {
    let command = match f.get("__command").and_then(Value::as_str)? {
        cmds::SET => cmds::SET,
        cmds::CREATE => cmds::CREATE,
        _ => return None,
    };
    if !context_ok(app, f) || shown_phase(f).is_none() {
        return None;
    }
    let st = app.session.active()?;
    let document = f.get("__params").and_then(|p| p.get("document")).and_then(Value::as_u64)?;
    let link = f.get("link").and_then(Value::as_bool);
    let pattern = f.get("pattern").and_then(Value::as_str).map(str::to_string);
    match command {
        cmds::SET => {
            let id = f.get("__params").and_then(|p| p.get("layer")).and_then(Value::as_u64)?;
            let layer = st.doc.layer(LayerId(id))?;
            let LayerContent::Fill(Fill::Pattern { link: layer_link, .. }) = &layer.content else { return None };
            if !link.unwrap_or(*layer_link) {
                return None;
            }
            let locks = st.doc.effective_locks(LayerId(id));
            if locks.pixels || locks.all {
                return None;
            }
            Some(DragTarget { document, command, pattern, layer: Some(id), owner: None })
        }
        _ => {
            if link == Some(false) {
                return None;
            }
            Some(DragTarget { document, command, pattern, layer: st.active_layer.map(|id| id.0), owner: None })
        }
    }
}

/// Ends a canvas drag's stored state. The canvas calls this whenever the gesture is over (its
/// press was released, another dialog or popup took it, its target changed, the pattern was
/// unlinked or replaced, or Space/the middle button/temporary zoom took the pointer).
pub(crate) fn end_phase_drag(f: &mut Map<String, Value>) {
    f.remove(DRAG_KEY);
}

/// Ends the stored gesture unless another canvas holds it: a release or `Gone` reported by one
/// canvas says nothing about a press held in another window.
fn end_own_drag(f: &mut Map<String, Value>, target: &DragTarget) {
    if drag_owner(f).is_none_or(|owner| owner.is_none() || owner == target.owner) {
        end_phase_drag(f);
    }
}

/// The private state of an owned drag: the phase it started from, the travel at which that
/// baseline was captured, the phase last published, the pointer's last screen position, and the
/// travel since the press.
struct DragState {
    base: [f64; 2],
    total: [f64; 2],
    shown: [f64; 2],
    at: [f64; 2],
    travel: [f64; 2],
}

/// The stored gesture, when it belongs to `target` and every field is a finite pair.
fn drag_state(f: &Map<String, Value>, target: &DragTarget) -> Option<DragState> {
    let state = f.get(DRAG_KEY).filter(|s| target.matches(s))?;
    Some(DragState {
        base: finite_pair(state.get("base"))?,
        total: finite_pair(state.get("total"))?,
        shown: finite_pair(state.get("shown"))?,
        at: finite_pair(state.get("at"))?,
        travel: finite_pair(state.get("travel"))?,
    })
}

/// A linked Pattern Fill drag: `start` records the phase shown on the press, and `total` is the
/// pointer's travel since the press, in document pixels. Each frame sets the phase to the baseline
/// plus the travel since that baseline, so repeated moves neither accumulate nor drift; a drag
/// back to the origin restores the baseline exactly. A phase changed by `ui.dialog.set` (or Snap
/// to Origin) under the press is kept and the drag continues from it. Coordinates are clamped to
/// what the engine accepts, and a non-finite total or malformed stored state ends the gesture
/// rather than panicking. `at` is the pointer's screen position, so a release in a later frame can
/// apply the motion since the last preview.
pub(crate) fn drag_phase(f: &mut Map<String, Value>, start: bool, total: [f64; 2], at: [f32; 2], target: &DragTarget) {
    let at = [f64::from(at[0]), f64::from(at[1])];
    if !total.iter().all(|v| v.is_finite()) || !at.iter().all(|v| v.is_finite()) {
        end_phase_drag(f);
        return;
    }
    let Some(current) = shown_phase(f) else {
        end_phase_drag(f);
        return;
    };
    let (base, base_total) = if start {
        (current, total)
    } else {
        let Some(state) = drag_state(f, target) else {
            // A press that began elsewhere (on a dialog, before this dialog was on top, or while
            // the pattern was unlinked) never starts in the middle of its hold.
            end_phase_drag(f);
            return;
        };
        if current == state.shown {
            (state.base, state.total)
        } else {
            // Keep the externally set phase and move on from it, not from a stale baseline.
            (current, total)
        }
    };
    let phase = [(base[0] + (total[0] - base_total[0])).clamp(-MAX_PHASE, MAX_PHASE), (base[1] + (total[1] - base_total[1])).clamp(-MAX_PHASE, MAX_PHASE)];
    if !phase.iter().all(|v| v.is_finite()) {
        end_phase_drag(f);
        return;
    }
    if current != phase {
        f.insert("phase".into(), json!([phase[0], phase[1]]));
    }
    let mut state = Map::new();
    state.insert("base".into(), json!([base[0], base[1]]));
    state.insert("total".into(), json!([base_total[0], base_total[1]]));
    state.insert("shown".into(), json!([phase[0], phase[1]]));
    state.insert("at".into(), json!([at[0], at[1]]));
    state.insert("travel".into(), json!([total[0], total[1]]));
    target.store(&mut state);
    f.insert(DRAG_KEY.into(), Value::Object(state));
}

/// The pointer screen position the drag last previewed at, if the stored state belongs to `target`.
pub(crate) fn drag_last(f: &Map<String, Value>, target: &DragTarget) -> Option<[f32; 2]> {
    let at = drag_state(f, target)?.at;
    let to_f32 = |v: f64| (v.abs() <= f64::from(f32::MAX)).then_some(v as f32);
    Some([to_f32(at[0])?, to_f32(at[1])?])
}

/// One primary pointer event of a frame, in the order egui delivered it.
pub(crate) enum CanvasEvent {
    Press(egui::Pos2),
    Move(egui::Pos2),
    Release(egui::Pos2),
    Gone,
}

/// Applies a frame's primary pointer events to a linked Pattern Fill drag in order. `free` says
/// whether a position is on the free canvas (not under a dialog or popup), and `delta` converts the
/// screen movement between two positions into document pixels.
///
/// A press starts a gesture only on the free canvas; a move previews only while it stays there; a
/// release there applies the motion since the last preview and always ends the gesture. `Gone`
/// ends it too, but neither ends a gesture another canvas holds. Order keeps each gesture's travel
/// its own: a press and release in one frame applies exactly that click's movement, a tap moves
/// nothing, and a release followed by a new press starts the new gesture instead of joining the
/// two.
pub(crate) fn canvas_events(
    f: &mut Map<String, Value>,
    target: &DragTarget,
    events: impl IntoIterator<Item = CanvasEvent>,
    free: impl Fn(egui::Pos2) -> bool,
    delta: impl Fn(egui::Pos2, egui::Pos2) -> [f64; 2],
) {
    for event in events {
        match event {
            CanvasEvent::Press(p) => {
                if free(p) {
                    drag_phase(f, true, [0.0, 0.0], [p.x, p.y], target);
                } else {
                    end_phase_drag(f);
                }
            }
            CanvasEvent::Move(p) => {
                // Over a dialog or off the canvas the drag waits at its last preview; moving back
                // onto the free canvas continues from it.
                if !free(p) {
                    continue;
                }
                let Some(state) = drag_state(f, target) else { continue };
                let Some(last) = drag_last(f, target) else {
                    end_own_drag(f, target);
                    continue;
                };
                let d = delta(egui::pos2(last[0], last[1]), p);
                let total = [state.travel[0] + d[0], state.travel[1] + d[1]];
                drag_phase(f, false, total, [p.x, p.y], target);
            }
            CanvasEvent::Release(p) => {
                if !free(p) {
                    end_own_drag(f, target);
                    continue;
                }
                let Some(last) = drag_last(f, target) else {
                    end_own_drag(f, target);
                    continue;
                };
                let d = delta(egui::pos2(last[0], last[1]), p);
                release_phase_drag(f, d, target);
            }
            CanvasEvent::Gone => end_own_drag(f, target),
        }
    }
}

/// Applies the pointer's motion on the frame its press is released, then ends the gesture. The
/// release frame has no press origin, so this is the last chance to show the pattern where the
/// pointer was let go instead of where it last moved. `delta` is in document pixels. The phase is
/// recomputed from the gesture's baseline and total travel, so a release just inside a bound that
/// the travel already passed stays on the bound.
pub(crate) fn release_phase_drag(f: &mut Map<String, Value>, delta: [f64; 2], target: &DragTarget) {
    let Some(current) = shown_phase(f) else {
        end_phase_drag(f);
        return;
    };
    let Some(state) = drag_state(f, target) else {
        end_phase_drag(f);
        return;
    };
    if !delta.iter().all(|v| v.is_finite()) {
        end_phase_drag(f);
        return;
    }
    let total = [state.travel[0] + delta[0], state.travel[1] + delta[1]];
    if !total.iter().all(|v| v.is_finite()) {
        end_phase_drag(f);
        return;
    }
    let (base, base_total) = if current == state.shown {
        (state.base, state.total)
    } else {
        // The phase changed under the press: keep it and apply the release motion from there.
        (current, state.travel)
    };
    let phase = [(base[0] + (total[0] - base_total[0])).clamp(-MAX_PHASE, MAX_PHASE), (base[1] + (total[1] - base_total[1])).clamp(-MAX_PHASE, MAX_PHASE)];
    if !phase.iter().all(|v| v.is_finite()) {
        end_phase_drag(f);
        return;
    }
    if current != phase {
        f.insert("phase".into(), json!([phase[0], phase[1]]));
    }
    end_phase_drag(f);
}

/// OK: one engine command. A stale document or insertion layer never receives the edit.
pub(crate) fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if !target_is_active(app, f) {
        return Err("the target document is no longer active".into());
    }
    if is_create(f) && !insertion_unchanged(app, f) {
        return Err("the layer the new fill would go above changed; reopen Pattern Fill".into());
    }
    if unchanged(f) {
        return Ok(json!({"unchanged": true}));
    }
    let (cmd, params) = command(app, f).ok_or("invalid Pattern Fill dialog")?;
    app.run(cmd, params)
}

/// The canvas preview while the dialog is open: the document as OK would leave it.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let dialog = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields));
    let Some(d) = dialog else {
        app.pattern_fill_preview = None;
        return None;
    };
    let st = app.session.documents().get(idx)?;
    if !context_ok(app, &d.fields) || app.session.active_index() != Some(idx) || unchanged(&d.fields) {
        return None;
    }
    let (cmd, params) = command(app, &d.fields)?;
    if let Some(p) = &app.pattern_fill_preview
        && p.document == st.doc.id
        && p.revision == st.revision
        && p.dialog == d.id
        && p.params == params
    {
        return Some((p.shown.clone(), p.key));
    }
    // Out-of-range values typed by automation show the committed document; OK reports them.
    let doc = cmds::preview(&app.session, cmd, &params).ok()?;
    // Include the dialog identity: cancelling and reopening at the same document revision must
    // not reuse an old preview's pixels in the canvas caches.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, d.id, params.to_string()).hash(&mut hash);
    let key = (1 << 47) | (hash.finish() & ((1 << 36) - 1));
    let (document, revision, dialog) = (st.doc.id, st.revision, d.id);
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.pattern_fill_preview = Some(Preview { document, revision, dialog, params, shown: shown.clone(), key });
    Some((shown, key))
}

fn num(f: &Map<String, Value>, k: &str, d: f64) -> f32 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d) as f32
}

/// The dialog body, in Photoshop's layout: the pattern with Snap to Origin beside it, then
/// Angle, Scale and Link with Layer.
pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal(|ui| {
        let pat = f.get("pattern").and_then(Value::as_str).unwrap_or_default().to_string();
        if app.session.patterns.items.is_empty() {
            ui.label(egui::RichText::new(tl!("No patterns")).color(Tokens::get(ui.ctx()).text_faint));
        } else if let Some(id) = crate::preset_panels::pattern_picker(app, ui, &pat)
            && f.get("pattern").and_then(Value::as_str) != Some(id.as_str())
        {
            f.insert("pattern".into(), json!(id));
            end_phase_drag(f);
        }
        ui.add_space(8.0);
        // Photoshop: Snap to Origin puts the pattern's origin back at the layer's (or canvas's).
        if crate::widgets::secondary_button(ui, tl!("Snap to Origin"), 110.0).clicked() {
            f.insert("phase".into(), json!([0.0, 0.0]));
            end_phase_drag(f);
        }
    });
    let mut angle = num(f, "angle", 0.0);
    if crate::widgets::angle_dial_row(ui, tl!("Angle:"), &mut angle, -180.0..=180.0).changed() {
        f.insert("angle".into(), json!(angle.clamp(-180.0, 180.0)));
    }
    let (min, max) = cmds::SCALE_RANGE;
    let mut scale = num(f, "scale", 100.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Scale:")).color(Tokens::get(ui.ctx()).text_dim));
        if crate::widgets::value_field(ui, &mut scale, min as f32..=max as f32, "%", 70.0).accessible_name(tl!("Scale:")).changed() {
            f.insert("scale".into(), json!(scale.clamp(min as f32, max as f32)));
        }
    });
    let mut link = f.get("link").and_then(Value::as_bool).unwrap_or(true);
    if crate::widgets::checkbox(ui, &mut link, tl!("Link with Layer")).changed() {
        f.insert("link".into(), json!(link));
        end_phase_drag(f);
    }
}

#[cfg(test)]
mod tests;
