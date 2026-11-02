use super::{
    CaptionStyle, Document, FileDialogFn, OsEvent, Value, dds_options_ui, jobs_ui, jpeg_options_ui, monitor_status, native_menu, screen_picker,
    targa_options_ui,
};

/// The error a save gets while another save waits on its format prompt.
pub(crate) const SAVE_OPTIONS_OPEN: &str = "Answer the TIFF, Targa, DDS or JPEG Options before starting another save";

/// The format answers a save takes instead of asking: the Targa bits per pixel (`"tgaBits"`),
/// the DDS options (`"ddsCompression"`, `"ddsMipmaps"`) and JPEG Options (`"quality"` 0–12,
/// `"jpegFormat"`). `None` asks in Save As and takes the default in automation saves, which never
/// prompt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FormatChoices {
    pub tga_bits: Option<photocraft_io::TgaBits>,
    pub dds: Option<photocraft_io::DdsOptions>,
    pub jpeg: Option<jpeg_options_ui::JpegChoice>,
}

impl FormatChoices {
    /// The answers in command or control call `cmd`'s params; a bad value is an error.
    pub fn from_params(params: &Value, cmd: &str) -> Result<Self, String> {
        Ok(FormatChoices {
            tga_bits: targa_options_ui::bits_param(params, cmd)?,
            dds: dds_options_ui::options_param(params, cmd)?,
            jpeg: jpeg_options_ui::choice_param(params, cmd)?,
        })
    }

    /// The answers in a recorded save step, read as leniently as the engine command that may have
    /// recorded it (see [`jpeg_options_ui::recorded_choice`]).
    pub(crate) fn from_step(params: &Value, cmd: &str) -> Result<Self, String> {
        Ok(FormatChoices {
            tga_bits: targa_options_ui::bits_param(params, cmd)?,
            dds: dds_options_ui::options_param(params, cmd)?,
            jpeg: jpeg_options_ui::recorded_choice(params, cmd)?,
        })
    }

    /// The export settings with these answers applied.
    pub fn apply(self, settings: ExportSettings) -> ExportSettings {
        let settings = ExportSettings { tga_bits: self.tga_bits.or(settings.tga_bits), dds: self.dds.unwrap_or(settings.dds), ..settings };
        match self.jpeg {
            Some(j) => j.apply(settings),
            None => settings,
        }
    }

    /// Adds the answers to a recorded save step.
    pub(crate) fn record(self, step: &mut Value) {
        if let Some(bits) = self.tga_bits {
            step["tgaBits"] = bits.bits().into();
        }
        if let Some(dds) = self.dds {
            dds_options_ui::record(dds, step);
        }
        if let Some(j) = self.jpeg {
            j.record(step);
        }
    }
}

/// Decode a file: (document, warnings about anything approximated or dropped).
pub type ImportFn = Box<dyn Fn(&str, &[u8], &photocraft_io::ImportOptions) -> Result<(Document, Vec<String>), String>>;

