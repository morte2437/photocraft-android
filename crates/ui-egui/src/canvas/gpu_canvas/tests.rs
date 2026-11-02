use super::*;

/// A GPU measurement arrives a frame or two after its refresh, so most GPU refreshes carry
/// none; the last one reported must stand until a newer one lands. A CPU refresh is
/// different: there is no GPU time at all then, and keeping the old number would show it as
/// this refresh's. Runs anywhere — the GPU timing tests need hardware and CI has none.
#[test]
fn a_gpu_measurement_stands_until_replaced_and_a_cpu_refresh_clears_it() {
    let timing = |ms, px| photocraft_gpu::GpuTiming { ms, px, spans: 1 };
    let refresh = |kind, gpu| Refresh { kind, gpu, ..Default::default() };
    let mut perf = Perf::default();
    assert_eq!(perf.gpu_ms, None, "nothing is reported before a measurement");

    perf.record_refresh(&refresh("gpu-full", Some(timing(83.9, 24_000_000))));
    assert_eq!((perf.gpu_ms, perf.gpu_ms_px), (Some(83.9), 24_000_000));

    // The common case: a GPU refresh whose own measurement has not come back yet.
    perf.record_refresh(&refresh("gpu-rect", None));
    assert_eq!((perf.gpu_ms, perf.gpu_ms_px), (Some(83.9), 24_000_000), "the last measurement stands");

    perf.record_refresh(&refresh("gpu-rect", Some(timing(11.2, 1_098_640))));
    assert_eq!((perf.gpu_ms, perf.gpu_ms_px), (Some(11.2), 1_098_640), "a newer measurement replaces it");

    // The CPU compositor did this one: the GPU number belongs to no refresh on screen.
    perf.record_refresh(&refresh("full", None));
    assert_eq!((perf.gpu_ms, perf.gpu_ms_px), (None, 0), "a CPU refresh clears it rather than showing a stale one");

    // And it comes back on the next GPU refresh that carries one.
    perf.record_refresh(&refresh("gpu-full", Some(timing(42.0, 6_000_000))));
    assert_eq!((perf.gpu_ms, perf.gpu_ms_px), (Some(42.0), 6_000_000));
}

#[test]
fn device_limits_support_v3d_shader_variable_limit() {
    let supported = wgpu::Limits { max_texture_dimension_2d: 7680, max_inter_stage_shader_variables: 15, ..wgpu::Limits::default() };
    let requested = device_limits_for(wgpu::Backend::Vulkan, supported.clone());
    assert_eq!(requested.max_inter_stage_shader_variables, 15);
    assert!(requested.check_limits(&supported));
    assert_eq!(requested.max_texture_dimension_2d, 7680);
    assert_eq!(requested.max_buffer_size, supported.max_buffer_size);
}

#[test]
fn device_limits_support_weaker_gl_adapters() {
    let supported = wgpu::Limits {
        max_texture_dimension_2d: 1024,
        max_texture_dimension_3d: 128,
        max_bind_groups: 2,
        max_buffer_size: 128 << 20,
        ..wgpu::Limits::downlevel_webgl2_defaults()
    };
    let requested = device_limits_for(wgpu::Backend::Gl, supported.clone());
    assert!(requested.check_limits(&supported));
    assert_eq!(requested, supported);
}

#[test]
fn device_limits_preserve_backend_defaults_on_capable_adapters() {
    let supported = wgpu::Limits {
        max_texture_dimension_2d: 16_384,
        max_buffer_size: 1 << 30,
        max_bind_groups: 8,
        max_inter_stage_shader_variables: 32,
        min_uniform_buffer_offset_alignment: 64,
        min_storage_buffer_offset_alignment: 64,
        ..wgpu::Limits::default()
    };
    for (backend, base) in [
        (wgpu::Backend::Vulkan, wgpu::Limits::default()),
        (wgpu::Backend::Dx12, wgpu::Limits::default()),
        (wgpu::Backend::Metal, wgpu::Limits::default()),
        (wgpu::Backend::Gl, wgpu::Limits::downlevel_webgl2_defaults()),
    ] {
        let requested = device_limits_for(backend, supported.clone());
        assert!(requested.check_limits(&supported));
        assert_eq!(requested, wgpu::Limits { max_texture_dimension_2d: 16_384, max_buffer_size: 1 << 30, ..base }, "{backend:?}");
    }
}

