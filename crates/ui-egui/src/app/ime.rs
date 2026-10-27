use super::*;

impl PhotocraftApp {
    /// Install the callback-mode markers in an app creator, before the first `run_ui` on this
    /// Context (and again through [`Self::setup_context`] when a host replaces its Context).
    /// Installing while a pass is already running cannot mark that pass; it only arms the next.
    pub fn install_type_reset_hooks(ctx: &egui::Context) {
        type_tool::install_reset_hooks(ctx);
    }

    /// The host's UI tail, after all panels and any shell-side model/focus mutations.
    pub fn finish_type_reset_pass(&mut self, ctx: &egui::Context) {
        type_tool::finish_reset_pass(self, ctx);
    }

    /// The host's logic tail; full UI passes leave availability to their later UI tail.
    pub fn finish_type_reset_logic(&mut self, ctx: &egui::Context) {
        type_tool::finish_reset_logic(self, ctx);
    }
}
