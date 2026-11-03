//! Save / Don't Save / Cancel prompt for actions that would throw away unsaved changes: closing
//! documents, Revert, and leaving the app (File › Exit or the window's close button).
//!
//! egui is immediate-mode, so the action is parked in [`Prompt`] while the modal is up and re-run
//! once every affected document has been answered. Cancel at any point drops it. Documents are
//! tracked by id, not tab index, so closing one elsewhere while the prompt is up can't retarget it.

use egui::Key;
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::widgets::{ButtonRole, DialogButton};

const EXIT: &str = "file.exit";

/// An action waiting on the user, and the dirty documents still to ask about.
pub struct Prompt {
    id: String,
    params: Value,
    /// The document the command was aimed at (`document` param, else the active one), if it has one.
    target: Option<DocId>,
    docs: Vec<DocId>,
    /// The document whose question has already put keyboard focus on its default answer.
    focused: Option<DocId>,
}

fn index_of(app: &PhotocraftApp, id: DocId) -> Option<usize> {
    app.session.documents().iter().position(|d| d.doc.id == id)
}

/// Which documents a command discards, relative to its target (the `document` param, else the active one).
#[derive(Clone, Copy)]
enum Reach {
    Target,
    AllButTarget,
    All,
}

/// Every command that can throw away unsaved changes. A new one only needs a row here.
const DISCARDING: &[(&str, Reach)] = &[
    ("file.close", Reach::Target),
    ("file.revert", Reach::Target),
    ("file.closeOthers", Reach::AllButTarget),
    ("file.closeAll", Reach::All),
    (EXIT, Reach::All),
];

fn reach(id: &str) -> Option<Reach> {
    DISCARDING.iter().find(|(c, _)| *c == id).map(|(_, r)| *r)
}

/// The command's target document and the unsaved documents it would discard (empty for commands
/// that discard nothing).
fn discarded(app: &PhotocraftApp, id: &str, params: &Value) -> (Option<DocId>, Vec<DocId>) {
    let docs = app.session.documents();
    let target = params.get("document").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index());
    let affected: Vec<usize> = match reach(id) {
        Some(Reach::Target) => target.into_iter().collect(),
        Some(Reach::AllButTarget) => (0..docs.len()).filter(|&i| Some(i) != target).collect(),
        Some(Reach::All) => (0..docs.len()).collect(),
        None => Vec::new(),
    };
    let dirty = affected.into_iter().filter_map(|i| docs.get(i)).filter(|d| d.is_dirty()).map(|d| d.doc.id).collect();
    (target.and_then(|i| docs.get(i)).map(|d| d.doc.id), dirty)
}

/// Park `id` behind a prompt if it would discard unsaved work. Returns whether it did.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> bool {
    let (target, docs) = discarded(app, id, params);
    if docs.is_empty() {
        return false;
    }
    // Once the window's GPU device is lost nothing can be drawn (`gpu_status::check_window`), so
    // the prompt would never show. While saving for recovery keeps failing, quitting (the close
    // button, File › Exit) must still go through rather than leave a blank window that can't be
    // closed.
    if id == EXIT && app.window_lost {
        log::warn!("quitting a window that lost its GPU device before its unsaved work could be saved for recovery");
        return false;
    }
    let prompt = Prompt { id: id.to_string(), params: params.clone(), target, docs, focused: None };
    match &app.discard {
        None => ask(app, prompt),
        // Quitting overrides whatever is pending: it covers every document, so nothing is lost.
        Some(open) if id == EXIT && open.id != EXIT => ask(app, prompt),
        // Repeated quit requests (the X pressed again) keep the prompt, and the answers so far.
        Some(open) if open.id == id => {}
        Some(_) => {
            app.ui.status = tl!("Answer the unsaved-changes prompt first").into();
            app.ui.status_error = true;
        }
    }
    true
}

/// Puts up `prompt`. One over several documents shows the document it asks about first; each
/// later one is shown as it comes up (see [`advance`]).
fn ask(app: &mut PhotocraftApp, prompt: Prompt) {
    if !matches!(reach(&prompt.id), Some(Reach::Target)) {
        show_tab(app, prompt.docs.first().copied());
    }
    app.discard = Some(prompt);
}

/// Makes `doc` the active tab, if it is still open.
fn show_tab(app: &mut PhotocraftApp, doc: Option<DocId>) {
    if let Some(i) = doc.and_then(|d| index_of(app, d)) {
        app.session.set_active(i);
    }
}

