//! A failed renderer-aware save must never replace the previous file, and closing a document
//! must release its renderer caches.

use std::fs;
use std::sync::{Arc, Mutex};

use photocraft_automation::workspace::AuthorizedWorkspace;
use photocraft_automation::{Headless, files};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{DocId, Document, Rect, Size};
use photocraft_io::{BandRenderer, CpuRenderer, ExportOptions, IoError, RenderBand, RenderError};

struct FailingRenderer;

impl BandRenderer for FailingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        _rect: Rect,
        _band_rows: i32,
        _ctl: &photocraft_raster::Interrupt<'_>,
        _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        Err(RenderError::Execution("forced renderer failure".into()))
    }
}

#[test]
fn failed_renderer_save_preserves_prior_file() {
    let dir = std::env::temp_dir().join(format!("photocraft-renderer-save-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("existing.png");
    let original = b"prior file contents that must survive a failed save";
    fs::write(&target, original).unwrap();

    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);

    let mut renderer = FailingRenderer;
    let res = files::save_with_renderer(&doc, &target, Some("png"), &ExportOptions::default(), None, Some(&mut renderer));
    assert!(res.is_err(), "the renderer failure must fail the save: {res:?}");
    assert_eq!(fs::read(&target).unwrap(), original, "a failed save must leave the previous file intact");
    let _ = fs::remove_dir_all(&dir);
}

struct ForgetRecordingRenderer {
    forgotten: Arc<Mutex<Vec<DocId>>>,
}

impl BandRenderer for ForgetRecordingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        _rect: Rect,
        _band_rows: i32,
        _ctl: &photocraft_raster::Interrupt<'_>,
        _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        Ok(())
    }

    fn forget_document(&mut self, doc: DocId) {
        self.forgotten.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(doc);
    }
}

