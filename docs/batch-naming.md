# Batch output naming

Select a recorded action, then open **File > Automate > Batch**. Source can be a folder (with
**Include All Subfolders**) or the open documents. **File Naming** accepts a template. Leave it
blank to keep source filenames and collision reporting. A template without `{ext}` gets the
chosen output extension appended.

| Token | Value |
| --- | --- |
| `{name}`, `{NAME}`, `{Name}` | Input filename without its last extension, unchanged, uppercase, or initial capital |
| `{serial}`, `{serial:1}` … `{serial:4}` | Input position plus **Starting serial** (default 1), padded to at least 3 or the specified number of digits |
| `{letter}`, `{LETTER}` | Serial letters: a … z, aa …; uppercase with `{LETTER}` |
| `{date}` | UTC date at the start of the batch, in `YYYY-MM-DD` form |
| `{date:mmddyy}`, `{date:mmdd}`, `{date:yyyymmdd}`, `{date:yymmdd}`, `{date:yyddmm}`, `{date:ddmmyy}` | Other UTC date arrangements |
| `{ext}`, `{EXT}` | Output extension, lowercase or uppercase |

For example, `{name}_{serial}` with starting serial 7 writes `photo_007.png` and
`photo_008.png` for two PNG inputs named `photo.png`. A serial after 999 becomes 1000;
it does not wrap. Folder inputs use the existing filename order, and an explicit input array
keeps its order. Failed inputs consume their position, so later filenames do not shift.
Braces in an input filename are literal text, not additional tokens. Subfolder inputs keep their
relative folders under the output folder. **Stop On Error** leaves later inputs unprocessed;
**Error Log** records failures and their reasons. Folder batches report progress per file and
can be cancelled after the current file. A cancelled batch returns `cancelled: true`, its
completed `files`, and `skipped` inputs. If writing the error log fails, the batch still
returns its per-file results with `errorLogError` instead of `errorLog`.

Unknown tokens, unmatched braces, invalid parameter types, path separators, reserved filename
characters, excessive filename length (255 UTF-8 bytes), and serial overflow are rejected before
export writes begin. Starting serial must be an integer from 1 to 4,294,967,295.
A custom template that resolves to an existing input file is also rejected; choose a separate
destination or another template. Ordinary per-file read, action and save failures still appear
in the batch's `errors` result. When two results share a name, including a case-only difference,
the first successful result is kept and the later collision is reported.

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
`--error-log` flags. Image Processor and droplet options retain their existing behavior.
