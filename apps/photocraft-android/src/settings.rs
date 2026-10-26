//! Edit › Preferences › Export Settings… / Import Settings…: every setting this app keeps, in one
//! `.zip` file the user saves where they like (a backup, or to move to another device; #17).
//!
//! The file holds the preferences, the presets (brushes, adjustments, tool presets and brush
//! tips), the Shortcut Pad, Touch Input and layout settings, and imported fonts. Never documents,
//! crash-recovery data or the file bridge.
//!
//! The app keeps its settings in memory and writes them back as they change, so an import can't
//! replace the files under a running app: it checks the file and stages its contents in
//! `settings-import/`, and the next start moves them into place ([`apply_pending`]) before
//! anything is loaded.

use crate::bridge::Bridge;
use crate::fonts::Outcome;
use photocraft_format::zip::{ZipReader, ZipWriter};
use photocraft_ui_egui::menus::MenuItem;
use photocraft_ui_egui::platform_menu::PlatformMenu;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

const EXPORT: &str = "android.settings.export";
const IMPORT: &str = "android.settings.import";
const MENU: &[&str] = &["Edit", "Preferences"];

/// The settings files in the app's files folder.
const FILES: &[&str] = &["prefs.json", "shortcut-pad.json", "touch-input.json", "layout.txt", "update-check-off"];
/// The settings folders: the presets (replaced on import) and imported fonts (added to).
const PRESETS: &str = "presets";
const FONTS: &str = "fonts";
/// Names the export, so an import can tell a settings file from any other zip.
const MARKER: &str = "photocraft-android-settings.json";
/// Where an import waits for the next start.
const STAGING: &str = "settings-import";
/// No settings file is this large (an entry, and all of them together).
const MAX_ENTRY: usize = 256 << 20;
const MAX_TOTAL: usize = 1 << 30;
/// A folder nested deeper than this inside the presets isn't one the app wrote.
const MAX_DEPTH: usize = 8;

/// The suggested name of an export.
pub const FILE_NAME: &str = "PhotoCraft Settings.zip";

