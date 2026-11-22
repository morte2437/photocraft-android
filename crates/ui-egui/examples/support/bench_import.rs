//! How the benches open the PSD/PSB files they time.
//!
//! The app admits a PSD import against a memory budget sized from the RAM free at that moment
//! (`photocraft_io::memory`), and a file over the hard limit is refused with no way to approve
//! it. The benches time decoding, not admission. On a small or busy machine, such as the 7 GB
//! hosted macOS runner of the nightly perf job after the earlier benches, that budget refused the
//! 36 MP PSD and the 2 GB PSD and 4.4 GB PSB fixtures, which the same runner opened before the
//! budget existed, so their rows went missing.
//!
//! So the benches open their files under an explicit budget: the app's own policy for a machine
//! whose free memory can't be measured (`MemorySnapshot::UNKNOWN`: up to 2 GiB opens
//! automatically, more needs the high-memory approval, and there is no measured ceiling), with that
//! approval given up front, as `file.openAs` with `approveHighMemory` does. Only the calling
//! thread sees it (a `testgen` seam, so never in the app). `photocraft-io`'s own tests cover the
//! real admission policy, and every bench row records its peak resident memory.

use photocraft_io::memory::{MemorySnapshot, with_mock_memory};
use photocraft_io::{ImportOptions, ImportResult};

/// Imports `bytes` as `name` under the benchmark budget. A refusal or a decode error comes back as
/// a message that names the file, for the bench to record as that row's error.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, String> {
    let opts = ImportOptions { psd_high_memory_approved: true, ..ImportOptions::default() };
    with_mock_memory(MemorySnapshot::UNKNOWN, || photocraft_io::import_with_options(name, bytes, &opts)).map_err(|e| format!("{name}: {e}"))
}
