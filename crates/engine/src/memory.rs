//! Physical memory of the machine, shared by the engine's memory budgets (Photomerge) and the
//! shell's GPU budgets.

/// Physical memory of this machine in bytes, if known. `PHOTOCRAFT_RAM_MB` overrides it (to
/// simulate a smaller machine).
pub fn physical_memory() -> Option<u64> {
    static RAM: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *RAM.get_or_init(|| {
        if let Some(mb) = std::env::var("PHOTOCRAFT_RAM_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()) {
            return mb.checked_mul(1 << 20);
        }
        detect_physical_memory()
    })
}

/// Physical memory as the operating system reports it, ignoring `PHOTOCRAFT_RAM_MB` and not
/// cached. `None` where nothing reports it (the web build, other platforms).
pub fn detect_physical_memory() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
        String::from_utf8(out.stdout).ok()?.trim().parse().ok()
    }
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
        kb.checked_mul(1024)
    }
    #[cfg(target_os = "freebsd")]
    {
        let out = std::process::Command::new("/sbin/sysctl").args(["-n", "hw.physmem"]).output().ok()?;
        String::from_utf8(out.stdout).ok()?.trim().parse().ok()
    }
    #[cfg(windows)]
    {
        winsafe::GlobalMemoryStatusEx().map(|m| m.ullTotalPhys).ok().filter(|b| *b > 0)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
    {
        None
    }
}
