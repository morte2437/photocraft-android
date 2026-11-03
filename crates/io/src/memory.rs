//! How much memory this process can still use, so an import too large for the machine is
//! refused with an error instead of ending the app. Linux overcommits: an allocation that can't
//! be backed still succeeds, and the out-of-memory killer ends the process (and every open
//! document) when its pages are written, so a fallible allocation alone never fails there.

use crate::IoError;

/// Memory left free beside what an import needs: for the app, its display and the system.
pub const RESERVE: u64 = 512 << 20;

/// Automatic allowance for native platforms when available memory is unknown.
///
/// Estimates above it need explicit per-invocation approval, up to the platform's aggregate
/// admission limit (see [`native_platform_cap`]). This is a conservative automatic admission
/// threshold, not a hard ceiling and not a per-buffer allocation bound.
pub const NATIVE_FALLBACK_BUDGET: u64 = 2 << 30;

/// Automatic allowance for WebAssembly execution.
///
/// Estimates above it need explicit per-invocation approval, up to
/// [`WASM_MAX_PEAK_BUDGET`]. This is a conservative product threshold, not an assertion that
/// the browser can allocate that much.
pub const WASM_BUDGET: u64 = 1 << 30;

/// Upper ceiling on the automatic soft import budget when available memory is known.
pub const MAX_SOFT_BUDGET: u64 = 8 << 30;

/// Estimate above which an import made with unknown available memory carries an extra warning.
///
/// It is a warning threshold, not a hard ceiling: explicit approval remains possible up to the
/// platform's actual allocation limits, but the process may run out of memory.
pub const UNKNOWN_MEMORY_WARNING_THRESHOLD: u64 = 8 << 30;

/// Reserve subtracted from commit headroom when known.
pub const COMMIT_RESERVE: u64 = 512 << 20;

/// Bytes Rust's layout rules allow for one contiguous allocation on this target
/// (`isize::MAX`), not a bound on an import's aggregate peak.
///
/// A layer plane, a decode buffer or a tile is one contiguous allocation, so each must fit this
/// limit at the point it is allocated. An import's peak is the *sum* of many such buffers and
/// may legitimately exceed this; decode paths enforce their own per-buffer limits
/// (`photocraft-psd::MAX_DECODED_BYTES`, `photocraft-codecs`' `Limits::max_alloc`) there.
pub const MAX_SINGLE_ALLOCATION: u64 = isize::MAX as u64;

/// Architectural maximum linear memory of wasm32: 65,536 pages of 64 KiB (4 GiB).
///
/// This is the whole address space of one wasm32 instance, shared with the resident document,
/// history, decoder buffers, the allocator, the stack and the runtime. It is an address-space
/// bound, not a promise that 4 GiB can be allocated; browsers may cap linear memory lower.
pub const WASM_LINEAR_MEMORY_LIMIT: u64 = 4u64 << 30;

/// Largest single Rust allocation on wasm32 (`isize::MAX`): 2 GiB - 1.
///
/// A per-buffer bound only. Aggregate admission on web uses [`WASM_MAX_PEAK_BUDGET`]; the layout
/// limit itself rejects a larger single buffer, and decoders may apply a tighter cap.
pub const WASM_MAX_SINGLE_ALLOCATION: u64 = (1u64 << 31) - 1;

/// Conservative product cap for the aggregate peak of one web import: 2 GiB.
///
/// Chosen by this project, not dictated by the architecture: it sits below both the 4 GiB
/// wasm32 linear-memory limit (leaving room for the runtime, stack, allocator metadata and the
/// resident document) and is reachable only through explicit approval above [`WASM_BUDGET`].
/// Browsers commonly allow less, and allocation may still fail. Do not read it as a guarantee.
pub const WASM_MAX_PEAK_BUDGET: u64 = 2 << 30;

/// Conservative product cap for the aggregate peak of a 32-bit native process: 2 GiB.
///
/// A 32-bit process has a 4 GiB address space, but the kernel/reserved range, runtime, stack,
/// allocator metadata, document and history share it. This is a product choice well below the
/// architectural limit, not an architectural bound.
pub const NATIVE_32BIT_PEAK_CAP: u64 = 2 << 30;

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
    /// A per-attempt provider consulted on every query, unlike the fixed [`MOCK_SNAPSHOT`].
    /// Tests use it to change the measured capacity between admission checks from another
    /// thread; it is thread-local, so it never affects unrelated parallel tests.
    static MOCK_PROVIDER: std::cell::RefCell<Option<std::rc::Rc<dyn Fn() -> MemorySnapshot>>> = const { std::cell::RefCell::new(None) };
}

impl MemorySnapshot {
    /// Unknown memory measurement.
    pub const UNKNOWN: Self = Self { available_ram: None, swap_free: None, commit_headroom: None };

