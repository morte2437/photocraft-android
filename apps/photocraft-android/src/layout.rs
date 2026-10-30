//! Phone or tablet layout: **Window › Layout** picks Automatic (a phone when the screen's
//! smallest side is under [`PHONE_MAX_DP`], Android's own phone/tablet line, so a phone stays
//! a phone on its side), Phone or Tablet. Saved in the app's files directory (`layout.txt`).
//!
//! The phone layout ([`photocraft_ui_egui::PhotocraftApp::compact_layout`]) makes Auto interface
//! scale smaller, dialogs as wide as the window (scrolling when taller), and the Shortcut Pad
//! slimmer.

use photocraft_ui_egui::menus::MenuItem;
use photocraft_ui_egui::platform_menu::PlatformMenu;
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Screens whose smallest side is under this many dp are phones.
pub const PHONE_MAX_DP: f32 = 600.0;

const MENU: &[&str] = &["Window", "Layout"];
const PREFIX: &str = "android.layout.";
const KEYBOARD_MENU: &[&str] = &["Window", "On-Screen Keyboard"];
/// Window › On-Screen Keyboard: the app moves up above it, or it covers the bottom of the app.
const KEYBOARD_SHRINK: &str = "android.keyboard.shrink";
const KEYBOARD_COVER: &str = "android.keyboard.cover";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    #[default]
    Auto,
    Phone,
    Tablet,
}

impl LayoutMode {
    pub const ALL: [LayoutMode; 3] = [LayoutMode::Auto, LayoutMode::Phone, LayoutMode::Tablet];

    fn key(self) -> &'static str {
        match self {
            LayoutMode::Auto => "auto",
            LayoutMode::Phone => "phone",
            LayoutMode::Tablet => "tablet",
        }
    }

    fn label(self) -> &'static str {
        match self {
            LayoutMode::Auto => "Automatic (Phone or Tablet)",
            LayoutMode::Phone => "Phone",
            LayoutMode::Tablet => "Tablet",
        }
    }

    fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == k.trim())
    }

    /// Whether a screen whose smallest side is `smallest_dp` gets the phone layout.
    pub fn is_phone(self, smallest_dp: f32) -> bool {
        match self {
            LayoutMode::Auto => smallest_dp > 0.0 && smallest_dp < PHONE_MAX_DP,
            LayoutMode::Phone => true,
            LayoutMode::Tablet => false,
        }
    }
}

struct Shared {
    mode: LayoutMode,
    path: PathBuf,
    /// The on-screen keyboard covers the bottom of the app instead of the app shrinking above it.
    keyboard_covers: bool,
    keyboard_path: PathBuf,
}

pub struct Layout {
    shared: Rc<RefCell<Shared>>,
}

impl Layout {
    pub fn new(files_dir: &std::path::Path) -> Self {
        let path = files_dir.join("layout.txt");
        let mode = std::fs::read_to_string(&path).ok().and_then(|t| LayoutMode::from_key(&t)).unwrap_or_default();
        let keyboard_path = files_dir.join("keyboard.txt");
        let keyboard_covers = std::fs::read_to_string(&keyboard_path).is_ok_and(|t| t.trim() == "cover");
        Self { shared: Rc::new(RefCell::new(Shared { mode, path, keyboard_covers, keyboard_path })) }
    }

    /// Whether the on-screen keyboard covers the bottom of the app (Window › On-Screen Keyboard ›
    /// Cover the Screen) rather than the app shrinking to fit above it.
    pub fn keyboard_covers(&self) -> bool {
        self.shared.borrow().keyboard_covers
    }

    /// Whether this frame gets the phone layout. egui points times the zoom factor are dp (the
    /// interface scale doesn't change the answer).
    /// `keyboard`: the height (points) the on-screen keyboard took off the bottom of the window,
    /// which a tablet keeps being a tablet through.
    pub fn is_phone(&self, ctx: &egui::Context, keyboard: f32) -> bool {
        let r = ctx.content_rect();
        self.shared.borrow().mode.is_phone(r.width().min(r.height() + keyboard.max(0.0)) * ctx.zoom_factor())
    }

    /// The Window › Layout rows.
    pub fn menu(&self) -> PlatformMenu {
        let (read, write) = (self.shared.clone(), self.shared.clone());
        PlatformMenu {
            items: Box::new(move || {
                let (mode, covers) = {
                    let s = read.borrow();
                    (s.mode, s.keyboard_covers)
                };
                let row = |id: String, label: &str, path: &[&str], checked: bool| MenuItem {
                    id,
                    label: label.into(),
                    path: path.iter().map(|s| s.to_string()).collect(),
                    shortcut: None,
                    enabled: true,
                    checked: Some(checked),
                    color: None,
                };
                let mut rows: Vec<MenuItem> = LayoutMode::ALL.iter().map(|m| row(format!("{PREFIX}{}", m.key()), m.label(), MENU, mode == *m)).collect();
                rows.push(row(KEYBOARD_SHRINK.into(), "Shrink the Screen to Fit", KEYBOARD_MENU, !covers));
                rows.push(row(KEYBOARD_COVER.into(), "Cover the Screen", KEYBOARD_MENU, covers));
                rows
            }),
            run: Box::new(move |id| {
                if id == KEYBOARD_SHRINK || id == KEYBOARD_COVER {
                    let mut s = write.borrow_mut();
                    s.keyboard_covers = id == KEYBOARD_COVER;
                    let key: &[u8] = if s.keyboard_covers { b"cover" } else { b"shrink" };
                    if let Err(e) = photocraft_format::atomic_write(&s.keyboard_path, key) {
                        log::warn!("couldn't save the keyboard choice: {e}");
                    }
                    return Ok(Value::Null);
                }
                let mode = id.strip_prefix(PREFIX).and_then(LayoutMode::from_key).ok_or_else(|| format!("unknown Layout command {id}"))?;
                let mut s = write.borrow_mut();
                s.mode = mode;
                if let Err(e) = photocraft_format::atomic_write(&s.path, mode.key().as_bytes()) {
                    log::warn!("couldn't save the layout choice: {e}");
                }
                Ok(Value::Null)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phones_are_told_from_tablets_by_their_smallest_side() {
        assert!(LayoutMode::Auto.is_phone(411.0), "a phone, either way up");
        assert!(!LayoutMode::Auto.is_phone(800.0), "a tablet");
        assert!(!LayoutMode::Auto.is_phone(0.0), "no screen yet");
        assert!(LayoutMode::Phone.is_phone(1200.0) && !LayoutMode::Tablet.is_phone(300.0), "the choice wins");
    }

    #[test]
    fn the_choice_is_saved() {
        let dir = std::env::temp_dir().join(format!("pc-layout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let l = Layout::new(&dir);
        assert_eq!(l.shared.borrow().mode, LayoutMode::Auto);
        (l.menu().run)("android.layout.phone").unwrap();
        assert_eq!(Layout::new(&dir).shared.borrow().mode, LayoutMode::Phone);
        assert!((l.menu().run)("android.layout.huge").is_err());
        // The keyboard: shrinking the screen until told to cover it, and kept.
        assert!(!l.keyboard_covers());
        (l.menu().run)("android.keyboard.cover").unwrap();
        assert!(l.keyboard_covers() && Layout::new(&dir).keyboard_covers());
        (l.menu().run)("android.keyboard.shrink").unwrap();
        assert!(!Layout::new(&dir).keyboard_covers());
        std::fs::remove_dir_all(dir).ok();
    }
}
