//! Platform measurements of the memory this process can use, shared by the tile residency
//! budget ([`super::measure`]) and the import admission budget (`photocraft_io::memory`), so
//! there is one detector of available RAM.
//!
//! - Linux: `/proc/meminfo` `MemAvailable`, capped by the process's memory cgroup (v2
//!   `memory.max`/`memory.high` and v1 `memory.limit_in_bytes`/`memsw`, with every visible
//!   ancestor); a cgroup that exists but cannot be read fails closed.
//! - Windows: `GlobalMemoryStatusEx` available physical memory and commit headroom.
//! - macOS: the reclaimable pool from the VM counters (free, file-backed, purgeable).
//! - Elsewhere (including the web): unknown.
//!
//! Swap is reported separately and never counted as available RAM.

/// Memory this process can use now, distinguishing physical RAM from swap and commit headroom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SystemMemory {
    /// Available physical RAM in bytes, bounded by applicable container or process limits. Free
    /// swap is excluded. `Some(0)` is zero capacity, not unknown memory.
    pub available_ram: Option<u64>,
    /// Available swap headroom in bytes, if known.
    pub swap_free: Option<u64>,
    /// System or process commit headroom in bytes, if measurable.
    pub commit_headroom: Option<u64>,
    /// A memory constraint exists but could not be read, so `available_ram` is the fail-closed
    /// `Some(0)` rather than a measurement. Admission refuses on it; the tile residency budget
    /// falls back to the host figure instead, because a zero budget would stall every operation.
    pub constraint_unreadable: bool,
}

impl SystemMemory {
    /// Unknown memory measurement.
    pub const UNKNOWN: Self = Self { available_ram: None, swap_free: None, commit_headroom: None, constraint_unreadable: false };
}

/// Measures [`SystemMemory`] from the platform provider.
pub fn current() -> SystemMemory {
    #[cfg(target_os = "linux")]
    {
        linux::snapshot()
    }
    #[cfg(windows)]
    {
        windows_mem::snapshot()
    }
    #[cfg(target_os = "macos")]
    {
        macos_mem::snapshot()
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        fallback_mem::snapshot()
    }
}

/// Bytes this process can still use without running out, counting free swap, or `None` where
/// that isn't known. On Linux: [`current`]'s available RAM plus free swap (both capped by the
/// memory cgroup). Elsewhere `None`: Windows refuses an allocation past its commit limit, which
/// the importers already report as an error, and macOS pages to disk as it needs.
pub fn available_with_swap() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
pub(super) mod linux {
    use super::SystemMemory;
    use std::path::{Path, PathBuf};

    /// cgroup v1 reports `PAGE_COUNTER_MAX * page_size` for "no limit", which is always close to
    /// `i64::MAX`. Any limit at or above 4 EiB is treated as unlimited.
    const V1_UNLIMITED: u64 = 1 << 62;

    /// Bound on ancestor walks; cgroup hierarchies are far shallower than this.
    const MAX_CGROUP_DEPTH: usize = 128;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Limit {
        Missing,
        Unlimited,
        Value(u64),
        Invalid,
    }

    /// What the process's memory cgroup says, independent of host `/proc/meminfo`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum CgroupLimits {
        /// No memory controller in this process's hierarchy: host memory applies.
        NotConstrained,
        /// Headroom under the memory controller; `None` means that dimension is unlimited.
        Limited { ram: Option<u64>, swap: Option<u64> },
        /// A memory controller exists but its mount, path or limit could not be read. Callers
        /// must fail closed instead of falling back to unrestricted host memory.
        Unreadable,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CgroupDir {
        leaf: PathBuf,
        /// The visible root of the walk (the mount point).
        root: PathBuf,
        /// Whether `root` is the hierarchy's own root rather than a subtree exposed by a mount.
        /// Only the hierarchy root is exempt from exposing its controller interface files.
        hierarchy_root: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Membership<'a> {
        /// Unified (v2) hierarchy path (`0::<path>`).
        v2: Option<&'a str>,
        /// v1 hierarchy path for the `memory` controller.
        v1_memory: Option<&'a str>,
    }