    /// Obtains the current system memory snapshot from platform providers.
    pub fn current() -> Self {
        #[cfg(any(test, feature = "testgen"))]
        {
            if let Some(provider) = MOCK_PROVIDER.with(|p| p.borrow().clone()) {
                return provider();
            }
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
        #[cfg(target_os = "macos")]
        {
            macos_mem::snapshot()
        }
        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
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
    /// Set when available memory is unknown: estimates above this threshold still need explicit
    /// approval but also carry the uncertain-capacity warning. `None` when capacity is known.
    pub warning_threshold: Option<u64>,
}

/// Admission decision for a given required memory estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Estimated memory <= soft limit: open automatically.
    Automatic,
    /// Soft limit < estimated memory <= hard limit: requires explicit user/caller confirmation.
    RequiresApproval,
    /// Estimated memory > hard limit, or the saturated `u64::MAX` overflow sentinel: refused.
    Refused,
}

impl MemoryBudget {
    /// Determines the admission decision for an estimated requirement in bytes.
    ///
    /// `u64::MAX` is the sentinel a format estimate produces when a size overflowed `u64`
    /// (`photocraft-io`'s TIFF estimates saturate there rather than wrap). It is not a real
    /// requirement, so it is always refused, even when the hard limit is itself `u64::MAX` (the
    /// native unknown-memory cap) and even with explicit approval or a matching one-shot
    /// estimate. The refusal keeps the estimate and limit as they are; no value is invented.
    pub fn decide(&self, required: u64) -> BudgetDecision {
        if required == u64::MAX {
            return BudgetDecision::Refused;
        }
        if required <= self.soft_limit {
            BudgetDecision::Automatic
        } else if required <= self.hard_limit {
            BudgetDecision::RequiresApproval
        } else {
            BudgetDecision::Refused
        }
    }

    /// Whether an approvable estimate also needs the uncertain-capacity warning.
    ///
    /// This is only meaningful together with [`BudgetDecision::RequiresApproval`]; it says the
    /// budget came from the unknown-memory fallback and the estimate is above
    /// [`UNKNOWN_MEMORY_WARNING_THRESHOLD`].
    pub fn requires_uncertainty_warning(&self, required: u64) -> bool {
        self.warning_threshold.is_some_and(|threshold| required > threshold)
    }
}

/// Whether an approval-required estimate carries the unknown-capacity warning, given the
/// capacity measured when the estimate was made.
///
/// A known capacity never carries it (a measured number is reported instead of a guess); when
/// capacity is unknown, an estimate above [`UNKNOWN_MEMORY_WARNING_THRESHOLD`] is still
/// approvable but says the process or tab may run out of memory. Kept separate from
/// [`MemoryBudget`] so a pending approval can carry the flag across a later, fresh measurement.
pub fn uncertain_capacity_warning(required: u64, available_ram: Option<u64>) -> bool {
    available_ram.is_none() && required > UNKNOWN_MEMORY_WARNING_THRESHOLD
}

/// Which platform policy to resolve. Tests use this to exercise the web policy on any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Native,
    Web,
}

/// The platform policy selected for this build: Web on wasm32, Native everywhere else.
fn active_platform() -> Platform {
    if cfg!(target_arch = "wasm32") { Platform::Web } else { Platform::Native }
}

/// Aggregate address-space bound for native admission (not a per-buffer allocation bound).
///
/// A 64-bit process has no practical aggregate address-space limit measurable through safe
/// standard APIs, so this is `u64::MAX`; per-buffer and format limits are enforced where buffers
/// are actually allocated. A 32-bit process uses [`NATIVE_32BIT_PEAK_CAP`].
pub fn native_platform_cap() -> u64 {
    if cfg!(target_pointer_width = "32") { NATIVE_32BIT_PEAK_CAP } else { u64::MAX }
}

/// Resolves the adaptive hard and soft import budgets from a snapshot and an optional caller limit.
///
/// The limits are for an import's aggregate peak estimate, not for one allocation: a peak may sum
/// many buffers, each of which is separately bounded where the decoder allocates it.
///
/// Policy:
/// - Known RAM: H = min(floor(3*A/4), A - 1 GiB, C - reserve if known, [`native_platform_cap`]);
///   S = min(A/2, 8 GiB, H).
/// - Unknown native RAM: S = min(2 GiB, H) where H is [`native_platform_cap`], reduced by known
///   commit headroom; approval above S is possible up to H. 8 GiB is a warning threshold, not H.
/// - Web (wasm32): S = 1 GiB; H = min([`WASM_MAX_PEAK_BUDGET`], caller limit).
/// - <= S: automatic; S < E <= H: explicit approval; > H: refuse, no unlimited override.
/// - The saturated `u64::MAX` overflow sentinel is always refused, even when H is `u64::MAX`
///   (see [`MemoryBudget::decide`]).
/// - Known zero available RAM stays zero (it never becomes unknown/free).
/// - swap is never part of the adaptive budget.
/// - Explicit caller limits may lower admission, never bypass the hard boundary.
pub fn resolve_budget(snapshot: MemorySnapshot, caller_limit: Option<u64>) -> MemoryBudget {
    resolve_budget_for(active_platform(), snapshot, caller_limit)
}

