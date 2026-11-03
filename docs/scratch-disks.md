# Scratch disks and memory residency

Native PhotoCraft keeps pixel tiles in RAM and spills cold tiles to a configured scratch
directory. A read or write guard keeps a tile resident while a consumer uses its bytes. Undo
snapshots keep their copy-on-write tiles, including disk-backed tiles, until History States
retires them. Web builds retain their explicit memory limit and do not use a filesystem.

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
the last tile truncates its file. Windows opens files with delete-on-close so process exit,
including exit without Rust destructors, removes them. Unix creates files with mode `0600`
and sweeps `photocraft-<pid>-<nonce>.scratch` when configuring the directory. It preserves
live-process files, other users' files, symlinks and unrelated names; unavailable process
enumeration leaves files untouched. PID reuse conservatively retains a stale file, and a
unique launch nonce avoids a collision. Normal Unix drops remove the file immediately.

The engine owns a shared, stoppable residency supervisor and joins it when its last session
is dropped. `photocraft-raster` only exposes synchronous measurement and bounded eviction
operations; standalone callers work without a background thread. `sysinfo` is native-only
with its default features disabled and only the `system` feature enabled.

An unavailable or full scratch disk retains resident pixels and reports the failure.
A failed read retains its backing extent and blocks editing, cache admission and saving
until the tile is readable again. Recovery can retry the original bytes after the storage
becomes available; zeros from a failed display read must never become saved pixels.

Scratch storage alone does not make every format, filter or renderer streamable. See
[viewport streaming](streaming-gpu.md) for rendering built on these guards and reservations.
