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
    let m =
        Marker { backend: "auto".into(), adapter: "A \"quoted\" GPU".into(), adapter_backend: "vulkan".into(), driver: "1.2".into(), version: "0.1".into() };
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
