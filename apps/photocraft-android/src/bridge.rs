//! The file bridge between the Rust app and its Activity: Android's Storage Access Framework
//! (document pickers, `content://` URIs) lives on the Java side, the documents on the Rust side.
//! They talk through small JSON files in the app's private storage (`<files>/bridge`), so no
//! JNI calls (and no `unsafe`) are needed:
//!
//! - `request.json`: Rust asks for a picker (`{"id", "kind": "open" | "save", …}`). The Activity
//!   polls for it, deletes it and shows the picker.
//! - `answer.json`: the Activity's answer for that id: copies of the picked documents in
//!   `inbox/<id>/` (Open), or a path in `out/<id>/` to write to (Save), or `cancelled`.
//! - `sync/*.json`: Rust wrote a file under the bridge (`{"path"}`); the Activity copies it back
//!   to the document it stands for, which it remembers by path (Save As, and Save of an opened
//!   document whose provider grants write access).
//! - `quit`: the app is leaving (File › Exit, once unsaved work is settled). The Activity copies
//!   back any saves still pending, finishes, and ends the process: winit's event loop can't be
//!   created twice in one process, so the next launch must start a new one.
//! - `incoming/`: documents another app sent (View / Share intents), copied in by the Activity
//!   and opened by the app.
//! - `check-update`: look for a newer release now (Help › Check for Updates…; see `updates.rs`).
//! - `keyboard-hidden`: left by the Activity when the on-screen keyboard is put away.
//!
//! Each side writes a file under a temporary name and renames it, so the other side never reads
//! a partial one.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A picker request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Request {
    Open { id: u64, multiple: bool, mimes: Vec<String> },
    Save { id: u64, name: String, mime: String },
}

/// The Activity's answer to request `id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub id: u64,
    #[serde(default)]
    pub cancelled: bool,
    /// Open: local copies of the picked documents.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Save: where to write.
    #[serde(default)]
    pub path: Option<String>,
    /// Why the picker failed, when it did.
    #[serde(default)]
    pub error: Option<String>,
}

/// Most bytes [`Bridge::take_incoming`] reads from one sent document (larger ones are skipped
/// with an error rather than exhausting memory).
const MAX_INCOMING: u64 = 2 << 30;

pub struct Bridge {
    root: PathBuf,
    next_id: AtomicU64,
}

