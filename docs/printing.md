# Printing

File › Print renders the flattened document onto one PDF page (paper size and orientation,
position and scale, crop and registration marks, labels, colour handling) and hands that page
to a printer. The rendering is `print_cmds.rs` in `photocraft-engine`, shared by the dialog, the
batch commands and automation (`file.print`, `file.printOneCopy`); this page is about the hand-off.

## Positioning the preview

Drag the visible image on the paper to change Top and Left. Starting a drag switches Center
off using the currently resolved layout, including Scale to Fit Media, so the image stays at
the grab point. Top/Left stay visible but disabled while Center is on, keeping the dialog and
paper stationary when a drag enables them. Position fields accept negative values in inches, centimetres or millimetres;
the stored command parameters are inches from the sheet's top-left. Manual positions are
bounded to ±10000 inches. Height, width and paper dimensions retain their positive fields.

The preview converts egui point displacement to PDF points using its paper display scale.
No extra Retina pixel factor is needed: painting and pointer input both use egui points.
Emulsion Down reverses the horizontal position change to match the mirrored output. The image
is clipped to the paper; any visible portion remains a drag target. A completely off-page image
can be recovered with Top/Left or Center. These controls affect print parameters only, and
the preview and generated PDF continue to share `print_cmds::layout`.

## Routes to the printer

| Route | When | What the user sees |
|---|---|---|
| **`craft-print`** (`lp` on CUPS, the Windows spooler) | `lp` is on the path (macOS, Linux, the Flatpak, which bundles it) and CUPS lists a printer, or on Windows | `Sent to the printer (request id is Office-42)` |
| **CUPS over IPP** (`photocraft-ipp`) | No `lp` on the path and a print server answers: `CUPS_SERVER`, then `/run/cups/cups.sock`, `/var/run/cups/cups.sock`, `/private/var/run/cupsd`, then `localhost:631`; always for a `printServer` named in the request | `Sent to the printer (job 42 on Office)` |
| **Print portal** (`org.freedesktop.portal.Print`, desktop app on Linux) | Linux only (the Flatpak included): no print server reachable or no printer set up (including when `lp` is there but CUPS lists no printer and none was named), or always with `PHOTOCRAFT_PRINT_PORTAL=1`. Elsewhere a missing route is an error | The system print dialog, seeded with the copies and paper; the outcome goes to the log |
| **Print to PDF** | `output` given (the Output field of the dialog) | The file; nothing is spooled unless `send` is true |

A route that exists but fails (an unknown printer, a job the queue refuses) is an error; only a
missing route (no `lp`, no print server, no printer) moves on to the next one. With no printer
named and no default, the only printer takes the job on the `lp` and IPP routes; Windows needs
a named or default printer.

The IPP client is in-process, pure Rust (RFC 8010/8011: `CUPS-Get-Printers`, `CUPS-Get-Classes`,
`CUPS-Get-Default`, `Print-Job` with the PDF as the document), so no `lp` binary is needed. It
only sees queues the server has, not driverless network printers, which is why the Flatpak
bundles `lp`/`lpstat` (the manifest's `cups-client` module) and grants `--socket=cups` (the
host's CUPS socket) and `org.freedesktop.Avahi` (`lpstat -e` finds driverless printers). On a
system without CUPS, or in a sandbox without the socket, the app falls back to the print portal,
whose dialog runs outside the sandbox.

## Which printer

The dialog's Printer field, or `"printer"` in the command, names a queue (case-insensitively; a
CUPS class counts, as with `lp -d`).
Left empty, the server's default printer is used; without a default, the only printer; with
several and no default, the error names them. With no printer at all, the portal takes over when
it is available, otherwise the error says to add one in the system settings or print to PDF.

`file.printers` lists what the print server knows: `{"printServer": "host[:port]|socket"}`
is optional and defaults to the system's CUPS; the result is
`{server, default, printers: [{name, info, state, acceptingJobs, default, class}]}`, or an empty list
with a `note` when no server answers. `"printServer"` on `file.print` sends one job to a named
server; `"dryRun": true` renders the page and reports the equivalent `lp` command without
spooling.

## Orientation (Windows)

On Windows the paper and its orientation are the driver's: the Print dialog takes them from the
printer's settings. Its Orientation buttons (Portrait, Landscape; Photoshop's Layout) turn those
settings, as Print Settings… would, and the paper and preview turn with them.
`file.print.page {"orientation": "portrait|landscape"}` does the same for scripts: it returns the
turned settings as `printerSettings`, with the page they give. A print sent to a Windows printer
whose layout is the driver's paper the other way round (a landscape layout, portrait settings)
turns the settings to the layout before printing, rather than stopping with "the driver paper or
orientation differs"; another paper is still refused.

## Results

`file.print` returns `sent`, `backend` (`lp`, `windows`, `ipp`, `portal`), `spooler` (where the
job went) and `pdf` (the page's path, only when `output` named one).

## Environment

| Variable | Effect |
|---|---|
| `CUPS_SERVER` | The print server, as CUPS itself reads it: `host[:port]` or a socket path |
| `PHOTOCRAFT_PRINT_PORTAL=1` | Always use the system print dialog (the portal) instead of `lp` or CUPS directly |
| `PHOTOCRAFT_PRINT_PORTAL=0` | Never use the portal; a missing print server is then an error |

## Verification

- `crates/ipp/src/tests.rs`: the client against a fake CUPS over TCP and a Unix socket:
  printer listing with state and default, the default printer and its absence, `Print-Job`
  with the document and the job id, chunked and content-length replies, HTTP and IPP error
  statuses, broken replies.
- `crates/engine/src/print_cmds/tests.rs`: printer resolution (requested, default, only one,
  several without a default, unknown, none); a complete `file.print` against a fake CUPS
  (printer URI, copies, job name, document format, the PDF itself) and `file.printers`;
  `lp` taking the job when it is on the path; the IPP route when it is not (a child process
  with an empty path); the app's spooler when preferred, as the fallback without a printer, and
  skipped when a server is named; print to PDF untouched.
- `apps/photocraft/src/print_portal.rs`: the request path rule and the paper/copies seeding of
  the dialog (PWG names for the standard papers, millimetres for custom sizes).
