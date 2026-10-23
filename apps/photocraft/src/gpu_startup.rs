//! Crash-safe GPU startup (#4).
//!
//! A graphics driver that segfaults while the window's wgpu device is created (the Intel Vulkan
//! driver `igvk64.dll` on Windows) can't be caught in-process. So before the device exists the app
//! writes a *marker* (`gpu-starting.json` in the config directory: backend and, once chosen, the
//! adapter) and holds an OS file lock on it; it clears the marker once the first frames have
//! rendered. If the next launch finds a marker that no running process holds, the previous start
//! died inside the driver, and this one retries with the next safer backend:
//!
//! - Windows: Vulkan → DX12 → CPU
//! - Linux and other Unix: Vulkan → GL → CPU
//! - macOS: Metal → CPU
//!
//! The choice that worked is remembered in `performance.gpuBackend` (Preferences › Performance,
//! where it can be reset). With `auto`, Intel and Qualcomm (Windows on ARM) adapters on Windows
//! use DX12, and when DXGI lists an Intel adapter the instance leaves Vulkan out, so the first
//! start doesn't load the Intel Vulkan driver either. `WGPU_BACKEND` still overrides everything,
//! and `--safe-gpu` forces the CPU path for one launch.
//!
//! "CPU" composites on the CPU and draws the window with a software adapter where the platform
//! has one (WARP on Windows, llvmpipe over GL elsewhere). On Linux a GL adapter that can't present
//! (NVIDIA's EGL, which also hides llvmpipe) falls back to a Vulkan adapter for the window only
//! (#1337); before, CPU mode then couldn't start at all.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use eframe::{egui_wgpu, wgpu};
use photocraft_engine::prefs::{GpuBackend, RenderingMode};
use serde_json::{Value, json};

/// The marker's file name in the config directory.
pub const MARKER_FILE: &str = "gpu-starting.json";
/// PCI vendor id of Intel.
pub const INTEL: u32 = 0x8086;
/// Qualcomm's vendor id as its Vulkan driver reports it.
pub const QUALCOMM: u32 = 0x5143;
/// Qualcomm's vendor id as DXGI (DX12) reports it: "QCOM".
pub const QUALCOMM_DXGI: u32 = 0x4D4F_4351;
/// Largest marker read (a marker is a few hundred bytes).
const MARKER_MAX: u64 = 64 << 10;

/// Platform families with different backend chains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    /// Linux, the BSDs and other Unix.
    Other,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Other
        }
    }
}

/// What a start records before touching the driver.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marker {
    /// The plan's backend (`auto`, `dx12`, …), or `env:<value>` under `WGPU_BACKEND`.
    pub backend: String,
    /// The chosen adapter's name, backend and driver (empty until the adapter is chosen).
    pub adapter: String,
    pub adapter_backend: String,
    pub driver: String,
    pub version: String,
}

impl Marker {
    pub fn to_json(&self) -> String {
        json!({
            "backend": self.backend,
            "adapter": self.adapter,
            "adapterBackend": self.adapter_backend,
            "driver": self.driver,
            "version": self.version,
        })
        .to_string()
    }

    /// Parse a marker leniently: anything unreadable is a marker of an unknown backend.
    pub fn parse(text: &str) -> Self {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").chars().take(256).collect::<String>();
        Marker { backend: s("backend"), adapter: s("adapter"), adapter_backend: s("adapterBackend"), driver: s("driver"), version: s("version") }
    }

    /// The backend the crashed start was using: the adapter's when it got that far with `auto`,
    /// else the plan's. `None` for a start under `WGPU_BACKEND` (the user's own override).
    pub fn tried(&self) -> Option<GpuBackend> {
        let planned = if self.backend.is_empty() { Some(GpuBackend::Auto) } else { GpuBackend::parse(&self.backend) }?;
        match (planned, GpuBackend::parse(&self.adapter_backend)) {
            (GpuBackend::Auto, Some(actual)) => Some(actual),
            _ => Some(planned),
        }
    }
}

/// The backend this launch uses and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub backend: GpuBackend,
    /// `WGPU_BACKEND`, which picks the backends itself.
    pub env: Option<String>,
    /// Why this isn't simply the preference.
    pub reason: Option<String>,
    /// Store `backend` in the preferences once the start succeeded (a crash fallback).
    pub remember: bool,
}

/// A backend the platform has (others fall back to `auto`).
pub fn sanitize(pref: GpuBackend, os: Os) -> GpuBackend {
    use GpuBackend::*;
    match (os, pref) {
        (Os::Windows, Metal) | (Os::Mac, Vulkan | Dx12 | Gl) | (Os::Other, Dx12 | Metal) => Auto,
        (_, b) => b,
    }
}

/// The next safer backend after a start on `tried` crashed.
pub fn next_safer(tried: GpuBackend, os: Os) -> GpuBackend {
    use GpuBackend::*;
    match (os, tried) {
        (Os::Windows, Auto | Vulkan | Gl | Metal) => Dx12,
        (Os::Windows, Dx12 | Cpu) => Cpu,
        (Os::Mac, _) => Cpu,
        (Os::Other, Auto | Vulkan | Dx12 | Metal) => Gl,
        (Os::Other, Gl | Cpu) => Cpu,
    }
}

/// Whether a start that returned an error (so it didn't crash in the driver) keeps its marker, so
/// the next start tries a safer backend. Not when there is none: the CPU path is the end of every
/// chain, and a marker left there pinned every later start to the same failing path. Not under
/// `WGPU_BACKEND` either (no plan reads that marker).
pub fn keep_marker_after_error(plan: &Plan, os: Os) -> bool {
    plan.env.is_none() && next_safer(plan.backend, os) != plan.backend
}

/// Decide this launch's backend from the preference, a marker left by a start that crashed,
/// `WGPU_BACKEND` and `--safe-gpu`.
pub fn plan(pref: GpuBackend, crashed: Option<&Marker>, env: Option<&str>, safe_gpu: bool, os: Os) -> Plan {
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        let backend = if safe_gpu { GpuBackend::Cpu } else { GpuBackend::Auto };
        return Plan { backend, env: Some(v.to_string()), reason: Some(format!("WGPU_BACKEND={v} overrides the backend")), remember: false };
    }
    if safe_gpu {
        return Plan { backend: GpuBackend::Cpu, env: None, reason: Some("--safe-gpu: CPU renderer for this launch".into()), remember: false };
    }
    let pref = sanitize(pref, os);
    if let Some((m, tried)) = crashed.and_then(|m| Some((m, m.tried()?))) {
        let next = next_safer(tried, os);
        let on = if m.adapter.is_empty() { String::new() } else { format!(" ({})", m.adapter) };
        return Plan {
            backend: next,
            env: None,
            reason: Some(format!("the previous start didn't finish on {}{on}; using {}", tried.name(), next.name())),
            remember: next != pref,
        };
    }
    Plan { backend: pref, env: None, reason: None, remember: false }
}

/// Resolve the rendering policy before the window exists. Explicit GPU/Automatic selections
/// reset a legacy CPU backend to automatic; CPU keeps software-preferred window presentation.
pub fn plan_with_mode(pref: GpuBackend, mode: RenderingMode, crashed: Option<&Marker>, env: Option<&str>, safe_gpu: bool, os: Os) -> Plan {
    let backend = match mode {
        RenderingMode::Cpu => GpuBackend::Cpu,
        RenderingMode::Auto | RenderingMode::Gpu if pref == GpuBackend::Cpu => GpuBackend::Auto,
        _ => pref,
    };
    let env = if safe_gpu || mode == RenderingMode::Cpu { None } else { env };
    let crashed = if mode == RenderingMode::Cpu { None } else { crashed };
    plan(backend, crashed, env, safe_gpu, os)
}

