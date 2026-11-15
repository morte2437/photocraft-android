//! Persistence model for deterministic GPU compositor build failures.
//!
//! Call [`GpuMemo::remember`] only for deterministic build failures. Transient device or
//! allocation errors are deliberately not classified here and are never saved automatically.

use serde::{Deserialize, Serialize};

/// Current on-disk schema version.
pub const GPU_MEMO_VERSION: u32 = 1;
/// Maximum accepted serialized document size.
pub const MAX_GPU_MEMO_BYTES: usize = 64 * 1024;
/// Number of failure records retained, newest last.
pub const MAX_GPU_MEMO_ENTRIES: usize = 8;

/// Identifies the application, shader set, and graphics stack that produced a failure.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct MemoKey {
    pub app_version: String,
    pub shader_fingerprint: u64,
    pub adapter_name: String,
    pub vendor: u32,
    pub device: u32,
    pub driver: String,
    pub driver_info: String,
    pub backend: String,
    pub shader_compiler: String,
}

/// One remembered deterministic build failure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoEntry {
    pub key: MemoKey,
    pub error: String,
    /// RFC3339 timestamp supplied by the caller.
    pub when: String,
}

/// Bounded, versioned compositor failure memo.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GpuMemo {
    version: u32,
    failures: Vec<MemoEntry>,
}

impl Default for GpuMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuMemo {
    /// Creates an empty memo using the current schema version.
    pub fn new() -> Self {
        Self { version: GPU_MEMO_VERSION, failures: Vec::new() }
    }

    /// Parse persisted JSON. Rejects oversized, malformed, and unknown-version data.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_GPU_MEMO_BYTES {
            return Err("GPU memo exceeds 64 KiB".into());
        }
        let mut memo: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if memo.version != GPU_MEMO_VERSION {
            return Err(format!("unsupported GPU memo version {}", memo.version));
        }
        if memo.failures.len() > MAX_GPU_MEMO_ENTRIES {
            let excess = memo.failures.len() - MAX_GPU_MEMO_ENTRIES;
            memo.failures.drain(..excess);
        }
        Ok(memo)
    }

    /// Serialize this memo as JSON.
    pub fn serialize(&self) -> Result<String, String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if text.len() > MAX_GPU_MEMO_BYTES {
            return Err("GPU memo exceeds 64 KiB".into());
        }
        Ok(text)
    }

    /// Returns the remembered failure for `key`, if present.
    pub fn get(&self, key: &MemoKey) -> Option<&MemoEntry> {
        self.failures.iter().rev().find(|entry| &entry.key == key)
    }

    /// Records a deterministic failure. Callers must exclude transient device-loss and OOM errors.
    /// Existing entries for the key are replaced and the oldest record is dropped at capacity.
    pub fn remember(&mut self, key: MemoKey, error: String, when: String) {
        if self.version != GPU_MEMO_VERSION {
            *self = Self::new();
        }
        self.remove(&key);
        if self.failures.len() == MAX_GPU_MEMO_ENTRIES {
            self.failures.remove(0);
        }
        self.failures.push(MemoEntry { key, error, when });
    }

    /// Removes a memoized failure for `key`; returns whether one existed.
    pub fn remove(&mut self, key: &MemoKey) -> bool {
        let before = self.failures.len();
        self.failures.retain(|entry| &entry.key != key);
        before != self.failures.len()
    }

    /// Number of retained failure records.
    pub fn len(&self) -> usize {
        self.failures.len()
    }

    /// Whether the memo has no records.
    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: usize) -> MemoKey {
        MemoKey { app_version: n.to_string(), ..MemoKey::default() }
    }

    #[test]
    fn round_trip_and_remove() {
        let mut memo = GpuMemo::new();
        memo.remember(key(1), "compiler failed".into(), "2026-10-28T00:00:00Z".into());
        let parsed = GpuMemo::parse(&memo.serialize().expect("serializes")).expect("parses");
        assert_eq!(parsed.get(&key(1)).map(|entry| entry.error.as_str()), Some("compiler failed"));
        let mut parsed = parsed;
        assert!(parsed.remove(&key(1)));
        assert!(!parsed.remove(&key(1)));
        assert!(parsed.is_empty());
    }

    #[test]
    fn changing_any_identity_field_invalidates_a_failure() {
        let original = MemoKey::default();
        let mut memo = GpuMemo::new();
        memo.remember(original.clone(), "compiler failed".into(), "now".into());
        let variants = [
            MemoKey { app_version: "new".into(), ..original.clone() },
            MemoKey { shader_fingerprint: 1, ..original.clone() },
            MemoKey { adapter_name: "new".into(), ..original.clone() },
            MemoKey { vendor: 1, ..original.clone() },
            MemoKey { device: 1, ..original.clone() },
            MemoKey { driver: "new".into(), ..original.clone() },
            MemoKey { driver_info: "new".into(), ..original.clone() },
            MemoKey { backend: "new".into(), ..original.clone() },
            MemoKey { shader_compiler: "new".into(), ..original.clone() },
        ];
        for variant in variants {
            assert!(memo.get(&variant).is_none());
        }
    }

    #[test]
    fn retains_only_eight_newest_entries() {
        let mut memo = GpuMemo::new();
        for n in 0..10 {
            memo.remember(key(n), "failure".into(), "now".into());
        }
        assert_eq!(memo.len(), MAX_GPU_MEMO_ENTRIES);
        assert!(memo.get(&key(0)).is_none());
        assert!(memo.get(&key(1)).is_none());
        assert!(memo.get(&key(2)).is_some());
        assert!(memo.get(&key(9)).is_some());
    }

    #[test]
    fn rejects_corrupt_oversized_and_unknown_versions() {
        assert!(GpuMemo::parse("{").is_err());
        assert!(GpuMemo::parse(&" ".repeat(MAX_GPU_MEMO_BYTES + 1)).is_err());
        assert!(GpuMemo::parse(r#"{"version":2,"failures":[]}"#).is_err());
    }
}
