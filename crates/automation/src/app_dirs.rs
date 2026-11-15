//! Where PhotoCraft keeps its per-user data, resolved the same way by the desktop app and
//! `photocraft-cli` (which finds the app's agent connection file there, see [`crate::agent_link`]).
//!
//! In order:
//! 1. `PHOTOCRAFT_CONFIG_DIR`, when set (tests, agents, custom setups).
//! 2. **Portable mode** (#228): a marker file ([`PORTABLE_MARKERS`]) beside the executable puts
//!    everything in `<exe dir>/PhotoCraftData`, so a portable copy on a USB stick leaves nothing in
//!    `%APPDATA%`. The Windows portable zip ships with `portable.txt`; the MSI doesn't. If that
//!    folder can't be written (read-only medium, Program Files), the platform folder is used and
//!    [`DataDir::warning`] says why.
//! 3. The platform convention: macOS `~/Library/Application Support/Photocraft`, Windows
//!    `%APPDATA%\Photocraft`, Linux `$XDG_CONFIG_HOME/photocraft` or `~/.config/photocraft`.
//!
//! The desktop app's `app_dirs` resolves it once per process and adds the crash-recovery folder.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Files beside the executable that switch on portable mode (either one; contents are ignored).
pub const PORTABLE_MARKERS: [&str; 2] = ["portable.txt", "PhotoCraft.portable"];
/// The data folder created beside the executable in portable mode.
pub const PORTABLE_DATA_DIR: &str = "PhotoCraftData";

/// How [`DataDir::dir`] was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `PHOTOCRAFT_CONFIG_DIR`.
    Override,
    /// `<exe dir>/PhotoCraftData`.
    Portable,
    /// The platform's per-user config directory.
    Platform,
}

/// The resolved data directory.
#[derive(Clone, Debug)]
pub struct DataDir {
    /// `None` only when no platform directory is known either (no `HOME`/`APPDATA`).
    pub dir: Option<PathBuf>,
    pub mode: Mode,
    /// Set when a portable marker was found but its data folder isn't writable.
    pub warning: Option<String>,
}

/// Resolve the data directory from the environment (`env`) and the executable's folder.
pub fn resolve(env: impl Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> DataDir {
    if let Some(d) = env("PHOTOCRAFT_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return DataDir { dir: Some(PathBuf::from(d)), mode: Mode::Override, warning: None };
    }
    let platform = platform_dir(&env);
    let Some((exe_dir, marker)) = exe_dir.and_then(|d| portable_marker(d).map(|m| (d, m))) else {
        return DataDir { dir: platform, mode: Mode::Platform, warning: None };
    };
    let data = exe_dir.join(PORTABLE_DATA_DIR);
    match ensure_writable(&data) {
        Ok(()) => DataDir { dir: Some(data), mode: Mode::Portable, warning: None },
        Err(e) => {
            let fallback = platform.as_ref().map_or_else(|| "nowhere (no user folder found)".to_string(), |p| p.display().to_string());
            let warning =
                format!("portable mode ({} found) but {} isn't writable ({e}); settings are kept in {fallback} instead", marker.display(), data.display());
            DataDir { dir: platform, mode: Mode::Platform, warning: Some(warning) }
        }
    }
}

/// The platform convention, from `env` (so tests can fake `HOME`/`APPDATA`/`XDG_CONFIG_HOME`).
pub fn platform_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/Photocraft"));
    }
    if cfg!(windows) {
        return env("APPDATA").filter(|a| !a.is_empty()).map(|a| PathBuf::from(a).join("Photocraft"));
    }
    env("XDG_CONFIG_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|c| c.join("photocraft"))
}

/// The first portable marker file in `exe_dir`, if any.
pub fn portable_marker(exe_dir: &Path) -> Option<PathBuf> {
    PORTABLE_MARKERS.iter().map(|m| exe_dir.join(m)).find(|p| p.is_file())
}

/// Probe names tried before giving up on a portable data folder.
pub const WRITE_PROBE_ATTEMPTS: u32 = 64;

/// Create `dir` and prove a file can be written in it (a folder can exist yet be read-only).
///
/// The probe is created exclusively (`create_new`), so an entry already at its name (a link
/// someone else placed there, say) is never opened, written through or removed: the next name
/// is tried instead. Only a probe this call created is removed.
pub fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let pid = std::process::id();
    for attempt in 0..WRITE_PROBE_ATTEMPTS {
        let probe = dir.join(format!(".write-test-{pid}-{attempt}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                // A leftover probe is harmless; don't fail portable mode over it.
                let _ = std::fs::remove_file(&probe);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "every write-test name is already taken"))
}

/// The data directory for this process's environment and executable (not cached).
pub fn from_env() -> DataDir {
    let exe = std::env::current_exe().ok();
    resolve(|k| std::env::var_os(k), exe.as_deref().and_then(Path::parent))
}
