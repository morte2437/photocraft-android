//! Paths stored in a document reach the file system only through [`resolve`] (built on
//! [`photocraft_io::doc_path::resolve`]): linked smart objects, the Displace map and Color Lookup
//! file a smart filter re-reads, data-set images and video footage. A refused path is handled like
//! a missing file: the layer keeps its last rendering, and commands that need the file say why.
//!
//! Smart-object rendering has no `Session` argument, so the path of the document being worked on
//! travels in a thread-local scope, entered where commands run ([`crate::Session::edit`], command
//! dispatch, background jobs). Outside a scope there is no document: relative paths are refused
//! and only local absolute paths are followed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub use photocraft_io::doc_path::NotFollowed;

thread_local! {
    static DOCUMENT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Restores the previous document path when dropped.
pub(crate) struct DocumentScope(Option<PathBuf>);

impl Drop for DocumentScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        DOCUMENT.with(|d| *d.borrow_mut() = previous);
    }
}

/// Resolve stored paths against `document` (the path of the document being worked on, `None`
/// for one without a file) until the returned scope is dropped.
pub(crate) fn scope(document: Option<&Path>) -> DocumentScope {
    let next = document.filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf);
    DocumentScope(DOCUMENT.with(|d| d.replace(next)))
}

/// The document path of the current scope (to carry it to another thread).
pub(crate) fn current() -> Option<PathBuf> {
    DOCUMENT.with(|d| d.borrow().clone())
}

/// The file to open for `stored`, a path stored in the document of the current scope.
pub fn resolve(stored: &str) -> Result<PathBuf, NotFollowed> {
    resolve_in(stored, current().as_deref())
}

/// The file to open for `stored`, a path stored in the document saved at `document`.
pub fn resolve_in(stored: &str, document: Option<&Path>) -> Result<PathBuf, NotFollowed> {
    photocraft_io::doc_path::resolve(stored, document)
}

/// The message for a stored path that is not followed.
pub(crate) fn refusal(what: &str, stored: &str, why: NotFollowed) -> crate::EngineError {
    crate::EngineError::Other(format!("{what} {stored} is not opened: {why}"))
}

#[cfg(test)]
mod tests;