/// Every file under `dir`, as paths relative to `base` with `/` separators (sorted).
fn files_under(base: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        if path.is_dir() {
            if depth < MAX_DEPTH {
                files_under(base, &path, depth + 1, out);
            }
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
}

/// The settings files in `files_dir`, relative paths.
fn settings_files(files_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = FILES.iter().filter(|f| files_dir.join(f).is_file()).map(|f| f.to_string()).collect();
    for dir in [PRESETS, FONTS] {
        files_under(files_dir, &files_dir.join(dir), 0, &mut out);
    }
    out
}

/// The export: a zip of every settings file, and a marker naming it.
pub fn export(files_dir: &Path) -> Result<Vec<u8>, String> {
    let mut zip = ZipWriter::new();
    let marker = serde_json::json!({ "format": 1, "app": "PhotoCraft for Android", "version": photocraft_engine::build_info::long_version() });
    zip.add(MARKER, marker.to_string().as_bytes()).map_err(|e| e.to_string())?;
    for rel in settings_files(files_dir) {
        let bytes = std::fs::read(files_dir.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
        zip.add(&rel, &bytes).map_err(|e| format!("{rel}: {e}"))?;
    }
    zip.finish().map_err(|e| e.to_string())
}

/// Whether `name` (from a settings file) is one the app keeps: a settings file, or a file inside
/// the presets or fonts folder, with no way out of them.
fn allowed(name: &str) -> bool {
    if FILES.contains(&name) {
        return true;
    }
    let parts: Vec<&str> = name.split('/').collect();
    let inside = matches!(parts.first(), Some(&PRESETS) | Some(&FONTS)) && parts.len() >= 2 && parts.len() <= MAX_DEPTH + 2;
    inside && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains('\\') && !p.contains(':') && !p.starts_with('.'))
}

/// Check a settings file and stage its contents for the next start. Returns how many files it
/// holds.
pub fn stage(files_dir: &Path, bytes: &[u8]) -> Result<usize, String> {
    let zip = ZipReader::new(bytes).map_err(|_| "this isn't a PhotoCraft settings file".to_string())?;
    let marker = zip.read_by_name(MARKER, 64 << 10).map_err(|_| "this isn't a PhotoCraft settings file".to_string())?;
    let marker: Value = serde_json::from_slice(&marker).map_err(|_| "this settings file is damaged".to_string())?;
    if marker.get("format").and_then(Value::as_u64) != Some(1) {
        return Err("this settings file comes from a newer version of PhotoCraft".into());
    }
    let staging = files_dir.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    let mut total = 0usize;
    let mut count = 0;
    for e in &zip.entries {
        if e.name == MARKER || e.name.ends_with('/') {
            continue;
        }
        if !allowed(&e.name) {
            log::warn!("settings import: skipped {}", e.name);
            continue;
        }
        let data = zip.read(e, MAX_ENTRY).map_err(|_| format!("{} is damaged", e.name))?;
        total = total.saturating_add(data.len());
        if total > MAX_TOTAL {
            let _ = std::fs::remove_dir_all(&staging);
            return Err("this settings file is too large".into());
        }
        let to = staging.join(&e.name);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
        photocraft_format::atomic_write(&to, &data).map_err(|err| err.to_string())?;
        count += 1;
    }
    if count == 0 {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("this settings file is empty".into());
    }
    Ok(count)
}

/// At start, before anything is loaded: move a staged import into place. Its presets replace
/// the old ones; its fonts are added to the imported fonts.
pub fn apply_pending(files_dir: &Path) {
    let staging = files_dir.join(STAGING);
    if !staging.is_dir() {
        return;
    }
    let mut moved = 0;
    let mut place = |from: PathBuf, to: PathBuf| {
        if let Some(dir) = to.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => moved += 1,
            Err(e) => log::warn!("settings import: {}: {e}", to.display()),
        }
    };
    for f in FILES {
        let from = staging.join(f);
        if from.is_file() {
            place(from, files_dir.join(f));
        }
    }
    if staging.join(PRESETS).is_dir() {
        let _ = std::fs::remove_dir_all(files_dir.join(PRESETS));
        place(staging.join(PRESETS), files_dir.join(PRESETS));
    }
    let mut fonts = Vec::new();
    files_under(&staging, &staging.join(FONTS), 0, &mut fonts);
    for rel in fonts {
        place(staging.join(&rel), files_dir.join(&rel));
    }
    let _ = std::fs::remove_dir_all(&staging);
    log::info!("settings import: {moved} item(s) applied");
}

fn tell(outcome: &Outcome, message: String, error: bool) {
    *outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some((message, error));
}

