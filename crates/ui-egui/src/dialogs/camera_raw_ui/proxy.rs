#[cfg(not(target_arch = "wasm32"))]
use super::PARALLEL_MIN_PIXELS;
use super::open::raw_stage;
use super::{CameraRaw, CameraRawDialog, RgbHistogram};

/// Everything one develop of the proxy reads, owned, so it can run off the UI thread.
pub(super) struct ProxyJob {
    pub(super) proxy: std::sync::Arc<Vec<[f32; 4]>>,
    pub(super) selection: Option<std::sync::Arc<Vec<f32>>>,
    pub(super) raw_curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    pub(super) params: CameraRaw,
    pub(super) float: bool,
    pub(super) size: (usize, usize),
    pub(super) full_w: usize,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) gpu: Option<crate::camera_raw_gpu::CameraRawGpu>,
    pub(super) source_key: u64,
}

/// A developed proxy, ready to show.
pub(super) struct ProxyFrame {
    pub(super) px: Vec<[f32; 4]>,
    pub(super) histogram: RgbHistogram,
    pub(super) histogram_ms: f64,
    pub(super) image: egui::ColorImage,
    pub(super) display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
    pub(super) ms: f64,
    pub(super) develop_ms: f64,
    pub(super) backend: &'static str,
}

impl ProxyJob {
    pub(super) fn run(self) -> ProxyFrame {
        let t0 = crate::gpu_canvas::now_ms();
        let (pw, ph) = self.size;
        let mut p = self.params;
        p.pixel_scale = (pw as f32 / self.full_w.max(1) as f32).min(1.0);
        // The open-time dialog re-tones the proxy for each job; otherwise the GPU reads the
        // proxy as it is and uploads it once.
        let retoned = self.raw_curve.as_ref().map(|curve| {
            let mut px = self.proxy.to_vec();
            photocraft_io::raw::retone(&mut px, pw, ph, curve, raw_stage(&p));
            px
        });
        if retoned.is_some() {
            (p.shadows, p.highlights, p.clarity, p.texture, p.dehaze) = (0.0, 0.0, 0.0, 0.0, 0.0);
        }
        let develop_start = crate::gpu_canvas::now_ms();
        let key = retoned.is_none().then_some(self.source_key);
        let input = retoned.as_deref().unwrap_or(&self.proxy);
        let (mut px, backend) = match self.gpu.as_ref().and_then(|g| g.develop(input, key, pw, ph, &p, self.float)) {
            Some(out) => (out, "gpu"),
            None => {
                let mut px = retoned.unwrap_or_else(|| self.proxy.to_vec());
                photocraft_algo::camera_raw::develop(&mut px, pw, ph, &p, self.float);
                (px, "cpu")
            }
        };
        let develop_ms = crate::gpu_canvas::now_ms() - develop_start;
        if let Some(mask) = &self.selection {
            let blend = |(out, (original, coverage)): (&mut [f32; 4], (&[f32; 4], &f32))| {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            };
            // Each pixel is independent, so bands of the proxy blend in parallel (#211, P36).
            #[cfg(not(target_arch = "wasm32"))]
            {
                use rayon::prelude::*;
                px.par_iter_mut().zip(self.proxy.par_iter().zip(mask.par_iter())).with_min_len(PARALLEL_MIN_PIXELS).for_each(blend);
            }
            #[cfg(target_arch = "wasm32")]
            px.iter_mut().zip(self.proxy.iter().zip(mask.iter())).for_each(blend);
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        let histogram = RgbHistogram::from_rgba(&px);
        let histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        let image = CameraRawDialog::image(&px, pw, ph, &self.display);
        ProxyFrame { px, histogram, histogram_ms, image, display: self.display, ms: crate::gpu_canvas::now_ms() - t0, develop_ms, backend }
    }
}
