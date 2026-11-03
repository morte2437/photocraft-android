# Command-line interface

`apps/photocraft-cli` provides the following current subcommands:

| Command | Purpose |
|---|---|
| `convert` | Open one document and export it to another format |
| `info` | Print document size, mode, depth, and layer information as JSON |
| `run` | Open or create a document, execute engine commands, and optionally save |
| `batch` | Apply an action list to files in one input directory |
| `droplet` | Run a PhotoCraft droplet against files or folders |
| `commands` | List command IDs and parameter documentation |
| `mcp` | Start the MCP server on stdio, optionally bridged to the desktop app |
| `serve` | Start the JSON-lines headless server on stdio or loopback TCP |

Use `photocraft-cli --help` (or `photocraft-cli <subcommand> --help`) as the executable source of truth. A typical bounded command run is:

```sh
photocraft-cli run input.psd \
  --cmd image.adjustments.invert \
  --out output.png
```

### Missing font warnings

If a type layer uses a font family that is not installed, `convert` and `run`
write a warning to standard error before exporting; they do not silently present
the fallback rendering as an exact match. `info --compact` includes the same
warning in its `warnings` JSON array, without writing it to stderr. `run`
also adds `warnings` to the JSON result for the command that first exposes
the missing family. A multi-command `run` reports each missing family once,
even if later commands still use it.

Warnings do not change the saved document's font declarations and do not stop
the operation. To list or replace the missing families intentionally, use
`type.resolveMissingFonts` on the document.

### Export selected adjustment layers as a .cube LUT

The engine command `file.export.colorLookupTables` creates a 3D LUT from an
identity RGB lattice. It can export all visible top-level adjustment layers
(the default), the current Layers panel selection, or a supplied list of
adjustment layer IDs. The `scope` and `layers` parameters are mutually
exclusive when `scope` is `selected`:

```json
{"size": 33, "scope": "selected", "path": "chosen-look.cube"}
```

For automation that does not use the Layers panel, supply `"layers": [10, 14]`
instead. Input ID order does not change the final color look: adjustments are
always baked in **document stacking order**, bottom to top. When `path` is
omitted, the command returns the .cube text in `cube`; both forms return
`size` and `layerCount`.

Valid LUT dimensions are 2–129 (same maximum as PhotoCraft's LUT reader).
Selected IDs must exist, be visible top-level adjustment layers, and contain
no duplicates; invalid selections report an error rather than silently
exporting a different look. Masks, vector masks and clipping are spatial
operations and cannot be represented faithfully by a color-only LUT, so the
baker omits them. Group-contained adjustments are not exported as independent
layers because that would lose their group's blending semantics.

Paths supplied to the current CLI are ordinary operating-system paths. The CLI does not restrict them to a workspace root. Batch output creates the requested output directory, and save operations may create parent directories. `batch` refuses an output directory that is its input directory (the results would replace the originals) unless `--in-place` is given. Run untrusted action files only in an appropriately isolated account or environment.

CLI parsing checks the flags and their values before any work starts. A flag the subcommand doesn't take (a typo such as `--fromat`), a flag given twice (only `--cmd` and `--params` may repeat), a value out of range (`--quality 0`), malformed `--new` or `--params` JSON, `<FILE>` together with `--new`, and `--control-token` together with `--control-token-file` are all usage errors (exit status 2), rather than being ignored or failing later. A value flag never takes the next flag as its value, so a value starting with `-` is written with `=` (`--out=-image.png`) or as `./-image.png`; negative numbers such as `--filter -1` still work. A path that is not valid Unicode reaches the command instead of being a usage error: `convert` reads and writes it, while `info`, `run` and `batch` open files through the automation layer, which still needs UTF-8 paths and fails with exit status 1 (`droplet` takes its arguments as text, so it refuses such a path as a usage error). `photocraft-cli --version` (or `-V`) prints the version; there is no `version` subcommand.

`mcp` and `serve` reject positional arguments, empty values and options for the wrong mode before
starting a server. Explicit control-token flags require `mcp --bridge` or `serve --port` (or
`PHOTOCRAFT_CONTROL_PORT`, below). Configure automation roots on the desktop process when using
bridge mode; the CLI rejects root flags there. See the [MCP launch guide](mcp.md) for examples.

`mcp` and `serve` fall back to environment variables: `PHOTOCRAFT_CONTROL_TOKEN` and `PHOTOCRAFT_CONTROL_TOKEN_FILE` for the token flags, and `PHOTOCRAFT_CONTROL_PORT` for `serve --port` and `mcp --bridge` (which then bridges to `127.0.0.1:<port>`). The automation roots are flags only: the desktop app's `PHOTOCRAFT_AUTOMATION_READ_ROOT` and `PHOTOCRAFT_AUTOMATION_WRITE_ROOT` grant the CLI nothing. A flag wins over its variable, an empty variable counts as unset, and a bad value is a usage error naming the variable. When `PHOTOCRAFT_CONTROL_PORT` switches `serve` to TCP or `mcp` to bridging, the command says so on standard error. A token together with a token file is a usage error only where a token is read (`serve` on TCP, `mcp` bridging), so a stale exported pair doesn't stop stdio `serve` or headless `mcp`. An automation root given to `mcp` while the variable would switch it to bridging is a usage error too: the roots are the headless engine's, and the bridge would drop them. `<subcommand> --help` lists each variable (unless it is set but empty), never with the token's value. A variable holds a port, so `PHOTOCRAFT_CONTROL_PORT=127.0.0.1:7878` is refused; only `--bridge` takes `host:port`.

This checking is not a security policy layer. Engine commands remain responsible for validating their own parameters.

### Shell completions and man pages

The Linux `.deb` and `.rpm` packages install `man photocraft-cli` (plus a page per subcommand, such as `man photocraft-cli-convert`) and completions for bash, zsh and fish (zsh's in `/usr/share/zsh/site-functions`, which the `.deb` also links from Debian's `/usr/share/zsh/vendor-completions`); the Linux tarball and the FreeBSD tarball carry the same files, the FreeBSD one under `man/man1` and `share/`. The pages list each flag's environment variable, as `--help` does. Without a package, generate them from a source checkout:

```sh
cargo run -p photocraft-cli --example man-and-completions -- out/
```

`out/` then holds `photocraft-cli.bash`, `_photocraft-cli` (zsh), `photocraft-cli.fish`, `photocraft-cli.elv` (elvish), `_photocraft-cli.ps1` (PowerShell) and the `.1` man pages. For example, copy `photocraft-cli.bash` to `~/.local/share/bash-completion/completions/photocraft-cli`, `photocraft-cli.fish` into `~/.config/fish/completions/`, `_photocraft-cli` into a directory on zsh's `$fpath`, and read a page with `man -l out/photocraft-cli.1`. The desktop app `photocraft` has neither.
