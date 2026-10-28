use super::PhotocraftApp;

impl PhotocraftApp {
    /// Issue due screenshot requests. UI phase only: settling counts real layout passes, and the
    /// geometry epoch is that of the pass the image will be rendered from.
    pub(super) fn issue_captures(&mut self, ctx: &egui::Context) {
        // A discarded pass (popup sizing, `request_discard`) will be re-run: capture only a pass
        // whose UI state will actually be present.
        if ctx.will_discard() {
            return;
        }
        let now = crate::screenshot::now();
        let doc = self.document_identity();
        let finished = self.captures.issue(ctx, now, doc, |callback| crate::screenshot::send_command(ctx, callback));
        self.finish_captures(finished);
    }

    /// Collect callbacks and expire requests. Logic phase; the image is encoded and written here,
    /// on the UI thread, never inside the callback.
    pub(super) fn drain_captures(&mut self, ctx: &egui::Context) {
        let now = crate::screenshot::now();
        let doc = self.document_identity();
        // begin_pass supplies the new RawInput before App::logic; run_logic refreshes native
        // viewport metadata too. Observe it before any completed callback can be delivered.
        let current = crate::screenshot::CaptureGeometry::current(ctx, doc);
        self.captures.note_geometry(current);
        let observed = self.captures.last_geometry();
        let finished = self.captures.drain(now, observed, doc);
        self.finish_captures(finished);
    }

    pub(super) fn finish_captures(&mut self, finished: Vec<crate::screenshot::Finished>) {
        for f in finished {
            let reply = match f.result {
                Ok(image) => {
                    let [w, h] = image.size;
                    // The encoder takes RGBA bytes; its copy is bounded by the same per-image
                    // reservation as the retained image, and the image itself is released first.
                    match w.checked_mul(h).and_then(|n| n.checked_mul(4)) {
                        Some(bytes) if bytes <= crate::screenshot::MAX_IMAGE_BYTES => {
                            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                            drop(image);
                            crate::control::save_screenshot_rgba(self, w, h, &rgba, f.path.as_deref())
                        }
                        _ => serde_json::json!({
                            "ok": false,
                            "error": format!(
                                "screenshot {} failed: the image exceeds the {} MiB encode budget",
                                f.token,
                                crate::screenshot::MAX_IMAGE_BYTES / (1 << 20)
                            ),
                        }),
                    }
                }
                Err(e) => serde_json::json!({"ok": false, "error": e}),
            };
            if let Some(tx) = f.reply {
                let _ = tx.send(reply);
            }
        }
    }
}
