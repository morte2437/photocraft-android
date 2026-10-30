use super::descriptors::{desc, get_obj, jstr};
use super::events::{event_and_descriptor, event_for};
use super::fields::Mapped;
use super::{Action, AtnAction, AtnEvent, AtnFile, AtnItem, D, DEFAULT_SET, Descriptor, PHOTOSHOP_STEP, UnicodeString, Value, atn, json};

// ------------------------------------------------------------------ import

/// A step with no PhotoCraft equivalent.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Unsupported {
    pub action: String,
    pub step: usize,
    pub event: String,
    pub name: String,
}

/// What a `.atn` file imports to.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub set: String,
    pub actions: Vec<Action>,
    /// Function keys the file assigns, as `(action name, key)`.
    pub function_keys: Vec<(String, String)>,
    pub unsupported: Vec<Unsupported>,
    pub warnings: Vec<String>,
}

fn photoshop_step(item: &AtnItem) -> (String, Value) {
    let descriptor = item.descriptor.as_ref().map(|d| photocraft_paint::tile::b64_encode(&d.to_bytes()));
    let event_kind = match &item.event {
        AtnEvent::Code(_) => "code",
        AtnEvent::Name(_) => "name",
    };
    (PHOTOSHOP_STEP.into(), json!({"event": item.event.id(), "eventKind": event_kind, "name": item.label(), "descriptor": descriptor}))
}

/// Integral floats become integers (`2.0` → `2`), as the commands' own docs and recordings
/// use, so an imported step reads and compares like a recorded one.
pub(super) fn normalise(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && f.abs() < 9.0e15
                && n.as_i64().is_none()
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalise),
        Value::Object(o) => o.values_mut().for_each(normalise),
        _ => {}
    }
}

/// Longest action or set name kept (Photoshop's own limit is 255). A file's names are lengths it
/// chooses, and each is copied into every action and every unsupported-step diagnostic.
pub(super) const MAX_NAME_CHARS: usize = 1024;

pub(super) fn clip_name(s: &str) -> String {
    s.chars().take(MAX_NAME_CHARS).collect()
}

fn convert_action(a: &AtnAction, set: &str, unsupported: &mut Vec<Unsupported>) -> Action {
    let name = clip_name(&a.name.to_string_lossy());
    let mut action = Action::new(name.clone(), Vec::new());
    action.set = Some(clip_name(set));
    for (i, item) in a.items.iter().enumerate() {
        let mapped = match (event_for(&item.event), &item.descriptor) {
            (Some(e), Some(d)) => (e.from)(d),
            (Some(e), None) => (e.from)(&Descriptor::default()),
            (None, _) => Mapped::Unsupported { harmless: false },
        };
        let (new_steps, enabled) = match mapped {
            Mapped::Steps(s) => (s, item.enabled),
            Mapped::Unsupported { harmless } => {
                unsupported.push(Unsupported { action: name.clone(), step: i, event: item.event.id(), name: item.label() });
                (vec![photoshop_step(item)], item.enabled && !harmless)
            }
        };
        for mut s in new_steps {
            normalise(&mut s.1);
            let idx = action.steps.len();
            action.steps.push(s);
            if !enabled {
                action.disabled.insert(idx);
            }
            if item.with_dialog {
                action.dialogs.insert(idx);
            }
        }
    }
    action
}

/// Convert a `.atn` file. Fails only when the file is unreadable: every step imports, as a
/// command or as a kept Photoshop step.
pub fn import(bytes: &[u8]) -> std::result::Result<Imported, String> {
    let file = atn::parse(bytes).map_err(|e| e.to_string())?;
    let set = file.name.to_string_lossy();
    let set = if set.trim().is_empty() { DEFAULT_SET.to_string() } else { clip_name(&set) };
    let mut out = Imported { set: set.clone(), warnings: file.warnings.clone(), ..Default::default() };
    for a in &file.actions {
        let action = convert_action(a, &set, &mut out.unsupported);
        if a.function_key > 0 {
            out.function_keys.push((action.name.clone(), function_key_name(a.function_key, a.shift, a.command)));
        }
        out.actions.push(action);
    }
    Ok(out)
}

// ------------------------------------------------------------------ export

/// A step Save Actions could not express as a Photoshop event.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Skipped {
    pub action: String,
    pub step: usize,
    pub id: String,
}

pub(super) fn export_item(id: &str, params: &Value, set: &str) -> Option<AtnItem> {
    if id == PHOTOSHOP_STEP {
        let event = jstr(params, "event")?;
        let event = match params.get("eventKind") {
            Some(Value::String(kind)) if kind == "code" => AtnEvent::Code(event.as_bytes().try_into().ok()?),
            Some(Value::String(kind)) if kind == "name" => AtnEvent::name(event),
            Some(_) => return None,
            // Older PhotoCraft action data did not retain the event kind. Keep its previous
            // best-effort export behavior for compatibility.
            None => match event.as_bytes() {
                [a, b, c, d] => AtnEvent::Code([*a, *b, *c, *d]),
                _ => AtnEvent::name(event),
            },
        };
        let descriptor = match jstr(params, "descriptor") {
            Some(b64) => Some(Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(b64)?).ok()?),
            None => None,
        };
        return Some(AtnItem { event, dictionary: jstr(params, "name").unwrap_or("").as_bytes().to_vec(), descriptor, ..Default::default() });
    }
    let mut params = params.clone();
    if (id == "actions.play" || id == "actions.conditional") && params.get("set").is_none() {
        params["set"] = json!(set);
    }
    let (e, descriptor) = event_and_descriptor(id, &params)?;
    let event = if e.code.is_empty() { AtnEvent::name(e.name) } else { AtnEvent::Code(e.code.as_bytes().try_into().ok()?) };
    Some(AtnItem { event, dictionary: e.label.as_bytes().to_vec(), descriptor: Some(descriptor), ..Default::default() })
}

