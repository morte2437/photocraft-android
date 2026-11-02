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
    read_performance(path).map(|performance| performance.linux_display_server).unwrap_or_default()
}

/// The preferences file's Performance section, read as the app reads it: a value of the wrong
/// type keeps its default without resetting the others. `None` without a file or section.
fn read_performance(path: Option<&Path>) -> Option<photocraft_engine::prefs::Performance> {
    let v: Value = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v.get("performance").cloned().map(photocraft_engine::prefs::read_section)
}

/// Preferences › Performance › Graphics Adapter (`performance.gpuAdapter`), read before the window
/// opens; empty (automatic) when the file or value is missing or not text.
pub fn read_gpu_adapter(path: Option<&Path>) -> String {
    read_performance(path).map(|performance| performance.gpu_adapter.chars().take(256).collect()).unwrap_or_default()
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
mod tests;
