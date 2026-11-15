//! Optional import reports use existing hashed blobs and have no execution semantics.

use std::io::Write;
use std::sync::Arc;

use photocraft_doc::import_conversion::{
    CONVERSION_SCHEMA_VERSION, ConversionReport, ConversionValidationError, ImportConversion, MAX_CONVERSION_ATTACHMENTS, MAX_CONVERSION_BYTES,
};

use crate::convert::{Fetch, Sink};
use crate::manifest::ImportConversionM;
use crate::{FormatError, Result};

/// Preserve the distinction between report resource limits and corrupt metadata in native-format errors.
fn validation(error: ConversionValidationError) -> FormatError {
    match error {
        ConversionValidationError::Limit(_) => FormatError::LimitExceeded(error.to_string()),
        ConversionValidationError::Invalid(_) => FormatError::Corrupt(error.to_string()),
    }
}

/// Avoid allocating a whole oversized serialized report before checking its encoded size.
struct RecordBytes(Vec<u8>);

impl Write for RecordBytes {
    /// Admit serialized report bytes before appending, preventing escaped JSON from exceeding the encoded-record
    /// ceiling.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_CONVERSION_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("import conversion record exceeds 32 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    /// Complete the in-memory report writer without introducing file or resource side effects.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Validate and bound inert report records and attachments before storing them through native hashed blobs.
pub(crate) fn to_manifest(report: &ImportConversion, sink: &mut dyn Sink) -> Result<ImportConversionM> {
    report.validate().map_err(validation)?;
    let (record, payloads) = match report {
        ImportConversion::Report(report) => {
            let mut encoded = RecordBytes(Vec::new());
            serde_json::to_writer(&mut encoded, report).map_err(|e| FormatError::LimitExceeded(format!("import conversion record: {e}")))?;
            (Arc::new(encoded.0), report.attachments.iter().map(|a| a.bytes.clone()).collect::<Vec<_>>())
        }
        ImportConversion::Opaque { record, attachments, .. } => (record.clone(), attachments.clone()),
    };
    let mut remaining = MAX_CONVERSION_BYTES.checked_sub(record.len()).ok_or_else(|| FormatError::LimitExceeded("import conversion record".into()))?;
    for bytes in &payloads {
        remaining = remaining.checked_sub(bytes.len()).ok_or_else(|| FormatError::LimitExceeded("import conversion payloads exceed 32 MiB".into()))?;
    }
    Ok(ImportConversionM { schema_version: report.schema_version(), record: sink.blob(&record), attachments: payloads.iter().map(|b| sink.blob(b)).collect() })
}

/// Load bounded known or opaque future reports, validating framing and attachment counts without interpreting
/// retained data.
pub(crate) fn from_manifest(m: &ImportConversionM, fetch: &mut dyn Fetch) -> Result<Arc<ImportConversion>> {
    if m.schema_version == 0 {
        return Err(FormatError::corrupt("import conversion schema version is zero"));
    }
    if m.attachments.len() > MAX_CONVERSION_ATTACHMENTS {
        return Err(FormatError::LimitExceeded("too many import conversion attachments".into()));
    }
    let record = fetch.blob_bounded(&m.record, MAX_CONVERSION_BYTES)?;
    let known = if m.schema_version == CONVERSION_SCHEMA_VERSION {
        let report: ConversionReport = serde_json::from_slice(&record)?;
        report.validate().map_err(validation)?;
        if report.attachments.len() != m.attachments.len() {
            return Err(FormatError::corrupt("import conversion attachment count differs from record"));
        }
        Some(report)
    } else {
        None
    };
    let mut remaining = MAX_CONVERSION_BYTES.saturating_sub(record.len());
    let mut attachments = Vec::new();
    for hash in &m.attachments {
        let bytes = fetch.blob_bounded(hash, remaining)?;
        remaining = remaining.checked_sub(bytes.len()).ok_or_else(|| FormatError::LimitExceeded("import conversion payloads exceed 32 MiB".into()))?;
        attachments.push(bytes);
    }
    let report = if let Some(mut report) = known {
        for (attachment, bytes) in report.attachments.iter_mut().zip(attachments) {
            attachment.bytes = bytes;
        }
        ImportConversion::Report(report)
    } else {
        ImportConversion::Opaque { schema_version: m.schema_version, record, attachments }
    };
    report.validate().map_err(validation)?;
    Ok(Arc::new(report))
}