/// Photoshop applies only the rename when a Set mixes it with opacity or blend mode (#8304).
/// Write each layer property separately, renaming last so named references still resolve.
fn export_items(id: &str, params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    let item = export_item(id, params, set)?;
    if id != "layer.setProps" {
        return Some(vec![item]);
    }
    let descriptor = item.descriptor.as_ref()?;
    let properties = get_obj(descriptor, "T   ")?;
    if properties.items.len() <= 1 {
        return Some(vec![item]);
    }
    let target = descriptor.get("null")?;
    let mut items = Vec::with_capacity(properties.items.len());
    for (key, value) in properties.items.iter().filter(|(k, _)| !k.is("Nm  ")).chain(properties.items.iter().filter(|(k, _)| k.is("Nm  "))) {
        let mut property = properties.clone();
        property.items = vec![(key.clone(), value.clone())];
        let descriptor = desc("null").with("null", target.clone()).with("T   ", D::Descriptor(property));
        items.push(AtnItem { descriptor: Some(descriptor), ..item.clone() });
    }
    Some(items)
}

/// An action's key as a shortcut string: `F5`, with Photoshop's Shift and Command options
/// `Shift+F5`, `Cmd+F5` or `Cmd+Shift+F5` (#7646).
pub fn function_key_name(key: u16, shift: bool, command: bool) -> String {
    format!("{}{}F{key}", if command { "Cmd+" } else { "" }, if shift { "Shift+" } else { "" })
}

/// The `.atn` function key (1…15), Shift and Command of a shortcut string, or `None` for
/// anything else.
pub fn parse_function_key(key: &str) -> Option<(u16, bool, bool)> {
    let (mut shift, mut command) = (false, false);
    let mut rest = key;
    loop {
        if let Some(r) = rest.strip_prefix("Cmd+").filter(|_| !command) {
            (command, rest) = (true, r);
        } else if let Some(r) = rest.strip_prefix("Shift+").filter(|_| !shift) {
            (shift, rest) = (true, r);
        } else {
            break;
        }
    }
    let n = rest.strip_prefix('F')?.parse::<u16>().ok().filter(|n| (1..=15).contains(n))?;
    Some((n, shift, command))
}

/// The `.atn` bytes of `actions` as the set `set`. `function_keys` gives each action's key
/// (`F1`…`F15`, with `Shift+` and `Cmd+` as [`function_key_name`] writes them). Steps with no Photoshop equivalent are an error unless `skip_unsupported`.
pub fn export(
    set: &str,
    actions: &[&Action],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    let mut skipped = Vec::new();
    let mut file = AtnFile { name: UnicodeString::new_nul(set), expanded: true, actions: Vec::new(), warnings: Vec::new() };
    for a in actions {
        let mut out = AtnAction { name: UnicodeString::new_nul(&a.name), expanded: false, ..Default::default() };
        if let Some((n, shift, command)) = function_keys.iter().find(|(n, _)| n == &a.name).and_then(|(_, key)| parse_function_key(key)) {
            (out.function_key, out.shift, out.command) = (n, shift, command);
        }
        for (i, (id, params)) in a.steps.iter().enumerate() {
            let Some(items) = export_items(id, params, set) else {
                if skip_unsupported {
                    skipped.push(Skipped { action: a.name.clone(), step: i, id: id.clone() });
                    continue;
                }
                let step_name = match params.get("layer") {
                    Some(Value::Number(n)) => format!("`{id}` with numeric layer {n}"),
                    _ => format!("`{id}`"),
                };
                return Err(format!(
                    "step {} of `{}` ({step_name}) has no Photoshop equivalent; pass \"skipUnsupported\": true to leave it out",
                    i + 1,
                    a.name
                ));
            };
            for mut item in items {
                item.enabled = a.step_enabled(i);
                item.with_dialog = a.dialogs.contains(&i);
                item.dialog_options = if item.with_dialog { 2 } else { 0 };
                out.items.push(item);
            }
        }
        file.actions.push(out);
    }
    let bytes = atn::write(&file).map_err(|e| e.to_string())?;
    Ok((bytes, skipped))
}

// ------------------------------------------------------------------ commands

/// Assigns names not already used, as `name`, `name 2`, `name 3`… A plain linear scan of the
/// taken list per candidate is cubic when an action file repeats one base name many times, so
/// membership uses a set and the next suffix to try is cached per base name.
pub(super) struct UniqueNames {
    taken: std::collections::HashSet<String>,
    next_suffix: std::collections::HashMap<String, u32>,
}

impl UniqueNames {
    pub(super) fn new<I: IntoIterator<Item = String>>(existing: I) -> Self {
        UniqueNames { taken: existing.into_iter().collect(), next_suffix: std::collections::HashMap::new() }
    }

    /// Reserves and returns a unique name derived from `name`, recording it as taken.
    pub(super) fn claim(&mut self, name: &str) -> String {
        if self.taken.insert(name.to_string()) {
            return name.to_string();
        }
        let mut i = self.next_suffix.get(name).copied().unwrap_or(2);
        loop {
            let candidate = format!("{name} {i}");
            i += 1;
            if self.taken.insert(candidate.clone()) {
                self.next_suffix.insert(name.to_string(), i);
                return candidate;
            }
        }
    }
}
