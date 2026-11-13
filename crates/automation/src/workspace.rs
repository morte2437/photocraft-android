//! Capability-based filesystem policy for untrusted automation paths.

use std::io::{Read, Write};
use std::path::{Component, Components, Path, PathBuf, Prefix};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use photocraft_format::atomic::RenameRetry;
use serde_json::Value;

use crate::AutomationError;

const DENIED: &str = "automation filesystem access is not granted";

#[derive(Clone)]
struct RootCapability {
    dir: Arc<Dir>,
    /// The canonical root path (trusted launch configuration), to recognise absolute request
    /// paths that name a file beneath it.
    path: PathBuf,
    /// The trusted launch spelling also names the same held capability, even through a link.
    configured_path: PathBuf,
}

/// Separate directory capabilities for automation reads and writes.
///
/// Root paths are trusted launch-time configuration. Request paths are always
/// untrusted, forward-slash relative paths resolved by `cap-std` beneath the
/// held directory handle, or absolute paths beneath the canonical or configured root spelling
/// (translated to relative ones, see [`request_path`]). Parent traversal,
/// alternate separators, Windows prefixes and empty components are rejected
/// before I/O.
#[derive(Clone, Default)]
pub struct AuthorizedWorkspace {
    read: Vec<RootCapability>,
    write: Option<RootCapability>,
}

impl AuthorizedWorkspace {
    /// Open the configured roots as capabilities. Either authority may be
    /// omitted; omitted authority fails closed.
    pub fn new(read_root: Option<&Path>, write_root: Option<&Path>) -> Result<Self, AutomationError> {
        Self::with_read_roots(read_root, write_root)
    }

