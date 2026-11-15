//! Background jobs: long commands run on a worker thread, report progress and can be cancelled
//! (#210).
//!
//! A command opts in by doing its heavy work through [`run`] (or [`edit_job`] for a plain document
//! edit). The work closure gets a [`JobCtx`] (progress plus a cancellation flag) and runs against
//! a snapshot: documents are copy-on-write, so cloning one is O(layers). Its result is applied on
//! the UI thread as one undo step, through the same path as [`Session::execute`], so the journal,
//! history, damage and post-command hooks all behave as for a synchronous command.
//!
//! - [`Session::execute`] runs everything inline, as before: tests, the CLI and scripts see no
//!   difference.
//! - [`Session::start`] runs a job-capable command on a worker thread and returns its
//!   [`JobId`]; [`Session::poll_jobs`] (called every frame by the UI) applies finished jobs.
//!   On wasm `start` runs inline (same API, written once).
//! - Work that is written as [`StepWork`] (Place) also runs where there are no threads: with
//!   [`Session::set_cooperative_jobs`] (the web build's default) [`run_stepped`] registers the job
//!   without a worker, and [`Session::poll_jobs`] runs one slice of it, about 8 ms, per call. The
//!   UI stays responsive between slices, and the result is the same as on a worker.
//! - [`Session::cancel_job`] sets the flag; the algorithms check it per tile, row band or
//!   iteration, and the result is discarded. The document is never touched by a worker.
//! - A job with an effect outside the session (a save replacing its file) calls
//!   [`JobCtx::commit`] right before it. From then on it can't be cancelled: every cancel
//!   path ([`Session::try_cancel_job`], [`Session::cancel_all_jobs`], closing its document,
//!   `jobs.cancel`) leaves it running and says so ([`CancelRefused::Committing`]), so nothing
//!   reports a job as cancelled after its effect has happened.
//! - A panic in the worker becomes an error; the document is unchanged.
//! - While a job runs on a document, commands that would edit that document are disabled with a
//!   message naming the job (queries and document-independent commands still run).

use std::any::Any;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_raster::Interrupt;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

const MAX_BACKGROUND_JOBS: usize = 16;

type Authorization = fn(&str, &Value) -> Result<()>;

thread_local! {
    // Smart-filter rendering has no Session argument. Carry the requesting session's gate
    // through synchronous rendering and explicitly reinstall it on background workers.
    static RENDER_AUTHORIZATION: std::cell::Cell<Option<Authorization>> = const { std::cell::Cell::new(None) };
}

pub(crate) struct AuthorizationScope(Option<Authorization>);

impl AuthorizationScope {
    fn enter(gate: Option<Authorization>) -> Self {
        Self(RENDER_AUTHORIZATION.with(|current| current.replace(gate.or(current.get()))))
    }
}

impl Drop for AuthorizationScope {
    fn drop(&mut self) {
        RENDER_AUTHORIZATION.with(|current| current.set(self.0));
    }
}

/// [`AuthorizationScope`] entered with `gate` (or the one already installed, so trusted work on
/// this thread keeps ambient paths usable). [`Session::edit`] and the job paths use this to carry
/// the requesting session's authority into every smart-object re-render.
pub(crate) fn render_authorization_scope(gate: Option<Authorization>) -> AuthorizationScope {
    AuthorizationScope::enter(gate)
}

/// Recheck a stored filter's command and parameters before evaluating it or using its cache.
pub(crate) fn authorize_render_command(id: &str, params: &Value) -> Result<()> {
    match RENDER_AUTHORIZATION.with(std::cell::Cell::get) {
        Some(gate) => gate(id, params),
        None => Ok(()),
    }
}

/// Whether the work on this thread is done for an untrusted session (a gate is installed).
pub(crate) fn authorization_installed() -> bool {
    RENDER_AUTHORIZATION.with(std::cell::Cell::get).is_some()
}

/// Identifies one background job for the session's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

/// What a job's work closure sees: report progress, check for cancellation. Cheap to clone; all
/// clones share the same state.
#[derive(Clone, Default)]
pub struct JobCtx {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// [`CANCEL_REQUESTED`], [`WORK_FINISHED`] and [`COMMITTED`] bits. One atomic orders a cancel
    /// against [`JobCtx::commit`] and against the worker finishing, so exactly one of them wins.
    state: AtomicU8,
    /// `f32` bits of the progress fraction. Non-negative floats order like their bits, so
    /// `fetch_max` keeps progress monotonic even with several reporting threads.
    progress: AtomicU32,
    message: Mutex<String>,
    batch: Mutex<BatchInterrupt>,
    batch_decision: Condvar,
    batch_preview: Mutex<Option<BatchPreview>>,
    batch_preview_generation: std::sync::atomic::AtomicU64,
}

/// The current folder Batch input, published as an immutable snapshot for the desktop canvas.
/// It is never added to the live session and cannot receive commands or saves.
#[derive(Clone, Debug)]
pub struct BatchPreview {
    pub path: String,
    pub document: Arc<Document>,
    pub generation: u64,
}

/// The desktop Batch Esc handshake. The worker alone enters `Waiting` at a pre-save
/// checkpoint; only the UI supplies a decision. Programmatic cancellation uses the separate
/// atomic flag above and wakes a waiting worker without asking a question.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BatchInterrupt {
    #[default]
    Processing,
    Requested,
    Waiting,
    Continue,
    Stop,
    Saving,
    /// Every input has been handled; Esc is no longer accepted.
    Closed,
}

/// Cancelled before it committed.
const CANCEL_REQUESTED: u8 = 1;
/// The worker has sent its result; a Batch no longer accepts a cancel.
const WORK_FINISHED: u8 = 2;
/// Past [`JobCtx::commit`]: the job runs to the end and can no longer be cancelled.
const COMMITTED: u8 = 4;

impl std::fmt::Debug for JobCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobCtx").field("progress", &self.fraction()).field("cancelled", &self.cancelled()).finish()
    }
}

impl JobCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Report progress in `0.0..=1.0` with a short status (empty keeps the previous one).
    /// Progress never goes backwards: a lower value than already reported is ignored.
    pub fn progress(&self, fraction: f32, message: &str) {
        if fraction.is_finite() {
            self.shared.progress.fetch_max(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
        if !message.is_empty() {
            let mut m = self.shared.message.lock().unwrap_or_else(PoisonError::into_inner);
            if *m != message {
                message.clone_into(&mut m);
            }
        }
    }

    /// Has the job been cancelled? Long loops check this and stop early.
    pub fn cancelled(&self) -> bool {
        self.shared.state.load(Ordering::Acquire) & CANCEL_REQUESTED != 0
    }

    /// Ask the job to stop (idempotent). Does nothing once it has [committed](Self::commit).
    pub fn cancel(&self) {
        let _ = self.try_cancel();
    }

    /// Cancel unless the job has [committed](Self::commit): true if it is cancelled (now or
    /// before), false if it is past its point of no return and will run to the end.
    pub fn try_cancel(&self) -> bool {
        let r = self.shared.state.try_update(Ordering::AcqRel, Ordering::Acquire, |state| (state & COMMITTED == 0).then_some(state | CANCEL_REQUESTED));
        if r.is_ok() {
            self.wake_batch_worker();
        }
        r.is_ok()
    }

    /// The job's point of no return, for work with an effect outside the session (a save calls
    /// it right before replacing the file). `Ok`: no cancel can stop the job any more; it must
    /// run to the end, and its result is applied even if its document was closed meanwhile.
    /// `Err(Cancelled)`: it was cancelled first, and nothing may be done.
    pub fn commit(&self) -> Result<()> {
        self.check()?;
        self.shared
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| (state & CANCEL_REQUESTED == 0).then_some(state | COMMITTED))
            .map(|_| ())
            .map_err(|_| EngineError::Cancelled)
    }

    /// Has the job passed [`Self::commit`]?
    pub fn committed(&self) -> bool {
        self.shared.state.load(Ordering::Acquire) & COMMITTED != 0
    }

    /// Wake a Batch worker waiting for the Esc decision, so it sees a cancel.
    fn wake_batch_worker(&self) {
        let _phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        self.shared.batch_decision.notify_all();
    }

    /// Accept a cancellation only while the worker has not finished its result. One atomic state
    /// orders a late cancel against completion, including the interval before the UI polls.
    fn cancel_if_running(&self) -> bool {
        let accepted = self
            .shared
            .state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & (CANCEL_REQUESTED | WORK_FINISHED | COMMITTED) == 0).then_some(state | CANCEL_REQUESTED)
            })
            .is_ok();
        if accepted {
            self.wake_batch_worker();
        }
        accepted
    }

    /// Request the reference application's Esc decision for an interactive Batch. A save that
    /// passed its final write checkpoint cannot be interrupted; the caller may try the next
    /// input. This does not affect the direct `jobs.cancel` automation API.
    pub fn request_batch_interrupt(&self) -> bool {
        if self.cancelled() || self.shared.state.load(Ordering::Acquire) & WORK_FINISHED != 0 {
            return false;
        }
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if *phase != BatchInterrupt::Processing {
            return false;
        }
        *phase = BatchInterrupt::Requested;
        true
    }

    /// The worker has stopped before writing the current input and awaits Continue or Stop.
    pub fn batch_waiting_for_decision(&self) -> bool {
        *self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner) == BatchInterrupt::Waiting
    }

    /// Answer a waiting interactive Batch question. `continue_next` skips the current input.
    pub fn decide_batch_interrupt(&self, continue_next: bool) -> bool {
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if *phase != BatchInterrupt::Waiting {
            return false;
        }
        *phase = if continue_next { BatchInterrupt::Continue } else { BatchInterrupt::Stop };
        self.shared.batch_decision.notify_all();
        true
    }

    /// Whether an action should stop before its next step because Esc was requested.
    pub fn batch_interrupt_requested(&self) -> bool {
        *self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner) == BatchInterrupt::Requested
    }

    /// Move an accepted Esc request to the question at a worker checkpoint.
    pub fn batch_take_interrupt(&self) -> bool {
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if *phase != BatchInterrupt::Requested {
            return *phase == BatchInterrupt::Waiting;
        }
        *phase = BatchInterrupt::Waiting;
        true
    }

    /// Wait for the UI decision without polling. Closing the session or `jobs.cancel` wakes this
    /// worker; the caller then reports its partial result.
    pub fn batch_wait_for_decision(&self) -> bool {
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        while *phase == BatchInterrupt::Waiting && !self.cancelled() {
            phase = self.shared.batch_decision.wait(phase).unwrap_or_else(PoisonError::into_inner);
        }
        let continue_next = *phase == BatchInterrupt::Continue && !self.cancelled();
        *phase = BatchInterrupt::Processing;
        continue_next
    }

    /// Mark the start of an input's output write. The same lock orders Esc against this
    /// checkpoint: a requested interruption never writes that input.
    pub(crate) fn batch_before_write(&self) -> Result<()> {
        self.check()?;
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if *phase == BatchInterrupt::Requested {
            *phase = BatchInterrupt::Waiting;
            return Err(EngineError::Cancelled);
        }
        if self.cancelled() {
            return Err(EngineError::Cancelled);
        }
        *phase = BatchInterrupt::Saving;
        Ok(())
    }

    /// The worker has passed its last checkpoint. A pending request is dropped (nothing is left to
    /// interrupt) and later requests return false.
    pub(crate) fn batch_close_interrupts(&self) {
        *self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner) = BatchInterrupt::Closed;
        self.shared.batch_decision.notify_all();
    }

    /// A completed write allows Esc on the next input.
    pub(crate) fn batch_after_write(&self) {
        let mut phase = self.shared.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if *phase == BatchInterrupt::Saving {
            *phase = BatchInterrupt::Processing;
        }
    }

    /// Publish the current input's document after it opens or an action step changes it.
    pub fn publish_batch_preview(&self, path: &str, document: Option<Arc<Document>>) {
        let mut preview = self.shared.batch_preview.lock().unwrap_or_else(PoisonError::into_inner);
        if preview.as_ref().zip(document.as_ref()).is_some_and(|(shown, next)| shown.path == path && Arc::ptr_eq(&shown.document, next))
            || (preview.is_none() && document.is_none())
        {
            return;
        }
        let generation = self.shared.batch_preview_generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        *preview = document.map(|document| BatchPreview { path: path.to_string(), document, generation });
    }

    /// A snapshot for rendering, if the current input still has a document open.
    pub fn batch_preview(&self) -> Option<BatchPreview> {
        self.shared.batch_preview.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn finish_work(&self) {
        self.shared.state.fetch_or(WORK_FINISHED, Ordering::Release);
    }

    /// `Err(Cancelled)` once cancelled, for `?` between stages.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
    }

    /// Progress reported so far (`0.0..=1.0`).
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.shared.progress.load(Ordering::Relaxed))
    }

    /// The latest status message.
    pub fn message(&self) -> String {
        self.shared.message.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Run `f` with an algorithm-level [`Interrupt`] whose progress maps onto `lo..hi` of this
    /// job (with `message`) and whose cancellation is this job's.
    pub fn stage<R>(&self, lo: f32, hi: f32, message: &str, f: impl FnOnce(&Interrupt) -> R) -> R {
        self.progress(lo, message);
        let cancel = || self.cancelled();
        let progress = |p: f32| self.progress(lo + (hi - lo) * p, "");
        f(&Interrupt::new(&cancel, &progress))
    }
}