/// [`resolve_budget`] for an explicit platform, so tests can exercise the web policy on a native host.
fn resolve_budget_for(platform: Platform, snapshot: MemorySnapshot, caller_limit: Option<u64>) -> MemoryBudget {
    if platform == Platform::Web {
        let mut hard = WASM_MAX_PEAK_BUDGET;
        if let Some(caller) = caller_limit.filter(|&c| c > 0) {
            hard = hard.min(caller);
        }
        return MemoryBudget { soft_limit: WASM_BUDGET.min(hard), hard_limit: hard, available_ram: None, warning_threshold: None };
    }

    let platform_cap = native_platform_cap();
    let (mut hard, mut soft, warning_threshold) = match snapshot.available_ram {
        Some(a) => {
            let h_pct = ((a as u128 * 3) / 4) as u64;
            let h_reserve = a.saturating_sub(1 << 30);
            let mut h = h_pct.min(h_reserve);

            if let Some(c) = snapshot.commit_headroom {
                let c_room = c.saturating_sub(COMMIT_RESERVE);
                h = h.min(c_room);
            }

            h = h.min(platform_cap);

            let s_pct = a / 2;
            let s = s_pct.min(MAX_SOFT_BUDGET).min(h);
            (h, s, None)
        }
        None => {
            let mut h = platform_cap;
            if let Some(c) = snapshot.commit_headroom {
                let c_room = c.saturating_sub(COMMIT_RESERVE);
                h = h.min(c_room);
            }
            let s = NATIVE_FALLBACK_BUDGET.min(h);
            (h, s, Some(UNKNOWN_MEMORY_WARNING_THRESHOLD))
        }
    };

    if let Some(caller) = caller_limit.filter(|&c| c > 0) {
        hard = hard.min(caller);
        soft = soft.min(hard);
    }

    MemoryBudget { soft_limit: soft, hard_limit: hard, available_ram: snapshot.available_ram, warning_threshold }
}

/// Result of evaluating an import admission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionOutcome {
    pub budget: MemoryBudget,
    /// Total authorized memory allowance for the entire attempt (top-level + nested decodes):
    /// `min(S if automatic else H if approved, caller cap if any, approved_estimate if present)`.
    pub authorized_total: u64,
}

/// Central evaluation of PSD admission against adaptive budgets and options.
///
/// Refuses before decoding if:
/// - `required` is the saturated `u64::MAX` overflow sentinel (see [`MemoryBudget::decide`]);
/// - `required > hard_limit` (regardless of approvals: hard ceiling cannot be overridden);
/// - `required > soft_limit` and high memory is not approved;
/// - high memory is approved, but `approved_estimate` is specified and `required > approved_estimate`
///   (preventing unbounded approval for altered/enlarged files).
pub(crate) fn evaluate_psd_admission(
    required: u64,
    caller_limit: u64,
    high_memory_approved: bool,
    approved_estimate: Option<u64>,
    snapshot: MemorySnapshot,
) -> Result<AdmissionOutcome, IoError> {
    evaluate_psd_admission_for(active_platform(), required, caller_limit, high_memory_approved, approved_estimate, snapshot)
}

