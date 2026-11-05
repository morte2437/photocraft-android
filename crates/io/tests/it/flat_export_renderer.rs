//! Tests for custom [`BandRenderer`] injection, complete CPU retry, and renderer error handling.

use std::sync::atomic::{AtomicUsize, Ordering};

use photocraft_codecs::Format;
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{
    BandRenderer, CpuRenderer, ExportOptions, IoError, RenderBand, RenderError, document_to_image_with_renderer, export_flat, export_flat_with_renderer,
    export_png_streamed_with_renderer, export_preview_png_with_renderer,
};
use photocraft_raster::{Interrupt, Surface};

fn synthetic_doc() -> Document {
    let mut doc = Document::with_background("synth", Size::new(128, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(20, 20, 100, 260), &[0.1, 0.4, 0.9, 0.7]);
    let mut top = Layer::new("top", LayerContent::Raster(s));
    top.opacity = 0.8;
    top.blend = BlendMode::Multiply;
    doc.layers.push(top);
    doc
}

#[derive(Default)]
struct RecordingRenderer {
    diagnostics: Vec<String>,
    /// How many times `render_bands` was called.
    renders: usize,
}

impl BandRenderer for RecordingRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        self.renders += 1;
        let mut cpu = CpuRenderer;
        cpu.render_bands(doc, rect, band_rows, ctl, sink)
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        self.diagnostics.push(diagnostic);
    }
}

struct FailingRenderer {
    fail_after_bands: usize,
    call_count: AtomicUsize,
    error_kind: &'static str,
}

impl FailingRenderer {
    fn new(fail_after_bands: usize, error_kind: &'static str) -> Self {
        Self { fail_after_bands, call_count: AtomicUsize::new(0), error_kind }
    }
}

impl BandRenderer for FailingRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        _ctl: &Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let rows = if band_rows > 0 { band_rows } else { 32 };
        photocraft_compose::render_bands(doc, rect, rows, |buf| {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count >= self.fail_after_bands {
                return match self.error_kind {
                    "unsupported" => Err(IoError::Render(RenderError::Unsupported("forced unsupported feature".into()))),
                    "device" => Err(IoError::Render(RenderError::Device("forced device loss".into()))),
                    "cancelled" => Err(IoError::Cancelled),
                    _ => Err(IoError::Render(RenderError::Execution("forced failure".into()))),
                };
            }
            sink(RenderBand { rect: buf.rect, pixels: buf.into_px() })
        })
        .map_err(|e| match e {
            IoError::Render(r) => r,
            IoError::Cancelled => RenderError::Cancelled,
            other => RenderError::sink(other),
        })
    }
}

struct GeometryViolatingRenderer {
    violation: &'static str,
}

impl BandRenderer for GeometryViolatingRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        rect: Rect,
        _band_rows: i32,
        _ctl: &Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let (w, h) = (rect.width(), rect.height());
        match self.violation {
            "wrong_pixels" => {
                sink(RenderBand { rect: Rect::new(0, 0, w as i32, h as i32), pixels: vec![[0.0; 4]; (w as usize * h as usize).saturating_sub(10)] })
                    .map_err(RenderError::sink)
            }
            "out_of_order" => {
                // Yield band starting at y=10 instead of 0
                sink(RenderBand { rect: Rect::new(0, 10, w as i32, 20), pixels: vec![[0.0; 4]; w as usize * 10] }).map_err(RenderError::sink)
            }
            "wrong_width" => {
                sink(RenderBand { rect: Rect::new(0, 0, (w as i32).saturating_sub(5), 10), pixels: vec![[0.0; 4]; (w as usize).saturating_sub(5) * 10] })
                    .map_err(RenderError::sink)
            }
            _ => Ok(()),
        }
    }
}

