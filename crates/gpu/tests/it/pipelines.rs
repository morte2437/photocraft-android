//! The compositor's WGSL must compile with the platform's own shader compiler, not only with naga.
//!
//! On DX12 wgpu translates the WGSL to HLSL and compiles it with FXC, unless `dxcompiler.dll`
//! happens to be on `PATH`, where it prefers DXC — which is why the first instance below forces
//! FXC. FXC's flow analysis stops following shapes WGSL allows (a switch whose every case returns,
//! past thirteen cases) and reports `error X3507: 'blend_channel': Not all control paths return a
//! value`. naga validates that same shader, so nothing caught it but the real compiler: no
//! compositor pipeline built, and DX12 users composited every document on the CPU for the session.
//!
//! The Windows CI runners have no GPU: their adapter is WARP over DX12, so this runs FXC there.

use crate::GPU_LOCK;

#[test]
fn every_compositor_pipeline_builds() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // FXC first: it is the compiler that rejected the shader, and the lowest shader model wgpu has.
    // Every backend then covers the rest of their own translations (Vulkan, Metal, DX12 through DXC).
    // `InstanceFlags::DEBUG` (the debug-build default) makes wgpu pass `D3DCOMPILE_DEBUG |
    // D3DCOMPILE_SKIP_OPTIMIZATION`, and FXC then accepts what it rejects in a release build, so
    // the flags are cleared here: a release build is what ships.
    let describe = |backends: wgpu::Backends, compiler: wgpu::Dx12Compiler| {
        let mut desc = wgpu::InstanceDescriptor { backends, flags: wgpu::InstanceFlags::empty(), ..wgpu::InstanceDescriptor::new_without_display_handle() };
        desc.backend_options.dx12.shader_compiler = compiler;
        desc
    };
    // One instance per backend: `Backends::all()` only enables the others on one instance, and
    // `request_adapter` then returns the single default adapter, so the other backends' own
    // translation went untested. One adapter per backend keeps a second, unrelated adapter of the
    // same backend (a dual-GPU laptop's integrated GPU) out of the run.
    for (backends, compiler) in [(wgpu::Backends::DX12, wgpu::Dx12Compiler::Fxc), (wgpu::Backends::all(), wgpu::Dx12Compiler::Auto)] {
        for backend in backends.iter() {
            let instance = wgpu::Instance::new(describe(backend, compiler.clone()));
            // The Windows runners always have a WARP adapter on DX12, so a missing one there means
            // the check this test exists for silently did not run. Any other backend is a plain
            // skip: `Backends::all()` also names backends this platform does not provide.
            let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
                #[cfg(windows)]
                assert!(backend != wgpu::Backends::DX12, "no adapter on Windows: the DX12/FXC pipeline check did not run");
                continue;
            };
            let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
                #[cfg(windows)]
                assert!(backend != wgpu::Backends::DX12, "no device on Windows: the DX12/FXC pipeline check did not run");
                continue;
            };
            let info = adapter.get_info();
            let format = photocraft_gpu::Compositor::preferred_acc_format(&adapter);
            if let Err(e) = photocraft_gpu::Compositor::try_new_with_format(&device, format) {
                panic!("{:?} adapter {} (driver {}): {e}", info.backend, info.name, info.driver_info);
            }
        }
    }
}
