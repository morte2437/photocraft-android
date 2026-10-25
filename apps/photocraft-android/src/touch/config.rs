//! Touch Input settings: what fingers do on the canvas, the multi-touch gestures and what the
//! taps run. Saved as JSON in the app's files directory, like the Shortcut Pad's.

pub use photocraft_tablet::motion::PenControl;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What one finger does on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FingerMode {
    /// Paints and clicks like the mouse (the pen does too).
    #[default]
    Draw,
    /// Drags the canvas around; only the pen paints.
    Navigate,
    /// Nothing: only the pen paints, and fingers are for gestures.
    Gestures,
}

impl FingerMode {
    pub const ALL: [FingerMode; 3] = [FingerMode::Draw, FingerMode::Navigate, FingerMode::Gestures];

    pub fn label(self) -> &'static str {
        match self {
            FingerMode::Draw => "Draw and click, like the pen",
            FingerMode::Navigate => "Move the canvas",
            FingerMode::Gestures => "Nothing (gestures only)",
        }
    }

    /// Short form for the Window › Touch Input menu.
    pub fn menu_label(self) -> &'static str {
        match self {
            FingerMode::Draw => "Finger Draws",
            FingerMode::Navigate => "Finger Moves Canvas",
            FingerMode::Gestures => "Finger for Gestures Only",
        }
    }
}

/// What one of the pen's controls (a side button, the eraser end) does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PenAction {
    /// In use, the pen erases (the Eraser tool, back to the previous tool after).
    #[default]
    Eraser,
    /// A press switches to the Eraser; the next press switches back. For pens that can't draw
    /// with a button held (the S Pen).
    ToggleEraser,
    /// A right-click under the pen: pressed while hovering, or held while touching.
    RightClick,
    /// A press runs a shortcut.
    Shortcut,
    /// Nothing (the eraser end draws like the tip).
    Nothing,
}

impl PenAction {
    pub const ALL: [PenAction; 5] = [PenAction::Eraser, PenAction::ToggleEraser, PenAction::RightClick, PenAction::Shortcut, PenAction::Nothing];

    pub fn label(self) -> &'static str {
        match self {
            PenAction::Eraser => "Erase while held",
            PenAction::ToggleEraser => "Switch to Eraser and back",
            PenAction::RightClick => "Right-click",
            PenAction::Shortcut => "Run a shortcut",
            PenAction::Nothing => "Nothing",
        }
    }
}

/// One pen control's setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PenSetting {
    pub action: PenAction,
    /// The shortcut a press runs ([`PenAction::Shortcut`]).
    pub shortcut: String,
}

impl PenSetting {
    fn new(action: PenAction) -> Self {
        Self { action, shortcut: "Ctrl+Z".into() }
    }
}

impl Default for PenSetting {
    fn default() -> Self {
        Self::new(PenAction::Eraser)
    }
}

/// A pen control's name in the settings.
pub fn pen_control_label(c: PenControl) -> &'static str {
    match c {
        PenControl::Lower => "Pen button 1",
        PenControl::Upper => "Pen button 2",
        PenControl::Top => "Pen button 3",
        PenControl::EraserEnd => "Eraser end",
    }
}

/// The trackpad cursor's speed range: cursor movement per finger movement.
pub const TRACKPAD_SPEED: std::ops::RangeInclusive<f32> = 0.5..=3.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchConfig {
    /// One finger on the canvas.
    pub finger: FingerMode,
    /// Two fingers dragging move the canvas.
    pub pan: bool,
    /// Pinching zooms around the fingers.
    pub zoom: bool,
    /// Twisting two fingers rotates the view.
    pub rotate: bool,
    /// The shortcut a two-finger tap presses (empty: nothing).
    pub two_finger_tap: String,
    /// The shortcut a three-finger tap presses (empty: nothing).
    pub three_finger_tap: String,
    /// Ignore fingers while the pen is touching or near the screen.
    pub palm_rejection: bool,
    /// Two fingers dragging over menus, panels and lists scroll them.
    pub two_finger_scroll: bool,
    /// A finger held still on a button, tab or row right-clicks it.
    pub long_press_right_click: bool,
    /// Fingers move a cursor, like a laptop trackpad, instead of touching things directly: the
    /// fingertip never covers what it points at (#4). See `pad::trackpad`.
    pub trackpad: bool,
    /// How far the cursor moves per finger movement.
    pub trackpad_speed: f32,
    /// Quick swipes move the cursor further (slow moves stay precise).
    pub trackpad_acceleration: bool,
    /// One finger resting while another moves holds the button (painting, dragging).
    pub trackpad_rest_to_drag: bool,
    /// A quick two-finger tap right-clicks instead of running its shortcut.
    pub trackpad_two_finger_right_click: bool,
    /// Fingers paint heavier when pressed harder, judged from the size of their contact with the
    /// screen (experimental), between a light touch's contact size and a firm press's.
    pub finger_pressure: bool,
    pub finger_light: f32,
    pub finger_firm: f32,
    /// The pen's controls, by [`PenControl::index`]: lower, upper and top side buttons, eraser
    /// end.
    pub pen: [PenSetting; 4],
}

