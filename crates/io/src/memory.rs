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
    /// Mocked [`scratch_capacity`]: `Some(None)` = spilling off, `Some(Some(n))` = `n` bytes.
    static MOCK_SCRATCH: std::cell::Cell<Option<Option<u64>>> = const { std::cell::Cell::new(None) };
}

/// Tile bytes the scratch disk can take beyond RAM: `None` while spilling is off, paused or
/// unavailable (web), or its room is unknown. See [`photocraft_raster::spill::scratch_capacity`].
///
/// Test builds: a mocked memory snapshot without a mocked scratch disk measures no scratch, so a
/// test that mocks RAM keeps the RAM-only policy whatever another test did to the process-wide
/// scratch disk.
pub fn scratch_capacity() -> Option<u64> {
    #[cfg(any(test, feature = "testgen"))]
    {
        if let Some(mock) = MOCK_SCRATCH.with(std::cell::Cell::get) {
            return mock;
        }
        if MOCK_SNAPSHOT.with(|m| m.borrow().is_some()) || MOCK_PROVIDER.with(|p| p.borrow().is_some()) {
            return None;
        }
    }
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    photocraft_raster::spill::scratch_capacity()
}

/// How much of `spillable` tile bytes the scratch disk covers on `platform` right now. The web
/// has no scratch disk, so it never gets credit.
fn scratch_credit_for(platform: Platform, spillable: u64) -> u64 {
    if platform == Platform::Web || spillable == 0 {
        return 0;
    }
    scratch_capacity().map_or(0, |room| spillable.min(room))
}

/// [`scratch_credit_for`] on this build's platform.
pub(crate) fn scratch_credit(spillable: u64) -> u64 {
    scratch_credit_for(active_platform(), spillable)
}

impl MemorySnapshot {
    /// Unknown memory measurement.
    pub const UNKNOWN: Self = Self { available_ram: None, swap_free: None, commit_headroom: None };

    /// The current measurement from the shared detector, [`photocraft_raster::memory::system`],
    /// which also sizes the tile residency budget.
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

        photocraft_raster::memory::system::current().into()
    }
}

