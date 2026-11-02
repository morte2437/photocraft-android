//! Files named by paths stored in a document: linked smart objects, the Displace map and Color
//! Lookup file of a smart filter, data-set images, video footage.
//!
//! Such a path is followed only through [`resolve`]: a relative path is taken from the document's
//! folder, and only local files, or files on the network share the document itself is on, are
//! opened (see `craft_docpath`). A refused path is treated like a missing file.

use std::fmt;
use std::path::{Path, PathBuf};

pub use craft_docpath::Refused;

/// Why a stored path is not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotFollowed {
    /// The path is not a local file, or is not on the document's own share.
    Refused(Refused),
    /// The path is relative and the document has no file to take it from.
    NoDocumentFolder,
}

impl fmt::Display for NotFollowed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(r) => r.fmt(f),
            Self::NoDocumentFolder => f.write_str("the path is relative and the document has not been saved"),
        }
    }
}

impl std::error::Error for NotFollowed {}

/// The file to open for `stored`, a path or `file:` URL stored in a document, or why it is not
/// followed. `document` is the path of the document it is stored in, if the document has a file;
/// a relative `document` is taken from the working directory, where it was opened from.
pub fn resolve(stored: &str, document: Option<&Path>) -> Result<PathBuf, NotFollowed> {
    let document = document.map(|d| if d.is_relative() { std::path::absolute(d).unwrap_or_else(|_| d.to_path_buf()) } else { d.to_path_buf() });
    craft_docpath::resolve_absolute(stored, document.as_deref()).map_err(|r| match r {
        // Without a document, a relative path resolves but stays relative.
        Refused::Invalid if document.is_none() && craft_docpath::resolve(stored, None).is_ok_and(|p| cfg!(windows) || !is_windows_form(&p)) => {
            NotFollowed::NoDocumentFolder
        }
        r => NotFollowed::Refused(r),
    })
}

/// Whether `path`, as `craft_docpath` writes it, has a Windows drive or share root (`C:x`, `C:\x`,
/// `\\server\share\x`). Outside Windows no document folder makes such a path absolute.
fn is_windows_form(path: &Path) -> bool {
    let s = path.as_os_str().as_encoded_bytes();
    s.starts_with(b"\\") || matches!(s, [l, b':', ..] if l.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_taken_from_the_document_folder() {
        let doc = std::env::temp_dir().join("art").join("poster.pcraft");
        assert_eq!(resolve("logo.png", Some(&doc)), Ok(std::env::temp_dir().join("art").join("logo.png")));
        assert_eq!(resolve("logo.png", None), Err(NotFollowed::NoDocumentFolder));
        assert_eq!(resolve(r"\\192.0.2.1\share\x.png", Some(&doc)), Err(NotFollowed::Refused(Refused::Network)));
        assert_eq!(resolve("//192.0.2.1/share/x.png", None), Err(NotFollowed::Refused(Refused::Network)));
        assert_eq!(resolve("jar:logo.png", None), Err(NotFollowed::Refused(Refused::Url)));
        // Colons in names: an alternate data stream or a URL on Windows, a plain character elsewhere.
        if cfg!(windows) {
            assert_eq!(resolve("img/fig:1.png", Some(&doc)), Err(NotFollowed::Refused(Refused::Invalid)));
            assert_eq!(resolve("chapter1:intro.png", None), Err(NotFollowed::Refused(Refused::Url)));
        } else {
            assert_eq!(resolve("img/fig:1.png", Some(&doc)), Ok(std::env::temp_dir().join("art").join("img").join("fig:1.png")));
            assert_eq!(resolve("chapter1:intro.png", None), Err(NotFollowed::NoDocumentFolder));
        }
        // Outside Windows a Windows path names no file: it is refused, not reported as relative.
        if !cfg!(windows) {
            for stored in [r"C:\art\logo.png", "C:logo.png"] {
                assert_eq!(resolve(stored, Some(&doc)), Err(NotFollowed::Refused(Refused::Invalid)), "{stored}");
                assert_eq!(resolve(stored, None), Err(NotFollowed::Refused(Refused::Invalid)), "{stored}");
            }
            assert_eq!(resolve("../logo.png", None), Err(NotFollowed::NoDocumentFolder));
        }
    }
}