/// Encoder settings chosen in Export As (the file format comes from the name's extension).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    /// JPEG quality 1–100 (None = codec default).
    pub jpeg_quality: Option<u8>,
    /// JPEG Options › Format Options: baseline, baseline optimized or progressive.
    pub jpeg_format: photocraft_codecs::JpegFormat,
    /// WebP: lossless (VP8L) rather than lossy (VP8 at [`Self::webp_quality`]).
    pub webp_lossless: bool,
    /// Lossy WebP quality 1–100 (None = codec default).
    pub webp_quality: Option<u8>,
    /// TIFF: keep the layers (Photoshop layer data); `false` is "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Embed the document's whole XMP packet. `true` by default (Save As keeps the metadata);
    /// Export As starts at Metadata: None, since the packet can carry the text of every type
    /// layer and one id per placed document (#647).
    pub xmp_all: bool,
    /// Targa: bits per pixel from Targa Options or Export As (`None`: 32 with an alpha channel or
    /// transparency, else 24).
    pub tga_bits: Option<photocraft_io::TgaBits>,
    /// DDS: the compression and mip chain from DDS Options (uncompressed without mipmaps unless
    /// chosen).
    pub dds: photocraft_io::DdsOptions,
    /// PSD/PSB: embed a thumbnail (Preferences › File Handling › Image Previews; see
    /// [`PhotocraftApp::save_settings`]).
    pub image_previews: bool,
    /// PSD/PSB: write the real merged image (Preferences › File Handling › Maximize PSD and PSB
    /// File Compatibility; see [`PhotocraftApp::save_settings`]).
    pub psd_maximize_compatibility: bool,
    /// Embed the document's ICC profile when the format supports it. On for Save As and, unless
    /// Export As turns it off, for export.
    pub embed_icc: bool,
    /// Embed EXIF, resolution and other non-XMP metadata when the format supports it. Export As
    /// turns this off for Metadata: None; Save As leaves it on.
    pub embed_metadata: bool,
    /// The path of the document's own file, which linked smart objects that PSD/PSB embed are
    /// resolved against ([`photocraft_io::ExportOptions::document_path`]).
    pub document_path: Option<std::path::PathBuf>,
    /// Native save cache owned by the open document; exports without one remain one-shot.
    pub pcraft_cache: Option<photocraft_io::PcraftExportCache>,
    /// Read and embed the files linked smart objects name ([`photocraft_io::ExportOptions::read_linked_files`]).
    /// Off for automation saves, which are granted only their target.
    pub read_linked_files: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            jpeg_quality: None,
            jpeg_format: photocraft_codecs::JpegFormat::Baseline,
            webp_lossless: true,
            webp_quality: None,
            tiff_layers: true,
            xmp_all: true,
            tga_bits: None,
            dds: photocraft_io::DdsOptions::default(),
            image_previews: true,
            psd_maximize_compatibility: true,
            embed_icc: true,
            embed_metadata: true,
            document_path: None,
            pcraft_cache: None,
            read_linked_files: true,
        }
    }
}

impl ExportSettings {
    /// The IO export options these encoder settings select. Save As and Export As share this so
    /// a profile or metadata choice cannot be applied in one and forgotten in the other.
    pub fn io_options(&self) -> photocraft_io::ExportOptions {
        let mut opts = photocraft_io::ExportOptions::default();
        if let Some(q) = self.jpeg_quality {
            opts.encode.jpeg_quality = q;
        }
        opts.encode.jpeg_format = self.jpeg_format;
        opts.encode.webp_lossless = self.webp_lossless;
        if let Some(q) = self.webp_quality {
            opts.encode.webp_quality = q;
        }
        opts.encode.embed_icc = self.embed_icc;
        opts.encode.embed_metadata = self.embed_metadata;
        opts.tiff_layers = self.tiff_layers;
        opts.xmp = if self.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
        opts.tga_bits = self.tga_bits;
        opts.encode.dds = self.dds;
        opts.image_previews = self.image_previews;
        opts.psd_maximize_compatibility = self.psd_maximize_compatibility;
        opts.document_path = self.document_path.clone();
        opts.pcraft_cache = self.pcraft_cache.clone();
        opts.read_linked_files = self.read_linked_files;
        opts
    }
}

/// The document's lone layer when it is a plain raster a flat file keeps: visible, at full
/// opacity, in Normal mode, with no masks, effects or blending restrictions (#2234).
pub(crate) fn plain_raster(doc: &Document) -> Option<&photocraft_doc::Layer> {
    match doc.layers.as_slice() {
        [layer]
            if matches!(layer.content, photocraft_doc::LayerContent::Raster(_))
                && layer.visible
                && layer.opacity >= 1.0
                && layer.fill_opacity >= 1.0
                && layer.mask.is_none()
                && layer.vector_mask.is_none()
                && layer.effects.items.is_empty()
                && layer.effects.psd_raw.is_none()
                && matches!(layer.blend, photocraft_color::BlendMode::Normal | photocraft_color::BlendMode::PassThrough)
                && photocraft_compose::channel_weights(layer, doc.mode).is_none()
                && !photocraft_compose::blend_if_active(layer, doc.mode) =>
        {
            Some(layer)
        }
        _ => None,
    }
}

/// Encode a document: (file bytes, warnings about anything approximated or dropped).
pub type ExportFn = Box<dyn Fn(&Document, &str, &ExportSettings) -> Result<(Vec<u8>, Vec<String>), String>>;

pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// What a file on disk looked like when a document last read or wrote it: File › Save asks
/// before replacing a file another app changed since (#6596).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// The [`FileStamp`] of the file at a path, `None` when it can't be read.
pub type FileStampFn = Box<dyn Fn(&str) -> Option<FileStamp>>;

