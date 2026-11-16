use super::descriptors::{desc, get_obj, get_ref, jstr};
use super::events::{event_and_descriptor, event_for};
use super::fields::Mapped;
use super::{Action, AtnAction, AtnEvent, AtnFile, AtnItem, D, DEFAULT_SET, Descriptor, PHOTOSHOP_STEP, ReferenceItem, UnicodeString, Value, atn, json};

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
    /// The encoding the file's 8-bit step names were read in (`"GBK"`, `"Shift_JIS"`,
    /// `"macintosh"`…); `None` when none needed one.
    pub encoding: Option<String>,
}

fn photoshop_step(item: &AtnItem, label: String) -> (String, Value) {
    let descriptor = item.descriptor.as_ref().map(|d| photocraft_paint::tile::b64_encode(&d.to_bytes()));
    let event_kind = match &item.event {
        AtnEvent::Code(_) => "code",
        AtnEvent::Name(_) => "name",
    };
    (PHOTOSHOP_STEP.into(), json!({"event": item.event.id(), "eventKind": event_kind, "name": label, "descriptor": descriptor}))
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

/// Whole-library replacement resolves these explicit scopes against the archive.
fn retain_action_scopes(d: &Descriptor, id: &str, params: &mut Value) {
    let fields: &[(&str, &str)] = match id {
        "actions.play" => &[("null", "set")],
        "actions.conditional" => &[("then", "thenSet"), ("else", "elseSet")],
        _ => return,
    };
    for (reference, scope) in fields {
        if let Some([ReferenceItem::Name { class: action, .. }, set, ..]) = get_ref(d, reference)
            && action.class_id.is("Actn")
        {
            match set {
                ReferenceItem::Name { class, name } if class.class_id.is("ASet") => {
                    params[*scope] = json!(clip_name(&name.to_string_lossy()));
                }
                // An indexed set is explicit, but its archive occurrence is unknown.
                ReferenceItem::Index { class, index } if class.class_id.is("ASet") => {
                    params[*scope] = json!(index);
                }
                _ => {}
            }
        }
    }
}

fn convert_action(file: &AtnFile, a: &AtnAction, set: &str, unsupported: &mut Vec<Unsupported>, replacing: bool) -> Action {
    let name = clip_name(&a.name.to_string_lossy());
    let mut action = Action::new(name.clone(), Vec::new());
    action.set = Some(clip_name(set));
    for (i, item) in a.items.iter().enumerate() {
        let menu = item.descriptor.is_none().then(|| super::menu_item::from_event(&item.event)).flatten();
        let mapped = match (menu, event_for(&item.event), &item.descriptor) {
            (Some(params), _, _) => Mapped::Steps(vec![(super::MENU_ITEM.into(), params)]),
            (_, Some(e), Some(d)) => (e.from)(d),
            (_, Some(e), None) => (e.from)(&Descriptor::default()),
            _ => Mapped::Unsupported { harmless: false },
        };
        let (new_steps, enabled) = match mapped {
            Mapped::Steps(mut s) => {
                super::steps_gallery::remember_event(item, &mut s);
                super::steps_stack::remember_item(item, &mut s);
                super::steps_video::remember(item, &mut s);
                (s, item.enabled)
            }
            Mapped::Unsupported { harmless } => {
                let label = file.item_label(item);
                unsupported.push(Unsupported { action: name.clone(), step: i, event: item.event.id(), name: label.clone() });
                (vec![photoshop_step(item, label)], item.enabled && !harmless)
            }
        };
        for mut s in new_steps {
            if replacing && let Some(descriptor) = &item.descriptor {
                retain_action_scopes(descriptor, &s.0, &mut s.1);
            }
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
    import_with_encoding(bytes, None)
}

/// [`import`], reading 8-bit step names in `encoding` (a label such as `"shift_jis"` or
/// `"gbk"`) instead of working it out. An unknown label fails.
pub fn import_with_encoding(bytes: &[u8], encoding: Option<&str>) -> std::result::Result<Imported, String> {
    let file = atn::parse_with_encoding(bytes, encoding).map_err(|e| e.to_string())?;
    Ok(convert_set(&file, None))
}

pub(super) struct ReplacementSet {
    pub imported: Imported,
    pub keys: Vec<(usize, String)>,
}

pub(super) fn import_first_with_keys(bytes: &[u8], encoding: Option<&str>) -> std::result::Result<ReplacementSet, String> {
    let file = atn::parse_with_encoding(bytes, encoding).map_err(|error| error.to_string())?;
    Ok(convert_with_keys(&file, None))
}

pub(super) fn import_sets(bytes: &[u8], encoding: Option<&str>) -> std::result::Result<Vec<(ReplacementSet, Vec<usize>)>, String> {
    let files = atn::parse_sets_with_encoding(bytes, encoding).map_err(|error| error.to_string())?;
    if bytes.starts_with(&12u32.to_be_bytes()) && files.iter().any(|file| file.warnings.iter().any(|warning| warning.ends_with(" trailing bytes ignored"))) {
        return Err("trailing data after the legacy Actions set".to_string());
    }
    Ok(files
        .iter()
        .map(|file| {
            let mut unsupported_rows = Vec::new();
            let set = convert_with_keys(file, Some(&mut unsupported_rows));
            (set, unsupported_rows)
        })
        .collect())
}

fn convert_with_keys(file: &AtnFile, unsupported_rows: Option<&mut Vec<usize>>) -> ReplacementSet {
    let keys = file
        .actions
        .iter()
        .enumerate()
        .filter(|(_, action)| action.function_key > 0)
        .map(|(index, action)| (index, function_key_name(action.function_key, action.shift, action.command)))
        .collect();
    ReplacementSet { imported: convert_set(file, unsupported_rows), keys }
}

fn convert_set(file: &AtnFile, mut unsupported_rows: Option<&mut Vec<usize>>) -> Imported {
    let set = file.name.to_string_lossy();
    let set = if set.trim().is_empty() { DEFAULT_SET.to_string() } else { clip_name(&set) };
    let mut out = Imported { set: set.clone(), warnings: file.warnings.clone(), encoding: file.encoding.clone(), ..Default::default() };
    for (row, a) in file.actions.iter().enumerate() {
        let action = convert_action(file, a, &set, &mut out.unsupported, unsupported_rows.is_some());
        if let Some(owners) = unsupported_rows.as_deref_mut() {
            owners.resize(out.unsupported.len(), row);
        }
        if a.function_key > 0 {
            out.function_keys.push((action.name.clone(), function_key_name(a.function_key, a.shift, a.command)));
        }
        out.actions.push(action);
    }
    out
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
    if id == super::MENU_ITEM {
        return super::menu_item::export(params);
    }
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
    if (id == "actions.play" || id == "actions.conditional") && params.get("set").is_none_or(Value::is_null) {
        params["set"] = json!(set);
    }
    let (e, descriptor) = event_and_descriptor(id, &params)?;
    // A Filter Gallery effect loaded from its own event saves back as that event (#10592).
    if e.code == "GEfc"
        && let Some((event, name)) = super::steps_gallery::recorded_event(id, &params)
    {
        return Some(AtnItem { event, dictionary: name, descriptor: Some(descriptor), ..Default::default() });
    }
    let event = if id == "path.set" || e.code.is_empty() { AtnEvent::name(e.name) } else { AtnEvent::Code(e.code.as_bytes().try_into().ok()?) };
    let omit_descriptor = super::steps_stack::without_descriptor(id, &params) || super::steps_video::omitted(id, &params);
    let descriptor = (!omit_descriptor).then_some(descriptor);
    Some(AtnItem { event, dictionary: e.label.as_bytes().to_vec(), descriptor, ..Default::default() })
}

/// Photoshop applies only the rename when a Set mixes it with opacity or blend mode (#8304):
/// write each layer property separately, renaming last so named references still resolve. A
/// `.atn` reference is also one target, so a `layer.setProps` step that named `layers` exports
/// as one Set per layer, each referenced by Photoshop's layer id: an edit that accepted the
/// list must not replay on the active layer alone (#7246). The list is split into one-target
/// params before the step maps, so the keyed check that refuses the keys the `.atn` adapters
/// cannot write (#7196) sees a mapped one. A step unable to represent the whole list exactly
/// (an id beyond `Idnt`'s u32; a rename of several layers, which PhotoCraft itself refuses)
/// comes back as `None`, to refuse or skip the source step.
fn export_items(id: &str, params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    if id == "layer.setProps" {
        if let Some(Value::Array(ids)) = params.get("layers") {
            return layer_set_items(ids, params, set);
        }
        if params.get("layers").map(|v| !v.is_null()) == Some(true) {
            return None;
        }
    }
    let item = export_item(id, params, set)?;
    if id != "layer.setProps" {
        return Some(vec![item]);
    }
    item_property_split(item)
}

/// One Set per layer of an explicit `layers` list, by Photoshop's ever-present layer id.
fn layer_set_items(ids: &[Value], params: &Value, set: &str) -> Option<Vec<AtnItem>> {
    if ids.is_empty() || (params.get("name").is_some() && ids.len() > 1) {
        return None;
    }
    let mut items = Vec::new();
    for v in ids {
        let layer = v.as_u64().filter(|n| *n <= u32::MAX as u64)?;
        let mut p = params.clone();
        let obj = p.as_object_mut()?;
        obj.remove("layers");
        obj.insert("layer".into(), Value::Number(serde_json::Number::from(layer)));
        items.extend(item_property_split(export_item("layer.setProps", &p, set)?)?);
    }
    Some(items)
}

/// One item per property of one `Lyr ` target, renaming last so named references resolve
/// when the whole Set replays (#8304).
fn item_property_split(item: AtnItem) -> Option<Vec<AtnItem>> {
    let descriptor = item.descriptor.as_ref()?;
    let properties = get_obj(descriptor, "T   ")?;
    if properties.items.len() <= 1 {
        return Some(vec![item]);
    }
    let target = descriptor.get("null")?.clone();
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

/// Resolve export targets once; repeated calls never scan the selected library again.
struct ExportTargets<'a> {
    actions: &'a [&'a Action],
    ids: std::collections::HashMap<&'a str, Option<usize>>,
    clipped: std::collections::HashMap<String, usize>,
    names: std::collections::HashSet<&'a str>,
    scopes: std::collections::HashSet<(&'a str, &'a str)>,
}

impl<'a> ExportTargets<'a> {
    fn new(actions: &'a [&'a Action]) -> Self {
        let mut out = Self { actions, ids: Default::default(), clipped: Default::default(), names: Default::default(), scopes: Default::default() };
        for (index, action) in actions.iter().enumerate() {
            if let Some(id) = action.action_id.as_deref() {
                out.ids.entry(id).and_modify(|row| *row = None).or_insert(Some(index));
            }
            *out.clipped.entry(clip_name(action.name.trim_end_matches('\0'))).or_default() += 1;
            out.names.insert(&action.name);
            if let Some(set) = action.set.as_deref() {
                out.scopes.insert((set, &action.name));
            }
        }
        out
    }

    fn by_id(&self, id: &Value) -> Option<&Action> {
        let id = id.as_str().filter(|id| crate::actions_cmds::valid_action_id(id))?;
        let action = self.actions[(*self.ids.get(id)?)?];
        (self.clipped.get(&clip_name(action.name.trim_end_matches('\0'))) == Some(&1)).then_some(action)
    }

    fn internal(&self, name: &str, scope: Option<&str>) -> bool {
        scope.map_or_else(|| self.names.contains(name), |scope| self.scopes.contains(&(scope, name)))
    }
}

/// ATN uses names for these calls. Only translate an ID when that name identifies
/// the same exported row; an absent or ambiguous target is an unsupported step.
fn export_targeted_items(id: &str, params: &Value, set: &str, targets: &ExportTargets<'_>) -> Option<Vec<AtnItem>> {
    let fields: &[(&str, &str, &str)] = match id {
        "actions.play" => &[("actionId", "action", "set")],
        "actions.conditional" => &[("thenActionId", "then", "thenSet"), ("elseActionId", "else", "elseSet")],
        _ => return export_items(id, params, set),
    };
    let mut params = params.clone();
    for (key, name, scope) in fields {
        if let Some(value) = params.get(*key) {
            let action = targets.by_id(value)?;
            params[*name] = json!(action.name);
            params[*scope] = json!(set);
        } else if let Some(target) = params.get(*name).and_then(Value::as_str) {
            let original_scope = params.get(*scope).filter(|value| !value.is_null()).or_else(|| params.get("set")).and_then(Value::as_str);
            if targets.internal(target, original_scope) {
                params[*scope] = json!(set);
            }
        }
    }
    export_items(id, &params, set)
}

/// The `.atn` bytes of `actions` as the set `set`. `function_keys` gives each action's key
/// (`F1`…`F15`, with `Shift+` and `Cmd+` as [`function_key_name`] writes them). Steps with no Photoshop equivalent are an error unless `skip_unsupported`.
pub fn export(
    set: &str,
    actions: &[&Action],
    function_keys: &[(String, String)],
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    export_with_keys(set, actions, |_, action| function_keys.iter().find(|(name, _)| name == &action.name).map(|(_, key)| key.as_str()), skip_unsupported)
}

pub(super) fn export_with_keys<'a>(
    set: &str,
    actions: &[&Action],
    key: impl Fn(usize, &Action) -> Option<&'a str>,
    skip_unsupported: bool,
) -> std::result::Result<(Vec<u8>, Vec<Skipped>), String> {
    let targets = ExportTargets::new(actions);
    let mut skipped = Vec::new();
    let mut file = AtnFile { name: UnicodeString::new_nul(set), expanded: true, actions: Vec::new(), warnings: Vec::new(), ..Default::default() };
    for (index, a) in actions.iter().enumerate() {
        let mut out = AtnAction { name: UnicodeString::new_nul(&a.name), expanded: false, ..Default::default() };
        if let Some((n, shift, command)) = key(index, a).and_then(parse_function_key) {
            (out.function_key, out.shift, out.command) = (n, shift, command);
        }
        for (i, (id, params)) in a.steps.iter().enumerate() {
            let Some(items) = export_targeted_items(id, params, set, &targets) else {
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
pub(crate) struct UniqueNames {
    taken: std::collections::HashSet<String>,
    next_suffix: std::collections::HashMap<String, u32>,
}

impl UniqueNames {
    pub(crate) fn new<I: IntoIterator<Item = String>>(existing: I) -> Self {
        UniqueNames { taken: existing.into_iter().collect(), next_suffix: std::collections::HashMap::new() }
    }

    /// Reserves and returns a unique name derived from `name`, recording it as taken.
    pub(crate) fn claim(&mut self, name: &str) -> String {
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
