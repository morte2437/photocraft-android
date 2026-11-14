//! Paragraph-box handle drags of the Type tool, rendered off the UI thread (#7032).
//!
//! Applying a dragged box with `type.edit` lays the text out and rasterizes it, which on a big
//! paragraph takes tens of milliseconds, and the canvas sends one pointer move per frame. So while
//! a handle is dragged the document is left alone: the moves only say which box is wanted, a
//! worker lays out and rasterizes a copy of the layer in that box, and the canvas shows the
//! document with that copy. One render runs at a time and the newest box wins: a result for an
//! older box is still shown (it is newer than what is on screen), then the next render starts for
//! the box wanted now. Releasing (or anything else that ends the drag) runs the same `type.edit`
//! a synchronous drag would have run for its last move, so the finished document and its history
//! are exactly what they were before this preview existed.
//!
//! A command that starts while a handle is held (Free Transform, a menu item, a shortcut) ends the
//! drag first ([`end_drag`]), so it sees the box applied. If the document changed some other way
//! during the drag, the preview is dropped and the gesture ends: the press-time box is never
//! applied over that change.
//!
//! Only with background jobs on a native build, and not for Type Mask edits (their overlay is
//! drawn from the document's layout); otherwise every move runs `type.edit` as before.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, PoisonError};

use photocraft_color::PixelFormat;
use photocraft_doc::{DocId, Document, LayerContent, LayerId, TextLayer};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;

/// Preview keys: `BASE + n`, one per result shown (see `canvas::display_doc`).
const BASE: u64 = 1 << 46;
/// How often a running render is polled for its result when its thread can't wake the canvas.
const POLL_MS: u64 = 30;
/// Results whose damage areas are remembered for canvas invalidation.
const MAX_AREAS: usize = 128;

/// Test-only controls of the worker. Empty (and free) outside tests.
#[derive(Clone, Default)]
struct Hooks {
    /// While `Some` and closed, a render waits before computing, so a test can hold one in flight.
    #[cfg(test)]
    gate: Option<Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>>,
}

