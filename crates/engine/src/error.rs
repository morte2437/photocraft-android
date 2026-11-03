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
    #[error("cannot open “{name}”: its estimated memory requirement of {required} bytes cannot be admitted within the {limit} byte hard limit on this machine")]
    PsdHardBudget { name: String, required: u64, limit: u64 },
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
        IoError::PsdImportBudget { required, limit } => EngineError::PsdHardBudget { name: name.to_string(), required, limit },
        IoError::SourceChanged => EngineError::SourceChanged { name: name.to_string() },
        error => EngineError::Other(format!("{name}: {error}")),
    }
}

impl From<photocraft_raster::AllocationError> for EngineError {
    fn from(error: photocraft_raster::AllocationError) -> Self {
        Self::Other(format!("not enough memory for this operation ({error}); the document was not changed"))
    }
}
