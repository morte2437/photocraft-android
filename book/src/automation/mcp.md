# MCP

`photocraft-automation` implements an MCP server over stdio with two backends:

- **Headless:** an in-process `Headless` session owns the engine and performs file I/O.
- **Bridge:** `BridgeClient` forwards tools to a desktop application started with `photocraft --control <port>`.

For a source checkout, first [install the CLI on PATH and register the server](../../../docs/mcp.md#install-and-register).
The installed binary survives `cargo clean`; a symlink into `target/release` does not.

Start the headless server with:

```sh
photocraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project
```

Bridge to a running desktop application with the same private token file used by the app:

```sh
photocraft-cli mcp --bridge 127.0.0.1:7878 --control-token-file /private/path/control.token
```

Use absolute root paths in MCP client configurations, since a relative root is resolved from the
client's launch directory. In bridge mode the roots are configured on the desktop app, and root
flags on `photocraft-cli mcp --bridge` are rejected. Explicit `--control-token` and
`--control-token-file` flags require `--bridge` (or `PHOTOCRAFT_CONTROL_PORT`) and are mutually
exclusive; bridge credentials can also come from `PHOTOCRAFT_CONTROL_TOKEN` or
`PHOTOCRAFT_CONTROL_TOKEN_FILE`. Each launch option
except `--help`/`-h` takes one non-empty value and cannot be repeated. Stray positional arguments,
missing values and options for the wrong mode are usage errors (exit 2) reported on stderr before
the server starts.

Tools cover session/document operations, command discovery and execution, batching, and—when bridged—UI inspection and control. Tool schemas improve correctness but are not authorization boundaries.

## From an installed release

The release packages ship `photocraft-cli` alongside the desktop app, so no build is needed:

| Install | CLI |
|---|---|
| Windows (MSI) | `%LOCALAPPDATA%\Programs\ArtCraft\PhotoCraft\photocraft-cli.exe` by default (`C:\Program Files\ArtCraft\PhotoCraft\photocraft-cli.exe` for an all-users install, under `Program Files (x86)` instead for the 32-bit build, or wherever you installed it otherwise), not on `PATH` |
| Linux (deb, rpm) | `/usr/bin/photocraft-cli` |
| macOS | the separate `photocraft-cli-<version>-macos-<arch>.zip` release asset (the `.app` holds only the desktop app) |

```sh
# Windows (PowerShell), default install folder
claude mcp add photocraft -- "$env:LOCALAPPDATA\Programs\ArtCraft\PhotoCraft\photocraft-cli.exe" mcp --automation-read-root <dir> --automation-write-root <dir>
# Linux, or macOS with the CLI unzipped onto PATH
claude mcp add photocraft -- photocraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>
```

## Security notes

Stdio MCP does not open a network listener. Its client receives only the read and write roots
explicitly granted at launch; omitting a root omits that authority. Bridge mode authenticates its
loopback control connection with a bearer token before invoking any control method and uses the
roots configured on the desktop process.

The current server does not issue scoped sessions or distinguish trusted tools from high-impact tools. Authentication proves possession of the token; it does not restrict an authenticated client to document-only access. Do not expose either transport through an untrusted broker, shell, proxy, or remote service without adding an external policy boundary.

Filesystem capabilities are implemented. General tool and method capabilities remain
[proposed](../security/automation-security.md).

The five shared core tools are `command_list`, `command_run`, `command_batch`, `doc_inspect`
and `render_preview`. Existing documented tools, including `doc_render_preview`, remain
listed. `photocraft://document` and `photocraft://commands` expose live JSON. Headless
long commands that are waited on report progress and stop when the request is cancelled, and
direct headless Render Video does too, using the granted write root; see
[`docs/mcp.md`](../../../docs/mcp.md) for arguments, cleanup and transport behavior.
