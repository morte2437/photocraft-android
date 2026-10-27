//! Lifecycle tests for `ui.screenshot` admission, issuance, settling and completion.

use super::screenshot::{CaptureGeometry, Captures, DEADLINE, DocumentId, Finished, MAX_IMAGE_BYTES, MAX_PENDING, MAX_TOTAL_BYTES, SETTLE_PASSES};
use egui::{Color32, ColorImage, Context, ViewportId, vec2};
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

fn geometry(w: f32, h: f32, doc: Option<DocumentId>) -> CaptureGeometry {
    CaptureGeometry {
        viewport: ViewportId::ROOT,
        logical_size: vec2(w, h),
        pixels_per_point: 1.0,
        zoom_factor: 1.0,
        max_texture_side: 8192,
        minimized: false,
        document: doc,
    }
}

fn doc(id: u64, revision: u64) -> Option<DocumentId> {
    doc_at(id, revision, 0)
}

fn doc_at(id: u64, revision: u64, generation: u64) -> Option<DocumentId> {
    Some(DocumentId { document: id, revision, generation })
}

fn queue(caps: &mut Captures, token: u64, document: Option<DocumentId>, now: Instant) -> std::sync::mpsc::Receiver<serde_json::Value> {
    let (tx, rx) = std::sync::mpsc::channel();
    caps.admit(token, None, tx, document, 0.0, now).expect("admitted");
    rx
}

/// Run the UI-pass settling the queue needs, returning the callbacks installed by issuance.
fn settle_and_issue(caps: &mut Captures, ctx: &Context, now: Instant, document: Option<DocumentId>) -> (Vec<Finished>, Vec<egui::ScreenshotCallback>) {
    for _ in 0..SETTLE_PASSES {
        caps.note_pass();
    }
    let mut callbacks = Vec::new();
    let finished = caps.issue(ctx, now, document, |cb| callbacks.push(cb));
    (finished, callbacks)
}

fn image(geometry: &CaptureGeometry) -> Arc<ColorImage> {
    let [w, h] = geometry.expected_pixels().expect("pixels");
    Arc::new(ColorImage::filled([w, h], Color32::BLACK))
}

#[test]
fn a_launch_and_capture_request_waits_for_the_first_ui_pass() {
    let now = Instant::now();
    let mut caps = Captures::default();
    // 0.15 queued the request; a client that connects before the first frame must not fail.
    let rx = queue(&mut caps, 1, None, now);
    assert_eq!(caps.pending(), 1, "parked, not rejected");
    assert_eq!(caps.reserved_bytes(), 0, "no geometry means no reservation yet");
    assert!(caps.any_unissued(), "logic asks for a hidden paint");
    assert!(rx.try_recv().is_err(), "no reply before the geometry is known");

    // The first UI pass records the geometry; the next logic drain admits the request under the
    // same token, count and deadline.
    let g = geometry(800.0, 600.0, None);
    caps.note_geometry(g);
    assert!(caps.drain(now, Some(g), None).is_empty());
    assert_eq!(caps.pending(), 1);
    assert_eq!(caps.reserved_bytes(), 800 * 600 * 4, "admitted with a reservation");
    let (tx, _rx) = std::sync::mpsc::channel();
    let (_, error) = caps.admit(1, None, tx, None, 0.0, now).expect_err("the token is pending");
    assert!(error.contains("already pending"), "{error}");

    let (_, callbacks) = settle_and_issue(&mut caps, &Context::default(), now, None);
    assert_eq!(callbacks.len(), 1, "then it is issued normally");
}

#[test]
fn a_request_parked_for_geometry_still_times_out() {
    let now = Instant::now();
    let mut caps = Captures::default();
    let rx = queue(&mut caps, 3, None, now);
    let finished = caps.drain(now + DEADLINE, None, None);
    assert!(finished.is_empty(), "the parked request answers itself");
    let reply = rx.recv().expect("the control request gets exactly one answer");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("timed out")), "{reply}");
    assert_eq!(caps.pending(), 0);
}

