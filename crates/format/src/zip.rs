//! Compatibility surface for callers that inspect ZIP archives.
//!
//! ZIP parsing, decompression, CRC checks, and writing are owned by `craft-zip`. This module only
//! adapts its bounded archive API to the older PhotoCraft call shape while downstream callers
//! migrate.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::FormatError;

/// An entry name from a ZIP archive.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The decoded entry path.
    pub name: String,
}

/// Read-only view of an archive in memory. The shared reader validates and decodes entries.
pub struct ZipReader<'a> {
    archive: Mutex<craft_zip::Archive<'a>>,
    /// Archive entries in central-directory order.
    pub entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

fn map_read_error(error: craft_zip::Error) -> FormatError {
    match error {
        craft_zip::Error::TooManyEntries { .. }
        | craft_zip::Error::TooManyEndRecords(_)
        | craft_zip::Error::EntryTooLarge { .. }
        | craft_zip::Error::BudgetSpent { .. } => FormatError::LimitExceeded(format!("zip: {error}")),
        _ => FormatError::corrupt(format!("zip: {error}")),
    }
}

impl<'a> ZipReader<'a> {
    /// Indexes an archive through craft-zip, with a bounded classic-format entry count.
    pub fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let archive = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: u16::MAX.into(), max_total_bytes: u64::MAX }).map_err(map_read_error)?;
        let entries: Vec<Entry> = archive.names().map(|name| Entry { name: name.into_owned() }).collect();
        let mut by_name = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            by_name.entry(entry.name.clone()).or_insert(index);
        }
        Ok(Self { archive: Mutex::new(archive), entries, by_name })
    }

    /// Finds the first entry with this name.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(name).and_then(|&index| self.entries.get(index))
    }

    /// Reads one entry through craft-zip, bounded by `max` decoded bytes.
    pub fn read(&self, entry: &Entry, max: usize) -> Result<Vec<u8>, FormatError> {
        let mut archive = self.archive.lock().map_err(|_| FormatError::corrupt("zip reader lock was poisoned"))?;
        archive.read(&entry.name, max as u64).map_err(map_read_error)?.ok_or_else(|| FormatError::corrupt(format!("missing `{}`", entry.name)))
    }

    /// Reads the named entry through craft-zip, bounded by `max` decoded bytes.
    pub fn read_by_name(&self, name: &str, max: usize) -> Result<Vec<u8>, FormatError> {
        let entry = self.find(name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        self.read(entry, max)
    }
}

/// Compatibility writer that adds stored entries through craft-zip.
#[derive(Default)]
pub struct ZipWriter {
    archive: craft_zip::Writer,
}

impl ZipWriter {
    /// Creates an empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a stored entry.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), FormatError> {
        self.archive.add(name, data, craft_zip::Compression::Stored).map_err(|error| FormatError::Unsupported(format!("zip: {error}")))
    }

    /// Finishes the archive.
    pub fn finish(self) -> Result<Vec<u8>, FormatError> {
        self.archive.finish().map_err(|error| FormatError::Unsupported(format!("zip: {error}")))
    }
}
