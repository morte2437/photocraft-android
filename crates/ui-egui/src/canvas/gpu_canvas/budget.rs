use super::doc_textures::mip_count;
use super::*;

/// Floor of the compositor's memory budget: enough for a viewport's pages of a dozen layers.
pub const MIN_GPU_BUDGET: u64 = 512 << 20;

/// Why a full refresh whose layer pages don't fit [`memory_budget`] (or GPU memory) goes to the
/// CPU compositor.
pub const OVER_BUDGET: &str = photocraft_gpu::OVER_BUDGET;

/// GPU memory the wgpu compositor may hold for layer pages and effect maps.
///
/// `fixed` (Preferences › Performance › GPU Memory, bytes) sets it when it isn't 0, within
/// [`vram_share`] of a discrete GPU's own memory (`vram`, from [`video_memory`]). Otherwise it is
/// Memory Usage (`allowance`) within a quarter of physical memory (`ram`; 16 GB assumed when
/// unknown) and the VRAM share. The document's pixels and History (`pixels`) come off Memory Usage
/// unless the card's own memory is known: a discrete card doesn't hold them, while integrated and
/// software adapters (and a card whose memory can't be read) share the system's. Never below
/// [`MIN_GPU_BUDGET`]. Layers beyond it redraw full refreshes on the CPU ([`OVER_BUDGET`]).
pub fn memory_budget(allowance: u64, pixels: u64, ram: Option<u64>, vram: Option<u64>, fixed: u64) -> u64 {
    let card = vram.map_or(u64::MAX, vram_share);
    if fixed > 0 {
        return fixed.min(card).max(MIN_GPU_BUDGET);
    }
    let ram = ram.unwrap_or(16 << 30);
    let charged = if vram.is_some() { 0 } else { pixels };
    allowance.saturating_sub(charged).min(ram / 4).min(card).max(MIN_GPU_BUDGET)
}

/// The share of a discrete GPU's memory each of [`memory_budget`] and [`canvas_budget`] may
/// plan for: three eighths, so the two together leave a quarter of the card to the window's
/// swapchain, egui and other applications instead of over-committing it and letting the driver
/// page (#4536).
pub fn vram_share(vram: u64) -> u64 {
    vram / 8 * 3
}

/// Why a document whose display textures don't fit [`canvas_budget`] is drawn on the CPU canvas.
pub const OVER_CANVAS_BUDGET: &str = "the canvas exceeds the GPU texture budget; drawing it on the CPU canvas";

/// [`Refresh::kind`] when the document's display textures don't fit [`canvas_budget`]: nothing
/// was created, and the caller draws the CPU canvas instead.
pub const REFRESH_OVER_BUDGET: &str = "over-budget";

/// [`Refresh::kind`] of a CPU refresh left to the caller ([`GpuCanvas::refresh_with`]).
pub const REFRESH_DEFERRED: &str = "deferred";

/// The smallest CPU refresh (pixels) [`GpuCanvas::refresh_with`] defers: below it, the
/// synchronous composite takes a fraction of a second even with many layers.
pub const DEFER_PX: u64 = 1 << 22;

/// GPU bytes one document's display texture set (every tile and mip level) may take: a quarter
/// of physical memory (`ram`; 16 GB assumed when unknown) and at most [`vram_share`] of a
/// discrete GPU's own memory (`vram`), at least [`MIN_GPU_BUDGET`]. The set covers the whole
/// canvas at full resolution, so its size follows the declared canvas, not the layer pixels; a
/// larger canvas is drawn on the (reduced) CPU canvas instead (#2934).
pub fn canvas_budget(ram: Option<u64>, vram: Option<u64>) -> u64 {
    (ram.unwrap_or(16 << 30) / 4).min(vram.map_or(u64::MAX, vram_share)).max(MIN_GPU_BUDGET)
}

/// GPU bytes of a display texture set for a `size` canvas in `tile`-sided tiles of `bpp` bytes
/// per texel, with every mip level (what [`GpuCanvas::texture_info`] reports once it exists).
/// Closed form over the (at most four) distinct tile shapes, so any size is cheap to price.
pub fn canvas_texture_bytes(size: [u32; 2], tile: u32, bpp: u64) -> u64 {
    let tile = tile.max(1);
    // (how many tiles, their side) along one axis: whole tiles, then the remainder.
    let spans = |n: u32| [(n / tile, tile), (u32::from(!n.is_multiple_of(tile)), n % tile)];
    let mut total = 0u64;
    for (nx, w) in spans(size[0]) {
        for (ny, h) in spans(size[1]) {
            if nx == 0 || ny == 0 {
                continue;
            }
            let one = (0..mip_count(w, h)).map(|l| u64::from((w >> l).max(1)) * u64::from((h >> l).max(1)) * bpp).fold(0, u64::saturating_add);
            total = total.saturating_add(one.saturating_mul(u64::from(nx) * u64::from(ny)));
        }
    }
    total
}

