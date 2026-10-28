//! Photoshop Actions files (`.atn`): Load Actions (`actions.load`) and Save Actions
//! (`actions.save`), plus the two step commands a Photoshop set needs: `actions.stopMessage`
//! (Photoshop's Insert Stop) and `actions.photoshopStep` (a step PhotoCraft has no command for
//! yet, kept verbatim so Save Actions writes it back unchanged).
//!
//! Parsing and writing live in `photocraft-psd` (`atn`); this module maps Photoshop *events*
//! (`gaussianBlur` with `Rds`, `make` of an adjustment layer, `set` of the selection…) onto
//! PhotoCraft commands and back. An event is keyed by both its four-character code (`GsnB`,
//! older files) and its string name (`gaussianBlur`, Photoshop 2024 and later), as the two are
//! interchangeable in Photoshop. The table ([`EVENTS`]) is bidirectional: a step that was
//! imported exports to the same event. Events outside it become `actions.photoshopStep` steps,
//! which fail at playback with the step's Photoshop name, so an action never silently skips
//! work; tool selections and other steps with no document effect are imported switched off.
//!
//! Layer references by name (`select` of layer "Shadow") become `{"layer": "Shadow"}`, which
//! `commands::layer_param` resolves in whatever document the action plays on (#219). A switch
//! to another open document (`select` of `Dcmn` by offset) becomes `document.activate
//! {"offset"}`, the tab that many places from the active one (#7103).

use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::{Map, Value, json};

use crate::actions_cmds::Action;
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod adjustments;
mod convert;
mod descriptors;
mod events;
mod fields;
mod files;
mod steps_doc;
mod steps_image;
mod steps_layer;
mod steps_make;
mod tables;

#[cfg(test)]
use adjustments::{levels_from, selective_from};
pub use convert::{Imported, Skipped, Unsupported, export, function_key_name, import, parse_function_key};
#[cfg(test)]
use convert::{MAX_NAME_CHARS, export_item, normalise};
#[cfg(test)]
use descriptors::{
    PCT, PT, PX, ang, class, class_ref, code, color_desc, color_hex, desc, enm, get_bool, get_enum, get_list, get_num, get_obj, get_ref, layer_ref, long,
    name_ref, path_text, path_value, pct, px, target_ref, text, trimmed,
};
use descriptors::{jbool, jstr};
#[cfg(test)]
use events::{EVENTS, event_and_descriptor, event_for};
#[cfg(test)]
use fields::Mapped;
use files::{load, save};
#[cfg(test)]
use steps_doc::crop_from;
#[cfg(test)]
use steps_image::{apply_image_from, transform_to};
#[cfg(test)]
use steps_layer::quick_mask_ref;
#[cfg(test)]
use steps_make::make_from;
#[cfg(test)]
use tables::{blend_label, blend_ps, channel_ref};

/// A step PhotoCraft cannot play yet, kept verbatim (`{"event", "name", "descriptor": base64}`).
pub const PHOTOSHOP_STEP: &str = "actions.photoshopStep";
/// Photoshop's Insert Stop: `{"message", "allowContinue"}`.
pub const STOP_MESSAGE: &str = "actions.stopMessage";
/// The set name Save Actions uses for actions that came from no set.
pub const DEFAULT_SET: &str = "PhotoCraft Actions";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn stop_message(_: &mut Session, p: &Value) -> Result<Value> {
    let message = jstr(p, "message").unwrap_or("Stop").to_string();
    if jbool(p, "allowContinue").unwrap_or(false) {
        Ok(json!({"message": message, "continued": true}))
    } else {
        Err(EngineError::Other(format!("stopped by the action: {message}")))
    }
}

fn photoshop_step_run(_: &mut Session, p: &Value) -> Result<Value> {
    let name = jstr(p, "name").filter(|n| !n.is_empty()).unwrap_or("this step");
    let event = jstr(p, "event").unwrap_or("?");
    Err(EngineError::Other(format!("Photoshop step \"{name}\" ({event}) has no PhotoCraft equivalent yet; switch it off in the Actions panel to skip it")))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.load",
            label: "Load Actions…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".atn file"?,"data":base64 bytes?,"set":name? (override the file's set name),"replace":bool=true (replace the actions of a set of the same name)} → {set, actions:[names], steps, unsupported:[{action, step, event, name}], functionKeys:[{action, key}], warnings}. Photoshop steps PhotoCraft cannot play become actions.photoshopStep steps (kept for Save Actions; they fail at playback with the step's name); tool selections and other steps with no document effect are imported switched off. Layer references by name replay in any document."##,
            enabled: always,
            run: load,
            journal: false,
        },
        CommandSpec {
            id: "actions.save",
            label: "Save Actions…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".atn file"? (else the bytes come back as base64 `data`),"set":name? (that set's actions; default: every action, as the set "PhotoCraft Actions"),"action":name|index? (one action),"skipUnsupported":bool=false (leave out steps with no Photoshop equivalent instead of failing)} → {set, actions:[names], steps, bytes, skipped:[{action, step, id}], path|data}"##,
            enabled: always,
            run: save,
            journal: false,
        },
        CommandSpec {
            id: STOP_MESSAGE,
            label: "Stop",
            menu: &[],
            shortcut: None,
            params: r##"{"message":str,"allowContinue":bool=false} (Photoshop's Insert Stop: a step that shows the message; headless playback continues when allowContinue is set and otherwise stops the action with the message)"##,
            enabled: always,
            run: stop_message,
            journal: true,
        },
        CommandSpec {
            id: PHOTOSHOP_STEP,
            label: "Photoshop Step",
            menu: &[],
            shortcut: None,
            params: r##"{"event":id,"name":str,"descriptor":base64?} (a step from a .atn file PhotoCraft has no command for: fails at playback, written back by actions.save)"##,
            enabled: always,
            run: photoshop_step_run,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod layer_props_tests;
