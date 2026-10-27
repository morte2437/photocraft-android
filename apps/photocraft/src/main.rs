//! PhotoCraft desktop app.
//!
//! Usage: `photocraft [options] [--] [files…]`; `photocraft --help` lists the options, which are
//! declared, with what makes a usage error (exit status 2), in `launch_args`.
//!
//! On Windows and Linux a launch hands its files to the PhotoCraft that is already running (new
//! tabs in its window) and exits; `--new-instance` starts a separate copy instead, as does any
//! `--control` launch (see `single_instance`).
//!
//! `--present-mode` (`fifo`, `autovsync`, `novsync`) and `--frame-latency` (1–3) override how
//! frames reach the display for this launch (`PHOTOCRAFT_PRESENT_MODE` / `…_FRAME_LATENCY`;
//! `presentation`); `--stroke-trace` times each brush stroke's frames (`PHOTOCRAFT_STROKE_TRACE`;
//! `photocraft_ui_egui::stroke_trace`).
//!
//! `--safe-gpu` starts with the CPU renderer (no GPU canvas; a software adapter for the window
//! where the platform has one) for this launch, e.g. after a graphics driver crash. A start that
//! crashes inside the driver also falls back by itself next time (see `gpu_startup`).
//!
//! `--in-window-menus` (or `PHOTOCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`mac_menu`).
//!
//! `--control <port>` (or `PHOTOCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server.
//! The first line must authenticate; subsequent request lines get reply lines.
//! `{"id":1,"ok":true,"result":…}`. See `photocraft_ui_egui::control` for the methods.

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--help`, `--version` and usage errors then only show when
// redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app_dirs;
mod app_icon;
mod appearance;
#[cfg(target_os = "macos")]
mod apple_events;
mod caps_lock;
mod caption_style;
mod control_server;
mod crash_guard;
mod cursor;
mod file_dialog_probe;
mod gpu_startup;
#[cfg(target_os = "linux")]
mod icon_theme;
mod launch_args;
#[cfg(target_os = "macos")]
mod mac_menu;
#[cfg(target_os = "macos")]
mod mac_window;
// Pure logic is tested on every platform; only Linux runs the check.
#[cfg(any(target_os = "linux", test))]
mod linux_libs;
mod logging;
mod monitor_profile;
mod photoshop_settings;
// Save As through the desktop portal, which reports the file type picked (#6121).
#[cfg(target_os = "linux")]
mod portal_save;
mod presentation;
// The print portal needs the session bus (zbus, Linux only); its pure helpers are tested everywhere.
#[cfg(any(target_os = "linux", test))]
mod print_portal;
mod screen_color;
mod services;
// macOS gets one instance from LaunchServices (open-documents Apple events).
#[cfg(any(not(target_os = "macos"), test))]
mod single_instance;
// Tablet support (macOS AppKit monitor, Linux X11/Wayland, Windows barrel button).
mod tablet;
mod ui_state;
mod updates;
mod window_frame;

use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
const APP_ID: &str = "ai.storyteller.photocraft";

/// Windows and Linux: no OS title bar; the app's top bar is the title bar, with its own caption
/// buttons and edge resizing (`photocraft_ui_egui::titlebar`), as Photoshop does on Windows. macOS
/// keeps its traffic lights over the integrated title strip.
const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");

/// Whether this start draws its own title bar: Windows and Linux do, unless Preferences ›
/// Interface › System Title Bar asks for the system's (#1271, #1316). Read from the saved
/// preferences before the window opens; a missing or unreadable file keeps the default.
fn custom_titlebar(prefs_file: Option<&std::path::Path>) -> bool {
    let prefs: photocraft_engine::prefs::Preferences =
        prefs_file.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    CUSTOM_TITLEBAR && !prefs.interface.system_title_bar
}

/// Preferences › Integrations, read before the window opens the way [`custom_titlebar`] is:
/// whether agents may drive this launch at all, and the port to listen on when no `--control`
/// was given (0 = only with `--control`). A missing or unreadable file keeps the defaults.
///
/// The section is read on its own, and when it doesn't read as a whole each value on its own,
/// as the app's lenient load reads them (#6611): a wrong-typed value elsewhere in the file, or
/// in the other field, must not turn Allow Agent Control back on while the dialog shows it off.
fn integrations(prefs_file: Option<&std::path::Path>) -> photocraft_engine::prefs::Integrations {
    use serde_json::Value;
    let mut out = photocraft_engine::prefs::Preferences::default().integrations;
    let file: Option<Value> = prefs_file.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
    let Some(section) = file.as_ref().and_then(|v| v.get("integrations")) else {
        return out;
    };
    if let Ok(whole) = serde_json::from_value(section.clone()) {
        return whole;
    }
    if let Some(allow) = section.get("allowAgentControl").and_then(Value::as_bool) {
        out.allow_agent_control = allow;
    }
    if let Some(port) = section.get("controlPort").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()) {
        out.control_port = port;
    }
    out
}

/// The control port this launch listens on, and the warning to print when a request was refused.
/// `--control` (or `PHOTOCRAFT_CONTROL_PORT`) wins over the preference; Allow Agent Control off
/// refuses both, and says so rather than ignoring the flag silently. A preferred port outside
/// 1..=65535 is refused the same way.
fn control_port_for(cli: Option<u16>, prefs: &photocraft_engine::prefs::Integrations) -> (Option<u16>, Option<String>) {
    if !prefs.allow_agent_control {
        let warning = cli.map(|p| format!("photocraft: Preferences › Integrations › Allow Agent Control is off: not listening on port {p}\n"));
        return (None, warning);
    }
    if let Some(port) = cli {
        return (Some(port), None);
    }
    match prefs.control_port {
        0 => (None, None),
        p => match u16::try_from(p).ok().filter(|p| *p != 0) {
            Some(port) => (Some(port), None),
            None => (None, Some(format!("photocraft: Preferences › Integrations › Control Port {p} is not a usable port: not listening\n"))),
        },
    }
}

/// The main window: 1440 × 900 (shrunk to fit the monitor, and maximized on the first frame
/// when it still doesn't fit, `work_area::fit_window`), centred on the main monitor. Without
/// `centered`, Windows cascades each new window from the top-left corner, so it opened at a
/// different offset every launch (#419). A later launch restores the saved position instead
/// (`main` turns `centered` off, #4826). Wayland compositors place windows themselves.
fn native_options(custom_titlebar: bool) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default().with_icon(app_icon::viewport_icon(std::env::current_exe().ok().as_deref()));
    eframe::NativeOptions {
        viewport: viewport
            .with_app_id(APP_ID)
            .with_title("PhotoCraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_decorations(!custom_titlebar)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        centered: true,
        // PhotoCraft owns its fallback fonts (`cjk_fonts`, craft-fonts): eframe's system-font
        // provider would compete with it and make glyph choice machine-dependent (#8512).
        system_font_fallback: false,
        // eframe saves native window geometry and egui panel/window sizes on exit.
        // Keep that state beside preferences, including config overrides and portable mode.
        persistence_path: services::config_dir().map(|dir| dir.join("ui.ron")),
        window_builder: Some(Box::new(restored_window)),
        ..Default::default()
    }
}

/// The window eframe restores from `ui.ron`, corrected before it is created: at least the
/// minimum size, and never full screen. PhotoCraft always starts in Standard Screen Mode, but
/// eframe also saves and restores full screen, and a window closed in a full-screen mode is
/// saved full screen and maximized at once: the overlap `view_cmds::screen_mode_commands` keeps
/// apart at run time. [OBSERVED 2026-10-12, Windows 11] Restored that way, View › Screen Mode ›
/// Full Screen did nothing until `F` had cycled through Standard.
fn restored_window(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    at_least_min_size(builder).with_fullscreen(false)
}

/// Grow a restored window that is smaller than the minimum size, keeping its centre. On
/// Windows, eframe saves a minimized window's size (160 pt wide, 64 pt tall once eframe has
/// clamped it), and the minimum size isn't applied to a restored one: PhotoCraft then reopened
/// as a 160 × 64 strip on every launch.
fn at_least_min_size(mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let (Some(size), Some(min)) = (builder.inner_size, builder.min_inner_size) else { return builder };
    let grown = size.max(min);
    if grown != size {
        builder.position = builder.position.map(|pos| pos - (grown - size) / 2.0);
        builder.inner_size = Some(grown);
    }
    builder
}

fn main() -> eframe::Result {
    // First, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    crash_guard::install_hook();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(done) = craft_update::self_update::finish(&craft_update::suite::PHOTOCRAFT, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        if let Err(e) = done {
            eprintln!("PhotoCraft update helper failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // The system font scan can take seconds (minutes on some Macs): run it off the UI thread.
    photocraft_ui_egui::type_tool::warm_fonts();
    // `--help` and `--version` print to standard output and exit 0. A usage error (an unknown
    // option, a missing or bad value, a bad PHOTOCRAFT_CONTROL_PORT: issues #701, #6429, #6431)
    // names the argument on standard error and exits 2, so a launcher sees the failure. Neither
    // panics when the stream is closed or a pipe (`photocraft --help | head -1`).
    let launch = launch_args::parse(launch_args::command(), std::env::args_os().skip(1)).unwrap_or_else(|e| e.exit());
    let launch_args::FileNames { openable: files, unreadable: unreadable_paths } = launch.file_names();
    let launch_args::LaunchArgs {
        files: _,
        safe_gpu,
        in_window_menus,
        new_instance,
        present_mode,
        frame_latency,
        stroke_trace,
        control: control_port,
        auth,
        automation,
        mut launch_options,
    } = launch;

    // The log file lives under the settings directory; opened after the arguments, so `--version`
    // and usage errors leave no file behind. Records logged until now are written to it first.
    if let (Some(logger), Some(dir)) = (logger, services::config_dir()) {
        match logger.attach_dir(&dir.join("logs")) {
            Ok(path) => log::info!("PhotoCraft {}, log file {}", photocraft_engine::build_info::long_version(), path.display()),
            // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
            Err(e) => log::warn!("no log file: {e}"),
        }
    }

    // An adapter named in the environment is chosen at startup, which a forwarded launch never reaches.
    let rendering_mode = gpu_startup::read_rendering_prefs(services::prefs_file().as_deref()).1;
    if gpu_startup::adapter_override_is_process_specific(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), rendering_mode, gpu_startup::ADAPTER_CHOICE) {
        launch_options.push(launch_args::PerProcess::GpuAdapterEnv);
    }
    // One PhotoCraft per user (#3708): files opened from Explorer or a file manager while it runs
    // become tabs in its window. Only with the per-user settings folder (a portable copy's may be
    // shared between users), and not for automation, `--new-instance` or per-process options.
    #[cfg(not(target_os = "macos"))]
    let single_instance = {
        let new_instance_env = std::env::var("PHOTOCRAFT_NEW_INSTANCE").ok();
        // A variable documented as the equivalent of a per-launch flag counts as that flag: a
        // launch that sets only `PHOTOCRAFT_PRESENT_MODE` (say) has per-process options too, so it
        // keeps its own process instead of handing its files over and being ignored.
        // Folded in here, inside the block that only non-macOS builds compile, so the binding
        // above keeps its `mut` off macOS, where nothing is folded in.
        let mut launch_options = launch_options;
        launch_options.extend_from_slice(&single_instance::env_launch_options());
        let context = single_instance::LaunchContext {
            per_user_settings: app_dirs::current().mode == app_dirs::Mode::Platform,
            new_instance_flag: new_instance,
            new_instance_env: new_instance_env.as_deref(),
            control: control_port.is_some(),
            unreadable_paths: !unreadable_paths.is_empty(),
            launch_options: &launch_options,
        };
        match (single_instance::skip_reason(&context), services::config_dir()) {
            (Some(reason), _) => {
                log::info!("running as a separate PhotoCraft: {reason}");
                None
            }
            (None, None) => None,
            (None, Some(dir)) => {
                let paths: Vec<String> = files.iter().map(|f| single_instance::absolute(f)).collect();
                match single_instance::claim(&dir, &paths) {
                    single_instance::Launch::Forwarded => {
                        log::info!("handed {} file(s) to the running PhotoCraft", paths.len());
                        return Ok(());
                    }
                    single_instance::Launch::Primary(primary) => Some(primary),
                    single_instance::Launch::Standalone(reason) => {
                        log::warn!("starting a separate PhotoCraft: {reason}");
                        None
                    }
                }
            }
        }
    };
    // Lives until the event loop returns, then `shutdown` lets the next launch become the primary.
    #[cfg(not(target_os = "macos"))]
    let single_instance = &single_instance;
    #[cfg(target_os = "macos")]
    let _ = (new_instance, &launch_options);

    // The display server to open the window on: Xwayland for a pen on Wayland, which gives this
    // app no pen input (#639).
    #[cfg(target_os = "linux")]
    let native_session = linux_libs::session_from_env(|name| std::env::var(name).ok());
    #[cfg(target_os = "linux")]
    let session = tablet::display_session();
    // Preferences › Performance › Linux display server = X11: Xwayland on a Wayland session too,
    // where native file drops work (winit 0.30 has none on Wayland, #386). Only when Xwayland can
    // run the window ($DISPLAY set, X11 libraries installed); otherwise the session's own.
    #[cfg(target_os = "linux")]
    let session = if session == linux_libs::DisplaySession::Wayland
        && gpu_startup::read_display_server(services::prefs_file().as_deref()) == photocraft_engine::prefs::LinuxDisplayServer::X11
        && std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
        && linux_libs::available(linux_libs::DisplaySession::X11)
    {
        eprintln!("photocraft: opening the window through Xwayland (Preferences › Performance › Linux display server)");
        linux_libs::DisplaySession::X11
    } else {
        session
    };

    // winit and wgpu dlopen the windowing and GPU libraries, and some of those crates panic when
    // one is missing (issue #201). Name the package to install and exit instead.
    #[cfg(target_os = "linux")]
    if let Err(message) = linux_libs::preflight(session) {
        eprint!("{message}");
        std::process::exit(1);
    }

    let (control_port, control_pref_warning) = control_port_for(control_port, &integrations(services::prefs_file().as_deref()));
    if let Some(warning) = &control_pref_warning {
        eprint!("{warning}");
    }
    let control = if let Some(port) = control_port {
        let (supplied, token_file) = (auth.control_token, auth.control_token_file);
        let token = match photocraft_automation::security::server_token(supplied.as_deref(), token_file.as_deref()) {
            Ok(token) => token,
            Err(e) => {
                eprintln!("photocraft: cannot configure control authentication: {e}");
                std::process::exit(2);
            }
        };
        if let Some(path) = token_file {
            eprintln!("photocraft: control token file: {}", path.display());
        } else if supplied.is_none() {
            eprintln!("photocraft: control token: {token}");
        } else {
            eprintln!("photocraft: using supplied control token");
        }
        let workspace = match automation.workspace() {
            Ok(workspace) => workspace,
            Err(error) => {
                eprintln!("photocraft: cannot configure automation workspace: {error}");
                std::process::exit(2);
            }
        };
        Some((port, token, workspace))
    } else {
        None
    };
    // Finder / Dock / Open With deliver files as Apple events, not arguments; catch the one that
    // launched us as well as later ones. Lives until the event loop returns.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;

    // The macOS pen tablet monitor (installed once eframe created the app, see below); it lives
    // until the event loop returns.
    #[cfg(target_os = "macos")]
    let tablet_monitor = std::cell::OnceCell::new();
    #[cfg(target_os = "macos")]
    let tablet_monitor = &tablet_monitor;
    // The Windows pointer message hook; dropping it unhooks when the event loop returns.
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = std::cell::OnceCell::new();
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = &tablet_monitor_win;

    // Read the displays' ICC profiles while the window opens (colour-managed canvas; `None`
    // where the platform has no reader).
    #[cfg(target_os = "linux")]
    let monitor = monitor_profile::detect_async_for_session(native_session);
    #[cfg(not(target_os = "linux"))]
    let monitor = monitor_profile::detect_async();
    // Brush presets load in the background; the app attaches them when they arrive.
    let presets = services::presets_dir().map(photocraft_engine::preset_store::open_dir_async);
    let custom_titlebar = custom_titlebar(services::prefs_file().as_deref());
    let mut options = native_options(custom_titlebar);
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; open on X11 when that was chosen.
    #[cfg(target_os = "linux")]
    if session == linux_libs::DisplaySession::X11 {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_x11();
        }));
    }
    // eframe restores the saved window layout before our code runs; drop values that would crash it.
    ui_state::sanitize(options.persistence_path.as_deref());
    // Centre only the first window: eframe centres after restoring the saved position, so a
    // window closed on another monitor reopened in the middle of the main one (#4826).
    options.centered = !ui_state::has_saved_position(options.persistence_path.as_deref());
    // Crash-safe GPU startup (#4): pick the backend (a marker left by a start that died in the
    // driver moves to a safer one), and lock this start's marker until the first frames render.
    let t_sentinel = std::time::Instant::now();
    let os = gpu_startup::Os::current();
    let (pref, mode) = gpu_startup::read_rendering_prefs(services::prefs_file().as_deref());
    let (previous, sentinel) = match services::config_dir() {
        Some(dir) => gpu_startup::Sentinel::begin(&dir),
        None => (gpu_startup::Previous::Clean, None),
    };
    let env_backend = std::env::var("WGPU_BACKEND").ok();
    let plan = gpu_startup::plan_with_mode(pref, mode, previous.crashed(), env_backend.as_deref(), safe_gpu, os);
    if let Some(m) = previous.crashed() {
        log::warn!("the previous start didn't finish (GPU backend {}, adapter {:?}); {}", m.backend, m.adapter, plan.reason.as_deref().unwrap_or(""));
    }
    let sentinel: gpu_startup::SharedSentinel = std::sync::Arc::new(std::sync::Mutex::new(sentinel));
    if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
        let backend = match &plan.env {
            Some(v) => format!("env:{v}"),
            None => plan.backend.name().to_string(),
        };
        let marker = gpu_startup::Marker { backend, version: photocraft_engine::build_info::long_version().to_string(), ..Default::default() };
        if let Err(e) = s.write(marker) {
            log::warn!("GPU startup marker: {e}");
        }
    }
    let gpu_note: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU.
    let wanted_adapter =
        gpu_startup::wanted_adapter(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), &gpu_startup::read_gpu_adapter(services::prefs_file().as_deref()));
    gpu_startup::configure(&mut options.wgpu_options.wgpu_setup, &plan, os, wanted_adapter, sentinel.clone(), gpu_note.clone());
    // Preferences › Performance › Low-Latency Drawing (#1343): without vsync a stroke shows about
    // 50 ms sooner; wgpu picks Immediate, else Mailbox, else Fifo by what the driver offers.
    if gpu_startup::read_low_latency(services::prefs_file().as_deref()) {
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    // Testers' present mode and frame latency overrides (#1343), after the preference.
    let present = presentation::Overrides::from_args_or_env(present_mode, frame_latency);
    options.wgpu_options.surface = present.apply(options.wgpu_options.surface);
    let present_desc = presentation::describe(&options.wgpu_options.surface, &present);
    // Without vsync, frames are capped at 240 fps (#4472) so repaint loops don't spin flat out.
    let uncapped = !matches!(
        options.wgpu_options.surface.present_mode,
        eframe::wgpu::PresentMode::Fifo | eframe::wgpu::PresentMode::FifoRelaxed | eframe::wgpu::PresentMode::AutoVsync
    );
    log::info!("presentation: {present_desc}");
    let sentinel_ms = t_sentinel.elapsed().as_secs_f64() * 1000.0;
    log::info!("GPU startup: {:?} ({sentinel_ms:.2} ms)", plan);
    let retry_cpu = !safe_gpu && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu;
    let keep_marker = gpu_startup::keep_marker_after_error(&plan, os);
    let app_created = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let created_in_callback = app_created.clone();
    let started_sentinel = sentinel.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            created_in_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            let automation = control.as_ref().map(|(_, _, workspace)| workspace.clone());
            let mut services = services::native(automation);
            services.preset_store = presets;
            #[cfg(target_os = "linux")]
            let display = tablet::DisplayKind::of(cc);
            #[cfg(target_os = "linux")]
            {
                services.is_wayland = display == Some(tablet::DisplayKind::Wayland);
                file_dialog_probe::start();
                if services.is_wayland {
                    let var = |name| std::env::var(name).ok();
                    services.xwayland_command =
                        services::xwayland_command(var("FLATPAK_ID").as_deref(), var("APPIMAGE").as_deref(), std::env::var_os("DISPLAY").is_some());
                }
            }
            #[cfg(target_os = "linux")]
            let session = {
                let mut session = Session::new();
                session.print_spooler = print_portal::spooler();
                session
            };
            #[cfg(not(target_os = "linux"))]
            let session = Session::new();
            PhotocraftApp::install_type_reset_hooks(&cc.egui_ctx);
            let mut app = PhotocraftApp::new(session, services);
            app.min_frame_ms = uncapped.then_some(1000.0 / 240.0);
            app.integrated_titlebar = cfg!(target_os = "macos");
            app.custom_titlebar = custom_titlebar;
            // Only the title bar's free gap drags the window, never the menus (mac_window.rs).
            #[cfg(target_os = "macos")]
            mac_window::disable_native_title_drag();
            // Long commands and file opens run as background jobs with progress and Cancel (#210).
            app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
            // Displays and their profiles (#569): wait briefly so the first frames already use
            // the right profile; a slower reading is applied when it arrives, and the shell reads
            // again when the displays may have changed.
            if let Some(rx) = monitor {
                #[cfg(target_os = "linux")]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(move || monitor_profile::detect_async_for_session(native_session)));
                }
                #[cfg(not(target_os = "linux"))]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(monitor_profile::detect_async));
                }
                match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(r) => photocraft_ui_egui::monitor_status::apply(&mut app, r),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => photocraft_ui_egui::monitor_status::pending(&mut app, rx),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        photocraft_ui_egui::monitor_status::apply(&mut app, Err("the display profile reader stopped without an answer".into()));
                    }
                }
            }
            // Preferences › Performance › Use Graphics Processor (and the GPU backend: `cpu`
            // composites on the CPU).
            let info = &mut app.perf.gpu_info;
            info.preference = pref.name().to_string();
            info.present = present_desc.clone();
            if let Some(v) = &stroke_trace {
                app.stroke_trace.set(v);
            }
            info.selected = if plan.env.is_some() { "env".into() } else { plan.backend.name().to_string() };
            info.fallback = match (plan.reason.clone(), gpu_note.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()) {
                (Some(reason), Some(note)) => Some(format!("{reason}. {note}")),
                (reason, note) => reason.or(note),
            };
            info.canvas = "cpu".into();
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
                app.perf.gpu_info.set_available(&rs.available_adapters);
                // Every egui texture uploads through this adapter, whichever canvas draws:
                // Iris Xe on DX12 shows stale memory in all-zero texels (texel_guard.rs).
                photocraft_ui_egui::texel_guard::install(&cc.egui_ctx, &rs.adapter.get_info());
                let software_window = rs.adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu;
                if software_window && mode != photocraft_engine::prefs::RenderingMode::Cpu {
                    // Keep an earlier note, such as a requested adapter that couldn't be used.
                    let software = "No compatible hardware graphics adapter; using software graphics.";
                    let fallback = &mut app.perf.gpu_info.fallback;
                    *fallback = Some(match fallback.take() {
                        Some(earlier) => format!("{earlier}. {software}"),
                        None => software.into(),
                    });
                }
                if !software_window
                    && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu
                    && std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_none()
                    && app.session.prefs().performance.effective_rendering_mode() != photocraft_engine::prefs::RenderingMode::Cpu
                {
                    app.set_wgpu(rs);
                } else {
                    // The window still draws with wgpu: record its errors instead of panicking,
                    // and notice when the device is lost (#3438).
                    app.watch_window_device(&rs.device);
                }
            }
            let fallback_reason = std::env::var("PHOTOCRAFT_GPU_STARTUP_FAILURE").ok().or_else(|| {
                (app.perf.gpu_info.canvas == "cpu" && mode != photocraft_engine::prefs::RenderingMode::Cpu)
                    .then(|| app.perf.gpu_info.fallback.clone())
                    .flatten()
            });
            if let Some(reason) = fallback_reason {
                photocraft_ui_egui::gpu_status::queue_fallback_notice(&mut app, &reason);
            }
            app.perf.span("gpuSentinel", sentinel_ms);
            // CPU rendering after a failed GPU start is temporary. Only the
            // recovery-dialog choice may persist it (#3483).
            let remember = gpu_startup::backend_to_remember(&plan);
            app.on_started(move |app| {
                if let Some(s) = started_sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    s.finish();
                }
                if let Some(b) = remember
                    && app.session.prefs().performance.gpu_backend != b
                    && let Err(e) = app.run("prefs.set", serde_json::json!({"path": "performance.gpuBackend", "value": b.name()}))
                {
                    log::warn!("couldn't remember GPU backend {}: {e}", b.name());
                }
            });
            if let Some((port, token, _)) = control {
                let rx = control_server::start(port, token, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            #[cfg(target_os = "macos")]
            {
                app.services.os_events = Some(apple_events.connect(&cc.egui_ctx));
                // Quit through AppKit, not by closing the window (Touch Bar Macs, #1575, #1458).
                app.services.quit = Some(Box::new(mac_window::terminate_later));
                // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                if !in_window_menus {
                    app.services.native_menu = mac_menu::install(&cc.egui_ctx, &app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = in_window_menus;
            // Files from later launches (Explorer, file managers) open here as new tabs.
            #[cfg(not(target_os = "macos"))]
            if let Some(primary) = single_instance {
                app.services.os_events = Some(primary.connect(&cc.egui_ctx));
            }
            // Where file drags and drops are (winit 0.30 doesn't say).
            app.services.cursor_pos = cursor::service(cc);
            // The custom title bar's undecorated window must stay borderless, or its caption and
            // border offset the content and the pointer on some Windows systems (#2246).
            app.services.window_frame = window_frame::service(cc, custom_titlebar);
            // Linux: the desktop's caption buttons (GNOME's, KDE's) on the sides its layout puts them.
            if custom_titlebar {
                app.services.caption_style = caption_style::service();
            }
            // Caps Lock state (X11/Windows/macOS; `None` on native Wayland): the canvas shows the
            // precise crosshair for painting tools while it is toggled on (#1758).
            app.services.caps_lock = caps_lock::service(cc);
            app.services.screen_pick = screen_color::service(cc, app.services.is_wayland);
            // Tablet pressure/tilt/eraser (winit drops them): the macOS monitor and the X11 reader
            // write into the stylus feed. The monitor goes in here, not before the event loop:
            // AppKit's shared application only exists once winit created it (#759).
            #[cfg(target_os = "macos")]
            if let Some(monitor) = tablet::install_macos(&app.stylus.feed) {
                let _ = tablet_monitor.set(monitor);
            }
            #[cfg(target_os = "windows")]
            if let Some(monitor) = tablet::install_windows() {
                let _ = tablet_monitor_win.set(monitor);
            }
            #[cfg(target_os = "linux")]
            tablet::spawn_x11(&app.stylus.feed, display);
            app.services.right_button_down = tablet::right_button_service();
            // Paths on the command line (Linux/Windows file associations, `photocraft a.psd`).
            app.open_paths(&files);
            if !unreadable_paths.is_empty() {
                use photocraft_ui_egui::i18n;
                let line = i18n::t("{path}: the path is not valid Unicode; rename the file and open it again.");
                let lines = unreadable_paths.iter().map(|p| i18n::fmt(line, &[("path", p.as_str())])).collect();
                photocraft_ui_egui::notices::post(&mut app, i18n::t("Could not open"), lines, true, None);
            }
            // Portable marker found but its data folder isn't writable (#228): say where settings went.
            if let Some(w) = &app_dirs::current().warning {
                photocraft_ui_egui::notices::post(&mut app, "Portable mode is off", vec![w.clone()], false, None);
            }
            Ok(Box::new(app))
        }),
    );
    // The window is gone: refuse further files and free the lock, so a launch that is waiting (or
    // the CPU-compatibility relaunch below) becomes the primary instead of handing its files to a
    // process that is exiting.
    #[cfg(not(target_os = "macos"))]
    if let Some(primary) = single_instance {
        primary.shutdown();
    }
    // Closed or failed outside graphics initialization: not a driver crash. A renderer error
    // keeps the marker, so the next start tries a safer backend, unless there is no safer one:
    // a failed CPU start would otherwise pin every later start to CPU.
    let renderer_error = gpu_startup::keep_marker_after_run(&result);
    if !(renderer_error && keep_marker)
        && let Some(mut s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        if result.is_err()
            && !renderer_error
            && let Some(marker) = previous.crashed()
        {
            // This failure supplies no new graphics-crash evidence: retain the previous
            // marker, rather than recording this attempt's fallback backend.
            if let Err(error) = s.write(marker.clone()) {
                log::warn!("couldn't restore previous GPU startup marker: {error}");
            }
        } else {
            s.finish();
        }
    }
    // Retry in a fresh process: winit event loops cannot be recreated reliably in-process.
    // Only renderer initialization failures qualify; never restart after editing has begun.
    if retry_cpu && !app_created.load(std::sync::atomic::Ordering::Relaxed) && matches!(&result, Err(eframe::Error::Wgpu(_))) {
        let reason = result.as_ref().err().map(ToString::to_string).unwrap_or_default();
        if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            s.finish();
        }
        if let Ok(exe) = std::env::current_exe() {
            // `--safe-gpu` first: after a `--`, it would be a file name.
            let launched = std::process::Command::new(exe)
                .arg("--safe-gpu")
                .args(std::env::args_os().skip(1))
                .env_remove("WGPU_BACKEND")
                .env("PHOTOCRAFT_GPU_STARTUP_FAILURE", &reason)
                .spawn();
            match launched {
                Ok(_) => return Ok(()),
                Err(error) => log::error!("could not start CPU compatibility mode: {error}"),
            }
        }
    }
    result
}

/// Tests that spawn child processes hold this for reading, tests that check who holds a file lock
/// for writing ([`no_child_spawns`]): until its `exec`, a child holds a copy of every open file,
/// `flock`s included, so a spawn in another test could keep a lock this process just dropped held
/// for a moment.
#[cfg(test)]
pub(crate) static CHILD_SPAWNS: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Held by a test that checks who holds a file lock: no other test spawns a child meanwhile.
#[cfg(test)]
pub(crate) fn no_child_spawns() -> std::sync::RwLockWriteGuard<'static, ()> {
    CHILD_SPAWNS.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_and_panel_geometry_survive_a_restart() {
        let options = super::native_options(super::CUSTOM_TITLEBAR);
        assert!(options.persist_window);
        assert_eq!(options.persistence_path, super::services::config_dir().map(|dir| dir.join("ui.ron")));

        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let ctx = egui::Context::default();
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input(), |ui| {
            egui::Panel::right("dock").exact_size(410.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let mut storage = Storage::default();
        ctx.memory(|memory| eframe::set_value(&mut storage, "egui", memory));
        let restored = egui::Context::default();
        restored.memory_mut(|memory| *memory = eframe::get_value(&storage, "egui").unwrap());
        let mut output = restored.run_ui(input(), |ui| {
            egui::Panel::right("dock").default_size(290.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let panel = egui::containers::panel::PanelState::load(&restored, photocraft_ui_egui::panels::panel_state_id(&restored, "dock")).unwrap();
        assert_eq!(panel.size().x, 410.0);
    }

    /// A 0.36 `ui.ron` saved panel widths under `Id::new("dock")`; the first 0.37 launch copies
    /// them to the parent-scoped key, leaves the old one for a rollback, and a later resize
    /// survives two more restarts.
    #[test]
    fn saved_036_panel_widths_migrate_and_survive_two_restarts() {
        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let state = |width: f32| egui::containers::panel::PanelState { outer_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0)) };
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let draw = |ctx: &egui::Context| {
            let mut output = ctx.run_ui(input(), |ui| {
                egui::Panel::right("dock").default_size(290.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
            });
            output.textures_delta.clear();
        };
        let save = |ctx: &egui::Context| {
            let mut storage = Storage::default();
            ctx.memory(|memory| eframe::set_value(&mut storage, "egui", memory));
            storage
        };
        let restore = |storage: &Storage| {
            let ctx = egui::Context::default();
            ctx.memory_mut(|memory| *memory = eframe::get_value(storage, "egui").expect("the saved memory"));
            ctx
        };

        // The real 0.14/0.15 shape: the panel state under the old key inside the egui memory.
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_persisted(egui::Id::unique("dock"), state(410.0)));
        let storage = save(&ctx);

        let first = restore(&storage);
        photocraft_ui_egui::panels::migrate_panel_state(&first);
        draw(&first);
        let key = photocraft_ui_egui::panels::panel_state_id(&first, "dock");
        assert_eq!(egui::containers::panel::PanelState::load(&first, key).map(|p| p.size().x), Some(410.0));
        assert!(egui::containers::panel::PanelState::load(&first, egui::Id::unique("dock")).is_some(), "the 0.36 key is kept");
        // Resize in 0.37: the new key wins from now on.
        first.data_mut(|d| d.insert_persisted(key, state(500.0)));
        let storage = save(&first);

        for restart in 0..2 {
            let next = restore(&storage);
            photocraft_ui_egui::panels::migrate_panel_state(&next);
            draw(&next);
            let key = photocraft_ui_egui::panels::panel_state_id(&next, "dock");
            let width = egui::containers::panel::PanelState::load(&next, key).map(|p| p.size().x).unwrap_or_default();
            assert!((width - 500.0).abs() < 0.5, "restart {restart}: the old 410 key must not win (got {width})");
        }
    }

    /// Schema-probe `ui.ron` shapes *reconstructed* from the documented 0.14/0.15/0.37 layouts,
    /// not recorded machine saves: 0.14 and 0.15 both store the flat window entry, which the
    /// first 0.37 launch rewrites; a 0.37 file is left alone. Keeping all three probes makes a
    /// future change to any migration rule fail here first. They do not prove eframe's real
    /// `Memory`/`PanelState` deserialization.
    #[test]
    fn the_ui_ron_schema_probe_fixtures_migrate() {
        for (name, fixture) in [("0.14", include_str!("../tests/fixtures/ui-0.14.ron")), ("0.15", include_str!("../tests/fixtures/ui-0.15.ron"))] {
            let text = super::ui_state::migrate_for_test(fixture).unwrap_or_else(|e| panic!("{name}: {e}")).unwrap_or_else(|| panic!("{name}: not rewritten"));
            assert!(text.contains("normal_geometry"), "{name}: the flat window entry is gone: {text}");
            assert!(!super::ui_state::saved_position_for_test(fixture), "{name}: a flat entry is not what 0.37 restores");
        }
        let old = include_str!("../tests/fixtures/ui-0.14.ron");
        assert!(super::ui_state::migrate_for_test(old).expect("migratable").expect("rewritten").contains("x:1100.0"), "the 0.14 size is preserved");
        let current = include_str!("../tests/fixtures/ui-0.15.ron");
        assert!(
            super::ui_state::migrate_for_test(current).expect("migratable").expect("rewritten").contains("maximized:true"),
            "0.15 maximized state survives"
        );
        // Already 0.37-shaped: no rewrite, and (with no saved position) the window centres.
        let current = include_str!("../tests/fixtures/ui-0.37.ron");
        assert_eq!(super::ui_state::migrate_for_test(current), Ok(None));
        assert!(!super::ui_state::saved_position_for_test(current));
        // And a 0.37 file with a restorable outer position is a saved position.
        let with_position = current.replace("outer_position_pixels:None", "outer_position_pixels:Some((x:2048.0,y:1228.0))");
        assert!(super::ui_state::saved_position_for_test(&with_position));
    }

    #[test]
    fn the_window_opens_centred_at_its_default_size() {
        let o = super::native_options(super::CUSTOM_TITLEBAR);
        assert!(o.centered, "#419: centred, not cascaded from the top-left corner");
        assert_eq!(o.viewport.inner_size, Some(egui::vec2(1440.0, 900.0)));
        // eframe shrinks the start size to the monitor, so the centred position is on-screen.
        assert_ne!(o.viewport.clamp_size_to_monitor_size, Some(false));
    }

    #[test]
    fn a_window_saved_while_minimized_reopens_at_the_minimum_size() {
        let hook = super::native_options(super::CUSTOM_TITLEBAR).window_builder.expect("a window builder hook");
        // Saved minimized on Windows, then centred on a 1920 × 1080 monitor by eframe.
        let saved = super::native_options(super::CUSTOM_TITLEBAR).viewport.with_inner_size([160.0, 64.0]).with_position([880.0, 508.0]);
        let opened = hook(saved);
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 480.0)));
        assert_eq!(opened.position, Some(egui::pos2(580.0, 300.0)), "still centred");
        // Too narrow only: only the width grows.
        let opened = super::at_least_min_size(super::native_options(false).viewport.with_inner_size([500.0, 900.0]));
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 900.0)));
        // A window at or above the minimum, or with no size, is left alone.
        for size in [[760.0, 480.0], [1100.0, 760.0]] {
            let builder = super::native_options(false).viewport.with_inner_size(size).with_position([10.0, 20.0]);
            let opened = super::at_least_min_size(builder);
            assert_eq!((opened.inner_size, opened.position), (Some(egui::Vec2::from(size)), Some(egui::pos2(10.0, 20.0))));
        }
        let mut builder = super::native_options(false).viewport;
        builder.inner_size = None;
        assert_eq!(super::at_least_min_size(builder).inner_size, None);
    }

    #[test]
    fn a_window_saved_in_full_screen_reopens_maximized_not_full_screen() {
        let hook = super::native_options(super::CUSTOM_TITLEBAR).window_builder.expect("a window builder hook");
        // What eframe restores from a ui.ron saved in a full-screen mode on Windows.
        let saved = super::native_options(super::CUSTOM_TITLEBAR)
            .viewport
            .with_inner_size([2560.0, 1392.0])
            .with_position([0.0, 0.0])
            .with_fullscreen(true)
            .with_maximized(true);
        let opened = hook(saved);
        assert_eq!(opened.fullscreen, Some(false), "Standard Screen Mode is where PhotoCraft starts");
        assert_eq!(opened.maximized, Some(true), "the rest of the saved geometry is kept");
        assert_eq!(opened.inner_size, Some(egui::vec2(2560.0, 1392.0)));
    }

    #[test]
    fn the_system_title_bar_preference_keeps_the_window_decorations() {
        // #1271, #1316: Preferences › Interface › System Title Bar gives the window back its
        // system decorations on Windows and Linux; macOS always has them.
        let dir = std::env::temp_dir().join(format!("pc-titlebar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        assert_eq!(super::custom_titlebar(None), super::CUSTOM_TITLEBAR, "no preferences file: the default");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(super::custom_titlebar(Some(&file)), super::CUSTOM_TITLEBAR, "an unreadable file: the default");
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":true}}"#).unwrap();
        assert!(!super::custom_titlebar(Some(&file)));
        assert_eq!(super::native_options(false).viewport.decorations, Some(true));
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":false}}"#).unwrap();
        assert_eq!(super::custom_titlebar(Some(&file)), super::CUSTOM_TITLEBAR);
        if super::CUSTOM_TITLEBAR {
            assert_eq!(super::native_options(true).viewport.decorations, Some(false));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn integrations_preferences_decide_the_control_port() {
        use photocraft_engine::prefs::Integrations;
        let off = Integrations { allow_agent_control: false, control_port: 0 };
        let on = Integrations { allow_agent_control: true, control_port: 0 };
        let preferred = Integrations { allow_agent_control: true, control_port: 7878 };

        // Nothing asked for, nothing listening.
        assert_eq!(super::control_port_for(None, &on), (None, None));
        // --control wins over the preference.
        assert_eq!(super::control_port_for(Some(9000), &preferred), (Some(9000), None));
        // The preference opens the port when --control is absent.
        assert_eq!(super::control_port_for(None, &preferred), (Some(7878), None));

        // Allow Agent Control off refuses both, and says so when --control asked for one
        // rather than ignoring the flag.
        assert_eq!(super::control_port_for(None, &off), (None, None));
        let (port, warning) = super::control_port_for(Some(9000), &off);
        assert_eq!(port, None);
        assert!(warning.is_some_and(|w| w.contains("Allow Agent Control") && w.contains("9000")), "the refusal names the port");
        let off_with_port = Integrations { allow_agent_control: false, control_port: 7878 };
        assert_eq!(super::control_port_for(None, &off_with_port), (None, None), "off beats a preferred port");

        // A port a hand-edited file could hold, outside 1..=65535, is refused and named.
        let huge = Integrations { allow_agent_control: true, control_port: 70_000 };
        let (port, warning) = super::control_port_for(None, &huge);
        assert_eq!(port, None);
        assert!(warning.is_some_and(|w| w.contains("70000")), "the refusal names the value");
    }

    #[test]
    fn integrations_fall_back_to_the_defaults_without_a_readable_file() {
        let dir = std::env::temp_dir().join(format!("pc-integrations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        let default = photocraft_engine::prefs::Integrations::default();

        assert_eq!(super::integrations(None), default, "no preferences file");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(super::integrations(Some(&file)), default, "an unreadable file");
        std::fs::write(&file, r#"{"integrations":{"allowAgentControl":false,"controlPort":7878}}"#).unwrap();
        let read = super::integrations(Some(&file));
        assert!(!read.allow_agent_control);
        assert_eq!(read.control_port, 7878);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The app loads preferences leniently (#6611): one wrong-typed value resets only itself. So
    /// a bad value elsewhere, or in the port, must not turn Allow Agent Control back on.
    #[test]
    fn a_bad_value_elsewhere_keeps_agent_control_off() {
        let dir = std::env::temp_dir().join(format!("pc-integrations-lenient-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        // What the Preferences dialog shows for the same file.
        let shown = |text: &str| {
            let mut session = photocraft_engine::Session::new();
            session.load_prefs_json(text).unwrap();
            session.prefs().integrations.clone()
        };
        let read = |text: &str| {
            std::fs::write(&file, text).unwrap();
            let read = super::integrations(Some(&file));
            assert_eq!(read, shown(text), "{text}");
            read
        };

        let r = read(r#"{"interface":{"enableAnimations":"nope"},"integrations":{"allowAgentControl":false,"controlPort":7878}}"#);
        assert!(!r.allow_agent_control, "a bad value in another section");
        assert_eq!(r.control_port, 7878);
        let r = read(r#"{"integrations":{"allowAgentControl":false,"controlPort":"x"}}"#);
        assert!(!r.allow_agent_control, "a bad port");
        assert_eq!(r.control_port, 0, "the bad port alone takes its default");
        // A bad Allow Agent Control alone takes its default.
        let r = read(r#"{"integrations":{"allowAgentControl":"no","controlPort":7878}}"#);
        assert_eq!(r, photocraft_engine::prefs::Integrations { allow_agent_control: true, control_port: 7878 });
        read(r#"{"integrations":{"allowAgentControl":false,"controlPort":-1}}"#);
        read(r#"{"integrations":{"allowAgentControl":false,"controlPort":4294967296}}"#);
        read(r#"{"integrations":"off"}"#);
        // serde reads a struct from a list too; the app does.
        assert!(!read(r#"{"integrations":[false]}"#).allow_agent_control, "a list");
        assert_eq!(read(r#"{"integrations":[false,7878]}"#).control_port, 7878);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// X11 startup notification and docks match a window to its launcher by `StartupWMClass`,
    /// so it has to be the class the window carries: the app id, on Wayland and X11 alike.
    #[test]
    fn the_desktop_entry_names_the_window_class() {
        let entry = include_str!("../../../packaging/linux/ai.storyteller.photocraft.desktop");
        let class = entry.lines().find_map(|line| line.strip_prefix("StartupWMClass="));
        assert_eq!(class, Some(super::APP_ID));
    }
}
