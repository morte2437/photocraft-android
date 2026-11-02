//! Live preview for the Color Picker targets that are UI state (#9886): a colour field of another
//! dialog ([`open_for_field`](super::open_for_field): Edit › Fill's Color…, Stroke, a filter's
//! colour, a guide or channel colour, a preference) and the Crop shield's custom colour.
//!
//! Photoshop shows the colour under the picker's cursor in its target while the picker is open,
//! so the target here follows the picker before OK: the parent dialog's swatch and its own preview
//! (a filter's canvas preview, the guide layout) update by themselves. Each time the colour changes
//! the target is set back to what it was when the picker opened, plus the new colour, so a dialog
//! that derives other settings from it (Replace Color's Result sets the sliders) never builds on a
//! colour it was only shown. Cancel, or a picker closed any other way, puts the target back as it
//! was; OK puts it back too and then sets it as before ([`super::confirm`]). The document is never
//! touched: the parent dialog's own OK still makes the one history step. What the target was is
//! kept here, not in the picker's fields, which automation may rewrite.

use serde_json::{Map, Value, json};

use super::{hex, parse_hex, set_field};
use crate::PhotocraftApp;
use crate::crop_shield::ShieldColor;

/// A live target as it was when its picker opened.
enum Before {
    /// `field` of dialog `parent`, with all of that dialog's fields.
    Field { parent: u64, field: String, fields: Map<String, Value> },
    /// The Crop shield's colour choice and custom colour.
    CropShield { color: ShieldColor, custom: [u8; 3] },
}

/// One open picker's live target.
pub(crate) struct Live {
    picker: u64,
    before: Before,
    /// The colour shown in the target; `None` while it shows what it was.
    shown: Option<String>,
}

/// Preview picker `picker` in `field` of dialog `parent` (a key or a JSON pointer).
pub(super) fn begin_field(app: &mut PhotocraftApp, picker: u64, parent: u64, field: &str) {
    let Some(d) = app.ui.dialogs.iter().find(|d| d.id == parent) else { return };
    let before = Before::Field { parent, field: field.into(), fields: d.fields.clone() };
    app.color_picker_live.push(Live { picker, before, shown: None });
}

/// Preview picker `picker` in the Crop shield's custom colour.
pub(super) fn begin_crop_shield(app: &mut PhotocraftApp, picker: u64) {
    let s = &app.ui.tool_options.crop_shield;
    let before = Before::CropShield { color: s.color, custom: s.custom_color };
    app.color_picker_live.push(Live { picker, before, shown: None });
}

/// The dialogs whose fields an open picker is previewing in: they record no ⌘Z step meanwhile
/// (`edit_history`), so a Cancel leaves no trace and an OK is one step.
pub(crate) fn previewed_dialogs(app: &PhotocraftApp) -> Vec<u64> {
    app.color_picker_live
        .iter()
        .filter_map(|l| match l.before {
            Before::Field { parent, .. } => Some(parent),
            Before::CropShield { .. } => None,
        })
        .collect()
}

/// The field pickers previewing in dialog `parent`: they close with it.
pub(crate) fn pickers_of(app: &PhotocraftApp, parent: u64) -> Vec<u64> {
    app.color_picker_live.iter().filter(|l| matches!(l.before, Before::Field { parent: p, .. } if p == parent)).map(|l| l.picker).collect()
}

/// Whether picker `picker` has a live target: then its parent is the one recorded here, not its
/// `__dialog` field, which automation could have rewritten.
pub(crate) fn previews(app: &PhotocraftApp, picker: u64) -> bool {
    app.color_picker_live.iter().any(|l| l.picker == picker)
}

/// The colour picker `picker` shows: `Some(None)` while it is the one it opened on (or not
/// a colour), `None` once the picker is closed.
fn picked(app: &PhotocraftApp, picker: u64) -> Option<Option<String>> {
    let d = app.ui.dialogs.iter().find(|d| d.id == picker)?;
    let colour = |key: &str| d.fields.get(key).and_then(Value::as_str).and_then(parse_hex).map(hex);
    let (color, orig) = (colour("color"), colour("__orig"));
    Some(color.filter(|c| orig.as_ref() != Some(c)))
}

/// `"#rrggbb"` as the Crop shield stores it.
fn shield_rgb(color: &str) -> Option<[u8; 3]> {
    parse_hex(color).map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

/// Show `color` in the target, or what it was (`None`), where it shows `shown` now. `Err` when
/// the target is gone.
fn show(app: &mut PhotocraftApp, before: &mut Before, shown: Option<&str>, color: Option<&str>) -> Result<(), String> {
    match before {
        Before::Field { parent, field, fields } => {
            let mut next = fields.clone();
            if let Some(c) = color {
                set_field(&mut next, field, json!(c))?;
            }
            app.ui.dialog_mut(*parent).ok_or("the dialog this colour was for is closed")?.fields = next;
        }
        Before::CropShield { color: was, custom } => {
            // A shield colour set elsewhere while the picker is open (`ui.set`) is what Cancel
            // puts back, not the one the picker opened on.
            let s = &app.ui.tool_options.crop_shield;
            let expected = match shown {
                Some(c) => (ShieldColor::Custom, shield_rgb(c)),
                None => (*was, Some(*custom)),
            };
            if (s.color, Some(s.custom_color)) != expected {
                (*was, *custom) = (s.color, s.custom_color);
            }
            match color {
                Some(c) => crate::crop_shield::set_custom_color(app, c)?,
                None => {
                    let s = &mut app.ui.tool_options.crop_shield;
                    (s.color, s.custom_color) = (*was, *custom);
                }
            }
        }
    }
    Ok(())
}

/// Show each open picker's colour in its target, and put back the target of a picker that closed
/// without [`end`]. True when a target changed (the caller repaints, so the parent redraws).
pub(crate) fn sync(app: &mut PhotocraftApp) -> bool {
    if app.color_picker_live.is_empty() {
        return false;
    }
    let mut live = std::mem::take(&mut app.color_picker_live);
    let mut changed = false;
    live.retain_mut(|l| {
        let Some(color) = picked(app, l.picker) else {
            if l.shown.is_some() {
                changed |= show(app, &mut l.before, l.shown.as_deref(), None).is_ok();
            }
            return false;
        };
        if color == l.shown {
            return true;
        }
        // A target that is gone (its dialog closed) stops the preview.
        if show(app, &mut l.before, l.shown.as_deref(), color.as_deref()).is_err() {
            return false;
        }
        l.shown = color;
        changed = true;
        true
    });
    live.append(&mut app.color_picker_live);
    app.color_picker_live = live;
    changed
}

/// Picker `picker` is closing (OK or Cancel): put its target back as it was when it opened and
/// stop previewing. OK then sets the target as it always has.
pub(crate) fn end(app: &mut PhotocraftApp, picker: u64) {
    let Some(i) = app.color_picker_live.iter().position(|l| l.picker == picker) else { return };
    let mut l = app.color_picker_live.remove(i);
    if l.shown.is_some() {
        let _ = show(app, &mut l.before, l.shown.as_deref(), None);
    }
}

#[cfg(test)]
mod tests;
