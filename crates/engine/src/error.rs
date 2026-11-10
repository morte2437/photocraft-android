use super::LayerId;
use crate::command_hints;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// Displays with the nearest registered ids, if any ([`command_hints`], #3674).
    #[error("unknown command `{id}`{hint}", id = .0, hint = command_hints::unknown_command_hint(.0))]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such layer {0:?}")]
    NoLayer(LayerId),
    /// [`Self::NoLayer`] as a command reports it: with the layers the document does have, as
    /// ids are not positions (#5058).
    #[error("no such layer {}; {known}", id.0)]
    UnknownLayer { id: LayerId, known: String },
    /// Opening a PSD/PSB file requires explicit high-memory confirmation.
    #[error("opening “{name}” requires explicit memory approval: {required} bytes required (soft limit {soft_limit}, hard limit {hard_limit})")]
    PsdApprovalRequired { name: String, required: u64, soft_limit: u64, hard_limit: u64, available_ram: Option<u64> },
    /// The file's estimated memory cannot be admitted within the hard ceiling, which no approval
    /// can raise. The requirement may equal the ceiling when the estimate saturated.
    ///
    /// `free_memory` is the free memory measured when it set the limit, if it did.
    #[error(
        "cannot open “{name}”: its estimated memory requirement of {required_text} ({required} bytes) cannot be admitted within the {limit_text} ({limit} byte) hard limit{scope}{scratch}{advice}",
        required_text = bytes_text(.required), limit_text = bytes_text(.limit), scope = build_limit_scope(build_memory_limit()),
        scratch = unmeasured_scratch_hint(scratch_room_unmeasured()), advice = photocraft_io::memory::low_memory_advice(.free_memory)
    )]
    PsdHardBudget { name: String, required: u64, limit: u64, free_memory: Option<u64> },
    /// The file behind an opened handle is not the one whose memory estimate was approved.
    /// The document is discarded and is not opened.
    #[error("“{name}” changed since it was checked; open it again to approve the new file")]
    SourceChanged { name: String },
    #[error("{0}")]
    Other(String),
    /// A background job was cancelled (see [`jobs`]); nothing changed.
    #[error("cancelled")]
    Cancelled,
}

/// Where this build's hard memory limit comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildMemoryLimit {
    /// Measured from the computer's memory (64-bit desktop).
    Measured,
    /// The 32-bit desktop build: its cap, or the computer's memory if lower.
    Desktop32Bit,
    /// Fixed by the browser build.
    FixedWeb,
}

/// This build's memory-limit source, from its target.
fn build_memory_limit() -> BuildMemoryLimit {
    memory_limit_for(cfg!(target_arch = "wasm32"), cfg!(target_pointer_width = "32"))
}

/// The limit source for a target. The wasm32 check comes first: wasm32 is also 32-bit.
fn memory_limit_for(is_wasm32: bool, is_32_bit: bool) -> BuildMemoryLimit {
    if is_wasm32 {
        BuildMemoryLimit::FixedWeb
    } else if is_32_bit {
        BuildMemoryLimit::Desktop32Bit
    } else {
        BuildMemoryLimit::Measured
    }
}

/// The text after "hard limit": where the limit comes from and, where another build could open
/// the file, where to get it. Its sentences stand apart so the action isn't lost in the cause.
fn build_limit_scope(limit: BuildMemoryLimit) -> String {
    let releases = crate::updates_cmds::RELEASES;
    match limit {
        BuildMemoryLimit::Measured => " on this machine".to_owned(),
        BuildMemoryLimit::Desktop32Bit => format!(
            " on this machine. This 32-bit build of PhotoCraft can use at most 2 GiB of memory for a file, or less if the computer has less memory free. The 64-bit build may be able to open it: download it from {releases}"
        ),
        BuildMemoryLimit::FixedWeb => format!(" of the browser version of PhotoCraft. To open it, download the desktop app from {releases}"),
    }
}

/// The current scratch configuration, not the admission-time cause of a refusal.
fn scratch_room_unmeasured() -> bool {
    let stats = photocraft_raster::spill::stats();
    stats.free_space_unmeasured && stats.size_cap_bytes.is_none()
}

/// Configuration guidance only: the refusal can instead be bound by a caller or build limit,
/// or by memory that cannot spill. Current scratch stats cannot identify that cause.
fn unmeasured_scratch_hint(unmeasured: bool) -> &'static str {
    if unmeasured {
        ". Current scratch configuration: free space can't be measured and no Scratch Size Limit is set. To configure scratch capacity, set a Scratch Size Limit in Preferences > Scratch Disks, or use a disk whose free space can be measured. Scratch capacity cannot override caller or build limits, or replace memory that cannot spill to disk"
    } else {
        ""
    }
}

/// A size for people, next to the exact byte count: "21.5 GiB", "900 MiB", "17 B".
fn bytes_text(bytes: &u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    if *bytes >= GIB {
        format!("{:.1} GiB", *bytes as f64 / GIB as f64)
    } else if *bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} B")
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

impl EngineError {
    /// Whether an approval-required error also carries the unknown-capacity warning: capacity was
    /// unknown when the estimate was made and the estimate is above the warning threshold. A
    /// fresh measurement or a known capacity never sets it, and no number is invented for a
    /// capacity that could not be measured.
    pub fn uncertain_capacity_warning(&self) -> bool {
        match self {
            Self::PsdApprovalRequired { required, available_ram, .. } => photocraft_io::memory::uncertain_capacity_warning(*required, *available_ram),
            _ => false,
        }
    }
}