#[test]
fn device_limits_support_larger_minimum_alignments() {
    let supported = wgpu::Limits { min_uniform_buffer_offset_alignment: 512, min_storage_buffer_offset_alignment: 1024, ..wgpu::Limits::default() };
    for backend in [wgpu::Backend::Vulkan, wgpu::Backend::Gl] {
        let requested = device_limits_for(backend, supported.clone());
        assert!(requested.check_limits(&supported));
        assert_eq!(requested.min_uniform_buffer_offset_alignment, 512);
        assert_eq!(requested.min_storage_buffer_offset_alignment, 1024);
    }
}

/// The scratch-texture workaround is for Intel's DX12 drivers (#892); other DX12 adapters
/// render straight into the level like every other backend (#4515).
#[test]
fn separate_mip_targets_only_for_intel_on_dx12() {
    use wgpu::Backend::{Dx12, Metal, Vulkan};
    let (intel, nvidia, amd) = (0x8086, 0x10de, 0x1002);
    assert!(separate_mip_targets(Dx12, intel, None));
    assert!(!separate_mip_targets(Dx12, nvidia, None));
    assert!(!separate_mip_targets(Dx12, amd, None));
    assert!(!separate_mip_targets(Vulkan, intel, None));
    assert!(!separate_mip_targets(Metal, 0, None));
    // PHOTOCRAFT_SEPARATE_MIPS forces either way; anything else leaves the default.
    assert!(separate_mip_targets(Dx12, nvidia, Some("1")));
    assert!(!separate_mip_targets(Dx12, intel, Some(" 0 ")));
    assert!(separate_mip_targets(Dx12, intel, Some("yes")));
}

/// #6418: levels of at most WHOLE_MIP_SIDE are redrawn whole; larger ones keep the damage.
#[test]
fn small_mip_levels_are_redrawn_whole() {
    // A 3 × 3 dab at (800, 500) of a 1791 × 1083 tile, at mip 1 (895 × 541): halved and padded.
    assert_eq!(mip_damage([800, 500, 803, 503], 895, 541), [399, 249, 403, 253]);
    // At most 128 px on either side: the whole level, wherever the damage is.
    assert_eq!(mip_damage([100, 60, 102, 63], 111, 67), [0, 0, 111, 67]);
    assert_eq!(mip_damage([0, 0, 1, 1], 128, 2), [0, 0, 128, 2]);
    assert_eq!(mip_damage([0, 0, 1, 1], 129, 2), [0, 0, 2, 2]);
}

#[test]
fn gpu_budget_follows_memory_usage_and_ram() {
    const G: u64 = 1 << 30;
    // Card memory unknown (integrated, software, or unreadable): what Memory Usage leaves
    // after the document, within a quarter of RAM.
    assert_eq!(memory_budget(8 * G, 3 * G, Some(48 * G), None, 0), 5 * G);
    assert_eq!(memory_budget(8 * G, 3 * G, Some(16 * G), None, 0), 4 * G);
    assert_eq!(memory_budget(64 * G, 3 * G, Some(16 * G), None, 0), 4 * G);
    // Never below the floor, even with nothing left; 16 GB assumed when RAM is unknown.
    assert_eq!(memory_budget(8 * G, 10 * G, Some(48 * G), None, 0), MIN_GPU_BUDGET);
    assert_eq!(memory_budget(64 * G, 0, None, None, 0), 4 * G);
    // A card's own memory, known, doesn't hold the document's pixels: 10 GB of layers no
    // longer squeeze the budget, which stays within 3/8 of the card (#4536).
    assert_eq!(memory_budget(8 * G, 10 * G, Some(32 * G), Some(32 * G), 0), 8 * G);
    assert_eq!(memory_budget(8 * G, 10 * G, Some(32 * G), Some(16 * G), 0), 6 * G);
    assert_eq!(memory_budget(8 * G, 10 * G, Some(16 * G), Some(32 * G), 0), 4 * G);
    // GPU Memory set: that much, within the card's share, above the floor.
    assert_eq!(memory_budget(8 * G, 10 * G, Some(16 * G), None, 12 * G), 12 * G);
    assert_eq!(memory_budget(8 * G, 10 * G, Some(16 * G), Some(16 * G), 12 * G), 6 * G);
    assert_eq!(memory_budget(8 * G, 0, None, Some(32 * G), 1 << 20), MIN_GPU_BUDGET);
    assert!(physical_memory().is_none_or(|b| b >= 256 << 20));
}