#[test]
fn admission_enforces_the_count_and_byte_budgets() {
    let now = Instant::now();
    let mut caps = Captures::default();
    // 2000×2000 at 4 bytes = 16 MiB: 16 requests would exceed 256 MiB, but only 4 are allowed.
    caps.note_geometry(geometry(2000.0, 2000.0, None));
    for token in 0..MAX_PENDING as u64 {
        queue(&mut caps, token, None, now);
    }
    let (tx, _rx) = std::sync::mpsc::channel();
    let (_, error) = caps.admit(99, None, tx, None, 0.0, now).expect_err("the queue is full");
    assert!(error.contains("too many"), "{error}");
    caps.cancel_all("test");

    // A single image over 128 MiB is rejected before issuance.
    caps.note_geometry(geometry(8000.0, 5000.0, None));
    let (tx, _rx) = std::sync::mpsc::channel();
    let (_, error) = caps.admit(100, None, tx, None, 0.0, now).expect_err("over the per-image limit");
    assert!(error.contains("per-image"), "{error}");
    const { assert!(MAX_IMAGE_BYTES < MAX_TOTAL_BYTES) };

    // Exact aggregate boundary: 4 × 64 MiB is exactly the 256 MiB limit and is admitted.
    caps.note_geometry(geometry(4096.0, 4096.0, None)); // 64 MiB
    for token in 200..204u64 {
        queue(&mut caps, token, None, now);
    }
    assert_eq!(caps.reserved_bytes(), MAX_TOTAL_BYTES);
    let (tx, _rx) = std::sync::mpsc::channel();
    assert!(caps.admit(205, None, tx, None, 0.0, now).is_err());
}

#[test]
fn a_geometry_growth_between_admission_and_issuance_is_revalidated() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    caps.note_geometry(geometry(800.0, 600.0, doc(1, 1)));
    let _rx = queue(&mut caps, 61, doc(1, 1), now);
    // The window is enlarged before the settled request is issued: 8000×5000 is over the
    // per-image limit, so the old 2 MiB reservation must not let it through.
    let large = geometry(8000.0, 5000.0, doc(1, 1));
    caps.note_geometry(large);
    let (finished, callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    assert!(callbacks.is_empty(), "no image is captured under the stale reservation");
    assert_eq!(finished.len(), 1);
    let error = finished[0].result.as_ref().expect_err("over the per-image limit");
    assert!(error.contains("per-image"), "{error}");
    assert_eq!(caps.pending(), 0, "the rejected request releases its budget");
    assert_eq!(caps.reserved_bytes(), 0);
}

#[test]
fn an_aggregate_over_budget_after_growth_is_refused() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    caps.note_geometry(geometry(4096.0, 4096.0, None)); // 64 MiB each
    for token in 70..73u64 {
        queue(&mut caps, token, None, now);
    }
    // 5000×5000 = 100 MiB each: two fit 256 MiB together, three do not.
    let grown = geometry(5000.0, 5000.0, None);
    caps.note_geometry(grown);
    let (finished, callbacks) = settle_and_issue(&mut caps, &ctx, now, None);
    assert!(callbacks.is_empty(), "the grown layout settles before any capture");
    assert!(!finished.is_empty(), "at least one request no longer fits the aggregate");
    assert!(finished.iter().all(|f| f.result.is_err()));
    assert!(caps.reserved_bytes() <= MAX_TOTAL_BYTES);
    assert!(caps.pending() < 3 || caps.reserved_bytes() <= MAX_TOTAL_BYTES);
}

#[test]
fn settling_counts_real_ui_passes_and_issues_once() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    caps.note_geometry(geometry(800.0, 600.0, doc(1, 1)));
    // Admission alone does not issue.
    assert!(queue(&mut caps, 7, doc(1, 1), now).try_recv().is_err());
    let mut callbacks = Vec::new();
    assert!(caps.issue(&ctx, now, doc(1, 1), |cb| callbacks.push(cb)).is_empty());
    assert!(callbacks.is_empty(), "no UI pass has settled yet");
    // Two passes are not enough; the third issues exactly one callback.
    caps.note_pass();
    caps.note_pass();
    assert!(caps.issue(&ctx, now, doc(1, 1), |cb| callbacks.push(cb)).is_empty());
    caps.note_pass();
    assert!(caps.issue(&ctx, now, doc(1, 1), |cb| callbacks.push(cb)).is_empty());
    assert_eq!(callbacks.len(), 1, "exactly one callback per request");
    assert!(!caps.any_unissued(), "an issued capture requests no further hidden paint");
    // Re-issuing on the next pass does not duplicate the request.
    caps.note_pass();
    assert!(caps.issue(&ctx, now, doc(1, 1), |cb| callbacks.push(cb)).is_empty());
    assert_eq!(callbacks.len(), 1);
}

