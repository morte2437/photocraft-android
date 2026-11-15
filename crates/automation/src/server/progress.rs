//! Progress and cancellation for calls that wait. Render Video runs on the existing blocking
//! worker; other long commands run as engine jobs that the server polls ([`Waiter`]): the headless
//! session's own, or the app's over the bridge.
use super::{Arc, AutomationError, BridgeClient, CallToolResult, PhotocraftMcp, PoisonError, Value, fail, json, no_backend, ok_json};
use crate::headless::JobPoll;
use crate::live::Target;
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
    /// The session or app that started the job: in live mode the current target can change
    /// before the guard drops, and the job id means nothing elsewhere.
    target: Target,
    job: Option<JobId>,
}

impl Drop for Abandon {
    fn drop(&mut self) {
        let Some(job) = self.job.take() else { return };
        let runtime = tokio::runtime::Handle::try_current();
        match &self.target {
            Target::Headless(session) => {
                let session = session.clone();
                let abandon = move || drop(session.lock().unwrap_or_else(PoisonError::into_inner).job_abandon(job));
                match runtime {
                    // A synchronous command can hold the lock for a while: don't block a runtime thread.
                    Ok(runtime) => drop(runtime.spawn_blocking(abandon)),
                    Err(_) => abandon(),
                }
            }
            // Without a runtime there is no way to reach the app; the job then runs to the end.
            Target::Bridge(client) => {
                if let Ok(runtime) = runtime {
                    let client = client.clone();
                    drop(runtime.spawn(async move { client.call("jobs.cancel", json!({"job": job.0})).await }));
                }
            }
        }
    }
}

