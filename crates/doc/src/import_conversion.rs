//! Historical import outcomes. None of this data drives rendering, editing or resource access.

use std::fmt;
use std::sync::Arc;

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::LayerId;

pub const CONVERSION_SCHEMA_VERSION: u32 = 1;
/// Separate from an input reader's existing metadata allowance.
pub const MAX_CONVERSION_BYTES: usize = 32 << 20;
pub const MAX_CONVERSION_STRING: usize = 1 << 20;
pub const MAX_CONVERSION_ENTRIES: usize = 65_536;
pub const MAX_CONVERSION_ATTACHMENTS: usize = 4_096;
const MAX_ITEM_PATH: usize = 64;
// Schema-v1 accounting units, independent of pointer width and Rust layout. Keep
// these conservative fixed costs so web and desktop accept the same reports.
const ENTRY_BYTES: usize = 256;
const TARGET_BYTES: usize = 128;
const ATTACHMENT_BYTES: usize = 160;

/// An optional future report is retained without interpreting its record or binary payloads.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportConversion {
    Report(ConversionReport),
    Opaque { schema_version: u32, record: Arc<Vec<u8>>, attachments: Vec<Arc<Vec<u8>>> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionReport {
    pub source_format: String,
    pub source_version: Option<u32>,
    pub policy_version: u32,
    #[serde(deserialize_with = "entries")]
    pub entries: Vec<ConversionEntry>,
    #[serde(deserialize_with = "attachments")]
    pub attachments: Vec<RetainedImportData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionEntry {
    pub feature_code: String,
    pub source: SourceItemRef,
    #[serde(deserialize_with = "targets")]
    pub targets: Vec<NativeTargetRef>,
    pub disposition: ConversionDisposition,
    pub reason: ConversionReason,
    pub source_description: String,
    pub native_description: String,
    #[serde(deserialize_with = "indices")]
    pub attachments: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionDisposition {
    NativeEquivalent,
    NativeApproximation,
    RetainedUnapplied,
    SkippedNonessential,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionReason {
    NativePolicy,
    NativeLimitation,
    MissingMapping,
    ResourceUnavailable,
    InvalidOptionalData,
    RetentionLimit,
    NumericalConversion,
}

/// Ordinals/paths identify the source snapshot, including duplicate display names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceItemRef {
    pub kind: String,
    pub ordinal: u32,
    #[serde(deserialize_with = "item_path")]
    pub item_path: Vec<u32>,
    pub tattoo: Option<u32>,
    pub record_offset: Option<u64>,
    pub name: String,
}

/// Import-time references may become stale; they are never remapped into live instructions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTargetRef {
    pub layer_id: Option<LayerId>,
    #[serde(deserialize_with = "item_path")]
    pub item_path: Vec<u32>,
    pub channel_index: Option<u32>,
    pub path_index: Option<u32>,
    pub filter_index: Option<u32>,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetainedImportData {
    pub owner: SourceItemRef,
    pub kind: String,
    pub encoding: String,
    /// Native format stores this separately in its hashed blob store, never as JSON bytes.
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionValidationError {
    Limit(&'static str),
    Invalid(&'static str),
}

impl fmt::Display for ConversionValidationError {
    /// Render bounded validation failures as actionable import-report errors without looking up live document
    /// targets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit(message) | Self::Invalid(message) => write!(f, "import conversion: {message}"),
        }
    }
}

impl std::error::Error for ConversionValidationError {}

type Validation = Result<(), ConversionValidationError>;

struct Budget(usize);

impl Budget {
    /// Debit the report's independent byte allowance and reject overflow before accepting more retained data.
    fn charge(&mut self, n: usize) -> Validation {
        self.0 = self.0.checked_sub(n).ok_or(ConversionValidationError::Limit("report exceeds the separate 32 MiB allowance"))?;
        Ok(())
    }

    /// Enforce the per-string ceiling before charging its UTF-8 bytes to the report budget.
    fn string(&mut self, s: &str) -> Validation {
        if s.len() > MAX_CONVERSION_STRING {
            return Err(ConversionValidationError::Limit("report string exceeds 1 MiB"));
        }
        self.charge(s.len())
    }

    /// Bound source/target path depth and charge its stored components without traversing a document.
    fn path(&mut self, path: &[u32]) -> Validation {
        if path.len() > MAX_ITEM_PATH {
            return Err(ConversionValidationError::Limit("item path exceeds 64 components"));
        }
        self.charge(std::mem::size_of_val(path))
    }

    /// Account for an item's kind, display name and historical path as inert report metadata.
    fn source(&mut self, source: &SourceItemRef) -> Validation {
        self.string(&source.kind)?;
        self.string(&source.name)?;
        self.path(&source.item_path)
    }
}

impl ImportConversion {
    /// Expose the known report schema or preserve an opaque future record's declared version.
    pub fn schema_version(&self) -> u32 {
        match self {
            Self::Report(_) => CONVERSION_SCHEMA_VERSION,
            Self::Opaque { schema_version, .. } => *schema_version,
        }
    }

    /// Validate before saving or installing data; no current document/target lookup is needed.
    pub fn validate(&self) -> Validation {
        match self {
            Self::Report(report) => report.validate(),
            Self::Opaque { schema_version, record, attachments } => {
                if *schema_version <= CONVERSION_SCHEMA_VERSION {
                    return Err(ConversionValidationError::Invalid("opaque report must have a newer schema version"));
                }
                if attachments.len() > MAX_CONVERSION_ATTACHMENTS {
                    return Err(ConversionValidationError::Limit("too many report attachments"));
                }
                let mut budget = Budget(MAX_CONVERSION_BYTES);
                budget.charge(record.len())?;
                for bytes in attachments {
                    budget.charge(bytes.len())?;
                }
                Ok(())
            }
        }
    }
}

impl ConversionReport {
    /// Validate report identities, references, counts and combined retained bytes without granting those references
    /// authority.
    pub fn validate(&self) -> Validation {
        if self.source_format.is_empty() {
            return Err(ConversionValidationError::Invalid("source format is empty"));
        }
        if self.entries.len() > MAX_CONVERSION_ENTRIES || self.attachments.len() > MAX_CONVERSION_ATTACHMENTS {
            return Err(ConversionValidationError::Limit("too many report entries or attachments"));
        }
        let mut budget = Budget(MAX_CONVERSION_BYTES);
        budget.string(&self.source_format)?;
        for entry in &self.entries {
            if entry.feature_code.is_empty() || entry.source.kind.is_empty() {
                return Err(ConversionValidationError::Invalid("entry feature code or source kind is empty"));
            }
            budget.charge(ENTRY_BYTES)?;
            budget.string(&entry.feature_code)?;
            budget.source(&entry.source)?;
            budget.string(&entry.source_description)?;
            budget.string(&entry.native_description)?;
            if entry.targets.len() > MAX_CONVERSION_ENTRIES || entry.attachments.len() > MAX_CONVERSION_ATTACHMENTS {
                return Err(ConversionValidationError::Limit("too many item targets or attachment references"));
            }
            for target in &entry.targets {
                budget.charge(TARGET_BYTES)?;
                budget.string(&target.label)?;
                budget.path(&target.item_path)?;
            }
            for index in &entry.attachments {
                if usize::try_from(*index).ok().is_none_or(|index| index >= self.attachments.len()) {
                    return Err(ConversionValidationError::Invalid("attachment reference is out of range"));
                }
                budget.charge(std::mem::size_of::<u32>())?;
            }
        }
        for attachment in &self.attachments {
            budget.charge(ATTACHMENT_BYTES)?;
            budget.source(&attachment.owner)?;
            budget.string(&attachment.kind)?;
            budget.string(&attachment.encoding)?;
            budget.charge(attachment.bytes.len())?;
        }
        Ok(())
    }
}

// Limit element counts during deserialization, before accepting the next allocation.
/// Deserialize at most the declared number of items and reject an extra element before accepting another
/// allocation.
fn bounded_sequence<'de, T: Deserialize<'de>, D: Deserializer<'de>, const MAX: usize>(d: D) -> Result<Vec<T>, D::Error> {
    struct Bounded<T, const MAX: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for Bounded<T, MAX> {
        type Value = Vec<T>;

        /// Describe the bounded report-array contract in deserialization errors.
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "an import conversion array of at most {MAX} items")
        }

        /// Collect only the allowed sequence length and reject a trailing element instead of silently truncating
        /// it.
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX {
                match seq.next_element()? {
                    Some(value) => out.push(value),
                    None => return Ok(out),
                }
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("import conversion array exceeds its count limit"));
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Bounded::<T, MAX>(std::marker::PhantomData))
}

/// Apply the report-entry count limit during deserialization rather than after collecting unbounded input.
fn entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ConversionEntry>, D::Error> {
    bounded_sequence::<'de, _, _, MAX_CONVERSION_ENTRIES>(d)
}
/// Bound retained attachment records while decoding their metadata array.
fn attachments<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RetainedImportData>, D::Error> {
    bounded_sequence::<'de, _, _, MAX_CONVERSION_ATTACHMENTS>(d)
}
/// Limit native target references while preserving their historical, non-executable meaning.
fn targets<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<NativeTargetRef>, D::Error> {
    bounded_sequence::<'de, _, _, MAX_CONVERSION_ENTRIES>(d)
}
/// Bound attachment-reference counts before later report validation resolves their indices.
fn indices<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    bounded_sequence::<'de, _, _, MAX_CONVERSION_ATTACHMENTS>(d)
}
/// Reject historical item paths deeper than the report format permits during decoding.
fn item_path<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    bounded_sequence::<'de, _, _, MAX_ITEM_PATH>(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the schema's byte boundary to fixed units rather than the host's Rust struct layout.
    #[test]
    fn report_budget_is_identical_on_web_and_desktop() {
        let mut report = ConversionReport {
            source_format: "xcf".into(),
            entries: vec![ConversionEntry {
                feature_code: "f".into(),
                source: SourceItemRef { kind: "layer".into(), item_path: vec![1, 2], ..Default::default() },
                targets: vec![NativeTargetRef { item_path: vec![3], label: "t".into(), ..Default::default() }],
                disposition: ConversionDisposition::RetainedUnapplied,
                reason: ConversionReason::MissingMapping,
                source_description: String::new(),
                native_description: String::new(),
                attachments: vec![0],
            }],
            attachments: vec![RetainedImportData {
                owner: SourceItemRef::default(),
                kind: "raw".into(),
                encoding: "binary".into(),
                bytes: Arc::new(Vec::new()),
            }],
            ..Default::default()
        };
        // Literal schema costs plus UTF-8 bytes, three u32 path components and one index.
        let overhead = 256 + 128 + 160 + 3 + 1 + 5 + 1 + 3 + 6 + 12 + 4;
        report.attachments[0].bytes = Arc::new(vec![0; MAX_CONVERSION_BYTES - overhead]);
        assert_eq!(report.validate(), Ok(()));
        Arc::make_mut(&mut report.attachments[0].bytes).push(0);
        assert!(matches!(report.validate(), Err(ConversionValidationError::Limit(_))));
    }

    #[test]
    /// Check absent-report defaults and ensure historical source/target references do not become live document
    /// state.
    fn reports_default_to_absent_and_have_no_document_authority() {
        assert!(crate::Metadata::default().import_conversion.is_none());
        let report = ImportConversion::Report(ConversionReport { source_format: "xcf".into(), ..Default::default() });
        assert!(report.validate().is_ok());
        assert_eq!(report.schema_version(), 1);
    }

    /// Document and history-style clones share the complete report, including entry strings and opaque future records.
    #[test]
    fn document_clones_share_known_and_opaque_conversion_reports() {
        let entry = ConversionEntry {
            feature_code: "xcf.source".into(),
            source: SourceItemRef { kind: "layer".into(), ..Default::default() },
            targets: Vec::new(),
            disposition: ConversionDisposition::RetainedUnapplied,
            reason: ConversionReason::MissingMapping,
            source_description: "source control ".repeat(64),
            native_description: "inert historical conversion information".into(),
            attachments: vec![0],
        };
        let known = ImportConversion::Report(ConversionReport {
            source_format: "xcf".into(),
            entries: vec![entry; 1024],
            attachments: vec![RetainedImportData {
                owner: SourceItemRef::default(),
                kind: "configuration".into(),
                encoding: "binary".into(),
                bytes: Arc::new(vec![17; 64 * 1024]),
            }],
            ..Default::default()
        });
        let opaque = ImportConversion::Opaque {
            schema_version: CONVERSION_SCHEMA_VERSION + 1,
            record: Arc::new(b"future report record".to_vec()),
            attachments: vec![Arc::new(vec![23; 64 * 1024])],
        };
        for report in [known, opaque] {
            report.validate().unwrap();
            let mut original =
                crate::Document::new("Shared report", crate::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
            original.metadata.import_conversion = Some(Arc::new(report));
            let cloned = original.clone();
            let first = original.metadata.import_conversion.as_ref().unwrap();
            let second = cloned.metadata.import_conversion.as_ref().unwrap();
            assert!(Arc::ptr_eq(first, second), "document clones must share the entire conversion report");
            if let ImportConversion::Report(report) = first.as_ref() {
                let ImportConversion::Report(copy) = second.as_ref() else { panic!("same shared variant") };
                assert_eq!(report.entries.as_ptr(), copy.entries.as_ptr(), "entry and string arrays are not deep-cloned");
                assert!(Arc::ptr_eq(&report.attachments[0].bytes, &copy.attachments[0].bytes));
            }
        }
    }

    #[test]
    /// Reject oversized strings and future opaque payloads under the independent report allowance.
    fn string_and_opaque_payload_limits_are_enforced() {
        let report = ConversionReport { source_format: "x".repeat(MAX_CONVERSION_STRING + 1), ..Default::default() };
        assert!(matches!(report.validate(), Err(ConversionValidationError::Limit(_))));
        let report = ImportConversion::Opaque { schema_version: 2, record: Arc::new(vec![0; MAX_CONVERSION_BYTES + 1]), attachments: Vec::new() };
        assert!(matches!(report.validate(), Err(ConversionValidationError::Limit(_))));
    }

    #[test]
    /// Verify oversized arrays fail in bounded deserialization instead of being accepted then truncated.
    fn counts_are_limited_during_deserialization() {
        let owner = SourceItemRef::default();
        let mut value = serde_json::to_value(&owner).unwrap();
        value["itemPath"] = serde_json::json!(vec![0; MAX_ITEM_PATH + 1]);
        assert!(serde_json::from_value::<SourceItemRef>(value).is_err());
    }

    #[test]
    /// Exercise attachment count and aggregate byte limits without borrowing the source metadata allowance.
    fn attachment_counts_and_totals_are_bounded_independently_of_source_metadata() {
        let empty = Arc::new(Vec::new());
        let count = ImportConversion::Opaque { schema_version: 2, record: empty.clone(), attachments: vec![empty; MAX_CONVERSION_ATTACHMENTS + 1] };
        assert!(count.validate().is_err());
        let chunk = Arc::new(vec![0; MAX_CONVERSION_BYTES / 2 + 1]);
        let total = ImportConversion::Opaque { schema_version: 2, record: chunk.clone(), attachments: vec![chunk] };
        assert!(matches!(total.validate(), Err(ConversionValidationError::Limit(_))));
    }
}