impl From<photocraft_raster::memory::system::SystemMemory> for MemorySnapshot {
    /// The shared detector's measurement; an unreadable constraint is already the fail-closed
    /// `Some(0)` available RAM, which admission refuses.
    fn from(m: photocraft_raster::memory::system::SystemMemory) -> Self {
        Self { available_ram: m.available_ram, swap_free: m.swap_free, commit_headroom: m.commit_headroom }
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

/// [`resolve_budget`] for an import whose estimate includes `spillable` bytes of tile pixels.
///
/// While the scratch disk is spilling (on and not paused), tiles beyond the residency budget are
/// written to it, so those pixels are not bounded by physical RAM: the part of `spillable` the
/// disk has room for ([`scratch_capacity`], after the Scratch Size Limit and the free-space
/// reserve) is added to both S and H before the caller's explicit limit applies. Equivalently,
/// the non-spillable working memory (decode buffers, conversion copies, merged planes) is
/// admitted against the RAM policy alone, and the whole estimate against RAM plus scratch room.
/// With spilling off or paused, and always on the web, this is exactly [`resolve_budget`].
pub fn resolve_budget_with_scratch(snapshot: MemorySnapshot, caller_limit: Option<u64>, spillable: u64) -> MemoryBudget {
    let platform = active_platform();
    resolve_budget_credited(platform, snapshot, caller_limit, scratch_credit_for(platform, spillable))
}

/// [`resolve_budget`] for an explicit platform, so tests can exercise the web policy on a native host.
fn resolve_budget_for(platform: Platform, snapshot: MemorySnapshot, caller_limit: Option<u64>) -> MemoryBudget {
    resolve_budget_credited(platform, snapshot, caller_limit, 0)
}

/// The policy with `credit` bytes of scratch room added to S and H before the caller's limit.
fn resolve_budget_credited(platform: Platform, snapshot: MemorySnapshot, caller_limit: Option<u64>, credit: u64) -> MemoryBudget {
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
    // Tile pixels the scratch disk takes need no RAM (see [`resolve_budget_with_scratch`]).
    hard = hard.saturating_add(credit);
    soft = soft.saturating_add(credit);

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
///
/// `spillable` is the part of `required` that is tile pixels; while the scratch disk is spilling
/// the limits include the room it has for them (see [`resolve_budget_with_scratch`]). Errors
/// still report the whole `required` (the figure approval binds to) and the effective limits.
pub(crate) fn evaluate_psd_admission(
    required: u64,
    spillable: u64,
    caller_limit: u64,
    high_memory_approved: bool,
    approved_estimate: Option<u64>,
    snapshot: MemorySnapshot,
) -> Result<AdmissionOutcome, IoError> {
    evaluate_psd_admission_for(active_platform(), required, spillable, caller_limit, high_memory_approved, approved_estimate, snapshot)
}

/// [`evaluate_psd_admission`] for an explicit platform, so tests can exercise the web policy on a native host.
fn evaluate_psd_admission_for(
    platform: Platform,
    required: u64,
    spillable: u64,
    caller_limit: u64,
    high_memory_approved: bool,
    approved_estimate: Option<u64>,
    snapshot: MemorySnapshot,
) -> Result<AdmissionOutcome, IoError> {
    let limit_opt = (caller_limit > 0).then_some(caller_limit);
    let budget = resolve_budget_credited(platform, snapshot, limit_opt, scratch_credit_for(platform, spillable.min(required)));

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
/// RAM-plus-swap helper; flat and layered TIFF conversion keep their existing behaviour. Measured
/// by [`photocraft_raster::memory::system::available_with_swap`].
pub fn available() -> Option<u64> {
    photocraft_raster::memory::system::available_with_swap()
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

/// Runs a closure with a mocked scratch disk on the current thread: `Some(n)` = spilling with
/// `n` bytes of room, `None` = spilling off or paused. Restored on return or panic, and captured
/// by [`TestMemoryScope`].
#[cfg(any(test, feature = "testgen"))]
pub fn with_mock_scratch<R>(capacity: Option<u64>, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<Option<u64>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            MOCK_SCRATCH.with(|m| m.set(self.0));
        }
    }
    let _reset = Reset(MOCK_SCRATCH.with(|m| m.replace(Some(capacity))));
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
    /// The tile-pixel share of that charge.
    #[cfg(any(test, feature = "testgen"))]
    tile_charge: u64,
    /// The mocked scratch disk ([`with_mock_scratch`]), if any.
    #[cfg(any(test, feature = "testgen"))]
    scratch: Option<Option<u64>>,
}

/// RAII guard that restores the previous thread-local mock memory and estimate charge upon drop.
pub struct TestMemoryGuard {
    #[cfg(any(test, feature = "testgen"))]
    prev: Option<MemorySnapshot>,
    #[cfg(any(test, feature = "testgen"))]
    prev_charge: (u64, u64),
    #[cfg(any(test, feature = "testgen"))]
    prev_scratch: Option<Option<u64>>,
}

#[cfg(any(test, feature = "testgen"))]
impl Drop for TestMemoryGuard {
    fn drop(&mut self) {
        MOCK_SNAPSHOT.with(|m| *m.borrow_mut() = self.prev);
        crate::psd_budget::enter_test_estimate_charge(self.prev_charge.0, self.prev_charge.1);
        MOCK_SCRATCH.with(|m| m.set(self.prev_scratch));
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
            let (estimate_charge, tile_charge) = crate::psd_budget::test_estimate_charge();
            Self { mock, estimate_charge, tile_charge, scratch: MOCK_SCRATCH.with(std::cell::Cell::get) }
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
            let prev_charge = crate::psd_budget::enter_test_estimate_charge(self.estimate_charge, self.tile_charge);
            let prev_scratch = MOCK_SCRATCH.with(|m| m.replace(self.scratch));
            TestMemoryGuard { prev, prev_charge, prev_scratch }
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
            match evaluate_psd_admission_for(Platform::Native, u64::MAX, 0, 0, approved, estimate, MemorySnapshot::UNKNOWN) {
                Err(IoError::PsdImportBudget { required, limit }) => {
                    assert_eq!((required, limit), (u64::MAX, u64::MAX), "the typed refusal keeps the estimate and limit unchanged")
                }
                other => panic!("approved={approved}, estimate={estimate:?}: expected a hard refusal, got {other:?}"),
            }
        }

        // A caller limit equal to the sentinel, or an approved estimate of it, is no exception.
        assert!(matches!(
            evaluate_psd_admission_for(Platform::Native, u64::MAX, 0, u64::MAX, true, Some(u64::MAX), MemorySnapshot::UNKNOWN),
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

        let err = evaluate_psd_admission_for(Platform::Native, required, 0, 0, false, None, snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { available_ram: None, .. }));

        let out = evaluate_psd_admission_for(Platform::Native, required, 0, 0, true, Some(required), snap).unwrap();
        assert_eq!(out.authorized_total, required);
        assert!(out.budget.requires_uncertainty_warning(required), "the consent warning must carry through admission");

        let err = evaluate_psd_admission_for(Platform::Native, required, 0, 0, true, Some(required - 1), snap).unwrap_err();
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
        let out = evaluate_psd_admission_for(Platform::Web, WASM_MAX_PEAK_BUDGET, 0, 0, true, Some(WASM_MAX_PEAK_BUDGET), MemorySnapshot::UNKNOWN).unwrap();
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

        let out = evaluate_psd_admission_for(Platform::Web, WASM_BUDGET, 0, 0, false, None, snap).unwrap();
        assert_eq!(out.authorized_total, WASM_BUDGET);

        let err = evaluate_psd_admission_for(Platform::Web, WASM_BUDGET + 1, 0, 0, false, None, snap).unwrap_err();
        assert!(matches!(
            err,
            IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram }
                if required == WASM_BUDGET + 1 && soft_limit == WASM_BUDGET && hard_limit == WASM_MAX_PEAK_BUDGET && available_ram.is_none()
        ));

        let out = evaluate_psd_admission_for(Platform::Web, approval, 0, 0, true, Some(approval), snap).unwrap();
        assert_eq!(out.authorized_total, approval);

        let err = evaluate_psd_admission_for(Platform::Web, approval, 0, 0, true, Some(approval - 1), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { .. }));

        let err = evaluate_psd_admission_for(Platform::Web, WASM_MAX_PEAK_BUDGET + 1, 0, 0, true, None, snap).unwrap_err();
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
        let out = evaluate_psd_admission(5 * gb, 0, 0, false, None, snap).unwrap();
        assert_eq!(out.authorized_total, 6 * gb);

        // In [S, H]: requires approval
        let err = evaluate_psd_admission(7 * gb, 0, 0, false, None, snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { required, .. } if required == 7 * gb));

        // In [S, H] with approval and no approved_estimate bound: admitted, authorized_total = H (9 GiB)
        let out = evaluate_psd_admission(7 * gb, 0, 0, true, None, snap).unwrap();
        assert_eq!(out.authorized_total, 9 * gb);

        // In [S, H] with approval but required exceeds approved_estimate: requires fresh approval
        let err = evaluate_psd_admission(7 * gb, 0, 0, true, Some(6 * gb), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdApprovalRequired { .. }));

