//! Photoshop's alerts when a painting tool can't paint (#11898): a press on a hidden target
//! layer, on a layer with Lock All, or with a group active (its pixels, not its mask) opens a
//! modal alert with OK instead of a status-bar message, and paints nothing. Only the tools whose
//! wording was measured have a name here ([`tool_name`]); the others keep today's status bar.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// The alerts; `{tool}` is [`tool_name`].
pub const HIDDEN: &str = "Could not use the {tool} because the target layer is hidden.";
pub const LOCKED: &str = "Could not use the {tool} because the layer is locked.";
pub const GROUP: &str = "Could not use the {tool} because the pixels in a group cannot be edited without merging the group.";

/// The tool's name in the alerts, for the tools whose wording was measured: the Brush.
pub fn tool_name(tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Brush => Some(tl!("brush tool")),
        _ => None,
    }
}

/// The alert a press with `tool` gets, if any. `hidden` is the press-time hidden-target refusal
/// (`canvas::tool_input`), which this only words; the lock and the group are checked here. Lock
/// transparent pixels is no refusal, and a group's targeted mask paints.
pub fn alert(app: &PhotocraftApp, tool: Tool, hidden: bool) -> Option<String> {
    let name = tool_name(tool)?;
    let template = if hidden {
        HIDDEN
    } else {
        let st = app.session.active()?;
        let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
        if crate::canvas::paint_target(app) != json!("pixels") {
            return None;
        }
        if layer.is_group() {
            GROUP
        } else if st.doc.effective_locks(layer.id).all {
            LOCKED
        } else {
            return None;
        }
    };
    Some(crate::i18n::fmt(tl!(template), &[("tool", name)]))
}

/// Opens the alert: ordinary dialog state, listed by `ui.inspect` and closed by `ui.dialog.confirm`.
pub fn prompt(app: &mut PhotocraftApp, message: &str) {
    app.ui.open_dialog(DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
}

#[cfg(test)]
#[path = "../brush_refusal_alert_tests.rs"]
mod brush_refusal_alert_tests;
