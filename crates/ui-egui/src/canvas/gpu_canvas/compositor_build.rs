//! Builds the compositor without blocking the UI's first document refresh.
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) enum BuildState {
    #[default]
    NotStarted,
    Building,
    Ready(Box<photocraft_gpu::Compositor>),
    Failed(photocraft_gpu::Unsupported),
    Adopted,
}

pub(super) type Slot = Arc<Mutex<BuildState>>;
pub(super) type Completion = Arc<Mutex<Option<Result<(), (String, bool)>>>>;

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn start(device: &eframe::wgpu::Device, format: eframe::wgpu::TextureFormat, ctx: &egui::Context, slot: &Slot, completion: &Completion) {
    let mut state = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !matches!(*state, BuildState::NotStarted) {
        return;
    }
    *state = BuildState::Building;
    let (device, ctx, out) = (device.clone(), ctx.clone(), slot.clone());
    let worker_completion = completion.clone();
    let spawned = std::thread::Builder::new().name("gpu-compositor-build".into()).spawn(move || {
        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            photocraft_gpu::Compositor::try_new_with_format_threads_classified(&device, format, threads)
        }));
        let (result, outcome) = match result {
            Ok(Ok(comp)) => (BuildState::Ready(Box::new(comp)), Ok(())),
            Ok(Err(error)) => {
                let deterministic = error.kind == photocraft_gpu::BuildFailureKind::DeterministicShaderCompilation;
                (BuildState::Failed(photocraft_gpu::Unsupported(error.message.clone())), Err((error.message, deterministic)))
            }
            Err(_) => (
                BuildState::Failed(photocraft_gpu::Unsupported("building the GPU compositor panicked".into())),
                Err(("building the GPU compositor panicked".into(), false)),
            ),
        };
        *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = result;
        *worker_completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        ctx.request_repaint();
    });
    if let Err(error) = spawned {
        record_spawn_failure(&mut state, completion, &error);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn record_spawn_failure(state: &mut BuildState, completion: &Completion, error: &std::io::Error) {
    let message = format!("failed to spawn GPU compositor build thread: {error}");
    *state = BuildState::Failed(photocraft_gpu::Unsupported(message.clone()));
    *completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Err((message, false)));
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn spawn_failure_is_reported_as_retryable() {
        let completion = super::Completion::default();
        let mut state = super::BuildState::Building;
        super::record_spawn_failure(&mut state, &completion, &std::io::Error::other("resource exhausted"));
        assert!(matches!(state, super::BuildState::Failed(_)));
        let outcome = completion.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        assert_eq!(outcome, Some(Err(("failed to spawn GPU compositor build thread: resource exhausted".into(), false))));
    }

    #[test]
    fn compositor_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<photocraft_gpu::Compositor>();
    }
}
