//! Interactive opening confirmations: damaged PSD data and Color Settings questions.
//! A decoded file is held back while its standard, agent-accessible dialog asks what to do.
//! Damage confirmation comes first; Continue admits the file normally once any profile question
//! is answered, with no persistent save restriction. Cancelling damage discards just that file;
//! cancelling a profile question retains its existing behaviour of cancelling the whole queue.
//! Files that finish reading while a prompt is up wait their turn.

use std::sync::Arc;

use photocraft_doc::{ColorMode, Document};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The field that marks a prompt as this module's.
const FIELD: &str = "__heldOpen";
/// A regular dialog: agents see it in `ui.inspect` and answer through
/// `ui.dialog.confirm` / `ui.dialog.cancel`, just like the profile prompt.
const DAMAGE_FIELD: &str = "__psdDamagedOpen";

/// A file read and decoded, waiting to open.
pub struct Held {
    pub doc: Document,
    /// The source is read-only (an Affinity file): Save doesn't write back to it.
    pub read_only: bool,
    /// Ask whether to continue before adding a damaged PSD or showing its profile prompt.
    /// Discarded after the answer: it does not become a document or save restriction.
    pub damage_pending: bool,
    pub name: String,
    pub path: Option<String>,
    pub warnings: Vec<String>,
    /// The file's bytes, for Camera Raw to develop from when there is no path.
    pub bytes: Option<Arc<Vec<u8>>>,
    /// The tab position a reopened file keeps.
    pub slot: Option<usize>,
    /// The file's place in a set opened together, when it is one of several
    /// ([`crate::jobs_ui::OpenOrder`]): kept while this file waits, so its tab still lands in the
    /// set's order.
    pub order: Option<crate::jobs_ui::OpenOrder>,
    /// The profile question ([`photocraft_engine::color_cmds::ColorState::open_question`]), or
    /// `None` to open without asking.
    pub question: Option<Value>,
    /// The Color Settings policy was already applied (a background open added it before it had
    /// to wait); otherwise opening applies it.
    pub policy_applied: bool,
    /// Held by Open as Smart Object: the document holds the smart object layer already, and is
    /// admitted by [`photocraft_engine::file_cmds::admit_as_smart_object`].
    pub smart_object: bool,
}

/// Dialog field for the high-memory confirmation prompt.
pub const MEMORY_FIELD: &str = "__psdMemoryOpen";

/// Item kind in the arrival-ordered open prompt queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptKind {
    Memory,
    Open,
}

/// An un-decoded PSD file whose preflight estimate requires user approval.
#[derive(Clone)]
pub struct MemoryPrompt {
    pub id: u64,
    pub name: String,
    pub path: Option<String>,
    pub slot: Option<usize>,
    pub order: Option<crate::jobs_ui::OpenOrder>,
    pub estimated: u64,
    /// The capacity measured when the estimate was made; `None` when it could not be measured.
    pub available: Option<u64>,
    /// The engine could not measure available memory for this estimate. No number is shown.
    pub capacity_unknown: bool,
    /// The estimate is above the unknown-capacity warning threshold (8 GiB), so the prompt also
    /// says the process or tab may run out of memory. Approval stays possible.
    pub uncertain_capacity_warning: bool,
}

/// Whether a file that just finished reading has to wait: one earlier in the queue does.
pub fn waiting(app: &PhotocraftApp) -> bool {
    !app.held_opens.is_empty() || !app.held_memory_prompts.is_empty()
}

/// Opens `held` now, or queues it behind the file whose prompt is up.
pub fn arrive(app: &mut PhotocraftApp, held: Held) {
    app.held_opens.push_back(held);
    app.prompt_order.push_back(PromptKind::Open);
    advance(app);
}

/// Whether any open prompt (damage, profile or high-memory) is currently on screen.
pub fn has_open_dialog(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.iter().any(|d| owns(&d.fields) || owns_damage(&d.fields) || owns_memory(&d.fields))
}