/// Whether two paths name one entry in one folder on disk; false when either can't be read.
pub type SameFileFn = Box<dyn Fn(&str, &str) -> bool>;

/// Encode a document and write it to a path, reporting the stage to the job; returns the export
/// warnings. Runs on a worker thread (see [`Services::save_file`]). It must call
/// [`jobs_ui::SaveCtl::commit`] right before replacing the file, and write nothing if that fails.
pub type SaveFileFn = std::sync::Arc<dyn Fn(&Document, &str, &ExportSettings, &jobs_ui::SaveCtl) -> Result<Vec<String>, String> + Send + Sync>;

/// The revision a document counts as saved at after `written` went to its writer: that one when
/// the writer confirms the bytes were stored, else still `previous`, so the document stays
/// unsaved. A browser download is only a request the user can cancel (#4962).
pub(crate) fn saved_revision_after_write(confirmed: bool, written: u64, previous: u64) -> u64 {
    if confirmed { written } else { previous }
}

/// Who a finished save serves.
#[derive(Clone, Copy)]
pub(crate) enum SaveKind {
    /// Interactive: arm the document's file and fire user-configured hooks.
    User,
    /// Automation: flat formats stay copies (#2579) and hooks never fire.
    Automation,
}

/// Read bytes through the desktop control session's authorized read root.
pub type AutomationReadFn = Box<dyn FnMut(&str) -> Result<(String, Vec<u8>), String>>;

/// Write bytes through the desktop control session's authorized write root.
pub type AutomationWriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// Load a directory-form `.pcraft` through the authorized read root, returning its display
/// name (the file name) and the loaded document. `Ok(None)` marks a regular file: the
/// caller falls back to the bytes path for the zip form.
pub type AutomationOpenPcraftFn = Box<dyn FnMut(&str) -> Result<Option<(String, Document)>, String>>;

/// Reject engine commands which still perform ambient filesystem I/O.
pub type AutomationCommandFn = Box<dyn Fn(&str, &Value) -> Result<(), String>>;

/// Open a URL in the system browser (native) — reliable cross-platform, unlike `ctx.open_url`.
pub type OpenUrlFn = Box<dyn Fn(&str) -> Result<(), String>>;

pub type EncodePngFn = Box<dyn Fn(u32, u32, &[u8]) -> Result<Vec<u8>, String>>;

/// Put an RGBA8 image (width, height, pixels) on the OS clipboard.
pub type ClipboardSetFn = Box<dyn FnMut(u32, u32, &[u8]) -> Result<(), String>>;

/// Read an RGBA8 image from the OS clipboard. `Ok(None)` means no image is available;
/// `Err` means the contents could not be read and must not invalidate the internal copy.
pub type ClipboardGetFn = Box<dyn FnMut() -> Result<Option<(u32, u32, Vec<u8>)>, String>>;

/// Whether the last clipboard read found no way to reach the OS clipboard at all.
pub type ClipboardUnreadableFn = Box<dyn Fn() -> bool>;

/// An editor selected by the user, never an executable supplied by document metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalSvgEditor {
    /// A known VectorCraft installation, found by the desktop shell.
    VectorCraft,
    /// A known Adobe Illustrator installation (offered where the shell can discover one).
    Illustrator,
    /// The absolute path the user chose with "Other application": an executable, or on macOS
    /// an application bundle. The shell validates it before starting anything.
    Program(String),
}

/// An immutable saved source and optional display-only preview of those exact bytes.
#[derive(Debug)]
pub struct ExternalVectorSave {
    /// The saved file in its original format (SVG, AI, PDF or EPS).
    pub bytes: Vec<u8>,
    /// For AI/PDF/EPS, an SVG rendering of `bytes` used only for display; `None` for SVG.
    pub preview_svg: Option<Vec<u8>>,
}

/// A native editing session supplies stable saved source snapshots without blocking the UI.
pub trait ExternalSvgSession {
    /// Take the next settled save (its bytes) or error, if one is ready; never blocks.
    fn poll(&mut self) -> Option<Result<Vec<u8>, String>>;
    /// SVG-only services remain compatible; native services return source and preview together.
    fn poll_vector(&mut self) -> Option<Result<ExternalVectorSave, String>> {
        self.poll().map(|result| result.map(|bytes| ExternalVectorSave { bytes, preview_svg: None }))
    }
    /// No more snapshots can arrive, for example after the desktop chooser was cancelled.
    fn finished(&self) -> bool {
        false
    }
}