    /// Open ordered read capabilities and an independent, optional write capability.
    /// Every configured root must be valid; a write root does not also grant reads.
    pub fn with_read_roots<'a>(read_roots: impl IntoIterator<Item = &'a Path>, write_root: Option<&Path>) -> Result<Self, AutomationError> {
        let mut read = Vec::new();
        for path in read_roots {
            let root = open_root(path, "read", &read)?;
            read.push(root);
        }
        let write = write_root.map(|path| open_root(path, "write", &read)).transpose()?;
        Ok(Self { read, write })
    }

    /// This workspace with `path` as its only read root (the write authority stays). A path
    /// spelled like the write root shares its capability, as [`Self::with_read_roots`] does, so
    /// an implicit save can still write where a file was read.
    pub fn with_read_root(&self, path: &Path) -> Result<Self, AutomationError> {
        let opened: Vec<RootCapability> = self.write.iter().cloned().collect();
        Ok(Self { read: vec![open_root(path, "read", &opened)?], write: self.write.clone() })
    }

    /// This workspace with `path` as its write root (the read authority stays), sharing the
    /// capability of a read root spelled the same.
    pub fn with_write_root(&self, path: &Path) -> Result<Self, AutomationError> {
        Ok(Self { read: self.read.clone(), write: Some(open_root(path, "write", &self.read)?) })
    }

    /// Read the first matching regular file, trying roots in their configured order.
    /// Only an open that reports NotFound tries the next root. Once opened, errors
    /// (including a non-regular file or a failed bounded read) never select another file.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, AutomationError> {
        self.read_with_save_path(path).map(|(bytes, _)| bytes)
    }

    /// Read bytes and derive a conservative implicit-save request from the same
    /// selected capability. A different read root must never redirect an implicit
    /// save to a same-named file in the write root. Aliases and nested roots need an
    /// explicit Save As; this is not canonical desktop document identity.
    pub fn read_with_save_path(&self, path: &str) -> Result<(Vec<u8>, Option<String>), AutomationError> {
        let (mut file, root) = self.open_read(path)?;
        let metadata = file.metadata().map_err(|e| file_error("read", path, e))?;
        if !metadata.is_file() {
            return Err(AutomationError::BadRequest(format!("automation read path is not a regular file: `{path}`")));
        }
        // Retain main's bounded, growth-checked reads while keeping save provenance.
        let bytes = read_to_observed_length(&mut file, metadata.len())?;
        let save_path = self.write.as_ref().filter(|write| Arc::ptr_eq(&root.dir, &write.dir)).map(|_| path.to_owned());
        Ok((bytes, save_path))
    }

    fn open_read(&self, path: &str) -> Result<(cap_std::fs::File, &RootCapability), AutomationError> {
        if self.read.is_empty() {
            return Err(AutomationError::BadRequest(format!("{DENIED}: read authority is absent")));
        }
        let mut not_found = None;
        for root in &self.read {
            let Some(relative) = scoped_path(path, root)? else { continue };
            match root.dir.open(&relative) {
                Ok(file) => return Ok((file, root)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => not_found = Some(error),
                Err(error) => return Err(file_error("read", path, error)),
            }
        }
        match not_found {
            Some(error) => Err(file_error("read", path, error)),
            None => Err(path_error(path, "absolute paths must be inside an automation read root")),
        }
    }

    /// Load one `.pcraft` document in directory form — the native on-disk layout — below the
    /// first matching read root. Directory opens never establish an implicit save target.
    /// `read` serves regular files only (the zip form), so a regular file at the
    /// path is reported as `Ok(None)` for the caller to fall back to.
    pub fn open_pcraft(&self, path: &str) -> Result<Option<photocraft_doc::Document>, AutomationError> {
        if self.read.is_empty() {
            return Err(AutomationError::BadRequest(format!("{DENIED}: read authority is absent")));
        }
        let mut not_found = None;
        for root in &self.read {
            let Some(relative) = scoped_path(path, root)? else { continue };
            let dir = match root.dir.open_dir(&relative) {
                Ok(dir) => dir,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    not_found = Some(error);
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => return Ok(None),
                Err(error) => return Err(file_error("read", path, error)),
            };
            let source = DirHandle { dir };
            return Ok(Some(photocraft_format::load_bytes_source(&source, &photocraft_format::LoadOptions::default())?));
        }
        match not_found {
            Some(error) => Err(file_error("read", path, error)),
            None => Err(path_error(path, "absolute paths must be inside an automation read root")),
        }
    }

    /// Create or replace one file below the configured write root, crash-safely: the bytes go to
    /// a temporary file beside the target, which is synced and renamed over it (the same steps
    /// as [`photocraft_format::atomic_write`], through the directory capability). On failure
    /// the previous file is untouched and the temporary file is removed.
    ///
    /// Replacing a file carries its extended attributes — macOS Finder tags, Linux `user.*` —
    /// onto the file that takes its place (#7095), through the two open descriptors rather than
    /// their paths. The rest of what [`photocraft_format::atomic_write`] preserves does not
    /// follow: its permission step is tracked in #4963, and the Windows `ReplaceFileW` swap is
    /// path-based, which this path may not use.
    ///
    /// Nothing here reaches outside the write root. Below a Windows UNC root
    /// (`\\wsl.localhost\...`, any network share) the capability's own rename and remove cannot
    /// reach their target at all (#10317, see [`is_unc_root`]), so there the temporary file is
    /// renamed into place on the canonical root path while every directory on the way is pinned
    /// by a handle opened through the root ([`rename_in`]): the same atomic replace, never
    /// outside the root, and never a stranded temporary file.
    ///
    /// The parent directory must already exist. `cap-std` performs path
    /// resolution and file creation relative to the held directory handle, so
    /// a non-existent final target is supported without ambient path access.
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        self.write_stream(path, &mut |w| w.write_all(bytes))
    }

    /// [`Self::write`] with the file's bytes produced by `write` straight into the temporary
    /// file (it may seek back to patch lengths), so a large PSD/PSB is never held in memory
    /// whole (#5333). The same refusals, the same atomic replace.
    pub fn write_stream(&self, path: &str, write: &mut photocraft_format::atomic::StreamWriter<'_>) -> Result<(), AutomationError> {
        let relative = request_path(path, self.write.as_ref())?;
        let root = self.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        let leaf = relative.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // The bytes are an image or document whatever the name says (`format` picks them), so
        // only a file of a kind PhotoCraft writes is replaced: never source, settings or notes
        // that share the root (`src/main.rs`, `.env`; #9439). A new file may have any name, and
        // an image saved under one (`out` with `format: png`) can be saved over again.
        let exists = root.dir.symlink_metadata(&relative).is_ok();
        // Only a regular file is ever opened below (its head here, its attributes on Unix): a
        // FIFO would block that open until a writer appears, and with it every other request.
        // Links are followed for this check; a dangling one falls through to the name rule.
        if exists && root.dir.metadata(&relative).is_ok_and(|m| !m.is_file()) {
            return Err(AutomationError::BadRequest(format!("automation write refused: `{path}` already exists and is not a regular file")));
        }
        if exists && !written_kind(&leaf) && !holds_image(&root.dir, &relative) {
            return Err(AutomationError::BadRequest(format!(
                "automation write refused: `{path}` already exists and is not an image or document file PhotoCraft writes; save to a new name"
            )));
        }
        let parent = relative.parent().map(Path::to_path_buf).unwrap_or_default();
        let tmp = parent.join(photocraft_format::atomic::temp_name(&leaf));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let opened = root.dir.open_with(&tmp, &options);
        // Removes the temporary file on every way out but a rename, a panicking writer included
        // (unwinding closes the handle inside the closure first), and only one this call made.
        let mut cleanup = TempRemoval { dir: &root.dir, path: &tmp, armed: opened.is_ok() };
        let written = opened.and_then(|file| {
            let mut file = file.into_std();
            write(&mut file)?;
            #[cfg(unix)]
            {
                use std::os::fd::AsFd;
                // Best effort, and only when there is a file being replaced: a save still
                // succeeds for a document whose Finder tag could not come along.
                if let Ok(old) = root.dir.open(&relative) {
                    let old = old.into_std();
                    let _ = photocraft_format::atomic::meta::copy_xattrs(old.as_fd(), file.as_fd());
                    // The creation time too (#9388), through the same handles.
                    let _ = photocraft_format::atomic::meta::copy_created_file(&old, &file);
                }
            }
            // Falls back to a plain fsync on shares that refuse a full flush (#1336).
            photocraft_format::atomic::sync_file(&file)
        });
        let renamed = written.and_then(|()| photocraft_format::atomic::retry_rename(RenameRetry::platform(), || rename_in(root, &tmp, &relative)));
        if let Err(e) = renamed {
            return Err(file_error("write", path, e));
        }
        cleanup.armed = false;
        // Flush the directory entry (Unix); best effort, the new bytes are already in place.
        #[cfg(unix)]
        {
            let dir = if parent.as_os_str().is_empty() { PathBuf::from(".") } else { parent };
            if let Ok(d) = root.dir.open(&dir) {
                let _ = photocraft_format::atomic::sync_file(&d.into_std());
            }
        }
        Ok(())
    }
}

