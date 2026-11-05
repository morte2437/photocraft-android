//! A standing warning while the web build's memory is nearly full (#10024).
//!
//! On the web a failed allocation aborts the module and the page freezes without a message. The
//! big operations refuse up front when they won't fit (`photocraft_engine::memory_gate`); this
//! tells the user earlier, while there is still room to save: when the module's memory is within
//! [`WARN_BELOW`] of the wasm32 ceiling, a notice that stays until dismissed asks them to save and
//! free memory. The answer comes from [`photocraft_raster::headroom::below`], which reads the
//! size of linear memory and allocates nothing. Linear memory never shrinks, so on the web the
//! warning doesn't re-arm once shown (only `?free-mb` or a test can make memory recover). It is always "plenty" on
//! the desktop, so the warning never shows there.

use photocraft_raster::headroom;

use crate::PhotocraftApp;

/// Warn when less than this can be allocated.
pub const WARN_BELOW: u64 = 512 << 20;

/// Warn again only after this much (or more) was available again, so a figure hovering around
/// the threshold doesn't flicker the notice on and off.
pub const REARM_ABOVE: u64 = 768 << 20;

/// Seconds between checks.
const CHECK_EVERY: f64 = 2.0;

/// The notice's title (an English source string, translated when drawn).
pub const TITLE: &str = "Memory is nearly full";

/// What a check decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing to do.
    Idle,
    /// Memory just ran low: post the notice.
    Warn,
    /// Memory recovered: the warning is armed again, and the old notice is stale.
    Rearm,
}

/// The warning's memory: when it last looked, and whether it has warned and not yet re-armed.
#[derive(Clone, Debug, Default)]
pub struct Watch {
    next_check: f64,
    warned: bool,
}

impl Watch {
    /// `true` when it is time to look again at `now` (seconds); then schedules the next look.
    pub fn due(&mut self, now: f64) -> bool {
        if now < self.next_check {
            return false;
        }
        self.next_check = now + CHECK_EVERY;
        true
    }

    /// `true` between a warning and its re-arming.
    pub fn warned(&self) -> bool {
        self.warned
    }

    /// Advances with what a check found: `below_warn` is "less than [`WARN_BELOW`] is available",
    /// `below_rearm` is "less than [`REARM_ABOVE`] is available".
    pub fn step(&mut self, below_warn: bool, below_rearm: bool) -> Step {
        if !self.warned {
            if below_warn {
                self.warned = true;
                return Step::Warn;
            }
            return Step::Idle;
        }
        if below_rearm {
            return Step::Idle;
        }
        self.warned = false;
        Step::Rearm
    }
}

/// Run once per frame: at most every two seconds, and only while a document is open, posts the
/// warning when memory is nearly full, and takes it down again once memory has recovered.
pub fn tick(app: &mut PhotocraftApp, now: f64) {
    if app.session.active().is_none() || !app.low_memory.due(now) {
        return;
    }
    let below_warn = headroom::below(WARN_BELOW);
    let below_rearm = if app.low_memory.warned() { headroom::below(REARM_ABOVE) } else { below_warn };
    match app.low_memory.step(below_warn, below_rearm) {
        Step::Idle => {}
        Step::Warn => post(app),
        Step::Rearm => app.ui.notices.retain(|n| n.title != TITLE),
    }
}

fn post(app: &mut PhotocraftApp) {
    if app.ui.notices.iter().any(|n| n.title == TITLE) {
        return;
    }
    let lines = vec![
        "Save your work now: if the browser runs out of memory this page stops, and changes since the last save can't be recovered.".to_owned(),
        "To make room, close documents you don't need or use Edit › Purge › All.".to_owned(),
    ];
    let id = crate::notices::post(app, TITLE, lines, true, None);
    if let Some(notice) = app.ui.notices.iter_mut().find(|n| n.id == id) {
        notice.sticky = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_once_then_waits_for_the_higher_threshold() {
        let mut w = Watch::default();
        assert_eq!(w.step(false, false), Step::Idle);
        assert_eq!(w.step(true, true), Step::Warn);
        // Staying low, or hovering between the two thresholds, doesn't warn again.
        assert_eq!(w.step(true, true), Step::Idle);
        assert_eq!(w.step(false, true), Step::Idle);
        assert_eq!(w.step(true, true), Step::Idle);
        // Rising above the higher threshold re-arms; dropping again warns again.
        assert_eq!(w.step(false, false), Step::Rearm);
        assert_eq!(w.step(false, false), Step::Idle);
        assert_eq!(w.step(true, true), Step::Warn);
    }

    #[test]
    fn checks_are_spaced_two_seconds_apart() {
        let mut w = Watch::default();
        assert!(w.due(0.0));
        assert!(!w.due(1.0));
        assert!(!w.due(1.99));
        assert!(w.due(2.0));
        assert!(!w.due(3.0));
    }
}
