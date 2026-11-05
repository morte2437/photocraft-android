//! Progress and cancellation for calls that wait. Render Video runs on the existing blocking
//! worker; other long commands run as engine jobs that the session polls ([`Waiter`]).
use super::{Arc, AutomationError, Backend, CallToolResult, Headless, Mutex, PhotocraftMcp, PoisonError, Value, fail, no_backend, ok_json};
use crate::headless::JobPoll;
use photocraft_engine::jobs::{JobId, Started};
use rmcp::RoleServer;
use rmcp::model::ProgressNotificationParam;
use rmcp::service::RequestContext;
use std::time::{Duration, Instant};

impl PhotocraftMcp {
    pub(super) async fn render_video_progress(&self, params: Value, context: RequestContext<RoleServer>) -> CallToolResult {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // rmcp may drop the request future on cancellation. The blocking worker owns the
        // token too, so it still stops and cleans up without needing that future to be polled.
        let cancel = context.ct.clone();
        let job = self.headless_op(move |h| {
            h.render_video(params, move |done, total| {
                let _ = tx.send((done, total));
                !cancel.is_cancelled()
            })
        });
        tokio::pin!(job);
        let token = context.meta.get_progress_token();
        let (mut last, mut sent_at): (Option<usize>, Option<Instant>) = (None, None);
        let result = loop {
            tokio::select! {
                biased;
                Some((done, total)) = rx.recv() => {
                    let due = done == total || sent_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
                    if let Some(token) = token.clone().filter(|_| due && last.is_none_or(|n| done > n) && !context.ct.is_cancelled()) {
                        last = Some(done);
                        sent_at = Some(Instant::now());
                        let _ = context.peer.notify_progress(ProgressNotificationParam::new(token, done as f64).with_total(total as f64)).await;
                    }
                }
                result = &mut job => break result,
            }
        };
        match result {
            Some(Ok(value)) => ok_json(&value),
            Some(Err(error)) => fail(error),
            None => no_backend(),
        }
    }
}

/// Where a waited call reports progress and learns that it was cancelled. Without a request
/// (a direct call in a test) nothing is reported and nothing cancels.
pub(super) struct Waiter {
    context: Option<RequestContext<RoleServer>>,
    last: Option<f64>,
    sent_at: Option<Instant>,
}

impl Waiter {
    pub(super) fn new(context: Option<RequestContext<RoleServer>>) -> Self {
        Waiter { context, last: None, sent_at: None }
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.ct.is_cancelled())
    }

    async fn cancelled(&self) {
        match &self.context {
            Some(c) => c.ct.cancelled().await,
            None => std::future::pending().await,
        }
    }

    /// Send `progress` of `total` when the request has a progress token. At most every 100ms,
    /// and only when it grew (MCP requires progress to increase).
    pub(super) async fn report(&mut self, progress: f64, total: f64, message: &str) {
        let Some(context) = &self.context else { return };
        let Some(token) = context.meta.get_progress_token() else { return };
        let due = self.sent_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(100));
        if !due || !progress.is_finite() || self.last.is_some_and(|n| progress <= n) || context.ct.is_cancelled() {
            return;
        }
        self.last = Some(progress);
        self.sent_at = Some(Instant::now());
        let mut note = ProgressNotificationParam::new(token, progress).with_total(total);
        if !message.is_empty() {
            note = note.with_message(message);
        }
        let _ = context.peer.notify_progress(note).await;
    }
}

/// Cancels a waited job when its waiter goes away without collecting it: rmcp may drop the
/// request future on cancellation, and nothing else would stop the job.
struct Abandon {
    session: Arc<Mutex<Headless>>,
    job: Option<JobId>,
}

impl Drop for Abandon {
    fn drop(&mut self) {
        let Some(job) = self.job.take() else { return };
        let session = self.session.clone();
        let abandon = move || drop(session.lock().unwrap_or_else(PoisonError::into_inner).job_abandon(job));
        match tokio::runtime::Handle::try_current() {
            // A synchronous command can hold the lock for a while: don't block a runtime thread.
            Ok(runtime) => drop(runtime.spawn_blocking(abandon)),
            Err(_) => abandon(),
        }
    }
}

impl PhotocraftMcp {
    /// Headless: run command `id` for a caller that waits (#8129). A job-capable command runs as
    /// a background job, so the session serves other calls meanwhile; its progress goes to
    /// `waiter` as `step.0 + fraction` of `step.1`, and cancelling the request cancels the job,
    /// leaving the document unchanged. `None` in bridge mode.
    pub(super) async fn headless_wait(&self, id: String, params: Value, waiter: &mut Waiter, step: (usize, usize)) -> Option<Result<Value, AutomationError>> {
        let Backend::Headless(session) = &*self.backend else { return None };
        let session = session.clone();
        let cancel = waiter.context.as_ref().map(|context| context.ct.clone());
        // The guard is armed under the lock that starts the job: when the request is dropped while
        // the start still waits for the lock, the closure's output is dropped unread and cancels it.
        let mut guard = match self
            .headless_op(move |h| {
                if cancel.as_ref().is_some_and(|token| token.is_cancelled()) {
                    return Err(photocraft_engine::EngineError::Cancelled.into());
                }
                Ok(match h.command_begin(&id, params)? {
                    Started::Job(job) if cancel.as_ref().is_some_and(|token| token.is_cancelled()) => {
                        return h.job_abandon(job).map(Err);
                    }
                    Started::Job(job) => Ok(Abandon { session, job: Some(job) }),
                    Started::Done(value) => Err(value),
                })
            })
            .await?
        {
            Ok(Ok(guard)) => guard,
            Ok(Err(value)) => return Some(Ok(value)),
            Err(error) => return Some(Err(error)),
        };
        let Some(job) = guard.job else { return Some(Err(AutomationError::Other("internal error: a started job has no id".into()))) };
        let mut delay = Duration::from_millis(1);
        loop {
            match self.headless_op(move |h| Ok(h.job_poll(job))).await? {
                Ok(JobPoll::Running(fraction, message)) => {
                    let fraction = if fraction.is_finite() { f64::from(fraction.clamp(0.0, 1.0)) } else { 0.0 };
                    waiter.report(step.0 as f64 + fraction, step.1 as f64, &message).await;
                }
                Ok(JobPoll::Ended(result)) => {
                    guard.job = None;
                    return Some(result);
                }
                Err(error) => return Some(Err(error)),
            }
            tokio::select! {
                biased;
                () = waiter.cancelled() => {
                    // Cancel before answering, so the next call already sees the document unchanged.
                    // A job that ended first is answered with its result, not as cancelled.
                    guard.job = None;
                    return Some(self.headless_op(move |h| Ok(h.job_abandon(job))).await?.and_then(|result| result));
                }
                () = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(Duration::from_millis(50));
        }
    }
}
