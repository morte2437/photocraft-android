//! Prepare the complete archive before publishing a whole Actions library.
use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use crate::actions_cmds::{Action, PLAY_SHORTCUT_PREFIX, branch_id_field};
use crate::actions_edit_cmds::{check_library_size, confirmed_idle, publish_library};
use crate::{Result, Session};

use super::super::bad;
use super::super::convert::{ReplacementSet, UniqueNames, Unsupported, import_sets};

const COMMAND: &str = "actions.replace";

struct Replacement {
    list: Vec<Action>,
    rows: Vec<String>,
    keys: BTreeMap<String, String>,
    warnings: Vec<String>,
    unsupported: Vec<Unsupported>,
}

impl Replacement {
    fn parse(bytes: &[u8], encoding: Option<&str>) -> Result<Self> {
        let mut list = Vec::new();
        let mut rows = Vec::new();
        let mut keys = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut unsupported = Vec::new();
        let mut names = UniqueNames::new(std::iter::empty::<String>());
        let mut first = HashMap::new();
        let mut scoped = HashMap::new();
        let mut local = HashMap::new();
        let mut generated = HashMap::new();
        for (ReplacementSet { mut imported, keys: function_keys }, unsupported_rows) in import_sets(bytes, encoding).map_err(|error| bad(COMMAND, error))? {
            let slot = rows.len();
            for action in &mut imported.actions {
                let name = names.claim(&action.name);
                let original = std::mem::replace(&mut action.name, name);
                if original != action.name {
                    generated.insert(action.name.clone(), imported.set.clone());
                }
                first.entry(original.clone()).or_insert_with(|| action.name.clone());
                scoped.entry((imported.set.clone(), original.clone())).or_insert_with(|| action.name.clone());
                local.entry((slot, original)).or_insert_with(|| action.name.clone());
                action.set_slot = Some(slot);
            }
            for (index, key) in function_keys {
                if let Some(action) = imported.actions.get(index) {
                    keys.retain(|_, assigned| assigned != &key);
                    keys.insert(format!("{PLAY_SHORTCUT_PREFIX}{}", action.name), key);
                }
            }
            for (diagnostic, row) in imported.unsupported.iter_mut().zip(unsupported_rows) {
                diagnostic.action.clone_from(&imported.actions[row].name);
            }
            list.extend(imported.actions);
            rows.push(imported.set);
            warnings.extend(imported.warnings);
            unsupported.extend(imported.unsupported);
        }
        for action in &mut list {
            for (id, params) in &mut action.steps {
                let named_automation = matches!(id.as_str(), "file.automate.batch" | "file.automate.createDroplet");
                let fields: &[&str] = match id.as_str() {
                    "actions.play" => &["action"],
                    "actions.conditional" => &["then", "else"],
                    _ if named_automation && params.get("steps").is_none() => &["action"],
                    _ => &[],
                };
                for field in fields {
                    let Some(name) = params.get(*field).and_then(Value::as_str) else { continue };
                    let scope = match *field {
                        "then" => params.get("thenSet").filter(|value| !value.is_null()),
                        "else" => params.get("elseSet").filter(|value| !value.is_null()),
                        _ => None,
                    }
                    .or_else(|| params.get("set").filter(|value| !value.is_null()));
                    // An unqualified call in this archive first looks in its original folder.
                    // Legacy callers outside this import still use their existing name lookup.
                    let renamed = match scope {
                        None => action.set_slot.and_then(|slot| local.get(&(slot, name.to_string()))).or_else(|| first.get(name)),
                        Some(value) => value.as_str().and_then(|set| {
                            let row = action.set_slot.filter(|_| action.set.as_deref() == Some(set));
                            row.and_then(|slot| local.get(&(slot, name.to_string()))).or_else(|| scoped.get(&(set.to_string(), name.to_string())))
                        }),
                    };
                    if let Some(renamed) = renamed {
                        params[*field] = json!(renamed);
                    } else if (!named_automation && scope.is_some())
                        || generated.get(name).is_some_and(|set| !named_automation || scope.is_none_or(|requested| requested.as_str() == Some(set.as_str())))
                    {
                        // An unresolved scoped call or generated caption must not fall back
                        // to another action. Keep the displayed name and block playback.
                        let identity = if *field == "action" { "actionId" } else { branch_id_field(field) };
                        if params.get(identity).is_none() {
                            params[identity] = Value::Null;
                        }
                    }
                }
            }
        }
        check_library_size(&list, &[], &rows, COMMAND)?;
        Ok(Self { list, rows, keys, warnings, unsupported })
    }
}

pub fn validate(bytes: &[u8]) -> std::result::Result<usize, String> {
    Replacement::parse(bytes, None).map(|prepared| prepared.list.len()).map_err(|error| error.to_string())
}

pub(in super::super) fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    confirmed_idle(s, p, COMMAND)?;
    if p.get("set").is_some() || p.get("replace").is_some() {
        return Err(bad(COMMAND, "set overrides and same-set replacement belong to actions.load"));
    }
    let encoding = match p.get("encoding") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value == "auto" => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
        _ => return Err(bad(COMMAND, "encoding must be auto or a non-empty label")),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, COMMAND, ".atn file")?;
    let Replacement { mut list, rows, keys, warnings, unsupported } = Replacement::parse(&bytes, encoding)?;
    // Whole-library imports keep their automation origin just as actions.load imports do.
    for action in &mut list {
        action.automation = s.authorize.is_some();
    }
    let result = json!({"sets":rows,"actions":list.iter().map(|a| &a.name).collect::<Vec<_>>(),"warnings":warnings,"unsupported":unsupported});
    publish_library(s, list, Vec::new(), rows, Some(keys), "Undo Replace Actions", COMMAND)?;
    Ok(result)
}
