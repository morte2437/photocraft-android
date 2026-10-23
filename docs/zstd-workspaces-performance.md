# Native save writer reuse

Each open document keeps a writer through the ordinary save settings. Unchanged tile and
blob identities reuse weakly cached hashes; compressed payload capacity retained between
saves is capped at 64 MiB. The last cache owner releases it. Other formats ignore the cache,
and one-shot exports still work. Save As names, previews, cancellation, atomic writes and
saved revisions keep their existing behavior. Tile hash identity includes sample depth.
Fallible edits move uniquely owned tile pixels when weak cache references remain; shared
pixels keep their copy-on-write behavior.

The format writer's compressed-cache limit defaults to unbounded for existing callers.
The UI selects 64 MiB. This is retained memory that a one-shot save would release; it does
not cap current-save scratch or ZIP output, and it does not reduce bytes written to disk.

## Measurements

Release build on Apple M4 Max. CC0 Pixls foliage (24 MP, unscaled crop) and city (12 MP,
native size) edits contain a photograph, global Levels, Curves and masked foreground Levels.
ProPhoto RGB is retained. U8/U16/F32 are tested; F32 derives from developed U16 samples.
Two upstream PSD designs add authored layered-file coverage. No generated noise is timed.

The table uses seven alternating pairs after warmup and includes thumbnail/composite
rendering, encoding, APFS atomic writes, fsync and rename. Hashing, reload equality and file
removal are outside timers. Intervals are paired bootstrap 95% descriptions of these samples.
These writer-reuse results compare main `d95fdf31` and the original parent `f64a143c`
with `05bd004b`; the original parent's production save path matches that main baseline.
Each small edit changes one real-photo tile. The later cache-ownership changes remove
redundant allocations and reference-count updates; no separate timing gain is claimed.

| Document | Operation | Main ms | Original parent ms | Writer reuse ms | Change vs original parent |
|---|---|---:|---:|---:|---:|
| 24 MP foliage, U16 | save-first-disk | 966.0 | 966.0 | 984.9 | +2.0% (-8.5..+2.7) |
| 24 MP foliage, U16 | save-repeat-disk | 983.0 | 983.0 | 651.8 | -33.7% (-35.8..-30.9) |
| 24 MP foliage, U16 | save-edit-disk | 968.6 | 968.6 | 640.1 | -33.9% (-34.2..-33.0) |
| 12 MP city, U8 | save-first-disk | 660.8 | 660.8 | 621.6 | -5.9% (-7.5..+1.1) |
| 12 MP city, U8 | save-repeat-disk | 658.9 | 658.9 | 496.6 | -24.6% (-26.4..-18.8) |
| 12 MP city, U8 | save-edit-disk | 628.4 | 628.4 | 502.2 | -20.1% (-26.3..-17.8) |

An independent nine-pair U16 first-save check changed its median delta from +2.0% to -5.9%,
with both intervals crossing zero; no repeatable first-save regression or speedup is established.
Initial in-memory repeated-export medians across six photo/depth cases improve 18–47%.
The independent disk runs confirm 20–34% gains for repeated and small-edit saves.
Peak RSS is close to main; the retained-payload idle-memory cost still applies.

Every output is byte-identical to the baseline and reloads to the complete source document.
Scoped tests cover cache capacity/output, weak identities, sample depth, Save As, lifetime and
save-settings forwarding. Existing save-identity tests cover failed encoding/writing and copies.