/// The Edit › Preferences rows. `outcome` gets the result for the status bar; `wake` repaints.
pub fn menu(files_dir: &Path, bridge: Arc<Bridge>, outcome: Outcome, wake: impl Fn() + Send + Sync + Clone + 'static) -> PlatformMenu {
    let files_dir = files_dir.to_path_buf();
    PlatformMenu {
        items: Box::new(|| {
            let row = |id: &str, label: &str| MenuItem {
                id: id.into(),
                label: label.into(),
                path: MENU.iter().map(|s| s.to_string()).collect(),
                shortcut: None,
                enabled: true,
                checked: None,
                color: None,
            };
            vec![row(EXPORT, "Export Settings…"), row(IMPORT, "Import Settings…")]
        }),
        run: Box::new(move |id| {
            let (dir, outcome, wake, b) = (files_dir.clone(), outcome.clone(), wake.clone(), bridge.clone());
            match id {
                EXPORT => {
                    let request = FileDialogRequest::Save { suggested: dir.join(FILE_NAME).to_string_lossy().into_owned() };
                    crate::services::ask(&bridge, request, move |answer| {
                        let path = match answer {
                            Some(FileDialogAnswer::SaveTo(p)) => p,
                            Some(FileDialogAnswer::Unavailable(e)) => return tell(&outcome, format!("Couldn't open the file picker: {e}"), true),
                            _ => return,
                        };
                        let result = export(&dir).and_then(|bytes| crate::services::write_and_sync(&b, Path::new(&path), &bytes));
                        match result {
                            Ok(()) => tell(&outcome, "Settings exported.".into(), false),
                            Err(e) => tell(&outcome, format!("Couldn't export the settings: {e}"), true),
                        }
                        wake();
                    });
                }
                IMPORT => {
                    let request = FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["zip".into()]) };
                    crate::services::ask(&bridge, request, move |answer| {
                        let path = match answer {
                            Some(FileDialogAnswer::Paths(p)) if !p.is_empty() => p[0].clone(),
                            Some(FileDialogAnswer::Unavailable(e)) => return tell(&outcome, format!("Couldn't open the file picker: {e}"), true),
                            _ => return,
                        };
                        let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| stage(&dir, &bytes));
                        match result {
                            Ok(_) => tell(&outcome, "Settings imported. Close PhotoCraft (File › Exit) and open it again to use them.".into(), false),
                            Err(e) => tell(&outcome, format!("Couldn't import the settings: {e}"), true),
                        }
                        wake();
                    });
                }
                _ => return Err(format!("unknown settings command {id}")),
            }
            Ok(Value::Null)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn settings_travel_to_another_install_and_apply_at_the_next_start() {
        let a = temp("a");
        write(&a, "prefs.json", r#"{"theme":"light"}"#);
        write(&a, "shortcut-pad.json", "{\"side\":\"Left\"}");
        write(&a, "presets/brushes/index.json", "[1]");
        write(&a, "presets/tips/t1.bin", "\u{1}\u{2}");
        write(&a, "fonts/Comic.ttf", "font");
        write(&a, "recovery/doc-1.pcraft", "not settings");
        write(&a, "bridge/answer.json", "not settings");
        let zip = export(&a).unwrap();

        let b = temp("b");
        write(&b, "prefs.json", "{\"theme\":\"dark\"}");
        write(&b, "presets/old.json", "old");
        write(&b, "fonts/Mine.ttf", "mine");
        assert_eq!(stage(&b, &zip).unwrap(), 5, "the settings, not documents or the bridge");
        assert_eq!(std::fs::read_to_string(b.join("prefs.json")).unwrap(), "{\"theme\":\"dark\"}", "nothing changes until the next start");
        apply_pending(&b);
        assert_eq!(std::fs::read_to_string(b.join("prefs.json")).unwrap(), r#"{"theme":"light"}"#);
        assert_eq!(std::fs::read_to_string(b.join("shortcut-pad.json")).unwrap(), "{\"side\":\"Left\"}");
        assert!(b.join("presets/tips/t1.bin").is_file() && !b.join("presets/old.json").exists(), "presets replaced");
        assert!(b.join("fonts/Comic.ttf").is_file() && b.join("fonts/Mine.ttf").is_file(), "fonts added");
        assert!(!b.join("recovery").exists() && !b.join(STAGING).exists());
        apply_pending(&b); // nothing staged: nothing happens
        let _ = (std::fs::remove_dir_all(a), std::fs::remove_dir_all(b));
    }

    #[test]
    fn other_files_and_hostile_names_are_refused() {
        let d = temp("bad");
        assert!(stage(&d, b"not a zip").is_err());
        let mut z = ZipWriter::new();
        z.add("hello.txt", b"hi").unwrap();
        assert!(stage(&d, &z.finish().unwrap()).unwrap_err().contains("isn't a PhotoCraft settings file"));
        // A settings file whose entries try to leave the settings folders.
        let mut z = ZipWriter::new();
        z.add(MARKER, br#"{"format":1}"#).unwrap();
        for name in ["../escape.json", "presets/../../x", "/abs.json", "bridge/answer.json", "presets/.hidden", "fonts\\..\\x.ttf"] {
            z.add(name, b"x").unwrap();
        }
        z.add("layout.txt", b"phone").unwrap();
        assert_eq!(stage(&d, &z.finish().unwrap()).unwrap(), 1, "only layout.txt");
        apply_pending(&d);
        assert!(d.join("layout.txt").is_file() && !d.join("escape.json").exists() && !d.parent().unwrap().join("escape.json").exists());
        // A newer format isn't guessed at.
        let mut z = ZipWriter::new();
        z.add(MARKER, br#"{"format":2}"#).unwrap();
        z.add("layout.txt", b"phone").unwrap();
        assert!(stage(&d, &z.finish().unwrap()).unwrap_err().contains("newer"));
        assert!(allowed("presets/brushes/a.json") && allowed("prefs.json") && !allowed("presets") && !allowed("presets/"));
        let _ = std::fs::remove_dir_all(d);
    }
}