#[test]
fn a_completed_image_is_saved_only_when_drained() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let rx = queue(&mut caps, 1, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    let callback = callbacks.pop().expect("one callback");
    callback.complete(image(&g));
    // The callback stores the image; no reply is sent from the callback thread.
    assert!(rx.try_recv().is_err());
    let finished = caps.drain(now, Some(g), doc(1, 1));
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_ok());
    assert!(finished[0].reply.is_some());
    assert_eq!(caps.pending(), 0, "the budget is released after the drain");
    assert!(caps.cancel_all("nothing left").is_empty(), "a drained request is not replied to twice");
}

#[test]
fn a_callback_after_the_deadline_is_not_written() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 2, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    // The image is already stored, but the deadline is checked before it is accepted.
    let late = now + DEADLINE + Duration::from_millis(1);
    let finished = caps.drain(late, Some(g), doc(1, 1));
    assert_eq!(finished.len(), 1);
    let error = finished[0].result.as_ref().expect_err("a late image is a timeout");
    assert!(error.contains("timed out"), "{error}");
    assert!(finished[0].reply.is_some());
    assert_eq!(caps.pending(), 0);
}

#[test]
fn cancellation_makes_a_late_callback_inert_and_releases_the_budget() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 3, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    let callback = callbacks.pop().expect("one callback");
    let finished = caps.cancel_all("test");
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_err());
    assert_eq!(caps.pending(), 0);
    assert_eq!(caps.reserved_bytes(), 0);
    // The slot holds no strong reference from the callback: the late image is dropped.
    callback.complete(image(&g));
    assert!(caps.drain(now, Some(g), doc(1, 1)).is_empty());
}

#[test]
fn an_out_of_order_completion_stays_with_its_own_token() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _a = queue(&mut caps, 11, doc(1, 1), now);
    let _b = queue(&mut caps, 12, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    let cb11 = callbacks.remove(0);
    let cb12 = callbacks.remove(0);
    // Complete in reverse order; each slot stores its own image.
    cb12.complete(Arc::new(ColorImage::filled([800, 600], Color32::RED)));
    cb11.complete(Arc::new(ColorImage::filled([800, 600], Color32::BLUE)));
    let mut finished = caps.drain(now, Some(g), doc(1, 1));
    finished.sort_by_key(|f| f.token);
    assert_eq!(finished.len(), 2);
    assert_eq!(finished[0].token, 11);
    assert_eq!(finished[1].token, 12);
    assert_eq!(finished[0].result.as_ref().unwrap().pixels[0], Color32::BLUE);
    assert_eq!(finished[1].result.as_ref().unwrap().pixels[0], Color32::RED);
}

#[test]
fn a_document_change_is_cancelled_without_capturing_the_new_document() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 21, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    let finished = caps.drain(now, Some(g), doc(2, 1));
    assert_eq!(finished.len(), 1);
    let error = finished[0].result.as_ref().expect_err("the document changed");
    assert!(error.contains("document changed"), "{error}");
}

#[test]
fn a_mid_pass_document_switch_does_not_issue_a_capture() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 23, doc(1, 1), now);
    for _ in 0..SETTLE_PASSES {
        caps.note_pass();
    }
    // The pass started on document 1 but ends on document 2 (a tab click or a control open in
    // `ui`): the frame being drawn is not the one the request was admitted for.
    let mut callbacks = Vec::new();
    let finished = caps.issue(&ctx, now, doc(2, 1), |cb| callbacks.push(cb));
    assert!(callbacks.is_empty(), "no capture is issued for a mixed pass");
    assert_eq!(finished.len(), 1, "the request fails instead of capturing the other document");
    assert!(finished[0].result.is_err());
    assert_eq!(caps.pending(), 0);
}

