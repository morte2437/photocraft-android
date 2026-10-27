use super::consts::STRIDE;
use super::textures::Cell;
use super::*;

/// Document pixels of page cell `c`.
pub(crate) fn cell_rect(c: Cell, page: u32) -> Rect {
    let p = page as i32;
    Rect::new(c.0.saturating_mul(p), c.1.saturating_mul(p), c.0.saturating_add(1).saturating_mul(p), c.1.saturating_add(1).saturating_mul(p))
}

/// `r` cut along the grid of `step`-sized squares (row-major).
pub(crate) fn grid_rects(r: Rect, step: u32) -> Vec<Rect> {
    let s = step.max(1) as i32;
    let mut out = Vec::new();
    let mut y = r.y0.div_euclid(s) * s;
    while y < r.y1 {
        let mut x = r.x0.div_euclid(s) * s;
        while x < r.x1 {
            out.push(Rect::new(x, y, x.saturating_add(s), y.saturating_add(s)).intersect(&r));
            x = x.saturating_add(s);
        }
        y = y.saturating_add(s);
    }
    out
}

/// Number of rectangles `grid_rects` would produce, without allocating them during preflight.
pub(crate) fn grid_rect_count(r: Rect, step: u32) -> Result<usize, Unsupported> {
    if r.is_empty() {
        return Ok(0);
    }
    let s = i64::from(step.max(1));
    let axis = |lo: i32, hi: i32| usize::try_from((i64::from(hi) - 1).div_euclid(s) - i64::from(lo).div_euclid(s) + 1).ok();
    axis(r.x0, r.x1).and_then(|x| axis(r.y0, r.y1).and_then(|y| x.checked_mul(y))).ok_or_else(|| Unsupported("GPU compositor chunk count is too large".into()))
}

/// Host length and GPU capacity for the chunk/pass uniform records, including their u32
/// dynamic binding offsets. Decline unsupported stacks before an invalid buffer faults the device.
pub(crate) fn uniform_buffer_len(chunks: usize, passes: usize, limit: u64) -> Result<usize, Unsupported> {
    let too_large = || Unsupported("GPU compositor uniform buffer is too large".into());
    let records = chunks.checked_add(passes).and_then(|n| u64::try_from(n).ok()).ok_or_else(too_large)?;
    let bytes = records.checked_mul(STRIDE).ok_or_else(too_large)?;
    if bytes > limit {
        return Err(Unsupported(format!("GPU compositor uniform buffer requires {bytes} bytes; device limit is {limit}")));
    }
    if bytes.saturating_sub(STRIDE) > u64::from(u32::MAX) {
        return Err(Unsupported("GPU compositor uniform offsets exceed u32".into()));
    }
    usize::try_from(bytes).map_err(|_| too_large())
}

/// The reach up to which each distance field must be exact (the largest any program asks for).
pub(crate) fn field_reaches(progs: &[fx::MapProgram]) -> HashMap<FieldKind, i32> {
    let mut want: HashMap<FieldKind, i32> = HashMap::new();
    for (k, r) in progs.iter().flat_map(|p| p.fields.iter()) {
        let w = want.entry(*k).or_insert(0);
        *w = (*w).max(*r);
    }
    want
}

/// Why a refresh whose layer pages don't fit GPU memory is drawn on the CPU.
pub const OVER_BUDGET: &str = "layers exceed the GPU memory budget; full refresh on the CPU";

/// Which of `pages` (LRU clock, key, bytes) to drop, least recently used first, so `incoming`
/// more bytes fit `budget` on top of the `total` held now. Callers leave out pages that must stay.
pub(crate) fn lru_victims<K>(mut pages: Vec<(u64, K, u64)>, total: u64, incoming: u64, budget: u64) -> Vec<K> {
    pages.sort_unstable_by_key(|p| p.0);
    let mut total = total;
    let mut out = Vec::new();
    for (_, k, bytes) in pages {
        if total.saturating_add(incoming) <= budget {
            break;
        }
        total = total.saturating_sub(bytes);
        out.push(k);
    }
    out
}

/// Runs `f` inside GPU error scopes and returns the first error raised.
/// Native wgpu reports validation/internal/OOM errors synchronously; where they only arrive
/// asynchronously (WebGPU) none is reported.
pub(crate) fn first_error<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> (T, Option<wgpu::Error>) {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let out = f();
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut error = None;
    // Scopes pop in reverse order.
    for pop in [memory.pop(), internal.pop(), validation.pop()] {
        if let std::task::Poll::Ready(Some(e)) = std::pin::pin!(pop).poll(&mut cx) {
            error.get_or_insert(e);
        }
    }
    (out, error)
}

/// Runs `f` inside the error scopes of [`first_error`], turning a reported error into
/// [`Unsupported`] instead of letting it escape to wgpu's panic handler.
///
/// Every native caller needs this, watched or not: a validation error (writing into a destroyed
/// or too-small buffer, say) is raised synchronously and never reaches the health callbacks, so a
/// watched compositor still panicked on it. WebGPU reports its errors asynchronously, long after
/// the scope has popped, so wasm relies on the health callbacks alone.
///
/// A scope also swallows the error that [`DeviceHealth`]'s uncaptured-error handler would have
/// recorded, so a reported error is handed back to `health`: a watched device must stop being
/// used and let the caller fall back to the CPU compositor.
pub(crate) fn scoped_work<T>(device: &wgpu::Device, health: Option<DeviceHealth>, f: impl FnOnce() -> Result<T, Unsupported>) -> Result<T, Unsupported> {
    // WebGPU errors arrive asynchronously; its callers still use the health callbacks.
    if cfg!(target_arch = "wasm32") {
        return f();
    }
    let (result, error) = first_error(device, f);
    match error {
        Some(error) => {
            if let Some(health) = health.as_ref() {
                health.caught(device, &error);
            }
            Err(Unsupported(format!("GPU work failed: {error}")))
        }
        None => result,
    }
}
