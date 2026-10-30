//! Small helpers shared by the Cutout stages.

use std::hash::Hasher;

use super::Px;
use crate::par;

/// Row-major `w`×`h` colours as `h`×`w` (columns become rows), so a pass along rows can run
/// along columns too.
pub(super) fn transpose(buf: &[Px], w: usize, h: usize) -> Vec<Px> {
    let mut out = vec![[0.0f32; 3]; w * h];
    par::chunks_mut(&mut out, h, true, |x, col| {
        for (y, p) in col.iter_mut().enumerate() {
            *p = buf[y * w + x];
        }
    });
    out
}

/// [`transpose`] for region ids.
pub(super) fn transpose_u32(buf: &[u32], w: usize, h: usize) -> Vec<u32> {
    let mut out = vec![0u32; w * h];
    for (x, col) in out.chunks_mut(h).enumerate() {
        for (y, p) in col.iter_mut().enumerate() {
            *p = buf[y * w + x];
        }
    }
    out
}

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
    fn transposing_twice_gives_the_image_back() {
        let (w, h) = (5usize, 3usize);
        let c: Vec<Px> = (0..w * h).map(|i| [i as f32, 0.0, 0.0]).collect();
        let t = transpose(&c, w, h);
        assert_eq!(t[1], c[w], "the second value of the first column is the first of the second row");
        assert_eq!(transpose(&t, h, w), c);
        let ids: Vec<u32> = (0..(w * h) as u32).collect();
        assert_eq!(transpose_u32(&ids, w, h), t.iter().map(|p| p[0] as u32).collect::<Vec<_>>());
        assert_eq!(transpose_u32(&transpose_u32(&ids, w, h), h, w), ids);
    }

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
