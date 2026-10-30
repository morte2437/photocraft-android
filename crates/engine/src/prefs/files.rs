use super::cmds::bad;
use super::{Result, Value};

/// An output parameter (`exportKys`, `summarize`): absent or false → `None`; true (or an object
/// without `path`) → `Some(None)`, the text comes back in the reply; `{"path": …}` →
/// `Some(Some(path))`, written there.
pub(super) fn file_target(cmd: &str, v: Option<&Value>) -> Result<Option<Option<String>>> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(None)),
        Some(Value::Object(o)) => match o.get("path") {
            None | Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(Some(s.clone()))),
            Some(_) => Err(bad(cmd, "`path` must be a non-empty string")),
        },
        Some(_) => Err(bad(cmd, "expected true or {\"path\": …}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_text_file(path: &str, max: usize) -> std::result::Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > max as u64 {
        return Err(format!("not a file of at most {} KiB", max / 1024));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
pub(super) fn read_text_file(_: &str, _: usize) -> std::result::Result<String, String> {
    Err("paths can't be read on the web; pass the file's text instead".into())
}

/// Whether `path` can be written: not a folder, in a folder that exists.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn writable(path: &str) -> std::result::Result<(), String> {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return Err("is a folder".into());
    }
    if std::fs::metadata(p).is_ok_and(|m| m.permissions().readonly()) {
        return Err("is read-only".into());
    }
    match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(d) if !d.is_dir() => Err("its folder doesn't exist".into()),
        _ => Ok(()),
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn writable(_: &str) -> std::result::Result<(), String> {
    Err("paths can't be written on the web; omit `path` to get the text back".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rename_path(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn rename_path(_: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("paths can't be written on the web"))
}

/// Where `path` points, for telling whether two targets are one file: its folder resolved, its
/// name compared without case where the file system ignores it.
#[cfg(not(target_arch = "wasm32"))]
fn path_key(path: &str) -> String {
    let p = std::path::Path::new(path);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let key = dir.join(p.file_name().unwrap_or_default()).to_string_lossy().into_owned();
    if cfg!(any(target_os = "windows", target_os = "macos")) { key.to_lowercase() } else { key }
}

/// A new, empty file beside `path` with a name nothing else uses (`create_new`), for staging
/// (`kind` "tmp") or a backup ("bak").
#[cfg(not(target_arch = "wasm32"))]
fn sibling(path: &str, kind: &str) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    for n in 0..1000u32 {
        let p = std::path::PathBuf::from(format!("{path}.{}-{n}.photocraft-{kind}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free name beside it"))
}

/// Write `files` ((path, text)) all or nothing. Each text is staged in a new file beside its
/// target; once every one is written, each existing target is moved aside to a backup and the
/// staged file moved in. Any failure moves the backups back and removes what was staged, so the
/// user's files end as they began; on success the backups go. Two targets naming one file are
/// refused. `rename` is `std::fs::rename` (a test fails it on purpose).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_files_atomically(
    files: &[(&str, &str)],
    rename: &dyn Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::result::Result<(), String> {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    for (i, (a, _)) in files.iter().enumerate() {
        if files.iter().skip(i + 1).any(|(b, _)| path_key(a) == path_key(b)) {
            return Err(format!("{a}: two exports to the same file"));
        }
    }
    let mut staged: Vec<PathBuf> = Vec::new();
    let drop_all = |paths: &[PathBuf]| {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    };
    for (path, text) in files {
        let (tmp, mut f) = match sibling(path, "tmp") {
            Ok(v) => v,
            Err(e) => {
                drop_all(&staged);
                return Err(format!("{path}: {e}"));
            }
        };
        staged.push(tmp);
        if let Err(e) = f.write_all(text.as_bytes()).and_then(|()| f.sync_all()) {
            drop_all(&staged);
            return Err(format!("{path}: {e}"));
        }
    }
    // (target, its backup) of each file moved in so far.
    let mut moved: Vec<(&Path, Option<PathBuf>)> = Vec::new();
    // Put each backup back over its target (a target that had none is removed). What couldn't be
    // put back is named, so the user can find their previous file.
    let undo = |moved: &[(&Path, Option<PathBuf>)]| -> String {
        let mut stranded = Vec::new();
        for (target, backup) in moved.iter().rev() {
            match backup {
                Some(b) => {
                    if rename(b, target).is_err() {
                        stranded.push(format!("the previous {} is at {}", target.display(), b.display()));
                    }
                }
                None => {
                    let _ = std::fs::remove_file(target);
                }
            }
        }
        if stranded.is_empty() { String::new() } else { format!(" ({})", stranded.join("; ")) }
    };
    for ((path, _), tmp) in files.iter().zip(&staged) {
        let target = Path::new(*path);
        let backup = if target.exists() {
            let moved_aside = sibling(path, "bak").and_then(|(b, f)| {
                drop(f);
                match rename(target, &b) {
                    Ok(()) => Ok(b),
                    Err(e) => {
                        // The placeholder the backup name reserved goes too.
                        let _ = std::fs::remove_file(&b);
                        Err(e)
                    }
                }
            });
            match moved_aside {
                Ok(b) => Some(b),
                Err(e) => {
                    let stranded = undo(&moved);
                    drop_all(&staged);
                    return Err(format!("{path}: {e}{stranded}"));
                }
            }
        } else {
            None
        };
        if let Err(e) = rename(tmp, target) {
            let mut stranded = undo(&moved);
            if let Some(b) = &backup
                && rename(b, target).is_err()
            {
                stranded = format!("{stranded} (the previous {path} is at {})", b.display());
            }
            drop_all(&staged);
            return Err(format!("{path}: {e}{stranded}"));
        }
        moved.push((target, backup));
    }
    for (_, backup) in moved {
        if let Some(b) = backup {
            let _ = std::fs::remove_file(b);
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_files_atomically(
    files: &[(&str, &str)],
    _: &dyn Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::result::Result<(), String> {
    if files.is_empty() { Ok(()) } else { Err("paths can't be written on the web; omit `path` to get the text back".into()) }
}