/// [`evaluate_psd_admission`] for an explicit platform, so tests can exercise the web policy on a native host.
fn evaluate_psd_admission_for(
    platform: Platform,
    required: u64,
    caller_limit: u64,
    high_memory_approved: bool,
    approved_estimate: Option<u64>,
    snapshot: MemorySnapshot,
) -> Result<AdmissionOutcome, IoError> {
    let limit_opt = (caller_limit > 0).then_some(caller_limit);
    let budget = resolve_budget_for(platform, snapshot, limit_opt);

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

/// Applies a caller's explicit local bounds to a nested decode.
///
/// A nested import spends the enclosing attempt's remaining allowance: it never resolves an
/// independent adaptive budget and never resets the parent's. The caller's explicit bounds still
/// apply, because they limit this attempt rather than describe a second budget:
/// `caller_limit > 0` is the lower caller allowance, and `approved_estimate` (when present) is the
/// estimate one-shot consent was given for. Both are checked before the parent is charged, so a
/// refusal leaves its remaining allowance untouched, and the tighter bound is the one reported.
///
/// A nested attempt cannot ask for fresh consent, so exceeding either bound is a hard refusal
/// ([`IoError::PsdImportBudget`], the form the outer caller limit already uses) rather than a new
/// approval request.
///
/// The saturated `u64::MAX` overflow sentinel is not covered here (there is no meaningful limit to
/// report when no local bound is set); [`crate::psd_gate::consume_attempt_allowance`] refuses it
/// before the parent is charged.
pub(crate) fn nested_import_local_bound(required: u64, caller_limit: u64, approved_estimate: Option<u64>) -> Result<(), IoError> {
    let caller = (caller_limit > 0).then_some(caller_limit);
    let limit = match (caller, approved_estimate) {
        (Some(caller), Some(approved)) => Some(caller.min(approved)),
        (caller, approved) => caller.or(approved),
    };
    match limit {
        Some(limit) if required > limit => Err(IoError::PsdImportBudget { required, limit }),
        _ => Ok(()),
    }
}

/// Bytes this process can still use without running out, or `None` where that isn't known. On
/// Linux: the free RAM the kernel can hand out (`MemAvailable`, capped by the process's memory
/// cgroup — v2 `memory.max` or v1 `memory.limit_in_bytes`/`memsw` — and those above it) plus free
/// swap (capped by the cgroup's swap limits). Elsewhere `None`: Windows refuses an allocation past
/// its commit limit, which the importers already report as an error, and macOS pages to disk as it
/// needs. macOS's physical-memory provider feeds the adaptive budget, not this legacy
/// RAM-plus-swap helper; flat and layered TIFF conversion keep their existing behaviour.
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

/// Runs a closure with a per-attempt memory provider on the current thread.
///
/// Unlike [`with_mock_memory`], the provider is consulted for every
/// [`MemorySnapshot::current`], so a test can change the measured capacity while an import is
/// waiting on the decode gate and observe the post-gate recheck. The provider is thread-local
/// and restored on return or panic, so it never overrides another test's measurement.
#[cfg(any(test, feature = "testgen"))]
pub fn with_mock_memory_provider<R>(provider: std::rc::Rc<dyn Fn() -> MemorySnapshot>, f: impl FnOnce() -> R) -> R {
    let prev = MOCK_PROVIDER.with(|p| p.borrow_mut().replace(provider));

    struct Reset {
        prev: Option<std::rc::Rc<dyn Fn() -> MemorySnapshot>>,
    }
    impl Drop for Reset {
        fn drop(&mut self) {
            MOCK_PROVIDER.with(|p| *p.borrow_mut() = self.prev.take());
        }
    }

    let _reset = Reset { prev };
    f()
}

/// Explicit test-support capture/scope object to propagate the thread-local fixed mock snapshot
/// and the scoped PSD estimate charge across worker threads without modifying global process
/// state.
///
/// It captures `set_mock_memory`/`with_mock_memory`'s fixed snapshot and
/// [`crate::psd_budget::with_test_estimate_charge`]'s extra charge, so an engine worker that
/// enters the scope makes the same admission estimates as the test thread. The per-attempt
/// provider set by `with_mock_memory_provider` is not captured: that provider is a non-`Send`
/// closure re-consulted on every [`MemorySnapshot::current`], and propagating it to a worker
/// needs an explicitly shared, `Send + Sync` scope designed for that purpose (follow-up work).
/// A worker an import starts therefore sees the fixed snapshot, not a live provider.
///
/// Capturing is explicit: nothing here changes a thread that never enters the scope, and no
/// global state is written.
///
/// In production, this is an inert zero-overhead type.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestMemoryScope {
    #[cfg(any(test, feature = "testgen"))]
    mock: Option<MemorySnapshot>,
    /// The estimate charge captured with the scope (zero when the test set none).
    #[cfg(any(test, feature = "testgen"))]
    estimate_charge: u64,
}

/// RAII guard that restores the previous thread-local mock memory and estimate charge upon drop.
pub struct TestMemoryGuard {
    #[cfg(any(test, feature = "testgen"))]
    prev: Option<MemorySnapshot>,
    #[cfg(any(test, feature = "testgen"))]
    prev_charge: u64,
}

#[cfg(any(test, feature = "testgen"))]
impl Drop for TestMemoryGuard {
    fn drop(&mut self) {
        MOCK_SNAPSHOT.with(|m| *m.borrow_mut() = self.prev);
        crate::psd_budget::enter_test_estimate_charge(self.prev_charge);
    }
}

impl TestMemoryScope {
    /// Captures the current thread's fixed mock memory snapshot and estimate charge, if any (see
    /// [`TestMemoryScope`] for what is not captured).
    ///
    /// Never captures actual system RAM as a permanent snapshot so that
    /// production memory rechecks remain fresh.
    pub fn capture() -> Self {
        #[cfg(any(test, feature = "testgen"))]
        {
            let mock = MOCK_SNAPSHOT.with(|m| *m.borrow());
            Self { mock, estimate_charge: crate::psd_budget::test_estimate_charge() }
        }
        #[cfg(not(any(test, feature = "testgen")))]
        {
            Self {}
        }
    }