/// Persist successful hardware fallbacks, but never silently pin CPU mode.
/// The recovery dialog explicitly stores CPU mode when the user chooses it.
pub fn backend_to_remember(plan: &Plan) -> Option<GpuBackend> {
    (plan.remember && plan.backend != GpuBackend::Cpu).then_some(plan.backend)
}

/// Instance backends for `plan` (`None`: egui's default, which honours `WGPU_BACKEND`).
pub fn backends(plan: &Plan, os: Os) -> Option<wgpu::Backends> {
    if plan.env.is_some() {
        return None;
    }
    Some(match plan.backend {
        GpuBackend::Auto => return None,
        GpuBackend::Vulkan => wgpu::Backends::VULKAN,
        GpuBackend::Dx12 => wgpu::Backends::DX12,
        GpuBackend::Metal => wgpu::Backends::METAL,
        GpuBackend::Gl => wgpu::Backends::GL,
        GpuBackend::Cpu => match os {
            Os::Windows => wgpu::Backends::DX12,
            Os::Mac => wgpu::Backends::METAL,
            // Vulkan only for the window when no GL adapter can present (`rank`): with NVIDIA's
            // EGL the GL adapter can't, and CPU mode failed every start (#1337).
            Os::Other => wgpu::Backends::GL | wgpu::Backends::VULKAN,
        },
    })
}

/// Instance backends for `auto` on Windows (no `WGPU_BACKEND`) when `dx12_vendors`, the PCI vendor
/// ids of the DX12 adapters, includes Intel: DX12, with GL as the last resort (see [`rank`]). An
/// instance with Vulkan loads the Intel Vulkan driver before the adapter selector runs, and #4's
/// crash is in that driver. `dx12_vendors` is called only for that plan; `None` keeps [`backends`]'
/// choice.
pub fn windows_auto_backends(plan: &Plan, os: Os, dx12_vendors: impl FnOnce() -> Vec<u32>) -> Option<wgpu::Backends> {
    let auto = plan.env.is_none() && plan.backend == GpuBackend::Auto && os == Os::Windows;
    (auto && dx12_vendors().contains(&INTEL)).then_some(wgpu::Backends::DX12 | wgpu::Backends::GL)
}

/// The PCI vendor ids of the DX12 adapters, from a DX12-only instance (DXGI, no Vulkan driver)
/// with `desc`'s flags and backend options (the shader compiler from [`dx12_compiler`]).
fn dx12_vendors(desc: &wgpu::InstanceDescriptor) -> Vec<u32> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: desc.flags,
        backend_options: desc.backend_options.clone(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    crate::services::block_on(instance.enumerate_adapters(wgpu::Backends::DX12)).iter().map(|a| a.get_info().vendor).collect()
}

/// The DX12 shader compiler (#712). wgpu's default loads `dxcompiler.dll` by name, and the
/// Windows DLL search falls through to the current directory and `PATH`, so another program's DXC
/// build (a browser's, an SDK's) got loaded and failed device creation or crashed the first shader
/// compile. Use a `dxcompiler.dll` shipped beside the executable, by its full path; otherwise FXC
/// (`d3dcompiler_47.dll`, part of Windows). `WGPU_DX12_COMPILER` still picks one explicitly.
pub fn dx12_compiler(env: Option<&str>, exe_dir: Option<&Path>) -> wgpu::Dx12Compiler {
    if let Some(c) = env.and_then(|v| v.trim().parse().ok()) {
        return c;
    }
    let beside_exe = exe_dir.map(|d| d.join("dxcompiler.dll")).filter(|p| p.is_file());
    match beside_exe.as_deref().and_then(Path::to_str) {
        Some(p) => wgpu::Dx12Compiler::DynamicDxc { dxc_path: p.to_string() },
        None => wgpu::Dx12Compiler::Fxc,
    }
}

/// The GPU makers whose Windows Vulkan driver `auto` avoids, so DX12 comes first: Intel's
/// (`igvk64.dll` crashed while the device was created, #4) and Qualcomm's Adreno driver on
/// Windows on ARM (it lost the device mid-session and the window went with it; DX12 on the same
/// machine didn't, #7537). The name is for System Info.
fn dx12_first_on_windows(vendor: u32) -> Option<&'static str> {
    match vendor {
        INTEL => Some("Intel"),
        QUALCOMM | QUALCOMM_DXGI => Some("Qualcomm"),
        _ => None,
    }
}

/// What adapter selection needs to know about an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub vendor: u32,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
    /// Can present to the window.
    pub surface_ok: bool,
    /// Its name matches the adapter the user asked for ([`adapter_matches`]).
    pub wanted: bool,
}

/// Rank of an adapter (lower is better): high-performance device types first (egui's power
/// preference) or software adapters first for `cpu`; then backends, with DX12 before Vulkan for
/// Intel and Qualcomm on Windows ([`dx12_first_on_windows`]). For `cpu`, any GL adapter comes
/// before Vulkan, which is only the fallback for presenting the window (see [`backends`]). On
/// Windows GL comes after every other adapter, WARP included: GL there (VMware's SVGA3D, GL-only
/// old GPUs) drew a black window (#5191, #1788).
fn rank(a: &Candidate, backend: GpuBackend, os: Os) -> (u8, u8, u8) {
    let fallback = u8::from((backend == GpuBackend::Cpu && a.backend == wgpu::Backend::Vulkan) || (os == Os::Windows && a.backend == wgpu::Backend::Gl));
    let ty = match a.device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    let ty = match (backend, a.device_type) {
        (GpuBackend::Cpu, wgpu::DeviceType::Cpu) => 0,
        (GpuBackend::Cpu, _) => ty + 1,
        _ => ty,
    };
    let be = match a.backend {
        wgpu::Backend::Dx12 if os == Os::Windows && dx12_first_on_windows(a.vendor).is_some() => 0,
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => 1,
        wgpu::Backend::Dx12 => 2,
        wgpu::Backend::Gl => 3,
        wgpu::Backend::BrowserWebGpu | wgpu::Backend::Noop => 4,
    };
    (fallback, ty, be)
}

/// The adapter to use among `adapters` (`None` when there is none). Adapters the user asked for
/// win over the rest when any of them can present; otherwise the ranking decides (see
/// [`wanted_missed`]).
pub fn pick(adapters: &[Candidate], backend: GpuBackend, os: Os) -> Option<usize> {
    let usable = |a: &&Candidate| a.surface_ok;
    let any_wanted = adapters.iter().filter(usable).any(|a| a.wanted);
    adapters.iter().enumerate().filter(|(_, a)| a.surface_ok && (a.wanted || !any_wanted)).min_by_key(|(i, a)| (rank(a, backend, os), *i)).map(|(i, _)| i)
}

/// Whether an adapter called `name` is the one the user asked for: `wanted` is a case-insensitive
/// piece of its name ("nvidia"), or several words that each appear in it ("arc a370m" finds
/// "Intel(R) Arc(tm) A370M Graphics"). An empty `wanted` matches nothing.
pub fn adapter_matches(name: &str, wanted: &str) -> bool {
    photocraft_engine::prefs::gpu_adapter_matches(name, wanted)
}

/// The user asked for an adapter and none of `adapters` that can present is it.
pub fn wanted_missed(adapters: &[Candidate], wanted: Option<&str>) -> bool {
    wanted.is_some() && !adapters.iter().any(|a| a.surface_ok && a.wanted)
}

/// Choosing an adapter is Linux-only for now: Windows has its own Intel/DX12 selection and macOS a
/// single Metal path, and neither is tested with a requested adapter.
pub const ADAPTER_CHOICE: bool = cfg!(target_os = "linux");

