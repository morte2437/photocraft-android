# MCP conventions

## Connecting the desktop app (no terminal)

**One switch, in the ArtCraft Launcher.** Settings › AI agents › **Let AI agents use Craft apps**
is the person's consent for every Craft app, PhotoCraft included; PhotoCraft has no switch of its
own. **Add** next to an MCP client (Claude Desktop, Claude Code, Cursor, LM Studio) puts one entry
in it, `ArtCraft`, for every installed Craft app (the `craft-ai-mcp` gateway, which starts
`photocraft-cli mcp`); **Copy** gives the configuration for any other client. Then restart the
client. Adding to MCP clients is the launcher's job alone: PhotoCraft writes no client
configuration. The design is in storytold/craft-ai-integrations#195.

This is behind the Craft suite's `AI-MCP` feature flag (`craft-flags`). Flags are set only in the
per-user `ArtCraft/flags.toml` (`AI-MCP = true` under `[flags]`; on Windows in
`%LOCALAPPDATA%\ArtCraft`), which every Craft app and the launcher read, or for one run with
`CRAFT_FLAGS=AI-MCP`; no app or launcher setting switches them. Without the flag, everything is as
it was: Preferences › AI Integrations shows Allow Agent Control and Control Port, a Control Port
already set keeps its connection, and the launcher's switch means nothing to PhotoCraft.

While the launcher's switch is on, its app list `ArtCraft/mcp/apps.json` (in the per-user local
configuration folder; `CRAFT_MCP_DIR` moves it) names `photocraft`, read through
`craft-ai-agents`, the same crate the launcher writes it with and the gateway reads it with.
PhotoCraft then serves agents on loopback (any free port by default, or the Control Port) with a
token it keeps in its settings folder, and publishes both in `agent-connection.json` there. The
app looks at the launcher's files every 2 s, so switching there applies without a restart. Allow
Agent Control off still refuses agents. Started with `--control`, the app ignores all this and the
launch flags manage the connection, as before.

**Edit › Preferences › AI Integrations** says whether agents can connect (and on which port), and
keeps what is PhotoCraft's own: the port, the folders agents can open from and save to, and what
happens when an agent needs PhotoCraft while it is closed (open it, work without a window, or ask
the user to open it).

### `photocraft-cli mcp`

Plain `photocraft-cli mcp` works in the open PhotoCraft window when the person lets agents use it,
and otherwise without a window, as it always did. It reads `agent-connection.json` before every
tool call, so the client configuration holds no port or token and keeps working when PhotoCraft
restarts or moves to another port.

- **Agents allowed, PhotoCraft open:** tools work in the window.
- **Agents allowed, PhotoCraft closed:** When PhotoCraft Is Closed decides (`--when-closed
  <launch|headless|error>` overrides it): start the app and wait for it, serve a headless
  session, or say to open it. The headless session takes `--automation-read-root` and
  `--automation-write-root` when given to `mcp`, each on its own: a given read root replaces the
  app's read folder, a given write root its write folder, and the app's folder is used for any
  direction without a flag. The app is started only on a tool
  call, never when the MCP client starts.
- **Agents not allowed:** a headless session with only the roots given to `mcp`, exactly as
  before; the app is never started and its folders aren't used.
- **`--headless`:** always headless, whatever the switch says.
- **`--bridge` (or `PHOTOCRAFT_CONTROL_PORT`):** a `photocraft --control` app, as before.

A session that went headless stays headless until the MCP client restarts it, so documents don't
move under the agent. In the Flatpak the connection needs
`flatpak override --user --share=network ai.storyteller.photocraft`.

When `mcp` starts the app, macOS goes through LaunchServices (`open -a`) and Windows through
Explorer, as a double-click would: the app is no child of the CLI, so it keeps running when the
MCP client ends the CLI's process tree (Claude Code does on Windows), and it stays out of a
Microsoft Store client's container, where new files under `%APPDATA%` would land in the client's
package instead of PhotoCraft's settings folder. Linux starts it in its own process group.

## Install and register

