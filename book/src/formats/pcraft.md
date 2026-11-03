# The `.pcraft` format

`photocraft-format` stores the native lossless document as a ZIP archive or directory bundle. Its main entries are:

```text
manifest.json
tiles/<blake3>.zst
blobs/<blake3>.zst
thumb.png
composite/preview.png
composite/full.png
save.lock
```

`thumb.png` (at most 256 px on the longer side), `composite/preview.png` (at most 1024 px) and `composite/full.png` (the canvas size) are the flattened document as 8-bit RGBA PNGs, whatever the document's depth. The manifest names each one it holds, under `thumbnail`, `composite` and `full_composite`.

A save writes `composite/full.png` only when it is asked to: *Save Full-Size Composite in .pcraft Files* in Preferences › File Handling for the app's saves, `"pcraftFullComposite": true` for the engine's `file.saveACopy` and its batch and export commands, and for the automation `doc.save` and MCP `doc_save`, and `--pcraft-full-composite` for `photocraft-cli convert`, `run` and `batch`. It is off by default. `composite/full.png` embeds the ICC profile of its pixels: an RGB document's own profile, a grayscale document's gray profile as RGB, and sRGB for CMYK, Lab and documents without a usable profile. A save leaves it out when the canvas is larger than 2^26 pixels, when its pixels cannot be allocated or encoded, or when a ZIP bundle would pass 4 GiB or 65,535 entries with it. A directory bundle saved without it has the file from an earlier save removed.

`save.lock` exists only in directory bundles: a save holds an OS file lock on it while it writes objects, publishes the manifest and removes unreferenced objects, so concurrent saves of one bundle take turns. It is empty, and readers ignore it.

The manifest is versioned and describes the document tree. Tiles and blobs are content-addressed. Loaders verify ZIP entry bounds and CRCs; document content also uses stored hashes to detect mismatched or corrupted tile/blob content. Unsupported newer format versions fail explicitly.

## Implemented load limits

Default `LoadOptions` apply these maximums:

| Limit | Default |
|---|---:|
| `manifest.json` | 256 MiB |
| One decompressed blob | 1 GiB |
| Total decompressed tiles and blobs | 16 GiB |

Layer groups may nest at most 100 levels (`MAX_GROUP_DEPTH`). Saving a deeper document fails with an error instead of writing a bundle that can't be opened, and a manifest whose JSON nesting goes beyond what that depth needs is rejected before it is parsed, so a hostile file can't exhaust the stack.

The ZIP reader bounds decompression by the caller-supplied maximum, rejects encrypted entries and unsupported compression methods, validates offsets with checked arithmetic, and rejects CRC or declared-size mismatches. Corruption and property tests cover truncation, byte mutation, missing content, invalid manifests, excessive sizes, and random bytes.

These defaults are intentionally finite but can still be expensive on constrained systems. Future hardening should add context-sensitive budgets and explicit limits for entry count, document complexity, and cumulative work, then test directory bundles against symlink and replacement races.

When fields are added to `photocraft-doc`, `photocraft-format` intentionally requires corresponding manifest and conversion updates so native saves do not silently drop new state.
