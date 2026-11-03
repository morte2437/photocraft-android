//! Explicit desktop SVG editing. Clipboard/source data never supplies a command or program path.
//!
//! The owned temporary SVG is watched off the UI thread. Closing the PhotoCraft session stops
//! watching and removes its temporary directory; it does not terminate the external editor.

use photocraft_ui_egui::{ExternalSvgEditor, ExternalSvgSession, ExternalVectorSave};

/// Start an explicit native editor session; artwork cannot supply executable paths.
pub(crate) fn open(editor: ExternalSvgEditor, source: photocraft_ui_egui::ExternalVectorSource) -> Result<Box<dyn ExternalSvgSession>, String> {
    native::open(editor, source)
}

mod native {
    use super::{ExternalSvgEditor, ExternalSvgSession, ExternalVectorSave};
    use std::fs::{Metadata, OpenOptions};
    use std::io::{Read, Write};
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Child;
    use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};
    use tempfile::TempDir;

    const MAX_BYTES: usize = 16 * 1024 * 1024;
    const POLL_INTERVAL: Duration = Duration::from_millis(200);
    const SAVE_SETTLE: Duration = Duration::from_millis(600);
    /// The file changed while being read. The watcher retries it; it is never shown to the user.
    const SAVE_IN_PROGRESS: &str = "The SVG is still being saved";

    /// Resolve the chosen editor and move conversion, launch and watching off the UI thread.
    pub(super) fn open(editor: ExternalSvgEditor, source: photocraft_ui_egui::ExternalVectorSource) -> Result<Box<dyn ExternalSvgSession>, String> {
        // A Flatpak payload belongs to the caller's sandbox when executed directly. Let the
        // desktop launch the actual editor and grant it just this writable working file.
        #[cfg(target_os = "linux")]
        let portal = matches!(&editor, ExternalSvgEditor::VectorCraft);
        #[cfg(not(target_os = "linux"))]
        let portal = false;
        let chosen_vectorcraft = matches!(&editor, ExternalSvgEditor::VectorCraft);
        let chosen_bundle = matches!(&editor, ExternalSvgEditor::Program(path) if cfg!(target_os = "macos") && Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app")) && Path::new(path).is_dir());
        let executable = if portal && source.extension == "svg" { None } else { Some(resolve(editor)?) };
        // Only VectorCraft's own working copy of an AI is regenerated (it opens foreign AI
        // read-only); Illustrator and other editors get the original bytes.
        let regenerate_ai = chosen_vectorcraft || executable.as_ref().is_some_and(|resolved| is_vectorcraft(resolved, chosen_bundle));
        // Display conversion is always performed by VectorCraft's CLI, even when the user
        // chooses a different interactive editor. A VectorCraft build with an adjacent CLI keeps
        // its matching CLI; for a macOS bundle that is the bundle's own `Contents/MacOS`.
        let converter = if source.extension == "svg" { None } else { Some(resolve_converter(executable.as_ref().map(|e| e.program.as_path()))?) };
        if source.bytes.is_empty() || source.bytes.len() > MAX_BYTES || !matches!(source.extension.as_str(), "svg" | "pdf" | "ai" | "eps") {
            return Err("Unsupported embedded vector source or source exceeds 16 MiB".into());
        }
        // Conversion and editor startup run off the UI thread. Dropping this session cancels
        // conversion; it never kills the interactive editor or changes the parent document.
        let (stop, stopped) = mpsc::channel();
        let (send, updates) = mpsc::sync_channel(2);
        thread::Builder::new()
            .name("photocraft-vector-editor".into())
            .spawn(move || {
                let prepared = match converter.as_deref() {
                    Some(executable) => super::vector_conversion::prepare(executable, &source, &stopped, regenerate_ai),
                    None => OwnedSvg::create(&source.bytes).map(|owned| (owned, source.bytes.as_ref().clone())),
                };
                match prepared {
                    Ok((owned, initial)) => {
                        if stopped.try_recv().is_ok() {
                            return;
                        }
                        #[cfg(target_os = "linux")]
                        if portal {
                            match super::portal::open(&owned, &stopped) {
                                Ok(()) => watch(owned, initial, None, stopped, send, (POLL_INTERVAL, SAVE_SETTLE), converter.as_deref().map(|e| (e, &source))),
                                Err(error) => {
                                    let _ = send.try_send(Err(error));
                                }
                            }
                            return;
                        }
                        if let Some(executable) = executable.as_ref() {
                            match executable.launch(&owned.file) {
                                Ok(child) => {
                                    watch(owned, initial, Some(child), stopped, send, (POLL_INTERVAL, SAVE_SETTLE), converter.as_deref().map(|e| (e, &source)))
                                }
                                Err(error) => {
                                    let _ = send.try_send(Err(format!("Could not open the vector editor: {error}")));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = send.try_send(Err(error));
                    }
                }
            })
            .map_err(|e| format!("Cannot start vector editor session: {e}"))?;
        Ok(Box::new(Session { stop, updates, finished: false }))
    }

    /// The editor-choice dialog's label for a user-selected program, named by every
    /// "not found" error so the user can find the control.
    const OTHER_APPLICATION: &str = "Other application";

    /// A validated editor and how to start it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct Resolved {
        /// The canonical executable file. It is run directly unless `bundle` is set, and its
        /// directory is where an adjacent `vectorcraft-cli` is looked for.
        pub(super) program: PathBuf,
        /// A canonical macOS application bundle opened through LaunchServices
        /// (`/usr/bin/open -a`), so a Cocoa app receives the file as an open-document event
        /// instead of an argument it would ignore.
        pub(super) bundle: Option<PathBuf>,
    }

    impl Resolved {
        /// Run `program` itself with the working file as its only argument.
        fn direct(program: PathBuf) -> Self {
            Self { program, bundle: None }
        }

        /// The process that opens `file`: no shell, no PATH lookup, one file argument.
        fn command(&self, file: &Path) -> std::process::Command {
            match &self.bundle {
                Some(bundle) => {
                    let mut command = std::process::Command::new("/usr/bin/open");
                    // Both paths are absolute, so neither can be read as an `open` option.
                    command.arg("-a").arg(bundle).arg(file);
                    command
                }
                None => {
                    let mut command = std::process::Command::new(&self.program);
                    command.arg(file);
                    command
                }
            }
        }

        /// Start the editor on `file`. For a bundle the child is `open`, which exits once
        /// LaunchServices has delivered the file; the watcher only reaps it.
        fn launch(&self, file: &Path) -> std::io::Result<Child> {
            self.command(file).spawn()
        }
    }

    /// Whether a chosen program is VectorCraft: a macOS bundle `resolve_program` identified by
    /// its bundle id (it then runs the executable directly), a known VectorCraft installation,
    /// or an executable named as VectorCraft is (`vectorcraft`/`vectorcraft.exe`, an AppImage
    /// `vectorcraft-<version>-….AppImage`, the Flatpak launcher `ai.storyteller.vectorcraft`).
    /// A neighbouring CLI alone doesn't make an editor VectorCraft.
    fn is_vectorcraft(resolved: &Resolved, chosen_bundle: bool) -> bool {
        resolved.bundle.is_none()
            && (chosen_bundle
                || resolved.program.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy().to_ascii_lowercase();
                    let stem = name.strip_suffix(".exe").unwrap_or(&name);
                    stem == "vectorcraft" || stem == "ai.storyteller.vectorcraft" || (stem.starts_with("vectorcraft-") && !stem.starts_with("vectorcraft-cli"))
                })
                || vectorcraft_programs().contains(&resolved.program))
    }

    /// True when `program`'s directory holds a regular (not linked) `vectorcraft-cli`.
    fn has_adjacent_cli(program: &Path) -> bool {
        let name = if cfg!(windows) { "vectorcraft-cli.exe" } else { "vectorcraft-cli" };
        program.parent().is_some_and(|directory| std::fs::symlink_metadata(directory.join(name)).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()))
    }

    /// The executable whose adjacent `vectorcraft-cli` converts AI/PDF/EPS previews. A selected
    /// build that carries its own CLI keeps it (for a macOS bundle, `Contents/MacOS` of that same
    /// bundle). Otherwise the first known VectorCraft installation that has one is used; document
    /// metadata never participates in discovery.
    fn resolve_converter(selected: Option<&Path>) -> Result<PathBuf, String> {
        if let Some(editor) = selected
            && has_adjacent_cli(editor)
        {
            return Ok(editor.to_path_buf());
        }
        vectorcraft_programs()
            .into_iter()
            .find(|program| has_adjacent_cli(program))
            .ok_or_else(|| "AI/PDF/EPS editing also needs an installed VectorCraft CLI to validate display previews".into())
    }

    /// A known VectorCraft location: a bare executable, or a macOS application bundle.
    #[cfg(unix)]
    #[derive(Debug, PartialEq, Eq)]
    enum Install {
        Executable(PathBuf),
        Bundle(PathBuf),
    }

    /// Fixed VectorCraft locations in search order. On macOS the standard application folders
    /// come first; the Unix install prefixes apply everywhere. Nothing is searched recursively.
    #[cfg(unix)]
    fn vectorcraft_installs(home: Option<&Path>, macos: bool) -> Vec<Install> {
        let mut installs = vec![];
        if macos {
            installs.push(Install::Bundle(PathBuf::from("/Applications/VectorCraft.app")));
            if let Some(home) = home {
                installs.push(Install::Bundle(home.join("Applications/VectorCraft.app")));
            }
        }
        if let Some(home) = home {
            installs.push(Install::Executable(home.join(".local/share/flatpak/app/ai.storyteller.vectorcraft/current/active/files/bin/vectorcraft")));
        }
        installs.push(Install::Executable(PathBuf::from("/usr/local/bin/vectorcraft")));
        installs.push(Install::Executable(PathBuf::from("/usr/bin/vectorcraft")));
        if let Some(home) = home {
            installs.push(Install::Executable(home.join(".local/bin/vectorcraft")));
        }
        installs
    }

    /// Every valid known VectorCraft executable, in search order.
    #[cfg(unix)]
    fn vectorcraft_programs() -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        vectorcraft_installs(home.as_deref(), cfg!(target_os = "macos"))
            .into_iter()
            .filter_map(|install| match install {
                Install::Executable(path) => validate_executable(&path).ok(),
                // VectorCraft opens the files named on its command line and does not handle
                // macOS open-document events, so its bundle executable is run directly.
                Install::Bundle(bundle) => bundle_executable(&bundle).ok().map(|(_, program)| program),
            })
            .collect()
    }

    /// Discover only known installations or an explicitly chosen executable or application.
    #[cfg(unix)]
    fn resolve(editor: ExternalSvgEditor) -> Result<Resolved, String> {
        match editor {
            ExternalSvgEditor::Program(path) => resolve_program(Path::new(&path), cfg!(target_os = "macos")),
            ExternalSvgEditor::VectorCraft => vectorcraft_programs()
                .into_iter()
                .next()
                .map(Resolved::direct)
                .ok_or_else(|| format!("VectorCraft was not found. Choose its executable with {OTHER_APPLICATION}")),
            ExternalSvgEditor::Illustrator if cfg!(target_os = "macos") => newest_illustrator(Path::new("/Applications")),
            ExternalSvgEditor::Illustrator => Err(format!("Choose a local executable with {OTHER_APPLICATION}")),
        }
    }

    /// A user-selected program: on macOS an application bundle (`.app`, which the system file
    /// chooser returns as a directory) opens through LaunchServices; anything else must be an
    /// executable regular file.
    #[cfg(unix)]
    fn resolve_program(path: &Path, macos_bundles: bool) -> Result<Resolved, String> {
        if macos_bundles && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app")) && path.is_dir() {
            let (bundle, program, vectorcraft) = bundle_executable_and_identity(path)?;
            // Unlike Cocoa editors, VectorCraft consumes file arguments, not LaunchServices
            // open-document events. Identify the app by its bundle id, not the presence of a
            // CLI beside an unrelated user-selected editor. SVG editing needs no CLI at all.
            return Ok(if vectorcraft { Resolved::direct(program) } else { Resolved { program, bundle: Some(bundle) } });
        }
        validate_executable(path).map(Resolved::direct)
    }

    /// The newest Adobe Illustrator bundle under `applications`, opened through LaunchServices.
    #[cfg(unix)]
    fn newest_illustrator(applications: &Path) -> Result<Resolved, String> {
        illustrator_bundles(applications)
            .into_iter()
            .rev()
            .find_map(|bundle| bundle_executable(&bundle).ok())
            .map(|(bundle, program)| Resolved { program, bundle: Some(bundle) })
            .ok_or_else(|| format!("Illustrator was not found. Choose its application with {OTHER_APPLICATION}"))
    }

    /// Adobe installs `Adobe Illustrator <version>/Adobe Illustrator <version>.app` in
    /// /Applications. Look at most one folder deep (512 entries, then 64 per folder), never
    /// recursively, and sort by name so the last entry is the highest version.
    #[cfg(unix)]
    fn illustrator_bundles(applications: &Path) -> Vec<PathBuf> {
        let is_illustrator = |name: &std::ffi::OsStr| name.to_str().is_some_and(|name| name.starts_with("Adobe Illustrator"));
        let is_app = |path: &Path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
        let mut bundles = vec![];
        let Ok(entries) = std::fs::read_dir(applications) else { return bundles };
        for entry in entries.take(512).flatten() {
            if !is_illustrator(&entry.file_name()) {
                continue;
            }
            let path = entry.path();
            if is_app(&path) {
                bundles.push(path);
                continue;
            }
            let Ok(inner) = std::fs::read_dir(&path) else { continue };
            bundles.extend(inner.take(64).flatten().filter(|app| is_illustrator(&app.file_name())).map(|app| app.path()).filter(|path| is_app(path)));
        }
        bundles.sort();
        bundles
    }

    /// Info.plist files are a few kilobytes; anything larger is not a real application.
    #[cfg(unix)]
    const MAX_INFO_PLIST_BYTES: u64 = 1024 * 1024;

    /// Resolve an application bundle to its canonical path and its executable,
    /// `Contents/MacOS/<CFBundleExecutable>`. The bundle must be a directory named `*.app`, its
    /// Info.plist (XML or binary) a bounded regular file, and the executable a plain file name
    /// that resolves to an executable regular file inside that same `Contents/MacOS`.
    #[cfg(unix)]
    fn bundle_executable(bundle: &Path) -> Result<(PathBuf, PathBuf), String> {
        bundle_executable_and_identity(bundle).map(|(bundle, program, _)| (bundle, program))
    }

    /// Validate the bundle and identify VectorCraft from its declared application identifier.
    /// A neighbouring converter alone does not identify an arbitrary chosen editor.
    #[cfg(unix)]
    fn bundle_executable_and_identity(bundle: &Path) -> Result<(PathBuf, PathBuf, bool), String> {
        if !bundle.is_absolute() {
            return Err("Choose an absolute application path".into());
        }
        let bundle = bundle.canonicalize().map_err(|e| format!("Cannot resolve the chosen application: {e}"))?;
        let named_app = bundle.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
        if !named_app || !std::fs::symlink_metadata(&bundle).is_ok_and(|m| m.is_dir()) {
            return Err("Choose an application bundle (.app) or an executable file".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        no_follow(&mut options);
        let info = options.open(bundle.join("Contents/Info.plist")).map_err(|e| format!("Cannot read the application's Info.plist: {e}"))?;
        let metadata = info.metadata().map_err(|e| format!("Cannot inspect the application's Info.plist: {e}"))?;
        if !metadata.is_file() || metadata.len() > MAX_INFO_PLIST_BYTES {
            return Err("The application's Info.plist must be a regular file of at most 1 MiB".into());
        }
        let mut bytes = Vec::new();
        info.take(MAX_INFO_PLIST_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Cannot read the application's Info.plist: {e}"))?;
        if bytes.len() as u64 > MAX_INFO_PLIST_BYTES {
            return Err("The application's Info.plist must be a regular file of at most 1 MiB".into());
        }
        let name = bundle_executable_name(&bytes)?;
        if name.is_empty() || name.len() > 255 || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return Err("The application's Info.plist names an invalid executable".into());
        }
        let directory = bundle.join("Contents/MacOS");
        let program = validate_executable(&directory.join(&name))?;
        let directory = directory.canonicalize().map_err(|e| format!("Cannot resolve the application's executable folder: {e}"))?;
        if !directory.starts_with(&bundle) || program.parent() != Some(directory.as_path()) {
            return Err("The application's executable must be inside its bundle".into());
        }
        let vectorcraft = bundle_string_property(&bytes, "CFBundleIdentifier")?.as_deref() == Some("ai.storyteller.vectorcraft");
        Ok((bundle, program, vectorcraft))
    }

    /// The top-level `CFBundleExecutable` string of an Info.plist. Events are streamed with a
    /// nesting limit instead of building a value tree, so hostile nesting cannot exhaust the stack.
    #[cfg(unix)]
    fn bundle_executable_name(info_plist: &[u8]) -> Result<String, String> {
        bundle_string_property(info_plist, "CFBundleExecutable")?.ok_or_else(|| "The application's Info.plist names no executable".into())
    }

    /// Read one top-level string without constructing a recursive property-list value.
    #[cfg(unix)]
    fn bundle_string_property(info_plist: &[u8], key: &str) -> Result<Option<String>, String> {
        use plist::stream::{Event, Reader};
        const MAX_DEPTH: usize = 32;
        let invalid = || "The application's Info.plist is not a valid property list".to_string();
        let mut depth = 0usize;
        // At depth 1 (the root dictionary) entries alternate between key and value.
        let mut expecting_key = true;
        let mut wanted = false;
        for event in Reader::new(std::io::Cursor::new(info_plist)) {
            let event = event.map_err(|_| invalid())?;
            match event {
                Event::StartArray(_) | Event::StartDictionary(_) => {
                    if depth == 0 && !matches!(event, Event::StartDictionary(_)) {
                        return Err(invalid());
                    }
                    if depth == 1 && wanted {
                        return Err(format!("The application's {key} is not a string"));
                    }
                    depth = depth.checked_add(1).filter(|d| *d <= MAX_DEPTH).ok_or_else(invalid)?;
                }
                Event::EndCollection => {
                    depth = depth.checked_sub(1).ok_or_else(invalid)?;
                    if depth == 0 {
                        break;
                    }
                    if depth == 1 {
                        expecting_key = true;
                    }
                }
                Event::String(text) if depth == 1 && expecting_key => {
                    wanted = text == key;
                    expecting_key = false;
                }
                Event::String(text) if depth == 1 && wanted => return Ok(Some(text.into_owned())),
                _ if depth == 1 && wanted => return Err(format!("The application's {key} is not a string")),
                _ if depth == 1 => expecting_key = true,
                _ if depth == 0 => return Err(invalid()),
                _ => {}
            }
        }
        Ok(None)
    }

    /// Canonicalize an absolute user-selected path and require an executable regular file.
    #[cfg(unix)]
    fn validate_executable(path: &Path) -> Result<PathBuf, String> {
        use std::os::unix::fs::PermissionsExt;
        if !path.is_absolute() {
            return Err("Choose an absolute executable path".into());
        }
        let path = path.canonicalize().map_err(|e| format!("Cannot resolve the chosen editor: {e}"))?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| format!("Cannot read the chosen editor: {e}"))?;
        ensure_regular(&metadata)?;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("The chosen file is not executable".into());
        }
        Ok(path)
    }

    /// Probe only fixed Program Files / LocalAppData install locations (one Adobe directory
    /// level, at most 64 entries) or validate an explicitly chosen executable.
    #[cfg(windows)]
    fn resolve_editor(editor: ExternalSvgEditor) -> Result<PathBuf, String> {
        match editor {
            ExternalSvgEditor::Program(path) => validate_executable(Path::new(&path)),
            ExternalSvgEditor::VectorCraft => {
                let mut candidates = vec![];
                for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(root) = std::env::var_os(variable) {
                        candidates.push(PathBuf::from(root).join("VectorCraft/vectorcraft.exe"));
                    }
                }
                if let Some(root) = std::env::var_os("LOCALAPPDATA") {
                    candidates.push(PathBuf::from(root).join("Programs/VectorCraft/vectorcraft.exe"));
                }
                candidates
                    .into_iter()
                    .find_map(|path| validate_executable(&path).ok())
                    .ok_or_else(|| format!("VectorCraft was not found. Choose its executable with {OTHER_APPLICATION}"))
            }
            ExternalSvgEditor::Illustrator => {
                let mut candidates = vec![];
                for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
                    let Some(root) = std::env::var_os(variable) else { continue };
                    let adobe = PathBuf::from(root).join("Adobe");
                    // One known installation level only; never search the user's disks recursively.
                    let Ok(entries) = std::fs::read_dir(&adobe) else { continue };
                    for entry in entries.take(64).flatten() {
                        if !entry.file_name().to_string_lossy().starts_with("Adobe Illustrator") {
                            continue;
                        }
                        candidates.push(entry.path().join("Support Files/Contents/Windows/Illustrator.exe"));
                    }
                }
                candidates.sort();
                candidates
                    .into_iter()
                    .rev()
                    .find_map(|path| validate_executable(&path).ok())
                    .ok_or_else(|| format!("Illustrator was not found. Choose its executable with {OTHER_APPLICATION}"))
            }
        }
    }

    /// Windows programs are always run directly with the working file as their argument.
    #[cfg(windows)]
    fn resolve(editor: ExternalSvgEditor) -> Result<Resolved, String> {
        resolve_editor(editor).map(Resolved::direct)
    }

    /// The known VectorCraft executable, if one is installed.
    #[cfg(windows)]
    fn vectorcraft_programs() -> Vec<PathBuf> {
        resolve_editor(ExternalSvgEditor::VectorCraft).ok().into_iter().collect()
    }

    /// Require an absolute `.exe` that is a regular file, not a link or reparse point, and
    /// return its canonical path; scripts and command lines are rejected.
    #[cfg(windows)]
    fn validate_executable(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) {
            return Err("Choose an absolute Windows .exe path; scripts and command lines are not supported".into());
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("Cannot read the chosen editor: {e}"))?;
        ensure_regular(&metadata)?;
        path.canonicalize().map_err(|e| format!("Cannot resolve the chosen editor: {e}"))
    }

    pub(super) struct OwnedSvg {
        directory: TempDir,
        pub(super) root: PathBuf,
        pub(super) file: PathBuf,
    }

    impl OwnedSvg {
        /// Create a private working file from bounded source bytes.
        pub(super) fn create(svg: &[u8]) -> Result<Self, String> {
            Self::create_format(svg, "svg")
        }

        /// Whitelisted extensions preserve the format; artwork cannot select a path.
        pub(super) fn create_format(svg: &[u8], extension: &str) -> Result<Self, String> {
            if !matches!(extension, "svg" | "ai" | "pdf" | "eps") {
                return Err("Unsupported vector working-copy format".into());
            }
            if svg.is_empty() || svg.len() > MAX_BYTES {
                return Err("The SVG must contain between 1 byte and 16 MiB".into());
            }
            let mut builder = tempfile::Builder::new();
            builder.prefix("photocraft-svg-edit-");
            // Flatpak's /tmp has no host pathname that the document portal can export.
            // The app-private cache does; TempDir still creates a private random directory
            // and removes it when syncing stops. Only Artwork.svg is shared with the editor.
            #[cfg(target_os = "linux")]
            let cache = if Path::new("/.flatpak-info").is_file() {
                let cache = std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .ok_or("A sandbox-private cache directory is required for vector editing")?;
                std::fs::create_dir_all(&cache).map_err(|e| format!("Cannot create vector editing cache: {e}"))?;
                Some(cache)
            } else {
                None
            };
            #[cfg(not(target_os = "linux"))]
            let cache: Option<PathBuf> = None;
            let directory = match cache {
                Some(cache) => builder.tempdir_in(cache),
                None => builder.tempdir(),
            }
            .map_err(|e| format!("Cannot create the SVG editing directory: {e}"))?;
            let root = directory.path().canonicalize().map_err(|e| format!("Cannot resolve the SVG editing directory: {e}"))?;
            let file = root.join(format!("Artwork.{extension}"));
            let mut output = OpenOptions::new().write(true).create_new(true).open(&file).map_err(|e| format!("Cannot create the SVG editing file: {e}"))?;
            output.write_all(svg).and_then(|()| output.sync_all()).map_err(|e| format!("Cannot write the SVG editing file: {e}"))?;
            Ok(Self { directory, root, file })
        }

        /// Validate the working directory and capture a regular file identity for save settling.
        pub(super) fn stamp(&self) -> Result<Stamp, String> {
            let directory_metadata = std::fs::symlink_metadata(&self.root).map_err(|e| format!("Cannot inspect the SVG editing directory: {e}"))?;
            if !directory_metadata.is_dir() || is_redirect(&directory_metadata) || self.directory.path().canonicalize().ok().as_ref() != Some(&self.root) {
                return Err("The SVG editing directory was replaced or redirected".into());
            }
            let metadata = std::fs::symlink_metadata(&self.file).map_err(|e| format!("Cannot inspect the edited SVG: {e}"))?;
            ensure_regular(&metadata)?;
            Stamp::from_metadata(&metadata)
        }

        /// Read one settled snapshot, retaining byte limits during concurrent file growth.
        pub(super) fn read(&self, expected: &Stamp) -> Result<Vec<u8>, String> {
            if &self.stamp()? != expected {
                return Err(SAVE_IN_PROGRESS.into());
            }
            let mut options = OpenOptions::new();
            options.read(true);
            no_follow(&mut options);
            let file = options.open(&self.file).map_err(|e| format!("Cannot read the edited SVG: {e}"))?;
            let metadata = file.metadata().map_err(|e| format!("Cannot inspect the edited SVG handle: {e}"))?;
            ensure_regular(&metadata)?;
            if &Stamp::from_metadata(&metadata)? != expected {
                return Err(SAVE_IN_PROGRESS.into());
            }
            let size = usize::try_from(metadata.len()).map_err(|_| "The edited SVG is too large")?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size).map_err(|_| "Not enough memory to read the edited SVG")?;
            // Also bound reads if a concurrent writer grows the file after the metadata check.
            file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("Cannot read the edited SVG: {e}"))?;
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                return Err("The edited SVG must contain between 1 byte and 16 MiB".into());
            }
            if bytes.len() != size || &self.stamp()? != expected {
                return Err(SAVE_IN_PROGRESS.into());
            }
            Ok(bytes)
        }
        /// Share a checked writable handle, never a sandbox-private pathname, with the desktop.
        #[cfg(target_os = "linux")]
        pub(super) fn share_handle(&self) -> Result<std::fs::File, String> {
            let expected = self.stamp()?;
            let mut options = OpenOptions::new();
            options.read(true).write(true);
            no_follow(&mut options);
            let file = options.open(&self.file).map_err(|e| format!("Cannot share vector working file: {e}"))?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            ensure_regular(&metadata)?;
            if Stamp::from_metadata(&metadata)? != expected || self.stamp()? != expected {
                return Err("The vector working file changed before sharing".into());
            }
            Ok(file)
        }

        /// Write normalized SVG through a checked, no-follow handle before editor launch.
        #[cfg(all(test, unix))]
        pub(super) fn overwrite(&self, bytes: &[u8]) -> Result<(), String> {
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                return Err("Prepared SVG exceeds the supported byte limit".into());
            }
            self.stamp()?;
            let mut options = OpenOptions::new();
            options.write(true);
            no_follow(&mut options);
            let mut file = options.open(&self.file).map_err(|e| format!("Cannot open the prepared SVG: {e}"))?;
            ensure_regular(&file.metadata().map_err(|e| format!("Cannot inspect the prepared SVG: {e}"))?)?;
            file.set_len(0).and_then(|()| file.write_all(bytes)).and_then(|()| file.sync_all()).map_err(|e| format!("Cannot write the prepared SVG: {e}"))
        }
    }

    /// Prevent redirected opens and avoid blocking on a substituted Unix FIFO.
    fn no_follow(options: &mut OpenOptions) {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Open the reparse point itself, so replacing the file with a symlink cannot
            // make this access follow it between the metadata check and opening the handle.
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Do not follow a replaced symlink or block opening a substituted FIFO/device.
            options.custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32);
        }
    }

    /// Reject anything but a regular file, including symlinks and Windows reparse points.
    fn ensure_regular(metadata: &Metadata) -> Result<(), String> {
        if !metadata.is_file() || is_redirect(metadata) {
            return Err("The SVG or editor must be a regular file, not a link or reparse point".into());
        }
        Ok(())
    }

    /// True for a symlink, or on Windows any reparse point, whose target could be swapped.
    /// Expects `symlink_metadata`, which describes the link itself rather than its target.
    fn is_redirect(metadata: &Metadata) -> bool {
        if metadata.file_type().is_symlink() {
            return true;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x0000_0400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    #[derive(Clone, PartialEq, Eq)]
    pub(super) struct Stamp {
        modified: SystemTime,
        length: u64,
        #[cfg(unix)]
        identity: (u64, u64),
    }

    impl Stamp {
        /// Reject unsupported file sizes before capturing save time, length and native identity.
        fn from_metadata(metadata: &Metadata) -> Result<Self, String> {
            if metadata.len() == 0 || metadata.len() > MAX_BYTES as u64 {
                return Err("The edited SVG must contain between 1 byte and 16 MiB".into());
            }
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                modified: metadata.modified().map_err(|e| format!("Cannot inspect the SVG save time: {e}"))?,
                length: metadata.len(),
                #[cfg(unix)]
                identity: (metadata.dev(), metadata.ino()),
            })
        }
    }

    pub(super) struct Session {
        stop: Sender<()>,
        updates: Receiver<Result<ExternalVectorSave, String>>,
        finished: bool,
    }

    impl ExternalSvgSession for Session {
        /// Like [`Self::poll_vector`], keeping only the saved source bytes.
        fn poll(&mut self) -> Option<Result<Vec<u8>, String>> {
            self.poll_vector().map(|result| result.map(|saved| saved.bytes))
        }

        /// Take one queued save or error without blocking; a closed queue marks the session finished.
        fn poll_vector(&mut self) -> Option<Result<ExternalVectorSave, String>> {
            match self.updates.try_recv() {
                Ok(update) => Some(update),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    None
                }
            }
        }

        /// True once a poll has seen the watcher thread exit and drop its sender.
        fn finished(&self) -> bool {
            self.finished
        }
    }

    impl Drop for Session {
        /// Signal the worker to stop without waiting for it.
        fn drop(&mut self) {
            // No thread join or editor termination on the UI thread. The worker owns TempDir
            // and drops it after this bounded wakeup, including when the update receiver closes.
            let _ = self.stop.send(());
        }
    }

    /// Watch an already-written working file with test timings, without conversion.
    #[cfg(test)]
    fn start(owned: OwnedSvg, initial: Vec<u8>, child: Option<Child>, interval: Duration, settle: Duration) -> Result<Session, String> {
        let (stop, stopped) = mpsc::channel();
        // At most two bounded updates await the UI, never an unbounded save queue.
        let (send, updates) = mpsc::sync_channel(2);
        thread::Builder::new()
            .name("photocraft-svg-editor".into())
            .spawn(move || watch(owned, initial, child, stopped, send, (interval, settle), None))
            .map_err(|e| format!("Cannot watch SVG editor saves: {e}"))?;
        Ok(Session { stop, updates, finished: false })
    }

    /// Watch a native working file with short test timings, converting each save's preview
    /// with `editor`'s adjacent CLI as a real session would.
    #[cfg(test)]
    pub(super) fn start_native_test(
        owned: OwnedSvg,
        initial: Vec<u8>,
        editor: PathBuf,
        source: photocraft_ui_egui::ExternalVectorSource,
    ) -> Result<Session, String> {
        let (stop, stopped) = mpsc::channel();
        let (send, updates) = mpsc::sync_channel(2);
        thread::Builder::new()
            .name("native-vector-test-watcher".into())
            .spawn(move || watch(owned, initial, None, stopped, send, (Duration::from_millis(30), Duration::from_millis(100)), Some((&editor, &source))))
            .map_err(|e| e.to_string())?;
        Ok(Session { stop, updates, finished: false })
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Delivery {
        Sent,
        Changed,
        Stopped,
    }

    /// Keep either a converted save or its error under queue backpressure, without
    /// re-running conversion. A newer file supersedes either result; cancellation never waits.
    fn deliver(
        owned: &OwnedSvg,
        stamp: &Stamp,
        send: &SyncSender<Result<ExternalVectorSave, String>>,
        stopped: &Receiver<()>,
        mut update: Result<ExternalVectorSave, String>,
        interval: Duration,
    ) -> Delivery {
        loop {
            if owned.stamp().ok().as_ref() != Some(stamp) {
                return Delivery::Changed;
            }
            match send.try_send(update) {
                Ok(()) => return Delivery::Sent,
                Err(TrySendError::Full(value)) => {
                    update = value;
                    if !matches!(stopped.recv_timeout(interval), Err(mpsc::RecvTimeoutError::Timeout)) {
                        return Delivery::Stopped;
                    }
                }
                Err(TrySendError::Disconnected(_)) => return Delivery::Stopped,
            }
        }
    }

    /// Worker loop: poll the working file each `interval`, read it only after its stamp stays
    /// unchanged for `settle`, and deliver changed bytes with an optional converted SVG preview
    /// for non-SVG sources. Repeated identical errors and known-failed sources are not re-sent.
    /// Returns when stopped or the UI drops its receiver, dropping the private working copy.
    fn watch(
        owned: OwnedSvg,
        mut last_bytes: Vec<u8>,
        mut child: Option<Child>,
        stopped: Receiver<()>,
        send: SyncSender<Result<ExternalVectorSave, String>>,
        timing: (Duration, Duration),
        converter: Option<(&Path, &photocraft_ui_egui::ExternalVectorSource)>,
    ) {
        let (interval, settle) = timing;
        // The editor may save before the watcher starts. First settled read must compare bytes.
        let mut last_stamp: Option<Stamp> = None;
        let mut pending: Option<(Stamp, Instant)> = None;
        let mut last_error: Option<String> = None;
        let mut failed_source: Option<[u8; 32]> = None;
        while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(interval) {
            if child.as_mut().is_some_and(|process| process.try_wait().ok().flatten().is_some()) {
                child = None;
            }
            let candidate = match owned.stamp() {
                Ok(stamp) => {
                    if last_stamp.as_ref() == Some(&stamp) {
                        continue;
                    }
                    match &pending {
                        Some((old, since)) if old == &stamp && since.elapsed() >= settle => owned.read(&stamp),
                        Some((old, _)) if old == &stamp => continue,
                        _ => {
                            pending = Some((stamp, Instant::now()));
                            continue;
                        }
                    }
                }
                Err(error) => {
                    pending = None;
                    Err(error)
                }
            };
            match candidate {
                Ok(bytes) => {
                    if bytes == last_bytes || failed_source.is_some_and(|hash| hash == *blake3::hash(&bytes).as_bytes()) {
                        last_stamp = pending.as_ref().map(|(stamp, _)| stamp.clone());
                        continue;
                    }
                    let preview = match converter.filter(|(_, source)| source.extension != "svg") {
                        Some((editor, source)) => {
                            let snapshot = photocraft_ui_egui::ExternalVectorSource {
                                bytes: std::sync::Arc::new(bytes.clone()),
                                extension: source.extension.clone(),
                                size: source.size,
                            };
                            match super::vector_conversion::render_preview(editor, &snapshot, &stopped) {
                                Ok(svg) => Some(svg),
                                Err(error) => {
                                    let Some((stamp, _)) = pending.as_ref() else { continue };
                                    if last_error.as_ref() == Some(&error) {
                                        failed_source = Some(*blake3::hash(&bytes).as_bytes());
                                        last_stamp = Some(stamp.clone());
                                    } else {
                                        match deliver(&owned, stamp, &send, &stopped, Err(error.clone()), interval) {
                                            Delivery::Sent => {
                                                last_error = Some(error);
                                                failed_source = Some(*blake3::hash(&bytes).as_bytes());
                                                last_stamp = Some(stamp.clone());
                                            }
                                            Delivery::Changed => {}
                                            Delivery::Stopped => return,
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                        None => None,
                    };
                    let Some((stamp, _)) = pending.as_ref() else { continue };
                    let update = Ok(ExternalVectorSave { bytes: bytes.clone(), preview_svg: preview });
                    match deliver(&owned, stamp, &send, &stopped, update, interval) {
                        Delivery::Sent => {
                            last_error = None;
                            failed_source = None;
                            last_bytes = bytes;
                            last_stamp = Some(stamp.clone());
                        }
                        Delivery::Changed => {}
                        Delivery::Stopped => return,
                    }
                }
                Err(error) => {
                    if !read_failed(error, &mut pending, &mut last_stamp, &mut last_error, &send) {
                        break;
                    }
                }
            }
        }
    }

    /// Handle a failed stamp or read. A save that changed during the read waits to settle
    /// again without a user-visible error. Other errors are reported once until they change.
    /// Returns false when the UI has dropped its receiver.
    fn read_failed(
        error: String,
        pending: &mut Option<(Stamp, Instant)>,
        last_stamp: &mut Option<Stamp>,
        last_error: &mut Option<String>,
        send: &SyncSender<Result<ExternalVectorSave, String>>,
    ) -> bool {
        *last_stamp = None;
        if error == SAVE_IN_PROGRESS {
            *pending = None;
            return true;
        }
        if last_error.as_ref() == Some(&error) {
            return true;
        }
        match send.try_send(Err(error.clone())) {
            Ok(()) => *last_error = Some(error),
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return false,
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Only VectorCraft gets a regenerated AI working copy: Illustrator, another program or a
        /// bundle opened through LaunchServices gets the original.
        #[test]
        fn only_vectorcraft_editors_get_a_regenerated_ai() {
            let root = std::env::temp_dir();
            let program = |name: &str| Resolved::direct(root.join(name));
            assert!(is_vectorcraft(&program(if cfg!(windows) { "vectorcraft.exe" } else { "vectorcraft" }), false));
            assert!(!is_vectorcraft(&program(if cfg!(windows) { "Illustrator.exe" } else { "Illustrator" }), false));
            assert!(is_vectorcraft(&program("vectorcraft-1.2.0-linux-x86_64.AppImage"), false));
            assert!(is_vectorcraft(&program("ai.storyteller.vectorcraft"), false));
            assert!(!is_vectorcraft(&program(if cfg!(windows) { "vectorcraft-cli.exe" } else { "vectorcraft-cli" }), false));
            assert!(!is_vectorcraft(&program("uninstall-vectorcraft.exe"), false));
            assert!(is_vectorcraft(&program("Renamed"), true), "a bundle identified as VectorCraft runs directly");
            let cocoa = Resolved { program: root.join("Editor"), bundle: Some(root.join("Editor.app")) };
            assert!(!is_vectorcraft(&cocoa, true));
        }

        #[test]
        fn conversion_errors_wait_for_queue_capacity_without_disappearing() {
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            let stamp = owned.stamp().unwrap();
            let (send, updates) = mpsc::sync_channel(1);
            send.send(Ok(ExternalVectorSave { bytes: b"previous save".to_vec(), preview_svg: None })).unwrap();
            let (_stop, stopped) = mpsc::channel();
            let worker = thread::spawn(move || deliver(&owned, &stamp, &send, &stopped, Err("native preview failed".into()), Duration::from_millis(2)));
            thread::sleep(Duration::from_millis(30));
            assert_eq!(updates.recv_timeout(Duration::from_secs(2)).unwrap().unwrap().bytes, b"previous save");
            assert_eq!(updates.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err(), "native preview failed");
            assert_eq!(worker.join().unwrap(), Delivery::Sent);
        }

        #[test]
        fn explicitly_selected_vector_build_keeps_its_adjacent_cli() {
            let directory = tempfile::tempdir().unwrap();
            let editor = directory.path().join("custom-vectorcraft");
            let cli = directory.path().join(if cfg!(windows) { "vectorcraft-cli.exe" } else { "vectorcraft-cli" });
            std::fs::write(&cli, b"trusted test fixture").unwrap();
            assert_eq!(resolve_converter(Some(&editor)).unwrap(), editor);
        }

        /// A synthetic `<root>/<name>.app` whose Info.plist is `info` and whose
        /// `Contents/MacOS/<executable>` is an executable script. Returns the bundle path.
        #[cfg(unix)]
        fn fake_bundle(root: &Path, name: &str, executable: &str, info: &[u8]) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let bundle = root.join(format!("{name}.app"));
            std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            std::fs::write(bundle.join("Contents/Info.plist"), info).unwrap();
            let program = bundle.join("Contents/MacOS").join(executable);
            std::fs::write(&program, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            bundle
        }

        #[cfg(unix)]
        fn xml_info(executable: &str) -> Vec<u8> {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleName</key><string>Editor</string><key>CFBundleDocumentTypes</key><array><dict><key>CFBundleTypeName</key><string>SVG</string></dict></array><key>CFBundleExecutable</key><string>{executable}</string></dict></plist>"#
            )
            .into_bytes()
        }

        #[cfg(unix)]
        #[test]
        fn macos_bundle_selection_opens_the_bundle_with_launch_services() {
            let directory = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(directory.path(), "Editor", "Editor Binary", &xml_info("Editor Binary"));
            let resolved = resolve_program(&bundle, true).unwrap();
            let canonical = bundle.canonicalize().unwrap();
            assert_eq!(resolved.bundle.as_deref(), Some(canonical.as_path()));
            assert_eq!(resolved.program, canonical.join("Contents/MacOS/Editor Binary"));
            let file = directory.path().join("Artwork.svg");
            let command = resolved.command(&file);
            assert_eq!(command.get_program(), "/usr/bin/open");
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args, [std::ffi::OsStr::new("-a"), canonical.as_os_str(), file.as_os_str()]);
            // Outside macOS a directory is never an editor, bundle or not.
            assert!(resolve_program(&bundle, false).is_err());
            // A plain executable keeps running directly with the file as its only argument.
            let program = canonical.join("Contents/MacOS/Editor Binary");
            let direct = resolve_program(&program, true).unwrap();
            assert_eq!(direct, Resolved::direct(program.clone()));
            let command = direct.command(&file);
            assert_eq!(command.get_program(), program.as_os_str());
            assert_eq!(command.get_args().collect::<Vec<_>>(), [file.as_os_str()]);
        }

        /// A selected VectorCraft bundle must receive the watched file as argv, including
        /// SVG-only installations without a CLI. Other applications retain open-document
        /// events even if someone placed a vectorcraft-cli file beside their executable.
        #[cfg(unix)]
        #[test]
        fn manually_selected_vectorcraft_bundle_uses_argv_without_misclassifying_other_apps() {
            let directory = tempfile::tempdir().unwrap();
            for binary in [false, true] {
                let mut info = plist::Dictionary::new();
                info.insert("CFBundleExecutable".into(), plist::Value::String("VectorCraft".into()));
                info.insert("CFBundleIdentifier".into(), plist::Value::String("ai.storyteller.vectorcraft".into()));
                let mut bytes = Vec::new();
                if binary {
                    plist::Value::Dictionary(info).to_writer_binary(&mut bytes).unwrap();
                } else {
                    plist::Value::Dictionary(info).to_writer_xml(&mut bytes).unwrap();
                }
                let bundle = fake_bundle(directory.path(), if binary { "Renamed" } else { "VectorCraft" }, "VectorCraft", &bytes);
                let resolved = resolve_program(&bundle, true).unwrap();
                assert!(resolved.bundle.is_none(), "VectorCraft must receive file arguments");
                assert!(!has_adjacent_cli(&resolved.program), "SVG editing works without the converter");
                let file = directory.path().join("Artwork.svg");
                let command = resolved.command(&file);
                assert_eq!(command.get_program(), resolved.program.as_os_str());
                assert_eq!(command.get_args().collect::<Vec<_>>(), [file.as_os_str()]);
            }
            let other = fake_bundle(directory.path(), "Other", "Other", &xml_info("Other"));
            std::fs::write(other.join("Contents/MacOS/vectorcraft-cli"), b"#!/bin/sh\n").unwrap();
            let resolved = resolve_program(&other, true).unwrap();
            assert!(has_adjacent_cli(&resolved.program));
            assert!(resolved.bundle.is_some(), "an adjacent CLI is not the application's identity");
            assert_eq!(resolved.command(&directory.path().join("Artwork.svg")).get_program(), "/usr/bin/open");
        }

        #[cfg(unix)]
        #[test]
        fn macos_bundle_reads_binary_info_plists() {
            let mut info = plist::Dictionary::new();
            info.insert("CFBundleExecutable".into(), plist::Value::String("VectorCraft".into()));
            let mut bytes = Vec::new();
            plist::Value::Dictionary(info).to_writer_binary(&mut bytes).unwrap();
            assert!(bytes.starts_with(b"bplist00"));
            let directory = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(directory.path(), "VectorCraft", "VectorCraft", &bytes);
            let (canonical, program) = bundle_executable(&bundle).unwrap();
            assert_eq!(program, canonical.join("Contents/MacOS/VectorCraft"));
        }

        #[cfg(unix)]
        #[test]
        fn macos_bundle_rejects_escapes_links_and_malformed_info_plists() {
            use std::os::unix::fs::PermissionsExt;
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            // The executable name is a file name inside Contents/MacOS, never a path.
            for name in ["../../../outside", "sub/dir", ".", ".."] {
                let bundle = fake_bundle(root, "Traversal", "Traversal", &xml_info(name));
                assert!(bundle_executable(&bundle).is_err(), "{name}");
            }
            // A link from inside the bundle to an executable elsewhere is not the bundle's program.
            let outside = root.join("outside-tool");
            std::fs::write(&outside, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
            let bundle = fake_bundle(root, "Linked", "Real", &xml_info("Escape"));
            std::os::unix::fs::symlink(&outside, bundle.join("Contents/MacOS/Escape")).unwrap();
            assert!(bundle_executable(&bundle).unwrap_err().contains("inside its bundle"));
            // Contents/MacOS itself redirected out of the bundle.
            let bundle = fake_bundle(root, "LinkedDir", "Tool", &xml_info("Tool"));
            std::fs::remove_dir_all(bundle.join("Contents/MacOS")).unwrap();
            std::fs::create_dir(root.join("elsewhere")).unwrap();
            std::fs::copy(&outside, root.join("elsewhere/Tool")).unwrap();
            std::os::unix::fs::symlink(root.join("elsewhere"), bundle.join("Contents/MacOS")).unwrap();
            assert!(bundle_executable(&bundle).unwrap_err().contains("inside its bundle"));
            // A linked Info.plist is not followed.
            let bundle = fake_bundle(root, "LinkedInfo", "Tool", &xml_info("Tool"));
            let real_info = root.join("Info.plist");
            std::fs::write(&real_info, xml_info("Tool")).unwrap();
            std::fs::remove_file(bundle.join("Contents/Info.plist")).unwrap();
            std::os::unix::fs::symlink(&real_info, bundle.join("Contents/Info.plist")).unwrap();
            assert!(bundle_executable(&bundle).is_err());
            // The named executable must be executable.
            let bundle = fake_bundle(root, "NotExecutable", "Tool", &xml_info("Tool"));
            std::fs::set_permissions(bundle.join("Contents/MacOS/Tool"), std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(bundle_executable(&bundle).is_err());
            // Oversized, non-dictionary, deeply nested and non-string entries fail without panicking.
            let huge = vec![b' '; MAX_INFO_PLIST_BYTES as usize + 1];
            assert!(bundle_executable(&fake_bundle(root, "Huge", "Tool", &huge)).unwrap_err().contains("1 MiB"));
            let deep = format!(
                "<plist><dict><key>Deep</key>{}{}<key>CFBundleExecutable</key><string>Tool</string></dict></plist>",
                "<array>".repeat(100_000),
                "</array>".repeat(100_000)
            );
            assert!(bundle_executable_name(deep.as_bytes()).is_err());
            assert!(bundle_executable_name(b"<plist><array><string>Tool</string></array></plist>").is_err());
            assert!(bundle_executable_name(b"<plist><dict><key>CFBundleExecutable</key><array/></dict></plist>").is_err());
            assert!(bundle_executable_name(b"<plist><dict><key>CFBundleExecutable</key><integer>1</integer></dict></plist>").is_err());
            assert!(bundle_executable_name(b"<plist><dict><key>CFBundleName</key><string>Tool</string></dict></plist>").is_err());
            assert!(bundle_executable_name(b"not a plist").is_err());
            // A nested CFBundleExecutable key does not name the bundle's executable.
            let nested = b"<plist><dict><key>Other</key><dict><key>CFBundleExecutable</key><string>Wrong</string></dict><key>CFBundleExecutable</key><string>Right</string></dict></plist>";
            assert_eq!(bundle_executable_name(nested).unwrap(), "Right");
            // A file or a non-.app directory is not a bundle.
            assert!(bundle_executable(&outside).is_err());
            assert!(bundle_executable(&root.join("elsewhere")).is_err());
            assert!(bundle_executable(Path::new("relative/Editor.app")).is_err());
        }

        #[cfg(unix)]
        #[test]
        fn macos_vectorcraft_discovery_checks_standard_application_folders_first() {
            let home = Path::new("/home/synthetic");
            let mac = vectorcraft_installs(Some(home), true);
            assert_eq!(mac[0], Install::Bundle(PathBuf::from("/Applications/VectorCraft.app")));
            assert_eq!(mac[1], Install::Bundle(home.join("Applications/VectorCraft.app")));
            let other = vectorcraft_installs(Some(home), false);
            assert!(other.iter().all(|install| matches!(install, Install::Executable(_))), "no bundles outside macOS");
            assert_eq!(&mac[2..], other.as_slice(), "the Unix locations keep their order");
            assert_eq!(
                other,
                [
                    Install::Executable(home.join(".local/share/flatpak/app/ai.storyteller.vectorcraft/current/active/files/bin/vectorcraft")),
                    Install::Executable(PathBuf::from("/usr/local/bin/vectorcraft")),
                    Install::Executable(PathBuf::from("/usr/bin/vectorcraft")),
                    Install::Executable(home.join(".local/bin/vectorcraft")),
                ]
            );
            assert_eq!(vectorcraft_installs(None, true)[0], Install::Bundle(PathBuf::from("/Applications/VectorCraft.app")));
        }

        #[cfg(unix)]
        #[test]
        fn bundled_vectorcraft_cli_is_found_inside_the_same_bundle() {
            use std::os::unix::fs::PermissionsExt;
            let directory = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(directory.path(), "VectorCraft", "VectorCraft", &xml_info("VectorCraft"));
            let (_, program) = bundle_executable(&bundle).unwrap();
            assert!(!has_adjacent_cli(&program), "no CLI yet");
            // A CLI beside the bundle directory is not inside the bundle.
            std::fs::write(directory.path().join("vectorcraft-cli"), b"#!/bin/sh\n").unwrap();
            assert!(!has_adjacent_cli(&program));
            let cli = bundle.join("Contents/MacOS/vectorcraft-cli");
            std::fs::write(&cli, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(resolve_converter(Some(&program)).unwrap(), program);
        }

        #[cfg(unix)]
        #[test]
        fn macos_illustrator_discovery_is_one_level_deep_and_prefers_the_newest() {
            let directory = tempfile::tempdir().unwrap();
            let applications = directory.path();
            for version in ["2024", "2025"] {
                let folder = applications.join(format!("Adobe Illustrator {version}"));
                std::fs::create_dir(&folder).unwrap();
                fake_bundle(&folder, &format!("Adobe Illustrator {version}"), "Adobe Illustrator", &xml_info("Adobe Illustrator"));
            }
            // Broken newer install, unrelated apps and deeper nesting are skipped.
            std::fs::create_dir_all(applications.join("Adobe Illustrator 2026/Adobe Illustrator 2026.app")).unwrap();
            fake_bundle(applications, "Other Editor", "Other", &xml_info("Other"));
            std::fs::create_dir_all(applications.join("Adobe Illustrator 2027/Deeper")).unwrap();
            fake_bundle(&applications.join("Adobe Illustrator 2027/Deeper"), "Adobe Illustrator 2027", "Adobe Illustrator", &xml_info("Adobe Illustrator"));
            let found = illustrator_bundles(applications);
            assert_eq!(found.len(), 3, "{found:?}");
            assert!(found.iter().all(|path| path.file_name().unwrap().to_str().unwrap().starts_with("Adobe Illustrator")));
            let newest = newest_illustrator(applications).unwrap();
            let expected = applications.join("Adobe Illustrator 2025/Adobe Illustrator 2025.app").canonicalize().unwrap();
            assert_eq!(newest.bundle, Some(expected.clone()));
            assert_eq!(newest.program, expected.join("Contents/MacOS/Adobe Illustrator"));
            assert!(newest_illustrator(&applications.join("missing")).unwrap_err().contains(OTHER_APPLICATION));
        }

        #[test]
        fn missing_editor_errors_name_the_dialog_option() {
            let dialog = include_str!("../../../crates/ui-egui/src/smart_svg_ui.rs");
            assert!(dialog.contains(&format!("(Editor::Program, \"{OTHER_APPLICATION}\")")), "the dialog label changed");
            let source = include_str!("external_svg.rs");
            assert!(!source.contains(concat!("Other ", "editor")), "errors must name the dialog's option");
            #[cfg(target_os = "linux")]
            assert!(resolve(ExternalSvgEditor::Illustrator).unwrap_err().contains(OTHER_APPLICATION));
        }

        const EMPTY_SAVE: &str = "The edited SVG must contain between 1 byte and 16 MiB";

        fn update(session: &mut Session, deadline: Instant) -> Result<Vec<u8>, String> {
            loop {
                assert!(Instant::now() < deadline, "timed out waiting for an SVG save");
                if let Some(update) = session.poll() {
                    return update;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn expect_saved(session: &mut Session, expected: &[u8], deadline: Instant) {
            loop {
                match update(session, deadline) {
                    Ok(bytes) => {
                        assert_eq!(bytes, expected, "unexpected SVG save contents");
                        return;
                    }
                    // A save that changes during a read is retried by the watcher, never shown.
                    Err(error) => assert!(error == EMPTY_SAVE, "unexpected SVG watcher error: {error}"),
                }
            }
        }

        fn expect_empty_error(session: &mut Session, deadline: Instant) {
            match update(session, deadline) {
                Err(error) if error == EMPTY_SAVE => {}
                other => panic!("expected the held-empty SVG error, got {other:?}"),
            }
        }

        fn queued_session(updates: Vec<Result<Vec<u8>, String>>) -> Session {
            let (stop, _stopped) = mpsc::channel();
            let (send, receive) = mpsc::channel();
            for update in updates {
                send.send(update.map(|bytes| ExternalVectorSave { bytes, preview_svg: None })).unwrap();
            }
            Session { stop, updates: receive, finished: false }
        }

        #[test]
        fn finished_session_reports_queued_error_before_releasing_capacity() {
            let mut session = queued_session(vec![Err("Editor selection cancelled".into())]);
            assert!(!session.finished());
            assert!(session.poll().unwrap().is_err());
            assert!(!session.finished());
            assert!(session.poll().is_none());
            assert!(session.finished());
        }

        #[test]
        fn saves_debounce_repeat_and_recover_after_invalid_file() {
            let initial = b"<svg/>".to_vec();
            let owned = OwnedSvg::create(&initial).unwrap();
            let file = owned.file.clone();
            let root = owned.root.clone();
            let settle = Duration::from_millis(100);
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut session = start(owned, initial, None, Duration::from_millis(10), settle).unwrap();
            let save_started = Instant::now();
            let first = b"<svg>first complete</svg>";
            std::fs::write(&file, first).unwrap();
            expect_saved(&mut session, first, deadline);
            assert!(save_started.elapsed() >= settle, "a changed save must settle before delivery");
            assert!(session.poll().is_none());
            std::fs::write(&file, b"").unwrap();
            expect_empty_error(&mut session, deadline);
            std::fs::write(&file, b"<svg>second complete</svg>").unwrap();
            expect_saved(&mut session, b"<svg>second complete</svg>", deadline);
            assert!(session.poll().is_none());
            drop(session);
            while root.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!root.exists(), "worker must clean its owned temporary SVG");
        }

        #[test]
        fn exact_save_wait_skips_only_expected_queued_transient_errors() {
            let bytes = b"<svg>complete</svg>";
            let mut session = queued_session(vec![Err(EMPTY_SAVE.into()), Ok(bytes.to_vec())]);
            expect_saved(&mut session, bytes, Instant::now() + Duration::from_secs(3));
            assert!(session.poll().is_none());
        }

        #[test]
        #[should_panic(expected = "unexpected SVG watcher error")]
        fn exact_save_wait_rejects_unexpected_errors() {
            let mut session = queued_session(vec![Err("Cannot read the edited SVG: permission denied".into()), Ok(b"<svg/>".to_vec())]);
            expect_saved(&mut session, b"<svg/>", Instant::now() + Duration::from_secs(3));
        }

        #[test]
        #[should_panic(expected = "unexpected SVG watcher error")]
        fn exact_save_wait_rejects_a_reported_save_in_progress() {
            let mut session = queued_session(vec![Err(SAVE_IN_PROGRESS.into()), Ok(b"<svg/>".to_vec())]);
            expect_saved(&mut session, b"<svg/>", Instant::now() + Duration::from_secs(3));
        }

        /// A save that changes during a read must settle again without a UI message, while
        /// other read errors are still reported once and dropped receivers stop the watcher.
        #[test]
        fn changing_save_retries_silently_but_other_read_errors_report_once() {
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            let stamp = owned.stamp().unwrap();
            let (send, updates) = mpsc::sync_channel(2);
            let mut pending = Some((stamp.clone(), Instant::now()));
            let mut last_stamp = Some(stamp.clone());
            let mut last_error = Some(EMPTY_SAVE.to_string());
            assert!(read_failed(SAVE_IN_PROGRESS.into(), &mut pending, &mut last_stamp, &mut last_error, &send));
            assert!(pending.is_none(), "the changed save must settle again before it is read");
            assert!(last_stamp.is_none(), "the next settled stamp must be read");
            assert_eq!(last_error.as_deref(), Some(EMPTY_SAVE), "a retry neither reports nor forgets the shown error");
            assert!(updates.try_recv().is_err(), "a save in progress is not a user-visible error");

            let denied = "Cannot read the edited SVG: permission denied".to_string();
            pending = Some((stamp.clone(), Instant::now()));
            assert!(read_failed(denied.clone(), &mut pending, &mut last_stamp, &mut last_error, &send));
            assert!(pending.is_some(), "other read errors keep their existing settle state");
            assert_eq!(updates.try_recv().unwrap().unwrap_err(), denied);
            assert!(read_failed(denied.clone(), &mut pending, &mut last_stamp, &mut last_error, &send));
            assert!(updates.try_recv().is_err(), "a repeated error is reported once");

            let full = || Ok(ExternalVectorSave { bytes: b"<svg/>".to_vec(), preview_svg: None });
            send.try_send(full()).unwrap();
            send.try_send(full()).unwrap();
            let gone = "Cannot inspect the edited SVG: not found".to_string();
            assert!(read_failed(gone.clone(), &mut pending, &mut last_stamp, &mut last_error, &send));
            assert_eq!(last_error.as_deref(), Some(denied.as_str()), "an error the full queue refused is retried later");
            drop(updates);
            assert!(!read_failed(gone, &mut pending, &mut last_stamp, &mut last_error, &send), "a dropped UI ends the watcher");
        }

        /// Saves that replace the file while it is being read deliver only complete SVGs, never
        /// a transient error, and the final stable save always arrives.
        #[cfg(unix)]
        #[test]
        fn saves_replaced_during_reads_deliver_the_final_save_without_transient_errors() {
            let initial = b"<svg/>".to_vec();
            let owned = OwnedSvg::create(&initial).unwrap();
            let file = owned.file.clone();
            let mut session = start(owned, initial, None, Duration::from_millis(1), Duration::ZERO).unwrap();
            let final_svg = b"<svg>final</svg>".to_vec();
            let expected = final_svg.clone();
            let writer = thread::spawn(move || {
                let next = file.with_extension("next");
                for i in 0..300usize {
                    let body = "x".repeat((i * 7919) % 262_144);
                    std::fs::write(&next, format!("<svg>{body}</svg>")).unwrap();
                    std::fs::rename(&next, &file).unwrap();
                    thread::sleep(Duration::from_micros((i as u64 * 37) % 900));
                }
                std::fs::write(&next, &final_svg).unwrap();
                std::fs::rename(&next, &file).unwrap();
            });
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut deliveries = 0usize;
            loop {
                match update(&mut session, deadline) {
                    Ok(bytes) => {
                        deliveries += 1;
                        assert!(bytes.starts_with(b"<svg>") && bytes.ends_with(b"</svg>"), "a torn save was delivered");
                        // Only the writer's last save has this content.
                        if bytes == expected {
                            break;
                        }
                    }
                    Err(error) => panic!("unexpected SVG watcher error during replacement saves: {error}"),
                }
            }
            writer.join().unwrap();
            assert!(deliveries >= 1);
            thread::sleep(Duration::from_millis(50));
            assert!(session.poll().is_none(), "a stable save is delivered once");
        }

        #[test]
        fn oversize_file_is_rejected_before_allocating() {
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            std::fs::File::create(&owned.file).unwrap().set_len(MAX_BYTES as u64 + 1).unwrap();
            assert!(owned.stamp().is_err());
            assert!(OwnedSvg::create(&[]).is_err());
        }

        #[cfg(unix)]
        #[test]
        fn atomic_replacement_with_equal_size_and_mtime_is_a_new_save() {
            let initial = b"<svg>one</svg>".to_vec();
            let owned = OwnedSvg::create(&initial).unwrap();
            let file = owned.file.clone();
            let original_stamp = owned.stamp().unwrap();
            let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
            let mut session = start(owned, initial, None, Duration::from_millis(10), Duration::from_millis(40)).unwrap();
            let replacement = file.with_extension("replacement");
            let saved = b"<svg>two</svg>";
            std::fs::write(&replacement, saved).unwrap();
            std::fs::File::options().write(true).open(&replacement).unwrap().set_modified(modified).unwrap();
            std::fs::rename(&replacement, &file).unwrap();
            let replaced_stamp = Stamp::from_metadata(&std::fs::metadata(&file).unwrap()).unwrap();
            assert_eq!(original_stamp.modified, replaced_stamp.modified);
            assert_eq!(original_stamp.length, replaced_stamp.length);
            assert!(original_stamp != replaced_stamp, "file identity must distinguish equal-time atomic saves");
            expect_saved(&mut session, saved, Instant::now() + Duration::from_secs(3));
        }

        #[cfg(unix)]
        #[test]
        fn unix_saved_file_rejects_symlinks_and_fifos_without_blocking() {
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            let stamp = owned.stamp().unwrap();
            let target = owned.root.join("private.svg");
            std::fs::write(&target, b"private contents").unwrap();
            std::fs::remove_file(&owned.file).unwrap();
            std::os::unix::fs::symlink(&target, &owned.file).unwrap();
            assert!(owned.stamp().is_err());
            assert!(owned.read(&stamp).is_err());
            assert!(owned.overwrite(b"<svg/>").is_err());
            std::fs::remove_file(&owned.file).unwrap();
            rustix::fs::mkfifoat(rustix::fs::CWD, &owned.file, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
            assert!(owned.stamp().is_err());
            let began = Instant::now();
            assert!(owned.read(&stamp).is_err());
            assert!(began.elapsed() < Duration::from_secs(1));
            assert_eq!(std::fs::read(&target).unwrap(), b"private contents");
        }

        #[cfg(windows)]
        #[test]
        fn script_and_command_line_are_not_editor_executables() {
            assert!(validate_executable(Path::new("cmd.exe /c start")).is_err());
            assert!(validate_executable(Path::new("C:/editor.cmd")).is_err());
            assert!(validate_executable(Path::new("editor.exe")).is_err());
        }

        #[cfg(windows)]
        #[test]
        fn replacement_symlink_is_rejected_without_reading_target() {
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            let target = owned.root.join("other.svg");
            std::fs::write(&target, b"private target").unwrap();
            std::fs::remove_file(&owned.file).unwrap();
            // Symlink creation requires Developer Mode or elevation. Other tests run everywhere.
            if std::os::windows::fs::symlink_file(&target, &owned.file).is_err() {
                return;
            }
            assert!(owned.stamp().is_err());
            let _ = std::fs::remove_file(&owned.file);
        }
    }
}

#[path = "vector_conversion.rs"]
mod vector_conversion;

#[cfg(target_os = "linux")]
#[path = "external_svg_portal.rs"]
mod portal;

#[cfg(test)]
#[path = "native_saveback_tests.rs"]
mod native_saveback_tests;
