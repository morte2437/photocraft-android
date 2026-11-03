# Windows native-save durability

An acknowledged overwrite must reopen as the complete new document after a Windows/NTFS
power loss. [Issue #7858](https://github.com/storytold/photocraft/issues/7858) demonstrated
that flushing a temporary file before replacement did not establish this guarantee: a fresh
Windows boot recovered the previous ZIP or directory bundle after Save returned success.

## Publication barrier

The shared atomic writer still writes and flushes a sibling temporary file, preserves the
destination's metadata, and replaces the destination. It now flushes the parent directory
on Windows before acknowledging success. `RealFs::sync_dir` opens a directory using
`FILE_FLAG_BACKUP_SEMANTICS` and `FILE_ADD_FILE`, then calls `File::sync_all`, which reaches
`FlushFileBuffers`. This uses ordinary directory permissions and does not open or flush a
volume or require administrative privileges. No unsupported `REPLACEFILE_WRITE_THROUGH`
flag is used.

Windows references:

- [Directory handles](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)
- [File access rights](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants)
- [Flushing cached data and directory structure](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0de7dc40-9627-437e-a4df-c4696cdc3d02)
- [Filesystem-specific behavior](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63): the directory-structure guarantee applies to NTFS.
- [ReplaceFileW flags](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)

A publication-flush error occurs after replacement. It preserves the underlying error kind
and says that the new data was written but may not survive a crash. It does not claim the
old file is unchanged, attempt an unsafe rollback, or return success. Existing callers then
retain the document's dirty state and recovery data.

## Directory bundles and retries

Every published bundle file receives a Windows directory flush. Before replacing the
manifest, the writer also flushes `tiles/` and `blobs/`, including when it reuses cached
objects. A previous attempt can leave valid objects in place after a failed publication
flush; validating their bytes alone does not establish their durability. These barriers
make a retry using the same writer safe without recompressing every unchanged object.

The manifest's publication barrier completes before obsolete objects are collected. An
error stops the save before that cleanup, preserving the objects referenced by the old
manifest. Garbage collection remains best effort after a successful commit.

The private `save_dir_with` seam exercises this complete ordering with injected tile,
blob and manifest publication failures. The tests check the reported error, loaded
document, preservation of old objects, barriers on reused objects, and successful retry.

## Verification

Run the format tests and repository checks:

```sh
cargo test -p photocraft-format
cargo clippy -p photocraft-format --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo xtask test-corpus -p io
```

Unit tests cannot prove power-loss durability. The VM test uses a disposable NTFS disk and
synthetic old/new documents at 8-, 16- and 32-bit depth, in ZIP and directory form. Flush the
old baseline during setup, call the production save API, wait for its successful response,
then abruptly power off the guest. Preserve and hash the crashed disk before recovery.
Boot Windows fresh against those exact bytes and compare the complete loaded document
and every expected bundle entry with the new revision. Do not restore a RAM snapshot for
recovery. Repeat each combination and retain failing unchanged-build controls.

Keep volume flushes and clean unmounts in setup or diagnostic controls only; they are not
part of the production save implementation. Archive comparisons must account for the
build's generator-version stamp while still comparing all document payloads exactly.

This change addresses Windows/NTFS publication. The existing Unix flush-error policy is
handled separately by [#5420](https://github.com/storytold/photocraft/pull/5420). Preview
replacement/deletion ordering on failed directory saves remains tracked by
[#7548](https://github.com/storytold/photocraft/issues/7548) and
[#7549](https://github.com/storytold/photocraft/issues/7549). VM tests do not establish
physical-controller or firmware-cache behavior, or guarantees on other filesystems.