/// The adapter request that applies to this launch: none off Linux (`linux`), and none in CPU mode,
/// which wants a software adapter whatever the preference says.
pub fn applicable_adapter(wanted: Option<String>, backend: GpuBackend, linux: bool) -> Option<String> {
    wanted.filter(|_| linux && backend != GpuBackend::Cpu)
}

/// Whether `PHOTOCRAFT_GPU_ADAPTER` makes this launch its own process: only when startup will act
/// on it (a non-empty value, on Linux, outside CPU mode). Otherwise it changes nothing, and the
/// launch may still be forwarded to the running window.
pub fn adapter_override_is_process_specific(env: Option<&str>, mode: RenderingMode, linux: bool) -> bool {
    env.is_some_and(|v| !v.trim().is_empty()) && linux && mode != RenderingMode::Cpu
}

/// The adapter to ask for: `PHOTOCRAFT_GPU_ADAPTER` when set (for one launch), else
/// `performance.gpuAdapter`; `None` is automatic.
pub fn wanted_adapter(env: Option<&str>, pref: &str) -> Option<String> {
    [env.unwrap_or(""), pref].iter().map(|s| s.trim()).find(|s| !s.is_empty()).map(|s| s.chars().take(256).collect())
}

/// The maker's name when picking `chosen` applied the Windows DX12 default of
/// [`dx12_first_on_windows`]: a Vulkan adapter of the same maker was available, or, for Intel,
/// `vulkan_left_out` ([`windows_auto_backends`] left Vulkan out of the instance).
pub fn dx12_default_applied(adapters: &[Candidate], chosen: usize, os: Os, vulkan_left_out: bool) -> Option<&'static str> {
    let c = adapters.get(chosen)?;
    if os != Os::Windows || c.backend != wgpu::Backend::Dx12 {
        return None;
    }
    let maker = dx12_first_on_windows(c.vendor)?;
    let applied =
        (vulkan_left_out && c.vendor == INTEL) || adapters.iter().any(|a| a.backend == wgpu::Backend::Vulkan && dx12_first_on_windows(a.vendor) == Some(maker));
    applied.then_some(maker)
}

/// wgpu compiles indirect-call validation shaders inside `request_device`; Metal's compiler
/// crashes on them on old Intel Macs (Iris Pro, GPUFamily 1 on Monterey), so no mode could start
/// (#3107). Only Intel Macs skip them: PhotoCraft issues no indirect draws or dispatches, but every
/// other GPU keeps wgpu's default. `WGPU_VALIDATION_INDIRECT_CALL=1` still wins.
fn skips_indirect_validation(os: Os, intel: bool) -> bool {
    os == Os::Mac && intel
}

/// Configure eframe's wgpu setup for `plan`: instance backends, the DX12 shader compiler, adapter
/// selection (Intel and Qualcomm on Windows, software adapters for `cpu`) and the marker update
/// once the adapter is chosen. `note` receives a remark for System Info (e.g. the DX12 default).
pub fn configure(setup: &mut egui_wgpu::WgpuSetup, plan: &Plan, os: Os, wanted: Option<String>, sentinel: SharedSentinel, note: Arc<Mutex<Option<String>>>) {
    if let egui_wgpu::WgpuSetup::CreateNew(create) = setup {
        if let Some(b) = backends(plan, os) {
            create.instance_descriptor.backends = b;
        }
        if skips_indirect_validation(os, cfg!(target_arch = "x86_64")) {
            create.instance_descriptor.flags = create.instance_descriptor.flags.difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
        }
        if os == Os::Windows {
            let exe = std::env::current_exe().ok();
            let env = std::env::var("WGPU_DX12_COMPILER").ok();
            create.instance_descriptor.backend_options.dx12.shader_compiler = dx12_compiler(env.as_deref(), exe.as_deref().and_then(Path::parent));
        }
        let without_vulkan = windows_auto_backends(plan, os, || dx12_vendors(&create.instance_descriptor));
        if let Some(b) = without_vulkan {
            create.instance_descriptor.backends = b;
        }
        let vulkan_left_out = without_vulkan.is_some();
        let wanted = applicable_adapter(wanted, plan.backend, ADAPTER_CHOICE);
        let select = wanted.is_some() || (plan.env.is_none() && (plan.backend == GpuBackend::Cpu || (plan.backend == GpuBackend::Auto && os == Os::Windows)));
        if select {
            let backend = plan.backend;
            create.native_adapter_selector = Some(Arc::new(move |adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
                let cands: Vec<Candidate> = adapters
                    .iter()
                    .map(|a| {
                        let i = a.get_info();
                        Candidate {
                            vendor: i.vendor,
                            device_type: i.device_type,
                            backend: i.backend,
                            surface_ok: surface.is_none_or(|s| a.is_surface_supported(s)),
                            wanted: wanted.as_deref().is_some_and(|w| adapter_matches(&i.name, w)),
                        }
                    })
                    .collect();
                let i = pick(&cands, backend, os).ok_or_else(|| "no graphics adapter found".to_string())?;
                if let Some(w) = wanted.as_deref().filter(|_| wanted_missed(&cands, wanted.as_deref())) {
                    *note.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(format!("No graphics adapter named \"{w}\" can draw the window; using the automatic choice"));
                }
                if backend == GpuBackend::Cpu && cands.get(i).is_some_and(|c| c.device_type != wgpu::DeviceType::Cpu) {
                    *note.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some("CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics".into());
                }
                if backend == GpuBackend::Auto
                    && let Some(maker) = dx12_default_applied(&cands, i, os, vulkan_left_out)
                {
                    *note.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(format!("{maker} graphics on Windows: using DirectX 12 (the {maker} Vulkan driver is known to crash)"));
                }
                adapters.get(i).cloned().ok_or_else(|| "no graphics adapter found".to_string())
            }));
        }
    }
    photocraft_ui_egui::gpu_canvas::use_adapter_limits_with(setup, move |adapter| {
        let info = adapter.get_info();
        let mut guard = sentinel.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = guard.as_mut() {
            let mut m = s.marker.clone();
            m.adapter = info.name.clone();
            m.adapter_backend = photocraft_ui_egui::gpu_canvas::backend_name(info.backend).to_string();
            m.driver = [info.driver.as_str(), info.driver_info.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
            if let Err(e) = s.write(m) {
                log::warn!("GPU startup marker: {e}");
            }
        }
    });
}

/// What a previous start left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Previous {
    /// Nothing: it finished starting (or never ran).
    Clean,
    /// Its marker, unlocked: it died before rendering its first frames.
    Crashed(Marker),
    /// Another instance is starting right now (it holds the lock).
    Busy,
}

impl Previous {
    pub fn crashed(&self) -> Option<&Marker> {
        match self {
            Previous::Crashed(m) => Some(m),
            _ => None,
        }
    }
}

/// The locked marker file of this start.
#[derive(Debug)]
pub struct Sentinel {
    path: PathBuf,
    file: Option<File>,
    marker: Marker,
}

/// The sentinel, shared with eframe's adapter hook and the app's started hook.
pub type SharedSentinel = Arc<Mutex<Option<Sentinel>>>;

/// Only renderer errors should affect the next launch's graphics backend. Windowing errors
/// (such as a missing Linux display) and app setup errors never indicate a driver failure.
pub fn keep_marker_after_run(result: &eframe::Result) -> bool {
    matches!(result, Err(eframe::Error::Wgpu(_)))
}

