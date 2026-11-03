# Scratch disks and memory residency

Native PhotoCraft keeps pixel tiles in RAM and spills cold tiles to a configured scratch
directory. A read or write guard keeps a tile resident while a consumer uses its bytes. Undo
snapshots keep their copy-on-write tiles, including disk-backed tiles, until History States
retires them. Web builds retain their explicit memory limit (capped like Memory Usage) and do not use a
filesystem.

New preferences use **70% of physical RAM** as the process ceiling. This is configurable from
1–95%; older preferences without `performance.memoryUsagePercent` retain their absolute
`performance.memoryUsageMb` limit. Other applications, non-tile process memory and bounded
working buffers reduce the live tile allowance. The separate `performance.gpuMemoryMb`
setting limits graphics resources. Memory virtual address-space usage is diagnostic data,
not a measurement of committed memory or a substitute for scratch storage.

Enable a directory in Preferences → Scratch Disks. `(system temp)` selects the operating
system's temporary directory. Only the first enabled, nonempty directory is used. Changing
directories creates a new scratch file; old disk-backed tiles keep their old file alive.
The development override `PHOTOCRAFT_TILE_BUDGET_MB` fixes the tile allowance for stress tests.

Scratch extents are LZ4 compressed, CRC-checked and reused after a tile is released. Closing
the last tile truncates its file. No scratch file outlives the process, including a crash or
an exit without Rust destructors: Windows opens it with delete-on-close, and Unix creates it
with mode `0600` and unlinks it immediately after opening, reading and writing through the
one open handle with positioned I/O (`pread`/`pwrite`). The space is returned when the last
handle closes. Configuring a directory also sweeps `photocraft-<pid>-<nonce>.scratch` files of
dead processes (older builds, or a crash between creating and unlinking). It preserves
live-process files, other users' files, symlinks and unrelated names; unavailable process
enumeration leaves files untouched. PID reuse conservatively retains a stale file, and a
unique launch nonce avoids a collision. A filesystem that refuses the unlink keeps the name
until the file is dropped or the next sweep.

The engine owns a shared, stoppable residency supervisor and joins it when its last session
is dropped. `photocraft-raster` only exposes synchronous measurement and bounded eviction
operations; standalone callers work without a background thread. `sysinfo` is native-only
with its default features disabled and only the `system` (RAM) and `disk` (free space)
features enabled.

An unavailable or full scratch disk never loses pixels or turns spilling off for the session.
A failed scratch write or file creation, or a volume about to run low (free space is checked
when configuring, every 64 MB of scratch growth and on each retry; spilling stops while less
than max(1 GiB, 5% of the volume) would stay free), pauses spilling: tiles stay in memory,
the status bar shows the reason, and History's Memory Usage byte limit applies again from the
next command, as without a scratch disk. The pause is retried after 1 s, doubling up to 60 s;
spilling resumes on its own once the directory is usable and has room, and changing
preferences retries at once. Preferences → Scratch Disks → Scratch Size Limit
(`scratchDisks.sizeLimitMb`, 0 = no limit; `spill::set_size_cap`) caps the scratch file:
reaching it pauses spilling the same way, and the free-space reserve applies regardless.
A failed read retains its backing extent and blocks editing, saving and exporting of the
document that holds the tile until it is readable again; other open documents keep working.
Each commit checks the health of its own document (before and after an edit, the document a
job captured, the imported result, the document being saved or exported) and refuses a result
if any tile failed or recovered while it ran; the retry succeeds. Derived-pixel caches never
admit results built from unreadable tiles and are purged once per new failure. Recovery can
retry the original bytes after the storage becomes available; zeros from a failed display
read must never become saved pixels. Limitation: a tile that was already unreadable and is
read from outside the edited document (for example a clipboard copy) is not detected.

Scratch storage alone does not make every format, filter or renderer streamable. See
[EXR streaming](streaming-io.md) for the block-by-block EXR import. Viewport rendering is a
separate change built on these guards and memory reservations.

Available RAM is measured in one place, `photocraft_raster::memory::system` (cgroup v1/v2
limits on Linux, reclaimable pages on macOS, the commit limit on Windows; swap never counts as
RAM, and an unreadable container limit fails closed). The residency budget and the PSD/PSB and
layered-TIFF import admission (`photocraft_io::memory`) both read it. While spilling is on and
not paused, import admission counts the tile pixels of a file against RAM plus the scratch room
(`spill::scratch_capacity`: the free space above the reserve, within the Scratch Size Limit);
decode buffers, conversion copies and merged planes still need RAM. With spilling off or
paused, and on the web, the RAM-only limits apply unchanged.
