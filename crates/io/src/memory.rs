//! How much memory this process can still use, so an import too large for the machine is
//! refused with an error instead of ending the app. Linux overcommits: an allocation that can't
//! be backed still succeeds, and the out-of-memory killer ends the process (and every open
//! document) when its pages are written, so a fallible allocation alone never fails there.

use crate::IoError;

/// Memory left free beside what an import needs: for the app, its display and the system.
pub const RESERVE: u64 = 512 << 20;

/// Default allowance for native platforms when available memory is unknown (no override allowed).
pub const NATIVE_FALLBACK_BUDGET: u64 = 2 << 30;

/// Maximum allowance for WebAssembly execution.
pub const WASM_BUDGET: u64 = 1 << 30;

/// Upper ceiling on the automatic soft import budget on desktop platforms.
pub const MAX_SOFT_BUDGET: u64 = 8 << 30;

/// Reserve subtracted from commit headroom when known.
pub const COMMIT_RESERVE: u64 = 512 << 20;

/// A snapshot of system memory availability, distinguishing physical RAM from swap and commit headroom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MemorySnapshot {
    /// Currently available physical RAM (A) in bytes, bounded by applicable container or process limits.
    /// Free swap is explicitly excluded. A value of `Some(0)` represents zero capacity, not unknown memory.
    pub available_ram: Option<u64>,
    /// Available swap headroom in bytes, if known.
    pub swap_free: Option<u64>,
    /// System or process commit headroom (C) in bytes, if measurable.
    pub commit_headroom: Option<u64>,
}

#[cfg(any(test, feature = "testgen"))]
thread_local! {
    static MOCK_SNAPSHOT: std::cell::RefCell<Option<MemorySnapshot>> = const { std::cell::RefCell::new(None) };
}

impl MemorySnapshot {
    /// Unknown memory measurement.
    pub const UNKNOWN: Self = Self { available_ram: None, swap_free: None, commit_headroom: None };

    /// Obtains the current system memory snapshot from platform providers.
    pub fn current() -> Self {
        #[cfg(any(test, feature = "testgen"))]
        {
            if let Some(mock) = MOCK_SNAPSHOT.with(|m| *m.borrow()) {
                return mock;
            }
        }

        #[cfg(target_os = "linux")]
        {
            linux::snapshot()
        }
        #[cfg(windows)]
        {
            windows_mem::snapshot()
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            fallback_mem::snapshot()
        }
    }
}

/// Resolved memory budgets derived from a [`MemorySnapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBudget {
    /// Automatic admission ceiling (S): imports requiring <= S open without confirmation.
    pub soft_limit: u64,
    /// Hard refusal ceiling (H): imports requiring > H are refused and cannot be overridden.
    pub hard_limit: u64,
    /// Currently available physical RAM from the snapshot, if known.
    pub available_ram: Option<u64>,
}

/// Admission decision for a given required memory estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Estimated memory <= soft limit: open automatically.
    Automatic,
    /// Soft limit < estimated memory <= hard limit: requires explicit user/caller confirmation.
    RequiresApproval,
    /// Estimated memory > hard limit: refused.
    Refused,
}

impl MemoryBudget {
    /// Determines the admission decision for an estimated requirement in bytes.
    pub fn decide(&self, required: u64) -> BudgetDecision {
        if required <= self.soft_limit {
            BudgetDecision::Automatic
        } else if required <= self.hard_limit {
            BudgetDecision::RequiresApproval
        } else {
            BudgetDecision::Refused
        }
    }
}