/// An 8 GB card in a 64 GB machine: both budgets used to plan for 16 GB, twice the card (#4536).
#[test]
fn gpu_budgets_fit_a_discrete_gpus_own_memory() {
    const G: u64 = 1 << 30;
    let (ram, vram) = (Some(64 * G), Some(8 * G));
    assert_eq!(memory_budget(64 * G, 3 * G, ram, vram, 0), 3 * G);
    assert_eq!(canvas_budget(ram, vram), 3 * G);
    assert!(memory_budget(64 * G, 0, ram, vram, 0) + canvas_budget(ram, vram) <= 6 * G, "a quarter of the card stays free");
    // A card larger than a quarter of RAM caps nothing (its own memory isn't charged for the
    // document's 3 GB); a tiny one keeps the floor.
    assert_eq!(memory_budget(8 * G, 3 * G, Some(48 * G), Some(24 * G), 0), 8 * G);
    assert_eq!(canvas_budget(Some(16 * G), Some(24 * G)), 4 * G);
    assert_eq!(memory_budget(64 * G, 0, ram, Some(G), 0), MIN_GPU_BUDGET);
    assert_eq!(canvas_budget(ram, Some(G)), MIN_GPU_BUDGET);
}

/// DXGI reports each adapter's dedicated memory, found by the vendor and device ids wgpu gives.
#[cfg(windows)]
#[test]
fn windows_reports_the_adapters_own_memory() {
    use winsafe::prelude::*;
    let descs: Vec<_> = winsafe::CreateDXGIFactory1().unwrap().EnumAdapters1().map(|a| a.unwrap().GetDesc1().unwrap()).collect();
    assert!(!descs.is_empty(), "DXGI lists at least its software adapter");
    for d in descs.iter().filter(|d| d.DedicatedVideoMemory > 0) {
        // The first adapter with these ids is the one matched.
        let first = descs.iter().find(|e| e.VendorId == d.VendorId && e.DeviceId == d.DeviceId).unwrap();
        assert_eq!(detect_video_memory(d.VendorId, d.DeviceId), Some(first.DedicatedVideoMemory as u64));
    }
    assert_eq!(detect_video_memory(0xdead, 0xbeef), None, "no such adapter");
}

/// System Info names the card's own memory, after the driver, when the card reports one; an
/// integrated GPU or an unknown card shows no such line.
#[test]
fn system_info_shows_the_cards_memory() {
    let info = GpuInfo { adapter: "Card".into(), driver: "Driver 1".into(), video_memory_mb: Some(8192), ..Default::default() };
    let lines = info.lines();
    assert_eq!(lines.get(2).map(String::as_str), Some("Driver: Driver 1"));
    assert_eq!(lines.get(3).map(String::as_str), Some("Graphics memory: 8.0G"));
    let none = GpuInfo { video_memory_mb: None, ..info }.lines();
    assert!(none.iter().all(|l| !l.starts_with("Graphics memory")), "{none:?}");
    assert_eq!(none.len() + 1, lines.len());
}