/// Opens queued files in order until one needs its prompt, which is shown if it isn't yet.
pub fn advance(app: &mut PhotocraftApp) {
    // Reconcile legacy callers that populated held queues directly.
    if app.prompt_order.is_empty() {
        app.prompt_order.extend(std::iter::repeat_n(PromptKind::Open, app.held_opens.len()));
        app.prompt_order.extend(std::iter::repeat_n(PromptKind::Memory, app.held_memory_prompts.len()));
    }
    if has_open_dialog(app) {
        return;
    }
    loop {
        match app.prompt_order.front() {
            None => return,
            Some(PromptKind::Memory) => {
                if let Some(front) = app.held_memory_prompts.front().cloned() {
                    show_memory(app, &front);
                    return;
                } else {
                    app.prompt_order.pop_front();
                }
            }
            Some(PromptKind::Open) => {
                let Some(front) = app.held_opens.front() else {
                    app.prompt_order.pop_front();
                    continue;
                };
                if front.damage_pending {
                    show_damage(app);
                    return;
                }
                if front.question.is_some() {
                    show(app);
                    return;
                }
                let Some(held) = app.held_opens.pop_front() else {
                    app.prompt_order.pop_front();
                    continue;
                };
                app.prompt_order.pop_front();
                let _ = open(app, held, None);
                if has_open_dialog(app) {
                    return;
                }
            }
        }
    }
}

/// Shows the Embedded Profile Mismatch (or missing profile) prompt for the first queued file.
fn show(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front() else { return };
    let Some(q) = &front.question else { return };
    // The mode reads as the catalogs name it (French "RVB"); the file name goes in last, so
    // nothing in it is taken for a placeholder.
    let mode = tl!(match front.doc.mode {
        ColorMode::Cmyk => "CMYK",
        ColorMode::Grayscale => "Gray",
        _ => "RGB",
    });
    let name = front.name.as_str();
    let working = crate::i18n::fmt(tl!("Working: {working}"), &[("working", q.get("working").and_then(Value::as_str).unwrap_or("?"))]);
    let missing = q.get("missing").and_then(Value::as_bool) == Some(true);
    // A mismatch names the two profiles on the choices they belong to (`prefs_ui::mismatch_body`).
    let message = if missing {
        let what = crate::i18n::fmt(tl!("The document “{name}” does not have an embedded {mode} profile."), &[("mode", mode), ("name", name)]);
        format!("{what}\n{working}")
    } else {
        crate::i18n::fmt(
            tl!("The document “{name}” has an embedded color profile that does not match the current {mode} working space."),
            &[("mode", mode), ("name", name)],
        )
    };
    let action = q.get("action").and_then(Value::as_str).unwrap_or("preserve").to_string();
    let names = json!({
        "embedded": q.get("embedded").and_then(Value::as_str).unwrap_or("?"),
        "working": q.get("working").and_then(Value::as_str).unwrap_or("?"),
        "policyKey": policy_key(front.doc.mode),
    });
    crate::prefs_ui::open_mismatch(app, &message, &action, missing, FIELD, names);
}

/// The Color Settings policy that answers this mode's files when Ask On Mismatch is off.
fn policy_key(mode: ColorMode) -> Option<&'static str> {
    match photocraft_engine::color_cmds::mode_space(mode) {
        photocraft_cms::ColorSpace::Rgb => Some("policyRgb"),
        photocraft_cms::ColorSpace::Cmyk => Some("policyCmyk"),
        photocraft_cms::ColorSpace::Gray => Some("policyGray"),
        _ => None,
    }
}

/// Whether `fields` are one of this module's prompts.
pub fn owns(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(FIELD)
}

/// The prompt's answer: `Some(action)` (OK) opens the first queued file with it and moves on to
/// the next; `None` (Cancel) drops every queued file and stops the reads still running.
pub fn answer(app: &mut PhotocraftApp, action: Option<&str>) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(action) = action else {
        let dropped = app.held_opens.len() + app.held_memory_prompts.len();
        let name = app.held_opens.front().map(|h| h.name.clone()).or_else(|| app.held_memory_prompts.front().map(|m| m.name.clone())).unwrap_or_default();
        for m in app.held_memory_prompts.drain(..) {
            app.session.cancel_pending_preflight(m.id);
        }
        app.session.clear_pending_preflights();
        app.held_opens.clear();
        app.prompt_order.clear();
        for job in app.jobs.opens.iter().map(|o| o.job).collect::<Vec<_>>() {
            app.session.cancel_job(job);
        }
        app.jobs.opens.clear();
        app.jobs.batches.clear();
        app.ui.dialogs.retain(|d| !owns(&d.fields) && !owns_damage(&d.fields) && !owns_memory(&d.fields));
        app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
        return Ok(json!({"cancelled": dropped}));
    };
    let Some(held) = app.held_opens.pop_front() else { return Err("no file is waiting to open".into()) };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
        app.prompt_order.remove(pos);
    }
    let r = open(app, held, Some(action));
    advance(app);
    crate::jobs_ui::end_batch(app);
    r
}