impl Default for TouchConfig {
    fn default() -> Self {
        Self {
            finger: FingerMode::Draw,
            pan: true,
            zoom: true,
            rotate: true,
            two_finger_tap: "Ctrl+Z".into(),
            three_finger_tap: "Ctrl+Shift+Z".into(),
            palm_rejection: true,
            two_finger_scroll: true,
            long_press_right_click: true,
            trackpad: false,
            trackpad_speed: 1.0,
            trackpad_acceleration: false,
            trackpad_rest_to_drag: false,
            trackpad_two_finger_right_click: false,
            finger_pressure: false,
            finger_light: 0.0,
            finger_firm: 0.0,
            // As a Wacom pen's on a PC: the lower button right-clicks; both other controls erase.
            pen: [
                PenSetting::new(PenAction::RightClick),
                PenSetting::new(PenAction::Eraser),
                PenSetting::new(PenAction::Nothing),
                PenSetting::new(PenAction::Eraser),
            ],
        }
    }
}

impl TouchConfig {
    pub fn pen(&self, c: PenControl) -> &PenSetting {
        &self.pen[c.index()]
    }

    /// Which pen controls erase, for the pen hook.
    pub fn erasing_controls(&self) -> [bool; 4] {
        PenControl::ALL.map(|c| self.pen(c).action == PenAction::Eraser)
    }

    pub fn sanitized(mut self) -> Self {
        self.trackpad_speed = if self.trackpad_speed.is_finite() { self.trackpad_speed.clamp(*TRACKPAD_SPEED.start(), *TRACKPAD_SPEED.end()) } else { 1.0 };
        for v in [&mut self.finger_light, &mut self.finger_firm] {
            *v = if v.is_finite() { v.clamp(0.0, 10_000.0) } else { 0.0 };
        }
        let [a, b, c, d] = &mut self.pen;
        for s in [&mut self.two_finger_tap, &mut self.three_finger_tap, &mut a.shortcut, &mut b.shortcut, &mut c.shortcut, &mut d.shortcut] {
            *s = s.trim().chars().take(40).collect();
        }
        self
    }

    pub fn load(path: &Path) -> Self {
        let read = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<TouchConfig>(&b).map_err(|e| log::warn!("touch input settings: {e}")).ok());
        read.unwrap_or_default().sanitized()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        photocraft_format::atomic_write(path, &text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_settings_fall_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("pc-touch-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("touch-input.json");
        assert_eq!(TouchConfig::load(&path), TouchConfig::default());
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(TouchConfig::load(&path), TouchConfig::default());
        // Older files without a field keep the default for it.
        std::fs::write(&path, br#"{"finger":"Navigate"}"#).unwrap();
        let c = TouchConfig::load(&path);
        assert_eq!(c.finger, FingerMode::Navigate);
        assert!(c.zoom && c.palm_rejection && c.long_press_right_click);
        assert_eq!(c.pen(PenControl::Lower).action, PenAction::RightClick);
        assert_eq!(c.erasing_controls(), [false, true, false, true]);
        let saved = TouchConfig { finger: FingerMode::Gestures, two_finger_tap: "  Ctrl+0 ".into(), ..Default::default() }.sanitized();
        saved.save(&path).unwrap();
        assert_eq!(TouchConfig::load(&path).two_finger_tap, "Ctrl+0");
        std::fs::remove_dir_all(dir).ok();
    }
}
