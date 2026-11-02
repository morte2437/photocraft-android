use super::{PhotocraftApp, SyntheticTarget};

/// Remove one contiguous run equal to `batch` from `events` (eframe's preserved synthetic
/// input). A no-op when the run is not present.
fn remove_event_run(events: &mut Vec<egui::Event>, batch: &[egui::Event]) {
    if batch.is_empty() || batch.len() > events.len() {
        return;
    }
    if let Some(pos) = events.windows(batch.len()).position(|w| w == batch) {
        events.drain(pos..pos + batch.len());
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_dropped(f: &(dyn egui::DroppedFile + Send + Sync)) -> Result<Vec<u8>, String> {
    f.bytes()
}

/// On the web, dropped-file bytes arrive asynchronously; the web shell handles drops itself.
#[cfg(target_arch = "wasm32")]
pub(crate) fn read_dropped(_f: &dyn egui::DroppedFile) -> Result<Vec<u8>, String> {
    Err("drag-and-drop on the web is handled by the page; use File → Open".into())
}

impl PhotocraftApp {
    /// The document/focus identity the in-flight synthetic batch was queued for.
    fn synthetic_target(&self, ctx: &egui::Context) -> SyntheticTarget {
        SyntheticTarget { document: self.document_identity(), focus: ctx.memory(|m| m.focused()) }
    }

    fn synthetic_target_matches(&self, ctx: &egui::Context) -> bool {
        self.synthetic_snapshot.is_some_and(|queued| queued.matches(self.synthetic_target(ctx)))
    }

    /// Put at most one queued synthetic batch into this pass's input.
    ///
    /// The batch is appended once and tracked; `begin_pass` consumes it. While it is in flight no
    /// further step is drained, so hidden logic ticks cannot accumulate several batches into one
    /// pass. A document or focus change observed here is before eframe consumes the events, so the
    /// batch is dropped and its waiting control replies fail instead of landing somewhere else.
    /// If a pass consumed it but PhotoCraft returned before handling input (fonts not ready), the
    /// same batch is appended again on the next hook, exactly once.
    pub(crate) fn inject_synthetic(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.synthetic_batch.is_some() {
            if !self.synthetic_target_matches(ctx) {
                // eframe may already carry the batch in its preserved input: withdraw exactly
                // those events before the host consumes them, then fail the waiting replies.
                if let Some(batch) = self.synthetic_batch.as_ref() {
                    remove_event_run(&mut raw_input.events, batch);
                }
                self.cancel_synthetic("the active document or focus changed while the input was queued");
                return;
            }
            if self.synthetic_reinject {
                if let Some(batch) = self.synthetic_batch.clone() {
                    raw_input.events.extend(batch);
                }
                self.synthetic_reinject = false;
                self.automation_input = true;
            }
            return;
        }
        if self.synthetic.is_empty() {
            return;
        }
        let events = self.take_synthetic_step();
        if events.is_empty() {
            return;
        }
        self.synthetic_snapshot = Some(self.synthetic_target(ctx));
        raw_input.events.extend(events.iter().cloned());
        self.synthetic_batch = Some(events);
    }

    /// A pass begins with the in-flight batch. The events are already in this pass's input, so a
    /// target change detected now cannot withdraw them: deliver them (never half-apply) and keep
    /// `automation_input` set so the commands they trigger still go through the automation policy.
    /// `inject_synthetic` performs the withdrawable check, before the host consumes the events.
    /// Called at the start of `App::ui` only (a hidden `run_logic` tick consumes no input).
    pub(crate) fn begin_synthetic_pass(&mut self, ctx: &egui::Context) {
        if self.synthetic_batch.is_none() {
            return;
        }
        if !self.synthetic_target_matches(ctx) {
            log::warn!("synthetic input: the document or focus changed inside the pass; delivering it as automation input");
        }
        self.automation_input = true;
    }

    /// One full UI pass handled the in-flight batch: deliver it once and let the next step in.
    pub(crate) fn finish_synthetic_pass(&mut self) {
        let _ = self.synthetic_batch.take();
        self.synthetic_snapshot = None;
        self.synthetic_reinject = false;
    }

    /// The pass returned before the input handling (fonts not ready): keep the batch and have the
    /// next hook append it again exactly once, because egui already consumed the events.
    pub(crate) fn defer_synthetic_pass(&mut self) {
        if self.synthetic_batch.is_some() {
            self.synthetic_reinject = true;
        }
    }

    /// Drop the in-flight batch and fail the replies waiting on it.
    fn cancel_synthetic(&mut self, reason: &str) {
        let pending = self.synthetic_batch.take().is_some() || !self.synthetic.is_empty();
        self.synthetic.clear();
        self.synthetic_snapshot = None;
        self.synthetic_reinject = false;
        self.automation_input = false;
        if pending || !self.input_waiters.is_empty() {
            for w in self.input_waiters.drain(..) {
                let _ = w.send(serde_json::json!({"ok": false, "error": format!("input cancelled: {reason}")}));
            }
        }
    }

    /// Next step of queued synthetic input (automation): events up to and including the first
    /// release, so egui sees press and release in separate frames. Hosts that don't call
    /// `raw_input_hook` (offscreen harnesses) feed these to their input themselves.
    pub fn take_synthetic_step(&mut self) -> Vec<egui::Event> {
        let n = self
            .synthetic
            .iter()
            .position(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. } | egui::Event::Key { pressed: false, .. }))
            .map_or(self.synthetic.len(), |i| i + 1);
        let events: Vec<_> = self.synthetic.drain(..n).collect();
        if !events.is_empty() {
            self.automation_input = true;
        }
        events
    }
}