#[test]
fn closing_a_document_forgets_its_renderer_caches() {
    let forgotten = Arc::new(Mutex::new(Vec::new()));
    let mut h = Headless::trusted_local();
    h.session.open_document(Document::with_background("close", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    let id = h.session.documents()[0].doc.id;
    h.set_renderer(Some(Box::new(ForgetRecordingRenderer { forgotten: forgotten.clone() })));

    h.close(Some(0)).expect("close document");

    let forgotten = forgotten.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert_eq!(forgotten, vec![id], "closing a document must release its renderer caches");
}

#[test]
fn closing_a_document_through_a_command_forgets_its_renderer_caches() {
    let forgotten = Arc::new(Mutex::new(Vec::new()));
    let mut h = Headless::trusted_local();
    for name in ["first", "second"] {
        h.session.open_document(Document::with_background(name, Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    }
    let closed = h.session.documents()[0].doc.id;
    h.set_renderer(Some(Box::new(ForgetRecordingRenderer { forgotten: forgotten.clone() })));

    // `file.close` removes the document through the session, not `Headless::close`.
    h.command_run("file.close", serde_json::json!({"document": 0})).expect("close command");

    let forgotten = forgotten.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    assert_eq!(forgotten, vec![closed], "a command that closes a document must release its renderer caches");
}

struct DiagnosingRenderer {
    diagnostics: Vec<String>,
}

impl BandRenderer for DiagnosingRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &photocraft_raster::Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        self.push_diagnostic("renderer diagnostic survives a failed write".into());
        let mut cpu = CpuRenderer;
        cpu.render_bands(doc, rect, band_rows, ctl, sink)
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        if !self.diagnostics.contains(&diagnostic) {
            self.diagnostics.push(diagnostic);
        }
    }
}

#[test]
fn failed_atomic_write_keeps_renderer_diagnostics_for_the_caller() {
    let dir = std::env::temp_dir().join(format!("photocraft-renderer-write-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // A directory at the destination makes the final atomic rename fail after encoding.
    let target = dir.join("target.png");
    fs::create_dir(&target).unwrap();

    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);

    let mut renderer = DiagnosingRenderer { diagnostics: Vec::new() };
    let res = files::save_with_renderer(&doc, &target, Some("png"), &ExportOptions::default(), None, Some(&mut renderer));
    assert!(res.is_err(), "a write over a directory must fail: {res:?}");
    let diagnostics = renderer.drain_diagnostics();
    assert!(diagnostics.iter().any(|d| d.contains("failed write")), "diagnostics drained before the failed write must be queued again: {diagnostics:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failed_workspace_write_keeps_export_diagnostics_for_the_caller() {
    let dir = std::env::temp_dir().join(format!("photocraft-workspace-save-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let root = dir.join("root");
    fs::create_dir_all(&root).unwrap();

    let workspace = AuthorizedWorkspace::new(Some(&root), Some(&root)).expect("authorized workspace");
    let mut h = Headless::with_workspace(workspace);
    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);
    h.session.open_document(doc, None);
    h.set_renderer(Some(Box::new(DiagnosingRenderer { diagnostics: Vec::new() })));

    // Outside the authorized root: the export succeeds, then its write is refused.
    let outside = dir.join("outside.png");
    let res = h.save(None, Some(&outside), Some("png"), &Default::default());
    assert!(res.is_err(), "a write outside the workspace must fail: {res:?}");
    let diagnostics = h.drain_diagnostics();
    assert!(
        diagnostics.iter().any(|d| d.contains("failed write")),
        "export diagnostics drained before the failed workspace write must be queued again: {diagnostics:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Overrides only `drain_diagnostics`: [`BandRenderer::push_diagnostic`] keeps its default no-op,
/// as most custom renderers leave it.
struct DrainOnlyRenderer;

impl BandRenderer for DrainOnlyRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &photocraft_raster::Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let mut cpu = CpuRenderer;
        cpu.render_bands(doc, rect, band_rows, ctl, sink)
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn failed_workspace_write_keeps_export_warnings_with_a_drain_only_renderer() {
    let dir = std::env::temp_dir().join(format!("photocraft-workspace-warnings-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let root = dir.join("root");
    fs::create_dir_all(&root).unwrap();

    let workspace = AuthorizedWorkspace::new(Some(&root), Some(&root)).expect("authorized workspace");
    let mut h = Headless::with_workspace(workspace);
    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);
    h.session.open_document(doc, None);
    h.set_renderer(Some(Box::new(DrainOnlyRenderer)));

    // Outside the authorized root: the export succeeds, then its write is refused.
    let outside = dir.join("outside.png");
    let res = h.save(None, Some(&outside), Some("png"), &Default::default());
    assert!(res.is_err(), "a write outside the workspace must fail: {res:?}");
    let diagnostics = h.drain_diagnostics();
    assert!(
        diagnostics.iter().any(|d| d.contains("flattened")),
        "the export warnings must survive a failed workspace write when the renderer ignores pushed diagnostics: {diagnostics:?}"
    );
    let again = h.drain_diagnostics();
    assert!(again.is_empty(), "the export warnings must drain exactly once: {again:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failed_local_write_keeps_export_warnings_with_a_drain_only_renderer() {
    let dir = std::env::temp_dir().join(format!("photocraft-local-warnings-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // A directory at the destination makes the final atomic rename fail after encoding.
    let target = dir.join("target.png");
    fs::create_dir(&target).unwrap();

    let mut h = Headless::trusted_local();
    let mut doc = Document::with_background("test", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut top = doc.layers[0].clone();
    top.name = "top".into();
    doc.layers.push(top);
    h.session.open_document(doc, None);
    h.set_renderer(Some(Box::new(DrainOnlyRenderer)));

    let res = h.save(None, Some(&target), Some("png"), &Default::default());
    assert!(res.is_err(), "a write over a directory must fail: {res:?}");
    let diagnostics = h.drain_diagnostics();
    assert!(
        diagnostics.iter().any(|d| d.contains("flattened")),
        "the export warnings must survive a failed local write when the renderer ignores pushed diagnostics: {diagnostics:?}"
    );
    assert!(h.drain_diagnostics().is_empty(), "the export warnings must drain exactly once");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scaled_preview_fallback_warning_is_drained_and_the_rpc_reply_carries_it() {
    let mut h = Headless::trusted_local();
    h.session.open_document(Document::with_background("preview", Size::new(64, 48), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    h.set_renderer(Some(Box::new(DrainOnlyRenderer)));

    // A scaled preview composes on the CPU; the warning must not depend on `push_diagnostic`.
    let png = h.render_png(None, 16).expect("scaled preview");
    assert!(png.starts_with(b"\x89PNG"));
    let diagnostics = h.drain_diagnostics();
    assert!(diagnostics.iter().any(|d| d.contains("scaled previews")), "preview fallback warning must be drained: {diagnostics:?}");

    let reply = h.handle("doc.render", serde_json::json!({"maxSide": 16})).expect("doc.render");
    assert!(reply.get("base64").is_some(), "existing reply fields stay: {reply}");
    let warnings = reply.get("warnings").and_then(|w| w.as_array()).expect("warnings array");
    assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.contains("scaled previews"))), "{reply}");
    assert!(h.drain_diagnostics().is_empty(), "the reply consumed the warnings");

    // No renderer, no fallback, no `warnings` key.
    let mut plain = Headless::trusted_local();
    plain.session.open_document(Document::with_background("plain", Size::new(64, 48), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    let reply = plain.handle("doc.render", serde_json::json!({"maxSide": 16})).expect("doc.render");
    assert!(reply.get("warnings").is_none(), "{reply}");
}
