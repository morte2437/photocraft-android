use super::PhotocraftApp;

impl PhotocraftApp {
    /// Whether the title bar's free gap gets the drag overlay this frame: the app draws its title
    /// strip (`chrome`: not hidden by the screen mode) and a dialog's modal layer covers it. Not
    /// only with the custom title bar: on macOS the strip under the traffic lights is the window's
    /// title bar too, and an open dialog (Embedded Profile Mismatch…) left it undraggable (#5826).
    pub(crate) fn titlebar_modal_drag(&self, chrome: bool) -> bool {
        chrome && self.ui.panels.menu_bar && !self.ui.dialogs.is_empty()
    }
}