/// OK with "Remember this choice": `action` becomes the Color Settings policy for the file's
/// mode and Ask On Mismatch turns off, so files read from now on open without asking. Files
/// already waiting with a mismatch in the same mode were asked before that and get the same
/// answer; any other question still asks.
pub fn answer_remembering(app: &mut PhotocraftApp, action: &str) -> Result<Value, String> {
    if app.held_opens.front().is_some_and(|h| h.damage_pending) {
        return Err("answer the damaged PSD confirmation first".into());
    }
    let Some(key) = app.held_opens.front().and_then(|h| policy_key(h.doc.mode)) else { return answer(app, Some(action)) };
    let policy = match action {
        "convert" => "convert",
        "discard" => "off",
        _ => "preserve",
    };
    app.run("edit.colorSettings", json!({key: policy, "askOnMismatch": false}))?;
    let same = |h: &Held| {
        !h.damage_pending && policy_key(h.doc.mode) == Some(key) && h.question.as_ref().is_some_and(|q| q.get("missing").and_then(Value::as_bool) != Some(true))
    };
    let Some(held) = app.held_opens.pop_front() else { return Err("no file is waiting to open".into()) };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
        app.prompt_order.remove(pos);
    }
    let r = open(app, held, Some(action));
    while app.prompt_order.front() == Some(&PromptKind::Open) && app.held_opens.front().is_some_and(same) {
        if let Some(held) = app.held_opens.pop_front() {
            app.prompt_order.pop_front();
            // A failure is shown to the user (`open_failed`); the next file still opens.
            let _ = open(app, held, Some(action));
        }
    }
    advance(app);
    crate::jobs_ui::end_batch(app);
    r
}

/// Whether the fields belong to the damage confirmation (not the profile prompt).
pub(crate) fn owns_damage(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(DAMAGE_FIELD)
}

/// Shows the standard agent-visible confirmation for the first damaged PSD
/// waiting in the open queue, including its decoding warnings.
fn show_damage(app: &mut PhotocraftApp) {
    let Some(front) = app.held_opens.front().filter(|h| h.damage_pending) else { return };
    let message = format!(
        "{}\n\n{}\n\n{}",
        crate::i18n::fmt(tl!("The document “{name}” may be damaged."), &[("name", &front.name)]),
        tl!("Some layers, pixels or masks could not be read. Do you want to continue opening it?"),
        tl!("If you continue, saving over the original may permanently discard unreadable data. Save a copy to keep the original."),
    );
    let fields = [
        (DAMAGE_FIELD.into(), json!(true)),
        ("__label".into(), json!("Damaged Photoshop document")),
        ("name".into(), json!(front.name)),
        ("message".into(), json!(message)),
        ("warnings".into(), json!(front.warnings)),
    ]
    .into_iter()
    .collect();
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

/// Body of the standard modal; the same fields are visible to automation.
pub(crate) fn damage_body(ui: &mut egui::Ui, fields: &serde_json::Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
    if let Some(warnings) = fields.get("warnings").and_then(Value::as_array) {
        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
            for warning in warnings.iter().filter_map(Value::as_str) {
                ui.label(warning);
            }
        });
    }
}