impl Hooks {
    /// In tests, holds a render back until the gate opens; does nothing otherwise.
    #[allow(clippy::unused_self)]
    fn wait(&self) {
        #[cfg(test)]
        {
            if let Some(gate) = &self.gate {
                let mut open = gate.0.lock().unwrap_or_else(PoisonError::into_inner);
                while !*open {
                    open = gate.1.wait(open).unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
    }
}

/// The render that is computing: the box it draws and where its surface arrives.
struct Running {
    boxed: [f64; 4],
    rx: Receiver<Option<Surface>>,
    /// Test-only notification sent after the worker has sent its result.
    #[cfg(test)]
    finished: Receiver<()>,
}

/// The newest result: the box it shows, the document with it, and its canvas key.
struct Shown {
    boxed: [f64; 4],
    doc: Arc<Document>,
    key: u64,
}

pub(crate) struct TypeBoxPreview {
    /// The document, its revision and the layer at the press: if any of them changes, the drag's
    /// box is not applied to something else.
    document: DocId,
    revision: u64,
    layer: LayerId,
    /// The edit session's coalescing key, for the `type.edit` that applies the box.
    session: String,
    /// The box (`[x, y, width, height]`, document px, as `type.edit`'s `box`) the pointer wants.
    wanted: Option<[f64; 4]>,
    /// A box whose render failed: not tried again (the release still renders it).
    failed: Option<[f64; 4]>,
    running: Option<Running>,
    shown: Option<Shown>,
    serial: u64,
    /// The layer's pixel bounds in the document (key 0), and in each result shown, for damage.
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
    /// To wake the canvas when a render finishes; set by [`wake`] on a canvas frame.
    ctx: Option<egui::Context>,
    hooks: Hooks,
    /// Test-only count of renders started.
    #[cfg(test)]
    spawned: usize,
}

/// Whether handle drags render in the background here.
fn enabled(app: &PhotocraftApp) -> bool {
    cfg!(not(target_arch = "wasm32")) && app.background_jobs
}

/// A handle drag of `p`'s layer is in progress.
fn dragging(app: &PhotocraftApp, p: &TypeBoxPreview) -> bool {
    app.ui.tool.is_type() && app.ui.text_edit.as_ref().is_some_and(|e| e.layer == p.layer.0 && e.resize.is_some())
}

/// The text layer `layer` of `doc`, if it is one.
fn text_of(doc: &Document, layer: LayerId) -> Option<&TextLayer> {
    match &doc.layer(layer)?.content {
        LayerContent::Text(t) => Some(t),
        _ => None,
    }
}

/// A handle drag of text layer `layer` starts: remember the document as it is now. Does nothing
/// when drags apply the box as they go.
pub(crate) fn begin(app: &mut PhotocraftApp, layer: LayerId, session: String) {
    // A drag left over from before (its release never came) still owes its box to the document.
    flush(app);
    // The mask overlay is drawn from the document's layout, which a background drag leaves stale.
    if !enabled(app) || app.ui.text_edit.as_ref().is_some_and(|e| e.mask) {
        return;
    }
    let Some(st) = app.session.active() else { return };
    if text_of(&st.doc, layer).is_none() {
        return;
    }
    let original_area = st.doc.layer(layer).and_then(|l| l.surface()).map_or(Rect::EMPTY, |s| s.tile_bounds());
    app.type_box_preview = Some(TypeBoxPreview {
        document: st.doc.id,
        revision: st.revision,
        layer,
        session,
        wanted: None,
        failed: None,
        running: None,
        shown: None,
        serial: 0,
        original_area,
        areas: VecDeque::new(),
        ctx: None,
        hooks: Hooks::default(),
        #[cfg(test)]
        spawned: 0,
    });
}

/// End the handle-drag gesture of `layer`'s edit session, as a release does.
fn end_gesture(app: &mut PhotocraftApp, layer: LayerId) {
    if let Some(e) = app.ui.text_edit.as_mut().filter(|e| e.layer == layer.0) {
        e.resize = None;
        e.dragging = false;
    }
}

/// The drag moved the box to `boxed`. Returns true when the move is dealt with: it is recorded for
/// the preview, or the preview was stale and the drag ended with it. Returns false when no
/// background drag is in progress for `layer` (or the box isn't a plain finite one): the caller
/// then applies it with `type.edit`.
pub(crate) fn want(app: &mut PhotocraftApp, layer: LayerId, boxed: [f64; 4]) -> bool {
    let Some(p) = app.type_box_preview.as_ref() else { return false };
    let current =
        p.layer == layer && app.session.documents().iter().any(|st| st.doc.id == p.document && st.revision == p.revision && text_of(&st.doc, layer).is_some());
    if !current {
        // Something else changed the document or layer since the press. The preview would show a
        // stale copy and `type.edit` from the press-time box would run over that change, so drop
        // the preview and end the drag; the rest of the gesture does nothing.
        app.type_box_preview = None;
        end_gesture(app, layer);
        return true;
    }
    if !boxed.iter().all(|v| v.is_finite()) {
        // Not a box the preview can draw: the command decides what it means.
        app.type_box_preview = None;
        return false;
    }
    if let Some(p) = app.type_box_preview.as_mut() {
        p.wanted = Some(boxed);
    }
    spawn_if_idle(app);
    true
}

/// The box the pointer wants while a background drag of `layer` is in progress, for the overlay.
/// The text layout, caret and overflow mark still come from the document, which has not changed.
pub(crate) fn wanted_box(app: &PhotocraftApp, layer: LayerId) -> Option<[f64; 4]> {
    let p = app.type_box_preview.as_ref().filter(|p| p.layer == layer && dragging(app, p))?;
    p.wanted
}

/// Start a render of the wanted box if none runs and it isn't what is shown already.
fn spawn_if_idle(app: &mut PhotocraftApp) {
    let Some(p) = app.type_box_preview.as_ref() else { return };
    let Some(boxed) = p.wanted else { return };
    if p.running.is_some() || p.failed == Some(boxed) || p.shown.as_ref().is_some_and(|s| s.boxed == boxed) {
        return;
    }
    let Some(st) = app.session.documents().iter().find(|st| st.doc.id == p.document && st.revision == p.revision) else { return };
    let Some(t) = text_of(&st.doc, p.layer) else { return };
    let mut layer = t.clone();
    photocraft_engine::type_cmds::apply_box(&mut layer, boxed);
    let (dpi, format) = (st.doc.resolution_dpi, st.doc.pixel_format());
    let (ctx, hooks) = (p.ctx.clone(), p.hooks.clone());
    let running = spawn(layer, dpi, format, boxed, ctx, hooks);
    if let Some(p) = app.type_box_preview.as_mut() {
        #[cfg(test)]
        {
            p.spawned += 1;
        }
        if running.is_none() {
            p.failed = Some(boxed);
        }
        p.running = running;
    }
}

/// Lay out and rasterize `layer` like `TextEngine::render`, but holding the shared engine only
/// for the layout: the UI thread locks it every frame, and the rasterizing is most of the time.
fn render(layer: &TextLayer, dpi: f32, format: PixelFormat) -> Surface {
    let layout = {
        let mut engine = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
        engine.layout(layer, dpi)
    };
    let warp = photocraft_text::render::layout_deform(&layout, layer);
    photocraft_text::render::rasterize_warped(&layout, &layer.transform, format, layer.antialias, warp.as_ref()).surface
}

/// Starts a worker thread that renders `layer` and sends back its surface; `None` if no thread could be started.
fn spawn(layer: TextLayer, dpi: f32, format: PixelFormat, boxed: [f64; 4], ctx: Option<egui::Context>, hooks: Hooks) -> Option<Running> {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(test)]
    let (finished_tx, finished) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("type-box-preview".into())
        .spawn(move || {
            hooks.wait();
            // A panic leaves no preview; the release still renders the box on the UI thread.
            let surface = std::panic::catch_unwind(AssertUnwindSafe(|| render(&layer, dpi, format))).ok();
            // The receiver is gone when the drag ended meanwhile.
            let _ = tx.send(surface);
            #[cfg(test)]
            let _ = finished_tx.send(());
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        })
        .ok()?;
    Some(Running {
        boxed,
        rx,
        #[cfg(test)]
        finished,
    })
}

/// Take a finished render, if there is one, and make its document the one shown.
fn poll(app: &mut PhotocraftApp) {
    let Some(p) = app.type_box_preview.as_mut() else { return };
    let Some(r) = p.running.as_ref() else { return };
    let result = match r.rx.try_recv() {
        Ok(surface) => surface,
        Err(TryRecvError::Disconnected) => None,
        Err(TryRecvError::Empty) => return,
    };
    let Some(done) = p.running.take() else { return };
    match result {
        Some(surface) => publish(app, done.boxed, surface),
        None => p.failed = Some(done.boxed),
    }
}

/// Show the session document with its layer in `boxed`, drawn as `surface`.
fn publish(app: &mut PhotocraftApp, boxed: [f64; 4], surface: Surface) {
    let Some(p) = app.type_box_preview.as_ref() else { return };
    let (document, revision, layer) = (p.document, p.revision, p.layer);
    let Some(st) = app.session.documents().iter().find(|st| st.doc.id == document && st.revision == revision) else { return };
    let mut doc = (*st.doc).clone();
    let area = surface.tile_bounds();
    {
        let Some(l) = doc.layer_mut(layer) else { return };
        let LayerContent::Text(t) = &mut l.content else { return };
        photocraft_engine::type_cmds::apply_box(t, boxed);
        t.cache = Some(surface);
    }
    let doc = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    let Some(p) = app.type_box_preview.as_mut() else { return };
    p.serial += 1;
    let key = BASE + p.serial;
    if p.areas.len() >= MAX_AREAS {
        p.areas.pop_front();
    }
    p.areas.push_back((key, area));
    p.shown = Some(Shown { boxed, doc, key });
}

/// The document to show for document `idx`: the session document with the dragged box applied,
/// once a render of one has arrived. `None` (and the preview dropped, if it is stale) otherwise.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let p = app.type_box_preview.as_ref()?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != p.document {
        return None;
    }
    let stale = st.revision != p.revision || text_of(&st.doc, p.layer).is_none();
    let drag = dragging(app, p);
    if stale {
        // As in `want`: the document moved on, so the held handle no longer has a box to apply.
        let layer = p.layer;
        app.type_box_preview = None;
        end_gesture(app, layer);
        return None;
    }
    // A drag that ended some other way is settled by `settle`, not here: this runs while the
    // canvas is being drawn.
    if !drag {
        return None;
    }
    poll(app);
    spawn_if_idle(app);
    let s = app.type_box_preview.as_ref()?.shown.as_ref()?;
    Some((s.doc.clone(), s.key))
}