        // In [S, H] with approval and required within approved_estimate: admitted, authorized_total = approved_estimate (7 GiB)
        let out = evaluate_psd_admission(7 * gb, 0, 0, true, Some(7 * gb), snap).unwrap();
        assert_eq!(out.authorized_total, 7 * gb);

        // Exceeding H: refused even if high_memory_approved is true
        let err = evaluate_psd_admission(10 * gb, 0, 0, true, Some(10 * gb), snap).unwrap_err();
        assert!(matches!(err, IoError::PsdImportBudget { .. }));
    }

    /// While the scratch disk spills, tile pixels are admitted against RAM plus its room, and
    /// everything else against RAM alone.
    #[test]
    fn spilling_scratch_admits_tile_pixels_beyond_ram() {
        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let snap = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        // RAM-only: S = 6 GiB, H = 9 GiB.
        with_mock_scratch(Some(40 * gb), || {
            // 20 GiB of which 18 GiB tiles: 2 GiB non-spillable fits S, so it is automatic.
            let out = evaluate_psd_admission(20 * gb, 18 * gb, 0, false, None, snap).unwrap();
            assert_eq!(out.authorized_total, 6 * gb + 18 * gb);
            // 8 GiB non-spillable sits in [S, H]: approval is still required, and is bound to
            // the whole estimate.
            let err = evaluate_psd_admission(26 * gb, 18 * gb, 0, false, None, snap).unwrap_err();
            assert!(matches!(err, IoError::PsdApprovalRequired { required, soft_limit, hard_limit, .. }
                if required == 26 * gb && soft_limit == 24 * gb && hard_limit == 27 * gb));
            assert!(evaluate_psd_admission(26 * gb, 18 * gb, 0, true, Some(26 * gb), snap).is_ok());
            assert!(evaluate_psd_admission(26 * gb, 18 * gb, 0, true, Some(26 * gb - 1), snap).is_err());
            // 10 GiB non-spillable is over H: refused even when approved.
            let err = evaluate_psd_admission(28 * gb, 18 * gb, 0, true, Some(28 * gb), snap).unwrap_err();
            assert!(matches!(err, IoError::PsdImportBudget { required, limit } if required == 28 * gb && limit == 27 * gb));
            // A spillable share claimed beyond the total is clamped to the total.
            assert!(evaluate_psd_admission(5 * gb, u64::MAX, 0, false, None, snap).is_ok());
            // The caller's explicit limit still bounds the whole import.
            let err = evaluate_psd_admission(20 * gb, 18 * gb, 10 * gb, false, None, snap).unwrap_err();
            assert!(matches!(err, IoError::PsdImportBudget { limit, .. } if limit == 10 * gb));
        });
        // Less room than tiles: only the room is credited.
        with_mock_scratch(Some(4 * gb), || {
            let err = evaluate_psd_admission(20 * gb, 18 * gb, 0, true, Some(20 * gb), snap).unwrap_err();
            assert!(matches!(err, IoError::PsdImportBudget { limit, .. } if limit == 13 * gb));
            assert_eq!(resolve_budget_with_scratch(snap, None, 18 * gb).soft_limit, 10 * gb);
        });
    }

    /// Spilling off or paused, or the web: the RAM-only policy, unchanged.
    #[test]
    fn without_spilling_scratch_the_ram_policy_is_unchanged() {
        if cfg!(any(target_arch = "wasm32", target_pointer_width = "32")) {
            return;
        }
        let gb = 1u64 << 30;
        let snap = MemorySnapshot { available_ram: Some(12 * gb), swap_free: None, commit_headroom: None };
        for scratch in [None, Some(0)] {
            with_mock_scratch(scratch, || {
                assert_eq!(resolve_budget_with_scratch(snap, None, 18 * gb), resolve_budget(snap, None));
                let err = evaluate_psd_admission(20 * gb, 18 * gb, 0, true, Some(20 * gb), snap).unwrap_err();
                assert!(matches!(err, IoError::PsdImportBudget { limit, .. } if limit == 9 * gb));
                let err = evaluate_psd_admission(7 * gb, 7 * gb, 0, false, None, snap).unwrap_err();
                assert!(matches!(err, IoError::PsdApprovalRequired { soft_limit, hard_limit, .. } if soft_limit == 6 * gb && hard_limit == 9 * gb));
            });
        }
        // A mocked RAM snapshot without a mocked scratch disk never measures the real one.
        with_mock_memory(snap, || assert_eq!(scratch_capacity(), None));
        with_mock_scratch(Some(40 * gb), || {
            assert_eq!(scratch_credit_for(Platform::Web, 18 * gb), 0);
            let err = evaluate_psd_admission_for(Platform::Web, WASM_BUDGET + 1, WASM_BUDGET, 0, false, None, snap).unwrap_err();
            assert!(matches!(err, IoError::PsdApprovalRequired { soft_limit, .. } if soft_limit == WASM_BUDGET));
            let err = evaluate_psd_admission_for(Platform::Web, WASM_MAX_PEAK_BUDGET + 1, WASM_MAX_PEAK_BUDGET, 0, true, None, snap).unwrap_err();
            assert!(matches!(err, IoError::PsdImportBudget { .. }));
        });
    }

    /// A [`TestMemoryScope`] carries the mocked scratch disk to a worker thread.
    #[test]
    fn test_memory_scope_captures_the_mocked_scratch_disk() {
        let scope = with_mock_scratch(Some(123), TestMemoryScope::capture);
        std::thread::spawn(move || {
            assert_eq!(MOCK_SCRATCH.with(std::cell::Cell::get), None);
            let _g = scope.enter();
            assert_eq!(scratch_capacity(), Some(123));
        })
        .join()
        .unwrap();
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
    fn swap_is_never_part_of_the_adaptive_budget() {
        let gb = 1u64 << 30;
        let with_swap = MemorySnapshot { available_ram: Some(4 * gb), swap_free: Some(64 * gb), commit_headroom: None };
        let without_swap = MemorySnapshot { available_ram: Some(4 * gb), swap_free: None, commit_headroom: None };
        assert_eq!(resolve_budget_for(Platform::Native, with_swap, None), resolve_budget_for(Platform::Native, without_swap, None));
    }
}
