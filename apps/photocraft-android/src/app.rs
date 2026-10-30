//! The Android entry point: GameActivity starts `android_main` on its own thread once the
//! library is loaded.

use crate::bridge::Bridge;
use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::stroke_trace::Summary;
use photocraft_ui_egui::stylus::PenSample;
use photocraft_ui_egui::theme::ThemeKind;
use std::sync::Arc;
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::input::{ImeOptions, InputType, TextInputAction, TextInputState, TextSpan};

/// android-activity looks the entry point up by name, hence `no_mangle` (the crate's only
/// `unsafe`; see `Cargo.toml`).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("PhotoCraft"));
    // Panics end up in logcat rather than vanishing with the process.
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    if let Err(e) = run(app) {
        log::error!("PhotoCraft stopped: {e}");
    }
}

fn run(android: AndroidApp) -> Result<(), String> {
    let files_dir = android.internal_data_path().unwrap_or_else(crate::services::fallback_files_dir);
    // Settings imported last session (Edit › Preferences › Import Settings…) go into place before
    // anything reads them.
    crate::settings::apply_pending(&files_dir);
    log::info!("PhotoCraft {} starting; data in {}", photocraft_engine::build_info::long_version(), files_dir.display());
    let bridge = Arc::new(Bridge::new(&files_dir).map_err(|e| format!("couldn't create the file bridge: {e}"))?);

    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // eframe hands it to winit's event loop (`with_android_app`).
        android_app: Some(android.clone()),
        ..Default::default()
    };
    // The adapter's real texture limits, so big documents stay on the GPU; Vulkan first, GLES
    // on devices without a usable Vulkan driver.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
    }
    let present = present_settings(&mut options, &android);

    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let inbox: photocraft_ui_egui::Inbox = Arc::default();
            // Fonts imported with Type › Import Font… load with the system's on every start.
            photocraft_text::fonts::add_font_dir(crate::fonts::fonts_dir(&files_dir));
            let mut services = crate::services::services(bridge.clone(), &files_dir);
            services.inbox = Some(inbox.clone());
            // The Shortcut Pad (Window › Shortcut Pad) and updates (Help › Check for Updates…):
            // their menu rows go to the app's menus.
            let pad = crate::pad::Pad::new(&files_dir);
            let touch = crate::touch::TouchInput::new(&files_dir);
            let layout = crate::layout::Layout::new(&files_dir);
            let font_outcome = crate::fonts::Outcome::default();
            // The keyboard opening or closing (and a cutout or corner change) lays the app out
            // again right away, not on the next touch.
            let wake = cc.egui_ctx.clone();
            photocraft_tablet::android::on_layout_change(move || wake.request_repaint());
            let wake = cc.egui_ctx.clone();
            let fonts_menu = crate::fonts::menu(&files_dir, bridge.clone(), font_outcome.clone(), move || wake.request_repaint());
            let wake = cc.egui_ctx.clone();
            let settings_menu = crate::settings::menu(&files_dir, bridge.clone(), font_outcome.clone(), move || wake.request_repaint());
            services.platform_menu =
                Some(crate::updates::combine(vec![layout.menu(), pad.menu(), touch.menu(), fonts_menu, settings_menu, crate::updates::menu(&files_dir, bridge.clone())]));
            crate::services::watch_incoming(bridge.clone(), inbox, cc.egui_ctx.clone());
            let mut app = PhotocraftApp::new(Session::new(), services);
            app.background_jobs = true;
            // No theme set here: `set_theme` saves its choice as the preference, and this ran
            // before the saved preferences load, so it replaced the user's theme on every start.
            // The first frame applies the saved one (Preferences › Interface), as on the desktop.
            // The pen's pressure comes from the Activity; a finger's touch "force" is its
            // contact size, which would paint near-invisible strokes.
            app.stylus.touch_force = false;
            enable_stroke_trace(&mut app, &android);
            app.perf.gpu_info.present = present;
            let feed = app.stylus.feed.clone();
            let ctx = cc.egui_ctx.clone();
            let first = std::sync::atomic::AtomicBool::new(true);
            let installed = photocraft_tablet::android::install(move |s| {
                // Once per run, so logcat shows the pen path is live and what the digitizer reports.
                if let Some(s) = s
                    && first.swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    log::info!("first pen sample: {s:?}");
                }
                feed.set(s.map(|s| PenSample { pressure: s.pressure, tilt_x: s.tilt_x, tilt_y: s.tilt_y, rotation: s.rotation, eraser: s.eraser }));
                ctx.request_repaint();
            });
            if let Err(e) = installed {
                log::warn!("{e}");
            }
            if let Some(rs) = cc.wgpu_render_state.clone() {
                let info = rs.adapter.get_info();
                log::info!("wgpu adapter: {} ({:?})", info.name, info.backend);
                app.perf.gpu_info.set_adapter(&info);
                if info.device_type != eframe::wgpu::DeviceType::Cpu {
                    app.set_wgpu(rs);
                }
            }
            Ok(Box::new(Shell {
                app,
                android,
                bridge: bridge.clone(),
                ime: String::new(),
                ime_stale: None,
                last_change: (0, 0.0),
                wake_at: None,
                editing: false,
                soft_keyboard: false,
                keys: Default::default(),
                pad,
                font_outcome,
                touch,
                layout,
                pen_path: Vec::new(),
                toggled_from: None,
                last_ime: None,
                ime_hidden: false,
                scrolled_for: None,
                text_bar: Vec::new(),
                text_bar_touch: None,
                text_bar_tap: None,
                paste_pending: false,
                drawn: Default::default(),
                trace_logged: None,
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

/// The stroke latency trace (`docs/development.md`), on when a file named `stroke-trace` is in
/// the app's external data directory (`/sdcard/Android/data/<package>/files`, which `adb` can
/// reach): every frame of each stroke is appended to `stroke-trace.csv` beside it and each
/// stroke's summary goes to logcat (tag `PhotoCraft`). There are no environment variables to set
/// on Android.
fn enable_stroke_trace(app: &mut PhotocraftApp, android: &AndroidApp) {
    let Some(dir) = android.external_data_path() else { return };
    if dir.join("stroke-trace").exists() {
        let csv = dir.join("stroke-trace.csv");
        log::info!("stroke trace on: frames go to {}", csv.display());
        app.stroke_trace.set(&csv.to_string_lossy());
    }
}

/// Latency experiments: the `present` file beside the `stroke-trace` flag sets the swap chain's
/// frame latency and vsync (see [`crate::present`]). Returns how frames are presented, for the
/// trace.
fn present_settings(options: &mut eframe::NativeOptions, android: &AndroidApp) -> String {
    let surface = &mut options.wgpu_options.surface;
    let text = android.external_data_path().and_then(|dir| std::fs::read_to_string(dir.join("present")).ok());
    let (present, problems) = crate::present::parse(text.as_deref().unwrap_or_default());
    for p in problems {
        log::warn!("{p}");
    }
    if let Some(vsync) = present.vsync {
        surface.present_mode = if vsync { eframe::wgpu::PresentMode::Fifo } else { eframe::wgpu::PresentMode::AutoNoVsync };
    }
    surface.desired_maximum_frame_latency = present.latency.or(surface.desired_maximum_frame_latency);
    let latency = surface.desired_maximum_frame_latency.map_or("default".to_string(), |n| n.to_string());
    let description = format!("{:?}, latency {latency}", surface.present_mode);
    log::info!("presentation: {description}");
    description
}

/// A named section on Perfetto's timeline, when a trace is running (`adb shell perfetto`).
fn section(name: &str) -> Option<ndk::trace::Section> {
    ndk::trace::is_trace_enabled().then(|| ndk::trace::Section::new(name).ok()).flatten()
}

/// The app plus what the Android shell adds each frame: on-screen keyboard text (see
/// [`crate::ime`]) and hardware keyboard modifiers ([`crate::keys`]).
struct Shell {
    app: PhotocraftApp,
    android: AndroidApp,
    bridge: Arc<Bridge>,
    /// The keyboard's text as last seen.
    ime: String,
    /// The keyboard's text from before it was last cleared. Android applies the clear a frame or
    /// two late, so the old text can show up once more; it's ignored until the keyboard reports
    /// something else (#15: a stale "Hello" and then five Backspaces, which deleted layers).
    ime_stale: Option<String>,
    /// The documents' revisions when they last changed, and when that was (`egui` time), for
    /// [`Shell::autosave_when_idle`].
    last_change: (u64, f64),
    /// Wakes the UI at a given time (see [`idle_waker`]).
    wake_at: Option<std::sync::mpsc::Sender<std::time::Instant>>,
    /// A text field had focus last frame.
    editing: bool,
    /// The Shortcut Pad's keyboard key opened the on-screen keyboard (it stays open, typing
    /// shortcuts, until the key is tapped again).
    soft_keyboard: bool,
    /// Hardware keyboard modifiers (see [`crate::keys`]).
    keys: crate::keys::Held,
    /// The Shortcut Pad (see [`crate::pad`]).
    pad: crate::pad::Pad,
    /// The last Type › Import Font… result, for the status bar.
    font_outcome: crate::fonts::Outcome,
    /// Touch gestures and their settings (see [`crate::touch`]).
    touch: crate::touch::TouchInput,
    /// The focused text field's last keyboard request (where it was on screen), and whether it
    /// was off screen last frame (see the end of `ui`).
    last_ime: Option<egui::output::IMEOutput>,
    ime_hidden: bool,
    /// The Select All / Cut / Copy / Paste bar over the focused text field, as drawn last frame.
    text_bar: Vec<(egui::Rect, TextAction)>,
    /// A touch that started on the bar (the action under it), and a finished tap's action.
    text_bar_touch: Option<(egui::TouchId, TextAction)>,
    text_bar_tap: Option<TextAction>,
    /// A paste waits for the system clipboard's text.
    paste_pending: bool,
    /// The focused field being scrolled into view above the keyboard, and how far so far.
    scrolled_for: Option<(egui::Id, f32)>,
    /// The tool a pen button's "Switch to Eraser and back" left, to return to.
    toggled_from: Option<photocraft_ui_egui::Tool>,
    /// Pen sample positions not yet put into a frame (see [`crate::pen_path`]).
    pen_path: Vec<(f32, f32)>,
    /// Phone or tablet layout (see [`crate::layout`]).
    layout: crate::layout::Layout,
    /// Where the last frame drew everything (see [`crate::touch::Snapshot`]).
    drawn: crate::touch::Snapshot,
    /// The last stroke trace summary written to logcat.
    trace_logged: Option<Summary>,
}

/// An edit from the text bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextAction {
    SelectAll,
    Cut,
    Copy,
    Paste,
}

impl Shell {
    /// Ask for the system clipboard's text, to paste when it arrives.
    fn request_paste(&mut self) {
        match self.bridge.request_clipboard() {
            Ok(()) => self.paste_pending = true,
            Err(e) => log::warn!("clipboard: {e}"),
        }
    }

    /// The Select All / Cut / Copy / Paste bar, above the text field being typed in (below it
    /// when there's no room), as a phone's text fields have. Returns its buttons' places, for
    /// the next frame's taps (see `raw_input_hook`).
    fn show_text_bar(&self, ctx: &egui::Context) -> Vec<(egui::Rect, TextAction)> {
        let Some(field) = self.last_ime.as_ref().map(|i| i.rect) else { return Vec::new() };
        if self.ime_hidden || !ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        const BUTTONS: [(&str, TextAction); 4] =
            [("Select All", TextAction::SelectAll), ("Cut", TextAction::Cut), ("Copy", TextAction::Copy), ("Paste", TextAction::Paste)];
        let (w, h, gap) = (78.0, 34.0, 6.0);
        let total = BUTTONS.len() as f32 * w + (BUTTONS.len() - 1) as f32 * 2.0;
        let screen = ctx.content_rect();
        let x = (field.center().x - total / 2.0).clamp(screen.left() + 4.0, (screen.right() - total - 4.0).max(screen.left() + 4.0));
        let y = if field.top() - h - gap >= screen.top() + 4.0 { field.top() - h - gap } else { field.bottom() + gap };
        let t = photocraft_ui_egui::theme::Tokens::for_kind(self.app.ui.theme);
        let mut places = Vec::new();
        egui::Area::new(egui::Id::unique("android-text-bar")).order(egui::Order::Tooltip).fixed_pos(egui::pos2(x, y)).interactable(false).show(ctx, |ui| {
            let bar = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(total, h));
            ui.painter().rect(bar, 8.0, t.chrome, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
            for (i, (label, action)) in BUTTONS.iter().enumerate() {
                let r = egui::Rect::from_min_size(egui::pos2(x + i as f32 * (w + 2.0), y), egui::vec2(w, h));
                if i > 0 {
                    ui.painter().line_segment(
                        [egui::pos2(r.left() - 1.0, r.top() + 7.0), egui::pos2(r.left() - 1.0, r.bottom() - 7.0)],
                        egui::Stroke::new(1.0, t.separator),
                    );
                }
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(13.0), t.text);
                places.push((r, *action));
            }
            ui.allocate_rect(bar, egui::Sense::hover());
        });
        places
    }
    /// Set the keyboard up for a text field or for shortcuts (`flags` on top of plain text) and
    /// empty its buffer.
    fn reset_keyboard(&mut self, flags: InputType) {
        self.android.set_ime_editor_info(InputType::TYPE_CLASS_TEXT | flags, TextInputAction::None, ImeOptions::IME_FLAG_NO_FULLSCREEN);
        self.android.set_text_input_state(TextInputState { text: String::new(), selection: TextSpan { start: 0, end: 0 }, compose_region: None });
        let old = std::mem::take(&mut self.ime);
        if !old.is_empty() {
            self.ime_stale = Some(old);
        }
    }
}

/// A thread that repaints `ctx` at the last time sent to it (each time replaces the one before).
fn idle_waker(ctx: egui::Context) -> std::sync::mpsc::Sender<std::time::Instant> {
    let (tx, rx) = std::sync::mpsc::channel::<std::time::Instant>();
    std::thread::spawn(move || {
        let mut at: Option<std::time::Instant> = None;
        loop {
            let next = match at {
                None => rx.recv().ok(),
                Some(t) => match rx.recv_timeout(t.saturating_duration_since(std::time::Instant::now())) {
                    Ok(t) => Some(t),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        ctx.request_repaint();
                        at = None;
                        continue;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
                },
            };
            match next {
                Some(t) => at = Some(t),
                None => return,
            }
        }
    });
    tx
}

/// Seconds without changes after which unsaved work is autosaved for recovery.
const IDLE_AUTOSAVE_SECS: f64 = 4.0;

impl Shell {
    /// Android closes apps in the background whenever it likes, and no frame runs once the app
    /// has left the screen, so the autosave timer (Preferences › File Handling) can't be relied
    /// on alone: unsaved work is also autosaved a few seconds after the last change, while the
    /// app is still on screen (#16). Only with autosave on; the save itself runs in the
    /// background and skips documents already saved at their current revision.
    fn autosave_when_idle(&mut self, ctx: &egui::Context) {
        if !self.app.session.prefs().file_handling.autosave {
            return;
        }
        let docs = self.app.session.documents();
        let revisions = docs.iter().filter(|d| d.is_dirty()).fold(docs.len() as u64, |h, d| h.wrapping_mul(31).wrapping_add(d.revision ^ d.doc.id.0));
        let now = ctx.input(|i| i.time);
        if revisions != self.last_change.0 {
            self.last_change = (revisions, now);
            if docs.iter().any(|d| d.is_dirty()) {
                // A frame when the wait is over. `request_repaint_after` doesn't wake an idle
                // app on Android (winit never wakes for it), a repaint from another thread does.
                let at = std::time::Instant::now() + std::time::Duration::from_secs_f64(IDLE_AUTOSAVE_SECS + 0.05);
                let wake = self.wake_at.get_or_insert_with(|| idle_waker(ctx.clone()));
                let _ = wake.send(at);
            }
        } else if self.last_change.1 > 0.0 && now - self.last_change.1 >= IDLE_AUTOSAVE_SECS {
            // Once per change: the next one starts the wait again.
            self.last_change.1 = 0.0;
            photocraft_ui_egui::prefs_ui::autosave_now(&mut self.app);
        }
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _section = section("photocraft:logic");
        // Phone or tablet, decided again every frame (rotation, a foldable opening, split screen).
        let phone = self.layout.is_phone(ctx);
        self.app.compact_layout = phone;
        self.pad.compact = phone;
        self.pad.run_pending(&mut self.app, ctx);
        if let Some((message, error)) = self.font_outcome.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            self.app.ui.status = message;
            self.app.ui.status_error = error;
        }
        self.autosave_when_idle(ctx);
        self.app.logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // The on-screen keyboard takes the bottom of the window: the app lays out in what's left
        // above it (panels, menus and dialogs alike), as apps that resize for the keyboard do.
        let ime = photocraft_tablet::android::ime_height_px() / ctx.pixels_per_point();
        if ime > 1.0
            && let Some(r) = &mut raw_input.screen_rect
        {
            r.max.y = (r.max.y - ime).max(r.min.y + r.height() * 0.3);
        }
        if self.pad.take_keyboard_tap() {
            self.soft_keyboard = !self.soft_keyboard;
            if self.soft_keyboard {
                self.bridge.take_keyboard_hidden();
                if !self.editing {
                    self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS | InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
                }
                // Explicitly (not "implicit"), so Android shows it with no text view focused.
                self.android.show_soft_input(false);
            } else {
                self.android.hide_soft_input(false);
            }
            self.pad.keyboard_on = self.soft_keyboard;
        }
        if self.soft_keyboard && self.bridge.take_keyboard_hidden() {
            // Put away with its own hide key or Back (it covers the pad's key while open).
            self.soft_keyboard = false;
            self.pad.keyboard_on = false;
        }
        // A text field, or the Type tool editing text on the canvas (no egui field has focus
        // then, but its typing comes the same way: text and Backspace/Enter events).
        let canvas_text = self.app.ui.text_edit.is_some();
        let editing = ctx.egui_wants_keyboard_input() || canvas_text;
        if editing && !self.editing {
            // A field just gained focus. Multi-line, so the keyboard's Enter key types a newline
            // into the text state (an editor action would be dropped by winit), which
            // `ime::diff` turns into Enter.
            self.reset_keyboard(InputType::TYPE_TEXT_FLAG_MULTI_LINE);
            if canvas_text {
                // winit opens the keyboard only for egui's own fields.
                self.android.show_soft_input(false);
            }
        }
        if !editing && self.editing {
            if self.soft_keyboard {
                // Back to typing shortcuts.
                self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS | InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
            } else {
                // Focus left the field (Enter, a click elsewhere): put the keyboard away, which
                // winit doesn't, so it stops covering the dialog's buttons.
                self.android.hide_soft_input(false);
            }
        }
        self.editing = editing;
        if editing || self.soft_keyboard {
            let now = self.android.text_input_state().text;
            if self.ime_stale.as_ref().is_some_and(|old| *old != now) {
                self.ime_stale = None;
            }
            if now != self.ime && self.ime_stale.is_none() {
                let edit = crate::ime::diff(&self.ime, &now);
                if editing {
                    raw_input.events.extend(crate::ime::events(&edit));
                    self.ime = now;
                } else {
                    // No field to type into: the letters are shortcuts. Each is handled once,
                    // from an empty buffer again (no suggestions, so nothing is composing).
                    raw_input.events.extend(crate::ime::key_events(&edit));
                    self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS | InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
                }
            }
            // GameActivity's text events don't wake the UI (winit drops them): poll while typing.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // The pad's touches become key presses (before the modifiers are worked out from them)
        // and wheel commands; the rest of the input goes on to the app.
        let config = self.touch.config();
        // The pen's side buttons and eraser end (Touch Input settings): erasing is the pen
        // hook's, right-clicks while touching are the router's, presses are handled here.
        use crate::touch::config::{PenAction, PenControl};
        use photocraft_tablet::{android, motion};
        android::set_erasing_controls(config.erasing_controls());
        android::set_finger_pressure(config.finger_pressure.then_some((config.finger_light, config.finger_firm)));
        let presses = android::take_presses();
        for p in &presses {
            let setting = config.pen(p.control);
            if setting.action == PenAction::ToggleEraser {
                // One press to the Eraser, the next back to the tool it came from.
                use photocraft_ui_egui::Tool;
                let ui = &mut self.app.ui;
                if ui.tool == Tool::Eraser {
                    ui.tool = self.toggled_from.take().unwrap_or(Tool::Brush);
                } else {
                    self.toggled_from = Some(ui.tool);
                    ui.tool = Tool::Eraser;
                }
                continue;
            }
            if setting.action != PenAction::Shortcut {
                continue;
            }
            match crate::pad::config::parse(&setting.shortcut) {
                Ok(chord) if !chord.is_empty() => {
                    raw_input.events.extend(crate::pad::press(chord));
                    raw_input.events.extend(crate::pad::release(chord));
                }
                Ok(_) => {}
                Err(e) => log::warn!("pen {:?}: {e}", p.control),
            }
        }
        let secondary = |id: egui::TouchId| {
            let buttons = android::pointer_buttons(id.0);
            match android::pointer_tool(id.0) {
                Some(motion::tool::MOUSE) => buttons & motion::button::SECONDARY != 0,
                Some(t) => {
                    let used = motion::controls(t, buttons);
                    PenControl::ALL.iter().any(|c| used[c.index()] && config.pen(*c).action == PenAction::RightClick)
                }
                None => false,
            }
        };
        // While Liquify is open its preview is the canvas: two fingers pan and zoom it (#20).
        let liquify = photocraft_ui_egui::liquify_ui::preview_rect(&self.app);
        let env = crate::pad::router::Env {
            // Gestures move the canvas only while a document is open.
            canvas: liquify.or_else(|| self.app.session.active_index().map(|_| self.app.last_canvas_rect)),
            is_finger: &|id: egui::TouchId| photocraft_tablet::android::is_finger(id.0),
            pen_near: photocraft_tablet::android::pen_seen_within(std::time::Duration::from_millis(300)),
            config: &config,
            list_item: &|p: egui::Pos2| self.drawn.list_item(p),
            draggable: &|p: egui::Pos2| self.drawn.draggable(p),
            secondary: &secondary,
            scroll_strip: photocraft_ui_egui::panels::options_bar_overflow(ctx),
            screen: ctx.content_rect(),
            now: raw_input.time.unwrap_or_else(|| ctx.input(|i| i.time)),
        };
        // Every pen sample's position, not just each event's latest (see `pen_path`). Samples
        // wait for a frame with the pen's touch in it; a new touch-down drops older ones.
        let is_pen = |id: egui::TouchId| matches!(android::pointer_tool(id.0), Some(motion::tool::STYLUS | motion::tool::ERASER));
        if raw_input.events.iter().any(|e| matches!(e, egui::Event::Touch { id, phase: egui::TouchPhase::Start, .. } if is_pen(*id))) {
            self.pen_path.clear();
        }
        self.pen_path.extend(android::take_pen_path());
        if self.pen_path.len() > 480 {
            let extra = self.pen_path.len() - 480;
            self.pen_path.drain(..extra);
        }
        if crate::pen_path::splice(&mut raw_input.events, &self.pen_path, ctx.pixels_per_point(), is_pen) > 0 {
            self.pen_path.clear();
        }
        // Taps on the text bar (drawn over the app last frame) never reach the app: a tap
        // anywhere else would take the focus from the field it edits.
        let bar = self.text_bar.clone();
        raw_input.events.retain(|e| {
            let egui::Event::Touch { id, phase, pos, .. } = e else { return true };
            if *phase == egui::TouchPhase::Start
                && let Some(b) = bar.iter().find(|(r, _)| r.contains(*pos))
            {
                self.text_bar_touch = Some((*id, b.1));
            }
            match self.text_bar_touch {
                Some((t, action)) if t == *id => {
                    if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
                        self.text_bar_touch = None;
                        if *phase == egui::TouchPhase::End {
                            self.text_bar_tap = Some(action);
                        }
                    }
                    false
                }
                _ => true,
            }
        });
        let drawn = &self.drawn;
        self.pad.input(ctx, raw_input, &env, |p| !liquify.is_some_and(|r| r.contains(p)) && drawn.covered(p));
        // A focused field the keyboard now covers: scroll its list until the field shows again, as
        // a finger would (a wheel over an empty part of the same list, never over a control, which
        // the wheel would change), a step a frame and no further than SCROLL_LIMIT. Its place is
        // the one it last had on screen, from its last keyboard request.
        const STEP: f32 = 48.0;
        const SCROLL_LIMIT: f32 = 900.0;
        let focus = ctx.memory(|m| m.focused());
        if self.scrolled_for.is_some_and(|(id, _)| Some(id) != focus) {
            self.scrolled_for = None;
        }
        let scrolled = self.scrolled_for.map_or(0.0, |(_, s)| s);
        if self.ime_hidden
            && ime > 1.0
            && scrolled < SCROLL_LIMIT
            && let Some(id) = focus
            && let (Some(field), Some(screen)) = (self.last_ime.as_ref().map(|i| i.rect), raw_input.screen_rect)
        {
            self.scrolled_for = Some((id, scrolled + STEP));
            let bottom = screen.max.y;
            let need = STEP;
            // The list holding the field runs up the field's column from where it was; a dialog's
            // button row can sit at the bottom, so aim well above it.
            let top = screen.min.y;
            let quiet = [0.7, 0.6, 0.5, 0.4]
                .into_iter()
                .map(|f| top + (field.min.y.min(bottom) - top) * f)
                .find_map(|y| self.drawn.quiet_point(field.min.x - 400.0..=field.max.x, y));
            if need > 0.0
                && let Some(p) = quiet
            {
                raw_input.events.push(egui::Event::PointerMoved(p));
                raw_input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -need),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                    source: egui::MouseWheelSource::Wheel,
                });
            }
        }
        // A side button pressed while the pen hovers: a right-click under the pen (after the
        // router, which replaces pointer events with its own).
        let ppp = ctx.pixels_per_point();
        for p in presses {
            if let (Some((x, y)), PenAction::RightClick, PenControl::Lower | PenControl::Upper | PenControl::Top) =
                (p.hover, config.pen(p.control).action, p.control)
            {
                let pos = egui::pos2(x / ppp, y / ppp);
                raw_input.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    raw_input.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
        // Ctrl+V from a hardware keyboard: egui-winit pastes its own (empty) clipboard on
        // Android; paste the system's instead, once it arrives.
        let before = raw_input.events.len();
        raw_input.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
        if raw_input.events.len() != before {
            self.request_paste();
        }
        self.keys.apply(raw_input);
        // The text bar's taps, as the edits they stand for (after the modifiers are worked out,
        // so Select All keeps its Ctrl).
        if let Some(action) = self.text_bar_tap.take() {
            match action {
                TextAction::SelectAll => {
                    for pressed in [true, false] {
                        raw_input.events.push(egui::Event::Key {
                            key: egui::Key::A,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        });
                    }
                }
                TextAction::Cut => raw_input.events.push(egui::Event::Cut),
                TextAction::Copy => raw_input.events.push(egui::Event::Copy),
                TextAction::Paste => self.request_paste(),
            }
        }
        if self.paste_pending {
            match self.bridge.take_clipboard() {
                Some(text) => {
                    self.paste_pending = false;
                    if !text.is_empty() {
                        raw_input.events.push(egui::Event::Paste(text));
                    }
                }
                None => ctx.request_repaint_after(std::time::Duration::from_millis(60)),
            }
        }
        self.app.raw_input_hook(ctx, raw_input);
        // After the app's hook, which starts the trace's frame.
        if let Some(age) = android::take_input_age() {
            self.app.stroke_trace.input_age(age.newest_ms);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _section = section("photocraft:ui");
        let tokens = photocraft_ui_egui::theme::Tokens::for_kind(self.app.ui.theme);
        // A front camera's cutout (punch hole or notch): the bars split around it (#21).
        let ppp = ui.ctx().pixels_per_point();
        let holes = photocraft_tablet::android::cutouts_px().into_iter().map(|[l, t, r, b]| egui::Rect::from_min_max(egui::pos2(l, t) / ppp, egui::pos2(r, b) / ppp)).collect();
        photocraft_ui_egui::cutout::set(ui.ctx(), holes);
        // The screen's rounded corners: controls in their curves move clear of them.
        let corners = photocraft_tablet::android::corners_px()
            .into_iter()
            .zip([(true, true), (false, true), (false, false), (true, false)])
            .filter(|([r, ..], _)| *r > 0.0)
            .map(|([r, x, y], (left, top))| photocraft_ui_egui::cutout::Corner { centre: egui::pos2(x, y) / ppp, radius: r / ppp, left, top })
            .collect();
        photocraft_ui_egui::cutout::set_corners(ui.ctx(), corners);
        // The pad first, so the app lays out beside it.
        self.pad.show(ui, &tokens, &self.app);
        let section = section("photocraft:app");
        self.app.ui(ui, frame);
        drop(section);
        if self.app.stroke_trace.enabled() && self.app.stroke_trace.last != self.trace_logged {
            self.trace_logged = self.app.stroke_trace.last.clone();
            if let Some(summary) = &self.trace_logged {
                log::info!("{}", summary.line());
            }
        }
        self.pad.paint_trackpad_cursor(ui.ctx(), self.touch.config().trackpad);
        self.pad.settings(ui.ctx(), &tokens);
        self.touch.settings(ui.ctx(), &tokens, photocraft_tablet::android::controls_held(), photocraft_tablet::android::last_buttons(), {
            use photocraft_tablet::android;
            android::last_contact().map(|k| crate::touch::Contact { pressure: k.pressure, size: k.size, major: k.major, contact: android::contact_size(&k), mapped: k.mapped })
        });
        // Copy and Cut in a text field go to the system clipboard.
        for command in ui.ctx().output(|o| o.commands.clone()) {
            if let egui::OutputCommand::CopyText(text) = command
                && let Err(e) = self.bridge.set_clipboard(&text)
            {
                log::warn!("clipboard: {e}");
            }
        }
        self.text_bar = self.show_text_bar(ui.ctx());
        // egui asks for the keyboard only while the focused field is on screen. When the layout
        // shrinks above the keyboard and the field ends up below it, the request would stop, the
        // keyboard close, the field come back and ask again: the keyboard flickered open and shut
        // and nothing could be typed (#14). Keep asking while a field has focus.
        let ctx = ui.ctx();
        match ctx.output(|o| o.ime.clone()) {
            Some(ime) => {
                self.last_ime = Some(ime);
                self.ime_hidden = false;
            }
            None if ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input() => {
                self.ime_hidden = true;
                if let Some(ime) = self.last_ime.clone() {
                    ctx.output_mut(|o| o.ime = Some(ime));
                }
            }
            None => {
                self.last_ime = None;
                self.ime_hidden = false;
            }
        }
        // What was drawn where, for the next frame's touches (see `touch::Snapshot`).
        self.drawn = crate::touch::Snapshot::take(ui.ctx());
    }
}
