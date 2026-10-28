use super::{ControlRequest, PhotocraftApp, Receiver};

impl PhotocraftApp {
    /// Attach a control channel (requests arrive from a transport thread: TCP, stdin, tests).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub(crate) fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            if req.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) || !req.dispatch.start() {
                let _ = reply.send(serde_json::json!({"ok": false, "error": crate::control::expired_error()}));
                continue;
            }
            match crate::control::handle(self, ctx, &req) {
                crate::control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                crate::control::Outcome::AfterInput => self.input_waiters.push(reply),
                crate::control::Outcome::AfterJob(job) => self.jobs.waiters.push((job, reply)),
                crate::control::Outcome::Screenshot { token, path } => {
                    // Admission is budgeted here, after this pass's completed captures were
                    // drained, so a just-finished request never causes a false rejection.
                    let settle = crate::screenshot::settle_ms(ctx.global_style().animation_time);
                    let document = self.document_identity();
                    if let Err((reply, e)) = self.captures.admit(token, path, reply, document, settle, crate::screenshot::now()) {
                        let _ = reply.send(serde_json::json!({"ok": false, "error": e}));
                    }
                }
            }
            // `ui.set` may have changed the edit target: the next request sees its colours.
            self.sync_mask_targets();
        }
        self.control_rx = Some(rx);
    }
}