/// Removes a temporary file on drop unless `armed` was cleared after its rename. The removal
/// goes through the held directory handle ([`remove_file_in`]), so it works below every root.
struct TempRemoval<'a> {
    dir: &'a Dir,
    path: &'a Path,
    armed: bool,
}

impl Drop for TempRemoval<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_file_in(self.dir, self.path);
        }
    }
}

/// Whether `name`'s extension is one of the formats PhotoCraft saves or exports.
fn written_kind(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    let ext = ext.to_ascii_lowercase();
    ["psd", "psb", "ora", "pdf", photocraft_format::EXTENSION].contains(&ext.as_str()) || photocraft_codecs::from_extension(&ext).is_some()
}

/// Whether the file at `relative` starts like an image PhotoCraft reads (PSD/PSB or a codec's
/// magic). Containers that also hold other things (zip, PDF) are judged by name only.
fn holds_image(dir: &Dir, relative: &Path) -> bool {
    let mut head = [0u8; 64];
    let Ok(mut file) = dir.open(relative) else { return false };
    let Ok(n) = file.read(&mut head) else { return false };
    let head = &head[..n];
    head.starts_with(b"8BPS") || photocraft_codecs::detect(head).is_some()
}

fn read_to_observed_length(reader: &mut impl Read, observed_length: u64) -> Result<Vec<u8>, AutomationError> {
    // Do not reserve from a potentially stale or attacker-controlled metadata length. Read in
    // chunks, and reject growth beyond the length observed on the opened file before appending.
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 << 10];
    loop {
        let remaining = observed_length.saturating_sub(bytes.len() as u64);
        let ask = chunk.len().min(usize::try_from(remaining.saturating_add(1)).unwrap_or(chunk.len()));
        let n = match reader.read(&mut chunk[..ask]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(AutomationError::Io(format!("automation read failed: {e}"))),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if n as u64 > remaining {
            return Err(AutomationError::BadRequest(format!("automation file grew beyond its observed length of {observed_length} bytes")));
        }
        let needed = bytes.len().checked_add(n).ok_or_else(|| AutomationError::BadRequest("automation file is too large to read".into()))?;
        if needed > bytes.capacity() {
            let target = bytes.capacity().saturating_add(bytes.capacity() / 2).max(needed).min(usize::try_from(observed_length).unwrap_or(usize::MAX));
            bytes.try_reserve_exact(target - bytes.len()).map_err(|e| AutomationError::Io(format!("automation read allocation failed: {e}")))?;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

fn open_root(path: &Path, authority: &str, opened: &[RootCapability]) -> Result<RootCapability, AutomationError> {
    if path.as_os_str().is_empty() {
        return Err(AutomationError::BadRequest(format!("automation {authority} root is empty")));
    }
    // Windows absolute() removes parent components. Keep an absolute launch spelling intact:
    // only that trusted prefix is exempt from request traversal checks, never its suffix.
    let configured_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path).map_err(|e| AutomationError::Io(format!("cannot resolve automation {authority} root `{}`: {e}", path.display())))?
    };
    // Reuse the held handle for the same launch spelling, not a second open of it: a root
    // symlink could otherwise change between the two opens. Only a shared handle lets an
    // implicit save write where the bytes were read (`read_with_save_path`); aliases do not.
    if let Some(root) = opened.iter().find(|root| root.configured_path == configured_path) {
        return Ok(root.clone());
    }
    // Resolve trusted configuration once, before opening that exact directory as a capability.
    let absolute =
        std::fs::canonicalize(path).map_err(|e| AutomationError::Io(format!("cannot resolve automation {authority} root `{}`: {e}", path.display())))?;
    let dir = Dir::open_ambient_dir(&absolute, ambient_authority())
        .map_err(|e| AutomationError::Io(format!("cannot open automation {authority} root `{}`: {e}", path.display())))?;
    Ok(RootCapability { dir: Arc::new(dir), path: absolute, configured_path })
}

/// `cap-std`'s Windows `rename`, `remove_file` and `create_dir` do not run relative to the held
/// handle: they append the request to the handle's own path and call `std::fs`
/// (sunfishcode/cap-std#348), and that path is read back without its `\\?\` prefix. Below a
/// drive root the result still resolves; below a UNC root (`\\?\UNC\wsl.localhost\...`, any
/// network share) it becomes the relative `UNC\wsl.localhost\...` and each of those steps
/// fails with `ERROR_PATH_NOT_FOUND`, so a save left only its temporary file behind (#10317).
/// Opening a file or directory and `stat` are handle-relative and work below any root, and
/// everything here is built on them: files are removed through handles ([`remove_file_in`]),
/// and the two steps that have no handle-relative form, making a directory and renaming the
/// finished temporary file into place, run on the canonical root path below directories pinned
/// by held handles ([`create_dir_all_unc`], [`rename_unc`]).
///
/// Whether the root is such a UNC root, judged by the canonical root path's prefix: trusted
/// launch configuration, never the request.
#[cfg(windows)]
fn is_unc_root(root: &RootCapability) -> bool {
    matches!(root.path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimUNC(..) | Prefix::UNC(..)))
}