/// Why [`Session::try_cancel_job`] left a job running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelRefused {
    /// No such job is running.
    NotRunning,
    /// A print belongs to the system print queue once it has started.
    Printing,
    /// Past its [commit](JobCtx::commit): it is writing its result (a save replacing its file)
    /// and will report its real outcome when it ends.
    Committing,
    /// A Batch already asked to stop, or past its last input: it ends on its own and reports
    /// the outputs it wrote.
    Finishing,
}

impl CancelRefused {
    /// The scripting name (`jobs.cancel`'s `notCancelled[].reason`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRunning => "notRunning",
            Self::Printing => "printing",
            Self::Committing => "committing",
            Self::Finishing => "finishing",
        }
    }
}

/// What a cancel of several jobs did ([`Session::cancel_all_jobs`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CancelReport {
    pub cancelled: Vec<JobId>,
    /// The jobs left running, and why.
    pub refused: Vec<(JobId, CancelRefused)>,
}

impl CancelReport {
    /// The `jobs.cancel` reply: `{cancelled: n}`, with `notCancelled: [{job, reason}]` when a
    /// job was left running.
    pub fn to_json(&self) -> Value {
        let mut v = json!({"cancelled": self.cancelled.len()});
        if !self.refused.is_empty() {
            v["notCancelled"] = self.refused.iter().map(|(id, why)| json!({"job": id.0, "reason": why.as_str()})).collect();
        }
        v
    }
}

/// Final state of a job.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutcome {
    /// Applied; the command's result.
    Done(Value),
    /// Failed (including a panic in the worker); the document is unchanged.
    Failed(String),
    /// Cancelled; the result (if any) was discarded and the document is unchanged.
    Cancelled,
}

/// A job that has ended, as reported by [`Session::poll_jobs`].
#[derive(Clone, Debug, PartialEq)]
pub struct JobEvent {
    pub id: JobId,
    pub command: String,
    pub label: String,
    pub document: Option<DocId>,
    pub outcome: JobOutcome,
    /// The typed failure behind a failed `outcome`: a hard-budget refusal keeps its
    /// name/required/limit, an approval request keeps its limits, a changed source keeps its
    /// kind. `None` for done, cancelled and panics without a typed error. Not serialized;
    /// `JobInfo::error` remains the display string on the wire.
    pub error: Option<EngineError>,
}

/// A job's state for the UI, the control channel and MCP (`jobs.list`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: JobId,
    pub command: String,
    /// Shown in the progress UI ("Gaussian Blur").
    pub label: String,
    /// The document the job edits (and locks), if any.
    pub document: Option<DocId>,
    /// `running`, `done`, `failed` or `cancelled`.
    pub state: &'static str,
    pub progress: f32,
    pub message: String,
    pub elapsed_ms: f64,
    /// The command's result (done), including partial outputs from a cancelled batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The typed failure behind `error`, for callers in this process (not serialized: the wire
    /// form is the message, as before).
    #[serde(skip)]
    pub error_kind: Option<EngineError>,
}

type Payload = Box<dyn Any + Send>;
type Apply = Box<dyn FnOnce(&mut Session, Payload) -> Result<Value> + Send>;

/// Milliseconds on the monotonic clock since some fixed point (`web_time` reads the browser's on wasm).
fn now_ms() -> f64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// How much of a frame [`Session::poll_jobs`] gives a [`StepWork`] job, unless
/// [`Session::set_job_step_budget`] says otherwise.
const STEP_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

/// A deadline no job reaches, for running [`StepWork`] to the end in one go.
fn no_deadline() -> web_time::Instant {
    let now = web_time::Instant::now();
    now.checked_add(std::time::Duration::from_secs(1 << 30)).unwrap_or(now)
}

/// Work a job does a slice at a time (see [`run_stepped`]).
pub trait StepWork<T>: Send + 'static {
    /// Does some of the work and returns `Ok(None)` for "not finished, call again", or the result.
    /// It should return soon after `deadline` passes (checking it between units of work), but
    /// always does at least one unit, so that calling it again makes progress. `ctx` reports
    /// progress and tells of cancellation, as for a worker.
    fn step(&mut self, ctx: &JobCtx, deadline: web_time::Instant) -> Result<Option<T>>;
}

/// Runs `work` to the end, as a worker does.
fn drive<T>(mut work: impl StepWork<T>, ctx: &JobCtx) -> Result<T> {
    loop {
        ctx.check()?;
        if let Some(t) = work.step(ctx, no_deadline())? {
            return Ok(t);
        }
    }
}

/// Runs one slice of a cooperative job: `Ok(None)` when it has more to do.
type Stepper = Box<dyn FnMut(&JobCtx, web_time::Instant) -> Result<Option<Payload>> + Send>;

struct Running {
    id: JobId,
    command: String,
    label: String,
    params: Value,
    journal: bool,
    /// The take active at dispatch, including strokes started with tool recording off.
    started_in_take: Option<u64>,
    /// The take eligible to receive this job's step under the mode at dispatch.
    recorded_by: Option<u64>,
    /// The request that started the job ran under an automation gate (`Session::authorize`, set
    /// by `dispatch`). Applied, its step marks the recording that owns it as automation's,
    /// whoever polls the job then (`actions_cmds::note_step_origin`).
    automation: bool,
    pre_selected: Option<Vec<LayerId>>,
    before: crate::actions_record::Before,
    doc: Option<DocId>,
    /// The document as the job saw it: applying fails if it was edited meanwhile.
    snapshot: Option<Weak<Document>>,
    ctx: JobCtx,
    started_ms: f64,
    coalesce: Option<String>,
    color_restrict: Option<crate::color_channels::Restriction>,
    filter_mask_layer: Option<LayerId>,
    /// The document's layer selection when the job started: applying targets it, not whatever
    /// got selected meanwhile (a Type click does that without editing the document, #7366).
    layers: Option<LayerSelection>,
    /// Where the worker thread sends its result. A cooperative job has none: [`Session::poll_jobs`]
    /// runs its `stepper` instead.
    rx: Option<std::sync::mpsc::Receiver<Result<Payload>>>,
    stepper: Option<Stepper>,
    apply: Option<Apply>,
    #[cfg(not(target_arch = "wasm32"))]
    handle: Option<std::thread::JoinHandle<()>>,
}

/// A document's active layer, selected layers and selection anchor.
#[derive(Clone, PartialEq)]
struct LayerSelection {
    active: Option<LayerId>,
    selected: Vec<LayerId>,
    anchor: Option<LayerId>,
}

impl LayerSelection {
    fn of(st: &crate::DocState) -> Self {
        Self { active: st.active_layer, selected: st.selected_layers.clone(), anchor: st.layer_anchor }
    }

    fn set(self, st: &mut crate::DocState) {
        st.active_layer = self.active;
        st.selected_layers = self.selected;
        st.layer_anchor = self.anchor;
        crate::channel_cmds::fix_view(st);
    }
}

impl Running {
    /// Runs a slice of a cooperative job until `deadline`: `Ok(None)` when it has more to do.
    /// A cancelled job ends with `Err(Cancelled)` before doing more.
    fn step(&mut self, deadline: web_time::Instant) -> Result<Option<Payload>> {
        self.ctx.check()?;
        match self.stepper.as_mut() {
            Some(stepper) => stepper(&self.ctx, deadline),
            None => Err(EngineError::Other(format!("`{}` has nothing to run", self.label))),
        }
    }

    fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            command: self.command.clone(),
            label: self.label.clone(),
            document: self.doc,
            state: "running",
            progress: self.ctx.fraction(),
            message: self.ctx.message(),
            elapsed_ms: now_ms() - self.started_ms,
            result: None,
            error: None,
            error_kind: None,
        }
    }
}

/// A cancelled job's worker that may still be unwinding.
#[cfg(not(target_arch = "wasm32"))]
struct Draining {
    handle: std::thread::JoinHandle<()>,
    doc: Option<DocId>,
    /// Its document closed: the worker still holds that document's snapshot, so the scratch
    /// pool is released again once it exits (the release at close came too early, #10479).
    release: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn reap_draining(draining: &mut Vec<Draining>) -> bool {
    let mut release = false;
    draining.retain(|d| {
        let done = d.handle.is_finished();
        release |= done && d.release;
        !done
    });
    release
}

/// The session's job table (see the module docs).
pub struct Jobs {
    next: u64,
    /// Set while [`Session::start`] runs a command: [`run`] spawns instead of running inline.
    spawn: bool,
    /// The job a command registered while `spawn` was set (picked up by `start`).
    pending: Option<Running>,
    running: Vec<Running>,
    /// Ended jobs not yet collected by [`Session::poll_jobs`].
    events: Vec<JobEvent>,
    /// The last few ended jobs, for `jobs.list`.
    recent: Vec<JobInfo>,
    /// Workers of cancelled jobs that may still be unwinding (native).
    #[cfg(not(target_arch = "wasm32"))]
    draining: Vec<Draining>,
    /// The context inline job work runs under (see [`Session::set_inline_job_ctx`]).
    inline: Option<JobCtx>,
    /// No worker threads: [`StepWork`] jobs are stepped by [`Session::poll_jobs`], everything else
    /// runs inline. On for wasm; see [`Session::set_cooperative_jobs`].
    cooperative: bool,
    /// How long [`Session::poll_jobs`] runs a cooperative job per call.
    step_budget: std::time::Duration,
}

// Derivable on a desktop, where `cooperative` starts off; wasm starts it on.
#[allow(clippy::derivable_impls)]
impl Default for Jobs {
    /// No jobs, spawning off; a session steps its jobs itself (cooperative) on wasm only.
    fn default() -> Self {
        Self {
            next: 0,
            spawn: false,
            pending: None,
            running: Vec::new(),
            events: Vec::new(),
            recent: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            draining: Vec::new(),
            inline: None,
            cooperative: cfg!(target_arch = "wasm32"),
            step_budget: STEP_BUDGET,
        }
    }
}

impl Jobs {
    /// Does a job started now run on a worker thread? (Not inline, and not stepped on this thread.)
    fn threaded(&self) -> bool {
        self.spawn && !self.cooperative && !cfg!(target_arch = "wasm32")
    }
}

const RECENT: usize = 16;

impl Drop for Jobs {
    fn drop(&mut self) {
        // Workers own snapshots only; stop them so they don't burn CPU for a closed session.
        for r in self.running.iter().chain(self.pending.iter()) {
            r.ctx.cancel();
        }
    }
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jobs").field("running", &self.running.len()).finish()
    }
}