#[test]
fn a_document_switch_away_and_back_fails_the_capture() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 22, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    // Same document id, but the activation generation advanced: the frame may have been drawn
    // from another document in between, so the image is not attributed to this request.
    let after = geometry(800.0, 600.0, doc_at(1, 1, 1));
    let finished = caps.drain(now, Some(after), doc_at(1, 1, 1));
    assert_eq!(finished.len(), 1);
    let error = finished[0].result.as_ref().expect_err("the switch invalidates the capture");
    assert!(error.contains("document changed"), "{error}");
}

#[test]
fn an_edit_in_the_same_document_does_not_cancel_the_capture() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 25, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    // The document was edited while the image was in flight: same document, new revision.
    let finished = caps.drain(now, Some(g), doc(1, 2));
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_ok(), "an edit is not a document change");
}

#[test]
fn a_geometry_change_retries_then_fails_without_saving_stale_pixels() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 31, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    let resized = geometry(900.0, 700.0, doc(1, 1));
    caps.note_geometry(resized);
    let finished = caps.drain(now, Some(resized), doc(1, 1));
    assert!(finished.is_empty(), "the first resize retries");
    assert_eq!(caps.pending(), 1);
    let mut retries = 0;
    loop {
        let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
        callbacks.pop().expect("a retry callback").complete(image(&resized));
        // A second, third, … resize must eventually give up inside the deadline.
        let moved = geometry(901.0 + retries as f32, 700.0, doc(1, 1));
        caps.note_geometry(moved);
        let finished = caps.drain(now, Some(moved), doc(1, 1));
        if !finished.is_empty() {
            let error = finished[0].result.as_ref().expect_err("stale pixels are not saved");
            assert!(error.contains("layout changed"), "{error}");
            break;
        }
        retries += 1;
        assert!(retries < 10, "the retry budget is bounded");
    }
}

#[test]
fn a_resize_and_back_retries_instead_of_accepting_the_old_image() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 32, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(image(&g));
    // The host resizes, then resizes back before the image is collected. The final values are
    // identical, but the monotonic epoch advanced twice: the image may be from either layout.
    caps.note_geometry(geometry(900.0, 700.0, doc(1, 1)));
    caps.note_geometry(g);
    let finished = caps.drain(now, Some(g), doc(1, 1));
    assert!(finished.is_empty(), "the changed-and-restored layout retries");
    assert_eq!(caps.pending(), 1);
    // The retried capture is issued and accepted against the stable geometry.
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("a retry callback").complete(image(&g));
    let finished = caps.drain(now, Some(g), doc(1, 1));
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_ok());
}

#[test]
fn an_image_whose_size_does_not_match_the_issued_geometry_is_rejected() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 41, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    // Not a uniform fit: a different aspect ratio than the issued 4:3 viewport.
    callbacks.pop().expect("one callback").complete(Arc::new(ColorImage::filled([400, 200], Color32::BLACK)));
    let finished = caps.drain(now, Some(g), doc(1, 1));
    let error = finished[0].result.as_ref().expect_err("wrong-aspect image");
    assert!(error.contains("400×200"), "{error}");
    // A larger image than the issued viewport is never accepted either.
    let ctx = Context::default();
    let mut caps = Captures::default();
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 42, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(Arc::new(ColorImage::filled([1600, 1200], Color32::BLACK)));
    let finished = caps.drain(now, Some(g), doc(1, 1));
    let error = finished[0].result.as_ref().expect_err("larger image");
    assert!(error.contains("larger than"), "{error}");
}

#[test]
fn an_arbitrarily_small_same_aspect_image_is_rejected() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = geometry(800.0, 600.0, doc(1, 1));
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 44, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(Arc::new(ColorImage::filled([100, 75], Color32::BLACK)));
    let finished = caps.drain(now, Some(g), doc(1, 1));
    let error = finished[0].result.as_ref().expect_err("a downscaled surface is not the frame");
    assert!(error.contains("100×75"), "{error}");
    assert!(error.contains("texture-side fit"), "{error}");
}