#[test]
fn single_layer_and_cmyk_exports_report_that_the_renderer_was_not_used() {
    // A single plain layer takes the native copy path, never calling a renderer.
    let mut single = Document::with_background("single", Size::new(24, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    single.layers[0].name = "Background".into();
    let mut renderer = RecordingRenderer::default();
    let mut warnings = Vec::new();
    document_to_image_with_renderer(&single, &mut warnings, Some(&mut renderer)).expect("single-layer export");
    assert!(warnings.iter().any(|d| d.contains("single-layer") && d.contains("CPU")), "a requested renderer that is bypassed must report why: {warnings:?}");
    assert_eq!(renderer.renders, 0, "the native copy never calls the renderer");

    // A CMYK composite mixed on ink values likewise returns before the renderer branch.
    let mut cmyk = Document::with_background("cmyk", Size::new(32, 32), ColorMode::Cmyk, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(cmyk.pixel_format());
    s.fill_rect(Rect::new(4, 4, 28, 28), &[0.1, 0.2, 0.3, 0.4, 0.5]);
    cmyk.layers.push(Layer::new("ink", LayerContent::Raster(s)));
    let mut renderer = RecordingRenderer::default();
    let res = export_flat_with_renderer(&cmyk, Format::Tiff, &ExportOptions::default(), &Interrupt::NONE, Some(&mut renderer)).expect("CMYK TIFF export");
    assert!(
        res.warnings.iter().any(|w| w.contains("CMYK ink mixing") && w.contains("CPU")),
        "CMYK ink mixing must report its CPU fallback: {:?}",
        res.warnings
    );
}

struct FailOnceDiagnosingRenderer {
    failed: bool,
    calls: usize,
    diagnostics: Vec<String>,
}

impl BandRenderer for FailOnceDiagnosingRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        self.calls += 1;
        if !self.failed {
            self.failed = true;
            self.push_diagnostic("GPU readback error: forced device loss".into());
            return Err(RenderError::Device("forced device loss".into()));
        }
        let mut cpu = CpuRenderer;
        cpu.render_bands(doc, rect, band_rows, ctl, sink)
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        self.diagnostics.push(diagnostic);
    }
}

#[test]
fn fallback_diagnostics_stay_with_the_failed_export() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    let mut renderer = FailOnceDiagnosingRenderer { failed: false, calls: 0, diagnostics: Vec::new() };
    let first = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer)).expect("first export retries on CPU");
    assert!(first.warnings.iter().any(|w| w.contains("falling back to CPU")), "the failed export reports its fallback: {:?}", first.warnings);
    assert!(
        first.warnings.iter().any(|w| w.contains("GPU readback error")),
        "the failed attempt's queued diagnostics belong to its retry result: {:?}",
        first.warnings
    );

    let calls_before_second = renderer.calls;
    let second = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer)).expect("second export uses the GPU path");
    assert!(renderer.calls > calls_before_second, "the second export must call the renderer: {}", renderer.calls);
    assert!(!second.warnings.iter().any(|w| w.contains("falling back to CPU")), "a fallback warning must not carry into a later export: {:?}", second.warnings);
    assert!(
        !second.warnings.iter().any(|w| w.contains("GPU readback error")),
        "queued diagnostics must not carry into a later successful export: {:?}",
        second.warnings
    );
}

/// Composes bands through the reference compositor but never polls `ctl` itself, so a band only
/// reaches the export sink when the export checks cancellation. Counts the bands it forwarded.
#[derive(Default)]
struct UnpollingRenderer {
    bands: usize,
}

impl BandRenderer for UnpollingRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        _ctl: &Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let rows = if band_rows > 0 { band_rows } else { 32 };
        photocraft_compose::render_bands(doc, rect, rows, |buf| {
            sink(RenderBand { rect: buf.rect, pixels: buf.into_px() })?;
            self.bands += 1;
            Ok(())
        })
        .map_err(RenderError::sink)
    }
}