/// The Linux reader finds the card by its PCI ids in a `/sys/class/drm`-shaped folder and reads
/// amdgpu's VRAM size; a card without one, unreadable ids or a malformed size give nothing.
#[test]
fn drm_video_memory_reads_the_matching_cards_vram() {
    let drm = std::env::temp_dir().join(format!("pc-drm-{}", std::process::id()));
    let card = |name: &str, vendor: &str, device: &str, vram: Option<&str>| {
        let dev = drm.join(name).join("device");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("vendor"), vendor).unwrap();
        std::fs::write(dev.join("device"), device).unwrap();
        if let Some(v) = vram {
            std::fs::write(dev.join("mem_info_vram_total"), v).unwrap();
        }
    };
    // An Intel iGPU (no VRAM file), an AMD card, and an entry with garbage ids.
    card("card0", "0x8086\n", "0x9a49\n", None);
    card("card1", "0x1002\n", "0x73bf\n", Some("17163091968\n"));
    card("card2", "not hex\n", "0x73bf\n", Some("1\n"));
    card("card3", "0x10de\n", "0x2684\n", Some("lots\n"));
    assert_eq!(drm_video_memory(&drm, 0x1002, 0x73bf), Some(17_163_091_968));
    assert_eq!(drm_video_memory(&drm, 0x8086, 0x9a49), None, "no amdgpu VRAM file");
    assert_eq!(drm_video_memory(&drm, 0x10de, 0x2684), None, "a malformed size");
    assert_eq!(drm_video_memory(&drm, 0xdead, 0xbeef), None, "no such card");
    assert_eq!(drm_video_memory(&drm.join("missing"), 0x1002, 0x73bf), None, "no drm folder");
    std::fs::remove_dir_all(&drm).unwrap();
}

/// #P60/P61: amdgpu's sysfs files have no NVIDIA equivalent, so a discrete NVIDIA card's own
/// memory comes from `nvidia-smi`'s CSV instead (real output: `0x2C0510DE, 16303`, device id
/// the high half of the hex, vendor the low). A second card, an upper-case `0X` prefix, extra
/// spaces, a non-matching id, a missing comma, a non-numeric size, empty output and a size
/// that would overflow all give nothing rather than the wrong (or a panicking) answer.
#[test]
fn parses_nvidia_smis_own_report_of_the_cards_memory() {
    let two = "0x2C0510DE, 16303\n0x061C10DE, 24576\n";
    assert_eq!(parse_nvidia_smi(two, NVIDIA_VENDOR, 0x2c05), Some(16303 << 20));
    assert_eq!(parse_nvidia_smi(two, NVIDIA_VENDOR, 0x061c), Some(24576 << 20));
    assert_eq!(parse_nvidia_smi("0X2C0510DE,   16303  \n", NVIDIA_VENDOR, 0x2c05), Some(16303 << 20), "upper-case prefix, loose spacing");
    assert_eq!(parse_nvidia_smi(two, NVIDIA_VENDOR, 0xdead), None, "no such device id");
    assert_eq!(parse_nvidia_smi(two, 0x1002, 0x2c05), None, "no such vendor id");
    assert_eq!(parse_nvidia_smi("not csv at all", NVIDIA_VENDOR, 0x2c05), None);
    assert_eq!(parse_nvidia_smi("0x2C0510DE 16303\n", NVIDIA_VENDOR, 0x2c05), None, "no comma");
    assert_eq!(parse_nvidia_smi("0x2C0510DE, lots\n", NVIDIA_VENDOR, 0x2c05), None, "not a number");
    assert_eq!(parse_nvidia_smi("", NVIDIA_VENDOR, 0x2c05), None, "empty output");
    assert_eq!(parse_nvidia_smi(&format!("0x2C0510DE, {}\n", u64::MAX), NVIDIA_VENDOR, 0x2c05), None, "would overflow");
}

/// Two cards of one model (same vendor and device id) with different memory: wgpu's ids can't
/// tell which one it picked, so the budget assumes the smaller rather than whichever came first.
#[test]
fn nvidia_smi_cards_sharing_an_id_report_the_smaller_memory() {
    assert_eq!(parse_nvidia_smi("0x2C0510DE, 16303\n0x2C0510DE, 8192\n", NVIDIA_VENDOR, 0x2c05), Some(8192 << 20));
    assert_eq!(parse_nvidia_smi("0x2C0510DE, 8192\n0x2C0510DE, 16303\n", NVIDIA_VENDOR, 0x2c05), Some(8192 << 20));
    assert_eq!(parse_nvidia_smi("0x2C0510DE, lots\n0x2C0510DE, 16303\n", NVIDIA_VENDOR, 0x2c05), Some(16303 << 20), "a bad line is skipped");
    assert_eq!(parse_nvidia_smi("0x2C0510DE, 0\n0x2C0510DE, 16303\n", NVIDIA_VENDOR, 0x2c05), Some(16303 << 20), "a zero size is no answer");
}

