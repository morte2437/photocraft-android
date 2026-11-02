use super::LayerId;
use crate::command_hints;

#[derive(Debug, thiserror::Error)]
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
    #[error("{0}")]
    Other(String),
    /// A background job was cancelled (see [`jobs`]); nothing changed.
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, EngineError>;

impl From<photocraft_raster::AllocationError> for EngineError {
    fn from(error: photocraft_raster::AllocationError) -> Self {
        Self::Other(format!("not enough memory for this operation ({error}); the document was not changed"))
    }
}