#[test]
fn cancellation_stops_the_cpu_band_sink() {
    let opts = ExportOptions::default();

    // RGB bands: cancellation is checked per band, so the second band is never assembled.
    let checks = AtomicUsize::new(0);
    let cancel = || checks.fetch_add(1, Ordering::SeqCst) + 1 >= 2;
    let mut renderer = UnpollingRenderer::default();
    let res = export_flat_with_renderer(&synthetic_doc(), Format::Tiff, &opts, &Interrupt::cancel_only(&cancel), Some(&mut renderer));
    assert!(matches!(res, Err(IoError::Cancelled)), "a cancelled RGB export must stop between bands: {res:?}");
    assert_eq!(renderer.bands, 1, "the band after a cancellation request must not be assembled");

    // CMYK inks mix in bands through the same sink: none is assembled once cancelled.
    let mut cmyk = Document::with_background("cmyk", Size::new(32, 32), ColorMode::Cmyk, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(cmyk.pixel_format());
    s.fill_rect(Rect::new(4, 4, 28, 28), &[0.1, 0.2, 0.3, 0.4, 0.5]);
    cmyk.layers.push(Layer::new("ink", LayerContent::Raster(s)));
    let mut renderer = UnpollingRenderer::default();
    let res = export_flat_with_renderer(&cmyk, Format::Tiff, &opts, &Interrupt::cancel_only(&|| true), Some(&mut renderer));
    assert!(matches!(res, Err(IoError::Cancelled)), "a cancelled CMYK export must stop before its first band: {res:?}");
    assert_eq!(renderer.bands, 0, "no CMYK band may be assembled when the export is already cancelled");
}

#[test]
fn cancelled_export_leaves_queued_diagnostics_on_the_renderer() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    let mut renderer = FailOnceDiagnosingRenderer { failed: false, calls: 0, diagnostics: Vec::new() };
    let cancelled = Interrupt::cancel_only(&|| true);
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &cancelled, Some(&mut renderer));
    assert!(matches!(res, Err(IoError::Cancelled)), "cancellation must be observed before the CPU retry: {res:?}");
    assert_eq!(renderer.calls, 1, "the cancelled export must not retry through the renderer: {}", renderer.calls);
    let diagnostics = renderer.drain_diagnostics();
    assert!(diagnostics.iter().any(|d| d.contains("GPU readback error")), "diagnostics must stay queued when the export is cancelled: {diagnostics:?}");
}

#[test]
fn ora_and_pdf_exports_report_cpu_use_when_a_renderer_is_set() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    for (name, marker) in [("out.ora", "OpenRaster export uses CPU compositor"), ("out.pdf", "PDF export uses CPU compositor")] {
        let mut renderer = RecordingRenderer::default();
        let res = photocraft_io::export_with_renderer(&doc, name, &opts, &Interrupt::NONE, Some(&mut renderer)).expect("CPU-only format export");
        assert!(res.warnings.iter().any(|w| w.contains(marker)), "{name} must report CPU use: {:?}", res.warnings);
    }
}

#[test]
fn custom_cpu_renderer_matches_default_export() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let def = export_flat(&doc, Format::Png, &opts).expect("default export");
    let mut custom = CpuRenderer;
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut custom)).expect("custom export");

    assert_eq!(def.bytes, res.bytes);
    assert_eq!(def.warnings, res.warnings);
}