/// Rename `from` to `to`, both relative to the write root, replacing whatever file is at `to`.
/// Through the capability wherever that works; below a Windows UNC root its `rename` reports
/// both names missing (see [`is_unc_root`]), so there the rename is done by [`rename_unc`].
fn rename_in(root: &RootCapability, from: &Path, to: &Path) -> std::io::Result<()> {
    let renamed = root.dir.rename(from, &root.dir, to);
    #[cfg(windows)]
    if renamed.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) && is_unc_root(root) {
        return rename_unc(root, from, to);
    }
    renamed
}

/// Rename `from` to `to` below a UNC root with `std::fs::rename` on the canonical root path:
/// the second of the two ambient steps, the other being [`create_dir_all_unc`]'s `create_dir`,
/// and guarded the same way. The directory of each name is first opened component by component
/// through the handle of its parent ([`pinned_directory_chain`]), every component verified a
/// plain directory and then held open for the duration, so the path the rename resolves names
/// exactly the directories the capability verified: a link anywhere on the way is refused, an
/// absent directory is not created, and a held one cannot be renamed away or replaced while the
/// rename runs. The rename itself replaces the entry at `to` without following it: a link there
/// is replaced, never written through, and a directory there is refused.
#[cfg(windows)]
fn rename_unc(root: &RootCapability, from: &Path, to: &Path) -> std::io::Result<()> {
    let parent = |name: &Path| name.parent().map(Path::to_path_buf).unwrap_or_default();
    let _from_chain = pinned_directory_chain(root, &parent(from))?;
    let _to_chain = pinned_directory_chain(root, &parent(to))?;
    std::fs::rename(below_root(root, from), below_root(root, to))
}

/// `relative` below the canonical root path, one component at a time, so the result keeps the
/// root's verbatim form (`\\?\UNC\...`): a `/` in a request would not separate there.
#[cfg(windows)]
fn below_root(root: &RootCapability, relative: &Path) -> PathBuf {
    let mut path = root.path.clone();
    path.extend(relative.components());
    path
}

/// Remove the file `name` relative to `dir`. On Windows the capability's own `remove_file`
/// reports every file below a UNC root missing (see [`is_unc_root`]); then the file is deleted
/// through a handle opened relative to `dir` instead ([`delete_on_close`]).
fn remove_file_in(dir: &Dir, name: &Path) -> std::io::Result<()> {
    let removed = dir.remove_file(name);
    #[cfg(windows)]
    if removed.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return delete_on_close(dir, name);
    }
    removed
}

/// Delete `name` by opening it relative to `dir` with `FILE_FLAG_DELETE_ON_CLOSE` and dropping
/// the handle. The open resolves against the held directory like every other open, so it
/// cannot reach outside it. `FILE_FLAG_OPEN_REPARSE_POINT` makes a link at `name` the thing
/// deleted rather than its target, as `remove_file` does; a directory is refused and a missing
/// name reports `NotFound`, both as `remove_file` would.
#[cfg(windows)]
fn delete_on_close(dir: &Dir, name: &Path) -> std::io::Result<()> {
    use cap_std::fs::OpenOptionsExt;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(FILE_FLAG_DELETE_ON_CLOSE | FILE_FLAG_OPEN_REPARSE_POINT);
    dir.open_with(name, &options).map(drop)
}

/// Make `relative` a directory below the write root, with its missing ancestors. Through the
/// capability wherever that works; below a Windows UNC root its `create_dir_all` reports
/// `NotFound` whether or not the directory exists (see [`is_unc_root`]), so there an existing
/// directory is accepted by opening it through the handle and a missing one is made by
/// [`create_dir_all_unc`].
fn ensure_export_dir(root: &RootCapability, relative: &Path) -> std::io::Result<()> {
    let created = root.dir.create_dir_all(relative);
    #[cfg(windows)]
    if created.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) && is_unc_root(root) {
        return match root.dir.open_dir(relative) {
            Ok(_) => Ok(()),
            Err(_) => create_dir_all_unc(root, relative),
        };
    }
    created
}