/// An immutable embedded source. The shell keeps the original format in its owned working directory.
#[derive(Clone)]
pub struct ExternalVectorSource {
    /// The embedded file, shared rather than copied.
    pub bytes: std::sync::Arc<Vec<u8>>,
    /// Its whitelisted format: `svg`, `ai`, `pdf` or `eps`.
    pub extension: String,
    /// The placed object's source-pixel frame, which a converted preview is fitted to.
    pub size: Option<(f64, f64)>,
}

pub type OpenExternalSvgFn = Box<dyn FnMut(ExternalSvgEditor, ExternalVectorSource) -> Result<Box<dyn ExternalSvgSession>, String>>;
/// Shared queue of (file name, bytes) delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// Read the saved preferences text (`None` when there is none yet).
pub type LoadTextFn = Box<dyn FnMut() -> Option<String>>;

/// Persist the preferences text.
pub type SaveTextFn = Box<dyn FnMut(&str) -> Result<(), String>>;

/// Platform appearance when egui cannot detect it (for example, Wayland without a theme event).
pub type SystemThemeFn = Box<dyn Fn(&egui::Context) -> Option<egui::Theme>>;

/// Autosave a document snapshot for crash recovery: (snapshot, revision, original path).
pub type AutosaveFn = Box<dyn FnMut(&std::sync::Arc<Document>, u64, Option<&str>) -> Result<(), String>>;

/// Poll successful or failed background writes: (document id, revision, result).
pub type AutosaveResultsFn = Box<dyn FnMut() -> Vec<(u64, u64, Result<(), String>)>>;

/// Drop the recovery data of a document (by `DocId` value) once it is saved or closed.
pub type DiscardAutosaveFn = Box<dyn FnMut(u64)>;

/// List recoverable documents without decoding them. Their data stays until the documents are
/// saved or closed; the shell runs each entry's loader on a background worker.
pub type RecoverFn = Box<dyn FnMut() -> Vec<Recoverable>>;

/// Photoshop's own keyboard shortcut set on this machine, as (source path, `.kys` XML text):
/// the newest install's live `Keyboard Shortcuts.psp` on the desktop, `None` without one.
pub type PhotoshopShortcutsFn = Box<dyn FnMut() -> Option<(String, String)>>;

/// A recovered document (by `DocId` value, once open) takes over its recovery entry (by key):
/// its autosaves replace the entry, and saving or closing it drops the entry.
pub type AdoptAutosaveFn = Box<dyn FnMut(u64, &str)>;

/// A recovery entry [`RecoverFn`] found; its loader owns only the data it needs to read.
pub struct Recoverable {
    /// The recovery entry to adopt once loading succeeds (see [`AdoptAutosaveFn`]).
    pub key: String,
    pub name: String,
    /// Where the user last saved it, if anywhere.
    pub path: Option<String>,
    pub load: Box<dyn FnOnce() -> Result<Document, String> + Send + 'static>,
}

/// Append text to a file (History Log).
pub type AppendTextFn = Box<dyn FnMut(&str, &str) -> Result<(), String>>;

/// Requests from the operating system since the last call (see [`OsEvent`]).
pub type OsEventsFn = Box<dyn FnMut() -> Vec<OsEvent>>;

/// Ends the app the platform's own way (see [`Services::quit`]).
pub type QuitFn = Box<dyn FnMut()>;

/// Where the OS pointer is now, in egui points within the window; `None` when unknown.
pub type CursorPosFn = Box<dyn FnMut(&egui::Context) -> Option<egui::Pos2>>;

/// Whether Caps Lock is toggled on, read from the OS. `None` where the platform cannot
/// report it (native Wayland): the cursor then follows the cursor preference (#1758).
pub type CapsLockFn = Box<dyn FnMut() -> bool>;

/// Keeps an undecorated window's OS frame from offsetting its content and pointer (#2246).
/// Called once per frame; `None` when the window is decorated or the platform needs no fixing.
pub type WindowFrameFn = Box<dyn FnMut()>;