/// `nvidia-smi` runs on the UI thread at startup, so a query the driver never answers must
/// give up at the deadline instead of keeping the window from opening.
#[cfg(any(unix, windows))]
#[test]
fn a_child_that_never_answers_gives_up_at_the_deadline() {
    let slow = if cfg!(windows) {
        let mut c = std::process::Command::new("ping");
        c.args(["-n", "11", "127.0.0.1"]);
        c
    } else {
        let mut c = std::process::Command::new("sleep");
        c.arg("10");
        c
    };
    let start = std::time::Instant::now();
    assert_eq!(bounded_output(slow, std::time::Duration::from_millis(200)), None);
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "took {:?}", start.elapsed());

    let quick = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "echo 0x2C0510DE, 16303"]);
        c
    } else {
        let mut c = std::process::Command::new("printf");
        c.arg("0x2C0510DE, 16303\n");
        c
    };
    let out = bounded_output(quick, std::time::Duration::from_secs(10)).expect("a quick command answers");
    assert_eq!(parse_nvidia_smi(&String::from_utf8_lossy(&out), NVIDIA_VENDOR, 0x2c05), Some(16303 << 20));

    let failing = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "exit 3"]);
        c
    } else {
        std::process::Command::new("false")
    };
    assert_eq!(bounded_output(failing, std::time::Duration::from_secs(10)), None, "a failed run");
    assert_eq!(bounded_output(std::process::Command::new("photocraft-no-such-tool"), std::time::Duration::from_secs(1)), None, "not installed");
}

/// Only a discrete NVIDIA adapter asks `nvidia-smi`: nothing else spawns a process whose
/// answer it would then ignore anyway (`nvidia_video_memory` gates on the vendor id first).
#[cfg(target_os = "linux")]
#[test]
fn nvidia_video_memory_only_asks_for_an_nvidia_vendor_id() {
    assert_eq!(nvidia_video_memory(0x1002, 0x73bf), None, "an AMD id never runs nvidia-smi");
    assert_eq!(nvidia_video_memory(0x8086, 0x9a49), None, "an Intel id never runs nvidia-smi");
}

/// Windows used to report no RAM at all, so the budget assumed 16 GB on every machine.
#[cfg(windows)]
#[test]
fn windows_reports_installed_ram() {
    let ram = detect_physical_memory().unwrap_or(0);
    assert!(ram >= 256 << 20, "{ram} bytes");
    assert!(ram < 1 << 50, "{ram} bytes");
}

#[test]
fn canvas_texture_bytes_price_every_tile_and_mip_level() {
    // The same sum DocTextures builds tile by tile (and texture_info reports).
    let brute = |size: [u32; 2], tile: u32, bpp: u64| -> u64 {
        let mut total = 0;
        for ty in (0..size[1]).step_by(tile as usize) {
            for tx in (0..size[0]).step_by(tile as usize) {
                let (w, h) = (tile.min(size[0] - tx), tile.min(size[1] - ty));
                total += (0..mip_count(w, h)).map(|l| u64::from((w >> l).max(1)) * u64::from((h >> l).max(1)) * bpp).sum::<u64>();
            }
        }
        total
    };
    for (size, tile) in [([1, 1], 64), ([300, 200], 64), ([128, 64], 64), ([1000, 7], 256), ([8192, 8193], 8192), ([20_000, 9_000], 8192)] {
        for bpp in [4, 8] {
            assert_eq!(canvas_texture_bytes(size, tile, bpp), brute(size, tile, bpp), "{size:?} tile {tile}");
        }
    }
    assert_eq!(canvas_texture_bytes([0, 500], 64, 4), 0);
    // A canvas declared in a few bytes of a file is priced without building (or looping over) it.
    let huge = canvas_texture_bytes([300_000, 300_000], 8192, 4);
    assert!(huge > 300_000u64 * 300_000 * 4 && huge > canvas_budget(Some(1 << 40), None));
    assert_eq!(canvas_texture_bytes([u32::MAX, u32::MAX], 64, 8), u64::MAX, "saturates");
    // A quarter of RAM (16 GB assumed), never below the compositor's floor.
    assert_eq!(canvas_budget(None, None), 4 << 30);
    assert_eq!(canvas_budget(Some(1 << 30), None), MIN_GPU_BUDGET);
    assert!(canvas_texture_bytes([12_000, 8_000], DEFAULT_TILE, 8) < canvas_budget(None, None), "a 96 MP 16-bit canvas stays on the GPU");
}