/// Directory creation is one of the two steps `cap-std` has no handle-relative form of on
/// Windows (it never opens with `FILE_DIRECTORY_FILE`; the other is the rename of
/// [`rename_unc`]), so below a UNC root an absent component is made with `std::fs::create_dir`
/// on the canonical root path — trusted launch configuration, never the request. The walk
/// ([`held_directory_chain`]) descends one component at a time and keeps a handle open on every
/// ancestor it has verified through its parent's handle. `cap-std` opens directories without
/// `FILE_SHARE_DELETE` precisely so that they cannot be renamed or deleted underneath its
/// sandboxed path lookups, and the held chain pins the ancestors here the same way: an ancestor
/// cannot be swapped for a link while the walk is below it, so the one ambient `create_dir` can
/// only add a new, empty directory inside the directory its held parent handle names. Something
/// that appears under the component's name between the check and `create_dir` makes
/// `create_dir` fail with `AlreadyExists`, which is reported, never resolved. No file is ever
/// written or removed ambiently, and the one ambient rename is guarded by the same held chain.
#[cfg(windows)]
fn create_dir_all_unc(root: &RootCapability, relative: &Path) -> std::io::Result<()> {
    held_directory_chain(root, relative).map(drop)
}

/// Open each component of `relative` through the handle of its parent, creating an absent one
/// first as described at [`create_dir_all_unc`]. The returned handles, outermost first, keep
/// every directory on the way pinned for as long as they are held.
#[cfg(windows)]
fn held_directory_chain(root: &RootCapability, relative: &Path) -> std::io::Result<Vec<Dir>> {
    directory_chain(root, relative, open_or_create_plain_dir)
}

/// [`held_directory_chain`] for directories that must already exist: an absent component is
/// `NotFound`, and nothing is created. This is what pins the way for [`rename_unc`].
#[cfg(windows)]
fn pinned_directory_chain(root: &RootCapability, relative: &Path) -> std::io::Result<Vec<Dir>> {
    directory_chain(root, relative, |parent, name, ancestor, _| open_plain_dir(parent, name, ancestor))
}

/// The walk shared by [`held_directory_chain`] and [`pinned_directory_chain`]: `open` receives
/// the parent's handle, the component's name, its path relative to the root and its path below
/// the canonical root, in that order.
#[cfg(windows)]
fn directory_chain(root: &RootCapability, relative: &Path, open: fn(&Dir, &Path, &Path, &Path) -> std::io::Result<Dir>) -> std::io::Result<Vec<Dir>> {
    let mut chain: Vec<Dir> = Vec::new();
    let mut ancestor = PathBuf::new();
    for component in relative.components() {
        ancestor.push(component);
        let parent = chain.last().unwrap_or(&root.dir);
        let dir = open(parent, Path::new(component.as_os_str()), &ancestor, &root.path.join(&ancestor))?;
        chain.push(dir);
    }
    Ok(chain)
}

/// Open `name`, one component of `parent`, when it is a plain directory; when it is absent,
/// create it at `ambient` (its path below the canonical root, which `parent` is a handle to)
/// and open what is there now. `ancestor` names the component relative to the root in errors.
#[cfg(windows)]
fn open_or_create_plain_dir(parent: &Dir, name: &Path, ancestor: &Path, ambient: &Path) -> std::io::Result<Dir> {
    match open_plain_dir(parent, name, ancestor) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(ambient)?;
            open_plain_dir(parent, name, ancestor)
        }
        opened => opened,
    }
}

/// Open `name` in `parent` only if it is a plain directory — not a link, which ambient
/// resolution would follow out of the root, and not a file.
#[cfg(windows)]
fn open_plain_dir(parent: &Dir, name: &Path, ancestor: &Path) -> std::io::Result<Dir> {
    let meta = parent.symlink_metadata(name)?;
    if meta.is_dir() && !meta.is_symlink() {
        parent.open_dir(name)
    } else {
        let message = format!("`{}` is not a plain directory below the root", ancestor.display());
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, message))
    }
}

/// Both root spellings refer to the capability opened at startup. Do not resolve an untrusted
/// absolute request using ambient filesystem access: strip a known root first and let cap-std
/// resolve the remaining name. New targets and nested export directories need not exist yet.
fn request_path(raw: &str, root: Option<&RootCapability>) -> Result<PathBuf, AutomationError> {
    let Some(root) = root else { return relative_path(raw) };
    scoped_path(raw, root)?.ok_or_else(|| path_error(raw, "absolute paths must be inside the automation root"))
}

