# Automation security model

Automation requests are untrusted input with potential effects beyond the active document. Opening, saving, rendering to a path, preferences changes, application control, pointer/type injection, and arbitrary engine commands have different risk levels but currently share broad transport access.

| Control | Current status |
|---|---|
| Loopback-only TCP binding | **Implemented** for desktop control and headless TCP serve |
| Stdio transport | **Implemented** for MCP and headless JSON lines |
| Command-level error handling | **Implemented**, with command-specific tests and `panic_hunt` coverage |
| Control-channel authentication | **Implemented:** 256-bit bearer token required before TCP method dispatch |
| MCP capability scopes | **Partial:** filesystem read/write capabilities only; general tool scopes are absent |
| Allowed read/write roots | **Implemented:** separate launch-time roots, with absent authority failing closed |
| Symlink-safe capability filesystem | **Implemented:** relative operations use held directory capabilities and reject link escapes |
| Windows UNC write roots | **Implemented:** handle-relative deletes; two ambient operations on the canonical root path (`create_dir`, `rename`), each below ancestors pinned by held handles; see below |
| Request-byte and JSON-depth limits | **Partial:** 1 MiB request-line limit on TCP and headless JSON-lines stdio (MCP stdio input is not byte-capped); no explicit JSON-depth policy |
| Batch-step limit | **Implemented:** 256 steps for headless and MCP batches |
| Reply-size limit | **Implemented:** 8 MiB encoded JSON/MCP result; aggregate batch reply budget |
| Headless preview budgets | **Implemented:** 2048-pixel requested edge, 67,108,864 source pixels, 5 MiB encoded PNG |
| Connection/worker limit | **Implemented:** 16 active TCP connections; one worker thread per accepted active connection |
| Security audit events | **Proposed** |

## Gateway status and remaining work

```text
MCP or control client
          |
 authenticated session
          v
 security gateway
   - general method capabilities (proposed)
   - path handles/read-write roots (implemented)
   - request budgets
   - command policy
   - audit events
          |
          v
 command engine / UI shell
```

The current token proves possession of a secret but does not provide general method authorization.
Filesystem handles and a defensive command-path policy are implemented; explicit non-filesystem
capabilities, session-memory/command-duration budgets, and audit events remain. Preview and
reply ceilings do not bound all document operations; desktop screenshots are admission-bounded
per image and in aggregate, though that admission is far above the reply ceiling. Private token
handling, process isolation, and least-privileged execution are still practical containment
measures.

## Windows UNC write roots

`cap-std` 4's Windows `rename`, `remove_file` and `create_dir` are not handle-relative: they
append the request to the handle's own path, read back without its `\\?\` prefix, which below a
UNC root (`\\wsl.localhost\...`, any network share) fails with `ERROR_PATH_NOT_FOUND`
(sunfishcode/cap-std#348). Below such a root, judged by the canonical root path's prefix and
never by a request, the automation workspace keeps every rule above and completes these steps
as follows:

- **Deletes** (a stranded temporary file, the frames of a cancelled export) open the file
  relative to the held directory handle with `FILE_FLAG_DELETE_ON_CLOSE` and
  `FILE_FLAG_OPEN_REPARSE_POINT`, so a link is deleted rather than its target. Nothing is deleted
  by path.
- **Directory creation** and the **rename of a finished temporary file into place** are the only
  two operations that touch the filesystem by path. Both run on the canonical verbatim root path
  (trusted launch configuration) only after every directory component on the way has been
  opened through the handle of its parent, verified a plain directory (not a link, not a file),
  and kept open. `cap-std` opens directories without `FILE_SHARE_DELETE`. This protects held
  ancestors from Windows-side rename or deletion only when the provider enforces share modes
  (`ERROR_SHARING_VIOLATION`). It does not protect against direct Linux-side changes in WSL. The
  UNC regression test uses an SMB administrative share, not `\\wsl.localhost`.
  The rename replaces the entry at the target without following it;
  a directory at the target is refused.

Saves below a UNC root are therefore as atomic as everywhere else (temporary file, flush,
rename). No file contents are written or read through an ambient path; directory creation and
rename use ambient paths. Regression tests
run against the loopback administrative share of the test drive and are skipped where none is
available.