impl Bridge {
    /// The bridge under the app's files directory; creates its folders and drops any request or
    /// answer left over from an earlier run.
    pub fn new(files_dir: &Path) -> std::io::Result<Self> {
        let root = files_dir.join("bridge");
        for dir in ["inbox", "out", "sync", "incoming"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        for stale in ["request.json", "answer.json", "quit", "keyboard-hidden", "check-update"] {
            let _ = std::fs::remove_file(root.join(stale));
        }
        // Ids only need to be unique within a run; start from the clock so a late answer from
        // an earlier run can't match.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64);
        Ok(Self { root, next_id: AtomicU64::new(seed) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Ask the Activity for a picker.
    pub fn request(&self, r: &Request) -> Result<(), String> {
        let text = serde_json::to_vec(r).map_err(|e| e.to_string())?;
        write_then_rename(&self.root.join("request.json"), &text)
    }

    /// The answer to request `id`, once it arrived (consumed). An answer for another id is
    /// stale and dropped.
    pub fn take_answer(&self, id: u64) -> Option<Answer> {
        let path = self.root.join("answer.json");
        let text = std::fs::read(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        let answer: Answer = serde_json::from_slice(&text).map_err(|e| log::warn!("bridge: unreadable answer: {e}")).ok()?;
        (answer.id == id).then_some(answer)
    }

    /// Tell the Activity that `path` was written, so it copies it to the document it stands
    /// for. Paths outside the bridge (the app's own files) need nothing.
    pub fn mark_written(&self, path: &Path) -> Result<(), String> {
        if !path.starts_with(&self.root) {
            return Ok(());
        }
        let text = serde_json::to_vec(&serde_json::json!({ "path": path.to_string_lossy() })).map_err(|e| e.to_string())?;
        write_then_rename(&self.root.join("sync").join(format!("{}.json", self.next_id())), &text)
    }

    /// Whether the on-screen keyboard was put away since the last call (the Activity leaves a
    /// `keyboard-hidden` marker; winit drops the event).
    pub fn take_keyboard_hidden(&self) -> bool {
        std::fs::remove_file(self.root.join("keyboard-hidden")).is_ok()
    }

    /// Put `text` on the system clipboard (a text field's Copy or Cut).
    pub fn set_clipboard(&self, text: &str) -> Result<(), String> {
        write_then_rename(&self.root.join("clip-set.txt"), text.as_bytes())
    }

    /// Ask for the system clipboard's text (a Paste); it comes back through [`Self::take_clipboard`].
    pub fn request_clipboard(&self) -> Result<(), String> {
        let _ = std::fs::remove_file(self.root.join("clip.txt"));
        write_then_rename(&self.root.join("clip-get"), b"")
    }

    /// The clipboard text asked for, once it has arrived (the file is removed as it's read).
    pub fn take_clipboard(&self) -> Option<String> {
        let path = self.root.join("clip.txt");
        let text = std::fs::read(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        Some(String::from_utf8_lossy(&text).into_owned())
    }

    /// Ask the Activity to look for an update now (Help › Check for Updates…).
    pub fn request_update_check(&self) -> Result<(), String> {
        write_then_rename(&self.root.join("check-update"), b"")
    }

    /// Ask the Activity to finish (see the module docs).
    pub fn request_quit(&self) -> Result<(), String> {
        write_then_rename(&self.root.join("quit"), b"")
    }

    /// Documents other apps sent since the last call, as (name, bytes); each is removed once read.
    pub fn take_incoming(&self) -> Vec<(String, Vec<u8>)> {
        let Ok(entries) = std::fs::read_dir(self.root.join("incoming")) else { return Vec::new() };
        let mut files: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_file() && !is_partial(p)).collect();
        files.sort();
        let mut out = Vec::new();
        for p in files {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            // The Activity prefixes a sequence number ("<n>-name") to keep names unique.
            let name = name.split_once('-').map_or(name.as_str(), |(_, rest)| rest).to_string();
            match std::fs::metadata(&p) {
                Ok(m) if m.len() <= MAX_INCOMING => match std::fs::read(&p) {
                    Ok(bytes) => out.push((name, bytes)),
                    Err(e) => log::error!("bridge: couldn't read sent document {}: {e}", p.display()),
                },
                Ok(_) => log::error!("bridge: sent document {} is too large", p.display()),
                Err(e) => log::error!("bridge: {}: {e}", p.display()),
            }
            let _ = std::fs::remove_file(&p);
        }
        out
    }
}

fn is_partial(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "tmp")
}

fn write_then_rename(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A file name safe to create (the last component of `suggested`, without separators or
/// control characters); `fallback` when nothing is left.
pub fn file_name(suggested: &str, fallback: &str) -> String {
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = last.chars().filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() { fallback.to_string() } else { clean }
}

/// The MIME type of a file name, for the save picker and for the providers that store it.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        Some("psd" | "psb") => "image/vnd.adobe.photoshop",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-bridge-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_suffix() -> u64 {
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn requests_and_answers_round_trip_through_files() {
        let dir = temp();
        let b = Bridge::new(&dir).unwrap();
        let id = b.next_id();
        b.request(&Request::Save { id, name: "a.png".into(), mime: "image/png".into() }).unwrap();
        let text = std::fs::read_to_string(b.root().join("request.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!((v["kind"].as_str(), v["id"].as_u64(), v["name"].as_str()), (Some("save"), Some(id), Some("a.png")));
        assert!(!b.root().join("request.tmp").exists(), "written under a temporary name, then renamed");

        assert_eq!(b.take_answer(id), None, "nothing yet");
        let answer = |json: &str| std::fs::write(b.root().join("answer.json"), json).unwrap();
        answer(&format!(r#"{{"id": {}, "path": "/x/out/1/a.png"}}"#, id + 100));
        assert_eq!(b.take_answer(id), None, "another request's answer is stale");
        assert!(!b.root().join("answer.json").exists(), "and dropped");
        answer(&format!(r#"{{"id": {id}, "path": "/x/out/1/a.png"}}"#));
        assert_eq!(b.take_answer(id).and_then(|a| a.path), Some("/x/out/1/a.png".into()));
        answer("{not json");
        assert_eq!(b.take_answer(id), None, "garbage is no answer, never a panic");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_new_bridge_drops_leftovers_of_an_earlier_run() {
        let dir = temp();
        let b = Bridge::new(&dir).unwrap();
        std::fs::write(b.root().join("request.json"), "{}").unwrap();
        std::fs::write(b.root().join("answer.json"), "{}").unwrap();
        let b = Bridge::new(&dir).unwrap();
        assert!(!b.root().join("request.json").exists() && !b.root().join("answer.json").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn quitting_leaves_a_flag_a_new_run_clears() {
        let dir = temp();
        let b = Bridge::new(&dir).unwrap();
        b.request_quit().unwrap();
        assert!(b.root().join("quit").is_file());
        let b = Bridge::new(&dir).unwrap();
        assert!(!b.root().join("quit").exists(), "a new run doesn't quit at once");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn writes_under_the_bridge_are_synced_back() {
        let dir = temp();
        let b = Bridge::new(&dir).unwrap();
        let mine = dir.join("prefs.json");
        b.mark_written(&mine).unwrap();
        assert_eq!(std::fs::read_dir(b.root().join("sync")).unwrap().count(), 0, "the app's own files stay private");
        let doc = b.root().join("out").join("7").join("a.png");
        b.mark_written(&doc).unwrap();
        let synced: Vec<_> = std::fs::read_dir(b.root().join("sync")).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(synced.len(), 1);
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&synced[0]).unwrap()).unwrap();
        assert_eq!(v["path"].as_str(), Some(doc.to_string_lossy().as_ref()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sent_documents_are_read_once_and_partial_copies_wait() {
        let dir = temp();
        let b = Bridge::new(&dir).unwrap();
        let inc = b.root().join("incoming");
        std::fs::write(inc.join("2-photo.jpg"), b"jpeg").unwrap();
        std::fs::write(inc.join("1-art-work.psd"), b"psd").unwrap();
        std::fs::write(inc.join("3-big.png.tmp"), b"partial").unwrap();
        let got = b.take_incoming();
        assert_eq!(got, vec![("art-work.psd".to_string(), b"psd".to_vec()), ("photo.jpg".to_string(), b"jpeg".to_vec())]);
        assert!(b.take_incoming().is_empty(), "consumed");
        assert!(inc.join("3-big.png.tmp").exists(), "a copy in progress is left alone");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(file_name("/storage/emulated/0/Pictures/Cat.psd", "x"), "Cat.psd");
        assert_eq!(file_name(r"C:\a\b.png", "x"), "b.png");
        assert_eq!(file_name("../../etc/passwd", "x"), "passwd");
        assert_eq!(file_name("..", "Untitled.png"), "Untitled.png");
        assert_eq!(file_name("a\u{0}b?:c.jpg", "x"), "abc.jpg");
        assert_eq!(file_name("", "Untitled.png"), "Untitled.png");
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for("a.PNG"), "image/png");
        assert_eq!(mime_for("a.b.jpeg"), "image/jpeg");
        assert_eq!(mime_for("doc.psd"), "image/vnd.adobe.photoshop");
        assert_eq!(mime_for("noext"), "application/octet-stream");
        assert_eq!(mime_for("x.pcraft"), "application/octet-stream");
    }
}