/// Continue or discard this file only, then move the queue forward. Accepting does not
/// mark the document read-only: Save, Export, scripts and web downloads work normally.
pub(crate) fn answer_damage(app: &mut PhotocraftApp, accept: bool) -> Result<Value, String> {
    let Some(front) = app.held_opens.front_mut().filter(|h| h.damage_pending) else {
        return Err("no damaged PSD is waiting for confirmation".into());
    };
    let name = front.name.clone();
    if accept {
        front.damage_pending = false;
    } else {
        app.held_opens.pop_front();
        if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Open) {
            app.prompt_order.remove(pos);
        }
        app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
        app.ui.status_error = false;
    }
    advance(app);
    crate::jobs_ui::end_batch(app);
    Ok(json!({"name": name, "continued": accept}))
}

/// Opens a queued file, applying `action` (the prompt's answer) or else the Color Settings
/// policy, then does what any open does afterwards.
fn open(app: &mut PhotocraftApp, held: Held, action: Option<&str>) -> Result<Value, String> {
    if held.damage_pending {
        return Err("damaged PSD cannot be opened before confirmation".into());
    }
    let Held { mut doc, read_only, damage_pending: _, name, path, warnings, bytes, slot, order, question, policy_applied, smart_object } = held;
    let index = if question.is_some() || policy_applied {
        if let Some(action) = action
            && let Err(e) = app.session.color.resolve_open(&mut doc, action)
        {
            app.open_failed(&name, &e.to_string());
            return Err(e.to_string());
        }
        if smart_object { photocraft_engine::file_cmds::admit_as_smart_object(&mut app.session, doc) } else { app.session.add_document(doc, None) }
    } else if smart_object {
        // A damage-only hold has no colour question: apply the policy as the engine's open does,
        // then admit it as the smart object document it is (name, history label, snapshot).
        let ignore_exif = app.session.prefs().file_handling.ignore_exif_profile_tag;
        app.session.color.open_policy(&mut doc, ignore_exif);
        photocraft_engine::file_cmds::admit_as_smart_object(&mut app.session, doc)
    } else {
        app.session.open_document(doc, None).0
    };
    // add_document may remap a persisted ID already owned by another open document.
    let document = app.session.documents().get(index).map(|st| st.doc.id).ok_or("the opened document is missing")?;
    if let Some(st) = app.session.active_mut() {
        st.source_read_only = read_only;
    }
    let slot = crate::jobs_ui::tab_slot(app, slot, order);
    let index = match slot {
        Some(slot) => app
            .run("document.move", json!({"document": index, "to": slot}))
            .ok()
            .and_then(|m| m.get("document").and_then(Value::as_u64))
            .map_or(index, |i| i as usize),
        None => index,
    };
    app.session.set_active(index);
    crate::jobs_ui::after_open(app, document, &name, path.as_deref(), &warnings, bytes.as_deref().map(Vec::as_slice))?;
    Ok(json!({"document": index, "action": action}))
}

/// Queues a high-memory prompt in arrival order and shows it when it reaches the front.
#[allow(clippy::too_many_arguments)]
pub fn arrive_memory(
    app: &mut PhotocraftApp,
    id: u64,
    name: String,
    path: Option<String>,
    slot: Option<usize>,
    order: Option<crate::jobs_ui::OpenOrder>,
    estimated: u64,
    available: Option<u64>,
    capacity_unknown: bool,
    uncertain_capacity_warning: bool,
) {
    app.held_memory_prompts.push_back(MemoryPrompt { id, name, path, slot, order, estimated, available, capacity_unknown, uncertain_capacity_warning });
    app.prompt_order.push_back(PromptKind::Memory);
    advance_memory(app);
}

/// Whether a dialog's fields mark it as the high-memory confirmation.
pub fn owns_memory(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(MEMORY_FIELD)
}

/// Draws the high-memory prompt's message body from its dialog fields.
pub(crate) fn memory_body(ui: &mut egui::Ui, fields: &serde_json::Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
}

/// Advances the shared open-prompt queue; memory prompts take their turn with damage and profile
/// prompts in arrival order.
pub fn advance_memory(app: &mut PhotocraftApp) {
    advance(app);
}