impl Sentinel {
    /// Open (creating) the marker in `dir` and lock it. Returns what a previous start left and,
    /// unless another instance holds the lock or the directory isn't writable, this start's
    /// sentinel (not yet written: see [`Sentinel::write`]).
    pub fn begin(dir: &Path) -> (Previous, Option<Sentinel>) {
        let path = dir.join(MARKER_FILE);
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("GPU startup marker: {e}");
            return (Previous::Clean, None);
        }
        let mut file = match OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("GPU startup marker {}: {e}", path.display());
                return (Previous::Clean, None);
            }
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return (Previous::Busy, None),
            Err(std::fs::TryLockError::Error(e)) => {
                log::warn!("GPU startup marker lock: {e}");
                return (Previous::Clean, None);
            }
        }
        let mut text = String::new();
        let _ = (&mut file).take(MARKER_MAX).read_to_string(&mut text);
        let previous = if text.trim().is_empty() { Previous::Clean } else { Previous::Crashed(Marker::parse(&text)) };
        (previous, Some(Sentinel { path, file: Some(file), marker: Marker::default() }))
    }

    /// Replace the marker's contents with `m`.
    pub fn write(&mut self, m: Marker) -> std::io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| std::io::Error::other("marker closed"))?;
        f.set_len(0)?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(m.to_json().as_bytes())?;
        f.flush()?;
        self.marker = m;
        Ok(())
    }

    /// The start succeeded: clear the marker and release the lock.
    pub fn finish(mut self) {
        if let Some(f) = self.file.take() {
            // Empty means "no crash" even if removing fails (another instance has it open).
            let _ = f.set_len(0);
            drop(f);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The Linux display-server preference (`performance.linuxDisplayServer`) from the preferences
/// file, read before the window opens; `Auto` when the file or value is missing or unknown.
#[cfg(any(target_os = "linux", test))]
pub fn read_display_server(path: Option<&Path>) -> photocraft_engine::prefs::LinuxDisplayServer {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v.get("performance")
        .and_then(|p| serde_json::from_value::<photocraft_engine::prefs::Performance>(p.clone()).ok())
        .map(|performance| performance.linux_display_server)
        .unwrap_or_default()
}

/// Preferences › Performance › Graphics Adapter (`performance.gpuAdapter`), read before the window
/// opens; empty (automatic) when the file or value is missing.
pub fn read_gpu_adapter(path: Option<&Path>) -> String {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v.get("performance").and_then(|p| p.get("gpuAdapter")).and_then(Value::as_str).unwrap_or("").chars().take(256).collect()
}

/// Preferences › Performance › Low-Latency Drawing (`performance.lowLatencyDrawing`, off unless
/// turned on), read before the window opens: it picks the surface's present mode.
pub fn read_low_latency(path: Option<&Path>) -> bool {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let default = photocraft_engine::prefs::Preferences::default().performance.low_latency_drawing;
    v.get("performance").and_then(|p| p.get("lowLatencyDrawing")).and_then(Value::as_bool).unwrap_or(default)
}

/// Read the policy leniently at startup, including old settings without renderingMode.
pub fn read_rendering_prefs(path: Option<&Path>) -> (GpuBackend, RenderingMode) {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let perf = v.get("performance");
    let backend = perf.and_then(|p| p.get("gpuBackend")).and_then(Value::as_str).and_then(GpuBackend::parse).unwrap_or_default();
    let use_gpu = perf.and_then(|p| p.get("useGpu")).and_then(Value::as_bool).unwrap_or(true);
    let explicit = perf.and_then(|p| p.get("renderingMode")).and_then(Value::as_str).and_then(RenderingMode::parse);
    let mode = explicit.unwrap_or(if !use_gpu || backend == GpuBackend::Cpu { RenderingMode::Cpu } else { RenderingMode::Auto });
    (backend, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use GpuBackend::*;

    fn crashed(backend: &str, adapter_backend: &str) -> Marker {
        Marker { backend: backend.into(), adapter: "Intel(R) UHD Graphics".into(), adapter_backend: adapter_backend.into(), ..Default::default() }
    }

    #[test]
    fn low_latency_drawing_is_read_before_the_window_opens_and_defaults_off() {
        let dir = std::env::temp_dir().join(format!("photocraft-low-latency-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("preferences.json");
        let read = |json: &str| {
            std::fs::write(&file, json).unwrap();
            read_low_latency(Some(&file))
        };
        assert!(read(r#"{"performance":{"lowLatencyDrawing":true}}"#));
        assert!(!read(r#"{"performance":{"lowLatencyDrawing":false}}"#));
        assert!(!read(r#"{"performance":{"lowLatencyDrawing":"nope"}}"#), "garbage keeps the default (off)");
        assert!(!read("not json"));
        assert!(!read_low_latency(None), "no settings file: the default (off)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn display_server_preference_is_read_before_the_window_opens() {
        use photocraft_engine::prefs::LinuxDisplayServer;
        let dir = std::env::temp_dir().join(format!("photocraft-display-server-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("preferences.json");
        let read = |json: &str| {
            std::fs::write(&file, json).unwrap();
            read_display_server(Some(&file))
        };
        assert_eq!(read(r#"{"performance":{"linuxDisplayServer":"x11"}}"#), LinuxDisplayServer::X11);
        assert_eq!(read(r#"{"performance":{"linuxDisplayServer":"auto","gpuBackend":"vulkan"}}"#), LinuxDisplayServer::Auto);
        // Unknown values, other types, broken JSON and a missing file all start as Auto.
        assert_eq!(read(r#"{"performance":{"linuxDisplayServer":"wayland"}}"#), LinuxDisplayServer::Auto);
        assert_eq!(read(r#"{"performance":{"linuxDisplayServer":11}}"#), LinuxDisplayServer::Auto);
        assert_eq!(read("{not json"), LinuxDisplayServer::Auto);
        assert_eq!(read_display_server(Some(&dir.join("missing.json"))), LinuxDisplayServer::Auto);
        assert_eq!(read_display_server(None), LinuxDisplayServer::Auto);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cpu_policy_ignores_hardware_backend_override() {
        let p = plan_with_mode(Vulkan, RenderingMode::Cpu, None, Some("vulkan"), false, Os::Windows);
        assert_eq!(p.backend, Cpu);
        assert!(p.env.is_none());
        assert_eq!(plan_with_mode(Vulkan, RenderingMode::Cpu, Some(&crashed("vulkan", "vulkan")), None, false, Os::Windows).backend, Cpu);
        let p = plan_with_mode(Auto, RenderingMode::Gpu, None, Some("vulkan"), true, Os::Windows);
        assert_eq!(p.backend, Cpu);
        assert!(p.env.is_none());
    }

    #[test]
    fn no_marker_uses_the_preference() {
        for os in [Os::Windows, Os::Mac, Os::Other] {
            let p = plan(Auto, None, None, false, os);
            assert_eq!(p, Plan { backend: Auto, env: None, reason: None, remember: false });
        }
        assert_eq!(plan(Dx12, None, None, false, Os::Windows).backend, Dx12);
        assert_eq!(plan(Gl, None, None, false, Os::Other).backend, Gl);
        // A backend the platform lacks is `auto`.
        assert_eq!(plan(Metal, None, None, false, Os::Windows).backend, Auto);
        assert_eq!(plan(Dx12, None, None, false, Os::Mac).backend, Auto);
        assert_eq!(plan(Dx12, None, None, false, Os::Other).backend, Auto);
    }

    #[test]
    fn a_crash_moves_down_each_chain_and_is_remembered() {
        // Windows: Vulkan → DX12 → CPU.
        let p = plan(Auto, Some(&crashed("auto", "vulkan")), None, false, Os::Windows);
        assert_eq!((p.backend, p.remember), (Dx12, true));
        assert!(p.reason.as_deref().unwrap_or("").contains("vulkan (Intel(R) UHD Graphics)"), "{p:?}");
        assert_eq!(plan(Dx12, Some(&crashed("dx12", "dx12")), None, false, Os::Windows).backend, Cpu);
        // Crashed before the adapter was chosen (enumeration): from `auto`.
        assert_eq!(plan(Auto, Some(&crashed("auto", "")), None, false, Os::Windows).backend, Dx12);
        // `auto` that had already picked DX12 (the Intel default) goes to the CPU.
        assert_eq!(plan(Auto, Some(&crashed("auto", "dx12")), None, false, Os::Windows).backend, Cpu);
        // Linux: Vulkan → GL → CPU.
        assert_eq!(plan(Auto, Some(&crashed("auto", "vulkan")), None, false, Os::Other).backend, Gl);
        assert_eq!(plan(Gl, Some(&crashed("gl", "gl")), None, false, Os::Other).backend, Cpu);
        // macOS: Metal → CPU.
        assert_eq!(plan(Auto, Some(&crashed("auto", "metal")), None, false, Os::Mac).backend, Cpu);
        // The CPU path stays there, and isn't "remembered" again.
        let p = plan(Cpu, Some(&crashed("cpu", "dx12")), None, false, Os::Windows);
        assert_eq!((p.backend, p.remember), (Cpu, false));
        // An unreadable marker still counts as a crash.
        assert_eq!(plan(Auto, Some(&Marker::parse("{garbage")), None, false, Os::Windows).backend, Dx12);
    }

    #[test]
    fn env_override_beats_everything() {
        let p = plan(Cpu, Some(&crashed("auto", "vulkan")), Some("vulkan"), false, Os::Windows);
        assert_eq!(p.env.as_deref(), Some("vulkan"));
        assert_eq!(p.backend, Auto);
        assert!(!p.remember);
        assert_eq!(backends(&p, Os::Windows), None, "egui reads WGPU_BACKEND itself");
        // A crash under the override doesn't count against `auto` later.
        let p = plan(Auto, Some(&Marker { backend: "env:vulkan".into(), ..Default::default() }), None, false, Os::Windows);
        assert_eq!(p.backend, Auto);
        // Empty means unset.
        assert_eq!(plan(Auto, None, Some("  "), false, Os::Windows).env, None);
        // --safe-gpu with the override: CPU canvas, the override's backends.
        assert_eq!(plan(Auto, None, Some("dx12"), true, Os::Windows).backend, Cpu);
    }

    /// #3483: A temporary CPU recovery must not change the next launch.
    #[test]
    fn cpu_recovery_requires_explicit_confirmation_to_persist() {
        let gl_crash = crashed("gl", "gl");
        let cpu = plan_with_mode(Auto, RenderingMode::Auto, Some(&gl_crash), None, false, Os::Other);
        assert_eq!((cpu.backend, cpu.remember), (Cpu, true));
        assert_eq!(backend_to_remember(&cpu), None);

        let safe = plan_with_mode(Auto, RenderingMode::Auto, None, None, true, Os::Other);
        assert_eq!(safe.backend, Cpu);
        assert_eq!(backend_to_remember(&safe), None);
        // An unconfirmed fallback leaves the original automatic selection intact.
        assert_eq!(plan_with_mode(Auto, RenderingMode::Auto, None, None, false, Os::Other).backend, Auto);
        // A working hardware fallback can still be remembered.
        let vk_crash = crashed("auto", "vulkan");
        let gl = plan_with_mode(Auto, RenderingMode::Auto, Some(&vk_crash), None, false, Os::Other);
        assert_eq!(backend_to_remember(&gl), Some(Gl));
        // CPU explicitly selected in preferences still works.
        assert_eq!(plan_with_mode(Auto, RenderingMode::Cpu, None, None, false, Os::Other).backend, Cpu);
    }

    #[test]
    fn safe_gpu_forces_the_cpu_for_one_launch() {
        let p = plan(Vulkan, Some(&crashed("vulkan", "vulkan")), None, true, Os::Other);
        assert_eq!((p.backend, p.remember), (Cpu, false));
        assert_eq!(backends(&p, Os::Windows), Some(wgpu::Backends::DX12));
        assert_eq!(backends(&p, Os::Other), Some(wgpu::Backends::GL | wgpu::Backends::VULKAN));
        assert_eq!(backends(&p, Os::Mac), Some(wgpu::Backends::METAL));
    }

    fn cand(vendor: u32, device_type: wgpu::DeviceType, backend: wgpu::Backend) -> Candidate {
        Candidate { vendor, device_type, backend, surface_ok: true, wanted: false }
    }

    #[test]
    fn a_requested_adapter_beats_the_automatic_ranking() {
        use wgpu::{Backend as B, DeviceType as T};
        // The order Vulkan lists this machine's GPUs in: iGPU, dGPU, eGPU.
        let mut a = [cand(INTEL, T::IntegratedGpu, B::Vulkan), cand(INTEL, T::DiscreteGpu, B::Vulkan), cand(0x10de, T::DiscreteGpu, B::Vulkan)];
        assert_eq!(pick(&a, Auto, Os::Other), Some(1), "automatic takes the first discrete adapter, the Arc");
        a[2].wanted = true;
        assert_eq!(pick(&a, Auto, Os::Other), Some(2));
        a[2].wanted = false;
        a[0].wanted = true;
        assert_eq!(pick(&a, Auto, Os::Other), Some(0), "even the iGPU, when asked for");
        // A wanted adapter that can't present is skipped, not an error.
        a[0].surface_ok = false;
        assert_eq!(pick(&a, Auto, Os::Other), Some(1));
        assert!(wanted_missed(&a, Some("iris")));
        assert!(!wanted_missed(&a, None));
        a[0].surface_ok = true;
        assert!(!wanted_missed(&a, Some("iris")));
        // Among several wanted ones (same GPU on Vulkan and GL) the usual ranking decides.
        let mut gl = [cand(0x10de, T::DiscreteGpu, B::Gl), cand(0x10de, T::DiscreteGpu, B::Vulkan)];
        gl.iter_mut().for_each(|c| c.wanted = true);
        assert_eq!(pick(&gl, Auto, Os::Other), Some(1));
    }

    #[test]
    fn an_adapter_request_applies_on_linux_outside_cpu_mode() {
        let w = || Some("nvidia".to_string());
        assert_eq!(applicable_adapter(w(), Auto, true), w());
        assert_eq!(applicable_adapter(w(), Vulkan, true), w());
        assert_eq!(applicable_adapter(w(), Cpu, true), None);
        assert_eq!(applicable_adapter(w(), Auto, false), None, "Windows and macOS keep their own selection");
        assert_eq!(applicable_adapter(None, Auto, true), None);
    }

    #[test]
    fn an_adapter_override_keeps_its_own_process_only_when_it_applies() {
        let on = |env, mode, linux| adapter_override_is_process_specific(env, mode, linux);
        assert!(on(Some("nvidia"), RenderingMode::Auto, true));
        assert!(on(Some("nvidia"), RenderingMode::Gpu, true));
        assert!(!on(Some("nvidia"), RenderingMode::Cpu, true), "ignored in CPU mode");
        assert!(!on(Some("nvidia"), RenderingMode::Auto, false), "ignored off Linux");
        assert!(!on(Some("  "), RenderingMode::Auto, true));
        assert!(!on(None, RenderingMode::Auto, true));
    }

    #[test]
    fn adapter_names_match_loosely_and_the_environment_wins() {
        assert!(adapter_matches("NVIDIA GeForce RTX 5060 Ti", "nvidia"));
        assert!(adapter_matches("Intel(R) Arc(tm) A370M Graphics (DG2)", " ARC A370M "));
        assert!(!adapter_matches("Intel(R) Iris(R) Xe Graphics (ADL GT2)", "arc"));
        assert!(!adapter_matches("anything", ""));
        assert!(!adapter_matches("anything", "   "));
        assert_eq!(wanted_adapter(None, ""), None);
        assert_eq!(wanted_adapter(Some("  "), "iris"), Some("iris".into()));
        assert_eq!(wanted_adapter(Some("nvidia"), "iris"), Some("nvidia".into()));
        assert_eq!(wanted_adapter(Some(&"x".repeat(1000)), "").map(|w| w.len()), Some(256));
    }

    #[test]
    fn gpu_adapter_is_read_before_the_window_opens() {
        let dir = std::env::temp_dir().join(format!("photocraft-gpu-adapter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("preferences.json");
        let read = |json: &str| {
            std::fs::write(&file, json).unwrap();
            read_gpu_adapter(Some(&file))
        };
        assert_eq!(read(r#"{"performance":{"gpuAdapter":"nvidia"}}"#), "nvidia");
        assert_eq!(read(r#"{"performance":{"gpuAdapter":7}}"#), "");
        assert_eq!(read(r#"{"performance":{}}"#), "");
        assert_eq!(read("not json"), "");
        assert_eq!(read_gpu_adapter(None), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn intel_on_windows_defaults_to_dx12() {
        use wgpu::{Backend as B, DeviceType as T};
        let intel = [cand(INTEL, T::IntegratedGpu, B::Vulkan), cand(INTEL, T::IntegratedGpu, B::Dx12), cand(INTEL, T::IntegratedGpu, B::Gl)];
        let i = pick(&intel, Auto, Os::Windows).unwrap();
        assert_eq!(intel[i].backend, B::Dx12);
        assert_eq!(dx12_default_applied(&intel, i, Os::Windows, false), Some("Intel"));
        // Elsewhere (and for other vendors) Vulkan stays first.
        assert_eq!(intel[pick(&intel, Auto, Os::Other).unwrap()].backend, B::Vulkan);
        let nv = [cand(0x10de, T::DiscreteGpu, B::Dx12), cand(0x10de, T::DiscreteGpu, B::Vulkan)];
        let i = pick(&nv, Auto, Os::Windows).unwrap();
        assert_eq!(nv[i].backend, B::Vulkan);
        assert_eq!(dx12_default_applied(&nv, i, Os::Windows, false), None);
        // Intel iGPU + discrete GPU: the discrete one wins (high performance), on Vulkan.
        let hybrid = [intel[0], intel[1], nv[1]];
        assert_eq!(pick(&hybrid, Auto, Os::Windows), Some(2));
        // Adapters that can't present lose to ones that can.
        let mut only = [cand(0x10de, T::DiscreteGpu, B::Vulkan), intel[1]];
        only[0].surface_ok = false;
        assert_eq!(pick(&only, Auto, Os::Windows), Some(1));
        assert_eq!(pick(&[], Auto, Os::Windows), None);
    }

    /// #7537: on Windows on ARM the Adreno Vulkan driver lost the device mid-session and took the
    /// window with it; DX12 on the same machine didn't. DXGI and Vulkan report different vendor ids.
    #[test]
    fn qualcomm_on_windows_defaults_to_dx12() {
        use wgpu::{Backend as B, DeviceType as T};
        let adreno = [cand(QUALCOMM, T::IntegratedGpu, B::Vulkan), cand(QUALCOMM_DXGI, T::IntegratedGpu, B::Dx12)];
        let i = pick(&adreno, Auto, Os::Windows).unwrap();
        assert_eq!(adreno[i].backend, B::Dx12);
        assert_eq!(dx12_default_applied(&adreno, i, Os::Windows, false), Some("Qualcomm"));
        // Only Vulkan offered: still Vulkan, and no note.
        assert_eq!(pick(&adreno[..1], Auto, Os::Windows), Some(0));
        assert_eq!(dx12_default_applied(&adreno, 0, Os::Windows, false), None);
        // An Intel Vulkan adapter doesn't make a Qualcomm DX12 pick "the default".
        let mixed = [cand(INTEL, T::IntegratedGpu, B::Vulkan), adreno[1]];
        assert_eq!(dx12_default_applied(&mixed, 1, Os::Windows, false), None);
        // Vulkan left out of the instance (an Intel adapter was listed) speaks only for Intel.
        assert_eq!(dx12_default_applied(&adreno[1..], 0, Os::Windows, true), None);
        // Elsewhere Vulkan stays first.
        assert_eq!(adreno[pick(&adreno, Auto, Os::Other).unwrap()].backend, B::Vulkan);
    }

    /// #4: an instance with Vulkan loads the Intel Vulkan driver, which crashed at startup, before
    /// the selector picks Intel's DX12 adapter. With an Intel adapter in the DX12 list, `auto` on
    /// Windows creates the instance without Vulkan.
    #[test]
    fn auto_on_windows_with_an_intel_adapter_leaves_vulkan_out_of_the_instance() {
        use wgpu::{Backend as B, DeviceType as T};
        const NVIDIA: u32 = 0x10de;
        const AMD: u32 = 0x1002;
        const WARP: u32 = 0x1414;
        let auto = plan(Auto, None, None, false, Os::Windows);
        let dx12_gl = Some(wgpu::Backends::DX12 | wgpu::Backends::GL);
        assert_eq!(windows_auto_backends(&auto, Os::Windows, || vec![INTEL, WARP]), dx12_gl);
        // Intel iGPU + discrete GPU: the discrete GPU is then used on DX12.
        assert_eq!(windows_auto_backends(&auto, Os::Windows, || vec![NVIDIA, INTEL, WARP]), dx12_gl);
        // No Intel adapter: egui's default backends, Vulkan included.
        assert_eq!(windows_auto_backends(&auto, Os::Windows, || vec![NVIDIA, WARP]), None);
        assert_eq!(windows_auto_backends(&auto, Os::Windows, || vec![AMD]), None);
        assert_eq!(windows_auto_backends(&auto, Os::Windows, Vec::new), None);
        // `WGPU_BACKEND`, `--safe-gpu`, a chosen backend, a crash fallback and other platforms
        // don't list the DX12 adapters at all.
        let not_listed = || -> Vec<u32> { panic!("DX12 adapters listed") };
        for (p, os) in [
            (plan(Auto, None, Some("vulkan"), false, Os::Windows), Os::Windows),
            (plan(Auto, None, None, true, Os::Windows), Os::Windows),
            (plan(Vulkan, None, None, false, Os::Windows), Os::Windows),
            (plan(Auto, Some(&crashed("auto", "vulkan")), None, false, Os::Windows), Os::Windows),
            (plan(Auto, None, None, false, Os::Other), Os::Other),
            (plan(Auto, None, None, false, Os::Mac), Os::Mac),
        ] {
            assert_eq!(windows_auto_backends(&p, os, not_listed), None, "{p:?}");
        }
        // Without Vulkan adapters to compare with, System Info still names the Intel default.
        let listed = [cand(INTEL, T::IntegratedGpu, B::Dx12), cand(WARP, T::Cpu, B::Dx12), cand(INTEL, T::IntegratedGpu, B::Gl)];
        let i = pick(&listed, Auto, Os::Windows).unwrap();
        assert_eq!(i, 0);
        assert_eq!(dx12_default_applied(&listed, i, Os::Windows, true), Some("Intel"));
        assert_eq!(dx12_default_applied(&listed, i, Os::Windows, false), None);
    }

    #[test]
    fn cpu_prefers_software_adapters() {
        use wgpu::{Backend as B, DeviceType as T};
        let a = [cand(INTEL, T::IntegratedGpu, B::Dx12), cand(0x1414, T::Cpu, B::Dx12)];
        assert_eq!(pick(&a, Cpu, Os::Windows), Some(1));
        // No software adapter: still something.
        assert_eq!(pick(&a[..1], Cpu, Os::Windows), Some(0));
    }

    /// #1337: on Linux with NVIDIA, the only GL adapter (NVIDIA's EGL) can't present, so CPU mode
    /// found no adapter and never started. GL still wins whenever it can present.
    #[test]
    fn cpu_on_linux_falls_back_to_vulkan_only_when_gl_cannot_present() {
        use wgpu::{Backend as B, DeviceType as T};
        const NVIDIA: u32 = 0x10de;
        let llvmpipe = cand(0x10005, T::Cpu, B::Gl);
        let nvidia_gl = cand(NVIDIA, T::DiscreteGpu, B::Gl);
        let nvidia_vk = cand(NVIDIA, T::DiscreteGpu, B::Vulkan);
        let lavapipe = cand(0x10005, T::Cpu, B::Vulkan);
        // GL presents: same choice as GL-only, software first, then hardware GL over any Vulkan.
        assert_eq!(pick(&[nvidia_vk, lavapipe, llvmpipe], Cpu, Os::Other), Some(2));
        assert_eq!(pick(&[nvidia_vk, lavapipe, nvidia_gl], Cpu, Os::Other), Some(2));
        // GL can't present: Vulkan for the window, software first.
        let mut gl = nvidia_gl;
        gl.surface_ok = false;
        assert_eq!(pick(&[gl, nvidia_vk, lavapipe], Cpu, Os::Other), Some(2));
        assert_eq!(pick(&[gl, nvidia_vk], Cpu, Os::Other), Some(1));
        // Other backends rank as before.
        assert_eq!(pick(&[nvidia_gl, nvidia_vk], Auto, Os::Other), Some(1));
    }

    /// #5191, #1788: a Windows VM offers VMware's SVGA3D over GL (`Other`) and WARP (`Cpu`, DX12).
    /// GL drew a black window; WARP works. GL stays the last resort.
    #[test]
    fn windows_auto_prefers_warp_over_gl() {
        use wgpu::{Backend as B, DeviceType as T};
        let svga_gl = cand(0x15ad, T::Other, B::Gl);
        let warp = cand(0x1414, T::Cpu, B::Dx12);
        assert_eq!(pick(&[svga_gl, warp], Auto, Os::Windows), Some(1));
        assert_eq!(pick(&[cand(INTEL, T::IntegratedGpu, B::Gl), warp], Auto, Os::Windows), Some(1));
        assert_eq!(pick(&[svga_gl], Auto, Os::Windows), Some(0));
        // Elsewhere GL keeps its rank.
        assert_eq!(pick(&[svga_gl, warp], Auto, Os::Other), Some(0));
    }

    #[test]
    fn adapters_must_present_even_when_none_support_the_surface() {
        let mut a = cand(INTEL, wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Dx12);
        a.surface_ok = false;
        assert_eq!(pick(&[a], Auto, Os::Windows), None);
        assert_eq!(pick(&[a], Cpu, Os::Windows), None);
    }

    #[test]
    fn explicit_policy_overrides_legacy_cpu_backend() {
        assert_eq!(plan_with_mode(Cpu, RenderingMode::Gpu, None, None, false, Os::Mac).backend, Auto);
        assert_eq!(plan_with_mode(Metal, RenderingMode::Cpu, None, None, false, Os::Mac).backend, Cpu);
        assert_eq!(plan_with_mode(Auto, RenderingMode::Gpu, None, None, true, Os::Mac).backend, Cpu);
    }

    #[test]
    fn marker_round_trip_and_lenient_parse() {
        let m = Marker {
            backend: "auto".into(),
            adapter: "A \"quoted\" GPU".into(),
            adapter_backend: "vulkan".into(),
            driver: "1.2".into(),
            version: "0.1".into(),
        };
        assert_eq!(Marker::parse(&m.to_json()), m);
        assert_eq!(Marker::parse(""), Marker::default());
        assert_eq!(Marker::parse("[1,2]").tried(), Some(Auto));
        assert_eq!(Marker::parse(r#"{"backend": 7}"#).tried(), Some(Auto));
        assert_eq!(Marker::parse(r#"{"backend": "env:dx12"}"#).tried(), None);
        let long = format!(r#"{{"adapter": "{}"}}"#, "x".repeat(10_000));
        assert_eq!(Marker::parse(&long).adapter.len(), 256);
    }

    #[test]
    fn a_failed_start_keeps_its_marker_only_when_a_safer_backend_exists() {
        // GPU backends move down the chain on the next start.
        assert!(keep_marker_after_error(&plan(Auto, None, None, false, Os::Other), Os::Other));
        assert!(keep_marker_after_error(&plan(Gl, None, None, false, Os::Other), Os::Other));
        assert!(keep_marker_after_error(&plan(Dx12, None, None, false, Os::Windows), Os::Windows));
        assert!(keep_marker_after_error(&plan(Auto, None, None, false, Os::Mac), Os::Mac));
        // The CPU path is the end of every chain, including the `--safe-gpu` retry.
        for os in [Os::Windows, Os::Mac, Os::Other] {
            assert!(!keep_marker_after_error(&plan(Cpu, None, None, false, os), os));
            assert!(!keep_marker_after_error(&plan(Auto, None, None, true, os), os));
        }
        // `WGPU_BACKEND` is the user's choice; no plan reads its marker.
        assert!(!keep_marker_after_error(&plan(Auto, None, Some("vulkan"), false, Os::Other), Os::Other));
    }

    /// A failed CPU start (no usable GL adapter under XWayland) left a `cpu` marker, so every
    /// later start was planned on CPU and failed the same way until the marker was deleted by hand.
    #[test]
    fn a_failed_cpu_start_does_not_pin_later_starts() {
        let dir = temp_dir("cpu-error");
        let (_, s) = Sentinel::begin(&dir);
        let mut s = s.expect("sentinel");
        let failed = plan(Auto, None, None, true, Os::Other);
        s.write(Marker { backend: failed.backend.name().into(), ..Default::default() }).unwrap();
        // eframe returned an error: what `main` does with the marker.
        if keep_marker_after_error(&failed, Os::Other) {
            drop(s);
        } else {
            s.finish();
        }
        let (prev, _s) = begin_when_unlocked(&dir);
        assert_eq!(prev, Previous::Clean);
        assert_eq!(plan(Auto, prev.crashed(), None, false, Os::Other).backend, Auto);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-gpu-startup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// `Sentinel::begin`, called again up to 50 times, 10 ms apart, while it returns
    /// `Previous::Busy`. Other tests in this binary spawn processes: between fork and exec a child
    /// briefly shares the marker's flock, so `begin` can see the lock as busy just after a test
    /// drops its sentinel.
    fn begin_when_unlocked(dir: &Path) -> (Previous, Option<Sentinel>) {
        let (mut prev, mut s) = Sentinel::begin(dir);
        for _ in 0..50 {
            if prev != Previous::Busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            (prev, s) = Sentinel::begin(dir);
        }
        (prev, s)
    }

    #[test]
    fn sentinel_detects_a_crash_and_clears_on_success() {
        let dir = temp_dir("cycle");
        // First launch: nothing left behind.
        let (prev, s) = Sentinel::begin(&dir);
        assert_eq!(prev, Previous::Clean);
        let mut s = s.expect("sentinel");
        s.write(Marker { backend: "auto".into(), ..Default::default() }).unwrap();
        s.write(Marker { backend: "auto".into(), adapter: "GPU".into(), adapter_backend: "vulkan".into(), ..Default::default() }).unwrap();
        // While it is starting, a second instance sees it busy, not crashed.
        let (busy, none) = Sentinel::begin(&dir);
        assert_eq!(busy, Previous::Busy);
        assert!(none.is_none());
        // The process dies in the driver: the lock goes, the marker stays.
        drop(s);
        let (prev, s) = begin_when_unlocked(&dir);
        let m = prev.crashed().cloned().expect("crash detected");
        assert_eq!((m.adapter.as_str(), m.tried()), ("GPU", Some(Vulkan)));
        // This start succeeds: the next one finds nothing.
        let mut s = s.expect("sentinel");
        s.write(Marker { backend: "dx12".into(), ..Default::default() }).unwrap();
        s.finish();
        assert!(!dir.join(MARKER_FILE).exists());
        let (prev, _s) = Sentinel::begin(&dir);
        assert_eq!(prev, Previous::Clean);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn begin_when_unlocked_waits_for_a_briefly_held_lock() {
        let dir = temp_dir("shared-lock");
        let (_, s) = Sentinel::begin(&dir);
        let mut s = s.expect("sentinel");
        s.write(Marker { backend: "vulkan".into(), ..Default::default() }).unwrap();
        // After the start dies, a second handle on the marker holds the lock for 50 ms, as a
        // spawned child's copy of the descriptor does until the child calls exec.
        let held = File::open(dir.join(MARKER_FILE)).unwrap();
        drop(s);
        held.lock().unwrap();
        assert_eq!(Sentinel::begin(&dir).0, Previous::Busy);
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            held.unlock().unwrap();
        });
        let (prev, s) = begin_when_unlocked(&dir);
        release.join().unwrap();
        assert_eq!(prev.crashed().map(|m| m.backend.as_str()), Some("vulkan"));
        s.expect("sentinel").finish();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returned_non_graphics_errors_do_not_trigger_backend_recovery() {
        let cases = [
            ("success", Ok(()), false),
            ("app-error", Err(eframe::Error::AppCreation(Box::new(std::io::Error::other("app setup failed")))), false),
            ("gpu-error", Err(eframe::Error::Wgpu(egui_wgpu::WgpuError::CustomNativeAdapterSelectionError("no adapter".into()))), true),
        ];
        for (name, result, keep) in cases {
            let dir = temp_dir(name);
            let (_, s) = Sentinel::begin(&dir);
            let mut s = s.expect("sentinel");
            s.write(Marker { backend: "vulkan".into(), ..Default::default() }).unwrap();
            if keep_marker_after_run(&result) {
                drop(s);
            } else {
                s.finish();
            }
            let (previous, s) = begin_when_unlocked(&dir);
            assert_eq!(previous.crashed().is_some(), keep, "{name}: {result:?}");
            if !keep {
                assert_eq!(plan(Vulkan, previous.crashed(), None, false, Os::Other).backend, Vulkan);
            }
            s.expect("sentinel").finish();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn unwritable_config_dir_is_not_fatal() {
        let dir = temp_dir("file");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, b"not a directory").unwrap();
        let (prev, s) = Sentinel::begin(&dir.join("sub"));
        assert_eq!(prev, Previous::Clean);
        assert!(s.is_none());
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn prefs_are_read_leniently() {
        let dir = temp_dir("prefs");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("preferences.json");
        assert_eq!(read_rendering_prefs(None), (Auto, RenderingMode::Auto));
        assert_eq!(read_rendering_prefs(Some(&p)), (Auto, RenderingMode::Auto));
        std::fs::write(&p, r#"{"performance": {"gpuBackend": "dx12", "useGpu": false}, "interface": {}}"#).unwrap();
        assert_eq!(read_rendering_prefs(Some(&p)), (Dx12, RenderingMode::Cpu));
        std::fs::write(&p, r#"{"performance": {"gpuBackend": "quantum"}}"#).unwrap();
        assert_eq!(read_rendering_prefs(Some(&p)), (Auto, RenderingMode::Auto));
        std::fs::write(&p, r#"{"performance": {"useGpu": false, "renderingMode": "gpu"}}"#).unwrap();
        assert_eq!(read_rendering_prefs(Some(&p)), (Auto, RenderingMode::Gpu));
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(read_rendering_prefs(Some(&p)), (Auto, RenderingMode::Auto));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #712: never a `dxcompiler.dll` found through the DLL search path (current directory, PATH).
    #[test]
    fn dx12_compiler_is_fxc_or_dxc_beside_the_exe() {
        use wgpu::Dx12Compiler as C;
        let dir = temp_dir("dxc");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(dx12_compiler(None, None), C::Fxc));
        assert!(matches!(dx12_compiler(None, Some(&dir)), C::Fxc));
        std::fs::write(dir.join("dxcompiler.dll"), b"MZ").unwrap();
        let expected = dir.join("dxcompiler.dll");
        assert!(matches!(dx12_compiler(None, Some(&dir)), C::DynamicDxc { dxc_path } if Path::new(&dxc_path) == expected));
        // An explicit WGPU_DX12_COMPILER wins; an unknown value is ignored.
        assert!(matches!(dx12_compiler(Some(" FXC "), Some(&dir)), C::Fxc));
        assert!(matches!(dx12_compiler(Some("dxc"), None), C::DynamicDxc { dxc_path } if dxc_path == "dxcompiler.dll"));
        assert!(matches!(dx12_compiler(Some("quantum"), None), C::Fxc));
        let _ = std::fs::remove_dir_all(&dir);

        // The app's setup replaces wgpu's default (`Auto`: DXC by name from the search path).
        let mut setup = egui_wgpu::WgpuSetup::without_display_handle();
        let plan = plan(Auto, None, None, false, Os::Windows);
        configure(&mut setup, &plan, Os::Windows, None, Arc::new(Mutex::new(None)), Arc::default());
        let egui_wgpu::WgpuSetup::CreateNew(create) = &setup else { panic!("a new instance") };
        if std::env::var_os("WGPU_DX12_COMPILER").is_none() {
            let safe = match &create.instance_descriptor.backend_options.dx12.shader_compiler {
                C::Fxc => true,
                C::DynamicDxc { dxc_path } => Path::new(dxc_path).is_absolute(),
                C::StaticDxc | C::Auto => false,
            };
            assert!(safe, "{:?}", create.instance_descriptor.backend_options.dx12.shader_compiler);
        }
    }

    #[test]
    fn intel_macs_skip_the_indirect_validation_shaders() {
        // Metal's compiler crashed on them on Intel Iris Pro, losing the device in every mode (#3107).
        assert!(skips_indirect_validation(Os::Mac, true));
        // Apple silicon, Windows and Linux keep wgpu's validation.
        for (os, intel) in [(Os::Mac, false), (Os::Windows, true), (Os::Other, true), (Os::Other, false)] {
            assert!(!skips_indirect_validation(os, intel), "{os:?} intel={intel}");
        }
        for (os, safe_gpu) in [(Os::Windows, false), (Os::Other, true), (Os::Other, false)] {
            let mut setup = egui_wgpu::WgpuSetup::without_display_handle();
            let egui_wgpu::WgpuSetup::CreateNew(before) = &setup else { panic!("a new instance") };
            let before = before.instance_descriptor.flags;
            configure(&mut setup, &plan(Auto, None, None, safe_gpu, os), os, None, Arc::new(Mutex::new(None)), Arc::default());
            let egui_wgpu::WgpuSetup::CreateNew(create) = &setup else { panic!("a new instance") };
            assert_eq!(create.instance_descriptor.flags, before, "{os:?}: flags untouched");
        }
        if cfg!(target_arch = "x86_64") && std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
            let mut setup = egui_wgpu::WgpuSetup::without_display_handle();
            configure(&mut setup, &plan(Auto, None, None, false, Os::Mac), Os::Mac, None, Arc::new(Mutex::new(None)), Arc::default());
            let egui_wgpu::WgpuSetup::CreateNew(create) = &setup else { panic!("a new instance") };
            assert!(!create.instance_descriptor.flags.contains(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL));
        }
    }
}