#[test]
fn a_uniformly_fitted_image_is_accepted() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    // A window larger than the adapter's texture side is fitted by the renderer (E2): the image
    // is the exact texture-side fit, not an arbitrary downscale.
    let g = CaptureGeometry { max_texture_side: 4096, ..geometry(6000.0, 3375.0, doc(1, 1)) };
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 43, doc(1, 1), now);
    let (_, mut callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    callbacks.pop().expect("one callback").complete(Arc::new(ColorImage::filled([4096, 2304], Color32::BLACK)));
    let finished = caps.drain(now, Some(g), doc(1, 1));
    assert!(finished[0].result.is_ok(), "an E2-fitted image keeps the issued aspect");
}

#[test]
fn a_minimized_window_is_refused_before_issuance() {
    let ctx = Context::default();
    let now = Instant::now();
    let mut caps = Captures::default();
    let g = CaptureGeometry { minimized: true, ..geometry(800.0, 600.0, doc(1, 1)) };
    caps.note_geometry(g);
    let _rx = queue(&mut caps, 52, doc(1, 1), now);
    let (finished, callbacks) = settle_and_issue(&mut caps, &ctx, now, doc(1, 1));
    assert!(callbacks.is_empty());
    assert_eq!(finished.len(), 1);
    let error = finished[0].result.as_ref().expect_err("no surface while minimized");
    assert!(error.contains("minimized"), "{error}");
}

#[test]
fn a_never_issued_request_times_out() {
    let now = Instant::now();
    let mut caps = Captures::default();
    caps.note_geometry(geometry(800.0, 600.0, doc(1, 1)));
    let _rx = queue(&mut caps, 51, doc(1, 1), now);
    let finished = caps.drain(now + DEADLINE, Some(geometry(800.0, 600.0, doc(1, 1))), doc(1, 1));
    assert_eq!(finished.len(), 1);
    assert!(finished[0].result.is_err());
    assert_eq!(caps.pending(), 0);
}

/// Execute the same logic→UI order as eframe, with genuinely new host geometry before logic.
fn actual_capture_frame(app: &mut crate::PhotocraftApp, ctx: &Context, size: egui::Vec2, scale: f32, minimized: bool) -> egui::FullOutput {
    use eframe::App as _;
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(scale), minimized: Some(minimized), ..Default::default() },
    );
    let mut input = egui::RawInput { screen_rect: Some(rect), viewports, focused: true, ..Default::default() };
    app.raw_input_hook(ctx, &mut input);
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
    output.textures_delta.clear();
    output
}

fn actual_callback(output: &egui::FullOutput) -> Option<egui::ScreenshotCallback> {
    output.viewport_output.values().flat_map(|viewport| &viewport.commands).find_map(|command| match command {
        egui::ViewportCommand::Screenshot(callback) => Some(callback.clone()),
        _ => None,
    })
}

