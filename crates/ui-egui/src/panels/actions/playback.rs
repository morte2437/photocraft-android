//! Playback Options: the Actions panel menu's dialog, and the Step by Step and Pause For pacing of a
//! person's Play.
//!
//! A paced Play is still one playback: after each step [`super::play_owned`] parks the rest of the
//! action like a step dialog does (a [`super::PausedPlayback`] token holding the suspended history and
//! recording), and [`tick`] resumes it once the wait is over, so the steps share one history state
//! and the canvas redraws between them. Esc, the panel's Stop button, an edit of the action list or a
//! new recording end it there, keeping the steps already applied. Programmatic playback never waits.

use serde_json::{Map, Value, json};
use web_time::{Duration, Instant};

use photocraft_engine::prefs::{Actions, PlaybackPerformance};

use crate::PhotocraftApp;

/// The marker a paced playback leaves where a step dialog's id would be (see [`super::waiting_dialog`]).
pub(crate) const PLAYBACK_TIMER: &str = "__playbackTimer";

/// The field marking a Playback Options dialog.
const PLAYBACK_OPTIONS: &str = "__actionsPlaybackOptions";

/// Step by Step's gap between two steps: the reference application takes about 1.15 s per step
/// with nothing to redraw (measured 2026-11-05), and adds Pause For's seconds to it.
pub(crate) const STEP_GAP: Duration = Duration::from_secs(1);

/// The paced playback waiting for its next step: its [`super::PausedPlaybacks`] token, when it goes
/// on, and the action list's revision it was parked at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timer {
    pub(crate) token: u64,
    pub(crate) due: Instant,
    rev: u64,
}

/// Native Play and control-channel menu clicks may open editors and wait between steps.
pub(super) fn is_interactive(app: &PhotocraftApp) -> bool {
    !app.automation_input && !app.programmatic_playback
}

/// How long a person's Play waits after a step, or `None` when it goes straight on (Accelerated,
/// and always for agents).
pub(super) fn step_pause(app: &PhotocraftApp) -> Option<Duration> {
    if !is_interactive(app) {
        return None;
    }
    let a = &app.session.prefs().actions;
    match a.performance {
        PlaybackPerformance::Accelerated => None,
        PlaybackPerformance::StepByStep => Some(STEP_GAP),
        PlaybackPerformance::Pause => Some(STEP_GAP.saturating_add(Duration::from_secs(u64::from(a.pause_seconds())))),
    }
}

/// Step `i` of `action` ended as `c` (`ran` steps of it before): when it simply ran and Step by
/// Step or Pause For waits before the next enabled step, park the rest ([`super::Called::Paced`]).
pub(super) fn paced(app: &mut PhotocraftApp, action: &super::Action, i: usize, id: &str, ran: u64, c: super::Called) -> super::Called {
    if !matches!(c, super::Called::Done) || !action.enabled_steps().any(|(j, _)| j > i) {
        return c;
    }
    match step_pause(app) {
        Some(wait) => super::Called::Paced(pace(app, &action.name, i, id, ran.saturating_add(1), wait)),
        None => c,
    }
}

/// Park `action` after its step `i` (`ran` steps of it so far): it continues from the next step
/// after `wait`. Returns the `paused` value [`super::play_owned`] reports.
fn pace(app: &mut PhotocraftApp, action: &str, i: usize, id: &str, ran: u64, wait: Duration) -> Value {
    let rev = app.session.actions.rev;
    let gate = app.session.authorize;
    let pb = &mut app.paused_playbacks;
    let token = pb.pause(action.to_string(), i.saturating_add(1), gate);
    if let Some(entry) = pb.paused.get_mut(&token) {
        entry.timed = true;
        entry.ran = ran;
    }
    let due = Instant::now().checked_add(wait).unwrap_or_else(Instant::now);
    // One paced playback at a time: Play ends a waiting one before it starts (`stop`).
    if let Some(old) = pb.timer.replace(Timer { token, due, rev }) {
        pb.paused.remove(&old.token);
    }
    json!({"step": i, "id": id, "dialog": PLAYBACK_TIMER})
}

/// The token of the paced playback that is waiting, if any.
pub(super) fn waiting(app: &PhotocraftApp) -> Option<u64> {
    app.paused_playbacks.timer.map(|t| t.token)
}

/// Whether a paced playback is waiting for its next step.
pub(crate) fn pending(app: &PhotocraftApp) -> bool {
    app.paused_playbacks.timer.is_some()
}

/// End a waiting paced playback where it is: the steps it applied stay, in one history state when
/// the option is on. Returns whether one was waiting.
pub(crate) fn stop(app: &mut PhotocraftApp) -> bool {
    let Some(timer) = app.paused_playbacks.timer.take() else { return false };
    let fields = Map::from_iter([(super::RESUME.to_string(), json!(timer.token))]);
    super::stopped(app, &fields);
    true
}