/// An absolute prefix mismatch is a noncandidate, whereas a malformed path
/// beneath a matching root is an error. Never turn either into a relative lookup.
fn scoped_path(raw: &str, root: &RootCapability) -> Result<Option<PathBuf>, AutomationError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return relative_path(raw).map(Some);
    }
    // Prefer the trusted launch spelling: it can itself contain `..`. Only that
    // exact granted prefix is exempt; traversal in the remaining request still fails below.
    let Some(request) = path_beneath_root(path, &root.configured_path).or_else(|| path_beneath_root(path, &root.path)) else {
        return Ok(None);
    };
    let rest = request.map(|c| c.as_os_str().to_str().ok_or_else(|| path_error(raw, "path is not UTF-8"))).collect::<Result<Vec<_>, _>>()?;
    relative_path(&rest.join("/")).map(Some)
}

/// Strip a granted root prefix without resolving the client path with ambient authority.
/// Windows ASCII case variants name the same grant; the unstripped suffix keeps its case.
fn path_beneath_root<'a>(path: &'a Path, root: &Path) -> Option<Components<'a>> {
    let mut request = path.components();
    for want in root.components() {
        let same = match (request.next()?, want) {
            (Component::Prefix(a), Component::Prefix(b)) => match (a.kind(), b.kind()) {
                // std::fs::canonicalize uses verbatim paths on Windows. Accept their ordinary
                // drive/UNC spelling too, without treating a device prefix as a disk prefix.
                (Prefix::Disk(a) | Prefix::VerbatimDisk(a), Prefix::Disk(b) | Prefix::VerbatimDisk(b)) => a.eq_ignore_ascii_case(&b),
                (Prefix::UNC(a, b) | Prefix::VerbatimUNC(a, b), Prefix::UNC(c, d) | Prefix::VerbatimUNC(c, d)) => {
                    a.eq_ignore_ascii_case(c) && b.eq_ignore_ascii_case(d)
                }
                _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
            },
            #[cfg(windows)]
            (Component::Normal(a), Component::Normal(b)) => a.eq_ignore_ascii_case(b),
            (got, want) => got == want,
        };
        if !same {
            return None;
        }
    }
    Some(request)
}

fn relative_path(raw: &str) -> Result<PathBuf, AutomationError> {
    if raw.is_empty() {
        return Err(path_error(raw, "path is empty"));
    }
    if raw.contains('\\') {
        return Err(path_error(raw, "alternate separators are not allowed; use `/`"));
    }
    if raw.starts_with('/') {
        return Err(path_error(raw, "absolute paths are not allowed"));
    }
    if raw.contains(':') {
        return Err(path_error(raw, "drive, device and stream prefixes are not allowed"));
    }
    for component in raw.split('/') {
        if component.is_empty() {
            return Err(path_error(raw, "empty path components are not allowed"));
        }
        if component == "." {
            return Err(path_error(raw, "`.` path components are not allowed"));
        }
        if component == ".." {
            return Err(path_error(raw, "parent traversal is not allowed"));
        }
        if component.ends_with(['.', ' ']) {
            return Err(path_error(raw, "path components ending in a dot or space are not allowed"));
        }
        if is_windows_device_name(component) {
            return Err(path_error(raw, "reserved device names are not allowed"));
        }
    }
    Ok(PathBuf::from(raw))
}

/// At least every name cap-std refuses to open on Windows (`COM0`–`COM9`, `COM¹`–`COM³`, the same
/// for `LPT`, spaces before the extension ignored), so nothing written is unreadable (#9424).
fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end().to_ascii_uppercase();
    let port = |n: &str| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³");
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
        || stem.strip_prefix("COM").is_some_and(port)
        || stem.strip_prefix("LPT").is_some_and(port)
}

fn path_error(path: &str, reason: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation path rejected: {reason}: `{path}`"))
}

/// A `.pcraft` directory behind the read-root capability: bounded reads, no ambient paths.
struct DirHandle {
    dir: Dir,
}

impl photocraft_format::ByteSource for DirHandle {
    fn get(&self, path: &str, max: usize) -> photocraft_format::Result<Vec<u8>> {
        let missing = || photocraft_format::FormatError::Corrupt(format!("missing `{path}`"));
        let mut file = self.dir.open(path).map_err(|_| missing())?;
        let metadata = file.metadata().map_err(|_| missing())?;
        if !metadata.is_file() {
            // A FIFO or device would block the open and stream without end.
            return Err(photocraft_format::FormatError::Corrupt(format!("`{path}` is not a regular file")));
        }
        photocraft_format::read_entry(&mut file, metadata.len(), max, path)
    }
}

fn file_error(operation: &str, path: &str, error: std::io::Error) -> AutomationError {
    let kind = error.kind();
    AutomationError::Io(format!("automation {operation} `{path}` failed ({kind:?}): {error}"))
}

