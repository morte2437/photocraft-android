use super::editing::edit_home;
use super::ime_edit::{ime_geometry, reconcile_preedit};
use super::*;

/// Per-editor IME owner/generation bookkeeping for the custom Type tool.
///
/// The tracker is shared by every viewport and keeps the context generation alive when this
/// editor is replaced. The overlay records the pass's canvas mapping and the pass end recomputes
/// the final geometry from the observed model, so an event-free or discarded pass never replays
/// stale text.
#[derive(Debug, Default)]
pub(crate) struct ImeState {
    pub(super) tracker: ImeContextTracker,
    pub(super) frame: Option<ImeFrame>,
    /// The canvas mapping of the current pass, so the final descriptor geometry is recomputed
    /// from the observed model rather than from an earlier point in the pass.
    pub(super) view: Option<ImeView>,
    /// The last descriptor identity the platform actually received (a discarded pass did not
    /// receive it, so its transition must still be reported by the final pass).
    pub(super) emitted: Option<ImeEmitted>,
    /// Ask the platform to cancel the native composition (owner/session transition or a stale
    /// pre-edit). Stays set until a non-discarded descriptor carries it.
    pub(super) interrupt: bool,
    /// Incoming composition provenance survives idle/discarded passes and an empty preedit.
    /// A stale owner stays fenced until the exact host reset completion arrives.
    pub(super) incoming: Option<IncomingComposition>,
    /// The composition text was invalidated; this pass must report `Unavailable` rather than
    /// presenting the orphaned composition as committed text.
    pub(super) stale: bool,
    /// The exact composition written into the layer, used to check the stored range before any
    /// replacement, deletion, underline or surrounding-text report.
    pub(super) preedit: Option<ImePreedit>,
    /// A local fence is released only by its exact host completion, never by terminal text.
    pub(super) resets: HashMap<egui::ViewportId, ResetPhase>,
    context: Option<egui::Context>,
    needs_request: bool,
    /// One exclusive event owner per actual UI pass; logic-only ticks never replay input.
    raw_pass: HashMap<egui::ViewportId, u64>,
    pub(super) custom_pass: HashMap<egui::ViewportId, (u64, Option<ResetContext>)>,
    last_custom: HashMap<egui::ViewportId, ResetContext>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResetContext(pub(super) egui::Id, pub(super) u64, pub(super) egui::IMEPurpose);

impl From<&egui::output::IMEOutput> for ResetContext {
    fn from(ime: &egui::output::IMEOutput) -> Self {
        Self(ime.owner, ime.context_generation, ime.purpose)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ResetPhase {
    NeedsRequest,
    Awaiting { id: egui::ImeResetId, destination: IncomingSelection, context: Option<ResetContext> },
    Suspended,
    AllocationFailed { destination: IncomingSelection, context: Option<ResetContext> },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ImeFrame {
    pub(super) viewport: egui::ViewportId,
    pub(super) frame_nr: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ImeEmitted {
    pub(super) target: ImeTarget,
    pub(super) generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ImeView {
    pub(super) viewport: egui::ViewportId,
    pub(super) pass_nr: u64,
    pub(super) xf: ViewXform,
}

/// The composition text and the edit session that wrote it. A range that still lies inside the
/// text is not enough: an application edit can shift it onto committed characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImePreedit {
    pub(super) layer: u64,
    pub(super) session: String,
    pub(super) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IncomingSelection {
    target: ImeTarget,
    caret: usize,
    anchor: usize,
    revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum IncomingComposition {
    Active(IncomingSelection),
    TerminalPending(IncomingSelection),
    Stale,
}

/// A bounded committed snapshot captured from the type layer.
///
/// The snapshot owns at most `MAX_IME_SURROUNDING_TEXT_BYTES`; the layer's whole text is only
/// borrowed while it is built. The current pre-edit lives in the layer so it lays out like
/// typed text, and is removed from this value rather than reported as committed text.
pub(super) struct ImeCommitted {
    pub(super) snapshot: egui::output::IMESurroundingText,
}

impl ImeCommitted {
    pub(super) fn stamp(&self) -> ImeMutationStamp {
        ImeMutationStamp::from_surrounding_text(self.snapshot.clone())
    }
}

impl ImeState {
    /// Remember the frame used by this pass's accepted IME receipts.
    pub(super) fn begin_frame(&mut self, ctx: &egui::Context) {
        let viewport = ctx.viewport_id();
        self.frame = Some(ImeFrame { viewport, frame_nr: ctx.cumulative_frame_nr_for(viewport) });
    }

    /// Record the canvas mapping of this pass. `sync_ime` recomputes both rectangles from the
    /// model after every panel has run.
    pub(super) fn note_view(&mut self, viewport: egui::ViewportId, pass_nr: u64, xf: &ViewXform) {
        self.view = Some(ImeView { viewport, pass_nr, xf: *xf });
    }

    /// Request the native composition reset; incoming ownership is tracked separately.
    pub(super) fn request_interrupt(&mut self) {
        self.interrupt = true;
        if self.incoming.is_some() || !self.resets.is_empty() {
            self.needs_request = true;
            // Context-less commands must retire every old id synchronously.
            for phase in self.resets.values_mut() {
                *phase = ResetPhase::NeedsRequest;
            }
        }
    }

    /// Forget a composition whose range no longer describes the recorded text.
    pub(super) fn invalidate_preedit(&mut self) {
        self.preedit = None;
        self.stale = true;
        self.incoming = Some(IncomingComposition::Stale);
        self.request_interrupt();
    }

    pub(super) fn note_preedit(&mut self, ed: &TextEdit, text: &str) {
        self.preedit = Some(ImePreedit { layer: ed.layer, session: ed.session.clone(), text: text.to_owned() });
    }

    pub(super) fn clear_preedit(&mut self) {
        self.preedit = None;
    }
}

/// The immutable owner of the Type tool's IME context.
///
/// The document, layer and viewport identify the target; the edit session is the replacement
/// epoch, so committing and editing the same layer again cannot reuse the old generation.
pub(super) fn ime_target(app: &PhotocraftApp, ed: &TextEdit, viewport: egui::ViewportId) -> Option<ImeTarget> {
    let document = ed.document.or_else(|| active_doc_id(app))?;
    // A Type Mask session is a distinct owner: its hidden working layer can have the same id as
    // a regular type layer in another tool session. The session epoch replaces the context when
    // an owner is recreated.
    let owner = egui::Id::unique(("photocraft-type-ime", document, ed.layer, ed.mask, viewport.0.value()));
    let epoch = egui::Id::unique(("photocraft-type-ime-session", &ed.session)).value().max(1);
    Some(ImeTarget::new(owner, epoch))
}

fn incoming_selection(app: &PhotocraftApp, viewport: egui::ViewportId) -> Option<IncomingSelection> {
    let ed = app.ui.text_edit.as_ref()?;
    Some(IncomingSelection { target: ime_target(app, ed, viewport)?, caret: ed.caret, anchor: ed.anchor, revision: app.session.active()?.revision })
}

/// Settle the already-visible preview where it was typed, never replay it at a moved caret.
pub(super) fn interrupt_incoming(app: &mut PhotocraftApp) {
    if !matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_) | IncomingComposition::TerminalPending(_))) {
        return;
    }
    app.type_ime.incoming = Some(IncomingComposition::Stale);
    if let Some(ed) = &mut app.ui.text_edit {
        ed.preedit = None;
    }
    app.type_ime.clear_preedit();
    app.type_ime.request_interrupt();
}

pub(super) fn interrupt_action(app: &mut PhotocraftApp, viewport: egui::ViewportId) {
    let guarded = reset_guarded(app, viewport);
    interrupt_incoming(app);
    if guarded {
        app.type_ime.request_interrupt();
    }
}

pub(super) fn sync_incoming(app: &mut PhotocraftApp, viewport: egui::ViewportId) {
    let destination = incoming_selection(app, viewport);
    if app.type_ime.resets.get(&viewport).is_some_and(|phase| match phase {
        ResetPhase::Awaiting { destination: old, .. } | ResetPhase::AllocationFailed { destination: old, .. } => Some(*old) != destination,
        _ => false,
    }) {
        app.type_ime.request_interrupt();
    }
    let expected = match app.type_ime.incoming {
        Some(IncomingComposition::Active(expected) | IncomingComposition::TerminalPending(expected)) => expected,
        _ => return,
    };
    let lost_preview = matches!(app.type_ime.incoming, Some(IncomingComposition::Active(_))) && app.ui.text_edit.as_ref().is_none_or(|ed| ed.preedit.is_none());
    if incoming_selection(app, viewport) != Some(expected) || lost_preview {
        // An external edit can move an in-bounds preview range onto committed characters.
        // Validate it before interruption clears the metadata needed to detect that mismatch.
        // A valid preview still settles normally; never inspect another document's layer here.
        if let Some(ed) = app.ui.text_edit.clone()
            && ed.preedit.is_some()
            && ime_target(app, &ed, viewport) == Some(expected.target)
            && let Some(active) = app.session.active_index()
            && edit_home(app, &ed) == Some(active)
            && let Some(text) = current_text(app, LayerId(ed.layer))
            && reconcile_preedit(app, &ed, &text)
        {
            return;
        }
        interrupt_incoming(app);
    }
}

/// Accepted IME mutations update provenance without canceling their own Delete→Commit chain.
pub(super) fn remember_incoming(app: &mut PhotocraftApp, preedit_empty: Option<bool>) {
    let viewport = app.type_ime.frame.map_or(egui::ViewportId::ROOT, |frame| frame.viewport);
    let Some(expected) = incoming_selection(app, viewport) else { return };
    let next = match (preedit_empty, app.type_ime.incoming) {
        (Some(false), _) | (None, Some(IncomingComposition::Active(_))) => IncomingComposition::Active(expected),
        (Some(true), _) | (None, Some(IncomingComposition::TerminalPending(_))) => IncomingComposition::TerminalPending(expected),
        _ => return,
    };
    app.type_ime.incoming = Some(next);
}

/// A host may recreate its Context while retaining the application. Old pass counters and
/// output keys are not evidence of publication in that new Context.
pub(super) fn bind_reset_context(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // `on_begin_pass` callbacks registered during this pass cannot mark the pass already
    // running. Reinstall here so a replacement Context is ready for its next pass, and rely on
    // the host's creator/`setup_context` having installed before the very first `run_ui`.
    install_reset_hooks(ctx);
    if app.type_ime.context.as_ref().is_some_and(|previous| previous != ctx) {
        interrupt_incoming(app);
        for phase in app.type_ime.resets.values_mut() {
            *phase = ResetPhase::Suspended;
        }
        app.type_ime.needs_request = false;
        app.type_ime.raw_pass.clear();
        app.type_ime.custom_pass.clear();
        app.type_ime.last_custom.clear();
        app.type_ime.view = None;
        app.type_ime.frame = None;
        app.type_ime.tracker.observe(ctx.viewport_id(), None);
    }
    app.type_ime.context = Some(ctx.clone());
}

fn ui_mode_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique(("photocraft-type-reset-ui", viewport))
}

/// Install before the first UI pass on every Context, including a replacement. These callbacks
/// retain neither the app nor its Context. Calling this from inside a running pass only affects
/// subsequent passes: egui's begin-pass callback list is already being iterated.
pub(crate) fn install_reset_hooks(ctx: &egui::Context) {
    let installed = egui::Id::unique("photocraft-type-reset-hooks");
    if ctx.data(|data| data.get_temp::<bool>(installed)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(installed, true));
    ctx.on_begin_pass(
        "photocraft type reset UI mode",
        Arc::new(|ui| {
            let key = ui_mode_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|data| data.insert_temp(key, true));
        }),
    );
    ctx.on_end_pass(
        "photocraft type reset UI mode end",
        Arc::new(|ui| {
            let key = ui_mode_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|data| data.insert_temp(key, false));
        }),
    );
}

pub(crate) fn in_ui_pass(ctx: &egui::Context) -> bool {
    let key = ui_mode_id(ctx.viewport_id());
    ctx.data(|data| data.get_temp::<bool>(key)).unwrap_or(false)
}

pub(super) fn reset_guarded(app: &PhotocraftApp, viewport: egui::ViewportId) -> bool {
    app.type_ime.needs_request || app.type_ime.resets.contains_key(&viewport) || matches!(app.type_ime.incoming, Some(IncomingComposition::Stale))
}

pub(super) fn reset_route_eligible(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    // The actual app draws only after `logic_frame` marks fonts ready; `ui_frame` early-returns
    // on the first frame. Direct handler tests that have not run the first logic frame use the
    // pre-styled app state and are not an actual UI route.
    (app.fonts_ready || !app.styled)
        && app.ui.text_edit.is_some()
        && app.ui.dialogs.is_empty()
        && !crate::screen_picker::busy(ctx)
        && ctx.input(|i| i.focused)
        && !ctx.text_edit_focused()
        && !ctx.egui_wants_keyboard_input()
}

/// Whether this input can be attributed to a visible custom descriptor right now. Completion
/// arrives before the final descriptor exists, so use the last canvas mapping and the current
/// model: requiring this pass's final output here would deadlock the ordered suffix.
fn reset_geometry_available(app: &mut PhotocraftApp, viewport: egui::ViewportId) -> bool {
    let Some(ed) = app.ui.text_edit.clone() else { return false };
    let Some(view) = app.type_ime.view.filter(|view| view.viewport == viewport) else { return false };
    ime_geometry(app, &ed, view.xf).is_some()
}

pub(super) fn suspend_reset(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    if reset_guarded(app, viewport) {
        app.type_ime.resets.insert(viewport, ResetPhase::Suspended);
        app.type_ime.needs_request = false;
    }
    app.type_ime.last_custom.remove(&viewport);
}

fn allocate_reset(app: &mut PhotocraftApp, ctx: &egui::Context, destination: IncomingSelection, context: Option<ResetContext>) {
    let viewport = ctx.viewport_id();
    let phase = match ctx.request_ime_reset(viewport) {
        Some(id) => ResetPhase::Awaiting { id, destination, context },
        None => ResetPhase::AllocationFailed { destination, context },
    };
    app.type_ime.resets.insert(viewport, phase);
    app.type_ime.needs_request = false;
}

/// Reconcile before the next raw element, not merely at descriptor publication.
pub(super) fn prepare_reset(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    sync_incoming(app, viewport);
    if app.type_ime.needs_request {
        app.type_ime.resets.insert(viewport, ResetPhase::NeedsRequest);
        app.type_ime.needs_request = false;
    }
    // Allocation belongs to the true UI tail: a stock field drawn later in this pass
    // must never inherit a speculative custom-editor reset request.
}

pub(super) fn complete_reset(app: &mut PhotocraftApp, ctx: &egui::Context, request_id: egui::ImeResetId) {
    let viewport = ctx.viewport_id();
    let Some(ResetPhase::Awaiting { id, destination, context }) = app.type_ime.resets.get(&viewport).copied() else { return };
    if request_id != id
        || request_id.viewport_id() != viewport
        || !reset_route_eligible(app, ctx)
        || incoming_selection(app, viewport) != Some(destination)
        || context != app.type_ime.last_custom.get(&viewport).copied()
    {
        return;
    }
    if !reset_geometry_available(app, viewport) {
        // Retire the old id and keep the following preedit blocked before it can reach the
        // editor. The UI tail may still rearm once this pass produces real custom output.
        suspend_reset(app, ctx);
        return;
    }
    app.type_ime.resets.remove(&viewport);
    app.type_ime.incoming = None;
    app.type_ime.preedit = None;
    app.type_ime.stale = false;
    app.type_ime.interrupt = false;
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.preedit = None;
    }
}

