//! Record Again executes the selected command before replacing its stored settings.
use super::{Result, Session, Value, bad, index, json, not_playing, resolve};

const COMMAND: &str = "actions.recordAgain";

/// A validated selection shared by the engine and the panel's seeded editor.
pub struct RecordAgainTarget {
    pub action: usize,
    pub step: usize,
    pub command: String,
    pub params: Value,
}

/// Resolve only executable steps, without flushing recording or editing the library.
pub fn target(s: &Session, p: &Value) -> Result<RecordAgainTarget> {
    not_playing(s, COMMAND)?;
    if s.actions.recording.is_some() {
        return Err(bad(COMMAND, "stop recording before recording a step again"));
    }
    if let Some(revision) = p.get("revision") {
        let revision = revision.as_u64().ok_or_else(|| bad(COMMAND, "`revision` must be an unsigned integer"))?;
        if revision != s.actions.rev {
            return Err(bad(COMMAND, "the actions changed; select the step again"));
        }
    }
    let action = resolve(&s.actions.list, p, COMMAND)?;
    let step = index(p, "step", COMMAND)?.ok_or_else(|| bad(COMMAND, "select a step"))?;
    let (id, params) = s.actions.list.get(action).and_then(|a| a.steps.get(step)).ok_or_else(|| bad(COMMAND, "step is out of range"))?;
    let (command, params) =
        if id == crate::atn_cmds::MENU_ITEM { (crate::atn_cmds::menu_item_command(params)?.to_string(), json!({})) } else { (id.clone(), params.clone()) };
    if command.starts_with("actions.") || !crate::commands::find(&command).is_some_and(|c| c.journal) {
        return Err(bad(COMMAND, "the selected step has no editable engine command"));
    }
    Ok(RecordAgainTarget { action, step, command, params })
}

/// Keep the original definition on invalid settings, denied execution or changed ownership.
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let selected = target(s, p)?;
    let params = p.get("params").filter(|p| p.is_object()).cloned().ok_or_else(|| bad(COMMAND, "supply the step's new settings in `params`"))?;
    let original = s.actions.list.get(selected.action).and_then(|a| a.steps.get(selected.step)).cloned().ok_or_else(|| bad(COMMAND, "step is out of range"))?;
    let revision = s.actions.rev;
    // Resolve portable recorded targets for execution; keep their portable form in the step.
    let resolved = crate::actions_cmds::playback_params(s, &selected.command, &params)?;
    let result = s.execute(&selected.command, resolved)?;
    if s.actions.rev != revision || s.actions.list.get(selected.action).and_then(|a| a.steps.get(selected.step)) != Some(&original) {
        return Err(bad(COMMAND, "the action changed during execution; its step was not overwritten"));
    }
    let action = s.actions.list.get_mut(selected.action).ok_or_else(|| bad(COMMAND, "the action no longer exists"))?;
    let step = action.steps.get_mut(selected.step).ok_or_else(|| bad(COMMAND, "the step no longer exists"))?;
    *step = (selected.command.clone(), params);
    let name = action.name.clone();
    s.actions.touch();
    Ok(json!({"action": name, "step": selected.step, "id": selected.command, "result": result}))
}