/// Filesystem-bearing engine commands have not yet been converted to consume
/// directory capabilities. Automation must use `doc.open`, `doc.save` and
/// `doc.render` for file effects until those commands are migrated.
pub fn authorize_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    let safe_file_command = matches!(
        id,
        "file.new"
            | "file.newFromClipboard"
            | "file.close"
            | "file.closeAll"
            | "file.closeOthers"
            | "file.fileInfo"
            | "file.automate.fitImage"
            | "file.automate.conditionalModeChange"
            | "file.automate.cropAndStraightenPhotos"
            | "file.scripts.flattenAllLayerEffects"
            | "file.scripts.flattenAllMasks"
            | "file.scripts.deleteAllEmptyLayers"
            | "file.export.exportPreferences"
    );
    if id.starts_with("file.") && !safe_file_command {
        return Err(command_error(id));
    }
    // Contents commands re-check the resolved source in the engine before any disk read.
    // Photoshop's embedded linked-layer blocks count as in-memory sources too.
    let in_memory_smart_command = matches!(
        id,
        "layer.smartObjects.convertToSmartObject"
            | "layer.smartObjects.editContents"
            | "layer.smartObjects.convertToLayers"
            | "layer.smartObjects.saveContents"
            | "layer.smartObjects.exportSvg"
            | "layer.smartObjects.updateSvg"
    );
    if (id.starts_with("layer.smartObjects.") && !in_memory_smart_command)
        || matches!(
            id,
            "pattern.import"
                | "pattern.export"
                | "edit.presets.migratePresets"
                | "measurementLog.export"
                | "layer.videoLayers.newVideoLayerFromFile"
                | "layer.videoLayers.replaceFootage"
                | "layer.videoLayers.reloadFrame"
        )
        || command_uses_ambient_path(id, params)
        || profile_command_may_read_ambient(id, params)
        || preferences_may_grant_ambient_paths(id, params)
        || params_contain_ambient_path(id, params)
    {
        return Err(command_error(id));
    }
    Ok(())
}

/// Desktop sessions may already contain user-configured ambient colour-profile
/// paths. Commands which implicitly resolve those settings are denied even
/// when their request parameters contain no path. Fresh headless sessions
/// cannot acquire such settings through the automation interface.
pub fn authorize_desktop_engine_command(id: &str, params: &Value) -> Result<(), AutomationError> {
    authorize_engine_command(id, params)?;
    if id.starts_with("image.mode.") {
        return Err(command_error(id));
    }
    Ok(())
}

/// [`authorize_engine_command`] as the function pointer [`photocraft_engine::Session::authorize`]
/// stores. `actions.play` calls it for every nested step.
pub fn authorize_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

/// [`authorize_desktop_engine_command`] as a [`photocraft_engine::Session::authorize`] hook.
pub fn authorize_desktop_engine_step(id: &str, params: &Value) -> photocraft_engine::Result<()> {
    authorize_desktop_engine_command(id, params).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
}

fn params_contain_ambient_path(id: &str, params: &Value) -> bool {
    let keys: &[&str] = match id {
        "image.adjustments.colorLookup" | "layer.newAdjustmentLayer.colorLookup" | "layer.setAdjustment" => &["file"],
        "filter.distort.displace" => &["mapPath"],
        "layer.quickExportAsPng"
        | "layer.exportAs"
        | "image.applyDataSet"
        | "layer.smartObjects.editContents"
        | "layer.smartObjects.convertToLayers"
        | "layer.smartObjects.exportSvg"
        | "layer.smartObjects.updateSvg" => &["path"],
        "image.mode.rgb" | "image.mode.grayscale" | "image.mode.cmyk" | "image.mode.lab" => &["profile"],
        "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo" | "view.proofSetup" | "view.gamutWarning" => &["profile"],
        // New Document's Color Profile (#321) can name an .icc file.
        "file.new" => &["profile"],
        "edit.colorSettings" => &["workingRgb", "workingCmyk", "workingGray"],
        _ => &[],
    };
    keys.iter().any(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| if matches!(*key, "file" | "mapPath" | "path") { !value.is_empty() } else { looks_like_path(value) })
    })
}

fn profile_command_may_read_ambient(id: &str, params: &Value) -> bool {
    // Installed profiles scan OS and user directories even without a request path.
    if matches!(id, "color.profileMismatch" | "edit.colorSettings" | "edit.installedProfiles" | "view.proofSetup") {
        return true;
    }
    // New Document skips exact `working` and `none`; the other working aliases resolve
    // the configured working space, which may be an .icc path.
    if id == "file.new" {
        return params.get("profile").and_then(Value::as_str).is_some_and(|profile| {
            matches!(profile, "default" | "working-cmyk" | "workingCmyk" | "working-rgb" | "workingRgb" | "working-gray" | "workingGray")
        });
    }
    matches!(id, "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo")
        && params.get("profile").and_then(Value::as_str).is_some_and(|profile| {
            matches!(profile, "working" | "default" | "working-cmyk" | "workingCmyk" | "working-rgb" | "workingRgb" | "working-gray" | "workingGray")
        })
}

fn preferences_may_grant_ambient_paths(id: &str, params: &Value) -> bool {
    if id != "prefs.set" {
        return false;
    }
    let direct = params.get("path").and_then(Value::as_str).is_some_and(preference_uses_ambient_filesystem);
    let batch = params.get("values").and_then(Value::as_object).is_some_and(|values| values.keys().any(|path| preference_uses_ambient_filesystem(path)));
    direct || batch
}