/// Resolves the adaptive hard and soft import budgets from a snapshot and an optional caller limit.
///
/// Policy:
/// - Hard H = min(floor(3*A/4), saturating A - 1 GiB, C - reserve if known, platform/address-space limits)
/// - Soft S = min(50% A, 8 GiB, H)
/// - <= S: automatic; S < E <= H: explicit approval; > H: refuse, no unlimited override.
/// - Unknown native fallback: 2 GiB bounded by C - reserve if known (S = H).
/// - Browser/wasm: fixed 1 GiB (S = H = 1 GiB).
/// - Explicit caller limits may lower admission, never bypass the hard boundary.
pub fn resolve_budget(snapshot: MemorySnapshot, caller_limit: Option<u64>) -> MemoryBudget {
    if cfg!(target_arch = "wasm32") {
        let mut hard = WASM_BUDGET;
        if let Some(caller) = caller_limit.filter(|&c| c > 0) {
            hard = hard.min(caller);
        }
        let soft = hard;
        return MemoryBudget { soft_limit: soft, hard_limit: hard, available_ram: None };
    }

    let address_space_cap = if cfg!(target_pointer_width = "32") { 2 << 30 } else { u64::MAX };

    let (mut hard, mut soft) = match snapshot.available_ram {
        Some(a) => {
            let h_pct = ((a as u128 * 3) / 4) as u64;
            let h_reserve = a.saturating_sub(1 << 30);
            let mut h = h_pct.min(h_reserve);

            if let Some(c) = snapshot.commit_headroom {
                let c_room = c.saturating_sub(COMMIT_RESERVE);
                h = h.min(c_room);
            }

            h = h.min(address_space_cap);

            let s_pct = a / 2;
            let s = s_pct.min(MAX_SOFT_BUDGET).min(h);
            (h, s)
        }
        None => {
            let mut limit = NATIVE_FALLBACK_BUDGET.min(address_space_cap);
            if let Some(c) = snapshot.commit_headroom {
                let c_room = c.saturating_sub(COMMIT_RESERVE);
                limit = limit.min(c_room);
            }
            (limit, limit)
        }
    };

    if let Some(caller) = caller_limit.filter(|&c| c > 0) {
        hard = hard.min(caller);
        soft = soft.min(hard);
    }

    MemoryBudget { soft_limit: soft, hard_limit: hard, available_ram: snapshot.available_ram }
}

/// Central evaluation of PSD admission against adaptive budgets and options.
///
/// Refuses before decoding if:
/// - `required > hard_limit` (regardless of approvals: hard ceiling cannot be overridden);
/// - `required > soft_limit` and high memory is not approved;
/// - high memory is approved, but `approved_estimate` is specified and `required > approved_estimate`
///   (preventing unbounded approval for altered/enlarged files).
///
/// Result of evaluating an import admission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionOutcome {
    pub budget: MemoryBudget,
    /// Total authorized memory allowance for the entire attempt (top-level + nested decodes):
    /// `min(S if automatic else H if approved, caller cap if any, approved_estimate if present)`.
    pub authorized_total: u64,
}

pub(crate) fn evaluate_psd_admission(
    required: u64,
    caller_limit: u64,
    high_memory_approved: bool,
    approved_estimate: Option<u64>,
    snapshot: MemorySnapshot,
) -> Result<AdmissionOutcome, IoError> {
    let limit_opt = (caller_limit > 0).then_some(caller_limit);
    let budget = resolve_budget(snapshot, limit_opt);

    let authorized_total = match budget.decide(required) {
        BudgetDecision::Automatic => {
            let mut cap = budget.soft_limit;
            if let Some(caller) = limit_opt {
                cap = cap.min(caller);
            }
            if let Some(appr) = approved_estimate {
                cap = cap.min(appr);
            }
            cap
        }
        BudgetDecision::RequiresApproval => {
            if !high_memory_approved {
                return Err(IoError::PsdApprovalRequired {
                    required,
                    soft_limit: budget.soft_limit,
                    hard_limit: budget.hard_limit,
                    available_ram: budget.available_ram,
                });
            }
            let mut cap = budget.hard_limit;
            if let Some(caller) = limit_opt {
                cap = cap.min(caller);
            }
            if let Some(appr) = approved_estimate {
                cap = cap.min(appr);
            }
            cap
        }
        BudgetDecision::Refused => return Err(IoError::PsdImportBudget { required, limit: budget.hard_limit }),
    };

    if required > authorized_total {
        return Err(IoError::PsdApprovalRequired {
            required,
            soft_limit: budget.soft_limit,
            hard_limit: budget.hard_limit,
            available_ram: budget.available_ram,
        });
    }
    Ok(AdmissionOutcome { budget, authorized_total })
}

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

/// Sets a thread-local mock memory snapshot for testing policy and import admission.
#[cfg(any(test, feature = "testgen"))]
pub fn set_mock_memory(snapshot: Option<MemorySnapshot>) {
    MOCK_SNAPSHOT.with(|m| *m.borrow_mut() = snapshot);
}