/// Once per frame: play the next step when its wait is over, or end the playback on Esc, an edit of
/// the action list or a new recording.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(timer) = app.paused_playbacks.timer else { return };
    let esc = app.ui.dialogs.is_empty() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let changed = timer.rev != app.session.actions.rev || app.session.actions.recording.is_some();
    if esc || changed || !app.paused_playbacks.paused.contains_key(&timer.token) {
        stop(app);
        return;
    }
    let now = Instant::now();
    // A dialog opened meanwhile (Playback Options itself, say) holds the next step until it closes.
    if now < timer.due || !app.ui.dialogs.is_empty() {
        ctx.request_repaint_after(timer.due.saturating_duration_since(now).max(Duration::from_millis(100)));
        return;
    }
    app.paused_playbacks.timer = None;
    let fields = Map::from_iter([(super::RESUME.to_string(), json!(timer.token))]);
    // `resume` reports the outcome in the status bar.
    let _ = super::resume(app, &fields);
    ctx.request_repaint();
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(PLAYBACK_OPTIONS)
}

/// Playback Options…: the saved options, Pause For's seconds kept while another mode is chosen.
pub(super) fn open_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let a = &app.session.prefs().actions;
    let fields = json!({
        PLAYBACK_OPTIONS: true,
        "__label": "Playback Options",
        "performance": a.performance.name(),
        "seconds": a.pause_seconds(),
        "singleState": a.create_single_history_state,
    });
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(json!({"dialog": id}))
}

/// Performance (Accelerated, Step by Step, Pause For: n seconds) and History (Create a single
/// history state), laid out as the reference application's dialog.
pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    crate::widgets::section_label(ui, tl!("Performance"));
    crate::widgets::hairline(ui);
    let mut performance = fields.get("performance").and_then(Value::as_str).and_then(PlaybackPerformance::parse).unwrap_or_default();
    let before = performance;
    ui.indent("playback-options-performance", |ui| {
        ui.radio_value(&mut performance, PlaybackPerformance::Accelerated, tl!("Accelerated"));
        ui.radio_value(&mut performance, PlaybackPerformance::StepByStep, tl!("Step by Step"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut performance, PlaybackPerformance::Pause, tl!("Pause For:"));
            let range = Actions::PAUSE_SECONDS;
            if performance == PlaybackPerformance::Pause {
                let mut seconds = f32::from(seconds_of(fields));
                let r = crate::widgets::value_field_prec(ui, &mut seconds, f32::from(*range.start())..=f32::from(*range.end()), "", 40.0, 0);
                if r.accessible_name(tl!("Pause For")).changed() {
                    fields.insert("seconds".into(), json!(clamp_seconds(f64::from(seconds))));
                }
            } else {
                // Empty and greyed until Pause For is chosen, as in the reference application.
                ui.add_enabled(false, egui::TextEdit::singleline(&mut String::new()).desired_width(40.0)).accessible_name(tl!("Pause For"));
            }
            ui.add_enabled(performance == PlaybackPerformance::Pause, egui::Label::new(tl!("seconds")));
        });
    });
    if performance != before {
        fields.insert("performance".into(), json!(performance.name()));
    }
    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("History"));
    crate::widgets::hairline(ui);
    let mut single = fields.get("singleState").and_then(Value::as_bool).unwrap_or(true);
    ui.indent("playback-options-history", |ui| {
        if ui.checkbox(&mut single, tl!("Create a single history state")).changed() {
            fields.insert("singleState".into(), json!(single));
        }
    });
}

/// A whole number of seconds within range: the reference application's field takes only digits and
/// puts the closest valid value in.
fn clamp_seconds(v: f64) -> u8 {
    let range = Actions::PAUSE_SECONDS;
    if !v.is_finite() {
        return *range.start();
    }
    v.round().clamp(f64::from(*range.start()), f64::from(*range.end())) as u8
}

fn seconds_of(fields: &Map<String, Value>) -> u8 {
    fields.get("seconds").and_then(Value::as_f64).map_or(*Actions::PAUSE_SECONDS.start(), clamp_seconds)
}

/// OK: store the options (`actions.setPlayback`). Performance and Create a single history state go
/// as the dialog holds them, so the command refuses a value automation set that the dialog can't
/// show (`null`, an unknown mode) instead of saving a default in its place.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let field = |key: &str| fields.get(key).cloned().unwrap_or(Value::Null);
    app.run("actions.setPlayback", json!({"performance": field("performance"), "seconds": seconds_of(fields), "singleState": field("singleState")}))
}