#[test]
fn renderer_preview_is_8bit_rgba_while_saved_export_keeps_depth() {
    let mut doc = Document::with_background("preview16", Size::new(64, 48), ColorMode::Rgb, SampleType::U16, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(8, 8, 56, 40), &[0.2, 0.4, 0.8, 0.5]);
    doc.layers.push(Layer::new("top", LayerContent::Raster(s)));

    let mut preview_renderer = RecordingRenderer::default();
    let preview = export_preview_png_with_renderer(&doc, Some(&mut preview_renderer)).expect("preview export");
    assert!(preview_renderer.renders > 0, "the preview must render through the supplied renderer");
    let img = photocraft_codecs::decode(&preview.bytes).expect("decode preview");
    assert_eq!((img.width(), img.height()), (64, 48));
    assert_eq!(img.layout(), photocraft_codecs::ChannelLayout::Rgba, "a preview is always RGBA");
    assert_eq!(img.sample_type(), photocraft_codecs::SampleType::U8, "a preview is always 8-bit");

    let pixel = |x: u32, y: u32| {
        let at = ((y * img.width() + x) * 4) as usize;
        &img.data()[at..at + 4]
    };
    // Inside the 50 % blue layer: half the layer colour over white.
    let inside = pixel(32, 24);
    for (got, want) in inside.iter().zip([153u8, 179, 230, 255]) {
        assert!(got.abs_diff(want) <= 2, "preview compositing is wrong inside the layer: {inside:?}");
    }
    // Outside the layer: the white background.
    assert_eq!(pixel(2, 2), [255, 255, 255, 255], "preview must keep the document background");

    // A saved export keeps the document's native depth: the preview conversion is preview-only.
    let opts = ExportOptions::default();
    let mut save_renderer = CpuRenderer;
    let saved = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut save_renderer)).expect("saved export");
    let saved_img = photocraft_codecs::decode(&saved.bytes).expect("decode saved export");
    assert_eq!(saved_img.sample_type(), photocraft_codecs::SampleType::U16, "saved exports keep the document's depth");
}

#[test]
fn forced_unsupported_renderer_restarts_complete_export_on_cpu() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_flat(&doc, Format::Png, &opts).expect("baseline CPU export");

    let mut failing = FailingRenderer::new(0, "unsupported");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing)).expect("export should retry and succeed on CPU");

    assert_eq!(expected.bytes, res.bytes, "output after retry must match clean CPU output");
    assert!(
        res.warnings.iter().any(|w| w.contains("custom renderer unavailable") && w.contains("falling back to CPU")),
        "fallback warning must be reported: {:?}",
        res.warnings
    );
}

#[test]
fn forced_mid_band_failure_restarts_complete_export_on_cpu() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_flat(&doc, Format::Png, &opts).expect("baseline CPU export");

    // Fail on the second band (mid-render)
    let mut failing = FailingRenderer::new(1, "device");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing)).expect("export should restart completely on CPU");

    assert_eq!(expected.bytes, res.bytes, "output after mid-band restart must match pure CPU output");
    assert!(res.warnings.iter().any(|w| w.contains("falling back to CPU")), "fallback warning must be present: {:?}", res.warnings);
}

#[test]
fn cancellation_is_not_retried() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let mut failing = FailingRenderer::new(0, "cancelled");
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut failing));

    assert!(matches!(res, Err(IoError::Cancelled)), "cancelled render must not restart on CPU");
}

#[test]
fn encoder_and_sink_failures_are_not_retried_on_cpu() {
    struct SinkFailingRenderer;
    impl BandRenderer for SinkFailingRenderer {
        fn render_bands(
            &mut self,
            _doc: &Document,
            _rect: Rect,
            _band_rows: i32,
            _ctl: &Interrupt<'_>,
            _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
        ) -> Result<(), RenderError> {
            Err(RenderError::sink(IoError::Unsupported("test sink failure".into())))
        }
    }

    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    // A non-cancellation sink error with no interrupt: a CPU retry would succeed with `Ok`, so
    // this assertion fails if the error is ever retried on the CPU.
    let mut renderer = SinkFailingRenderer;
    let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer));
    assert!(matches!(res, Err(IoError::Unsupported(_))), "sink error must propagate without a CPU retry: {res:?}");
}

#[test]
fn band_geometry_violations_are_rejected() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    for violation in ["wrong_pixels", "out_of_order", "wrong_width"] {
        let mut renderer = GeometryViolatingRenderer { violation };
        let res = export_flat_with_renderer(&doc, Format::Png, &opts, &Interrupt::NONE, Some(&mut renderer));
        assert!(
            matches!(res, Err(IoError::Render(RenderError::Execution(_)))),
            "violation '{violation}' must be rejected before codec consumes corrupt data: {res:?}"
        );
    }
}

