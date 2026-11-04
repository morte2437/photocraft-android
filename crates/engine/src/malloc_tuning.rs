//! glibc malloc tuning for the native binaries (#8743).
//!
//! With glibc's defaults, the memory of a closed document stays with the process: freeing a large
//! buffer raises glibc's dynamic mmap threshold (up to 32 MiB), so later tile buffers come from the
//! heap, whose freed pages glibc keeps for reuse instead of giving them back. A session that opened
//! and closed a 5400 × 7200 document held ~540 MiB afterwards; with `MALLOC_TRIM_THRESHOLD_` set it
//! went back to ~20 MiB, and the peak was lower too. Setting the threshold explicitly turns the
//! dynamic adjustment off (mallopt(3)), so large buffers are mapped and unmapped on their own and
//! the heap top is trimmed as it is freed. Mapping fresh pages on every call is slow, so the large
//! short-lived buffers come from `photocraft_raster::scratch` instead, which reuses them and
//! releases them when a document closes or the pool goes idle (#10479).
//!
//! glibc reads its tunables once, when the process starts, and changing the environment from Rust
//! needs `unsafe` (and comes too late), so [`apply`] re-executes the binary with the variable set:
//! the same process (PID), arguments, `argv[0]` and environment, on Linux with glibc only. A value
//! the user or a package already set, for this or another malloc tunable, is left alone, so setting
//! `MALLOC_TRIM_THRESHOLD_` to anything opts out. Programs PhotoCraft starts inherit the variable.

use std::ffi::OsString;

/// The glibc environment variable [`apply`] sets.
pub const TRIM_THRESHOLD_VAR: &str = "MALLOC_TRIM_THRESHOLD_";

/// Its value: glibc's own default trim threshold (128 KiB), now fixed instead of dynamic.
pub const TRIM_THRESHOLD: &str = "131072";

/// glibc malloc variables that mean someone tuned malloc themselves.
const MALLOC_VARS: [&str; 5] = [TRIM_THRESHOLD_VAR, "MALLOC_MMAP_THRESHOLD_", "MALLOC_TOP_PAD_", "MALLOC_MMAP_MAX_", "MALLOC_ARENA_MAX"];

/// Whether a start-up should re-execute with [`TRIM_THRESHOLD_VAR`] set, given the environment:
/// only when no malloc variable or `glibc.malloc.*` tunable is set (which also ends the re-exec).
pub fn needs_restart(var: impl Fn(&str) -> Option<OsString>) -> bool {
    if MALLOC_VARS.iter().any(|v| var(v).is_some()) {
        return false;
    }
    !var("GLIBC_TUNABLES").is_some_and(|t| t.to_string_lossy().contains("glibc.malloc."))
}

/// Re-execute this binary with [`TRIM_THRESHOLD_VAR`] set when [`needs_restart`] says so. Call it
/// first in `main`, before any thread starts. It returns only when nothing needed to change or the
/// re-exec failed, and then the process carries on with glibc's defaults.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn apply() {
    use std::os::unix::process::CommandExt;
    if !needs_restart(|k| std::env::var_os(k)) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut args = std::env::args_os();
    let mut cmd = std::process::Command::new(exe);
    if let Some(argv0) = args.next() {
        cmd.arg0(argv0);
    }
    // `exec` replaces the process and only returns on failure.
    let err = cmd.args(args).env(TRIM_THRESHOLD_VAR, TRIM_THRESHOLD).exec();
    eprintln!("PhotoCraft: could not restart with {TRIM_THRESHOLD_VAR} set ({err}); memory of closed documents may not be returned to the system");
}

/// Nothing to tune off glibc.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn apply() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn restarts_only_without_malloc_tuning() {
        assert!(needs_restart(env(&[])));
        assert!(needs_restart(env(&[("HOME", "/home/a"), ("GLIBC_TUNABLES", "glibc.cpu.hwcaps=-AVX2")])));
        assert!(!needs_restart(env(&[(TRIM_THRESHOLD_VAR, TRIM_THRESHOLD)])), "the re-executed process stops");
        assert!(!needs_restart(env(&[(TRIM_THRESHOLD_VAR, "")])), "any value opts out");
        assert!(!needs_restart(env(&[("MALLOC_MMAP_THRESHOLD_", "1048576")])));
        assert!(!needs_restart(env(&[("MALLOC_ARENA_MAX", "2")])));
        assert!(!needs_restart(env(&[("GLIBC_TUNABLES", "glibc.malloc.trim_threshold=0")])));
    }
}