#[test]
fn view_uniform_packs_rotation() {
    assert_eq!(VIEW_FLOATS, 32);
    assert_eq!(VIEW_UNIFORM_SIZE, 128);
    assert_eq!(VIEW_FLOATS * 4, VIEW_UNIFORM_SIZE as usize);
    assert!(gpu_view_ok(false, 0.0, 1.0));
    assert!(gpu_view_ok(false, 45.0_f32.to_radians(), 1.0));
    assert!(!gpu_view_ok(true, 45.0_f32.to_radians(), 1.0));
    assert!(!gpu_view_ok(false, 0.0, 2.0), "pixel aspect correction draws on the CPU");
    // Identity packing: rotation 0 matches the unrotated origin + scale map.
    let origin = [10.0, 20.0];
    let (scale, center) = (2.0, [40.0, 8.0]);
    let a = map_doc_px(origin, scale, center, 1.0, 0.0, 5.0, 9.0);
    assert!((a[0] - (origin[0] + 5.0 * scale)).abs() < 1e-4);
    assert!((a[1] - (origin[1] + 9.0 * scale)).abs() < 1e-4);
    let (x0, y0, x1, y1) = tile_screen_aabb(origin, scale, center, 1.0, 0.0, [0.0, 0.0, 10.0, 4.0]);
    assert!((x0 - origin[0]).abs() < 1e-4 && (y0 - origin[1]).abs() < 1e-4);
    assert!((x1 - (origin[0] + 20.0)).abs() < 1e-4 && (y1 - (origin[1] + 8.0)).abs() < 1e-4);
}

#[test]
fn filter_modes() {
    assert_eq!(filter_mode(0.25), (0.0, 2.0));
    assert_eq!(filter_mode(1.0).0, 2.0);
    assert_eq!(filter_mode(1.5).0, 1.0);
    assert_eq!(filter_mode(3.3).0, 2.0);
}

#[test]
fn mips_and_premultiply() {
    assert_eq!(mip_count(1, 1), 1);
    assert_eq!(mip_count(4096, 100), 13);
    assert_eq!(mip_count(5, 3), 3);
    assert_eq!(premultiply_rgba8(&[[1.0, 0.5, 0.0, 0.5]]), vec![128, 64, 0, 128]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn half_floats() {
    // Every finite half survives the round trip; ties round to even; overflow saturates.
    for h in 0..=u16::MAX {
        if h & 0x7c00 != 0x7c00 && h != 0x8000 {
            assert_eq!(f32_to_f16(f16_to_f32(h)), h, "{h:#06x}");
        }
    }
    assert_eq!(f32_to_f16(1.0), 0x3c00);
    assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3c00);
    assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
    assert_eq!(f32_to_f16(2f32.powi(-24)), 1);
    assert_eq!(f32_to_f16(2f32.powi(-26)), 0);
    assert_eq!(f32_to_f16(65504.0), 0x7bff);
    assert_eq!(f32_to_f16(1e9), 0x7bff);
    assert_eq!(f32_to_f16(f32::INFINITY), 0x7bff);
    assert_eq!(f32_to_f16(f32::NAN), 0);
    assert_eq!(f32_to_f16(-2.0), 0xc000);
    assert_eq!(premultiply_rgba16f(&[[2.0, -1.0, f32::NAN, 0.5]]), [0x00, 0x3c, 0, 0, 0, 0, 0x00, 0x38]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn shaders_validate() {
    use wgpu::naga;
    for (name, src) in [("canvas", CANVAS_WGSL), ("mip", MIP_WGSL), ("encode", ENCODE_WGSL)] {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
}