/// Builds and opens the confirmation dialog for a queued high-memory preflight.
fn show_memory(app: &mut PhotocraftApp, prompt: &MemoryPrompt) {
    let est_gb = prompt.estimated as f64 / (1u64 << 30) as f64;
    // An unmeasured capacity is never printed as a number, even if a stale measurement is
    // attached: the engine's `capacityUnknown` wins.
    let available = prompt.available.filter(|_| !prompt.capacity_unknown);
    let mut message = match available {
        Some(a) => crate::i18n::fmt(
            tl!("Opening “{name}” requires an estimated {est} GiB of memory ({avail} GiB available)."),
            &[("name", &prompt.name), ("est", &format!("{est_gb:.1}")), ("avail", &format!("{:.1}", a as f64 / (1u64 << 30) as f64))],
        ),
        None => crate::i18n::fmt(
            tl!("Opening “{name}” requires an estimated {est} GiB of memory. The available memory could not be measured."),
            &[("name", &prompt.name), ("est", &format!("{est_gb:.1}"))],
        ),
    };
    message.push_str("\n\n");
    message.push_str(tl!("Opening this document uses more memory and may slow down your computer or other running applications."));
    if available.is_some_and(|a| a.saturating_sub(prompt.estimated) < photocraft_io::memory::LOW_MEMORY_WARNING_MARGIN) {
        message.push_str("\n\n");
        message.push_str(tl!("Little memory would be left after opening this file. Close other apps first, or PhotoCraft may run out of memory."));
    }
    if prompt.uncertain_capacity_warning {
        message.push_str("\n\n");
        message.push_str(tl!(
            "The estimate is above 8 GiB and the available memory is unknown, so this process or browser tab may run out of memory. You can still try to open it."
        ));
    }

    let fields = [
        (MEMORY_FIELD.into(), json!(true)),
        ("__label".into(), json!(tl!("High Memory Photoshop Document"))),
        ("name".into(), json!(prompt.name)),
        ("id".into(), json!(prompt.id)),
        ("message".into(), json!(message)),
        ("estimated".into(), json!(prompt.estimated)),
        ("available".into(), json!(available)),
        ("capacityUnknown".into(), json!(prompt.capacity_unknown)),
        ("uncertainCapacityWarning".into(), json!(prompt.uncertain_capacity_warning)),
    ]
    .into_iter()
    .collect();

    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

/// Answers the front high-memory prompt: accepts and starts the approved open, or declines and
/// advances the queue. The engine command revalidates the source identity and the hard limit.
pub(crate) fn answer_memory(app: &mut PhotocraftApp, accept: bool) -> Result<Value, String> {
    let Some(front) = app.held_memory_prompts.pop_front() else {
        advance(app);
        return Err("no high-memory PSD is waiting for confirmation".into());
    };
    if let Some(pos) = app.prompt_order.iter().position(|p| *p == PromptKind::Memory) {
        app.prompt_order.remove(pos);
    }
    let id = front.id;
    let name = front.name.clone();
    let path = front.path.clone();
    let slot = front.slot;
    let order = front.order;

    let res = app.run("file.approveMemory", json!({"id": id, "approve": accept, "background": app.background_jobs})).map_err(|e| format!("{}: {e}", name));

    match res {
        Ok(r) => {
            if accept {
                if let Some(job) = r.get("job").and_then(Value::as_u64) {
                    app.jobs.opens.push(crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(job), name: name.clone(), path, slot, order });
                    app.jobs.focus = Some(photocraft_engine::jobs::JobId(job));
                    app.ui.chrome.home = None;
                    app.ui.status = crate::i18n::fmt(tl!("Opening {name}…"), &[("name", &name)]);
                    app.ui.status_error = false;
                } else if let Some(result) = r.get("result") {
                    let tab = crate::jobs_ui::OpenTab { job: photocraft_engine::jobs::JobId(0), name: name.clone(), path: path.clone(), slot, order };
                    if let Err(e) = crate::jobs_ui::finish_open_tab(app, &tab, result) {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                }
            } else {
                app.ui.status = crate::i18n::fmt(tl!("Canceled opening {name}"), &[("name", &name)]);
                app.ui.status_error = false;
                crate::jobs_ui::end_batch(app);
            }
            advance(app);
            crate::jobs_ui::end_batch(app);
            Ok(r)
        }
        Err(err) => {
            app.ui.status = err.clone();
            app.ui.status_error = true;
            crate::jobs_ui::end_batch(app);
            advance(app);
            crate::jobs_ui::end_batch(app);
            Err(err)
        }
    }
}