/// Maps an io open/import failure to the engine's typed errors, preserving the memory fields the
/// UI, control channel and agents need to answer or explain it.
pub(crate) fn io_open_error(name: &str, error: photocraft_io::IoError) -> EngineError {
    use photocraft_io::IoError;
    match error {
        IoError::PsdApprovalRequired { required, soft_limit, hard_limit, available_ram } => {
            EngineError::PsdApprovalRequired { name: name.to_string(), required, soft_limit, hard_limit, available_ram }
        }
        IoError::PsdImportBudget { required, limit, free_memory } => EngineError::PsdHardBudget { name: name.to_string(), required, limit, free_memory },
        IoError::SourceChanged => EngineError::SourceChanged { name: name.to_string() },
        error => EngineError::Other(format!("{name}: {error}")),
    }
}

impl From<photocraft_raster::AllocationError> for EngineError {
    fn from(error: photocraft_raster::AllocationError) -> Self {
        Self::Other(format!("not enough memory for this operation ({error}); the document was not changed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hard(free_memory: Option<u64>) -> EngineError {
        EngineError::PsdHardBudget { name: "big.psd".into(), required: 9000, limit: 4000, free_memory }
    }

    #[test]
    fn hard_budget_message_tells_the_user_to_free_memory_when_it_was_measured() {
        let message = hard(Some(900 << 20)).to_string();
        assert!(message.ends_with("Close other apps and try again"), "{message}");
        assert!(message.contains("900 MiB"), "{message}");

        let message = hard(Some(0)).to_string();
        assert!(message.contains("measured no free memory"), "{message}");

        let message = hard(None).to_string();
        assert!(message.contains(&build_limit_scope(build_memory_limit())), "{message}");
        assert!(!message.contains("Close other apps"), "{message}");
    }

    #[test]
    fn io_open_error_carries_the_free_memory_through() {
        for free_memory in [None, Some(0), Some(3 << 30)] {
            let error = photocraft_io::IoError::PsdImportBudget { required: 9000, limit: 4000, free_memory };
            match io_open_error("big.psd", error) {
                EngineError::PsdHardBudget { required: 9000, limit: 4000, free_memory: carried, .. } => assert_eq!(carried, free_memory),
                other => panic!("expected the typed hard refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn hard_budget_scope_text_per_build() {
        assert_eq!(build_limit_scope(BuildMemoryLimit::Measured), " on this machine");
        let releases = crate::updates_cmds::RELEASES;
        assert_eq!(
            build_limit_scope(BuildMemoryLimit::Desktop32Bit),
            format!(
                " on this machine. This 32-bit build of PhotoCraft can use at most 2 GiB of memory for a file, or less if the computer has less memory free. The 64-bit build may be able to open it: download it from {releases}"
            )
        );
        assert_eq!(
            build_limit_scope(BuildMemoryLimit::FixedWeb),
            format!(" of the browser version of PhotoCraft. To open it, download the desktop app from {releases}")
        );
    }

    #[test]
    fn unmeasured_scratch_hint_only_when_unmeasured() {
        assert_eq!(unmeasured_scratch_hint(false), "");
        let hint = unmeasured_scratch_hint(true);
        assert!(hint.starts_with(". Current scratch configuration:"), "{hint}");
        assert!(hint.contains("Scratch Size Limit in Preferences > Scratch Disks") && hint.contains("disk whose free space can be measured"), "{hint}");
    }

    #[test]
    fn scratch_configuration_hint_does_not_promise_to_raise_a_binding_limit() {
        let hint = unmeasured_scratch_hint(true);
        // A caller-bound refusal can carry the same live scratch state as a capacity refusal.
        // The note must describe that state without attributing this file's refusal to it.
        assert!(hint.contains("cannot override caller or build limits"), "{hint}");
        assert!(hint.contains("memory that cannot spill to disk"), "{hint}");
        assert!(!hint.contains("for this file") && !hint.contains("so it adds no room"), "{hint}");
    }

    #[test]
    fn hard_budget_message_names_file_numbers_and_scope() {
        let text = EngineError::PsdHardBudget { name: "big.psd".into(), required: 23_049_071_616, limit: 2_147_483_648, free_memory: None }.to_string();
        for part in ["big.psd", "21.5 GiB", "23049071616 bytes", "2.0 GiB", "2147483648 byte", "hard limit"] {
            assert!(text.contains(part), "{text}");
        }
        assert!(text.contains(&build_limit_scope(build_memory_limit())), "{text}");
    }

    #[test]
    fn bytes_text_picks_a_readable_unit() {
        assert_eq!(bytes_text(&17), "17 B");
        assert_eq!(bytes_text(&(900 << 20)), "900 MiB");
        assert_eq!(bytes_text(&(2 << 30)), "2.0 GiB");
        assert_eq!(bytes_text(&23_049_071_616), "21.5 GiB");
        assert!(bytes_text(&u64::MAX).ends_with(" GiB"));
    }

    #[test]
    fn memory_limit_follows_the_target() {
        assert_eq!(memory_limit_for(true, true), BuildMemoryLimit::FixedWeb);
        assert_eq!(memory_limit_for(true, false), BuildMemoryLimit::FixedWeb);
        assert_eq!(memory_limit_for(false, true), BuildMemoryLimit::Desktop32Bit);
        assert_eq!(memory_limit_for(false, false), BuildMemoryLimit::Measured);
    }

    #[test]
    fn two_gib_in_the_text_matches_the_cap() {
        assert_eq!(photocraft_io::memory::NATIVE_32BIT_PEAK_CAP, 2 << 30);
    }
}