/// Physical memory of this machine in bytes, if known. `PHOTOCRAFT_RAM_MB` overrides it (to
/// simulate a smaller machine).
pub fn physical_memory() -> Option<u64> {
    static RAM: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *RAM.get_or_init(|| {
        if let Some(mb) = std::env::var("PHOTOCRAFT_RAM_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()) {
            return mb.checked_mul(1 << 20);
        }
        detect_physical_memory()
    })
}

#[cfg(target_os = "macos")]
fn detect_physical_memory() -> Option<u64> {
    let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn detect_physical_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

#[cfg(target_os = "freebsd")]
fn detect_physical_memory() -> Option<u64> {
    let out = std::process::Command::new("/sbin/sysctl").args(["-n", "hw.physmem"]).output().ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(windows)]
pub(super) fn detect_physical_memory() -> Option<u64> {
    winsafe::GlobalMemoryStatusEx().map(|m| m.ullTotalPhys).ok().filter(|b| *b > 0)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
fn detect_physical_memory() -> Option<u64> {
    None
}

/// The memory of `adapter` itself, in bytes, when it is a discrete GPU and the platform reports
/// it: DXGI's dedicated video memory on Windows, amdgpu's VRAM size on Linux. `None` for
/// integrated and software adapters, whose textures live in system memory (the RAM-based
/// budgets already cover them), and where nothing reports it. `PHOTOCRAFT_VRAM_MB` overrides it
/// for any adapter, simulating a discrete card of that size: on an integrated GPU the budget then
/// stops charging the document's pixels too, as for a real card (a development switch).
pub fn video_memory(adapter: &wgpu::AdapterInfo) -> Option<u64> {
    if let Some(mb) = std::env::var("PHOTOCRAFT_VRAM_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()) {
        return mb.checked_mul(1 << 20).filter(|b| *b > 0);
    }
    if adapter.device_type != wgpu::DeviceType::DiscreteGpu {
        return None;
    }
    // Detected once per adapter, as `physical_memory` is: on Windows each detection builds a
    // DXGI factory, and startup asks three times. Keyed by the adapter, since a GPU retry can
    // pick another one.
    /// (vendor, device) ids and the memory found for them.
    type Seen = Vec<((u32, u32), Option<u64>)>;
    static SEEN: std::sync::Mutex<Seen> = std::sync::Mutex::new(Vec::new());
    let key = (adapter.vendor, adapter.device);
    let mut seen = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, bytes)) = seen.iter().find(|(k, _)| *k == key) {
        return *bytes;
    }
    let bytes = detect_video_memory(key.0, key.1).filter(|b| *b > 0);
    seen.push((key, bytes));
    bytes
}

#[cfg(windows)]
pub(super) fn detect_video_memory(vendor: u32, device: u32) -> Option<u64> {
    use winsafe::prelude::*;
    let factory = winsafe::CreateDXGIFactory1().ok()?;
    factory
        .EnumAdapters1()
        .filter_map(|a| a.ok()?.GetDesc1().ok())
        .find(|d| d.VendorId == vendor && d.DeviceId == device)
        .and_then(|d| u64::try_from(d.DedicatedVideoMemory).ok())
}

#[cfg(target_os = "linux")]
fn detect_video_memory(vendor: u32, device: u32) -> Option<u64> {
    drm_video_memory(std::path::Path::new("/sys/class/drm"), vendor, device)
}

/// amdgpu's VRAM size (`<card>/device/mem_info_vram_total`, bytes) of the card under `drm` (the
/// sysfs `/sys/class/drm` folder) whose PCI `vendor` and `device` ids (hex files beside it) match.
#[cfg(any(target_os = "linux", test))]
pub(super) fn drm_video_memory(drm: &std::path::Path, vendor: u32, device: u32) -> Option<u64> {
    let hex = |p: std::path::PathBuf| u32::from_str_radix(std::fs::read_to_string(p).ok()?.trim().trim_start_matches("0x"), 16).ok();
    std::fs::read_dir(drm).ok()?.flatten().find_map(|card| {
        let dev = card.path().join("device");
        if hex(dev.join("vendor"))? != vendor || hex(dev.join("device"))? != device {
            return None;
        }
        std::fs::read_to_string(dev.join("mem_info_vram_total")).ok()?.trim().parse().ok()
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn detect_video_memory(_vendor: u32, _device: u32) -> Option<u64> {
    None
}
