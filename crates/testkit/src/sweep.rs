//! Long test sweeps spread over several days.

use std::time::{SystemTime, UNIX_EPOCH};

/// The cases of a long sweep that this run covers. By default a run takes one of `parts` slices,
/// chosen by the UTC date, so every case runs at least once every `parts` days.
/// `PHOTOCRAFT_FULL_SWEEP=1` runs every case, and `PHOTOCRAFT_SWEEP_PART=<n>` replays slice `n`;
/// [`Sweep::new`] prints the slice it picked.
#[derive(Debug, Clone, Copy)]
pub struct Sweep {
    part: u64,
    parts: u64,
}

impl Sweep {
    pub fn new(parts: u64) -> Self {
        if std::env::var_os("PHOTOCRAFT_FULL_SWEEP").is_some_and(|v| v == "1") {
            eprintln!("full sweep (PHOTOCRAFT_FULL_SWEEP=1)");
            return Self { part: 0, parts: 1 };
        }
        let parts = parts.max(1);
        let part = match std::env::var("PHOTOCRAFT_SWEEP_PART") {
            Ok(v) => {
                let p = v.parse::<u64>().unwrap_or(u64::MAX);
                assert!(p < parts, "PHOTOCRAFT_SWEEP_PART={v:?} is not a part number below {parts}");
                p
            }
            Err(_) => SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or_default() % parts,
        };
        eprintln!("sweep part {part} of {parts}: replay with PHOTOCRAFT_SWEEP_PART={part}, run every part with PHOTOCRAFT_FULL_SWEEP=1");
        Self { part, parts }
    }

    /// Whether case `i` runs. Cases go to slices by a hash of `i`, so each slice mixes every
    /// position of the sweep's inner loops instead of taking every `parts`-th one.
    pub fn includes(&self, i: u64) -> bool {
        self.parts == 1 || mix(i) % self.parts == self.part
    }
}

/// splitmix64's finalizer.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_cover_every_case_once() {
        let parts: Vec<Sweep> = (0..4).map(|part| Sweep { part, parts: 4 }).collect();
        for i in 0..1000 {
            assert_eq!(parts.iter().filter(|s| s.includes(i)).count(), 1, "case {i}");
        }
        assert!(parts.iter().all(|s| (0..1000).filter(|&i| s.includes(i)).count() > 200));
    }
}
