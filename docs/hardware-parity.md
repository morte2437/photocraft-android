# Hardware parity

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-11-08 · **Change:** minor (Graphics Adapter choice on Linux, #1885; Camera Raw's preview develops its per-pixel stages on the GPU; GPU, Wayland (tilt and eraser end via Xwayland) and monitor-profile rows refreshed against main; Windows: flipping the pen to its eraser end selects the Eraser with Windows Ink on, #2285, not yet confirmed on a pen with an eraser end; previously minor: optional WinTab backend on Windows: pressure and the eraser end; tilt on devices with orientation axes; rotation only with a pen and driver that report twist, with Windows Ink off) · **Target:** Adobe Photoshop 2026

Each hardware feature Photoshop 2026 (27.11.0) uses, per platform, against PhotoCraft. Photoshop
side: its bundle (Metal and OpenCL paths, "Precise Color Management for HDR Display" and EDR
strings, Touch Bar resources, pen pressure/tilt strings) and Adobe's GPU FAQ. PhotoCraft side:
`crates/tablet`, `crates/ui-egui/src/stylus.rs`, `apps/photocraft/src/gpu_startup.rs`,
`engine/src/display_color.rs`, `apps/photocraft/src/monitor_profile.rs`.

**Summary (estimated):** ~50% ready. GPU compositing and colour-managed display are solid; pen
input is uneven by platform (the default Windows Ink path lacks tilt and rotation, Wayland is
broken; the opt-in WinTab path on Windows supplies tilt only on devices that report orientation
axes, and rotation only when the pen and driver report twist) and there is no HDR
output. Remaining: 80–150 h (gap G12). Human: tablets (Wacom, XP-Pen, Huion) on each OS and an
HDR display to verify.

## GPU

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| GPU canvas, zoom/pan, rotate view | Metal (macOS), DirectX/OpenCL (Windows) | wgpu 30: Metal, Vulkan, DX12, GL; backend selectable | done |
| GPU compositing of layers, blend modes, effects | partial (Photoshop composites mostly on CPU) | wgpu compositor, ≤ 1/255 vs CPU oracle; Multichannel and > texture-limit regions fall back to CPU | done (ahead in places) |
| GPU compositor startup | available | Pipeline preparation runs in the background on native hardware; the CPU composites until ready. Windows WARP probe: 0.329 ms p95, 6.860 ms maximum device-work stall during two FXC builds; hardware measurements remain pending (#9157, #9364, #6419) | partial |
| GPU filters (Blur Gallery, Liquify, Smart Sharpen, Camera Raw) | yes | CPU (rayon) for filters; GPU for compose and effects. Camera Raw's preview runs its per-pixel stages (white balance to grain) in one wgpu pass, about 2-3 ms vs 11-28 ms on the CPU for the 858×572 proxy, depending on the machine, Shadows and Highlights (the local Laplacian filter) as fragment passes in front of it, 8.7-10.6 ms vs 147-300 ms, and Dehaze, Clarity and Texture (box, minimum and guided filters) as fragment passes too, 2.7-7.7 ms vs 7.6-17.7 ms each (Dehaze's dark channel opened as in the CPU develop, #9273); with all of them and every per-pixel stage 17-24 ms vs 222 ms on an RTX 5060 Ti. Detail, Calibration and OK still develop on the CPU (#6916) | partial |
| Documents larger than the GPU texture limit | yes (tiled) | paged GPU residency, composited page cell by page cell (`crates/gpu/src/lib.rs`, #49 closed) | done |
| CPU-only fallback when no GPU | yes, reduced | `cpu` canvas preference; fails to start without an adapter on some Linux systems (#2519, #2412) | partial |
| Choosing the GPU on a multi-GPU machine | Preferences › Performance › Use Graphics Processor | Linux only: Preferences › Performance › Advanced › GPU adapter and `PHOTOCRAFT_GPU_ADAPTER` (#1885); ignored on Windows and macOS | partial |
| Old / integrated GPUs | yes | black line on Intel HD (#2426), OpenCore Macs fail (#1859); the Rgba32Float panic is fixed (#2020) | partial |

## Pen tablets

| Feature | Photoshop | macOS | Windows | Linux X11 | Linux Wayland | Web |
|---|---|---|---|---|---|---|
| Pressure | yes | yes (AppKit monitor) | yes (WM_POINTER), jumpy (#2046) | yes (XInput2) | via Xwayland only; GNOME 50 pen doesn't click (#2622); KDE fixed in 0.6.0 (#2297, #1070) | yes |
| Tilt | yes | yes | **no** (WinTab: with orientation axes, opt-in) | yes | via Xwayland only (native Wayland: no) | yes |
| Barrel rotation | yes | yes (not applied to tip angle, #1301) | no (WinTab: with a twist-reporting pen and driver, opt-in) | partial | no | yes (twist) |
| Eraser end | yes | yes | yes with Windows Ink, not yet confirmed on hardware (#2285); WinTab: yes (opt-in) | partial | partial, via Xwayland only (native Wayland: no) | partial |
| WinTab driver mode | yes | – | yes (opt-in; Windows Ink off) | – | – | – |
| Pressure curve | Windows Ink / driver | global curve in Preferences (#2195) | same | same | same | same |

Windows: the opt-in WinTab backend (`tools.tabletInput = "winTab"`, with Windows Ink turned off
in the tablet utility) supplies pressure and the eraser end, tilt only on devices that report
orientation axes, and rotation only when the pen and driver report twist (pressure-only devices
get neutral tilt and rotation). The default Windows Ink path still lacks tilt and
rotation; its eraser end comes from the pointer hooks (#2285).

## Displays and colour

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| Colour-managed canvas (document → monitor profile) | yes | yes, folded into the GPU 3D LUT | done |
| Monitor profile detection | per display, live | macOS and Linux X11 (per XRandR output, #2962) per display, following each window's display (#591); Windows and Wayland: sRGB or a chosen .icc | partial |
| HDR / EDR output for 32-bit and HDR images | yes ("Precise Color Management for HDR Display") | no; 8-bit sRGB-encoded canvas texture | missing |
| High-DPI, physical 100% | yes | yes (#1943) | done |
| Multiple monitors: documents in separate OS windows | yes | yes (`extra_windows`) | done |
| Panels in separate OS windows | yes | in-app floating only (UI-217-5) | missing |

## Input devices

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| Trackpad pinch zoom, two-finger pan | yes | yes; pinch scrolls instead in 0.6.0 (#2463) | partial |
| Trackpad rotate (Rotate View) | yes | yes (`rotate_view.rs`) | done |
| Touch Bar | yes (Intel Macs) | no | missing (low value) |
| Touch screens (Windows) | yes | basic pointer only | partial |
| 3D mouse, MIDI, Surface Dial | Surface Dial on Windows | no | missing (low value) |
| Scanners (WIA / TWAIN) | WIA menu (Windows) | menu item live; no driver integration | partial |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-06 | minor | Windows eraser end: the pointer hooks read `PEN_FLAG_INVERTED` and the Eraser switch follows it (Windows Ink on; #2285) |
| 2026-11-05 | minor | Optional WinTab driver mode on Windows: pressure and eraser; tilt with orientation axes; rotation with a twist-reporting pen, with Windows Ink off |
| 2026-10-28 | minor | Build GPU compositor pipelines in the background; benchmark baseline pending (#9157) |
| 2026-10-10 | major | First version |
| 2026-10-12 | minor | GPU rows (documents past the texture limit, Rgba32Float panic), Wayland KDE pen and monitor-profile detection refreshed against main |
| 2026-10-17 | minor | GPU filters: Camera Raw's preview, per-pixel stages (#6916 step 1) |
| 2026-10-17 | minor | GPU filters: Camera Raw's preview, Shadows and Highlights (#6916 step 2) |
| 2026-10-17 | minor | GPU filters: Camera Raw's preview, Dehaze, Clarity and Texture (#6916 step 3) |
| 2026-10-19 | minor | Graphics Adapter choice on Linux (#1885) |
| 2026-11-08 | minor | Linux Wayland: tilt and the eraser end work through Xwayland, like pressure; native Wayland still has no pen |