/// Keep frames coming while a render computes, and remember the context to wake from the worker.
pub(crate) fn wake(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.type_box_preview.as_mut() else { return };
    if p.ctx.is_none() {
        p.ctx = Some(ctx.clone());
    }
    if p.running.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(POLL_MS));
    }
}

/// The drag ends: apply the box it wanted with `type.edit`, exactly as a synchronous drag has by
/// now (one render, in the edit session's history step), and drop the preview. Nothing happens
/// without a preview, if it never got a box, or if the document, its revision or the layer is no
/// longer the one the drag started on.
pub(crate) fn flush(app: &mut PhotocraftApp) {
    let Some(p) = app.type_box_preview.take() else { return };
    let Some(boxed) = p.wanted else { return };
    let Some(i) = app.session.documents().iter().position(|st| st.doc.id == p.document && st.revision == p.revision && text_of(&st.doc, p.layer).is_some())
    else {
        return;
    };
    // The drag's document may no longer be the active one (a tab switched mid-drag).
    let current = app.session.active_index();
    if current != Some(i) {
        app.session.set_active(i);
    }
    let _ = app.run("type.edit", json!({"layer": p.layer.0, "box": boxed, "coalesce": p.session}));
    if let Some(c) = current.filter(|c| *c != i) {
        app.session.set_active(c);
    }
}

