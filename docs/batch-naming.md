# Batch output naming

Select a recorded action, then open **File > Automate > Batch**. Source can be a folder (with
**Include All Subfolders**) or the open documents. **File Naming** accepts a template. Leave it
blank to keep source stems with the selected output extension. An explicit template spells the
whole filename; without `{ext}`, no extension is added.

| Token | Value |
| --- | --- |
| `{name}`, `{NAME}`, `{Name}` | Input filename without its last extension, unchanged, uppercase, or initial capital |
| `{serial}`, `{serial:1}` … `{serial:4}` | Input position plus **Starting serial** (default 1), padded to at least 3 or the specified number of digits |
| `{letter}`, `{LETTER}` | Serial letters: a … z, aa …; uppercase with `{LETTER}` |
| `{date}` | UTC date at the start of the batch, in `YYYY-MM-DD` form |
| `{date:mmddyy}`, `{date:mmdd}`, `{date:yyyymmdd}`, `{date:yymmdd}`, `{date:yyddmm}`, `{date:ddmmyy}` | Other UTC date arrangements |
| `{ext}`, `{EXT}` | Output extension, lowercase or uppercase; a dot is added when this is the final component, unless the template already supplied one |

For example, `{name}_{serial}` with starting serial 7 writes `photo_007` and
`photo_008` for two PNG inputs named `photo.png`; `{name}_{serial}{ext}` writes
`photo_007.png` and `photo_008.png`. The file bytes still use the selected output format.
A serial after 999 becomes 1000;
it does not wrap. Folder inputs use the existing filename order, and an explicit input array
keeps its order. Failed inputs consume their position, so later filenames do not shift.
Braces in an input filename are literal text, not additional tokens. Batch writes subfolder
inputs flat into the output folder in input order. A later same-name result replaces an earlier
one, as measured in Photoshop 2026. **Stop On Error** leaves later inputs unprocessed;
**Error Log** writes a UTF-16 LE file with a BOM and CRLF line endings even when no file fails.
It starts with `Batch Start`, records a `File` / `Error` pair per failure, and ends with
`Batch End`. The wording is English because PhotoCraft does not have Photoshop's numeric error
codes or German message catalog. Folder batches report progress per file and
can be cancelled after the current file. A cancelled batch returns `cancelled: true`, its
completed `files`, and `skipped` inputs. Background job status is `cancelled` and retains
the partial result; a waiting control request replies with `ok: false`, `error: "cancelled"`,
and that result. If writing the error log fails, the batch still
returns its per-file results with `errorLogError` instead of `errorLog`.

Unknown tokens, unmatched braces, invalid parameter types, path separators, reserved filename
characters, excessive filename length (255 UTF-8 bytes), and serial overflow are rejected before
export writes begin. Starting serial must be an integer from 1 to 4,294,967,295.
A custom template that resolves to an existing input file is also rejected before processing;
the same guard checks default flat output names. This prevents an early result from destroying
a later unread input. Ordinary per-file read,
action and save failures still appear in the batch's `errors` result. Image Processor and lens
correction continue to report output-name collisions.

The same options are available on `file.automate.batch` through the engine command interface:

```json
{
  "input": ["first/photo.png", "second/photo.png"],
  "output": "exports",
  "steps": [["image.adjustments.invert", {}]],
  "format": "png",
  "naming": "{name}_{serial}",
  "startingSerial": 7
}
```

The CLI has matching `batch --recursive`, `--naming`, `--start-serial`, `--stop-on-error`, and
`--error-log` flags. Droplets use Batch's flat output and naming behavior.

## Image Processor sources

**File > Scripts > Image Processor…** has a **Source** selector: **Current document**,
**All open documents**, or **Folder**. It defaults to the current document when one is open.
Open-document sources include unsaved edits and untitled images. Resizing and export run on
copies; the open documents, their history and their source files remain intact. Choose a
separate output folder. Duplicate output names appear in the per-file errors instead of
replacing an earlier result.

Untitled document names follow the portable filename rules used for layer exports: for
example, `foo/bar` becomes `foo_bar`. The document's displayed name stays unchanged.

Agents can use the same engine command:

```json
{"source": "current", "output": "exports", "format": "jpg", "width": 1920, "height": 1080}
```

Pass this to `file.scripts.imageProcessor`. Use `"source": "open"` for all open documents;
`"source": "folder"` (the command default) takes `"input"` as before. The input folder and
**Include All Subfolders** controls appear only for the folder source.

## Reference behavior and conformance

The [Photoshop 2026 Batch measurements](https://github.com/davidmann4/photocraft/blob/docs/atn-event-reference/docs/atn.md#batch-behaviour)
record flat subfolder output, last-result-wins collisions, and extension components that add a
dot only at the end. `batch_include_all_subfolders`,
`batch_naming_later_result_replaces_the_earlier_one`, and
`extension_component_only_adds_a_dot_at_the_end` check those cases. PhotoCraft deliberately
rejects a plan that would replace another input file before it is read, to prevent data loss.
The desktop Batch follows the measured flow: it shows no progress in the status bar, previews each
input read-only on the canvas, and Esc asks whether to continue or stop. Headless control clients
still read the job's progress and stop it with `jobs.cancel`. The inputs are not live tabs with
Layers panel content, so AUTO-219-5 stays partial.