    /// Result of reading `/proc/self/cgroup`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum MembershipParse<'a> {
        Parsed(Membership<'a>),
        /// A non-empty line could not be understood, so the absence of a memory controller cannot
        /// be established.
        Malformed,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct Mount {
        pub(super) root: String,
        pub(super) mount_point: String,
        pub(super) fstype: String,
        pub(super) options: String,
        pub(super) super_options: String,
    }

    /// Measures available RAM and free swap from `/proc/meminfo`, capped by the process's memory
    /// cgroup. Unreadable cgroup constraints fail closed as zero available memory rather than
    /// falling back to unrestricted host memory.
    pub(super) fn snapshot() -> SystemMemory {
        snapshot_at(Path::new("/proc"), Path::new("/sys/fs/cgroup"))
    }

    /// Provider body with explicit roots, so tests exercise the real filesystem parsing without
    /// global mocks.
    pub(super) fn snapshot_at(proc_root: &Path, sys_root: &Path) -> SystemMemory {
        let info = std::fs::read_to_string(proc_root.join("meminfo")).ok();
        let host_ram = info.as_deref().and_then(|i| meminfo(i, "MemAvailable:"));
        let host_swap = info.as_deref().and_then(|i| meminfo(i, "SwapFree:"));

        let (cgroup_ram, cgroup_swap) = match cgroup_limits(proc_root, sys_root) {
            CgroupLimits::NotConstrained => (None, None),
            CgroupLimits::Limited { ram, swap } => (ram, swap),
            CgroupLimits::Unreadable => return SystemMemory { available_ram: Some(0), swap_free: Some(0), commit_headroom: None, constraint_unreadable: true },
        };

        let available_ram = match (host_ram, cgroup_ram) {
            (Some(host), Some(cgroup)) => Some(host.min(cgroup)),
            (Some(host), None) => Some(host),
            (None, Some(cgroup)) => Some(cgroup),
            (None, None) => None,
        };
        let swap_free = match (host_swap, cgroup_swap) {
            (Some(host), Some(cgroup)) => Some(host.min(cgroup)),
            (Some(host), None) => Some(host),
            (None, Some(cgroup)) => Some(cgroup),
            (None, None) => None,
        };
        SystemMemory { available_ram, swap_free, commit_headroom: None, constraint_unreadable: false }
    }

    pub(super) fn available() -> Option<u64> {
        available_at(Path::new("/proc"), Path::new("/sys/fs/cgroup"))
    }

    /// Legacy RAM-plus-swap helper with explicit roots. Its meaning is deliberately unchanged:
    /// the adaptive budget excludes swap, but flat and layered TIFF conversion still count it
    /// (now bounded by the cgroup figures derived above, including v1 `memsw`).
    pub(super) fn available_at(proc_root: &Path, sys_root: &Path) -> Option<u64> {
        let snap = snapshot_at(proc_root, sys_root);
        snap.available_ram.map(|ram| ram.saturating_add(snap.swap_free.unwrap_or(0)))
    }

    /// A `/proc/meminfo` field in bytes.
    pub(super) fn meminfo(info: &str, field: &str) -> Option<u64> {
        let kb: u64 = info.lines().find_map(|l| l.strip_prefix(field))?.trim().trim_end_matches("kB").trim().parse().ok()?;
        kb.checked_mul(1024)
    }

    /// Resolves the memory cgroup from `/proc/self/cgroup` plus `/proc/self/mountinfo`.
    ///
    /// v1 is preferred when the `memory` controller appears there; otherwise the unified v2
    /// hierarchy is used. A memory controller that cannot be resolved or read, or membership that
    /// cannot be parsed, is [`CgroupLimits::Unreadable`], never host memory. Ancestors above a
    /// cgroup namespace's root are not visible to an unprivileged reader (the kernel still
    /// enforces them), so only visible ancestors can be accounted for. The widest visible view of
    /// the hierarchy is used, not merely the deepest matching mount point.
    pub(super) fn cgroup_limits(proc_root: &Path, sys_root: &Path) -> CgroupLimits {
        let Ok(groups) = std::fs::read_to_string(proc_root.join("self/cgroup")) else {
            // Without membership we cannot prove that no memory cgroup applies.
            return CgroupLimits::Unreadable;
        };
        let membership = match parse_membership(&groups) {
            MembershipParse::Parsed(membership) => membership,
            MembershipParse::Malformed => return CgroupLimits::Unreadable,
        };
        let mountinfo = std::fs::read_to_string(proc_root.join("self/mountinfo")).ok();
        // A sandbox with no cgroup filesystem mounted and no `/sys/fs/cgroup` at all (Flatpak) or
        // only an empty mount-point directory left by `sysfs` shows the process its membership path
        // but no interface files, so no limit is observable. Failing closed there reported 0 bytes
        // free and refused every PSD on a machine with tens of GB. Only a present non-empty or
        // unreadable directory takes the guarded path below: a stat or read_dir that fails for
        // another reason (permissions, non-directory) leaves a mounted hierarchy possibly present,
        // so that case stays fail-closed.
        if sys_root_absent_or_empty(sys_root) && mountinfo.as_deref().is_some_and(lists_mounts_without_cgroup) {
            return CgroupLimits::NotConstrained;
        }

        if let Some(path) = membership.v1_memory {
            let Some(dir) = resolve_dir(sys_root, mountinfo.as_deref(), path, "cgroup", Some("memory")) else {
                return CgroupLimits::Unreadable;
            };
            return v1_limits(&dir);
        }
        if let Some(path) = membership.v2 {
            let Some(dir) = resolve_dir(sys_root, mountinfo.as_deref(), path, "cgroup2", None) else {
                return CgroupLimits::Unreadable;
            };
            return v2_limits(&dir);
        }
        CgroupLimits::NotConstrained
    }

    /// Parses `/proc/self/cgroup`.
    ///
    /// Kernel format is `<hierarchy-id>:<comma-separated controllers>:<absolute path>`; the
    /// unified hierarchy uses `0::<path>`. A line that does not have that shape makes the whole
    /// file untrustworthy: returning "no memory controller" from a partial parse would silently
    /// fall back to unrestricted host memory, so an empty or malformed file is reported as
    /// malformed instead.
    fn parse_membership(text: &str) -> MembershipParse<'_> {
        let mut membership = Membership { v2: None, v1_memory: None };
        let mut saw_line = false;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            saw_line = true;
            let mut fields = line.splitn(3, ':');
            let (Some(hierarchy), Some(controllers), Some(path)) = (fields.next(), fields.next(), fields.next()) else {
                return MembershipParse::Malformed;
            };
            let Ok(hierarchy) = hierarchy.parse::<u32>() else { return MembershipParse::Malformed };
            if !path.starts_with('/') {
                return MembershipParse::Malformed;
            }
            if hierarchy == 0 {
                if !controllers.is_empty() {
                    return MembershipParse::Malformed;
                }
                membership.v2.get_or_insert(path);
            } else if controllers.is_empty() {
                return MembershipParse::Malformed;
            } else if controllers.split(',').any(|c| c.trim() == "memory") {
                membership.v1_memory.get_or_insert(path);
            }
        }
        if !saw_line {
            return MembershipParse::Malformed;
        }
        MembershipParse::Parsed(membership)
    }

    /// Whether `/sys/fs/cgroup` is absent (`ErrorKind::NotFound`) or a readable empty directory.
    ///
    /// Permission errors, non-directory paths, and unreadable directories return `false` and stay
    /// fail-closed.
    pub(super) fn sys_root_absent_or_empty(sys_root: &Path) -> bool {
        match std::fs::metadata(sys_root) {
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            Ok(meta) => meta.is_dir() && std::fs::read_dir(sys_root).is_ok_and(|mut entries| entries.next().is_none()),
        }
    }

    /// Parses `/proc/self/mountinfo` into the mount fields used to locate a cgroup hierarchy.
    ///
    /// Malformed lines are skipped; callers fail closed when no matching mount is found.
    pub(super) fn parse_mountinfo(text: &str) -> Vec<Mount> {
        let mut mounts = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let Some(separator) = fields.iter().position(|f| *f == "-") else { continue };
            let super_options = fields.get(separator + 3).copied().or_else(|| {
                // `show_mountinfo` writes `<fstype> <source> <super_options>`; an empty source
                // (e.g. a bubblewrap tmpfs mount) leaves two spaces between `fstype` and options.
                let after = line.split_once(" - ")?.1.trim_start();
                let fstype = fields.get(separator + 1).copied()?;
                after.strip_prefix(fstype)?.starts_with("  ").then(|| fields.get(separator + 2).copied()).flatten()
            });
            let (Some(root), Some(point), Some(fstype), Some(super_options)) =
                (fields.get(3).copied(), fields.get(4).copied(), fields.get(separator + 1).copied(), super_options)
            else {
                continue;
            };
            let options = fields.get(5).copied().unwrap_or("");
            mounts.push(Mount {
                root: unescape(root),
                mount_point: unescape(point),
                fstype: fstype.to_string(),
                options: options.to_string(),
                super_options: super_options.to_string(),
            });
        }
        mounts
    }

    /// Finds the mount for `fstype`/`controller` and joins the process's cgroup path to it.
    ///
    /// Reads `/proc/self/mountinfo` (dedicated controller mounts, bind mounts and non-root mount
    /// roots). If that file cannot be read, the conventional `/sys/fs/cgroup[/<controller>]`
    /// layout is the only guess available. When mountinfo *is* readable, a missing matching mount
    /// fails closed instead of guessing another mount's hierarchy: a wrong directory must never
    /// become unrestricted host memory.
    ///
    /// A bind mount of a subtree has a deeper `root` and hides the cgroup ancestors above that
    /// subtree, so the *widest* matching view is preferred: the mount whose cgroup root is
    /// highest in the hierarchy (fewest path components), not the longest mount point. A deep
    /// bind mount must not be allowed to hide a limit on an ancestor that is visible through the
    /// hierarchy's real mount.
    fn resolve_dir(sys_root: &Path, mountinfo: Option<&str>, membership: &str, fstype: &str, controller: Option<&str>) -> Option<CgroupDir> {
        let membership = unescape(membership.trim());
        let Some(info) = mountinfo else {
            let root = match controller {
                Some(c) => sys_root.join(c),
                None => sys_root.to_path_buf(),
            };
            let leaf = join_membership(&root, &membership)?;
            // The conventional layout mounts each hierarchy at its own root.
            return leaf.is_dir().then_some(CgroupDir { leaf, root, hierarchy_root: true });
        };

        let mut best: Option<(usize, usize, CgroupDir)> = None;
        for mount in parse_mountinfo(info) {
            if mount.fstype != fstype {
                continue;
            }
            if controller.is_some_and(|c| !has_option(&mount.options, c) && !has_option(&mount.super_options, c)) {
                continue;
            }
            let Some(relative) = strip_mount_root(&mount.root, &membership) else { continue };
            let root = normalize_root(&mount.mount_point);
            let Some(leaf) = join_membership(&root, relative) else { continue };
            if !leaf.is_dir() {
                continue;
            }
            let key = (cgroup_path_depth(&mount.root), mount.mount_point.len());
            if best.as_ref().is_none_or(|(depth, point, _)| key < (*depth, *point)) {
                let hierarchy_root = mount.root.trim_end_matches('/').is_empty();
                best = Some((key.0, key.1, CgroupDir { leaf, root, hierarchy_root }));
            }
        }
        best.map(|(_, _, dir)| dir)
    }

    /// A cgroup (v1 or v2) filesystem.
    fn is_cgroup_mount(mount: &Mount) -> bool {
        mount.fstype == "cgroup" || mount.fstype == "cgroup2"
    }

    /// `mountinfo` parsed to at least one mount, none of them a cgroup filesystem. An empty or
    /// garbled file parses to no mounts at all, which proves nothing about cgroups; a live kernel
    /// always lists the process's own root, so only a real list can show a cgroup mount is absent.
    /// Every non-empty line must parse: a skipped truncated line could be the cgroup mount.
    fn lists_mounts_without_cgroup(info: &str) -> bool {
        let mounts = parse_mountinfo(info);
        let lines = info.lines().filter(|l| !l.trim().is_empty()).count();
        !mounts.is_empty() && mounts.len() == lines && !mounts.iter().any(is_cgroup_mount)
    }

    /// Trims trailing slashes so `Path` equality holds while walking ancestors, keeping `/`.
    fn normalize_root(point: &str) -> PathBuf {
        let trimmed = point.trim_end_matches('/');
        PathBuf::from(if trimmed.is_empty() { "/" } else { trimmed })
    }

    /// Joins a cgroup path below a visible hierarchy root, normalizing `.` components and
    /// rejecting parent traversal so a malformed path cannot escape the mount.
    fn join_membership(root: &Path, path: &str) -> Option<PathBuf> {
        let mut leaf = root.to_path_buf();
        for component in Path::new(path.trim_start_matches('/')).components() {
            match component {
                std::path::Component::Normal(part) => leaf.push(part),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
            }
        }
        Some(leaf)
    }

    /// Number of path components in a mount's cgroup root; `/` or an empty root is zero.
    fn cgroup_path_depth(root: &str) -> usize {
        Path::new(root).components().filter(|c| matches!(c, std::path::Component::Normal(_))).count()
    }

    /// Whether a v2 mount's `cgroup.controllers` exposes the memory controller.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum V2Controller {
        /// `cgroup.controllers` lists `memory`.
        Available,
        /// `cgroup.controllers` is readable and omits `memory`, so no cgroup memory limit exists.
        Absent,
        /// `cgroup.controllers` could not be read, so absence cannot be established.
        Unknown,
    }

    /// Whether the cgroup v2 hierarchy at `root` exposes the memory controller. An unreadable
    /// `cgroup.controllers` is `Unknown`, so its absence cannot be proven.
    fn v2_controller(root: &Path) -> V2Controller {
        match std::fs::read_to_string(root.join("cgroup.controllers")) {
            Ok(text) => {
                if text.split_whitespace().any(|controller| controller == "memory") {
                    V2Controller::Available
                } else {
                    V2Controller::Absent
                }
            }
            Err(_) => V2Controller::Unknown,
        }
    }

    /// Whether a cgroup's `cgroup.subtree_control` enables memory for its children.
    ///
    /// Interface files are created by the parent, so this decides whether a child *must* expose
    /// `memory.max`: a readable control file without memory means the child legitimately has no
    /// limit, while an unreadable one means that cannot be established.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Delegation {
        Enabled,
        Disabled,
        Unknown,
    }

    /// Whether `parent` enables the memory controller for its children in
    /// `cgroup.subtree_control`. An unreadable file is `Unknown`, not "disabled".
    fn v2_delegation(parent: &Path) -> Delegation {
        match std::fs::read_to_string(parent.join("cgroup.subtree_control")) {
            Ok(text) => {
                if text.split_whitespace().any(|controller| controller.trim_start_matches('+') == "memory") {
                    Delegation::Enabled
                } else {
                    Delegation::Disabled
                }
            }
            Err(_) => Delegation::Unknown,
        }
    }

    /// cgroup v2: `memory.max`/`memory.current`, reclaiming `inactive_file`, plus
    /// `memory.swap.max`/`memory.swap.current`. Every visible ancestor limit applies.
    ///
    /// A missing `memory.max` is not always an error: the hierarchy root is exempt from resource
    /// control, and a child has the file only when its parent delegates the controller. A missing
    /// file where the parent *does* delegate memory, or where delegation cannot be read, means the
    /// visible view is incomplete and is reported as unreadable instead of unrestricted.
    fn v2_limits(dir: &CgroupDir) -> CgroupLimits {
        if !dir.leaf.starts_with(&dir.root) || !dir.leaf.is_dir() {
            return CgroupLimits::Unreadable;
        }
        let controller = v2_controller(&dir.root);

        let mut nodes = Vec::new();
        let mut node = Some(dir.leaf.as_path());
        while let Some(path) = node {
            nodes.push(path);
            if path == dir.root.as_path() {
                break;
            }
            if nodes.len() > MAX_CGROUP_DEPTH {
                return CgroupLimits::Unreadable;
            }
            node = path.parent();
        }
        if nodes.last().is_none_or(|path| *path != dir.root.as_path()) {
            return CgroupLimits::Unreadable;
        }
        nodes.reverse();

        let mut ram: Option<u64> = None;
        let mut swap: Option<u64> = None;
        let mut enabled = false;
        for (index, path) in nodes.iter().enumerate() {
            let expected = if index == 0 {
                if dir.hierarchy_root {
                    // The hierarchy root is exempt from resource control and may have no
                    // `memory.max` at all.
                    None
                } else if controller == V2Controller::Absent {
                    Some(Delegation::Disabled)
                } else {
                    // A mounted subtree root must expose the controller when it is enabled for
                    // that cgroup; without the parent's control file its absence is unprovable.
                    Some(Delegation::Unknown)
                }
            } else {
                Some(v2_delegation(nodes[index - 1]))
            };
            match read_limit(&path.join("memory.max")) {
                Limit::Value(max) => {
                    enabled = true;
                    let Some(current) = read_u64(&path.join("memory.current")) else { return CgroupLimits::Unreadable };
                    let inactive = read_stat(&path.join("memory.stat"), "inactive_file");
                    let room = max.saturating_sub(current.saturating_sub(inactive));
                    ram = Some(ram.map_or(room, |r| r.min(room)));
                }
                Limit::Unlimited => enabled = true,
                Limit::Invalid => return CgroupLimits::Unreadable,
                Limit::Missing => match expected {
                    None | Some(Delegation::Disabled) => {}
                    Some(Delegation::Enabled | Delegation::Unknown) => return CgroupLimits::Unreadable,
                },
            }
            match read_limit(&path.join("memory.swap.max")) {
                Limit::Value(max) => {
                    enabled = true;
                    let Some(current) = read_u64(&path.join("memory.swap.current")) else { return CgroupLimits::Unreadable };
                    let room = max.saturating_sub(current);
                    swap = Some(swap.map_or(room, |r| r.min(room)));
                }
                Limit::Unlimited => enabled = true,
                Limit::Invalid => return CgroupLimits::Unreadable,
                // Kernels before swap accounting expose `memory.max` without `memory.swap.*`;
                // absence there means no swap limit at this cgroup, not an unreadable constraint.
                Limit::Missing => {}
            }
        }

        if enabled {
            return CgroupLimits::Limited { ram, swap };
        }
        match controller {
            // The controller is genuinely absent, or available but not enabled anywhere on the
            // visible path (the root may normally have no interface files): no cgroup limit.
            V2Controller::Absent | V2Controller::Available => CgroupLimits::NotConstrained,
            // No readable limit file and no metadata proving the controller absent.
            V2Controller::Unknown => CgroupLimits::Unreadable,
        }
    }

    /// cgroup v1: `memory.limit_in_bytes`/`memory.usage_in_bytes` (inactive cache reclaimable)
    /// and the separate `memory.memsw` memory-plus-swap bound.
    ///
    /// A memsw limit also caps RAM because memory usage counts toward it, but its remaining room
    /// is not swap that can be added on top of a different ancestor's RAM headroom. Each
    /// ancestor's swap allowance is `memsw limit - memory limit - swap already used`, and those
    /// allowances are combined separately. Subtracting one global combined headroom from one
    /// global RAM headroom would invent swap room from different ancestors.
    fn v1_limits(dir: &CgroupDir) -> CgroupLimits {
        if !dir.leaf.starts_with(&dir.root) || !dir.leaf.is_dir() {
            return CgroupLimits::Unreadable;
        }
        let mut ram: Option<u64> = None;
        let mut swap: Option<u64> = None;
        let mut enabled = false;
        let mut node = Some(dir.leaf.as_path());
        let mut depth = 0;
        while let Some(path) = node {
            if depth >= MAX_CGROUP_DEPTH {
                return CgroupLimits::Unreadable;
            }

            // `None` means unlimited at this ancestor; the raw usage feeds the swap bound.
            let mut memory_usage = None;
            let memory_limit = match read_limit(&path.join("memory.limit_in_bytes")) {
                Limit::Missing | Limit::Invalid => return CgroupLimits::Unreadable,
                Limit::Unlimited => {
                    enabled = true;
                    None
                }
                Limit::Value(bytes) if bytes >= V1_UNLIMITED => {
                    enabled = true;
                    None
                }
                Limit::Value(bytes) => {
                    enabled = true;
                    let Some(usage) = read_u64(&path.join("memory.usage_in_bytes")) else { return CgroupLimits::Unreadable };
                    memory_usage = Some(usage);
                    let inactive = std::fs::read_to_string(path.join("memory.stat"))
                        .ok()
                        .and_then(|s| stat(&s, "total_inactive_file").or_else(|| stat(&s, "inactive_file")))
                        .unwrap_or(0);
                    let room = bytes.saturating_sub(usage.saturating_sub(inactive));
                    ram = Some(ram.map_or(room, |r| r.min(room)));
                    Some(bytes)
                }
            };

            match read_limit(&path.join("memory.memsw.limit_in_bytes")) {
                Limit::Missing | Limit::Unlimited => {}
                Limit::Invalid => return CgroupLimits::Unreadable,
                Limit::Value(bytes) if bytes >= V1_UNLIMITED => {}
                Limit::Value(bytes) => {
                    let Some(memsw_usage) = read_u64(&path.join("memory.memsw.usage_in_bytes")) else {
                        return CgroupLimits::Unreadable;
                    };
                    // Memory plus swap is bounded by memsw, so its remaining room also bounds RAM.
                    let combined_room = bytes.saturating_sub(memsw_usage);
                    ram = Some(ram.map_or(combined_room, |r| r.min(combined_room)));
                    if let Some(memory_limit) = memory_limit {
                        // Swap can only use the part of memsw above the memory limit after the
                        // swap already in use; filling RAM to its own limit does not consume it.
                        let used_swap = memsw_usage.saturating_sub(memory_usage.unwrap_or(0));
                        let room = bytes.saturating_sub(memory_limit).saturating_sub(used_swap);
                        swap = Some(swap.map_or(room, |s| s.min(room)));
                    } else {
                        // With no RAM limit the remaining memsw room can all be consumed by RAM,
                        // so no swap room is guaranteed on top of the RAM allowance.
                        swap = Some(0);
                    }
                }
            }

            if path == dir.root.as_path() {
                break;
            }
            node = path.parent();
            depth += 1;
        }
        if !enabled {
            // The v1 hierarchy claims a memory controller but never exposed a limit file.
            return CgroupLimits::Unreadable;
        }
        CgroupLimits::Limited { ram, swap }
    }

    /// Reads a cgroup limit file: `max` is unlimited, a number is a byte limit, and a missing file
    /// is distinguished from one that cannot be read or parsed.
    fn read_limit(path: &Path) -> Limit {
        match std::fs::read_to_string(path) {
            Ok(text) => match text.trim() {
                "max" => Limit::Unlimited,
                value => match value.parse::<u64>() {
                    Ok(bytes) => Limit::Value(bytes),
                    Err(_) => Limit::Invalid,
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Limit::Missing,
            Err(_) => Limit::Invalid,
        }
    }

    /// Reads a whitespace-terminated unsigned value from a cgroup interface file.
    fn read_u64(path: &Path) -> Option<u64> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    /// Reads `field` from a cgroup `*.stat` file, treating a missing file or field as zero.
    fn read_stat(path: &Path, field: &str) -> u64 {
        std::fs::read_to_string(path).ok().and_then(|s| stat(&s, field)).unwrap_or(0)
    }

    /// Finds `field` in `key value` stat lines and parses its value.
    fn stat(s: &str, field: &str) -> Option<u64> {
        s.lines().find_map(|l| {
            let mut parts = l.split_whitespace();
            let key = parts.next()?;
            if key == field { parts.next()?.parse().ok() } else { None }
        })
    }

    /// Whether a comma-separated mount option list contains `controller`.
    fn has_option(options: &str, controller: &str) -> bool {
        options.split(',').any(|option| option.trim() == controller)
    }

    /// Strips a mount's root prefix from a cgroup path, returning the path relative to the mount.
    fn strip_mount_root<'a>(root: &str, path: &'a str) -> Option<&'a str> {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            return Some(path);
        }
        let rest = path.strip_prefix(root)?;
        if rest.is_empty() {
            return Some("");
        }
        rest.strip_prefix('/')
    }

    /// Decodes mountinfo/mount-path octal escapes (`\040` space, `\011` tab, `\134` backslash).
    fn unescape(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while let Some(&byte) = bytes.get(index) {
            if byte == b'\\'
                && let Some(digits) = bytes.get(index + 1..index + 4)
            {
                let code = digits.iter().try_fold(0u32, |acc, digit| {
                    let value = digit.checked_sub(b'0')?;
                    (value < 8).then_some(acc * 8 + u32::from(value))
                });
                if let Some(byte) = code.and_then(|code| u8::try_from(code).ok()) {
                    out.push(byte);
                    index += 4;
                    continue;
                }
            }
            out.push(byte);
            index += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Exposed for the Linux provider tests; the policy itself never calls this.
    #[cfg(test)]
    pub(super) fn membership_for_test(text: &str) -> Option<(Option<String>, Option<String>)> {
        match parse_membership(text) {
            MembershipParse::Parsed(membership) => Some((membership.v2.map(str::to_string), membership.v1_memory.map(str::to_string))),
            MembershipParse::Malformed => None,
        }
    }
}

#[cfg(windows)]
mod windows_mem {
    use super::SystemMemory;

    /// Measures available physical and commit memory through `GlobalMemoryStatusEx`. A failed
    /// query reports unknown memory rather than inventing a measurement.
    pub(super) fn snapshot() -> SystemMemory {
        match winsafe::GlobalMemoryStatusEx() {
            Ok(msx) => SystemMemory {
                available_ram: Some(msx.ullAvailPhys),
                swap_free: None,
                commit_headroom: Some(msx.ullAvailPageFile),
                constraint_unreadable: false,
            },
            Err(_) => SystemMemory::UNKNOWN,
        }
    }
}

/// Conservative reclaimable physical RAM from macOS VM counters, or `None` when the query failed.
///
/// `total` is installed physical memory (`sysctl` `HW_MEMSIZE`). The other three are the same
/// `host_statistics64` `vm_statistics64` fields that `sysinfo` derives:
///
/// - `free` = `free_memory` = `free_count - speculative_count` pages;
/// - `available` = `available_memory` = `active + inactive + free_count` pages;
/// - `used` = `used_memory` = `internal_page_count - purgeable_count + wire_count +
///   compressor_page_count` pages.
///
/// XNU documents `AVAILABLE_NON_COMPRESSED_MEMORY = active + inactive + free + speculative` as
/// "everything that is not either wired, compressed, or stolen"
/// (<https://github.com/apple-oss-distributions/xnu/blob/main/doc/vm/memorystatus_notify.md>).
/// That includes anonymous working sets, which can only be released by compressing or swapping
/// them, so on its own it is too optimistic for a budget that excludes swap.
///
/// `internal_page_count` counts anonymous pages and `external_page_count` counts file-backed
/// pages (<https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/vm_statistics.h>),
/// so `total - used` is the pool that can be handed out without evicting a working set:
/// `free + file-backed + purgeable`. That is close to the fully-reclaimable pool jetsam monitors
/// (`file-backed + free + purgeable`, same document). The smaller of the two derivations is
/// taken so a counter disagreement stays conservative, and the result is never below `free`:
/// pages that are already free are allocatable by definition, so a derived value below them
/// means the counters disagree and cannot be used to claim less.
///
/// Only a demonstrably failed query is unknown. A live host always has a non-zero installed size
/// and non-zero used memory (wired kernel pages alone), so either reading zero proves the
/// provider query failed. A genuine "no reclaimable pages" reading stays a known zero rather
/// than becoming the 2 GiB unknown-memory allowance.
#[cfg(any(target_os = "macos", test))]
fn macos_available_ram(total: u64, free: u64, available: u64, used: u64) -> Option<u64> {
    if total == 0 || used == 0 {
        return None;
    }
    let free = free.min(total);
    let reclaimable = total.saturating_sub(used).min(available).max(free);
    Some(reclaimable.min(total))
}

#[cfg(target_os = "macos")]
mod macos_mem {
    use super::SystemMemory;
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};

    /// Available physical memory through `sysinfo`'s safe API.
    ///
    /// On macOS that is `host_statistics64`'s `vm_statistics64` (see
    /// [`super::macos_available_ram`]). The estimate counts free pages, file-backed pages and
    /// purgeable pages, not anonymous working sets, wired memory, compressed memory or swap. A
    /// failed query reports unknown; a successful zero reading stays zero.
    pub(super) fn snapshot() -> SystemMemory {
        let system = match System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram())) {
            Ok(system) => system,
            Err(_) => return SystemMemory::UNKNOWN,
        };
        match super::macos_available_ram(system.total_memory(), system.free_memory(), system.available_memory(), system.used_memory()) {
            Some(ram) => SystemMemory { available_ram: Some(ram), swap_free: None, commit_headroom: None, constraint_unreadable: false },
            None => SystemMemory::UNKNOWN,
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod fallback_mem {
    use super::SystemMemory;

    /// Fallback provider for platforms without a validated memory-availability provider.
    ///
    /// Available memory is reported as unknown (`None`), which applies the conservative native
    /// fallback budget (2 GiB, approval above it) without inventing a measurement.
    pub(super) fn snapshot() -> SystemMemory {
        SystemMemory::UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_available_ram_counts_reclaimable_pages_not_free_only() {
        let gb = 1u64 << 30;
        // A failed provider query: no installed size, or no used memory on a live host.
        assert_eq!(macos_available_ram(0, 0, 0, 0), None, "an empty query is not a measurement");
        assert_eq!(macos_available_ram(0, 1 << 20, 4 * gb, 4 * gb), None, "no installed size: the sysctl query failed");
        assert_eq!(macos_available_ram(16 * gb, 1 << 20, 8 * gb, 0), None, "no used memory on a live host: the VM query failed");

        // A genuine zero-capacity reading stays zero, not unknown.
        assert_eq!(macos_available_ram(16 * gb, 0, 0, 16 * gb), Some(0), "everything is wired or compressed");

        // Low free memory beside a large file-backed cache: free alone would report ~0 and the
        // hard budget would collapse, while 12 GiB is reclaimable without touching a working set.
        assert_eq!(macos_available_ram(16 * gb, 100 << 20, 12 * gb, 4 * gb), Some(12 * gb));

        // A loaded machine: most memory is anonymous working set, which is not reclaimable, so
        // `available` (which counts it) does not raise the estimate above total - used.
        assert_eq!(macos_available_ram(16 * gb, 200 << 20, 8 * gb, 14 * gb), Some(2 * gb));

        // Counters that disagree: free pages are allocatable by definition, so never claim less.
        assert_eq!(macos_available_ram(16 * gb, 3 * gb, gb, 15 * gb), Some(3 * gb));

        // Never above installed memory.
        assert_eq!(macos_available_ram(gb, 2 * gb, 4 * gb, 1), Some(gb));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_provider_excludes_swap_and_commit_headroom() {
        let snap = current();
        assert_eq!(snap.swap_free, None, "the adaptive budget must not count swap");
        assert_eq!(snap.commit_headroom, None);
        // The numeric floor and zero-versus-unknown semantics are covered deterministically by
        // `macos_available_ram_counts_reclaimable_pages_not_free_only`; asserting a live `> 0`
        // here would be vacuous because a genuine zero is a valid reading.
    }

    #[cfg(target_os = "linux")]
    use std::path::{Path, PathBuf};

    #[cfg(target_os = "linux")]
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("photocraft-cgroup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc/self")).unwrap();
        std::fs::create_dir_all(root.join("sys/fs/cgroup")).unwrap();
        (root.join("proc"), root.join("sys/fs/cgroup"))
    }

    #[cfg(target_os = "linux")]
    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_memory_is_read_from_meminfo() {
        let info = "MemTotal:       24314352 kB\nMemAvailable:   12626632 kB\nSwapFree:          1024 kB\n";
        assert_eq!(linux::meminfo(info, "MemAvailable:"), Some(12_626_632 * 1024));
        assert_eq!(linux::meminfo(info, "SwapFree:"), Some(1024 * 1024));
        assert_eq!(linux::meminfo(info, "Missing:"), None);
        assert_eq!(linux::meminfo("MemAvailable: lots kB", "MemAvailable:"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cgroup_membership_distinguishes_v1_and_v2() {
        assert_eq!(linux::membership_for_test("0::/user.slice/session.scope\n"), Some((Some("/user.slice/session.scope".into()), None)));
        assert_eq!(linux::membership_for_test("4:memory:/docker/abc\n"), Some((None, Some("/docker/abc".into()))));
        assert_eq!(linux::membership_for_test("4:cpu,cpuacct:/user.slice\n5:memory:/a/b\n"), Some((None, Some("/a/b".into()))));
        assert_eq!(linux::membership_for_test("12:pids:/\n"), Some((None, None)), "no memory controller is demonstrably absent");
        assert_eq!(linux::membership_for_test("garbage\n"), None);
        assert_eq!(linux::membership_for_test(""), None, "an empty file cannot prove absence");
        assert_eq!(linux::membership_for_test("0:memory:/app\n"), None, "the unified hierarchy cannot name controllers");
        assert_eq!(linux::membership_for_test("4::/app\n"), None, "a v1 line must name a controller");
        assert_eq!(linux::membership_for_test("4:memory:relative\n"), None, "cgroup paths are absolute");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mountinfo_octal_escapes_are_decoded() {
        let (proc, sys) = fixture("escaped");
        write(&proc.join("self/cgroup"), "0::/app\n");
        let mount = sys.join("cg root");
        let escaped = mount.display().to_string().replace(' ', "\\040");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw,relatime - cgroup2 cgroup2 rw\n", escaped));
        write(&mount.join("app/memory.max"), "1073741824\n");
        write(&mount.join("app/memory.current"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None });
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_v2_cgroup_applies_mount_path_and_ancestor_limits() {
        let (proc, sys) = fixture("v2");
        write(&proc.join("self/cgroup"), "0::/app.scope\n");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw,nosuid,nodev,noexec,relatime - cgroup2 cgroup2 rw,nsdelegate\n", sys.display()));
        let leaf = sys.join("app.scope");
        write(&sys.join("memory.max"), "8589934592\n");
        write(&sys.join("memory.current"), "2147483648\n");
        write(&sys.join("memory.stat"), "inactive_file 1073741824\n");
        write(&leaf.join("memory.max"), "max\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(7 << 30), swap: None }, "the parent's limit applies");

        write(&leaf.join("memory.max"), "4294967296\n");
        write(&leaf.join("memory.current"), "1073741824\n");
        write(&leaf.join("memory.stat"), "inactive_file 0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(3 << 30), swap: None }, "the tighter leaf limit wins");

        write(&leaf.join("memory.swap.max"), "1073741824\n");
        write(&leaf.join("memory.swap.current"), "268435456\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(3 << 30), swap: Some(768 << 20) });

        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_v2_distinguishes_disabled_controller_from_missing_expected_limit() {
        // The hierarchy root may legitimately have no `memory.max`; a child has one exactly when
        // its parent delegates the controller.
        let (proc, sys) = fixture("v2-metadata");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw - cgroup2 cgroup2 rw\n", sys.display()));
        write(&sys.join("cgroup.controllers"), "cpu io memory\n");
        write(&sys.join("cgroup.subtree_control"), "+cpu +memory\n");
        write(&sys.join("app/memory.max"), "1073741824\n");
        write(&sys.join("app/memory.current"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None }, "a missing root file is normal");

        write(&sys.join("cgroup.controllers"), "cpu io\n");
        write(&sys.join("cgroup.subtree_control"), "+cpu\n");
        std::fs::remove_file(sys.join("app/memory.max")).unwrap();
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::NotConstrained,
            "readable controllers without memory prove the controller is absent"
        );

        write(&sys.join("cgroup.controllers"), "cpu io memory\n");
        write(&sys.join("cgroup.subtree_control"), "+memory\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Unreadable,
            "memory is delegated to the child, so the missing child file means the view is incomplete"
        );

        std::fs::remove_file(sys.join("cgroup.controllers")).unwrap();
        std::fs::remove_file(sys.join("cgroup.subtree_control")).unwrap();
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "without metadata the absence of a constraint cannot be established");
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // No cgroup filesystem mounted (Flatpak): the limit is invisible, host memory applies.
        let (proc, _) = fixture("no-cgroup-mount");
        let sys = proc.parent().unwrap().join("absent-sys");
        write(&proc.join("self/cgroup"), "0::/app.slice/app-flatpak.scope\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::NotConstrained);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // Isolated case 1: a readable empty `/sys/fs/cgroup` mount-point directory with normal
        // non-cgroup `mountinfo` also reports `NotConstrained`.
        let (proc, sys) = fixture("empty-cgroup-mountpoint");
        write(&proc.join("self/cgroup"), "0::/app.slice/app-flatpak.scope\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::NotConstrained);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // Isolated case 2: absent `/sys/fs/cgroup` with a bubblewrap empty-source non-cgroup mount
        // (`- tmpfs  rw,...`) parses cleanly and reports `NotConstrained`.
        let (proc, _) = fixture("absent-cgroup-empty-source");
        let sys = proc.parent().unwrap().join("absent-sys");
        write(&proc.join("self/cgroup"), "0::/app.slice/app-flatpak.scope\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n26 20 0:24 / /tmp rw,nosuid,nodev - tmpfs  rw,nosuid,nodev\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::NotConstrained);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // A truncated mount line may be the cgroup mount: with /sys/fs/cgroup absent, fail closed.
        let (proc, _) = fixture("truncated-mountinfo-line");
        let sys = proc.parent().unwrap().join("absent-sys");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n30 20 0:26 / /sys/fs/cgroup rw - cgroup2\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // A cgroup2 mount at an alternate path must be honoured while /sys/fs/cgroup is absent.
        let (proc, _) = fixture("alternate-cgroup-mount");
        let sys = proc.parent().unwrap().join("absent-sys");
        let alt = proc.parent().unwrap().join("alt-cgroup");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw - cgroup2 cgroup2 rw\n", alt.display()));
        write(&alt.join("app/memory.max"), "1073741824\n");
        write(&alt.join("app/memory.current"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None });
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // Available but not delegated anywhere on the path: no cgroup limit applies.
        let (proc, sys) = fixture("v2-not-enabled");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw - cgroup2 cgroup2 rw\n", sys.display()));
        write(&sys.join("cgroup.controllers"), "cpu io memory\n");
        write(&sys.join("cgroup.subtree_control"), "+cpu\n");
        write(&sys.join("app/cgroup.controllers"), "\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::NotConstrained);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prefers_the_widest_hierarchy_view_over_a_deep_bind_mount() {
        let (proc, sys) = fixture("bind-widest");
        write(&proc.join("self/cgroup"), "0::/user.slice/app.scope\n");
        let narrow = sys.join("bind");
        write(
            &proc.join("self/mountinfo"),
            &format!("30 20 0:26 / {} rw - cgroup2 cgroup2 rw\n31 30 0:26 /user.slice/app.scope {} rw - cgroup2 cgroup2 rw\n", sys.display(), narrow.display()),
        );
        write(&sys.join("user.slice/memory.max"), "2147483648\n");
        write(&sys.join("user.slice/memory.current"), "0\n");
        write(&sys.join("user.slice/cgroup.subtree_control"), "+memory\n");
        write(&sys.join("user.slice/app.scope/memory.max"), "max\n");
        write(&narrow.join("memory.max"), "max\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: None },
            "the hierarchy's own mount exposes user.slice's limit; the deep bind mount would hide it"
        );

        write(&proc.join("self/mountinfo"), &format!("31 30 0:26 /user.slice/app.scope {} rw - cgroup2 cgroup2 rw\n", narrow.display()));
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: None, swap: None },
            "the bind mount alone cannot see the ancestor limit, which is why the widest view is preferred"
        );

        std::fs::remove_file(narrow.join("memory.max")).unwrap();
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Unreadable,
            "a mounted subtree root cannot silently lose the controller file it should expose"
        );
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_v1_cgroup_honours_controller_mounts_and_ancestors() {
        let (proc, sys) = fixture("v1");
        write(&proc.join("self/cgroup"), "4:memory:/docker/abc\n");
        let mount = sys.join("memory");
        write(&proc.join("self/mountinfo"), &format!("31 24 0:26 / {} rw,nosuid,nodev,noexec,relatime shared:12 - cgroup cgroup rw,memory\n", mount.display()));
        let leaf = mount.join("docker/abc");
        write(&mount.join("docker/memory.limit_in_bytes"), "9223372036854771712\n");
        write(&mount.join("memory.limit_in_bytes"), "8589934592\n");
        write(&mount.join("memory.usage_in_bytes"), "2147483648\n");
        write(&mount.join("memory.stat"), "total_inactive_file 1073741824\n");
        write(&leaf.join("memory.limit_in_bytes"), "9223372036854771712\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(7 << 30), swap: None }, "the v1 sentinel is unlimited");

        write(&leaf.join("memory.limit_in_bytes"), "3221225472\n");
        write(&leaf.join("memory.usage_in_bytes"), "1073741824\n");
        write(&leaf.join("memory.stat"), "total_inactive_file 0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: None }, "the tighter leaf limit wins");

        write(&mount.join("memory.memsw.limit_in_bytes"), "9663676416\n");
        write(&mount.join("memory.memsw.usage_in_bytes"), "2147483648\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: Some(1 << 30) },
            "swap room is memsw limit - memory limit - swap used at the same ancestor, not combined headroom minus RAM headroom"
        );

        write(&mount.join("memory.memsw.usage_in_bytes"), "3221225472\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: Some(0) },
            "a gigabyte already swapped uses the ancestor's whole swap allowance"
        );

        write(&mount.join("memory.memsw.limit_in_bytes"), "10737418240\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: Some(1 << 30) },
            "only the memsw allowance above the memory limit counts as added swap room"
        );

        write(&mount.join("memory.limit_in_bytes"), "9223372036854771712\n");
        write(&leaf.join("memory.limit_in_bytes"), "9223372036854771712\n");
        write(&mount.join("memory.memsw.limit_in_bytes"), "2147483648\n");
        write(&mount.join("memory.memsw.usage_in_bytes"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: Some(0) }, "memsw alone still caps RAM");

        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_v1_swap_bounds_combine_per_ancestor() {
        let (proc, sys) = fixture("v1-swap-ancestors");
        write(&proc.join("self/cgroup"), "4:memory:/a/b\n");
        let mount = sys.join("memory");
        write(&proc.join("self/mountinfo"), &format!("31 24 0:26 / {} rw - cgroup cgroup rw,memory\n", mount.display()));
        write(&mount.join("memory.limit_in_bytes"), "8589934592\n");
        write(&mount.join("memory.usage_in_bytes"), "2147483648\n");
        write(&mount.join("memory.memsw.limit_in_bytes"), "9663676416\n");
        write(&mount.join("memory.memsw.usage_in_bytes"), "2147483648\n");
        write(&mount.join("a/memory.limit_in_bytes"), "10737418240\n");
        write(&mount.join("a/memory.usage_in_bytes"), "1073741824\n");
        write(&mount.join("a/memory.memsw.limit_in_bytes"), "21474836480\n");
        write(&mount.join("a/memory.memsw.usage_in_bytes"), "2147483648\n");
        write(&mount.join("a/b/memory.limit_in_bytes"), "3221225472\n");
        write(&mount.join("a/b/memory.usage_in_bytes"), "1073741824\n");

        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Limited { ram: Some(2 << 30), swap: Some(1 << 30) },
            "root swap allowance is 9 - 8 - 0 = 1 GiB; /a contributes 9 GiB and the leaf adds no swap limit"
        );
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_v1_mount_root_prefix_is_stripped() {
        let (proc, sys) = fixture("v1-root");
        write(&proc.join("self/cgroup"), "4:memory:/docker/abc/child\n");
        let mount = sys.join("memory");
        write(&proc.join("self/mountinfo"), &format!("31 24 0:26 /docker/abc {} rw,relatime - cgroup cgroup rw,memory\n", mount.display()));
        write(&mount.join("memory.limit_in_bytes"), "1073741824\n");
        write(&mount.join("memory.usage_in_bytes"), "0\n");
        write(&mount.join("memory.stat"), "total_inactive_file 0\n");
        write(&mount.join("child/memory.limit_in_bytes"), "1073741824\n");
        write(&mount.join("child/memory.usage_in_bytes"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None });
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cgroup_falls_back_to_conventional_layouts() {
        let (proc, sys) = fixture("fallback-v2");
        write(&proc.join("self/cgroup"), "0::/app.scope\n");
        write(&sys.join("app.scope/memory.max"), "1073741824\n");
        write(&sys.join("app.scope/memory.current"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None });
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("fallback-v1");
        write(&proc.join("self/cgroup"), "4:memory:/app\n");
        write(&sys.join("memory/memory.limit_in_bytes"), "1073741824\n");
        write(&sys.join("memory/memory.usage_in_bytes"), "0\n");
        write(&sys.join("memory/memory.stat"), "total_inactive_file 0\n");
        write(&sys.join("memory/app/memory.limit_in_bytes"), "1073741824\n");
        write(&sys.join("memory/app/memory.usage_in_bytes"), "0\n");
        write(&sys.join("memory/app/memory.stat"), "total_inactive_file 0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Limited { ram: Some(1 << 30), swap: None });
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unreadable_constraints_fail_closed() {
        let (proc, sys) = fixture("unreadable-missing-dir");
        write(&proc.join("self/cgroup"), "0::/missing\n");
        write(&proc.join("meminfo"), "MemAvailable: 12582912 kB\nSwapFree: 2097152 kB\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        let snap = linux::snapshot_at(&proc, &sys);
        assert_eq!(snap.available_ram, Some(0), "host memory must not be assumed");
        assert_eq!(snap.swap_free, Some(0));
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("unreadable-v1-limit");
        write(&proc.join("self/cgroup"), "4:memory:/app\n");
        let mount = sys.join("memory");
        write(&proc.join("self/mountinfo"), &format!("31 24 0:26 / {} rw,relatime - cgroup cgroup rw,memory\n", mount.display()));
        std::fs::create_dir_all(mount.join("app")).unwrap();
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("unreadable-v2-limit");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&sys.join("app/memory.max"), "lots\n");
        write(&sys.join("app/memory.current"), "0\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("no-memory-controller");
        write(&proc.join("self/cgroup"), "4:cpu,cpuacct:/user.slice\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::NotConstrained);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("malformed-membership");
        write(&proc.join("self/cgroup"), "0:memory:/app\n");
        write(&proc.join("meminfo"), "MemAvailable: 12582912 kB\nSwapFree: 2097152 kB\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "a malformed membership line cannot prove the controller absent");
        assert_eq!(linux::snapshot_at(&proc, &sys).available_ram, Some(0), "host memory must not be assumed");
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("empty-membership");
        write(&proc.join("self/cgroup"), "");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "an empty membership file cannot prove absence");
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        let (proc, sys) = fixture("mountinfo-without-cgroup");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "30 20 8:1 / / rw - ext4 /dev/root rw\n");
        write(&sys.join("app/memory.max"), "1073741824\n");
        write(&sys.join("app/memory.current"), "0\n");
        assert_eq!(
            linux::cgroup_limits(&proc, &sys),
            linux::CgroupLimits::Unreadable,
            "a readable mountinfo without a cgroup mount must not be guessed away by the conventional path"
        );
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    /// A missing `/sys/fs/cgroup` only means "no limit visible" when `mountinfo` was read, parsed
    /// to real mounts and lists no cgroup filesystem; every other combination stays fail-closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_absent_cgroup_root_needs_proof_before_it_means_unconstrained() {
        let cases: [(&str, Option<&str>); 3] =
            [("mountinfo-missing", None), ("mountinfo-empty", Some("")), ("mountinfo-garbage", Some("not a mountinfo line\n\n- - -\n"))];
        for (name, mountinfo) in cases {
            let (proc, _) = fixture(name);
            let sys = proc.parent().unwrap().join("absent-sys");
            write(&proc.join("self/cgroup"), "0::/app\n");
            if let Some(text) = mountinfo {
                write(&proc.join("self/mountinfo"), text);
            }
            assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "{name}");
            std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
        }

        // A stat error that is not NotFound (a path component that is a file) may hide a real
        // directory, so it is not "absent" either.
        let (proc, _) = fixture("sys-root-not-a-directory");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        let sys = proc.join("self/cgroup/fs/cgroup");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    /// A cgroup mount listed at an alternate path means a limit may exist: v1 and v2 alike, and
    /// when its files cannot be read the answer is fail-closed, not "unconstrained".
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_alternate_cgroup_mounts_with_unreadable_limits_fail_closed() {
        for (name, membership, fstype, options) in [("alt-v1", "4:memory:/app\n", "cgroup", "rw,memory"), ("alt-v2", "0::/app\n", "cgroup2", "rw")] {
            let (proc, _) = fixture(name);
            let sys = proc.parent().unwrap().join("absent-sys");
            let alt = proc.parent().unwrap().join("alt-cgroup");
            write(&proc.join("self/cgroup"), membership);
            write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw - {fstype} {fstype} {options}\n", alt.display()));
            std::fs::create_dir_all(alt.join("app")).unwrap();
            assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "{name}");
            std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_snapshot_combines_host_meminfo_and_cgroup_room() {
        let (proc, sys) = fixture("snapshot");
        write(&proc.join("self/cgroup"), "0::/app.scope\n");
        write(&proc.join("meminfo"), "MemTotal: 16777216 kB\nMemAvailable: 12582912 kB\nSwapFree: 2097152 kB\n");
        write(&proc.join("self/mountinfo"), &format!("30 20 0:26 / {} rw,relatime - cgroup2 cgroup2 rw\n", sys.display()));
        write(&sys.join("app.scope/memory.max"), "4294967296\n");
        write(&sys.join("app.scope/memory.current"), "0\n");
        write(&sys.join("app.scope/memory.swap.max"), "536870912\n");
        write(&sys.join("app.scope/memory.swap.current"), "0\n");

        let snap = linux::snapshot_at(&proc, &sys);
        assert_eq!(snap.available_ram, Some(4 << 30), "the cgroup limit is smaller than host MemAvailable");
        assert_eq!(snap.swap_free, Some(512 << 20));
        assert_eq!(snap.commit_headroom, None);
        assert_eq!(linux::available_at(&proc, &sys), Some((4 << 30) + (512 << 20)), "legacy helper keeps RAM + swap");
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_parse_mountinfo_handles_empty_mount_source() {
        let tmpfs = "26 20 0:24 / /tmp rw,nosuid,nodev - tmpfs  rw,nosuid,nodev,size=65536k\n";
        let cg2 = "30 20 0:26 /user.slice /sys/fs/cg\\0402 rw,relatime - cgroup2  rw,nsdelegate\n";
        let truncated = "31 20 0:27 / /sys/fs/cgroup rw - cgroup2 rw\n";
        let mounts = linux::parse_mountinfo(&format!("{tmpfs}{cg2}{truncated}"));
        assert_eq!(mounts.len(), 2, "single-space line missing super_options is skipped");
        let m0 = &mounts[0];
        assert_eq!((&m0.root[..], &m0.mount_point[..], &m0.fstype[..]), ("/", "/tmp", "tmpfs"));
        assert_eq!((&m0.options[..], &m0.super_options[..]), ("rw,nosuid,nodev", "rw,nosuid,nodev,size=65536k"));
        let m1 = &mounts[1];
        assert_eq!((&m1.root[..], &m1.mount_point[..], &m1.fstype[..]), ("/user.slice", "/sys/fs/cg 2", "cgroup2"));
        assert_eq!((&m1.options[..], &m1.super_options[..]), ("rw,relatime", "rw,nsdelegate"));
    }

    /// An empty `/sys/fs/cgroup` directory only reports `NotConstrained` when `mountinfo` parses
    /// cleanly with no cgroup mount; cgroup mounts (including empty-source lines), missing/empty/
    /// truncated `mountinfo`, non-empty directories, files, and unreadable directories fail closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_empty_or_invalid_sys_root_fails_closed_unless_proven_unmounted() {
        use std::os::unix::fs::PermissionsExt;

        let bad_mountinfo: [(&str, Option<&str>); 6] = [
            ("empty-sys-missing-mountinfo", None),
            ("empty-sys-empty-mountinfo", Some("")),
            ("empty-sys-garbage-mountinfo", Some("not a mountinfo line\n")),
            ("empty-sys-truncated-mountinfo", Some("25 20 0:23 / /proc rw - proc proc rw\n30 20 0:26 / /sys/fs/cgroup rw - cgroup2\n")),
            ("empty-sys-cgroup2-mount", Some("30 20 0:26 / /sys/fs/cgroup rw - cgroup2 cgroup2 rw\n")),
            ("empty-sys-empty-source-cgroup2", Some("30 20 0:26 / /sys/fs/cgroup rw - cgroup2  rw\n")),
        ];
        for (name, mountinfo) in bad_mountinfo {
            let (proc, sys) = fixture(name);
            write(&proc.join("self/cgroup"), "0::/app\n");
            if let Some(text) = mountinfo {
                write(&proc.join("self/mountinfo"), text);
            }
            assert!(linux::sys_root_absent_or_empty(&sys), "{name}");
            assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable, "{name}");
            std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
        }

        // Empty `/sys/fs/cgroup` with a v1 memory mount in `mountinfo` also fails closed.
        let (proc, sys) = fixture("empty-sys-cgroup-v1-mount");
        write(&proc.join("self/cgroup"), "4:memory:/app\n");
        write(&proc.join("self/mountinfo"), "31 24 0:26 / /sys/fs/cgroup/memory rw - cgroup cgroup rw,memory\n");
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // A non-empty `/sys/fs/cgroup` directory is not treated as unmounted even with non-cgroup mountinfo.
        let (proc, sys) = fixture("non-empty-sys-root");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        write(&sys.join("memory.max"), "1073741824\n");
        assert!(!linux::sys_root_absent_or_empty(&sys));
        assert_eq!(linux::cgroup_limits(&proc, &sys), linux::CgroupLimits::Unreadable);
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // `sys_root` as a regular file and a `NotADirectory` path component return false and fail closed.
        let (proc, _) = fixture("file-sys-root");
        let file_sys = proc.parent().unwrap().join("sys-as-file");
        write(&file_sys, "");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        for path in [&file_sys, &file_sys.join("cgroup")] {
            assert!(!linux::sys_root_absent_or_empty(path));
            assert_eq!(linux::cgroup_limits(&proc, path), linux::CgroupLimits::Unreadable);
        }
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();

        // An unreadable `/sys/fs/cgroup` directory (`read_dir` error) returns false and fails closed.
        let (proc, sys) = fixture("unreadable-sys-root");
        write(&proc.join("self/cgroup"), "0::/app\n");
        write(&proc.join("self/mountinfo"), "25 20 0:23 / /proc rw - proc proc rw\n");
        std::fs::set_permissions(&sys, std::fs::Permissions::from_mode(0o000)).unwrap();
        let read_blocked = std::fs::read_dir(&sys).is_err();
        let absent = linux::sys_root_absent_or_empty(&sys);
        let limits = linux::cgroup_limits(&proc, &sys);
        std::fs::set_permissions(&sys, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(proc.parent().unwrap()).unwrap();
        if read_blocked {
            assert!(!absent);
            assert_eq!(limits, linux::CgroupLimits::Unreadable);
        }
    }
}