/// A command other than a pure view one starts while a handle may be held: apply the box the drag
/// reached and end the gesture, so the command (Free Transform capturing the document, say) works
/// on the box, not the press-time one. Does nothing without a background drag.
pub(crate) fn end_drag(app: &mut PhotocraftApp) {
    let Some(layer) = app.type_box_preview.as_ref().map(|p| p.layer) else { return };
    flush(app);
    end_gesture(app, layer);
}

/// The edit was cancelled: its session is undone, so the box the drag wanted is not applied.
pub(crate) fn discard(app: &mut PhotocraftApp) {
    app.type_box_preview = None;
}

/// Once a frame: a drag that ended without a release reaching the Type tool (focus lost, a Free
/// Transform started, the tool switched, the edit replaced) keeps the box it had reached, as it
/// did when every move was applied. If the document changed meanwhile, the box is dropped.
pub(crate) fn settle(app: &mut PhotocraftApp) {
    if app.type_box_preview.as_ref().is_some_and(|p| !dragging(app, p)) {
        flush(app);
    }
}

/// Where the layer's pixels were and are, between two keys the canvas showed (0: the document).
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    if before == after {
        return None;
    }
    let p = app.type_box_preview.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}

/// The preview's box as the overlay draws it: the layer's transform moved to the box's origin.
pub(crate) fn overlay_affine(aff: Affine, boxed: [f64; 4]) -> Affine {
    Affine { m: [aff.m[0], aff.m[1], aff.m[2], aff.m[3], boxed[0], boxed[1]] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;
    use egui::Modifiers;
    use photocraft_doc::text::TextShape;

    /// An app with one paragraph text layer in a box, open for editing.
    fn app_with_box() -> (PhotocraftApp, LayerId) {
        app_with_box_and(crate::Services::default())
    }

    /// [`app_with_box`] with the given shell services.
    fn app_with_box_and(services: crate::Services) -> (PhotocraftApp, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 1600, "height": 900})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.extras.snap = false;
        app.ui.tool = Tool::Type;
        let id = LayerId(
            app.run("type.create", json!({"text": "Hello é世界\nSecond line", "size": 32, "x": 300, "y": 420, "box": [300, 300, 240, 180]})).unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        app.run("type.edit", json!({"layer": id.0, "transform": [1.0, 0.0, 0.0, 1.0, 300.0, 300.0]})).unwrap();
        crate::type_tool::edit_active(&mut app).unwrap();
        (app, id)
    }

    // The bottom edge's handle sits at the midpoint of the box's lower edge.
    /// Press the bottom-edge handle of the box.
    fn press(app: &mut PhotocraftApp) {
        tool_event(app, ToolEvent::Down { x: 420.0, y: 480.0, pressure: 1.0 }, Modifiers::NONE);
    }
    /// Move the held bottom-edge handle to document `y`.
    fn to(app: &mut PhotocraftApp, y: f64) {
        tool_event(app, ToolEvent::Move { x: 420.0, y, pressure: 1.0 }, Modifiers::NONE);
    }
    /// Release the bottom-edge handle at document `y`.
    fn release(app: &mut PhotocraftApp, y: f64) {
        tool_event(app, ToolEvent::Up { x: 420.0, y }, Modifiers::NONE);
    }
    // The right edge's handle sits at the midpoint of the box's right edge.
    /// Press the right-edge handle of the box.
    fn press_right(app: &mut PhotocraftApp) {
        tool_event(app, ToolEvent::Down { x: 540.0, y: 390.0, pressure: 1.0 }, Modifiers::NONE);
    }
    /// Move the held right-edge handle to document `x`.
    fn to_x(app: &mut PhotocraftApp, x: f64) {
        tool_event(app, ToolEvent::Move { x, y: 390.0, pressure: 1.0 }, Modifiers::NONE);
    }
    /// Release the right-edge handle at document `x`.
    fn release_x(app: &mut PhotocraftApp, x: f64) {
        tool_event(app, ToolEvent::Up { x, y: 390.0 }, Modifiers::NONE);
    }
    /// The active document's revision.
    fn revision(app: &PhotocraftApp) -> u64 {
        app.session.active().unwrap().revision
    }
    /// The text layer `id` of `doc`.
    fn layer_of(doc: &Document, id: LayerId) -> TextLayer {
        text_of(doc, id).unwrap().clone()
    }
    /// The text layer `id` of the active document.
    fn session_layer(app: &PhotocraftApp, id: LayerId) -> TextLayer {
        layer_of(&app.session.active().unwrap().doc, id)
    }
    /// The height of a box-shaped text layer.
    fn height(t: &TextLayer) -> f32 {
        match t.shape {
            TextShape::Box { height, .. } => height,
            ref s => panic!("not a box: {s:?}"),
        }
    }
    /// The width of a box-shaped text layer.
    fn width(t: &TextLayer) -> f32 {
        match t.shape {
            TextShape::Box { width, .. } => width,
            ref s => panic!("not a box: {s:?}"),
        }
    }
    /// Whether two layers' cached pixels are identical.
    fn same_pixels(a: &TextLayer, b: &TextLayer) -> bool {
        let (a, b) = (a.cache.as_ref().unwrap(), b.cache.as_ref().unwrap());
        let r = a.content_bounds();
        r == b.content_bounds() && (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| a.rgba(x, y) == b.rgba(x, y)))
    }
    /// Turn background jobs on, as the desktop app does.
    fn background(app: &mut PhotocraftApp) {
        app.background_jobs = true;
    }
    /// Let held renders run.
    fn open_gate(gate: &Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>) {
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
    }
    /// Hold every render back until the returned gate opens.
    fn hold(app: &mut PhotocraftApp) -> Arc<(std::sync::Mutex<bool>, std::sync::Condvar)> {
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        app.type_box_preview.as_mut().unwrap().hooks.gate = Some(gate.clone());
        gate
    }
    /// Wait for each render in turn (their threads signal when done), polling after each, until
    /// none runs: the document shown then is for the newest box.
    fn wait_idle(app: &mut PhotocraftApp) -> Option<(Arc<Document>, u64)> {
        let mut out = None;
        for _ in 0..16 {
            if let Some(r) = app.type_box_preview.as_ref().and_then(|p| p.running.as_ref()) {
                let _ = r.finished.recv_timeout(std::time::Duration::from_secs(300));
            }
            out = display_doc(app, 0);
            if app.type_box_preview.as_ref().is_none_or(|p| p.running.is_none()) {
                break;
            }
        }
        out
    }

    /// A background drag changes nothing until release, then equals the synchronous drag.
    #[test]
    fn an_async_drag_leaves_the_document_alone_until_release_and_ends_as_the_sync_drag() {
        let (mut sync, id) = app_with_box();
        press(&mut sync);
        to(&mut sync, 520.0);
        to(&mut sync, 560.0);
        release(&mut sync, 560.0);
        let want = session_layer(&sync, id);
        assert_eq!(height(&want), 260.0);

        let (mut app, id) = app_with_box();
        background(&mut app);
        let (rev, steps) = (revision(&app), app.session.active().unwrap().history.past_len());
        press(&mut app);
        to(&mut app, 520.0);
        to(&mut app, 560.0);
        assert_eq!(revision(&app), rev, "moves leave the document alone");
        assert_eq!(height(&session_layer(&app, id)), 180.0);
        let (shown, key) = wait_idle(&mut app).expect("a preview once the render arrived");
        assert!(key > BASE);
        let preview = layer_of(&shown, id);
        assert_eq!(height(&preview), 260.0, "the newest box");
        assert!(preview.cache.is_some());
        assert_eq!(revision(&app), rev, "showing it changes nothing");

        release(&mut app, 560.0);
        assert_eq!(revision(&app), rev + 1, "release is one edit");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        assert!(app.type_box_preview.is_none());
        let got = session_layer(&app, id);
        assert_eq!(got.shape, want.shape);
        assert_eq!(got.transform.m, want.transform.m);
        assert!(same_pixels(&got, &want), "the finished layer is the sync drag's");
        assert!(same_pixels(&got, &preview), "and what the preview showed");
    }

    /// Moves while a render runs start no second one, and the next render draws the newest box.
    #[test]
    fn the_newest_box_wins_with_one_render_at_a_time() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        let gate = hold(&mut app);
        to(&mut app, 500.0);
        to(&mut app, 520.0);
        to(&mut app, 560.0);
        assert_eq!(app.type_box_preview.as_ref().unwrap().spawned, 1, "moves while a render runs start no other");
        open_gate(&gate);
        let (shown, _) = wait_idle(&mut app).expect("a preview");
        assert_eq!(app.type_box_preview.as_ref().unwrap().spawned, 2, "the first render, then one for the newest box");
        assert_eq!(height(&layer_of(&shown, id)), 260.0);
        release(&mut app, 560.0);
    }

    /// Without background jobs every move runs `type.edit` at once.
    #[test]
    fn a_sync_drag_applies_every_move_at_once() {
        let (mut app, id) = app_with_box();
        let rev = revision(&app);
        press(&mut app);
        assert!(app.type_box_preview.is_none());
        to(&mut app, 520.0);
        assert_eq!(revision(&app), rev + 1);
        to(&mut app, 560.0);
        assert_eq!(revision(&app), rev + 2);
        assert_eq!(height(&session_layer(&app, id)), 260.0);
        release(&mut app, 560.0);
    }

    /// A background drag is one history step.
    #[test]
    fn one_undo_restores_the_box_after_an_async_drag() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let steps = app.session.active().unwrap().history.past_len();
        press(&mut app);
        to(&mut app, 560.0);
        wait_idle(&mut app);
        release(&mut app, 560.0);
        assert_eq!(height(&session_layer(&app, id)), 260.0);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(height(&session_layer(&app, id)), 180.0);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }

    /// Commit keeps the box the drag reached; cancel undoes the session.
    #[test]
    fn committing_mid_drag_keeps_the_dragged_box_and_cancelling_drops_it() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        crate::type_tool::commit(&mut app);
        assert!(app.type_box_preview.is_none() && app.ui.text_edit.is_none());
        assert_eq!(height(&session_layer(&app, id)), 260.0, "the box the drag reached");

        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        crate::type_tool::cancel(&mut app);
        assert!(app.type_box_preview.is_none() && app.ui.text_edit.is_none());
        assert_eq!(height(&session_layer(&app, id)), 180.0, "cancelling leaves the box");
    }

    /// `settle` applies the box of a drag that ended without a release, unless the document changed.
    #[test]
    fn a_drag_that_ends_without_a_release_is_applied_by_the_frame_step() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        settle(&mut app);
        assert!(app.type_box_preview.is_some(), "still dragging");
        assert_eq!(height(&session_layer(&app, id)), 180.0);
        // As `cancel_drag` does when the window loses focus.
        app.ui.text_edit.as_mut().unwrap().resize = None;
        assert!(display_doc(&mut app, 0).is_none());
        settle(&mut app);
        assert!(app.type_box_preview.is_none());
        assert_eq!(height(&session_layer(&app, id)), 260.0);

        // Something else edited the document meanwhile: the box isn't applied over it.
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        app.run("layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
        app.ui.text_edit.as_mut().unwrap().resize = None;
        settle(&mut app);
        assert!(app.type_box_preview.is_none());
        assert_eq!(height(&session_layer(&app, id)), 180.0);
    }

    /// A layer deleted mid-drag clears the preview and nothing panics.
    #[test]
    fn deleting_the_layer_mid_drag_clears_the_preview_without_a_panic() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 520.0);
        wait_idle(&mut app);
        app.session.execute("layer.delete", json!({"layer": id.0})).unwrap();
        assert!(display_doc(&mut app, 0).is_none());
        assert!(app.type_box_preview.is_none());
        to(&mut app, 560.0);
        release(&mut app, 560.0);
        assert!(app.type_box_preview.is_none());
    }

    /// A width drag rewraps the text in the preview and ends as the synchronous drag does.
    #[test]
    fn a_width_drag_rewraps_the_preview_and_ends_as_the_sync_drag() {
        let (mut sync, id) = app_with_box();
        press_right(&mut sync);
        to_x(&mut sync, 480.0);
        to_x(&mut sync, 420.0);
        release_x(&mut sync, 420.0);
        let want = session_layer(&sync, id);
        assert_eq!(width(&want), 120.0);

        let (mut app, id) = app_with_box();
        background(&mut app);
        let original = session_layer(&app, id);
        let rev = revision(&app);
        press_right(&mut app);
        assert!(app.type_box_preview.is_some(), "a background drag started");
        to_x(&mut app, 480.0);
        to_x(&mut app, 420.0);
        assert_eq!(revision(&app), rev, "moves leave the document alone");
        let (shown, _) = wait_idle(&mut app).expect("a preview once the render arrived");
        let preview = layer_of(&shown, id);
        assert_eq!(width(&preview), 120.0, "the newest box");
        assert!(!same_pixels(&preview, &original), "the narrower box wraps the text differently");

        release_x(&mut app, 420.0);
        let got = session_layer(&app, id);
        assert_eq!(got.shape, want.shape);
        assert_eq!(got.transform.m, want.transform.m);
        assert!(same_pixels(&got, &want), "the finished layer is the sync drag's");
    }

    /// Commands apply the box and end the drag first; zoom commands leave it going.
    #[test]
    fn a_command_mid_drag_applies_the_box_and_ends_the_drag_but_a_view_command_does_not() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let ctx = egui::Context::default();
        let steps = app.session.active().unwrap().history.past_len();
        press(&mut app);
        to(&mut app, 560.0);
        assert_eq!(height(&session_layer(&app, id)), 180.0, "not applied yet");

        // A pure view command leaves the drag going.
        crate::menus::invoke(&mut app, &ctx, "view.zoomIn", json!({})).unwrap();
        assert!(app.type_box_preview.is_some());
        assert!(app.ui.text_edit.as_ref().unwrap().resize.is_some());
        assert_eq!(height(&session_layer(&app, id)), 180.0);

        // Any other command sees the box applied, in the drag's one history step.
        crate::menus::invoke(&mut app, &ctx, "layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
        let t = session_layer(&app, id);
        assert_eq!(height(&t), 260.0, "the dragged box was applied before the command ran");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 2, "the drag's step, then the command");
        assert!(app.type_box_preview.is_none());
        let e = app.ui.text_edit.as_ref().unwrap();
        assert!(e.resize.is_none() && !e.dragging);

        // Without a background drag nothing changes.
        let (mut sync, _) = app_with_box();
        press(&mut sync);
        let rev = revision(&sync);
        end_drag(&mut sync);
        assert_eq!(revision(&sync), rev);
        assert!(sync.ui.text_edit.as_ref().unwrap().resize.is_some(), "a sync drag is left going");
    }

    /// Opening or closing a panel (Cmd+T toggles the Character panel while typing) only lays out
    /// windows, so a held handle keeps dragging.
    #[test]
    fn toggling_a_panel_mid_drag_leaves_the_drag_going() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let ctx = egui::Context::default();
        press(&mut app);
        to(&mut app, 520.0);
        for command in ["type.panels.character", "window.toggle.layers", "window.panel.close"] {
            let _ = crate::menus::invoke(&mut app, &ctx, command, json!({}));
            assert!(app.type_box_preview.is_some(), "{command} ended the drag");
            assert!(app.ui.text_edit.as_ref().unwrap().resize.is_some(), "{command} ended the drag");
        }
        to(&mut app, 560.0);
        release(&mut app, 560.0);
        assert_eq!(height(&session_layer(&app, id)), 260.0, "the drag carried on to its last move");
    }

    /// The control channel's `engine.execute` runs engine commands without `menus::invoke`, and
    /// `app.save` writes the document directly: both must see the dragged box too.
    #[test]
    fn control_commands_mid_drag_see_the_dragged_box() {
        use crate::control::{ControlRequest, Outcome, handle};
        use std::cell::RefCell;
        use std::rc::Rc;
        let call = |app: &mut PhotocraftApp, method: &str, params: serde_json::Value| {
            let (req, _rx) = ControlRequest::new(method, params);
            match handle(app, &egui::Context::default(), &req) {
                Outcome::Done(v) => v,
                _ => panic!("{method}: expected an immediate reply"),
            }
        };

        // engine.execute of a command that edits the document.
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        assert_eq!(height(&session_layer(&app, id)), 180.0, "not applied yet");
        let r = call(&mut app, "engine.execute", json!({"command": "layer.setProps", "params": {"layer": id.0, "opacity": 0.5}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(height(&session_layer(&app, id)), 260.0, "applied before the command ran, not lost as stale");
        assert!(app.type_box_preview.is_none());

        // app.save writes the document as it is, which has to hold the dragged box.
        let saved: Rc<RefCell<Option<Document>>> = Rc::default();
        let s = saved.clone();
        let services = crate::Services {
            export: Some(Box::new(move |d: &Document, _p: &str, _s: &crate::ExportSettings| {
                *s.borrow_mut() = Some(d.clone());
                Ok((b"out".to_vec(), Vec::new()))
            })),
            automation_write: Some(Box::new(|_path: &str, _b: &[u8]| Ok(()))),
            ..Default::default()
        };
        let (mut app, id) = app_with_box_and(services);
        background(&mut app);
        press(&mut app);
        to(&mut app, 560.0);
        let r = call(&mut app, "app.save", json!({"path": "a.png"}));
        assert_eq!(r["ok"], true, "{r}");
        let doc = saved.borrow();
        let doc = doc.as_ref().expect("the document was exported");
        assert_eq!(height(text_of(doc, id).unwrap()), 260.0, "the saved layer has the dragged box");
        assert!(app.type_box_preview.is_none());
    }

    /// `window.nextDocument` switches documents: it must not leave a drag going, or the new
    /// canvas would move the old document's box with its own view's coordinates.
    #[test]
    fn switching_documents_mid_drag_ends_the_drag() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.session.set_active(0);
        press(&mut app);
        to(&mut app, 560.0);
        crate::menus::invoke(&mut app, &ctx, "window.nextDocument", json!({})).unwrap();
        assert!(app.type_box_preview.is_none(), "the drag ended");
        assert!(app.ui.text_edit.as_ref().is_none_or(|e| e.resize.is_none() && !e.dragging));
        let first = &app.session.documents()[0].doc;
        assert_eq!(height(text_of(first, id).unwrap()), 260.0, "the first document kept the dragged box");
    }

    /// `view.clearSelectedGuides` is a view command that edits the document. It must not leave
    /// the drag going: its edit would make the preview stale and the dragged box would be lost.
    #[test]
    fn a_view_command_that_edits_the_document_ends_the_drag_first() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let ctx = egui::Context::default();
        press(&mut app);
        to(&mut app, 560.0);
        assert_eq!(height(&session_layer(&app, id)), 180.0, "not applied yet");
        let _ = crate::menus::invoke(&mut app, &ctx, "view.clearSelectedGuides", json!({}));
        assert!(app.type_box_preview.is_none(), "the drag ended");
        assert_eq!(height(&session_layer(&app, id)), 260.0, "the box was applied, not lost");
    }

    /// `edit.freeTransform` is a shell command (in `UI_COMMANDS`), yet it captures the document,
    /// so it must see the dragged box like any other command does: its frame is the resized
    /// layer's, not the one from before the drag.
    #[test]
    fn free_transform_mid_drag_sees_the_dragged_box() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        let ctx = egui::Context::default();
        let frame = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(id).unwrap());
            [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64]
        };
        let before = frame(&app);
        // A narrower box wraps the text differently, so the layer's frame changes with it.
        press_right(&mut app);
        to_x(&mut app, 420.0);
        assert_eq!(width(&session_layer(&app, id)), 240.0, "not applied yet");
        crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).expect("Free Transform opens");
        assert_eq!(width(&session_layer(&app, id)), 120.0, "applied before Free Transform captured the document");
        assert!(app.type_box_preview.is_none());
        let after = frame(&app);
        assert_ne!(after, before, "the resized text has another frame");
        let t = app.ui.transform.as_ref().expect("a Free Transform session");
        assert_eq!(t.rect, after, "its frame is the resized layer's");
    }

    /// If the document changed during the drag, the press-time box is never applied over it.
    #[test]
    fn a_stale_preview_ends_the_drag_instead_of_applying_the_press_time_box() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        press(&mut app);
        to(&mut app, 520.0);
        app.run("layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
        let (rev, steps) = (revision(&app), app.session.active().unwrap().history.past_len());
        to(&mut app, 560.0);
        assert_eq!(revision(&app), rev, "the move applied nothing over the other edit");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(height(&session_layer(&app, id)), 180.0);
        assert!(app.type_box_preview.is_none());
        let e = app.ui.text_edit.as_ref().unwrap();
        assert!(e.resize.is_none() && !e.dragging, "the gesture ended");
        // The rest of the gesture does nothing either.
        to(&mut app, 600.0);
        release(&mut app, 600.0);
        assert_eq!(revision(&app), rev);
    }

    /// Type Mask edits drag synchronously, so their overlay matches the box.
    #[test]
    fn a_type_mask_edit_keeps_the_synchronous_drag() {
        let (mut app, id) = app_with_box();
        background(&mut app);
        app.ui.text_edit.as_mut().unwrap().mask = true;
        let rev = revision(&app);
        press(&mut app);
        assert!(app.type_box_preview.is_none(), "no background drag for a mask edit");
        to(&mut app, 520.0);
        assert_eq!(revision(&app), rev + 1, "the move was applied");
        assert_eq!(height(&session_layer(&app, id)), 220.0);
        release(&mut app, 520.0);
    }
}