    /// Enters this captured mock memory scope, returning a RAII guard that
    /// restores the thread's previous mock state and estimate charge on drop.
    pub fn enter(self) -> TestMemoryGuard {
        #[cfg(any(test, feature = "testgen"))]
        {
            let prev = MOCK_SNAPSHOT.with(|m| {
                let old = *m.borrow();
                *m.borrow_mut() = self.mock;
                old
            });
            let prev_charge = crate::psd_budget::enter_test_estimate_charge(self.estimate_charge);
            TestMemoryGuard { prev, prev_charge }
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
    struct Mount {
        root: String,
        mount_point: String,
        fstype: String,
        options: String,
        super_options: String,
    }

    /// Measures available RAM and free swap from `/proc/meminfo`, capped by the process's memory
    /// cgroup. Unreadable cgroup constraints fail closed as zero available memory rather than
    /// falling back to unrestricted host memory.
    pub(super) fn snapshot() -> MemorySnapshot {
        snapshot_at(Path::new("/proc"), Path::new("/sys/fs/cgroup"))
    }

    /// Provider body with explicit roots, so tests exercise the real filesystem parsing without
    /// global mocks.
    pub(super) fn snapshot_at(proc_root: &Path, sys_root: &Path) -> MemorySnapshot {
        let info = std::fs::read_to_string(proc_root.join("meminfo")).ok();
        let host_ram = info.as_deref().and_then(|i| meminfo(i, "MemAvailable:"));
        let host_swap = info.as_deref().and_then(|i| meminfo(i, "SwapFree:"));

        let (cgroup_ram, cgroup_swap) = match cgroup_limits(proc_root, sys_root) {
            CgroupLimits::NotConstrained => (None, None),
            CgroupLimits::Limited { ram, swap } => (ram, swap),
            CgroupLimits::Unreadable => return MemorySnapshot { available_ram: Some(0), swap_free: Some(0), commit_headroom: None },
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
        MemorySnapshot { available_ram, swap_free, commit_headroom: None }
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

    /// Parses `/proc/self/mountinfo` into the mount fields used to locate a cgroup hierarchy.
    ///
    /// Malformed lines are skipped; callers fail closed when no matching mount is found.
    fn parse_mountinfo(text: &str) -> Vec<Mount> {
        let mut mounts = Vec::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let Some(separator) = fields.iter().position(|f| *f == "-") else { continue };
            let (Some(root), Some(point), Some(fstype), Some(super_options)) =
                (fields.get(3).copied(), fields.get(4).copied(), fields.get(separator + 1).copied(), fields.get(separator + 3).copied())
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
    use super::MemorySnapshot;

    /// Measures available physical and commit memory through `GlobalMemoryStatusEx`. A failed
    /// query reports unknown memory rather than inventing a measurement.
    pub(super) fn snapshot() -> MemorySnapshot {
        match winsafe::GlobalMemoryStatusEx() {
            Ok(msx) => MemorySnapshot { available_ram: Some(msx.ullAvailPhys), swap_free: None, commit_headroom: Some(msx.ullAvailPageFile) },
            Err(_) => MemorySnapshot::UNKNOWN,
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
    use super::MemorySnapshot;
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};

    /// Available physical memory through `sysinfo`'s safe API.
    ///
    /// On macOS that is `host_statistics64`'s `vm_statistics64` (see
    /// [`super::macos_available_ram`]). The estimate counts free pages, file-backed pages and
    /// purgeable pages, not anonymous working sets, wired memory, compressed memory or swap. A
    /// failed query reports unknown; a successful zero reading stays zero.
    pub(super) fn snapshot() -> MemorySnapshot {
        let system = match System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram())) {
            Ok(system) => system,
            Err(_) => return MemorySnapshot::UNKNOWN,
        };
        match super::macos_available_ram(system.total_memory(), system.free_memory(), system.available_memory(), system.used_memory()) {
            Some(ram) => MemorySnapshot { available_ram: Some(ram), swap_free: None, commit_headroom: None },
            None => MemorySnapshot::UNKNOWN,
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod fallback_mem {
    use super::MemorySnapshot;

    /// Fallback provider for platforms without a validated memory-availability provider.
    ///
    /// Available memory is reported as unknown (`None`), which applies the conservative native
    /// fallback budget (2 GiB, approval above it) without inventing a measurement.
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
        if cfg!(target_arch = "wasm32") {
            return; // the web policy has its own tests
        }
        if native_platform_cap() < 24 << 30 {
            // This table uses 6-24 GiB limits, above a 32-bit aggregate cap.
            return;
        }
        let gb = 1u64 << 30;

        // 12 GiB RAM: H = min(9 GiB, 11 GiB) = 9 GiB; S = min(6 GiB, 8 GiB, 9 GiB) = 6 GiB.
        let snap12 = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        let b12 = resolve_budget(snap12, None);
        assert_eq!(b12.soft_limit, 6 * gb);
        assert_eq!(b12.hard_limit, 9 * gb);
        assert_eq!(b12.warning_threshold, None);

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
        assert_eq!(b32.warning_threshold, None);
        assert!(!b32.requires_uncertainty_warning(10 * gb), "known memory has no unknown-capacity warning");

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
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let snap_zero = MemorySnapshot { available_ram: Some(0), swap_free: None, commit_headroom: None };
        let b_zero = resolve_budget(snap_zero, None);
        assert_eq!(b_zero.soft_limit, 0);
        assert_eq!(b_zero.hard_limit, 0);
        assert_eq!(b_zero.warning_threshold, None);
        assert_eq!(b_zero.decide(1), BudgetDecision::Refused);

        let snap_unknown = MemorySnapshot::UNKNOWN;
        let b_unknown = resolve_budget(snap_unknown, None);
        assert_eq!(b_unknown.soft_limit, NATIVE_FALLBACK_BUDGET.min(native_platform_cap()));
        assert_eq!(b_unknown.hard_limit, native_platform_cap());
        assert_eq!(b_unknown.available_ram, None);
        assert_eq!(b_unknown.warning_threshold, Some(UNKNOWN_MEMORY_WARNING_THRESHOLD));
        assert_eq!(b_unknown.decide(1), BudgetDecision::Automatic);
        let expected = if b_unknown.hard_limit > b_unknown.soft_limit { BudgetDecision::RequiresApproval } else { BudgetDecision::Automatic };
        assert_eq!(b_unknown.decide(b_unknown.hard_limit.min(u64::MAX - 1)), expected);
        assert_eq!(b_unknown.decide(u64::MAX), BudgetDecision::Refused, "the saturated overflow sentinel is never approved");
        if b_unknown.hard_limit < u64::MAX {
            assert_eq!(b_unknown.decide(b_unknown.hard_limit + 1), BudgetDecision::Refused);
        }
    }

    /// A format estimate that overflows `u64` saturates at `u64::MAX`. Unknown native memory has
    /// `u64::MAX` as its hard limit, so without a sentinel rule explicit approval would admit it.
    #[test]
    fn a_saturated_estimate_is_refused_even_at_the_unknown_native_hard_limit() {
        if cfg!(target_pointer_width = "32") {
            // A 32-bit aggregate cap already refuses it; the case this test guards is the
            // 64-bit unknown-memory cap, where the limit itself is the sentinel.
            return;
        }
        let budget = resolve_budget_for(Platform::Native, MemorySnapshot::UNKNOWN, None);
        assert_eq!(budget.hard_limit, u64::MAX, "the case that would otherwise admit the sentinel");
        assert_eq!(budget.decide(u64::MAX), BudgetDecision::Refused, "the decision agrees with evaluation");
        assert_eq!(budget.decide(u64::MAX - 1), BudgetDecision::RequiresApproval);

        for (approved, estimate) in [(false, None), (true, None), (true, Some(u64::MAX))] {
            match evaluate_psd_admission_for(Platform::Native, u64::MAX, 0, approved, estimate, MemorySnapshot::UNKNOWN) {
                Err(IoError::PsdImportBudget { required, limit }) => {
                    assert_eq!((required, limit), (u64::MAX, u64::MAX), "the typed refusal keeps the estimate and limit unchanged")
                }
                other => panic!("approved={approved}, estimate={estimate:?}: expected a hard refusal, got {other:?}"),
            }
        }

        // A caller limit equal to the sentinel, or an approved estimate of it, is no exception.
        assert!(matches!(
            evaluate_psd_admission_for(Platform::Native, u64::MAX, u64::MAX, true, Some(u64::MAX), MemorySnapshot::UNKNOWN),
            Err(IoError::PsdImportBudget { required: u64::MAX, limit: u64::MAX })
        ));
    }

    #[test]
    fn unknown_memory_is_conservative_but_not_capped_at_eight_gib() {
        if cfg!(target_pointer_width = "32") {
            return;
        }
        let gb = 1u64 << 30;
        let b = resolve_budget_for(Platform::Native, MemorySnapshot::UNKNOWN, None);
        assert_eq!(b.soft_limit, NATIVE_FALLBACK_BUDGET);
        assert_eq!(b.hard_limit, native_platform_cap());

        assert_eq!(b.decide(NATIVE_FALLBACK_BUDGET), BudgetDecision::Automatic);
        assert_eq!(b.decide(NATIVE_FALLBACK_BUDGET + 1), BudgetDecision::RequiresApproval);
        assert_eq!(b.decide(UNKNOWN_MEMORY_WARNING_THRESHOLD), BudgetDecision::RequiresApproval);
        assert!(!b.requires_uncertainty_warning(UNKNOWN_MEMORY_WARNING_THRESHOLD));
        assert!(b.requires_uncertainty_warning(UNKNOWN_MEMORY_WARNING_THRESHOLD + 1));
        assert_eq!(b.decide(UNKNOWN_MEMORY_WARNING_THRESHOLD + 1), BudgetDecision::RequiresApproval, "8 GiB is a warning threshold, not a hard ceiling");
        assert_eq!(b.decide(64 * gb), BudgetDecision::RequiresApproval);
        assert!(b.requires_uncertainty_warning(64 * gb));
    }

    #[test]
    fn unknown_memory_with_known_commit_headroom_stays_conservative() {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let gb = 1u64 << 30;

        let snap_zero = MemorySnapshot { available_ram: None, swap_free: None, commit_headroom: Some(0) };
        let b_zero = resolve_budget(snap_zero, None);
        assert_eq!(b_zero.hard_limit, 0);
        assert_eq!(b_zero.soft_limit, 0);
        assert_eq!(b_zero.decide(1), BudgetDecision::Refused);

        let snap_tight = MemorySnapshot { available_ram: None, swap_free: None, commit_headroom: Some(gb) };
        let b_tight = resolve_budget(snap_tight, None);
        assert_eq!(b_tight.hard_limit, gb - COMMIT_RESERVE);
        assert_eq!(b_tight.soft_limit, gb - COMMIT_RESERVE);

        let snap_room = MemorySnapshot { available_ram: None, swap_free: None, commit_headroom: Some(4 * gb) };
        let b_room = resolve_budget(snap_room, None);
        assert_eq!(b_room.hard_limit, 4 * gb - COMMIT_RESERVE);
        assert_eq!(b_room.soft_limit, NATIVE_FALLBACK_BUDGET);
    }

    #[test]
    fn unknown_memory_admits_approved_imports_above_eight_gib() {
        if cfg!(target_pointer_width = "32") {
            return; // the 32-bit platform cap is below 8 GiB
        }
        let gb = 1u64 << 30;
        let snap = MemorySnapshot::UNKNOWN;
        let required = 9 * gb;

        let err = evaluate_psd_admission_for(Platform::Native, required, 0, false, None, snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { available_ram: None, .. }));

        let out = evaluate_psd_admission_for(Platform::Native, required, 0, true, Some(required), snap).unwrap();
        assert_eq!(out.authorized_total, required);
        assert!(out.budget.requires_uncertainty_warning(required), "the consent warning must carry through admission");

        let err = evaluate_psd_admission_for(Platform::Native, required, 0, true, Some(required - 1), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { .. }), "a larger estimate needs fresh consent");
    }

    #[test]
    fn web_policy_allows_approval_above_one_gib_within_its_own_cap() {
        let b = resolve_budget_for(Platform::Web, MemorySnapshot::UNKNOWN, None);
        assert_eq!(b.soft_limit, WASM_BUDGET);
        assert_eq!(b.hard_limit, WASM_MAX_PEAK_BUDGET);
        assert!(b.hard_limit < WASM_LINEAR_MEMORY_LIMIT, "the aggregate cap stays below the wasm32 address space");
        const { assert!(WASM_MAX_SINGLE_ALLOCATION < WASM_LINEAR_MEMORY_LIMIT, "a single Rust allocation cannot use all 4 GiB") };
        assert_eq!(b.available_ram, None);
        assert_eq!(b.warning_threshold, None);

        assert_eq!(b.decide(WASM_BUDGET), BudgetDecision::Automatic);
        assert_eq!(b.decide(WASM_BUDGET + 1), BudgetDecision::RequiresApproval);
        assert_eq!(b.decide(WASM_MAX_PEAK_BUDGET), BudgetDecision::RequiresApproval);
        assert_eq!(b.decide(WASM_MAX_PEAK_BUDGET + 1), BudgetDecision::Refused);
        const { assert!(WASM_MAX_SINGLE_ALLOCATION < WASM_MAX_PEAK_BUDGET, "the aggregate cap must not be the single-allocation layout bound") };
        let out = evaluate_psd_admission_for(Platform::Web, WASM_MAX_PEAK_BUDGET, 0, true, Some(WASM_MAX_PEAK_BUDGET), MemorySnapshot::UNKNOWN).unwrap();
        assert_eq!(out.authorized_total, WASM_MAX_PEAK_BUDGET, "approval may authorize an aggregate above one buffer's layout limit");

        let capped = resolve_budget_for(Platform::Web, MemorySnapshot::UNKNOWN, Some(512 << 20));
        assert_eq!(capped.hard_limit, 512 << 20);
        assert_eq!(capped.soft_limit, 512 << 20);

        let no_raise = resolve_budget_for(Platform::Web, MemorySnapshot::UNKNOWN, Some(64 << 30));
        assert_eq!(no_raise.hard_limit, WASM_MAX_PEAK_BUDGET);
    }

    #[test]
    fn web_admission_requires_approval_above_its_automatic_allowance() {
        let snap = MemorySnapshot::UNKNOWN;
        let approval = 3 * (WASM_BUDGET / 2);

        let out = evaluate_psd_admission_for(Platform::Web, WASM_BUDGET, 0, false, None, snap).unwrap();
        assert_eq!(out.authorized_total, WASM_BUDGET);

        let err = evaluate_psd_admission_for(Platform::Web, WASM_BUDGET + 1, 0, false, None, snap).unwrap_err();
        assert!(matches!(
            err,
            IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }
                if required == WASM_BUDGET + 1 && soft_limit == WASM_BUDGET && hard_limit == WASM_MAX_PEAK_BUDGET && available_ram.is_none()
        ));

        let out = evaluate_psd_admission_for(Platform::Web, approval, 0, true, Some(approval), snap).unwrap();
        assert_eq!(out.authorized_total, approval);

        let err = evaluate_psd_admission_for(Platform::Web, approval, 0, true, Some(approval - 1), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { .. }));

        let err = evaluate_psd_admission_for(Platform::Web, WASM_MAX_PEAK_BUDGET + 1, 0, true, None, snap).unwrap_err();
        assert!(matches!(err, IoError::PsdImportBudget { .. }));
    }

    #[test]
    fn caller_caps_lower_unknown_memory_admission() {
        let b = resolve_budget_for(Platform::Native, MemorySnapshot::UNKNOWN, Some(1 << 30));
        assert_eq!(b.hard_limit, 1 << 30);
        assert_eq!(b.soft_limit, 1 << 30);
        assert_eq!(b.warning_threshold, Some(UNKNOWN_MEMORY_WARNING_THRESHOLD));

        let no_raise = resolve_budget_for(Platform::Native, MemorySnapshot::UNKNOWN, Some(64 << 30));
        assert_eq!(no_raise.hard_limit, native_platform_cap().min(64 << 30));
        assert_eq!(no_raise.soft_limit, NATIVE_FALLBACK_BUDGET.min(native_platform_cap()));
    }

    #[test]
    fn arithmetic_overflow_safety() {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let snap_huge = MemorySnapshot { available_ram: Some(u64::MAX), swap_free: Some(u64::MAX), commit_headroom: Some(u64::MAX) };
        let b = resolve_budget(snap_huge, None);
        let expected_hard = (((u64::MAX as u128) * 3) / 4) as u64;
        let expected_hard = expected_hard.min(native_platform_cap());
        assert_eq!(b.hard_limit, expected_hard);
        assert_eq!(b.soft_limit, MAX_SOFT_BUDGET.min(expected_hard));
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

    #[test]
    fn known_snapshot_uses_known_memory_policy_on_every_target() {
        let gb = 1u64 << 30;
        let snap = MemorySnapshot { available_ram: Some(16 * gb), swap_free: None, commit_headroom: None };
        let b = resolve_budget_for(Platform::Native, snap, None);
        let expected_hard = (12 * gb).min(native_platform_cap());
        assert_eq!(b.hard_limit, expected_hard);
        assert_eq!(b.soft_limit, (8 * gb).min(expected_hard));
        assert_eq!(b.available_ram, Some(16 * gb));
        assert_eq!(b.warning_threshold, None);
    }

    #[test]
    fn aggregate_cap_is_not_a_single_allocation_bound() {
        let cap = native_platform_cap();
        if cfg!(target_pointer_width = "32") {
            assert_eq!(cap, NATIVE_32BIT_PEAK_CAP, "32-bit native uses the documented product cap");
        } else {
            assert_eq!(cap, u64::MAX, "64-bit native has no measurable aggregate address-space cap");
            assert!(MAX_SINGLE_ALLOCATION < cap, "a per-buffer limit must not become the aggregate limit");
        }
    }

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
        let snap = MemorySnapshot::current();
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

    #[test]
    fn swap_is_never_part_of_the_adaptive_budget() {
        let gb = 1u64 << 30;
        let with_swap = MemorySnapshot { available_ram: Some(4 * gb), swap_free: Some(64 * gb), commit_headroom: None };
        let without_swap = MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
        assert_eq!(resolve_budget_for(Platform::Native, with_swap, None), resolve_budget_for(Platform::Native, without_swap, None));
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
}
