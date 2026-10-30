use super::doc_textures::mip_count;
use super::wgpu;

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
/// it: DXGI's dedicated video memory on Windows, amdgpu's VRAM size or `nvidia-smi`'s report on
/// Linux. `None` for integrated and software adapters, whose textures live in system memory (the
/// RAM-based budgets already cover them), and where nothing reports it. `PHOTOCRAFT_VRAM_MB`
/// overrides it for any adapter, simulating a discrete card of that size: on an integrated GPU the
/// budget then stops charging the document's pixels too, as for a real card (a development switch).
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
    drm_video_memory(std::path::Path::new("/sys/class/drm"), vendor, device).or_else(|| nvidia_video_memory(vendor, device))
}

/// PCI vendor id of NVIDIA, whose proprietary Linux driver writes none of amdgpu's
/// `mem_info_vram_total` sysfs files (a card of its own still reads as [`None`] above): without
/// this, a discrete NVIDIA card was charged for the document's pixels like an integrated one,
/// `memory_budget` shrank to Memory Usage minus them, and a many-layer document whose pages then
/// missed that budget redrew whole-document edits on the CPU every time (P60/P61 on an NVIDIA
/// Linux box: 1.5 s where the fix's own benchmark measures tens of milliseconds elsewhere).
#[cfg(any(target_os = "linux", test))]
pub(super) const NVIDIA_VENDOR: u32 = 0x10de;

/// How long `nvidia-smi` gets to answer: it runs on the UI thread while the window opens, and a
/// driver that hangs the query must not keep PhotoCraft from starting.
#[cfg(target_os = "linux")]
const NVIDIA_SMI_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// NVIDIA's own memory size on Linux, from `nvidia-smi` (installed with the driver that put this
/// adapter there in the first place): absent, slower than [`NVIDIA_SMI_DEADLINE`] or odd output
/// all read as `None`, never a hang or a panic.
#[cfg(target_os = "linux")]
pub(super) fn nvidia_video_memory(vendor: u32, device: u32) -> Option<u64> {
    if vendor != NVIDIA_VENDOR {
        return None;
    }
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=pci.device_id,memory.total", "--format=csv,noheader,nounits"]);
    let out = bounded_output(cmd, NVIDIA_SMI_DEADLINE)?;
    parse_nvidia_smi(&String::from_utf8_lossy(&out), vendor, device)
}

/// The first 64 KiB of `cmd`'s stdout when it exits successfully within `deadline`; `None` when
/// it can't start, fails or is still running by then. A late one is killed and reaped on a thread
/// of its own (a process stuck in the driver may not die at once), so the caller waits about
/// `deadline` at most.
#[cfg(any(target_os = "linux", test))]
pub(super) fn bounded_output(mut cmd: std::process::Command, deadline: std::time::Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};
    fn abandon(mut child: Child) {
        let _ = child.kill();
        let _ = std::thread::Builder::new().name("reap".into()).spawn(move || child.wait());
    }
    let end = Instant::now().checked_add(deadline)?;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let Some(stdout) = child.stdout.take() else {
        abandon(child);
        return None;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new().name("child-stdout".into()).spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(64 << 10).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    if reader.is_err() {
        abandon(child);
        return None;
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                abandon(child);
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    // The child has exited, so its stdout is at EOF unless it handed the pipe on to a process of
    // its own: bounded too, rather than joining the reader.
    rx.recv_timeout(end.saturating_duration_since(Instant::now()).max(Duration::from_millis(50))).ok()
}

/// `nvidia-smi --query-gpu=pci.device_id,memory.total --format=csv,noheader,nounits`'s own
/// format: one `0xDDDDVVVV, size-in-MiB` line per card (device id the high half, vendor id the
/// low, as `lspci -n` and wgpu's own ids read them). The matching line's size, in bytes, and the
/// smallest when several cards share those ids (the ids can't tell which one wgpu picked, so the
/// budget assumes the smaller); `None` when no line names this `vendor`/`device`, or the output
/// isn't shaped like that at all.
#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_nvidia_smi(output: &str, vendor: u32, device: u32) -> Option<u64> {
    output
        .lines()
        .filter_map(|line| {
            let (id, mb) = line.split_once(',')?;
            let id = u32::from_str_radix(id.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
            if id & 0xffff != vendor || id >> 16 != device {
                return None;
            }
            mb.trim().parse::<u64>().ok()?.checked_mul(1 << 20).filter(|b| *b > 0)
        })
        .min()
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
