//! How much memory this process can still use, so an import too large for the machine is
//! refused with an error instead of ending the app. Linux overcommits: an allocation that can't
//! be backed still succeeds, and the out-of-memory killer ends the process (and every open
//! document) when its pages are written, so a fallible allocation alone never fails there.

use crate::IoError;

/// Memory left free beside what an import needs: for the app, its display and the system.
pub const RESERVE: u64 = 512 << 20;

/// Bytes this process can still use without running out, or `None` where that isn't known. On
/// Linux: the free RAM the kernel can hand out (`MemAvailable`, capped by the process's cgroup
/// v2 `memory.max` and those above it) plus free swap (capped by the cgroup's `memory.swap.max` and those above it).
/// Elsewhere `None`: Windows refuses an allocation past its commit limit, which the importers
/// already report as an error, and macOS pages to disk as it needs.
pub fn available() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Refuses to open a `w` × `h` px image whose layer and decode buffers take `needed` bytes when
/// `available` bytes (see [`available`]) can't hold them with [`RESERVE`] to spare.
pub(crate) fn check_fits(w: u32, h: u32, needed: u64, available: Option<u64>) -> Result<(), IoError> {
    let Some(available) = available else { return Ok(()) };
    if needed.checked_add(RESERVE).is_some_and(|n| n <= available) {
        return Ok(());
    }
    let gb = |b: u64| b as f64 / 1e9;
    Err(IoError::Unsupported(format!(
        "not enough memory to open this {w} × {h} px image: it needs about {:.1} GB and {:.1} GB is free",
        gb(needed),
        gb(available)
    )))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};

    pub(super) fn available() -> Option<u64> {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let ram = meminfo(&info, "MemAvailable:")?;
        let swap = meminfo(&info, "SwapFree:").unwrap_or(0);
        let Some(dir) = cgroup_dir() else { return Some(ram.saturating_add(swap)) };
        let (ram_room, swap_room) = cgroup_room(&dir);
        Some(ram.min(ram_room.unwrap_or(u64::MAX)).saturating_add(swap.min(swap_room.unwrap_or(u64::MAX))))
    }

    /// A `/proc/meminfo` field in bytes.
    pub(super) fn meminfo(info: &str, field: &str) -> Option<u64> {
        let kb: u64 = info.lines().find_map(|l| l.strip_prefix(field))?.trim().trim_end_matches("kB").trim().parse().ok()?;
        kb.checked_mul(1024)
    }

    /// The process's cgroup v2 directory.
    fn cgroup_dir() -> Option<PathBuf> {
        let groups = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        let path = groups.lines().find_map(|l| l.strip_prefix("0::"))?;
        Some(Path::new("/sys/fs/cgroup").join(path.trim().trim_start_matches('/')))
    }

    /// Memory and swap left under the limits of `dir` and every cgroup above it (`None`: no
    /// limit). Both limits apply hierarchically. `memory.current` includes page cache, which the
    /// kernel reclaims before a cgroup hits `memory.max`, so its inactive part counts as free.
    pub(super) fn cgroup_room(dir: &Path) -> (Option<u64>, Option<u64>) {
        let read = |d: &Path, f: &str| std::fs::read_to_string(d.join(f)).ok();
        let room = |d: &Path, max: &str, current: &str, reclaimable: u64| -> Option<u64> {
            let max = read(d, max)?.trim().parse::<u64>().ok()?;
            let current = read(d, current).and_then(|c| c.trim().parse::<u64>().ok()).unwrap_or(0);
            Some(max.saturating_sub(current.saturating_sub(reclaimable)))
        };
        let inactive_file = |d: &Path| read(d, "memory.stat").and_then(|s| stat(&s, "inactive_file")).unwrap_or(0);
        let swap = dir.ancestors().take(64).filter_map(|d| room(d, "memory.swap.max", "memory.swap.current", 0)).min();
        let ram = dir.ancestors().take(64).filter_map(|d| room(d, "memory.max", "memory.current", inactive_file(d))).min();
        (ram, swap)
    }

    /// A `memory.stat` field (`<name> <bytes>` per line).
    fn stat(stat: &str, field: &str) -> Option<u64> {
        stat.lines().find_map(|l| l.strip_prefix(field)?.strip_prefix(' '))?.trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_import_that_does_not_fit_is_refused() {
        let gb = 1u64 << 30;
        assert!(check_fits(10, 10, 4 * gb, None).is_ok(), "unknown: no check");
        assert!(check_fits(10, 10, 4 * gb, Some(8 * gb)).is_ok());
        assert!(check_fits(10, 10, 4 * gb, Some(4 * gb + RESERVE)).is_ok());
        let e = check_fits(100_000, 100_000, 30 * gb, Some(16 * gb)).unwrap_err().to_string();
        assert!(e.contains("not enough memory") && e.contains("100000 × 100000") && e.contains("32.2 GB"), "{e}");
        assert!(check_fits(10, 10, 4 * gb, Some(4 * gb)).is_err(), "no room for the reserve");
        assert!(check_fits(10, 10, u64::MAX, Some(u64::MAX)).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_memory_is_read_from_meminfo_and_the_cgroup() {
        let info = "MemTotal:       24314352 kB\nMemAvailable:   12626632 kB\nSwapFree:          1024 kB\n";
        assert_eq!(linux::meminfo(info, "MemAvailable:"), Some(12_626_632 * 1024));
        assert_eq!(linux::meminfo(info, "SwapFree:"), Some(1024 * 1024));
        assert_eq!(linux::meminfo(info, "Missing:"), None);
        assert_eq!(linux::meminfo("MemAvailable: lots kB", "MemAvailable:"), None);

        let root = std::env::temp_dir().join(format!("photocraft-cgroup-{}", std::process::id()));
        let leaf = root.join("app.slice/app.scope");
        std::fs::create_dir_all(&leaf).unwrap();
        let write = |d: &Path, f: &str, v: &str| std::fs::write(d.join(f), v).unwrap();
        write(&root, "memory.max", "8589934592\n");
        write(&root, "memory.current", "2147483648\n");
        write(&leaf, "memory.max", "max\n");
        write(&leaf, "memory.current", "1073741824\n");
        write(&leaf, "memory.swap.max", "1073741824\n");
        write(&leaf, "memory.swap.current", "0\n");
        assert_eq!(linux::cgroup_room(&leaf), (Some(6 << 30), Some(1 << 30)), "the parent's limit applies");
        write(&root, "memory.swap.max", "1073741824\n");
        write(&root, "memory.swap.current", "805306368\n");
        write(&leaf, "memory.swap.max", "max\n");
        assert_eq!(linux::cgroup_room(&leaf).1, Some(256 << 20), "the parent's remaining swap applies to an unlimited leaf");
        write(&root, "memory.swap.max", "0\n");
        assert_eq!(linux::cgroup_room(&leaf).1, Some(0), "a parent can forbid swap");
        write(&root, "memory.swap.max", "1073741824\n");
        write(&leaf, "memory.swap.max", "134217728\n");
        assert_eq!(linux::cgroup_room(&leaf).1, Some(128 << 20), "the tighter leaf swap limit wins");
        write(&leaf, "memory.max", "1610612736\n");
        assert_eq!(linux::cgroup_room(&leaf).0, Some(512 << 20), "the tighter limit wins");
        write(&leaf, "memory.max", "2147483648\n");
        write(&leaf, "memory.current", "2147483648\n");
        write(&leaf, "memory.stat", "anon 268435456\nfile 1879048192\ninactive_file_x 1\ninactive_file 1610612736\n");
        assert_eq!(linux::cgroup_room(&leaf).0, Some(1610612736), "inactive page cache is reclaimed before the limit");
        write(&leaf, "memory.stat", "anon 268435456\n");
        assert_eq!(linux::cgroup_room(&leaf).0, Some(0), "no inactive_file: memory.current is all in use");
        std::fs::remove_dir_all(&root).unwrap();
        assert!(available().is_some());
    }

    #[cfg(target_os = "linux")]
    use std::path::Path;
}