impl PhotocraftApp {
    /// Whether closing now would lose work: a document has unsaved changes and closing wasn't
    /// confirmed. The web build has the browser ask before leaving the page while this holds (#1380).
    pub fn has_unsaved_work(&self) -> bool {
        !self.allow_close && self.session.documents().iter().any(|d| d.is_dirty())
    }
}

/// Called once per frame: holds back a window close request while there is unsaved work. Every
/// way of leaving (File › Exit / Quit, the close button, the OS's quit event, `app.quit`) ends up
/// here as a close request.
pub fn guard_window_close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    if !app.allow_close && intercept(app, EXIT, &Value::Null) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        return;
    }
    // A save or print worker must finish its external side effect before the process exits;
    // `jobs_ui` repeats the close after completion and keeps errors visible.
    if app.saving() || app.printing() {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        app.jobs.close_after_saves = true;
        return;
    }
    // Leaving: through the platform's own quit where there is one (see `Services::quit`), with
    // the window left open for it to close.
    if let Some(quit) = app.services.quit.as_mut() {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        quit();
    }
}

/// Moves on to the next document, or runs the parked action once none are left.
fn advance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.discard.as_mut() else { return };
    let answered = (!p.docs.is_empty()).then(|| p.docs.remove(0));
    let (next, one_by_one) = (p.docs.first().copied(), !matches!(reach(&p.id), Some(Reach::Target)));
    // Close All, Close Others and quitting close each document once it is answered, as Photoshop
    // does, and show the next one asked about: otherwise the tab just answered stays up and it
    // looks like it is being asked about again (#2547). Cancel then keeps only the unanswered ones.
    if one_by_one {
        if let Some(i) = answered.and_then(|d| index_of(app, d))
            && let Err(e) = crate::menus::invoke_unguarded(app, ctx, "file.close", json!({"document": i}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        show_tab(app, next);
    }
    if next.is_some() {
        return;
    }
    // A save or close hook may have dirtied a document that was clean when the prompt opened (or
    // made a new one): ask about those before the final close, which discards without checking.
    if one_by_one && let Some(p) = app.discard.as_ref() {
        let (id, target) = (p.id.clone(), p.target);
        let rest: Vec<DocId> = app
            .session
            .documents()
            .iter()
            .filter(|d| d.is_dirty() && (matches!(reach(&id), Some(Reach::All)) || Some(d.doc.id) != target))
            .map(|d| d.doc.id)
            .collect();
        if !rest.is_empty() {
            show_tab(app, rest.first().copied());
            if let Some(p) = app.discard.as_mut() {
                p.docs = rest;
            }
            return;
        }
    }
    let Some(Prompt { id, mut params, target, .. }) = app.discard.take() else { return };
    if id == EXIT {
        app.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    if let Some(target) = target {
        // The tab may have moved since the command was issued; aim it at the same document.
        params = match index_of(app, target) {
            Some(i) => json!({"document": i}),
            // Close All may have closed its target above already; it still closes the rest.
            None if matches!(reach(&id), Some(Reach::All)) => json!({}),
            None => return,
        };
    }
    if let Err(e) = crate::menus::invoke_unguarded(app, ctx, &id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Saves `doc`; true once it is saved. A document without a file to write back to asks where in
/// the save dialog: the prompt then hides until it is answered, and moves on only if it saved.
fn save(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) -> bool {
    let Some(i) = index_of(app, doc) else { return false };
    app.session.set_active(i);
    let saved = crate::menus::invoke_unguarded(app, ctx, "file.save", json!({}));
    if saved.is_ok() && app.file_dialog_open() {
        let ctx = ctx.clone();
        app.after_file_dialog(move |app, saved| {
            let saved = saved.map_err(couldnt_save)?;
            saved_document(app, &ctx, doc);
            Ok(saved)
        });
        return false;
    }
    match saved {
        // A Save Document script may have edited the document after it was written: still dirty.
        // A write the platform can't confirm (a web download) leaves it dirty too, and counts.
        Ok(_) => {
            !app.save_options_open()
                && !app.saving()
                && index_of(app, doc).is_some_and(|i| app.services.write_unconfirmed || !app.session.documents()[i].is_dirty())
        }
        // Backing out of the file dialog is the user's choice, not an error.
        Err(e) if e == crate::file_dialog::CANCELLED => false,
        Err(e) => {
            app.ui.status = couldnt_save(e);
            app.ui.status_error = true;
            false
        }
    }
}

/// A completed save may release the close prompt. Choosing a path only starts a layered TIFF
/// save; TIFF Options calls this after the write, and `jobs_ui` when a background save ends.
/// Copies and failed writes leave the doc dirty; so does any write when it is unconfirmed.
pub(crate) fn saved_document(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) {
    // Still the prompt that asked (quitting may have replaced it meanwhile).
    if !app.save_options_open()
        && app.discard.as_ref().and_then(|p| p.docs.first()) == Some(&doc)
        && index_of(app, doc).is_some_and(|i| app.services.write_unconfirmed || !app.session.documents()[i].is_dirty())
    {
        advance(app, ctx);
    }
}

/// A save failure as reported ("cancelled" stays as it is: it isn't reported).
fn couldnt_save(e: String) -> String {
    if e == crate::file_dialog::CANCELLED { e } else { format!("Couldn't save: {e}") }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Save can ask for a path and then TIFF or Targa Options, and then write in the background.
    // Wait for all three; cancellation or a failed save brings us back.
    if app.file_dialog_open() || app.save_options_open() || app.saving() {
        // Hidden meanwhile: the question takes focus again when it comes back.
        if let Some(p) = app.discard.as_mut() {
            p.focused = None;
        }
        return;
    }
    let Some(p) = &app.discard else { return };
    let Some(&doc) = p.docs.first() else { return };
    let (exits, reverts) = (p.id == EXIT, p.id == "file.revert");
    let Some(name) = index_of(app, doc).map(|i| app.session.documents()[i].doc.name.clone()) else {
        // Closed by something else while the prompt was up: nothing left to ask about.
        advance(app, ctx);
        return;
    };
    let (title, message) = if reverts {
        ("Revert", crate::i18n::fmt(tl!("Revert “{name}” to the last saved version? Your changes will be lost."), &[("name", &name)]))
    } else {
        let template = if exits {
            tl!("Do you want to save the changes you made to “{name}” before quitting?")
        } else {
            tl!("Do you want to save the changes you made to “{name}” before closing?")
        };
        (tl!("Unsaved changes"), crate::i18n::fmt(template, &[("name", &name)]))
    };
    // Each question comes up with its default answer focused, as in Photoshop: the arrow keys then
    // move between the answers and Enter presses the focused one (#6441).
    let focus = p.focused != Some(doc);
    if let Some(p) = app.discard.as_mut() {
        p.focused = Some(doc);
    }
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    // The answers in the platform's words; `dialog_buttons` puts them in its order. Windows (and
    // Linux) ask Yes / No / Cancel with Y, N and Esc, as Photoshop does there; macOS asks
    // Don't Save / Cancel / Save. Esc always cancels (the modal closes on it).
    // Yes/No already show Y/N. Cancel gets C on every platform (#4582). macOS still hides the
    // letters and keeps the shortcut.
    let cancel = (ButtonRole::Cancel, "Cancel", Some(Key::C), 84.0, Answer::Cancel);
    let buttons = if reverts {
        vec![(ButtonRole::Default, "Revert", Some(Key::R), 84.0, Answer::Discard), cancel]
    } else if mac {
        vec![
            (ButtonRole::Default, "Save", Some(Key::S), 84.0, Answer::Save),
            (ButtonRole::Alternate, "Don't Save", Some(Key::D), 100.0, Answer::Discard),
            cancel,
        ]
    } else {
        vec![(ButtonRole::Default, "Yes", Some(Key::Y), 84.0, Answer::Save), (ButtonRole::Alternate, "No", Some(Key::N), 84.0, Answer::Discard), cancel]
    };
    let mut answer = ctx.input_mut(|i| buttons.iter().find(|b| b.2.is_some_and(|k| i.consume_key(egui::Modifiers::NONE, k))).map(|b| b.4));
    let labels = button_labels(mac, &buttons.iter().map(|b| (b.1, b.2)).collect::<Vec<_>>());
    let row: Vec<DialogButton> =
        buttons.iter().zip(&labels).map(|(b, label)| DialogButton::new(b.0, label, b.3).focus(focus && b.0 == ButtonRole::Default)).collect();
    let modal = egui::Modal::new(egui::Id::unique("discard-prompt")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!(&title)).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(message);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if let Some(role) = crate::widgets::dialog_buttons(ui, &row) {
                answer = buttons.iter().find(|b| b.0 == role).map(|b| b.4);
            }
        });
    });
    if modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    // Enter takes the default answer, unless Tab focused a button: that one took the Enter as a click.
    if answer.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        answer = buttons.iter().find(|b| b.0 == ButtonRole::Default).map(|b| b.4);
    }
    if let Some(answer) = answer {
        apply(app, ctx, doc, answer);
    }
}

