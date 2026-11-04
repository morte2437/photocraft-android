//! Small helpers shared by the Cutout stages.

use std::hash::Hasher;

/// Union-find root of `i`, halving the path on the way.
pub(super) fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let p = parent[i as usize];
        parent[i as usize] = parent[p as usize];
        i = p;
    }
    i
}

/// A fast deterministic hasher for the region adjacency map.
#[derive(Default)]
pub(super) struct FxHasher(u64);

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_returns_the_root_and_shortens_the_path() {
        // 4 -> 3 -> 2 -> 1 -> 0.
        let mut parent = vec![0, 0, 1, 2, 3];
        assert_eq!(find(&mut parent, 4), 0);
        assert_eq!(find(&mut parent, 0), 0);
        assert!(parent[4] < 3, "the path was halved: {parent:?}");
        assert!(parent.iter().all(|&p| p <= 2));
    }
}