/// Runs a closure with a temporarily injected mock memory snapshot on the current thread.
///
/// Restores the previous snapshot upon return or panic, supporting nested invocations.
#[cfg(any(test, feature = "testgen"))]
pub fn with_mock_memory<R>(snapshot: MemorySnapshot, f: impl FnOnce() -> R) -> R {
    let prev = MOCK_SNAPSHOT.with(|m| {
        let old = *m.borrow();
        *m.borrow_mut() = Some(snapshot);
        old
    });

    struct Reset {
        prev: Option<MemorySnapshot>,
    }
    impl Drop for Reset {
        fn drop(&mut self) {
            MOCK_SNAPSHOT.with(|m| *m.borrow_mut() = self.prev);
        }
    }

    let _reset = Reset { prev };
    f()
}

/// Explicit test-support capture/scope object to propagate a thread-local mock
/// snapshot across worker threads without modifying global process state.
///
/// In production, this is an inert zero-overhead type.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestMemoryScope {
    #[cfg(any(test, feature = "testgen"))]
    mock: Option<MemorySnapshot>,
}

/// RAII guard that restores the previous thread-local mock memory upon drop.
pub struct TestMemoryGuard {
    #[cfg(any(test, feature = "testgen"))]
    prev: Option<MemorySnapshot>,
}

#[cfg(any(test, feature = "testgen"))]
impl Drop for TestMemoryGuard {
    fn drop(&mut self) {
        MOCK_SNAPSHOT.with(|m| *m.borrow_mut() = self.prev);
    }
}

impl TestMemoryScope {
    /// Captures the current thread's mock memory snapshot, if any.
    ///
    /// Never captures actual system RAM as a permanent snapshot so that
    /// production memory rechecks remain fresh.
    pub fn capture() -> Self {
        #[cfg(any(test, feature = "testgen"))]
        {
            let mock = MOCK_SNAPSHOT.with(|m| *m.borrow());
            Self { mock }
        }
        #[cfg(not(any(test, feature = "testgen")))]
        {
            Self {}
        }
    }

    /// Enters this captured mock memory scope, returning a RAII guard that
    /// restores the thread's previous mock state on drop.
    pub fn enter(self) -> TestMemoryGuard {
        #[cfg(any(test, feature = "testgen"))]
        {
            let prev = MOCK_SNAPSHOT.with(|m| {
                let old = *m.borrow();
                *m.borrow_mut() = self.mock;
                old
            });
            TestMemoryGuard { prev }
        }
        #[cfg(not(any(test, feature = "testgen")))]
        {
            TestMemoryGuard {}
        }
    }

    /// Executes a closure within this captured mock memory scope.
    pub fn run<R>(self, f: impl FnOnce() -> R) -> R {
        let _guard = self.enter();
        f()
    }
}

#[cfg(target_os = "linux")]
pub(super) mod linux {
    use super::MemorySnapshot;
    use std::path::{Path, PathBuf};

    pub(super) fn snapshot() -> MemorySnapshot {
        let Some(info) = std::fs::read_to_string("/proc/meminfo").ok() else {
            return MemorySnapshot::UNKNOWN;
        };
        let ram = meminfo(&info, "MemAvailable:");
        let swap = meminfo(&info, "SwapFree:");
        let (cgroup_ram, cgroup_swap) = cgroup_dir().as_deref().map_or((None, None), cgroup_room);
        let available_ram = ram.map(|r| r.min(cgroup_ram.unwrap_or(u64::MAX)));
        let swap_free = swap.map(|s| s.min(cgroup_swap.unwrap_or(u64::MAX)));
        MemorySnapshot { available_ram, swap_free, commit_headroom: None }
    }

    pub(super) fn available() -> Option<u64> {
        let snap = snapshot();
        snap.available_ram.map(|ram| ram.saturating_add(snap.swap_free.unwrap_or(0)))
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

    fn stat(s: &str, field: &str) -> Option<u64> {
        s.lines().find_map(|l| {
            let mut parts = l.split_whitespace();
            let key = parts.next()?;
            if key == field { parts.next()?.parse().ok() } else { None }
        })
    }
}

#[cfg(windows)]
mod windows_mem {
    use super::MemorySnapshot;