/// The caption buttons' current style ([`Services::caption_style`]).
pub type CaptionStyleFn = Box<dyn Fn(&egui::Context) -> CaptionStyle>;

/// Physical right mouse button or stylus barrel button down (Windows; read for brush resize).
pub type RightButtonDownFn = Box<dyn Fn() -> bool + Send + Sync>;

/// Platform services injected by the app binary (file dialogs, codecs), keeping this crate free of
/// I/O dependencies.
#[derive(Default)]
pub struct Services {
    /// Explicit interactive SVG smart-object editing; unavailable in browser and automation.
    pub open_external_svg: Option<OpenExternalSvgFn>,
    /// User-initiated desktop/browser pixel sampling.
    pub screen_pick: Option<screen_picker::Service>,
    /// Decode a file's bytes into a document (PSD, PNG, JPEG, …).
    pub import: Option<ImportFn>,
    /// Encode a document for a file name (format chosen by extension).
    pub export: Option<ExportFn>,
    /// Background save over the automation write authority (`app.save`, #2017): same worker
    /// contract as `save_file`, but the bytes go through the capability-scoped workspace.
    pub automation_save: Option<SaveFileFn>,
    /// Show an Open or Save dialog without waiting for it (see `file_dialog`).
    pub file_dialog: Option<FileDialogFn>,
    /// Write bytes to a path (native) or trigger a download (web).
    pub write: Option<WriteFn>,
    /// Size and modification time of a file (native); `None` on the web, where Save never
    /// writes back to a file.
    pub file_stamp: Option<FileStampFn>,
    /// Whether two paths name one entry in one folder (native): a save dialog's answer and the
    /// path the save writes to after Lowercase Extension, on a file system that ignores letter
    /// case. A hard link is another entry. A save asks before replacing an entry it finds
    /// different (`replace_prompt`).
    pub same_file: Option<SameFileFn>,
    /// `write` only asks for the bytes to be kept (a browser download, which the user can
    /// cancel and which reports nothing back), so a save through it names the document but
    /// leaves its changes unsaved and the leave-page guard armed (#4962).
    pub write_unconfirmed: bool,
    /// Encode and write in one step on a worker thread, so a large save doesn't freeze the
    /// window (#2017). With background jobs on, saves use it instead of `export` and `write`.
    pub save_file: Option<SaveFileFn>,
    /// File access used only by control/MCP requests. Interactive dialogs keep
    /// using `file_dialog` and `write` with the user's authority.
    pub automation_read: Option<AutomationReadFn>,
    pub automation_write: Option<AutomationWriteFn>,
    pub automation_open_pcraft: Option<AutomationOpenPcraftFn>,
    pub automation_command: Option<AutomationCommandFn>,
    /// Same policy as [`Self::automation_command`], as a function pointer the engine calls for
    /// each step of `actions.play`. Installed on the session only while a control request or
    /// an automation-driven [`PhotocraftApp::run`] runs, so a local play of a recorded `file.*` step still works.
    pub automation_authorize: Option<fn(&str, &serde_json::Value) -> photocraft_engine::Result<()>>,
    /// Encode an RGBA8 image as PNG (used for screenshots and `ui.render`).
    pub encode_png: Option<EncodePngFn>,
    /// Open a URL in the system browser (native). Falls back to `ctx.open_url` (web) when unset.
    pub open_url: Option<OpenUrlFn>,
    /// Preferences ▸ General ▸ Beep When Done: the platform's default beep, played when a
    /// background job finishes successfully. `None` in the web build (and in tests).
    pub beep: Option<fn()>,
    /// The system's alert sound, played when a key does nothing in a zoom field being typed in
    /// (Tab, Alt; see `zoom_field`). No platform sets it yet, so the alert falls back to
    /// [`Self::beep`].
    pub alert: Option<fn()>,
    /// Starts a background check for published releases (desktop, outside Flatpak).
    pub check_updates: Option<photocraft_engine::updates_cmds::CheckFn>,
    /// Files delivered asynchronously (web drag-and-drop): drained every frame and opened as
    /// documents.
    pub inbox: Option<Inbox>,
    /// Files delivered asynchronously for a drop onto the active document's canvas (web
    /// drag-and-drop, routed by [`PhotocraftApp::dropped_on_canvas`]): placed as layers like a
    /// desktop canvas drop (Place Embedded, each in Free Transform).
    pub place_inbox: Option<Inbox>,
    /// OS clipboard images: copies go out, screenshots and images from other apps come in.
    pub clipboard_set_image: Option<ClipboardSetFn>,
    pub clipboard_get_image: Option<ClipboardGetFn>,
    /// True when the last `clipboard_get_image` read had no clipboard to read, as in a Flatpak on
    /// Wayland (#5031), rather than a clipboard that holds no image. `None` where that can't happen.
    pub clipboard_unreadable: Option<ClipboardUnreadableFn>,
    /// Preferences persistence: the platform config directory on native, browser storage on
    /// the web (see `prefs_ui`).
    pub load_prefs: Option<LoadTextFn>,
    pub save_prefs: Option<SaveTextFn>,
    /// Photoshop's live keyboard shortcut set, imported once at first launch (`kys_import`).
    pub photoshop_shortcuts: Option<PhotoshopShortcutsFn>,
    pub system_theme: Option<SystemThemeFn>,
    /// The native window is connected directly to a Wayland compositor.
    pub is_wayland: bool,
    /// On Wayland, the shell command that starts this install under XWayland, where native file
    /// drops work (#386); `None` when there is no X server to run it on.
    pub xwayland_command: Option<String>,
    /// Crash-recovery autosave (Preferences › File Handling) and recovery at launch.
    pub autosave: Option<AutosaveFn>,
    /// None for services that report synchronous success through `autosave`.
    pub autosave_results: Option<AutosaveResultsFn>,
    /// Finish every queued crash-recovery autosave and wait for it, with outcomes like
    /// `autosave_results`. For a session that ends without asking: the window's graphics device
    /// was lost (see `gpu_status::check_window`).
    pub flush_autosaves: Option<AutosaveResultsFn>,
    pub discard_autosave: Option<DiscardAutosaveFn>,
    pub recover: Option<RecoverFn>,
    pub adopt_autosave: Option<AdoptAutosaveFn>,
    /// History Log text file output.
    pub append_text: Option<AppendTextFn>,
    /// OS requests (macOS open-documents / quit Apple events), polled every frame.
    pub os_events: Option<OsEventsFn>,
    /// The pointer position read from the OS (desktop) or from the page's drag events (web):
    /// file drops carry none, and the window gets no pointer events during an OS drag (see
    /// `file_open::DropTarget`).
    pub cursor_pos: Option<CursorPosFn>,
    /// Caps Lock toggled on (desktop; `None` on Wayland and the web). Read once per frame, so
    /// the canvas can show the precise crosshair for painting tools, whatever the preference.
    pub caps_lock: Option<CapsLockFn>,
    /// Keeps the undecorated window borderless, so the content and the pointer stay aligned
    /// (Windows custom title bar, #2246). Called once per frame; `None` when the window is
    /// decorated or the platform needs no fixing.
    pub window_frame: Option<WindowFrameFn>,
    /// How the custom title bar's caption buttons look and where they sit (Linux: the desktop's
    /// style and button layout). Read every frame, with the context to wake when it changes;
    /// `None` keeps Windows' buttons at the right end.
    pub caption_style: Option<CaptionStyleFn>,
    /// Physical right mouse button or pen barrel button currently down.
    pub right_button_down: Option<RightButtonDownFn>,
    /// The persistent brush preset store, loading in the background (desktop; see
    /// `photocraft_engine::preset_store`). Attached to the session once it arrives; without
    /// one, brush presets are session-only unless the shell attached a store before startup.
    pub preset_store: Option<std::sync::mpsc::Receiver<photocraft_engine::preset_store::Opened>>,
    /// Reads the displays and their ICC profiles in the background (desktop macOS; see
    /// `monitor_status`). Without one, the canvas uses the profile chosen in Color Settings, or sRGB.
    pub read_displays: Option<monitor_status::ReadDisplaysFn>,
    /// The macOS menu bar, when the desktop app installed one; the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
    /// Ends the app once leaving is settled (nothing unsaved, or the prompt answered), instead of
    /// letting eframe close the window. macOS: `-[NSApplication terminate:]`. Closing the window
    /// while AppKit's run loop is still going crashes or hangs Touch Bar Macs (#1575, #1458).
    /// Without one, eframe closes the window and the app ends with it.
    pub quit: Option<QuitFn>,
}