From the repository root, with the [source-build prerequisites](development.md#prerequisites)
installed, install the CLI independently of the build directory:

```sh
cargo install --path apps/photocraft-cli --locked
# POSIX shells, for Cargo's default install location:
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
photocraft-cli --help
photocraft-cli mcp --help
```

Cargo installs a binary, not a symlink into `target/`, so it survives `cargo clean`.
Re-run the install command after updating the checkout.
By default the executable goes in `$CARGO_HOME/bin` (`~/.cargo/bin` when unset), or
`%USERPROFILE%\.cargo\bin` on Windows. If you set `--root`, `CARGO_INSTALL_ROOT`, or
Cargo's `install.root`, use that installation root's `bin` directory instead. Add it
to the `PATH` inherited by the MCP client, not just an interactive shell, and restart
the client after changing its environment.

Register the installed command explicitly (replace `/work/project` with an existing folder):

```sh
claude mcp add photocraft -- photocraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project
```

Other MCP clients can use this bare-name configuration:

```json
{"mcpServers":{"photocraft":{"command":"photocraft-cli","args":["mcp","--automation-read-root","/work/project","--automation-write-root","/work/project"]}}}
```

Linux `.deb` and `.rpm` packages already install `/usr/bin/photocraft-cli`; those users
can skip the source install. Other release locations are listed in the
[installed-release guide](../book/src/automation/mcp.md#from-an-installed-release).
Installing the CLI does not start or register an MCP server automatically.

If a client reports `ENOENT` or "Executable not found in $PATH", verify the install with
`command -v photocraft-cli` (PowerShell: `Get-Command photocraft-cli`) in the client's
environment. If that environment cannot be changed, set JSON `command` to the absolute
installed binary path, not a literal `~` or `$HOME` expression. Avoid symlinks into
`target/release`: cleaning the build directory breaks them. A source build alone does
not put the CLI on `PATH`.

## Start a session

PhotoCraft follows the same conventions as FilmCraft #28. Start a headless session (`--headless`:
without it, `mcp` works in the open PhotoCraft window while the ArtCraft Launcher's agents switch is
on, and may start the app; scripts, tests and agents pass it):

```sh
photocraft-cli mcp --headless \
  --automation-read-root /work/project \
  --automation-read-root /library/reference \
  --automation-write-root /work/project
```

Repeat `--automation-read-root` to grant ordered read directories in headless `mcp`, `serve`,
or desktop `photocraft --control`. Both `--flag <dir>` and `--flag=<dir>` work and can be mixed;
duplicate read roots are allowed. Relative file requests search the roots in launch order.
Headless reads select the first existing regular file; only a missing file tries the next root.
Directory, permission, path-escape and decoding errors stop headless requests instead of falling back.
Desktop `app.open`, including bridged `doc_open`, also supports native directory `.pcraft`
documents through the selected read capability. An earlier root's ZIP file wins over a later
root's directory; loading errors in a selected directory are terminal. Directory opens have
no implicit save target, even through the write root's capability, and need explicit layered
Save As beneath the write root.
All configured roots must be valid existing directories; an empty, missing or invalid root
fails startup rather than being dropped.

There is still only one `--automation-write-root`. Repeating it is a usage error (exit 2),
even with an identical value or mixed flag syntax. Writes are relative to that root, with no
fallback to any read root. Read and write authority are independent; a write root does not grant
reads, and omitting either kind of root denies that direction of filesystem access.
Absolute request paths must be beneath a corresponding granted root and are opened through
its directory capability, without ambient filesystem resolution of the requested path.

A document opened through a read root that does not share the write root's held directory
capability has no implicit save target. The same configured root shares that capability;
canonical filesystem identity is not inferred. Use an explicit `doc_save {path:"copy.pcraft"}`
(or another supported layered format) beneath the write root to establish one. Aliases and nested roots conservatively also
require explicit Save As; a flat export does not establish a save target. Existing restrictions
on pathless saves still apply. These grants do not enable ambient asset reads or headless
directory-bundle access; desktop directory `.pcraft` reads remain scoped to the granted capabilities.

Bridge mode uses `--bridge` and `--control-token-file` as described in
[development.md](development.md). Configure all roots on the desktop process; passing root flags
to `photocraft-cli mcp --bridge` is a usage error because the bridge cannot grant them.
UI tools require a running desktop app. The desktop environment variable
`PHOTOCRAFT_AUTOMATION_READ_ROOT` supplies one fallback directory only when no explicit read-root
flags are present; explicit flags replace it, preserving CLI order. Similarly, one explicit
write-root flag overrides `PHOTOCRAFT_AUTOMATION_WRITE_ROOT`. These variables do not grant
headless CLI file access.

A client has the full rights of the roots it is given: it can read any file below the read
root, and `doc_save`, `doc_export` and the export tools can create files anywhere below the
write root. `doc_save` and `doc_export` replace an existing file only when its extension is an
image or document format PhotoCraft writes (PNG, JPEG, TIFF, PSD, PSB, .pcraft, OpenRaster,
PDF, …) or its bytes are an image; anything else (`src/main.rs`, `.env`) is refused and left
untouched. The multi-file exports (layers, data sets) never replace a file. An agent reads
text from documents (layer names, metadata) that may try to steer it, so prefer a dedicated
output folder as the write root over a whole project.

Use absolute root paths in MCP client configurations, since a relative root is resolved from
the client's launch directory. In bridge mode, configure roots on the **desktop app**; root
flags on `photocraft-cli mcp --bridge` are rejected. Explicit `--control-token` and
`--control-token-file` flags require `--bridge` (or `PHOTOCRAFT_CONTROL_PORT`) and are mutually
exclusive. Bridge credentials can also come from `PHOTOCRAFT_CONTROL_TOKEN` or `PHOTOCRAFT_CONTROL_TOKEN_FILE`.

MCP and `serve` launch options (all but `--help`/`-h`) take one non-empty value each and cannot
be repeated. Missing values, stray positional arguments and options for the wrong mode are
usage errors (exit 2), reported on stderr before the server starts; stdout stays available for
protocol messages. For `serve`, explicit credential flags require `--port` (or
`PHOTOCRAFT_CONTROL_PORT`); stdio needs no control token.

## Tools and resources

- `command_list`: discover engine ids, parameter docs and current enabled state; accepts
  `filter` and `enabled_only`. Returns a bare JSON array of
  `{id, label, menu, shortcut, params, enabled, disabledReason, needsDocument}` objects (see
  [Command list replies](control-protocol.md#command-list-replies)).
- `command_run`: `id`, optional `params` and `wait`. Command parameters remain engine-defined.
- `command_batch`: ordered `steps`, optional `stop_on_error` (default true); stops at the first
  error unless false. Not an atomic transaction: earlier successful steps stay applied, with
  no rollback. Existing per-step results and reply budgets are preserved.
- `doc_inspect`: layer tree, history and selection; an empty session returns `document:null`.
  Every layer (including groups and nested children) reports raster-only `hasMask`,
  `hasVectorMask` for vector mask presence, and `vectorMaskEnabled` (`true`/`false` when
  present, `null` when absent). A disabled vector mask still has `hasVectorMask:true`.
- `render_preview`: bounded PNG content; `index` and `max_side` (default 1024, maximum 2048).
- `ui_inspect` and `ui_screenshot`: live app state/window in bridge mode.

Existing document/session/job/UI tools remain listed, including the documented
`doc_render_preview` spelling. There are no hidden legacy aliases. Every tool has a title
and all four annotation hints. Reads (`command_list`, `doc_inspect`, `session_list`, the
previews, `ui_inspect`, `ui_screenshot`, `jobs_list`) are read-only and idempotent. `doc_new`,
`doc_open`, `doc_place`, `doc_select` and the scoped exports (`doc_export_layers`,
`doc_export_data_sets`, which refuse to overwrite) are not destructive; `doc_select` is also
idempotent. Generic command/control tools conservatively advertise writes; file saves may
replace existing image and document files. Hints do not grant permission.

After a failed batch, `completed` and per-step `results` report successful steps, not a
rolled-back transaction. Inspect the document and its history with `doc_inspect`; commands
do not necessarily add one history entry each. For undoable edits, use `command_run` with
`{"id":"edit.undo","params":{}}` once per undoable history entry added, inspecting history
as you recover. Undo does not roll back files written to disk. If a reply-budget error says
the current step may have completed, inspect its effects before retrying or undoing it.

Before experiments, use `command_run` with `image.duplicate` and `{}` params, or
`history.newDocument` and `{}` params (copies the current state by default). Both create a
new active document, leaving the source unchanged; run experimental edits on that copy.
The optional `state` parameter of `history.newDocument` selects a retained historical state.

Unknown tool arguments return JSON-RPC `-32602`, naming the key, before execution. Batch
step envelopes are strict too. Nested engine `params` retain their command-specific handling.
Tool execution failures return `isError:true`. A worker panic becomes a tool error and the
existing poisoned-session recovery keeps serving. Invalid JSON lines receive `-32700` with
`id:null`; the next line is still processed.

Embedded Smart Objects support `command_run` with `layer.smartObjects.editContents` and
`layer.smartObjects.convertToLayers`, optionally targeting `params.layer`. Use
`layer.smartObjects.saveContents` in an opened contents document to update its parent in memory;
save nested contents from the inside outward, then `doc_save` the parent to a scoped path.
`doc_save` on a contents document exports that document rather than updating its parent.
No filesystem root is needed for the in-memory commands. External linked sources that require
a file read remain refused. See the [control policy](control-protocol.md#engine-commands)
for the shared restrictions.

`photocraft://document` and `photocraft://commands` return live JSON matching `doc_inspect`
and `command_list`. Resource reads are uncached; tool/resource catalogs are private and cached
for ten minutes. List/read responses include the MCP 2026-07-28 result/cache fields.

## Progress and cancellation

`command_run` (and each `command_batch` step) that waits, the default, runs a job-capable
command (filters, Content-Aware Fill/Scale, Photomerge, …) as a background job and returns its
result when the job ends. Meanwhile the server serves other calls (`doc_inspect`, `jobs_list`,
…), and the running job shows in `jobs_list`. With `_meta.progressToken`, the call sends
`notifications/progress` with the job's progress (0–1, `total` 1; for a batch, finished steps
plus the current step's fraction, `total` the step count) and the job's `message`, increasing
strictly and at most ten per second. `notifications/cancelled`, or the client dropping the
request, cancels the job: the document stays unchanged and a cancelled batch runs no further
step. A job that already finished before the cancellation arrived stays applied. Commands that
are not jobs still run to the end.

Headless, the session runs the job itself (#8129). In bridge mode (#8130) the app runs it: the
server starts it with `engine.execute {wait: false}` (sent once, never resent) and polls
`jobs.list`, so a job may run past the bridge's 55s reply timeout. A command that is not a job,
a UI command id and a lost reply to the start still fail at that timeout; see
[MCP bridge](control-protocol.md#mcp-bridge) for the limits.

A direct headless `command_run` of `file.export.renderVideo` reports frame counts when
`_meta.progressToken` is present (a string or number). Notifications increase strictly,
are throttled to at most ten per second except completion, and include `total`. No token
means no progress notifications. Ping remains responsive while rendering.

```json
{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"command_run","arguments":{"id":"file.export.renderVideo","params":{"dir":"frames","format":"png"}},"_meta":{"progressToken":"export-20"}}}
{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":20}}
```

Create a document and timeline first (`doc_new`, then `timeline.create`). Export uses the
configured write root and creates `dir` beneath it. Filenames must be single components;
absolute paths, traversal and escaping symlinks are rejected. Existing target filenames
are refused, including symlinks, so a cancelled or failed job cannot replace an earlier export.
Only files actually created by this job are removed on cancellation, error or an escaped panic;
unrelated files and the destination directory remain. PNG sequences and animated GIF share
the engine renderer. The document and playhead are unchanged.

Cancellation suppresses the request response and stops at a frame boundary (the current
frame/encoding may finish first). Unknown request ids are ignored. Render Video stays a
synchronous command even with `wait:false`; other job-capable commands keep their existing
`jobs_list`/`jobs_cancel` behavior. Render Video as a `command_batch` step reports no
progress and cannot be cancelled. Nested engine actions and bridge Render Video remain denied by the existing
ambient-filesystem policy; the new scoped writer is available for direct headless commands.