#[test]
fn streamed_png_retries_on_cpu_upon_renderer_failure() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    let expected = export_png_streamed_with_renderer(&doc, &opts, &Interrupt::NONE, 16, None).expect("baseline streamed PNG");

    let mut failing = FailingRenderer::new(1, "device");
    let res = export_png_streamed_with_renderer(&doc, &opts, &Interrupt::NONE, 16, Some(&mut failing)).expect("streamed PNG should retry on CPU");

    assert_eq!(expected.bytes, res.bytes);
    assert!(res.warnings.iter().any(|w| w.contains("falling back to CPU")));
}

#[test]
fn unknown_export_format_is_rejected() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();

    // `export` takes a name or bare extension; an unknown one fails before any codec runs.
    let export_err = photocraft_io::export(&doc, "invalid_unknown_extension_xyz", &opts);
    assert!(export_err.is_err());
}

/// A renderer that fails retryably on its first call and, as it does, requests cancellation, so
/// the complete CPU retry that follows cannot succeed. It queues pushed diagnostics for the test.
struct FailThenCancelRenderer {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    diagnostics: Vec<String>,
}

impl BandRenderer for FailThenCancelRenderer {
    fn render_bands(
        &mut self,
        _doc: &Document,
        _rect: Rect,
        _band_rows: i32,
        _ctl: &Interrupt<'_>,
        _sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        self.cancel.store(true, Ordering::SeqCst);
        Err(RenderError::Device("forced device loss".into()))
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        self.diagnostics.push(diagnostic);
    }
}

#[test]
fn a_failed_cpu_retry_keeps_the_fallback_reason_on_the_renderer() {
    let doc = synthetic_doc();
    let opts = ExportOptions::default();
    for streamed in [false, true] {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = flag.clone();
        let cancel = move || seen.load(Ordering::SeqCst);
        let ctl = Interrupt::cancel_only(&cancel);
        let mut renderer = FailThenCancelRenderer { cancel: flag, diagnostics: Vec::new() };
        let res = if streamed {
            export_png_streamed_with_renderer(&doc, &opts, &ctl, 0, Some(&mut renderer))
        } else {
            export_flat_with_renderer(&doc, Format::Png, &opts, &ctl, Some(&mut renderer))
        };
        assert!(matches!(res, Err(IoError::Cancelled)), "the CPU retry is cancelled (streamed: {streamed}): {res:?}");
        let diagnostics = renderer.drain_diagnostics();
        assert!(
            diagnostics.iter().any(|d| d.contains("custom renderer unavailable") && d.contains("falling back to CPU")),
            "the fallback reason must survive the failed retry (streamed: {streamed}): {diagnostics:?}"
        );
    }
}

#[test]
fn single_layer_notice_reaches_the_export_warnings_without_renderer_diagnostics() {
    // `FailingRenderer` keeps the default no-op `push_diagnostic`, like any renderer that ignores it.
    let single = Document::with_background("single", Size::new(24, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut renderer = FailingRenderer::new(usize::MAX, "device");
    let res = export_flat_with_renderer(&single, Format::Png, &ExportOptions::default(), &Interrupt::NONE, Some(&mut renderer)).expect("single-layer export");
    assert!(res.warnings.iter().any(|w| w.contains("single-layer") && w.contains("CPU")), "{:?}", res.warnings);
    let res = export_png_streamed_with_renderer(&single, &ExportOptions::default(), &Interrupt::NONE, 0, Some(&mut renderer)).expect("streamed single layer");
    assert!(res.warnings.iter().any(|w| w.contains("single-layer") && w.contains("CPU")), "{:?}", res.warnings);
}