/// How [`Session::start`] ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Started {
    /// The command ran to completion inline (it isn't a job, or this is wasm).
    Done(Value),
    /// The command is running in the background.
    Job(JobId),
}

/// Commands that never edit the active document, allowed while a job runs on it.
fn independent_of_document(id: &str) -> bool {
    const PREFIXES: &[&str] = &["jobs.", "tools.", "view.", "window.", "help.", "brush.presets.", "gradient.presets."];
    const IDS: &[&str] = &["file.new", "file.close", "file.closeAll", "edit.preferences", "edit.colorSettings", "edit.keyboardShortcuts", "edit.menus"];
    PREFIXES.iter().any(|p| id.starts_with(p)) || IDS.contains(&id)
}

/// Imports the file at `path`, taking its identity from the very handle it decodes.
///
/// `expected` is the identity approved for this file, if any. It is checked against the opened
/// handle before decoding and again when the memory preflight pauses for approval, so the
/// document opened is the approved file even if the path is replaced meanwhile. `opened` receives
/// the handle's identity as soon as it is known, including when the import stops for approval.
#[cfg(not(target_arch = "wasm32"))]
fn import_path(
    name: &str,
    path: &str,
    opts: &photocraft_io::ImportOptions,
    expected: Option<&SourceFingerprint>,
    ctl: &photocraft_raster::Interrupt,
    opened: &mut Option<photocraft_io::FileIdentity>,
) -> std::result::Result<(photocraft_io::ImportResult, bool), photocraft_io::IoError> {
    let expected = match expected {
        None => None,
        Some(SourceFingerprint::Path { identity, .. }) => Some(identity),
        Some(SourceFingerprint::Bytes { .. }) => return Err(photocraft_io::IoError::SourceChanged),
    };
    // Bounded reads, and a clear error for a file larger than memory (#375); a PSD/PSB is
    // streamed from the open file instead of being read whole.
    photocraft_io::import_file_for_open_verified(name, std::path::Path::new(path), expected, opts, ctl, opened)
}

#[cfg(target_arch = "wasm32")]
fn import_path(
    _: &str,
    path: &str,
    _: &photocraft_io::ImportOptions,
    _: Option<&SourceFingerprint>,
    _: &photocraft_raster::Interrupt,
    _: &mut Option<photocraft_io::FileIdentity>,
) -> std::result::Result<(photocraft_io::ImportResult, bool), photocraft_io::IoError> {
    Err(photocraft_io::IoError::Read(format!("cannot open paths on the web: {path}")))
}