/// Acts on the answer about `doc`, the document the prompt is asking about.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId, answer: Answer) {
    match answer {
        Answer::Cancel => app.discard = None,
        Answer::Discard => advance(app, ctx),
        Answer::Save if save(app, ctx, doc) => advance(app, ctx),
        Answer::Save => {}
    }
}

/// The id `ui.inspect` lists the prompt under; `ui.dialog.confirm` and `ui.dialog.cancel` answer
/// it by this id, so a control client can see and answer it like the window's buttons (#4920).
pub const CONTROL_ID: &str = "discard";

/// The prompt as `ui.inspect` lists it among the dialogs: the command waiting on it, the document
/// it asks about, how many are still to answer, and the choices `ui.dialog.confirm` takes.
pub fn inspect(app: &PhotocraftApp) -> Option<Value> {
    let p = app.discard.as_ref()?;
    let doc = *p.docs.first()?;
    let name = index_of(app, doc).and_then(|i| app.session.documents().get(i)).map(|d| d.doc.name.clone())?;
    let reverts = p.id == "file.revert";
    let (choices, default): (&[&str], _) = if reverts { (&["dontSave", "cancel"], "dontSave") } else { (&["save", "dontSave", "cancel"], "save") };
    Some(json!({
        "id": CONTROL_ID,
        "kind": "unsavedChanges",
        "title": if reverts { "Revert" } else { "Unsaved changes" },
        "fields": {"command": p.id, "document": name, "remaining": p.docs.len(), "choices": choices, "default": default},
    }))
}