    pub(super) fn snapshot() -> MemorySnapshot {
        match winsafe::GlobalMemoryStatusEx() {
            Ok(msx) => MemorySnapshot { available_ram: Some(msx.ullAvailPhys), swap_free: None, commit_headroom: Some(msx.ullAvailPageFile) },
            Err(_) => MemorySnapshot::UNKNOWN,
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod fallback_mem {
    use super::MemorySnapshot;

    /// Fallback provider for platforms without a validated memory-availability provider (e.g. macOS).
    ///
    /// Available memory is reported as unknown (`None`), which applies the safe native fallback
    /// budget (2 GiB) without allowing an unverified memory override. macOS dynamic paging makes
    /// physical RAM a poor indicator of allocation capacity without platform-specific Mach VM APIs.
    pub(super) fn snapshot() -> MemorySnapshot {
        MemorySnapshot::UNKNOWN
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

    #[test]
    fn pure_policy_adheres_to_approved_thresholds() {
        let gb = 1u64 << 30;

        // 12 GiB RAM: H = min(9 GiB, 11 GiB) = 9 GiB; S = min(6 GiB, 8 GiB, 9 GiB) = 6 GiB.
        let snap12 = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        let b12 = resolve_budget(snap12, None);
        assert_eq!(b12.soft_limit, 6 * gb);
        assert_eq!(b12.hard_limit, 9 * gb);

        // 5.80 GiB estimate (5.8 * 1 GiB = 6227702579)
        let est_5_8 = (5.80 * (gb as f64)).round() as u64;
        assert_eq!(b12.decide(est_5_8), BudgetDecision::Automatic);
        assert_eq!(b12.decide(6 * gb), BudgetDecision::Automatic);
        assert_eq!(b12.decide(6 * gb + 1), BudgetDecision::RequiresApproval);
        assert_eq!(b12.decide(9 * gb), BudgetDecision::RequiresApproval);
        assert_eq!(b12.decide(9 * gb + 1), BudgetDecision::Refused);

        // 8 GiB RAM: H = min(6 GiB, 7 GiB) = 6 GiB; S = min(4 GiB, 8 GiB, 6 GiB) = 4 GiB.
        let snap8 = MemorySnapshot { available_ram: Some(8 * gb), swap_free: None, commit_headroom: None };
        let b8 = resolve_budget(snap8, None);
        assert_eq!(b8.soft_limit, 4 * gb);
        assert_eq!(b8.hard_limit, 6 * gb);
        assert_eq!(b8.decide(est_5_8), BudgetDecision::RequiresApproval);

        // 6 GiB RAM: H = min(4.5 GiB, 5 GiB) = 4.5 GiB; S = 3 GiB.
        let snap6 = MemorySnapshot { available_ram: Some(6 * gb), swap_free: None, commit_headroom: None };
        let b6 = resolve_budget(snap6, None);
        assert_eq!(b6.soft_limit, 3 * gb);
        assert_eq!(b6.hard_limit, (4.5 * (gb as f64)) as u64);
        assert_eq!(b6.decide(est_5_8), BudgetDecision::Refused);

        // 32 GiB RAM: H = min(24 GiB, 31 GiB) = 24 GiB; S = min(16 GiB, 8 GiB, 24 GiB) = 8 GiB cap.
        let snap32 = MemorySnapshot { available_ram: Some(32 * gb), swap_free: None, commit_headroom: None };
        let b32 = resolve_budget(snap32, None);
        assert_eq!(b32.soft_limit, 8 * gb);
        assert_eq!(b32.hard_limit, 24 * gb);

        // Commit headroom constraint: C = 5 GiB -> C - 512 MiB = 4.5 GiB.
        let snap_commit = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: Some(5 * gb) };
        let b_commit = resolve_budget(snap_commit, None);
        assert_eq!(b_commit.hard_limit, 5 * gb - COMMIT_RESERVE);
        assert_eq!(b_commit.soft_limit, 5 * gb - COMMIT_RESERVE);

        // Caller limits: lowers admission, cannot raise past H.
        let b_caller = resolve_budget(snap12, Some(5 * gb));
        assert_eq!(b_caller.hard_limit, 5 * gb);
        assert_eq!(b_caller.soft_limit, 5 * gb);

        let b_caller_high = resolve_budget(snap8, Some(20 * gb));
        assert_eq!(b_caller_high.hard_limit, 6 * gb);
        assert_eq!(b_caller_high.soft_limit, 4 * gb);
    }

    #[test]
    fn zero_available_ram_is_not_unknown() {
        let snap_zero = MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
        let b_zero = resolve_budget(snap_zero, None);
        assert_eq!(b_zero.soft_limit, 0);
        assert_eq!(b_zero.hard_limit, 0);
        assert_eq!(b_zero.decide(1), BudgetDecision::Refused);

        let snap_unknown = MemorySnapshot::UNKNOWN;
        let b_unknown = resolve_budget(snap_unknown, None);
        assert_eq!(b_unknown.soft_limit, NATIVE_FALLBACK_BUDGET);
        assert_eq!(b_unknown.hard_limit, NATIVE_FALLBACK_BUDGET);
        assert_eq!(b_unknown.decide(1), BudgetDecision::Automatic);
        assert_eq!(b_unknown.decide(NATIVE_FALLBACK_BUDGET + 1), BudgetDecision::Refused);
    }

    #[test]
    fn unknown_ram_with_known_zero_commit_bounds_admission() {
        let snap = MemorySnapshot { available_ram: None, swap_free: None, commit_headroom: Some(0) };
        let b = resolve_budget(snap, None);
        assert_eq!(b.hard_limit, 0);
        assert_eq!(b.soft_limit, 0);
        assert_eq!(b.decide(1), BudgetDecision::Refused);

        let snap_room = MemorySnapshot { available_ram: None, swap_free: None, commit_headroom: Some(1 << 30) };
        let b_room = resolve_budget(snap_room, None);
        assert_eq!(b_room.hard_limit, (1 << 30) - COMMIT_RESERVE);
        assert_eq!(b_room.soft_limit, (1 << 30) - COMMIT_RESERVE);
    }

    #[test]
    fn arithmetic_overflow_safety() {
        let snap_huge = MemorySnapshot { available_ram: Some(u64::MAX), swap_free: Some(u64::MAX), commit_headroom: Some(u64::MAX) };
        let b = resolve_budget(snap_huge, None);
        let expected_hard = ((u64::MAX as u128 * 3) / 4) as u64;
        let expected_hard = if cfg!(target_pointer_width = "32") { 2 << 30 } else { expected_hard };
        assert_eq!(b.hard_limit, expected_hard);
        assert_eq!(b.soft_limit, MAX_SOFT_BUDGET);
    }

    #[test]
    fn nested_mock_memory_restores_previous() {
        let snap1 = MemorySnapshot { available_ram: Some(100), swap_free: None, commit_headroom: None };
        let snap2 = MemorySnapshot { available_ram: Some(200), swap_free: None, commit_headroom: None };
        with_mock_memory(MemorySnapshot::UNKNOWN, || {
            assert_eq!(MemorySnapshot::current(), MemorySnapshot::UNKNOWN);
            with_mock_memory(snap1, || {
                assert_eq!(MemorySnapshot::current().available_ram, Some(100));
                with_mock_memory(snap2, || {
                    assert_eq!(MemorySnapshot::current().available_ram, Some(200));
                });
                assert_eq!(MemorySnapshot::current().available_ram, Some(100));
            });
            assert_eq!(MemorySnapshot::current(), MemorySnapshot::UNKNOWN);
        });
    }

    #[test]
    fn central_admission_evaluation_enforces_bounds() {
        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let snap = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        // S = 6 GiB, H = 9 GiB

        // Below S: automatic, authorized_total = S (6 GiB)
        let out = evaluate_psd_admission(5 * gb, 0, false, None, snap).unwrap();
        assert_eq!(out.authorized_total, 6 * gb);

        // In [S, H]: requires approval
        let err = evaluate_psd_admission(7 * gb, 0, false, None, snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { required, .. } if required == 7 * gb));

        // In [S, H] with approval and no approved_estimate bound: admitted, authorized_total = H (9 GiB)
        let out = evaluate_psd_admission(7 * gb, 0, true, None, snap).unwrap();
        assert_eq!(out.authorized_total, 9 * gb);

        // In [S, H] with approval but required exceeds approved_estimate: requires fresh approval
        let err = evaluate_psd_admission(7 * gb, 0, true, Some(6 * gb), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { .. }));

        // In [S, H] with approval and required within approved_estimate: admitted, authorized_total = approved_estimate (7 GiB)
        let out = evaluate_psd_admission(7 * gb, 0, true, Some(7 * gb), snap).unwrap();
        assert_eq!(out.authorized_total, 7 * gb);

        // Exceeding H: refused even if high_memory_approved is true
        let err = evaluate_psd_admission(10 * gb, 0, true, Some(10 * gb), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdImportBudget { .. }));
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
    }

    #[cfg(target_os = "linux")]
    use std::path::Path;
}