#[test]
fn completed_callback_revalidates_next_raw_input_before_actual_app_writes() {
    use std::cell::RefCell;
    use std::rc::Rc;
    // Unchanged success, resize, DPI change, and minimized are separate real app instances.
    for (size, scale, minimized) in
        [(vec2(600.0, 400.0), 1.0, false), (vec2(640.0, 400.0), 1.0, false), (vec2(600.0, 400.0), 2.0, false), (vec2(600.0, 400.0), 1.0, true)]
    {
        let changed = size != vec2(600.0, 400.0) || scale != 1.0 || minimized;
        let writes = Rc::new(RefCell::new(Vec::new()));
        let encodes = Rc::new(RefCell::new(Vec::new()));
        let (written, encoded) = (writes.clone(), encodes.clone());
        let services = crate::Services {
            automation_write: Some(Box::new(move |path, bytes| {
                written.borrow_mut().push((path.to_owned(), bytes.to_vec()));
                Ok(())
            })),
            encode_png: Some(Box::new(move |w, h, rgba| {
                encoded.borrow_mut().push((w, h, rgba.get(..4).unwrap_or_default().to_vec()));
                let image =
                    photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, rgba.to_vec()).map_err(|error| error.to_string())?;
                photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default())
                    .map_err(|error| error.to_string())
            })),
            ..Default::default()
        };
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
        let ctx = Context::default();
        for _ in 0..4 {
            actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
        }
        let old = app.captures.last_geometry().expect("the old real UI geometry");
        let document = app.document_identity();
        let (tx, rx) = std::sync::mpsc::channel();
        app.captures.admit(91, Some("fixture/current.png".into()), tx, document, 0.0, super::screenshot::now()).unwrap();
        let mut callback = None;
        for _ in 0..8 {
            let output = actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
            if let Some(found) = actual_callback(&output) {
                callback = Some(found);
                break;
            }
        }
        let old_image = Arc::new(ColorImage::filled(old.expected_pixels().unwrap(), Color32::RED));
        let old_pixels = Arc::downgrade(&old_image);
        callback.expect("actual app issued one screenshot callback").complete(old_image);
        assert!(writes.borrow().is_empty() && encodes.borrow().is_empty(), "the real callback only stores pixels");
        let output = actual_capture_frame(&mut app, &ctx, size, scale, minimized);
        if changed {
            assert!(writes.borrow().is_empty() && encodes.borrow().is_empty(), "no stale encode/write on the first changed input");
            assert!(old_pixels.upgrade().is_none(), "old pixels are released on geometry rejection");
            let current = app.captures.last_geometry().unwrap();
            assert!(current.logical_size != old.logical_size || current.pixels_per_point != old.pixels_per_point || current.minimized != old.minimized);
            if minimized {
                // A minimized surface cannot be retried. It fails once, then an independent
                // request after restoration proves that cancellation releases its budget.
                let reply = rx.try_recv().expect("one minimized error");
                assert_eq!(reply["ok"], false, "{reply}");
                assert!(reply["error"].as_str().unwrap().contains("minimized"));
                assert_eq!(app.captures.pending(), 0);
                assert_eq!(app.captures.reserved_bytes(), 0);
                assert!(actual_callback(&output).is_none());
                actual_capture_frame(&mut app, &ctx, size, scale, false);
                let (tx, restored_rx) = std::sync::mpsc::channel();
                app.captures.admit(92, Some("fixture/restored.png".into()), tx, document, 0.0, super::screenshot::now()).unwrap();
                for _ in 0..8 {
                    let output = actual_capture_frame(&mut app, &ctx, size, scale, false);
                    if let Some(callback) = actual_callback(&output) {
                        callback.complete(Arc::new(ColorImage::filled(app.captures.last_geometry().unwrap().expected_pixels().unwrap(), Color32::BLUE)));
                        break;
                    }
                }
                actual_capture_frame(&mut app, &ctx, size, scale, false);
                let reply = restored_rx.try_recv().expect("the restored window accepts a new request");
                assert_eq!(reply["ok"], true, "{reply}");
                assert_eq!(writes.borrow().len(), 1);
                assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
                assert_eq!(encodes.borrow().len(), 1);
                assert_eq!(encodes.borrow()[0].2, [0, 0, 255, 255]);
                assert_eq!(app.captures.pending(), 0);
                assert_eq!(app.captures.reserved_bytes(), 0);
                assert!(rx.try_recv().is_err() && restored_rx.try_recv().is_err());
                continue;
            }
            assert!(rx.try_recv().is_err(), "bounded retry has not delivered a stale success");
            let mut retry = actual_callback(&output);
            for _ in 0..8 {
                if retry.is_some() {
                    break;
                }
                retry = actual_callback(&actual_capture_frame(&mut app, &ctx, size, scale, minimized));
            }
            retry
                .expect("the actual app reissues at bounded current geometry")
                .complete(Arc::new(ColorImage::filled(current.expected_pixels().unwrap(), Color32::BLUE)));
            actual_capture_frame(&mut app, &ctx, size, scale, minimized);
            assert_eq!(encodes.borrow().first().map(|entry| entry.2.as_slice()), Some([0, 0, 255, 255].as_slice()), "only the new callback image is encoded");
        }
        let reply = rx.try_recv().expect("one response after the valid callback");
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(writes.borrow().len(), 1);
        assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"), "real PNG encoding and automation_write ran");
        assert_eq!(encodes.borrow().len(), 1);
        assert_eq!(app.captures.pending(), 0);
        assert_eq!(app.captures.reserved_bytes(), 0);
        actual_capture_frame(&mut app, &ctx, size, scale, minimized);
        assert_eq!(writes.borrow().len(), 1, "no duplicate output");
        assert!(rx.try_recv().is_err());
    }
}