/// Run a job-capable command's heavy part. `work` runs on a worker thread when the command was
/// started with [`Session::start`] (inline otherwise, and always on wasm); `apply` then runs on
/// the calling thread with its output and returns the command's result. With `lock_document`,
/// the active document is locked against other edits while the job runs, and applying fails if
/// it was edited anyway.
///
/// `work` must only read what it captured (clone the document `Arc` first): it never sees the
/// session.
pub fn run<T: Send + 'static>(
    s: &mut Session,
    label: &str,
    lock_document: bool,
    work: impl FnOnce(&JobCtx) -> Result<T> + Send + 'static,
    apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
) -> Result<Value> {
    // The job reads the active document it captured; only that document's pixels gate it.
    let source = s.active().map(|d| d.doc.clone());
    let work = move |ctx: &JobCtx| -> Result<T> {
        photocraft_compose::discard_unreadable_pixel_caches();
        let health = || source.as_deref().map_or(Ok(()), photocraft_doc::Document::scratch_health).map_err(EngineError::Other);
        health()?;
        let generation = photocraft_raster::spill::read_error_generation();
        let value = work(ctx)?;
        photocraft_raster::spill::check_since(generation).map_err(EngineError::Other)?;
        health()?;
        Ok(value)
    };
    let authorization = s.authorize.or_else(|| RENDER_AUTHORIZATION.with(std::cell::Cell::get));
    let _authorization = AuthorizationScope::enter(authorization);
    // The job's document: stored paths it follows are resolved against it on any thread.
    let document = s.active().and_then(|d| d.path.as_deref()).map(std::path::PathBuf::from);
    let _document = crate::doc_path::scope(document.as_deref());
    if !s.jobs.threaded() {
        let t = work(&s.jobs.inline.clone().unwrap_or_default())?;
        return apply(s, t);
    }
    let mut job = new_job(s, label, lock_document, authorization, document.clone(), apply)?;
    let (tx, rx) = std::sync::mpsc::channel::<Result<Payload>>();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let wctx = job.ctx.clone();
        let label_w = label.to_string();
        // The work closure is moved into the thread. If spawning fails (thread limits), there is
        // no way to get it back, so the error is reported instead.
        let spawned = std::thread::Builder::new().name(format!("photocraft-job-{}", job.id.0)).spawn(move || {
            let _authorization = AuthorizationScope::enter(authorization);
            let _document = crate::doc_path::scope(document.as_deref());
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&wctx).map(|t| Box::new(t) as Payload)))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{label_w}` failed with an internal error (logged); the document is unchanged"))));
            wctx.finish_work();
            // The receiver is gone when the job was cancelled: nothing to report.
            let _ = tx.send(r);
        });
        job.handle = Some(spawned.map_err(|e| EngineError::Other(format!("could not start a background job: {e}")))?);
    }
    #[cfg(target_arch = "wasm32")]
    {
        // Unreachable (wasm runs inline above); keep the types used.
        drop((work, tx));
    }
    job.rx = Some(rx);
    let id = job.id;
    s.jobs.pending = Some(job);
    Ok(json!({"job": id.0}))
}

/// [`run`] for work done a slice at a time ([`StepWork`]): where the session has no worker threads
/// ([`Session::set_cooperative_jobs`], the web build) the job runs on the UI thread, a slice of
/// about 8 ms per [`Session::poll_jobs`], so the window keeps repainting meanwhile. Elsewhere it
/// is [`run`] with `work` driven to the end: on a worker thread for [`Session::start`], inline for
/// [`Session::execute`]. The result is the same either way.
pub fn run_stepped<T: Send + 'static>(
    s: &mut Session,
    label: &str,
    lock_document: bool,
    work: impl StepWork<T>,
    apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
) -> Result<Value> {
    if !(s.jobs.spawn && s.jobs.cooperative) {
        return run(s, label, lock_document, move |ctx| drive(work, ctx), apply);
    }
    let authorization = s.authorize.or_else(|| RENDER_AUTHORIZATION.with(std::cell::Cell::get));
    let document = s.active().and_then(|d| d.path.as_deref()).map(std::path::PathBuf::from);
    let mut job = new_job(s, label, lock_document, authorization, document.clone(), apply)?;
    let (label_w, mut work) = (label.to_string(), work);
    // The slices run under the requesting session's authority and document, as a worker would.
    job.stepper = Some(Box::new(move |ctx, deadline| {
        let _authorization = AuthorizationScope::enter(authorization);
        let _document = crate::doc_path::scope(document.as_deref());
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work.step(ctx, deadline)))
            .unwrap_or_else(|_| Err(EngineError::Other(format!("`{label_w}` failed with an internal error (logged); the document is unchanged"))))
            .map(|done| done.map(|t| Box::new(t) as Payload))
    }));
    let id = job.id;
    s.jobs.pending = Some(job);
    Ok(json!({"job": id.0}))
}

/// A job for [`run`] and [`run_stepped`] to start, without its worker.
fn new_job<T: Send + 'static>(
    s: &mut Session,
    label: &str,
    lock_document: bool,
    authorization: Option<Authorization>,
    document: Option<std::path::PathBuf>,
    apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
) -> Result<Running> {
    if s.jobs.pending.is_some() {
        return Err(EngineError::Other(format!("`{label}` started a second background job in one command")));
    }
    #[cfg(not(target_arch = "wasm32"))]
    s.reap_draining_jobs();
    let admitted = s.jobs.running.len();
    #[cfg(not(target_arch = "wasm32"))]
    let admitted = admitted.saturating_add(s.jobs.draining.len());
    if admitted >= MAX_BACKGROUND_JOBS {
        return Err(EngineError::Other(format!(
            "too many background jobs ({MAX_BACKGROUND_JOBS}); wait for a running or cancelling job to finish before starting another"
        )));
    }
    let (doc, snapshot, layers) = match (lock_document, s.active()) {
        (true, Some(st)) => (Some(st.doc.id), Some(Arc::downgrade(&st.doc)), Some(LayerSelection::of(st))),
        (true, None) => return Err(EngineError::NoDocument),
        (false, _) => (None, None, None),
    };
    let erased: Apply = Box::new(move |s, any| {
        let _authorization = AuthorizationScope::enter(authorization);
        let _document = crate::doc_path::scope(document.as_deref());
        let t = any.downcast::<T>().map_err(|_| EngineError::Other("internal error: job result type mismatch".into()))?;
        apply(s, *t)
    });
    s.jobs.next = s.jobs.next.checked_add(1).ok_or_else(|| EngineError::Other("background job identifier space exhausted".into()))?;
    Ok(Running {
        id: JobId(s.jobs.next),
        command: String::new(),
        label: label.to_string(),
        params: Value::Null,
        journal: true,
        started_in_take: None,
        recorded_by: None,
        automation: false,
        pre_selected: None,
        before: crate::actions_record::Before::capture(s),
        doc,
        snapshot,
        ctx: JobCtx::new(),
        started_ms: now_ms(),
        coalesce: s.coalesce_request.clone(),
        color_restrict: s.color_restrict,
        filter_mask_layer: s.filter_mask_layer,
        layers,
        rx: None,
        stepper: None,
        apply: Some(erased),
        #[cfg(not(target_arch = "wasm32"))]
        handle: None,
    })
}

/// A document edit whose body is the job: like [`Session::edit`] (one undo step named `label`),
/// but `f` runs on a worker against a copy of the active document when started in the
/// background. `finish` turns `f`'s output into the command's result.
pub fn edit_job<R: Send + 'static>(
    s: &mut Session,
    label: &str,
    f: impl FnOnce(&mut Document, &mut Option<LayerId>, &JobCtx) -> Result<R> + Send + 'static,
    finish: impl FnOnce(R) -> Value + Send + 'static,
) -> Result<Value> {
    if !s.jobs.threaded() {
        // Inline: exactly `Session::edit`, as before jobs existed.
        let ctx = s.jobs.inline.clone().unwrap_or_default();
        let r = s.edit(label, |doc, active| f(doc, active, &ctx))?;
        return Ok(finish(r));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (base, active) = (st.doc.clone(), st.active_layer);
    let label_owned = label.to_string();
    run(
        s,
        label,
        true,
        move |ctx| {
            let mut doc = (*base).clone();
            let mut active = active;
            let r = f(&mut doc, &mut active, ctx)?;
            Ok((doc, active, r))
        },
        move |s, (doc, new_active, r)| {
            s.edit(&label_owned, move |d, active| {
                *d = doc;
                *active = new_active;
                Ok(())
            })?;
            Ok(finish(r))
        },
    )
}

/// Scan every layer surface's content bounds into the shared tile bounds cache, so the first
/// draw of a document opened in the background doesn't do it on the UI thread: GPU planning
/// asks each layer for its bounds, a multi-second scan on a multi-gigabyte PSB (#4024).
/// `check` runs between layers (cancellation).
pub fn warm_layer_bounds(doc: &Document, mut check: impl FnMut() -> Result<()>) -> Result<()> {
    let mut pending: Vec<&photocraft_doc::Layer> = doc.layers.iter().collect();
    while let Some(layer) = pending.pop() {
        check()?;
        if let Some(children) = layer.children() {
            pending.extend(children);
        }
        if let Some(surface) = layer.surface() {
            photocraft_compose::bounds::content_bounds(surface);
        }
    }
    Ok(())
}

/// Where a background open reads its file from.
#[derive(Clone, Debug)]
pub enum OpenSource {
    /// File contents already in memory (web pickers, drops, automation).
    Bytes(Arc<Vec<u8>>),
    /// A path the worker reads itself (native), so even the read stays off the UI thread.
    Path(String),
}

/// The command id open jobs report (there is no engine command: opening is the shell's).
pub const OPEN_JOB: &str = "file.open";

/// The import settings files are opened and placed with.
pub fn import_options(prefs: &crate::prefs::Preferences) -> photocraft_io::ImportOptions {
    photocraft_io::ImportOptions {
        max_group_depth: prefs.file_handling.rasterize_svg_groups_deeper_than as usize,
        demosaic: prefs.raw_defaults.demosaic.method(),
        keep_raw: false,
        pdf_dpi: prefs.file_handling.pdf_dpi,
        pdf_pages: match (prefs.file_handling.pdf_first_page, prefs.file_handling.pdf_last_page) {
            (0, 0) => None,
            (first, last) => {
                let first = first.max(1);
                Some((first, if last == 0 { u32::MAX } else { last.max(first) }))
            }
        },
        ..Default::default()
    }
}

/// Fingerprint verifying that a source has not changed between preflight and approval.
///
/// A path's fingerprint is the [`photocraft_io::FileIdentity`] of an opened handle, never a path
/// stat made before a separate open: the importer takes it from the very handle it decodes
/// (`photocraft_io::import_file_for_open_verified`), so a path replaced or rewritten between
/// the estimate and the approval is refused with [`EngineError::SourceChanged`]. The Unix
/// identity includes the inode change time and nanoseconds, so restoring a modification time
/// cannot hide a same-size rewrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceFingerprint {
    Bytes { len: usize, identity: usize },
    Path { path: String, identity: photocraft_io::FileIdentity },
}

impl SourceFingerprint {
    /// Fingerprint of `source`.
    ///
    /// An in-memory source's identity is its shared buffer. A path is inspected here for
    /// discovery only; the decode path re-reads its identity from the handle it opens. Fails
    /// closed when the identity data cannot be read.
    pub fn of(source: &OpenSource) -> Result<Self> {
        match source {
            OpenSource::Bytes(b) => Ok(SourceFingerprint::Bytes { len: b.len(), identity: Arc::as_ptr(b) as usize }),
            OpenSource::Path(p) => {
                Ok(Self::path(p.clone(), photocraft_io::FileIdentity::of_path(std::path::Path::new(p)).map_err(|e| EngineError::Other(format!("{p}: {e}")))?))
            }
        }
    }

    /// Fingerprint of the file identity read from an opened handle for `path`.
    pub fn path(path: String, identity: photocraft_io::FileIdentity) -> Self {
        SourceFingerprint::Path { path, identity }
    }

    /// The handle identity a path fingerprint compares against, if this is one.
    pub fn file_identity(&self) -> Option<&photocraft_io::FileIdentity> {
        match self {
            SourceFingerprint::Path { identity, .. } => Some(identity),
            SourceFingerprint::Bytes { .. } => None,
        }
    }

    /// Whether both fingerprints capture the same source identity (and, for a path, the same path).
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }

    /// Re-reads `source`'s fingerprint and compares it with this one; an unreadable source does not match.
    pub fn matches_source(&self, source: &OpenSource) -> bool {
        match Self::of(source) {
            Ok(cur) => self.matches(&cur),
            Err(_) => false,
        }
    }
}

/// A pending high-memory PSD preflight request waiting for user confirmation before pixel decoding.
#[derive(Clone, Debug)]
pub struct PendingPsdPreflight {
    pub id: u64,
    pub name: String,
    pub source: OpenSource,
    pub fingerprint: SourceFingerprint,
    pub required: u64,
    pub soft_limit: u64,
    pub hard_limit: u64,
    pub available_ram: Option<u64>,
    pub confirm_damage: bool,
}

impl Session {
    /// A recording mode change must not reclassify an in-flight stroke from this take.
    pub(crate) fn tool_stroke_job_running(&self) -> bool {
        self.jobs.running.iter().any(|job| job.started_in_take == Some(self.actions.take) && crate::actions_cmds::tool_stroke(&job.command))
    }

    /// Run the heavy part of inline (not background) job commands under `ctx`, so another thread
    /// can cancel them: a live preview whose parameters changed stops its stale computation
    /// (`Err(Cancelled)`, the document unchanged) instead of finishing it. `None` restores the
    /// default, a context nothing can cancel.
    pub fn set_inline_job_ctx(&mut self, ctx: Option<JobCtx>) {
        self.jobs.inline = ctx;
    }

    /// Run a command, in the background when it supports it (see [`jobs`](crate::jobs)). Errors
    /// like [`Session::execute`] for unknown, disabled or failing commands.
    pub fn start(&mut self, id: &str, params: Value) -> Result<Started> {
        let before = self.active_layer_stamp();
        let started = self.dispatch(id, params, true);
        self.note_layer_change(before);
        started
    }

    pub const MAX_PENDING_PREFLIGHTS: usize = 32;

    /// Most bytes of file data pending approvals may keep alive together.
    ///
    /// This is a retention bound for the *sources* of files waiting for an answer, not a pixel
    /// budget and not an admission limit: only an [`OpenSource::Bytes`] source holds data (a path
    /// streams from its file once opened), the allocation's capacity is charged rather than its
    /// contents, and one shared buffer counts once however many preflights refer to it. 1 GiB is
    /// chosen conservatively: it is below the 2 GiB web hard
    /// product cap, so even a web session that queues several large dropped files cannot spend
    /// the whole hard budget on data it has not decoded, and a native session keeps a comparable
    /// amount out of the adaptive allowance. A file that would exceed the cap cannot become
    /// pending: it is refused with an error instead of being decoded, and the caller can answer
    /// or cancel queued prompts and try again, rather than holding an unbounded amount of memory
    /// for unanswered prompts. The record count
    /// ([`Self::MAX_PENDING_PREFLIGHTS`]) stays as a separate bound, so many small files are
    /// bounded too.
    pub const MAX_PENDING_PREFLIGHT_BYTES: u64 = 1 << 30;

    /// Queues an approval prompt, assigning its id, and enforces both the pending-count and
    /// retained-byte limits.
    pub fn store_pending_preflight(&mut self, preflight: PendingPsdPreflight) -> Result<u64> {
        self.store_pending_preflight_with_limit(preflight, Self::MAX_PENDING_PREFLIGHT_BYTES)
    }

    /// [`Self::store_pending_preflight`] with an explicit retention cap (tests use a tiny one so
    /// the boundary is exercised without holding a gigabyte).
    pub(crate) fn store_pending_preflight_with_limit(&mut self, mut preflight: PendingPsdPreflight, byte_limit: u64) -> Result<u64> {
        if self.pending_preflights.len() >= Self::MAX_PENDING_PREFLIGHTS {
            return Err(EngineError::Other("too many pending high-memory preflights waiting for confirmation".into()));
        }
        let held = self.pending_preflight_bytes_with(&preflight.source)?;
        if held > byte_limit {
            return Err(EngineError::Other(format!(
                "too much memory is already held by files waiting for a memory approval ({held} bytes, limit {byte_limit}); answer or cancel one of them and try again"
            )));
        }
        loop {
            self.next_pending_preflight = self.next_pending_preflight.checked_add(1).unwrap_or(1);
            if self.next_pending_preflight != 0 && !self.pending_preflights.iter().any(|p| p.id == self.next_pending_preflight) {
                break;
            }
        }
        preflight.id = self.next_pending_preflight;
        self.pending_preflights.push(preflight);
        Ok(self.next_pending_preflight)
    }

    /// Bytes held alive by the queued preflights, counting an `Arc<Vec<u8>>` shared by several of
    /// them once. Computed from the queue, so approving, cancelling, refusing or clearing always
    /// releases the charge; a rejected preflight never adds one.
    pub fn pending_preflight_bytes(&self) -> Result<u64> {
        Self::retained_source_bytes(self.pending_preflights.iter().map(|p| &p.source))
    }

    /// [`Self::pending_preflight_bytes`] as if `incoming` were queued too, so accepting it can be
    /// checked before the list is changed.
    fn pending_preflight_bytes_with(&self, incoming: &OpenSource) -> Result<u64> {
        let mut sources: Vec<&OpenSource> = self.pending_preflights.iter().map(|p| &p.source).collect();
        sources.push(incoming);
        Self::retained_source_bytes(sources)
    }

    /// Total retained capacity of distinct in-memory sources, counting a shared buffer once.
    ///
    /// Capacity, not length, is charged because a truncated `Vec` keeps its allocation; overflow
    /// is an error rather than a wrapped total.
    fn retained_source_bytes<'a>(sources: impl IntoIterator<Item = &'a OpenSource>) -> Result<u64> {
        let mut seen: Vec<usize> = Vec::new();
        let mut total: u64 = 0;
        for source in sources {
            let OpenSource::Bytes(bytes) = source else { continue };
            let identity = Arc::as_ptr(bytes) as usize;
            // Distinct live allocations cannot share an address, so identity dedupes clones of
            // one buffer without under-counting two different files with equal contents.
            if seen.contains(&identity) {
                continue;
            }
            seen.push(identity);
            // The allocation is what stays resident, not its contents: a `Vec` that once held a
            // large file keeps that capacity after being truncated, so charging `len()` would
            // under-count the memory a pending prompt holds. Overflow is an error, never a wrap.
            let held = u64::try_from(bytes.capacity()).unwrap_or(u64::MAX);
            total = total.checked_add(held).ok_or_else(|| EngineError::Other("held file size overflow while counting pending memory approvals".into()))?;
        }
        Ok(total)
    }

    /// Removes and returns the pending preflight with `id`, releasing the source it held.
    pub fn take_pending_preflight(&mut self, id: u64) -> Option<PendingPsdPreflight> {
        let i = self.pending_preflights.iter().position(|p| p.id == id)?;
        Some(self.pending_preflights.remove(i))
    }

    /// Cancels the pending preflight with `id` by removing it; `None` means no such prompt.
    pub fn cancel_pending_preflight(&mut self, id: u64) -> Option<PendingPsdPreflight> {
        self.take_pending_preflight(id)
    }

    /// Drops every pending preflight, releasing all retained source buffers.
    pub fn clear_pending_preflights(&mut self) {
        self.pending_preflights.clear();
    }

    /// Starts an open with high-memory approval already granted, on a background worker.
    /// `expected_fingerprint`, when given, must still match the decoded source.
    pub fn start_open_approved(
        &mut self,
        name: &str,
        source: OpenSource,
        confirm_damage: bool,
        approved_estimate: Option<u64>,
        expected_fingerprint: Option<SourceFingerprint>,
    ) -> Result<Started> {
        self.start_open_approved_with_background(name, source, confirm_damage, approved_estimate, expected_fingerprint, true)
    }

    /// [`Self::start_open_approved`] with an explicit choice between a background worker and
    /// inline work.
    pub fn start_open_approved_with_background(
        &mut self,
        name: &str,
        source: OpenSource,
        confirm_damage: bool,
        approved_estimate: Option<u64>,
        expected_fingerprint: Option<SourceFingerprint>,
        background: bool,
    ) -> Result<Started> {
        self.start_open_inner(name, source, confirm_damage, true, approved_estimate, expected_fingerprint, background)
    }

    /// Open a file in the background: read (for a path) and decode on a worker, with progress
    /// (per layer for PSD/PSB) and cancellation, then add the document like
    /// [`Session::open_document`] (Color Settings policies apply). `name` names the document.
    /// The job's result is `{document, docId, name, warnings, color}`; on wasm it runs inline.
    /// `document` is the tab index at apply time; resolve `docId` before handling a delayed
    /// completion, since earlier handlers may have moved or closed tabs. When
    /// Color Settings ask about the file's profile, the document is held back instead and the
    /// result is `{held, name, warnings, color}` (`color` the question): take it with
    /// [`Session::take_held_open`] once answered.
    pub fn start_open(&mut self, name: &str, source: OpenSource) -> Result<Started> {
        self.start_open_inner(name, source, false, false, None, None, true)
    }

    /// Interactive opening: hold a damaged PSD until the user answers a damage
    /// confirmation, without changing import or subsequent save behaviour.
    /// Headless callers keep using [`Self::start_open`].
    pub fn start_open_for_user(&mut self, name: &str, source: OpenSource) -> Result<Started> {
        self.start_open_for_user_with_background(name, source, true)
    }

    /// Interactive open with an explicit background/inline choice: damaged PSD content and
    /// high-memory preflights wait for the user instead of opening non-interactively.
    pub fn start_open_for_user_with_background(&mut self, name: &str, source: OpenSource, background: bool) -> Result<Started> {
        self.start_open_inner(name, source, true, false, None, None, background)
    }

    /// Place a file's bytes in the background, like `file.placeEmbedded` (`linked`: `file.placeLinked`
    /// to that path) for bytes that have no path to read: decode, flatten and fit on a worker, then
    /// insert the layer as one undo step when [`Session::poll_jobs`] applies it. `name` names the
    /// layer and the file; `params` are the command's (`scale`, `center`, `fit`, `smartObject`).
    /// The job's result is the command's: `{layer, frame, scale, bounds, smartObject}`. The active
    /// document is locked meanwhile, so this is refused while another job runs on it. On wasm (and
    /// in a [cooperative](Session::set_cooperative_jobs) session) it is not a worker's job but
    /// one [`Session::poll_jobs`] runs a slice of per call.
    pub fn start_place_bytes(&mut self, name: &str, bytes: Vec<u8>, linked: Option<String>, params: Value) -> Result<Started> {
        let input = crate::file_cmds::PlaceInput::new(self, name, bytes, linked, &params)?;
        let (command, label) = (input.command(), input.job_label());
        if let Some(conflict) = self.job_conflict(command, true) {
            return Err(EngineError::Other(conflict));
        }
        let work = crate::file_cmds::PlaceSteps::new(input, self.cooperative_jobs());
        self.start_job_stepped(command, json!({"name": name}), &label, true, work, crate::file_cmds::apply_place)
    }

    /// Decodes a file in a job and optionally holds damaged PSD content for
    /// interactive confirmation while leaving headless opens non-interactive.
    // Keep the worker and apply closures together with their execution options.
    #[allow(clippy::too_many_arguments)]
    fn start_open_inner(
        &mut self,
        name: &str,
        source: OpenSource,
        confirm_damage: bool,
        approve_high_memory: bool,
        approved_estimate: Option<u64>,
        expected_fingerprint: Option<SourceFingerprint>,
        background: bool,
    ) -> Result<Started> {
        let label = format!("Opening {name}");
        let name_w = name.to_string();
        let name_a = name.to_string();
        let mut opts = import_options(self.prefs());
        // An interactive raw open keeps its sensor data for the Camera Raw dialog it may show.
        opts.keep_raw = confirm_damage && self.prefs().raw_defaults.open_in_camera_raw;
        opts.psd_high_memory_approved = approve_high_memory;
        opts.psd_approved_estimate = approved_estimate;
        let test_memory_scope = photocraft_io::memory::TestMemoryScope::capture();
        enum WorkResult {
            Decoded(Box<(photocraft_io::ImportResult, bool)>),
            PreflightNeeded { required: u64, soft_limit: u64, hard_limit: u64, available_ram: Option<u64>, source: OpenSource, fingerprint: SourceFingerprint },
        }
        self.start_job_with_spawn(
            OPEN_JOB,
            json!({"name": name}),
            &label,
            false,
            background,
            move |ctx| {
                let _test_memory = test_memory_scope.enter();
                ctx.progress(0.0, "Reading");

                // An in-memory source's identity is its shared buffer, known before decoding. A
                // path's identity can only come from the handle the verified import opens, so
                // that import compares it against `expected_fingerprint` itself, before reading
                // any pixel and again after its memory preflight.
                let initial_fingerprint = match &source {
                    OpenSource::Bytes(_) => Some(SourceFingerprint::of(&source)?),
                    OpenSource::Path(_) => None,
                };
                if let Some(expected) = &expected_fingerprint
                    && let Some(current) = &initial_fingerprint
                    && !expected.matches(current)
                {
                    return Err(EngineError::SourceChanged { name: name_w.clone() });
                }

                let outcome = match source {
                    OpenSource::Bytes(bytes) => {
                        ctx.check()?;
                        ctx.progress(0.02, "Decoding");
                        let res = ctx.stage(0.02, 0.95, "Decoding", |ctl| photocraft_io::import_for_open_with(&name_w, &bytes, &opts, ctl));
                        match res {
                            Ok(imported) => WorkResult::Decoded(Box::new(imported)),
                            Err(photocraft_io::IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) if confirm_damage => {
                                // The buffer cannot change while it is decoded, so the identity
                                // captured before reading is still exact.
                                let fingerprint = initial_fingerprint.clone().ok_or_else(|| EngineError::Other("missing source identity".into()))?;
                                WorkResult::PreflightNeeded { required, soft_limit, hard_limit, available_ram, source: OpenSource::Bytes(bytes), fingerprint }
                            }
                            Err(photocraft_io::IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) => {
                                return Err(EngineError::PsdApprovalRequired { name: name_w.clone(), required, soft_limit, hard_limit, available_ram });
                            }
                            Err(photocraft_io::IoError::Cancelled) => return Err(EngineError::Cancelled),
                            Err(e) => return Err(crate::error::io_open_error(&name_w, e)),
                        }
                    }
                    OpenSource::Path(p) => {
                        let mut opened = None;
                        let res = ctx.stage(0.0, 0.95, "Decoding", |ctl| import_path(&name_w, &p, &opts, expected_fingerprint.as_ref(), ctl, &mut opened));
                        match res {
                            Ok(imported) => WorkResult::Decoded(Box::new(imported)),
                            Err(photocraft_io::IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) if confirm_damage => {
                                // This identity is the one read from the handle that was
                                // estimated, and the verified import refused a handle that is
                                // not the approved file or changed while preflighting.
                                let identity = opened.ok_or_else(|| EngineError::Other(format!("{p}: the file's identity could not be read")))?;
                                let fingerprint = SourceFingerprint::path(p.clone(), identity);
                                WorkResult::PreflightNeeded { required, soft_limit, hard_limit, available_ram, source: OpenSource::Path(p), fingerprint }
                            }
                            Err(photocraft_io::IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }) => {
                                return Err(EngineError::PsdApprovalRequired { name: name_w.clone(), required, soft_limit, hard_limit, available_ram });
                            }
                            Err(photocraft_io::IoError::Cancelled) => return Err(EngineError::Cancelled),
                            Err(e) => return Err(crate::error::io_open_error(&name_w, e)),
                        }
                    }
                };

                match &outcome {
                    WorkResult::PreflightNeeded { .. } => Ok(outcome),
                    WorkResult::Decoded(imported) => {
                        ctx.progress(0.95, "Preparing layers");
                        warm_layer_bounds(&imported.0.document, || ctx.check())?;
                        if crate::type_extra_cmds::has_type_layers(&imported.0.document) {
                            ctx.progress(0.97, "Loading fonts");
                            let _ = photocraft_text::shared();
                            ctx.check()?;
                        }
                        Ok(outcome)
                    }
                }
            },
            move |s, outcome| match outcome {
                WorkResult::PreflightNeeded { required, soft_limit, hard_limit, available_ram, source, fingerprint } => {
                    // The warning follows the capacity measured for this estimate: an unknown
                    // capacity carries no fabricated number, and above the threshold it also
                    // says the process or tab may run out of memory. The UI shows both.
                    let capacity_unknown = available_ram.is_none();
                    let uncertain_capacity = photocraft_io::memory::uncertain_capacity_warning(required, available_ram);
                    let id = s.store_pending_preflight(PendingPsdPreflight {
                        id: 0,
                        name: name_a.clone(),
                        source,
                        fingerprint,
                        required,
                        soft_limit,
                        hard_limit,
                        available_ram,
                        confirm_damage,
                    })?;
                    Ok(json!({
                        "pendingPreflight": id,
                        "name": name_a,
                        "estimated": required,
                        "softLimit": soft_limit,
                        "hardLimit": hard_limit,
                        "availableRam": available_ram,
                        "capacityUnknown": capacity_unknown,
                        "uncertainCapacityWarning": uncertain_capacity,
                    }))
                }
                WorkResult::Decoded(imported) => {
                    let (mut r, damaged) = *imported;
                    crate::file_cmds::tag_exr_colorspace(s, &name_a, &mut r.document);
                    crate::file_cmds::raw_defaults(s, &r.warnings, &mut r.document);
                    let raw = r.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE));
                    let damaged = confirm_damage && damaged;
                    let question = s.color.open_question(&r.document).filter(|_| !raw);
                    if damaged || question.is_some() {
                        let held = s.hold_open(r.document, r.source_read_only);
                        return Ok(json!({"held": held, "name": name_a, "warnings": r.warnings, "color": question, "damaged": damaged}));
                    }
                    let (index, color) = s.open_document(r.document, None);
                    let id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
                    if let Some(st) = s.active_mut() {
                        st.source_read_only = r.source_read_only;
                    }
                    Ok(json!({"document": index, "docId": id.0, "name": name_a, "warnings": r.warnings, "color": color}))
                }
            },
        )
    }

    /// Start a job that isn't an engine command (opening a file, a shell operation, a test's fake
    /// job): `work` runs on a worker, `apply` on this thread when [`Session::poll_jobs`] or
    /// [`Session::wait_job`] picks the result up. `command` and `params` identify it in
    /// [`JobInfo`]; it is not journaled. With `lock_document` the active document is locked as
    /// for a command. Runs inline on wasm.
    pub fn start_job<T: Send + 'static>(
        &mut self,
        command: &str,
        params: Value,
        label: &str,
        lock_document: bool,
        work: impl FnOnce(&JobCtx) -> Result<T> + Send + 'static,
        apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
    ) -> Result<Started> {
        self.start_job_with_spawn(command, params, label, lock_document, true, work, apply)
    }

    // Keep the worker and apply closures together with their execution options.
    /// [`Self::start_job`] with an explicit `spawn` flag: when false (or on wasm) the work runs
    /// inline and its result is returned directly; otherwise it becomes a background job.
    #[allow(clippy::too_many_arguments)]
    pub fn start_job_with_spawn<T: Send + 'static>(
        &mut self,
        command: &str,
        params: Value,
        label: &str,
        lock_document: bool,
        spawn: bool,
        work: impl FnOnce(&JobCtx) -> Result<T> + Send + 'static,
        apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
    ) -> Result<Started> {
        self.start_registered(command, params, spawn, |s| run(s, label, lock_document, work, apply))
    }

    /// [`Session::start_job`] for [`StepWork`]: on a worker, or where the session is cooperative
    /// (see [`Session::set_cooperative_jobs`]) a slice per [`Session::poll_jobs`] on this thread.
    pub fn start_job_stepped<T: Send + 'static>(
        &mut self,
        command: &str,
        params: Value,
        label: &str,
        lock_document: bool,
        work: impl StepWork<T>,
        apply: impl FnOnce(&mut Session, T) -> Result<Value> + Send + 'static,
    ) -> Result<Started> {
        self.start_registered(command, params, true, |s| run_stepped(s, label, lock_document, work, apply))
    }

    /// Runs `register` (which calls [`run`] or [`run_stepped`]) with background jobs allowed when
    /// `spawn` (the jobs then run on a worker, or on wasm inline or in slices, see
    /// `Jobs::threaded`), and files the job it started under `command`.
    fn start_registered(&mut self, command: &str, params: Value, spawn: bool, register: impl FnOnce(&mut Session) -> Result<Value>) -> Result<Started> {
        let prev_spawn = self.jobs.spawn;
        self.jobs.spawn = spawn;
        let r = register(self);
        self.jobs.spawn = prev_spawn;
        let pending = self.jobs.pending.take();
        match (r, pending) {
            (Err(e), Some(job)) => {
                job.ctx.cancel();
                Err(e)
            }
            (Err(e), None) => Err(self.explain_missing_layer(e)),
            (Ok(_), Some(mut job)) => {
                job.command = command.to_string();
                job.params = params;
                job.journal = false;
                let id = job.id;
                self.jobs.running.push(job);
                Ok(Started::Job(id))
            }
            (Ok(v), None) => Ok(Started::Done(v)),
        }
    }

    /// Whether this session has no worker threads: [`StepWork`] jobs are run a slice at a time by
    /// [`Session::poll_jobs`] on the UI thread, and every other command runs inline. The web
    /// build (wasm) is always like this; tests turn it on to run that path on a desktop.
    pub fn cooperative_jobs(&self) -> bool {
        self.jobs.cooperative
    }

    /// How long one [`Session::poll_jobs`] runs a cooperative job (8 ms by default, a share of a
    /// frame). A slice always does one unit of work, so zero makes every poll do just that: tests
    /// use it to count polls without depending on the machine's speed.
    pub fn set_job_step_budget(&mut self, budget: std::time::Duration) {
        self.jobs.step_budget = budget;
    }

    /// Turn [`Session::cooperative_jobs`] on or off. Jobs already running keep running as they
    /// started.
    pub fn set_cooperative_jobs(&mut self, on: bool) {
        self.jobs.cooperative = on;
    }

    /// Apply finished background jobs (on this thread) and return every job that ended since
    /// the last call. Also runs one slice (about 8 ms) of a cooperative job, if there is one. Cheap
    /// when nothing runs; the UI calls it every frame.
    pub fn poll_jobs(&mut self) -> Vec<JobEvent> {
        let mut i = 0;
        let mut stepped = false;
        let budget = self.jobs.step_budget;
        while i < self.jobs.running.len() {
            let Some(job) = self.jobs.running.get_mut(i) else { break };
            let finished = match &job.rx {
                Some(rx) => match rx.try_recv() {
                    Ok(r) => Some(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some(Err(EngineError::Other(format!("`{}` stopped unexpectedly; the document is unchanged", job.label))))
                    }
                },
                // One cooperative job gets the frame's budget; any others wait for the next poll.
                None if !stepped => {
                    stepped = true;
                    // An absurd budget (`Duration::MAX` through `set_job_step_budget`) must not
                    // overflow the deadline: fall back to one no job reaches.
                    job.step(web_time::Instant::now().checked_add(budget).unwrap_or_else(no_deadline)).transpose()
                }
                None => None,
            };
            match finished {
                Some(r) => {
                    let job = self.jobs.running.remove(i);
                    self.finish_job(job, r);
                }
                None => i += 1,
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.reap_draining_jobs();
        std::mem::take(&mut self.jobs.events)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reap_draining_jobs(&mut self) {
        if reap_draining(&mut self.jobs.draining) {
            photocraft_raster::scratch::release();
        }
    }

    /// Block until job `id` ends, apply it, and return its result (the control channel's and
    /// MCP's default `wait`, and the CLI). `Err(Cancelled)` if it was cancelled. A cooperative job
    /// is run to the end here.
    pub fn wait_job(&mut self, id: JobId) -> Result<Value> {
        if let Some(i) = self.jobs.running.iter().position(|r| r.id == id) {
            let mut job = self.jobs.running.remove(i);
            let r = match &job.rx {
                Some(rx) => rx.recv().unwrap_or_else(|_| Err(EngineError::Other(format!("`{}` stopped unexpectedly", job.label)))),
                None => loop {
                    if let Some(r) = job.step(no_deadline()).transpose() {
                        break r;
                    }
                },
            };
            self.finish_job(job, r);
        }
        match self.jobs.recent.iter().rev().find(|j| j.id == id) {
            Some(j) if j.state == "done" => Ok(j.result.clone().unwrap_or(Value::Null)),
            Some(j) if j.state == "cancelled" => Err(EngineError::Cancelled),
            // A typed failure keeps its fields for a caller that answers or explains it; the
            // display string stays available through `jobs.list`.
            Some(j) => Err(j.error_kind.clone().unwrap_or_else(|| EngineError::Other(j.error.clone().unwrap_or_else(|| "the job failed".into())))),
            None => Err(EngineError::Other(format!("no job {}", id.0))),
        }
    }

    /// Cancel job `id`: ordinary jobs stop at their next check, discard their result and leave
    /// the document unchanged; the document is unlocked at once. A batch finishes the current
    /// input and reports outputs already written. False if no such job is running or it
    /// can't be cancelled (see [`Self::try_cancel_job`]).
    pub fn cancel_job(&mut self, id: JobId) -> bool {
        self.try_cancel_job(id).is_ok()
    }

    /// [`Self::cancel_job`], saying why a job was not cancelled.
    pub fn try_cancel_job(&mut self, id: JobId) -> std::result::Result<(), CancelRefused> {
        let Some(i) = self.jobs.running.iter().position(|r| r.id == id) else { return Err(CancelRefused::NotRunning) };
        // Earlier batch outputs are already on disk. Let the worker stop after its current
        // input and return the completed and skipped lists.
        if self.jobs.running[i].command == "file.automate.batch" {
            return if self.jobs.running[i].ctx.cancel_if_running() { Ok(()) } else { Err(CancelRefused::Finishing) };
        }
        // A print worker can already have handed the job to the OS. Discarding its result
        // would falsely report cancellation; removal belongs to the system print queue.
        if matches!(self.jobs.running[i].command.as_str(), "file.print" | "file.printOneCopy") {
            return Err(CancelRefused::Printing);
        }
        // Past its commit (a save replacing its file) the job runs to the end and reports its
        // real outcome; calling it cancelled would be false.
        if !self.jobs.running[i].ctx.try_cancel() {
            return Err(CancelRefused::Committing);
        }
        let job = self.jobs.running.remove(i);
        self.end_job(&job, JobOutcome::Cancelled);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handle) = job.handle {
            self.jobs.draining.push(Draining { handle, doc: job.doc, release: false });
        }
        Ok(())
    }

    /// Ask an interactive Batch worker to stop its current input at the next safe checkpoint.
    /// Headless clients keep using [`Self::cancel_job`] for a direct partial result.
    pub fn request_batch_interrupt_job(&self, id: JobId) -> bool {
        self.jobs.running.iter().find(|job| job.id == id && job.command == "file.automate.batch").is_some_and(|job| job.ctx.request_batch_interrupt())
    }

    /// Whether a Batch worker has acknowledged Esc and is waiting for the desktop decision.
    pub fn batch_interrupt_waiting(&self, id: JobId) -> bool {
        self.jobs.running.iter().find(|job| job.id == id && job.command == "file.automate.batch").is_some_and(|job| job.ctx.batch_waiting_for_decision())
    }

    /// Continue with the next input or stop a Batch paused by the desktop Esc key.
    pub fn decide_batch_interrupt_job(&self, id: JobId, continue_next: bool) -> bool {
        self.jobs
            .running
            .iter()
            .find(|job| job.id == id && job.command == "file.automate.batch")
            .is_some_and(|job| job.ctx.decide_batch_interrupt(continue_next))
    }

    /// The current read-only folder Batch document shown by the desktop while work runs.
    pub fn batch_preview(&self, id: JobId) -> Option<BatchPreview> {
        self.jobs.running.iter().find(|job| job.id == id && job.command == "file.automate.batch").and_then(|job| job.ctx.batch_preview())
    }

    /// Cancel every running job (e.g. before quitting). Prints and jobs past their commit keep
    /// running; the report lists them.
    pub fn cancel_all_jobs(&mut self) -> CancelReport {
        let ids: Vec<JobId> = self.jobs.running.iter().map(|r| r.id).collect();
        self.cancel_each(ids)
    }

    fn cancel_each(&mut self, ids: Vec<JobId>) -> CancelReport {
        let mut report = CancelReport::default();
        for id in ids {
            match self.try_cancel_job(id) {
                Ok(()) => report.cancelled.push(id),
                Err(why) => report.refused.push((id, why)),
            }
        }
        report
    }

    /// Is any job running? (Cheap: the UI asks every frame.)
    pub fn has_jobs(&self) -> bool {
        !self.jobs.running.is_empty()
    }

    /// Running job `id`.
    pub fn job(&self, id: JobId) -> Option<JobInfo> {
        self.jobs.running.iter().find(|r| r.id == id).map(Running::info)
    }

    /// Running jobs.
    pub fn jobs(&self) -> Vec<JobInfo> {
        self.jobs.running.iter().map(Running::info).collect()
    }

    /// Running jobs followed by the last few that ended (newest last), for `jobs.list`.
    pub fn jobs_with_recent(&self) -> Vec<JobInfo> {
        self.jobs().into_iter().chain(self.jobs.recent.iter().cloned()).collect()
    }

    /// The running job that locks document `doc`, if any.
    pub fn job_on(&self, doc: DocId) -> Option<JobInfo> {
        self.jobs.running.iter().find(|r| r.doc == Some(doc)).map(Running::info)
    }

    /// The running job that locks the active document, if any.
    pub fn active_job(&self) -> Option<JobInfo> {
        self.active().and_then(|st| self.job_on(st.doc.id))
    }

    /// Wait for background workers of cancelled jobs to exit (tests and benchmarks measure
    /// cancel latency with it). Native only; returns at once on wasm.
    pub fn join_cancelled_jobs(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut release = false;
            for d in self.jobs.draining.drain(..) {
                let _ = d.handle.join();
                release |= d.release;
            }
            if release {
                photocraft_raster::scratch::release();
            }
        }
    }

    /// Why command `id` can't run now because of a background job (None = no conflict).
    pub(crate) fn job_conflict(&self, id: &str, journal: bool) -> Option<String> {
        if !journal || independent_of_document(id) {
            return None;
        }
        let job = self.active_job()?;
        Some(format!("“{}” is still running on this document; wait for it to finish or cancel it (Esc)", job.label))
    }

    /// Cancel the jobs editing document `doc` (it is closing). A job past its commit keeps
    /// running and its result is still applied (see [`JobCtx::commit`]).
    pub(crate) fn cancel_jobs_on(&mut self, doc: DocId) -> CancelReport {
        let ids: Vec<JobId> = self.jobs.running.iter().filter(|r| r.doc == Some(doc)).map(|r| r.id).collect();
        let report = self.cancel_each(ids);
        // Workers cancelled now, or earlier and still unwinding, hold the closing document.
        #[cfg(not(target_arch = "wasm32"))]
        for d in self.jobs.draining.iter_mut().filter(|d| d.doc == Some(doc)) {
            d.release = true;
        }
        report
    }

    /// Run the command `id`: inline, or (with `background`) as a job when it is one.
    pub(crate) fn dispatch(&mut self, id: &str, params: Value, background: bool) -> Result<Started> {
        // The offer belongs to this one dispatch. Early errors drop it locally, including
        // authorization, validation, floating-selection and admission failures.
        let live_commit = self.live_commit.take();
        self.sync_scratch_state();
        let spec = crate::commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // Params are named fields: anything but an object (or null, meaning none) is a caller
        // mistake, not "use every default".
        if !(params.is_object() || params.is_null()) {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: "params must be a JSON object".into() });
        }
        let params = crate::file_cmds::batch_open_params(self, id, &params).unwrap_or(params);
        // Untrusted sessions gate every command, including the ones a command runs on its own
        // behalf (`file.automate.conditionalModeChange` runs `image.mode.*`), before any side
        // effect.
        if let Some(gate) = self.authorize {
            gate(id, &params)?;
        }
        // Whether this request runs under automation's gate: its step, journaled now or when its
        // job is applied, marks the recording that owns it as automation's.
        let automation = self.authorize.is_some();
        let _authorization = AuthorizationScope::enter(self.authorize);
        let _document = crate::doc_path::scope(self.active().and_then(|d| d.path.as_deref()).map(std::path::Path::new));
        // Last Filter runs as the filter it repeats: journaled, recorded in an action, faded and
        // run as a job like that filter, so a recorded step replays the same filter anywhere
        // (#3089), as Transform Again records its transform (#1440). With no filter yet it falls
        // through to its own precondition.
        if id == "filter.lastFilter"
            && let Some((filter, mut p)) = crate::filters::last_filter(self).filter(|(f, _)| f != id)
        {
            for key in ["target", "layer"] {
                if let Some(value) = params.get(key) {
                    if p.is_null() {
                        p = serde_json::json!({});
                    }
                    if let Some(object) = p.as_object_mut() {
                        object.insert(key.into(), value.clone());
                    }
                }
            }
            return self.dispatch(&filter, p, background);
        }
        if id == "file.new" {
            crate::document_preset_cmds::validate_new_params(self, &params)?;
        }
        crate::param_check::check(spec, &params)?;
        crate::filters::validate_params(id, &params)?;
        // A floating selection drops before any other command (Undo puts it back instead).
        if let Some(v) = crate::float_cmds::before_command(self, id)? {
            return Ok(Started::Done(v));
        }
        // Pixel commands follow the Channels panel target unless the caller names one.
        let run_params = self.admit(id, spec, &params)?;
        // This UI-only disclosure preference applies to both `execute` and `start`:
        // desktop UI normally calls `start`, even for synchronous Layer Style commands.
        // The layer the command will change, named as the command accepts it (id or name), else the
        // active one. `layer.layerStyle.clear` needs no case: it leaves no effects, so nothing expands.
        // Transfer Effects changes its destination (`to`), not the layer it takes the effects from.
        let new_fx_target = if id.starts_with("layer.layerStyle.") {
            let target = if id == "layer.layerStyle.transferEffects" {
                crate::commands::layer_param(self, &json!({"layer": params.get("to").cloned().unwrap_or(Value::Null)}))
            } else {
                crate::commands::layer_param(self, &params)
            }
            .ok();
            self.active().and_then(|st| {
                let target = target?;
                (st.doc.layer(target)?.effects.items.is_empty()).then_some((st.doc.id, target))
            })
        } else {
            None
        };
        let expand_new_fx = crate::commands::layer_panel_option(self, "expandNewEffects", true);
        self.coalesce_request = params.get("coalesce").and_then(Value::as_str).map(str::to_string);
        self.color_restrict = crate::channel_cmds::color_restriction(self, id, &run_params);
        self.filter_mask_layer = crate::channel_cmds::filter_mask_layer(self, id, &run_params);
        self.jobs.spawn = background;
        // Commands this one runs on its own behalf journal after this mark (#3067).
        let nested_from = self.journal.len();
        let pre_selected = if self.actions.recording.is_some() { self.active().map(|st| st.selected_layers()) } else { None };
        let mut record_before = crate::actions_record::Before::capture(self);
        // A replayable command is recorded as one step, so recording pauses while it runs: an
        // action it plays (a script step) must not flush the steps it ran so far into the action
        // being recorded, nor move the recording mark past the entries it is about to replace.
        let recording = if spec.journal && crate::actions_cmds::replayable(id) { self.actions.recording.take() } else { None };
        let before = self.active().map(|st| (st.doc.clone(), st.revision));
        let docs_before: Vec<_> = self.docs.iter().map(|st| st.doc.id).collect();
        // Last-resort guard (AGENTS.md, Never crash): a command that panics anyway fails with an
        // error instead of taking the app down. `edit` only commits a document after its closure
        // returns, so the documents are unchanged.
        self.live_commit = live_commit.filter(|live| live.cmd == id);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, &run_params)))
            .unwrap_or_else(|_| Err(EngineError::Other(format!("`{id}` failed with an internal error (logged); the document is unchanged"))));
        self.live_commit = None;
        if self.actions.recording.is_none() {
            self.actions.recording = recording;
        }
        self.jobs.spawn = false;
        self.coalesce_request = None;
        self.color_restrict = None;
        self.filter_mask_layer = None;
        #[cfg(any(test, debug_assertions))]
        {
            self.hint_before = None;
        }
        let pending = self.jobs.pending.take();
        if pending.is_none()
            && let Some((doc, revision)) = &before
        {
            self.note_command_damage(id, doc, *revision);
        }
        // Shape Blur by an installed shape's id or name records the geometry it ran with, as the
        // filter's own prepared params carry it: the journal (Last Filter, recorded actions)
        // replays where that shape is not installed. The run resolved it already, so it can't fail.
        let mut params = params;
        if r.is_ok() {
            let _ = crate::filters_ext::resolve_shape_path(self, id, &mut params);
        }
        let out = match (r, pending) {
            (Err(e), Some(job)) => {
                job.ctx.cancel();
                Err(e)
            }
            (Err(e), None) => Err(self.explain_missing_layer(e)),
            (Ok(_), Some(mut job)) => {
                job.command = id.to_string();
                job.params = params;
                job.journal = spec.journal;
                job.started_in_take = self.actions.recording.map(|_| self.actions.take);
                job.recorded_by = job.started_in_take.filter(|_| self.actions.record_tools || !crate::actions_cmds::tool_stroke(id));
                job.automation = automation;
                job.pre_selected = pre_selected.clone();
                job.before = std::mem::take(&mut record_before);
                let jid = job.id;
                self.jobs.running.push(job);
                Ok(Started::Job(jid))
            }
            (Ok(v), None) => {
                if let Some((before, _)) = &before {
                    self.add_relayout_damage(before);
                }
                if let Some((doc_id, layer_id)) = new_fx_target
                    && let Some(st) = self.active_mut()
                    && st.doc.id == doc_id
                    && st.doc.layer(layer_id).is_some_and(|l| !l.effects.items.is_empty())
                {
                    if expand_new_fx {
                        st.fx_collapsed.retain(|id| *id != layer_id);
                    } else if !st.fx_collapsed.contains(&layer_id) {
                        st.fx_collapsed.push(layer_id);
                    }
                }
                self.after_command(id, params, spec.journal, automation, Some(nested_from), pre_selected.as_deref(), &record_before);
                Ok(Started::Done(v))
            }
        };
        // A closed document's freed buffers went to the scratch pool (`before` and the action
        // recording snapshot kept it alive until here). Drop both before handing the pool back
        // to the OS (#10479). Compare ids since Split/Merge Channels open replacement documents.
        if docs_before.iter().any(|id| !self.docs.iter().any(|st| st.doc.id == *id)) {
            drop(before);
            drop(record_before);
            photocraft_raster::scratch::release();
        }
        out
    }

    /// The one place a command's redraw area spans more than one revision. A command that moved
    /// the active document's revision more than once (Paste Into, Stroke Path, a compound, a
    /// script) reports what changed since it began, [`layer_multi_cmds::step_damage`] of the
    /// document before and after, measured from the revision before it. A single-revision command
    /// keeps the damage it reported. Debug and test builds also count a whole-document redraw of
    /// a small change ([`Session::whole_redraws`]).
    pub(crate) fn note_command_damage(&mut self, id: &str, before: &Arc<Document>, from: u64) {
        let Some(st) = self.active_mut().filter(|st| st.doc.id == before.id && st.revision > from) else { return };
        if st.revision - from > 1 {
            st.last_damage = crate::layer_multi_cmds::step_damage(before, &st.doc);
            st.damage_span = Some((from, st.revision));
        }
        #[cfg(any(test, debug_assertions))]
        if st.last_damage.is_none()
            && let Some(pct) = crate::layer_multi_cmds::small_change_pct(before, &st.doc)
        {
            eprintln!("warning: {id}: redraws the whole document but changed {pct}% of it");
            self.whole_redraws += 1;
        }
        #[cfg(not(any(test, debug_assertions)))]
        let _ = id;
    }

    /// Grows the damage a command reported for the pixels it changed by what effects laid out
    /// over a layer's frame repaint when it moved ([`photocraft_compose::relayout_damage`]): a
    /// stroke on a layer with a Gradient Overlay reports the stroke, but the gradient is laid out
    /// again over the whole grown layer (#4053). `before` is the active document before.
    fn add_relayout_damage(&mut self, before: &Arc<Document>) {
        let Some(st) = self.active_mut().filter(|st| st.doc.id == before.id && !Arc::ptr_eq(&st.doc, before)) else { return };
        let Some(d) = st.last_damage.filter(|d| !d.is_empty()) else { return };
        let extra = photocraft_compose::relayout_damage(before, &st.doc);
        if !extra.is_empty() {
            st.last_damage = Some(d.union(&extra));
        }
    }

    /// A command that failed on a layer id the document doesn't have says which ids it does have
    /// (#5058): ids are not positions (an opened PNG's Background is not layer 1), and an agent
    /// should not need `document.inspect` to recover.
    fn explain_missing_layer(&self, e: EngineError) -> EngineError {
        match (e, self.active()) {
            (EngineError::NoLayer(id), Some(st)) if st.doc.layer(id).is_none() => {
                EngineError::UnknownLayer { id, known: crate::commands::layer_listing(&st.doc) }
            }
            (e, _) => e,
        }
    }

    /// Bookkeeping after a command (or a job's apply) succeeded.
    ///
    /// `nested_from` is the journal length when the command started. A replayable command stands
    /// for the commands it ran itself (Straighten runs Image Rotation and Crop): their entries are
    /// dropped, so a recorded action replays the command once, not its steps as well (#3067).
    /// Edit › Transform › Again (and Again on a Copy) is not replayable and keeps the concrete
    /// steps it ran instead. `automation` says the step is automation's (its request ran under an
    /// automation gate), which marks the action being recorded.
    #[allow(clippy::too_many_arguments)]
    fn after_command(
        &mut self,
        id: &str,
        params: Value,
        journal: bool,
        automation: bool,
        nested_from: Option<usize>,
        pre_selected: Option<&[LayerId]>,
        before: &crate::actions_record::Before,
    ) {
        // A layer-mask view ends when another layer becomes active (#196).
        if let Some(st) = self.active_mut() {
            crate::mask_view_cmds::fix(st);
        }
        crate::edit_menu_cmds::after_command(self, id, &params);
        crate::automate_cmds::after_command(self, id);
        self.sync_preset_store();
        if !journal {
            return;
        }
        if let Some(from) = nested_from.filter(|_| crate::actions_cmds::replayable(id)) {
            self.journal.truncate(from);
        }
        let typed = crate::actions_record::typed_layer(self, id, &params);
        let params = match id {
            _ if self.actions.recording.is_none() => params,
            "layer.duplicate" => crate::artboard_cmds::recorded_duplicate(self, crate::layer_multi_cmds::recorded_step(self, id, params, pre_selected)),
            "layerComp.apply" => crate::comps_cmds::recorded_apply(self, params),
            "edit.checkSpelling" if crate::type_spell_cmds::records(&params) => crate::type_spell_cmds::recorded_step(&params),
            _ => crate::layer_multi_cmds::recorded_step(self, id, params, pre_selected),
        };
        let params = crate::actions_cmds::recorded_params(self, id, params);
        let params = crate::actions_record::portable(self, id, params, before);
        // A step a control client runs while the user records keeps automation's gate.
        crate::actions_cmds::note_step_origin(self, automation, id, &params);
        match crate::actions_record::fold_keystroke(self, id, &params, self.typed_layer) {
            crate::actions_record::Keystroke::Folded => {}
            crate::actions_record::Keystroke::None => {
                if !crate::brush_cmds::coalesce_journal(self, id, &params) {
                    self.journal.push((id.to_string(), params));
                }
            }
        }
        self.typed_layer = typed;
    }

    /// Apply (or report the failure of) a job whose worker has delivered `r`.
    fn finish_job(&mut self, job: Running, r: Result<Payload>) {
        let before = self.active_layer_stamp();
        self.finish_job_step(job, r);
        self.note_layer_change(before);
    }

    fn finish_job_step(&mut self, mut job: Running, r: Result<Payload>) {
        let apply = job.apply.take();
        // Keep the typed error beside its display text, so `wait_job` and `JobEvent` can still
        // report a hard budget's required/limit or an approval request's limits.
        let (outcome, error) = match r {
            Err(EngineError::Cancelled) => (JobOutcome::Cancelled, None),
            Err(e) => (JobOutcome::Failed(e.to_string()), Some(e)),
            Ok(_) if job.ctx.cancelled() && job.command != "file.automate.batch" => (JobOutcome::Cancelled, None),
            Ok(payload) => match self.apply_job(&job, apply, payload) {
                Ok(mut v) => {
                    // A batch may take its final cancellation snapshot before the worker
                    // finishes. A cancellation accepted during that interval still counts.
                    if job.command == "file.automate.batch"
                        && job.ctx.cancelled()
                        && let Some(result) = v.as_object_mut()
                    {
                        result.insert("cancelled".into(), Value::Bool(true));
                    }
                    (JobOutcome::Done(v), None)
                }
                Err(EngineError::Cancelled) => (JobOutcome::Cancelled, None),
                Err(e) => (JobOutcome::Failed(e.to_string()), Some(e)),
            },
        };
        self.end_job_with_error(&job, outcome, error);
    }

    /// Apply a finished job's `payload` on this thread and do the command's bookkeeping, journaling
    /// its step into the recording that owns it with the origin the job was admitted with.
    fn apply_job(&mut self, job: &Running, apply: Option<Apply>, payload: Payload) -> Result<Value> {
        let Running {
            label,
            doc,
            snapshot,
            coalesce,
            color_restrict,
            filter_mask_layer,
            layers,
            command,
            params,
            journal,
            recorded_by,
            automation,
            pre_selected,
            before,
            ..
        } = job;
        // A recording started after the job (#7289) keeps what it has so far, and the job's step
        // goes into the journal past its start, so that recording doesn't get it.
        let foreign = self.actions.recording.is_some() && *recorded_by != Some(self.actions.take);
        if foreign {
            crate::actions_cmds::flush_recording(self)?;
        }
        let prev_active = self.active;
        let target = match doc {
            // A job past its commit has done its work outside the session (a save wrote its
            // file); a closed document doesn't undo that, so its result still applies, to no
            // document.
            Some(d) if job.ctx.committed() && !self.documents().iter().any(|st| st.doc.id == *d) => None,
            Some(d) => {
                let i =
                    self.documents().iter().position(|st| st.doc.id == *d).ok_or_else(|| EngineError::Other(format!("`{label}`: the document was closed")))?;
                let unchanged = snapshot.as_ref().is_some_and(|w| self.documents().get(i).is_some_and(|st| Weak::ptr_eq(w, &Arc::downgrade(&st.doc))));
                if !unchanged {
                    return Err(EngineError::Other(format!("`{label}`: the document changed while it ran; the result was discarded")));
                }
                self.active = Some(i);
                self.note_active_change();
                Some(i)
            }
            None => None,
        };
        // Selecting a layer doesn't change the document, so the snapshot check above lets it
        // through. Apply against the layers the job started on: the colour restriction and any
        // `active`-based edit land on them (#7366). The user's selection is put back below.
        let user_layers = match (target, layers) {
            (Some(i), Some(started)) => self.docs.get_mut(i).and_then(|st| {
                let now = LayerSelection::of(st);
                (now != *started).then(|| {
                    started.clone().set(st);
                    now
                })
            }),
            _ => None,
        };
        self.coalesce_request = coalesce.clone();
        self.color_restrict = *color_restrict;
        self.filter_mask_layer = *filter_mask_layer;
        // Apply-time hooks can dispatch commands too. Keep the same single-step recording
        // contract as synchronous dispatch without truncating work done while the worker ran.
        let nested_from = self.journal.len();
        let recording = if *journal && crate::actions_cmds::replayable(command) { self.actions.recording.take() } else { None };
        let r = match apply {
            Some(f) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, payload)))
                .unwrap_or_else(|_| Err(EngineError::Other(format!("`{label}` failed with an internal error (logged); the document is unchanged")))),
            None => Err(EngineError::Other("internal error: job applied twice".into())),
        };
        if self.actions.recording.is_none() {
            self.actions.recording = recording;
        }
        self.coalesce_request = None;
        self.color_restrict = None;
        self.filter_mask_layer = None;
        // Finish successful command bookkeeping on the document the job edited.
        // Restoring the viewed document first would attribute Fade and slice updates
        // to that document instead, when the user switched tabs mid-job.
        if r.is_ok() {
            // The step is automation's when the job's own request was gated, not the caller
            // polling it now, and only for the recording that owns it: a foreign recording does
            // not get the step.
            self.after_command(command, params.clone(), *journal, *automation && !foreign, Some(nested_from), pre_selected.as_deref(), before);
            if foreign && let Some((start, _)) = self.actions.recording.as_mut() {
                *start = self.journal.len();
            }
        }
        // Give the user back the layers they selected while the job ran, unless the job chose a
        // selection itself (a new layer, say) or one of their layers is gone.
        if let (Some(started), Some(user), Some(st)) = (layers, user_layers, target.and_then(|i| self.docs.get_mut(i))) {
            let kept = LayerSelection::of(st) == *started;
            let exist = user.active.iter().chain(&user.selected).all(|id| st.doc.layer(*id).is_some());
            if (r.is_err() || kept) && exist {
                user.set(st);
            }
        }
        // Keep the user's active document unless the job opened a new one.
        // This also restores the selection when the apply step returned an error.
        if target.is_some() && self.active == target {
            self.active = prev_active.filter(|i| *i < self.docs.len()).or(self.active);
            self.note_active_change();
        }
        r
    }

    fn end_job(&mut self, job: &Running, outcome: JobOutcome) {
        self.end_job_with_error(job, outcome, None);
    }

    /// Records a finished job's terminal state (done, failed or cancelled) and optional typed
    /// error in the recent-job list, invalidating any live preview.
    fn end_job_with_error(&mut self, job: &Running, outcome: JobOutcome, error_kind: Option<EngineError>) {
        // Every terminal path invalidates a preview, including worker errors, cancellation
        // and discarded results whose document was closed or changed.
        self.live_commit = None;
        let (state, result, error) = match &outcome {
            JobOutcome::Done(v) if job.command == "file.automate.batch" && v.get("cancelled").and_then(Value::as_bool) == Some(true) => {
                ("cancelled", Some(v.clone()), None)
            }
            JobOutcome::Done(v) => ("done", Some(v.clone()), None),
            JobOutcome::Failed(e) => ("failed", None, Some(e.clone())),
            JobOutcome::Cancelled => ("cancelled", None, None),
        };
        let mut info = job.info();
        info.state = state;
        info.result = result;
        info.error = error;
        info.error_kind = error_kind.clone();
        if state == "done" {
            info.progress = 1.0;
        }
        self.jobs.recent.push(info);
        if self.jobs.recent.len() > RECENT {
            let extra = self.jobs.recent.len() - RECENT;
            self.jobs.recent.drain(..extra);
        }
        self.jobs.events.push(JobEvent { id: job.id, command: job.command.clone(), label: job.label.clone(), document: job.doc, outcome, error: error_kind });
    }
}

/// Engine commands for the job table (the control channel and MCP use them too).
pub fn specs() -> Vec<crate::commands::CommandSpec> {
    use crate::commands::CommandSpec;
    vec![
        CommandSpec {
            id: "jobs.list",
            label: "List Background Jobs",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: |s, _| Ok(json!({"jobs": s.jobs_with_recent()})),
            journal: false,
        },
        CommandSpec {
            id: "jobs.cancel",
            label: "Cancel Background Job",
            menu: &[],
            shortcut: None,
            params: r##"{"job":u64? (default: every running job)}"##,
            enabled: |_| Ok(()),
            run: |s, p| match p.get("job") {
                None | Some(Value::Null) => Ok(s.cancel_all_jobs().to_json()),
                Some(v) => {
                    let id = v.as_u64().ok_or_else(|| EngineError::BadParams { cmd: "jobs.cancel".into(), msg: "`job` must be a job id".into() })?;
                    match s.try_cancel_job(JobId(id)) {
                        Ok(()) => Ok(json!({"cancelled": 1})),
                        // Not an error: the job is fine and will report how it ended.
                        Err(why @ (CancelRefused::Committing | CancelRefused::Finishing)) => {
                            Ok(CancelReport { cancelled: Vec::new(), refused: vec![(JobId(id), why)] }.to_json())
                        }
                        Err(CancelRefused::Printing) => Err(EngineError::Other("printing cannot be cancelled here; use the system print queue".into())),
                        Err(CancelRefused::NotRunning) => Err(EngineError::Other(format!("no running job {id}"))),
                    }
                }
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../io/tests/support/recovery_psd.rs"]
pub(crate) mod psd_open_fixture;

#[cfg(test)]
mod psd_open_confirmation_tests {
    use super::*;

    /// Collects either an inline open result or the completed background-job result.
    fn result(s: &mut Session, started: Started) -> Value {
        match started {
            Started::Done(v) => v,
            Started::Job(id) => s.wait_job(id).unwrap(),
        }
    }

    /// Verifies headless opens return readable PSD content and warnings without a held prompt.
    #[test]
    fn headless_open_keeps_best_effort_psd_import() {
        let mut s = Session::new();
        let started = s.start_open("damaged.psd", OpenSource::Bytes(Arc::new(super::psd_open_fixture::make_psd(true)))).unwrap();
        let v = result(&mut s, started);
        assert!(v.get("document").is_some(), "{v}");
        assert!(v.get("held").is_none(), "no new prompt for a headless caller");
        assert!(!s.active().unwrap().source_read_only);
        assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("could not be decoded")));
    }

    /// Verifies interactive opens hold damaged PSD files but admit valid files immediately.
    #[test]
    fn interactive_open_holds_only_a_damaged_psd() {
        for corrupt in [false, true] {
            let mut s = Session::new();
            let started = s.start_open_for_user("file.psd", OpenSource::Bytes(Arc::new(super::psd_open_fixture::make_psd(corrupt)))).unwrap();
            let v = result(&mut s, started);
            if corrupt {
                assert_eq!(v["damaged"], true);
                assert!(s.documents().is_empty());
                let held = v["held"].as_u64().unwrap();
                let (doc, read_only) = s.take_held_open(held).unwrap();
                assert_eq!(doc.layers.len(), 2);
                assert!(!read_only);
            } else {
                assert!(v.get("document").is_some(), "{v}");
                assert!(!s.active().unwrap().source_read_only);
            }
        }
    }
}