pub(super) fn claim_raw_pass(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let viewport = ctx.viewport_id();
    let pass = ctx.cumulative_pass_nr_for(viewport);
    app.type_ime.raw_pass.insert(viewport, pass) != Some(pass)
}

/// The exclusive route when no custom editor may mutate. Do not consume stock text/keys.
pub(crate) fn controls_only(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    if !claim_raw_pass(app, ctx) {
        return;
    }
    if !reset_route_eligible(app, ctx) {
        interrupt_incoming(app);
    }
    suspend_reset(app, ctx); // Retire an old id before visiting even the first marker.
    for event in ctx.input(|i| i.events.clone()) {
        if let egui::Event::ImeResetCompleted { request_id } = event {
            complete_reset(app, ctx, request_id);
        }
    }
}

/// True outer UI tail, including discarded/early-return passes and host-side mutations.
pub(crate) fn finish_reset_pass(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    let viewport = ctx.viewport_id();
    if app.type_ime.raw_pass.get(&viewport).copied() != Some(ctx.cumulative_pass_nr_for(viewport)) {
        controls_only(app, ctx);
    }
    sync_incoming(app, viewport);
    let produced = app.type_ime.custom_pass.get(&viewport).copied().filter(|(pass, _)| *pass == ctx.cumulative_pass_nr_for(viewport)).and_then(|(_, key)| key);
    let actual = ctx.output(|o| o.ime.as_ref().map(ResetContext::from));
    let Some(context) = produced.filter(|key| Some(*key) == actual && reset_route_eligible(app, ctx)) else {
        suspend_reset(app, ctx);
        return;
    };
    // Never allocate a request for output egui will throw away. Keep any existing guard and its
    // queued R1 blocked; the final pass either publishes this context and reuses it or retires
    // the old id through the ineligible branch above.
    if ctx.will_discard() {
        return;
    }
    app.type_ime.last_custom.insert(viewport, context);
    if !reset_guarded(app, viewport) {
        return;
    }
    let Some(destination) = incoming_selection(app, viewport) else {
        suspend_reset(app, ctx);
        return;
    };
    let unchanged = app.type_ime.resets.get(&viewport).is_some_and(|phase| match phase {
        ResetPhase::Awaiting { destination: old, context: old_context, .. } | ResetPhase::AllocationFailed { destination: old, context: old_context } => {
            *old == destination && *old_context == Some(context) && !app.type_ime.needs_request
        }
        _ => false,
    });
    if !unchanged {
        allocate_reset(app, ctx, destination, Some(context));
    }
}

pub(crate) fn finish_reset_logic(app: &mut PhotocraftApp, ctx: &egui::Context) {
    bind_reset_context(app, ctx);
    if !in_ui_pass(ctx) {
        sync_incoming(app, ctx.viewport_id());
        suspend_reset(app, ctx);
    }
}