/// The command the prompt holds back and its params, for the automation policy to see before a
/// control client answers it.
pub fn pending_command(app: &PhotocraftApp) -> Option<(String, Value)> {
    app.discard.as_ref().map(|p| (p.id.clone(), p.params.clone()))
}

/// Answers the prompt for a control client: `save`, `dontSave` or `cancel` (Revert takes
/// `dontSave` or `cancel`). Like the buttons, Save may first ask for a path in the save dialog.
pub fn answer_control(app: &mut PhotocraftApp, ctx: &egui::Context, choice: &str) -> Result<Value, String> {
    let Some(doc) = app.discard.as_ref().and_then(|p| p.docs.first().copied()) else {
        return Err("no unsaved-changes prompt is open".into());
    };
    if app.file_dialog_open() || app.save_options_open() || app.saving() {
        return Err("the unsaved-changes prompt is waiting for a save to finish".into());
    }
    let reverts = app.discard.as_ref().is_some_and(|p| p.id == "file.revert");
    let answer = match choice {
        "save" if !reverts => Answer::Save,
        "dontSave" => Answer::Discard,
        "cancel" => Answer::Cancel,
        other => {
            let choices = if reverts { "dontSave or cancel" } else { "save, dontSave or cancel" };
            return Err(format!("unknown choice `{other}`: {choices}"));
        }
    };
    apply(app, ctx, doc, answer);
    Ok(json!({"open": app.discard.is_some(), "fileDialog": app.file_dialog_open()}))
}

#[derive(Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// macOS keeps the plain button wording even though the keyboard shortcuts still work.
#[cfg(test)]
fn button_label(mac: bool, label: &str, key: Option<Key>) -> String {
    button_labels(mac, &[(label, key)]).into_iter().next().unwrap_or_default()
}

/// The labels of one dialog's buttons, each with its key in the same style: "(Y)es" when every
/// translation starts with its key, otherwise appended to all ("Ja (Y)", "Nein (N)") (#5564).
fn button_labels(mac: bool, buttons: &[(&str, Option<Key>)]) -> Vec<String> {
    let starts_with_key = |label: &str, k: Key| label.split_at_checked(1).is_some_and(|(first, _)| first.eq_ignore_ascii_case(k.name()));
    let inline = buttons.iter().all(|&(label, key)| key.is_none_or(|k| starts_with_key(tl!(label), k)));
    buttons
        .iter()
        .map(|&(label, key)| {
            let text = tl!(label);
            match key {
                None => text.to_string(),
                Some(_) if mac => text.to_string(),
                Some(k) => match text.split_at_checked(1) {
                    Some((first, rest)) if inline => format!("({first}){rest}"),
                    _ => format!("{text} ({})", k.name()),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