impl PhotocraftMcp {
    /// Headless: run command `id` for a caller that waits (#8129). A job-capable command runs as
    /// a background job, so the session serves other calls meanwhile; its progress goes to
    /// `waiter` as `step.0 + fraction` of `step.1`, and cancelling the request cancels the job,
    /// leaving the document unchanged. `None` in bridge mode.
    pub(super) async fn headless_wait(&self, id: String, params: Value, waiter: &mut Waiter, step: (usize, usize)) -> Option<Result<Value, AutomationError>> {
        let target = Target::Headless(self.headless_session()?);
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
                    Started::Job(job) => Ok(Abandon { target, job: Some(job) }),
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
    /// Bridge: run command `id` for a caller that waits (#8130). An engine command starts with
    /// `wait: false`; when the app runs it as a background job, `jobs.list` is polled until the job
    /// ends, so no single reply has to arrive within the client's call timeout. Progress and
    /// cancellation as in [`Self::headless_wait`]. Only the start is an edit, and it is sent once
    /// (#1007); a failed poll reports the job instead of guessing its outcome.
    pub(super) async fn bridge_wait(
        &self,
        client: &Arc<BridgeClient>,
        id: String,
        params: Value,
        waiter: &mut Waiter,
        step: (usize, usize),
    ) -> Result<Value, AutomationError> {
        // A UI command can start a job without naming it in its reply: the app waits on those.
        if photocraft_engine::commands::find(&id).is_none() {
            return client.call("engine.execute", json!({"command": id, "params": params, "wait": true})).await;
        }
        let started = client.call("engine.execute", json!({"command": id, "params": params, "wait": false})).await?;
        let pending = started.get("pending").and_then(Value::as_bool) == Some(true);
        let Some(job) = started.get("job").and_then(Value::as_u64).filter(|_| pending) else {
            return Ok(started);
        };
        let mut guard = Abandon { target: Target::Bridge(client.clone()), job: Some(JobId(job)) };
        let mut delay = Duration::from_millis(10);
        loop {
            // Race the poll against a cancel: an app slow to answer must not hold it until the call times out.
            let polled = tokio::select! {
                biased;
                listed = client.call_read("jobs.list", json!({})) => Some(listed),
                () = waiter.cancelled() => None,
            };
            let listed = match polled {
                Some(Ok(listed)) => listed,
                Some(Err(error)) if !waiter.is_cancelled() => {
                    // The app may be fine and the job still running: leave it, and say how to find it.
                    guard.job = None;
                    return Err(AutomationError::Bridge(format!(
                        "lost track of job {job} ({id}): {error}; it may still be running and apply its result: check jobs_list, or stop it with jobs_cancel"
                    )));
                }
                // Cancelled during the poll, which may also have failed: the app may answer a
                // fresh connection.
                _ => return Err(bridge_cancel(client, &mut guard, job, &id).await),
            };
            let entry = listed.get("jobs").and_then(Value::as_array).and_then(|jobs| jobs.iter().find(|j| j.get("id").and_then(Value::as_u64) == Some(job)));
            let Some(entry) = entry else {
                guard.job = None;
                return Err(AutomationError::Bridge(format!(
                    "job {job} ({id}) ended but jobs.list no longer shows it (it keeps the last few); inspect the document for its result"
                )));
            };
            let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            match entry.get("state").and_then(Value::as_str) {
                Some("running") => {
                    let fraction = entry.get("progress").and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(0.0, |f| f.clamp(0.0, 1.0));
                    waiter.report(step.0 as f64 + fraction, step.1 as f64, &text("message")).await;
                }
                state => {
                    guard.job = None;
                    // The same results the app's own waited reply gives.
                    return match state {
                        Some("done") => Ok(entry.get("result").cloned().unwrap_or(Value::Null)),
                        Some("failed") => Err(AutomationError::App(Some(text("error")).filter(|e| !e.is_empty()).unwrap_or_else(|| "unknown error".into()))),
                        Some("cancelled") => Err(AutomationError::App("cancelled".into())),
                        _ => Err(AutomationError::Bridge(format!("job {job} ({id}) has an unknown state: {entry}"))),
                    };
                }
            }
            tokio::select! {
                biased;
                () = waiter.cancelled() => return Err(bridge_cancel(client, &mut guard, job, &id).await),
                () = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(Duration::from_millis(200));
        }
    }
}

/// Cancel bridged `job` (`id`) before answering a cancelled wait. Failed calls leave the guard
/// armed for one best-effort cleanup attempt on drop.
async fn bridge_cancel(client: &BridgeClient, guard: &mut Abandon, job: u64, id: &str) -> AutomationError {
    let cancel = client.call("jobs.cancel", json!({"job": job})).await;
    // A failed or abandoned call keeps the guard armed for one cleanup attempt on drop.
    // Only an acknowledged cancellation proves that cleanup is no longer needed.
    // A definite refusal is not improved by retrying; malformed replies and transport
    // failures leave the guard armed because the outcome is uncertain.
    if cancel.as_ref().is_ok_and(|value| cancellation_acknowledged(value) || refusal(value, job).is_some()) {
        guard.job = None;
    }
    cancel_answer(job, id, cancel)
}

/// The answer to a bridged wait that was cancelled, from the app's reply to `jobs.cancel`. The app
/// refuses to cancel a job that already ended, whose result may then be applied: say so rather
/// than report a cancellation.
fn cancel_answer(job: u64, id: &str, reply: Result<Value, AutomationError>) -> AutomationError {
    match reply {
        Ok(value) if refusal(&value, job).is_some() => {
            let reason = refusal(&value, job).unwrap_or("unknown reason");
            AutomationError::Bridge(format!("job {job} ({id}) was not cancelled: {reason}; check jobs_list for its outcome"))
        }
        Ok(value) if cancellation_acknowledged(&value) => photocraft_engine::EngineError::Cancelled.into(),
        Ok(value) => AutomationError::Bridge(format!("cancellation of job {job} ({id}) was not confirmed: {value}; check jobs_list for its outcome")),
        Err(error) => AutomationError::Bridge(format!("job {job} ({id}) cancellation was not confirmed: {error}; check jobs_list for its outcome")),
    }
}

fn cancellation_acknowledged(value: &Value) -> bool {
    value.get("cancelled").and_then(Value::as_u64) == Some(1)
}

fn refusal(value: &Value, job: u64) -> Option<&str> {
    value.get("notCancelled")?.as_array()?.iter().find(|entry| entry.get("job").and_then(Value::as_u64) == Some(job))?.get("reason")?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_cancel_is_not_reported_as_cancelled() {
        let cancelled = cancel_answer(7, "filter.blur.gaussianBlur", Ok(json!({"cancelled": 1})));
        assert!(matches!(cancelled, AutomationError::Engine(photocraft_engine::EngineError::Cancelled)), "{cancelled}");
        let refused = cancel_answer(7, "filter.blur.gaussianBlur", Err(AutomationError::App("job 7 is not running".into())));
        assert!(matches!(&refused, AutomationError::Bridge(m) if m.contains("not confirmed") && m.contains("job 7 is not running")), "{refused}");
        let refused = cancel_answer(7, "filter.blur.gaussianBlur", Ok(json!({"cancelled": 0, "notCancelled": [{"job": 7, "reason": "committing"}]})));
        assert!(matches!(&refused, AutomationError::Bridge(m) if m.contains("job 7") && m.contains("committing") && m.contains("jobs_list")), "{refused}");
    }
}
