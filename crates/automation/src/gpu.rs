//! Native GPU renderer adapter and session integration for headless automation.

use std::sync::Arc;

use photocraft_doc::{DocId, Document, Rect};
use photocraft_gpu::context::{NativeContext, NativeContextOptions};
use photocraft_gpu::{Compositor, readback};
use photocraft_io::{BandRenderer, IoError, RenderBand, RenderError};

use crate::AutomationError;

/// Headless GPU renderer implementing [`BandRenderer`] with device and compositor reuse.
pub struct GpuRenderer {
    context: Option<Arc<NativeContext>>,
    compositor: Option<Compositor>,
    last_doc_id: Option<DocId>,
    options: NativeContextOptions,
    max_staging_bytes: usize,
    init_error: Option<String>,
    diagnostics: Vec<String>,
    adapter_reported: bool,
}

impl GpuRenderer {
    /// Create and immediately initialize a GPU renderer.
    pub fn new() -> Result<Self, AutomationError> {
        let mut r = Self::lazy();
        r.ensure_init().map_err(|e| AutomationError::Other(format!("failed to initialize GPU renderer: {e}")))?;
        Ok(r)
    }

    /// Create a GPU renderer that lazily requests an adapter on first render.
    pub fn lazy() -> Self {
        Self {
            context: None,
            compositor: None,
            last_doc_id: None,
            options: NativeContextOptions::default(),
            max_staging_bytes: readback::DEFAULT_MAX_STAGING_BYTES,
            init_error: None,
            diagnostics: Vec::new(),
            adapter_reported: false,
        }
    }

    /// Create with custom [`NativeContextOptions`].
    pub fn with_options(options: NativeContextOptions) -> Self {
        Self {
            context: None,
            compositor: None,
            last_doc_id: None,
            options,
            max_staging_bytes: readback::DEFAULT_MAX_STAGING_BYTES,
            init_error: None,
            diagnostics: Vec::new(),
            adapter_reported: false,
        }
    }

    /// Set maximum in-flight staging bytes for bounded readback.
    pub fn set_max_staging_bytes(&mut self, bytes: usize) {
        self.max_staging_bytes = bytes;
    }

    fn push_diag(&mut self, msg: String) {
        const MAX_DIAGNOSTICS: usize = 16;
        if self.diagnostics.len() < MAX_DIAGNOSTICS && !self.diagnostics.contains(&msg) {
            self.diagnostics.push(msg);
        }
    }

    fn ensure_init(&mut self) -> Result<(), RenderError> {
        if let Some(err) = &self.init_error {
            return Err(RenderError::Unavailable(err.clone()));
        }

        if self.context.is_none() {
            match NativeContext::request(&self.options) {
                Ok(ctx) => match ctx.create_compositor() {
                    Ok(comp) => {
                        if !self.adapter_reported {
                            self.adapter_reported = true;
                            let is_software = ctx.is_software();
                            let adapter_type = if is_software { "software rasterizer" } else { "hardware" };
                            let msg = format!("GPU adapter: {} ({:?}, {adapter_type})", ctx.adapter_name(), ctx.backend());
                            self.push_diag(msg);
                        }
                        self.context = Some(Arc::new(ctx));
                        self.compositor = Some(comp);
                    }
                    Err(e) => {
                        let msg = format!("could not build GPU compositor: {e}");
                        self.push_diag(msg.clone());
                        self.init_error = Some(msg.clone());
                        return Err(RenderError::Unavailable(msg));
                    }
                },
                Err(e) => {
                    let msg = format!("no GPU adapter available: {e}");
                    self.push_diag(msg.clone());
                    self.init_error = Some(msg.clone());
                    return Err(RenderError::Unavailable(msg));
                }
            }
        }
        Ok(())
    }

    /// The underlying GPU context if initialized.
    pub fn context(&self) -> Option<&Arc<NativeContext>> {
        self.context.as_ref()
    }

    /// Whether this renderer is healthy.
    pub fn is_healthy(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.is_healthy())
    }

    /// Whether the active adapter is a software rasterizer.
    pub fn is_software(&self) -> bool {
        self.context.as_ref().is_some_and(|c| c.is_software())
    }

    /// Adapter name reported by the driver.
    pub fn adapter_name(&self) -> Option<&str> {
        self.context.as_ref().map(|c| c.adapter_name())
    }

    /// Invalidate compositor caches for a closed or modified document.
    pub fn forget_doc(&mut self, doc: DocId) {
        if let Some(comp) = &mut self.compositor {
            comp.forget_doc(doc);
        }
        if self.last_doc_id == Some(doc) {
            self.last_doc_id = None;
        }
    }

    /// Invalidate all compositor caches (resident pages, effect maps, pattern textures).
    pub fn reset_cache(&mut self) {
        if let Some(comp) = &mut self.compositor {
            comp.reset_cache();
        }
        self.last_doc_id = None;
    }
}

impl BandRenderer for GpuRenderer {
    fn render_bands(
        &mut self,
        doc: &Document,
        rect: Rect,
        band_rows: i32,
        ctl: &photocraft_raster::Interrupt<'_>,
        sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
    ) -> Result<(), RenderError> {
        let max_staging = self.max_staging_bytes;
        self.ensure_init()?;

        let ctx = self.context.clone().ok_or_else(|| RenderError::Unavailable("GPU context missing".into()))?;
        if !ctx.is_healthy() {
            let fault = ctx.health().fault().map_or_else(|| "GPU device lost".into(), |f| f.to_string());
            return Err(RenderError::Device(fault));
        }

        let comp = self.compositor.as_mut().ok_or_else(|| RenderError::Unavailable("GPU compositor missing".into()))?;
        if let Err(unsupported) = comp.supports(doc) {
            self.push_diag(format!("GPU rendering unsupported for document: {}", unsupported.0));
            return Err(RenderError::Unsupported(unsupported.0));
        }

        if self.last_doc_id != Some(doc.id) {
            if let Some(prev) = self.last_doc_id {
                comp.forget_doc(prev);
            }
            self.last_doc_id = Some(doc.id);
        }

        let device = ctx.device();
        let queue = ctx.queue();

        readback::render_bands(comp, device, queue, doc, rect, band_rows, max_staging, ctl, |band_rect, pixels| sink(RenderBand { rect: band_rect, pixels }))
            .map_err(|e| {
                let render_err = match e {
                    readback::ReadbackError::Unsupported(s) => RenderError::Unsupported(s),
                    readback::ReadbackError::DeviceLost(s) => RenderError::Device(s),
                    readback::ReadbackError::AllocationFailed(s) => RenderError::Unavailable(s),
                    readback::ReadbackError::MappingFailed(s) => RenderError::Device(s),
                    readback::ReadbackError::Cancelled => RenderError::Cancelled,
                    readback::ReadbackError::Execution(s) => RenderError::Execution(s),
                    readback::ReadbackError::Sink(io) => RenderError::sink(io),
                };
                if render_err.is_retryable() {
                    self.push_diag(format!("GPU readback error: {render_err}"));
                }
                render_err
            })
    }

    fn drain_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn push_diagnostic(&mut self, diagnostic: String) {
        self.push_diag(diagnostic);
    }

    fn forget_document(&mut self, doc: DocId) {
        self.forget_doc(doc);
    }
}