fn preference_uses_ambient_filesystem(path: &str) -> bool {
    // The engine skips empty segments (`.scriptEvents.enabled` sets `scriptEvents.enabled`), so
    // judge the first non-empty one; no segment at all is the whole-preferences update.
    let mut segments = path.split('.').filter(|segment| !segment.is_empty());
    let Some(section) = segments.next() else { return true };
    // File Handling's LUT folder (#6558) is listed from disk; the rest of that section is not.
    // Integrations' automation roots are what this policy grants (applied at once): an agent must
    // not widen its own file access.
    let key = segments.next();
    matches!(section, "colorSettings" | "scriptEvents" | "historyLog" | "plugIns" | "scratchDisks")
        || (section == "fileHandling" && key.is_none_or(|key| key == "lutFolder"))
        || (section == "integrations" && key.is_none_or(|key| matches!(key, "readRoot" | "writeRoot")))
}

fn command_uses_ambient_path(id: &str, params: &Value) -> bool {
    match id {
        "actions.load"
        | "brush.presets.importAbr"
        | "brush.presets.importImage"
        | "gradient.presets.importGrd"
        | "shape.presets.importCsh"
        | "plugin.install"
        | "swatches.import"
        | "style.presets.importAsl"
        | "layer.layerStyle.contours.load" => {
            // `data` wins over `path` in these commands; any `path` without it reads the filesystem.
            params.get("data").is_none() && params.get("path").is_some()
        }
        // With a `path` the file is written there; without one the bytes come back as `data`.
        "actions.save" | "swatches.export" | "style.presets.exportAsl" | "layer.layerStyle.contours.save" => params.get("path").is_some(),
        // Shortcut sets: a .kys read or written by path, or a Summarize page written by path.
        "edit.keyboardShortcuts" => {
            params.get("path").is_some() || ["importKys", "exportKys", "summarize"].iter().any(|k| params.get(*k).and_then(|v| v.get("path")).is_some())
        }
        "plugin.reload" => true,
        _ => false,
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.to_ascii_lowercase().ends_with(".icc")
        || value.to_ascii_lowercase().ends_with(".icm")
}

fn command_error(id: &str) -> AutomationError {
    AutomationError::BadRequest(format!("automation command `{id}` uses ambient filesystem paths and is disabled; use capability-scoped document methods"))
}

/// An export owns only files it created through this held directory capability. Refuse
/// collisions (including symlinks) so cancellation cannot delete or replace an earlier export.
pub(crate) struct ExportSequence {
    dir: Dir,
    created: Vec<String>,
}

impl AuthorizedWorkspace {
    /// Open the export directory `path` below the write root as its own capability, creating it
    /// (and missing ancestors) first; see [`ensure_export_dir`]. Every file the sequence then
    /// writes or removes goes through that held directory handle.
    pub(crate) fn export_sequence(&self, path: &str) -> Result<ExportSequence, AutomationError> {
        let relative = request_path(path, self.write.as_ref())?;
        let root = self.write.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: write authority is absent")))?;
        ensure_export_dir(root, &relative).map_err(|e| file_error("mkdir", path, e))?;
        let dir = root.dir.open_dir(&relative).map_err(|e| file_error("open export directory", path, e))?;
        Ok(ExportSequence { dir, created: Vec::new() })
    }
}

impl ExportSequence {
    /// Create `name` (a single path component) in the export directory and write `bytes` to
    /// it. An existing entry of that name, link or file, is a collision and is refused.
    pub(crate) fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), AutomationError> {
        let relative = relative_path(name)?;
        if relative.components().count() != 1 {
            return Err(path_error(name, "export filename must be a single component"));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.dir.open_with(&relative, &options).map_err(|e| file_error("create export", name, e))?;
        // Record immediately after creation, before any fallible write, including partial writes.
        self.created.push(name.to_owned());
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| file_error("write export", name, e))
    }

    /// Keep every file written so far: nothing is removed on cleanup or drop.
    pub(crate) fn commit(&mut self) {
        self.created.clear();
    }

    /// Remove every file written since the last [`commit`](Self::commit), each through the held
    /// directory handle (see [`remove_file_in`]). A file already gone is fine; any other failure
    /// is reported, and that name stays listed for a later attempt.
    pub(crate) fn cleanup(&mut self) -> Result<(), AutomationError> {
        let mut failed = Vec::new();
        let (dir, created) = (&self.dir, &mut self.created);
        created.retain(|name| {
            let removed = remove_file_in(dir, Path::new(name));
            match removed {
                Ok(()) => false,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => {
                    failed.push(format!("{name}: {e}"));
                    true
                }
            }
        });
        if failed.is_empty() { Ok(()) } else { Err(AutomationError::Io(format!("export cleanup failed: {}", failed.join("; ")))) }
    }
}

impl Drop for ExportSequence {
    /// Remove whatever was not committed, best effort.
    fn drop(&mut self) {
        // Also cover an escaped rendering panic; normal errors report cleanup failures above.
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests;
