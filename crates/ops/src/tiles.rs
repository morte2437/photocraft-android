//! Tile references owned by history snapshots. The snapshots keep these addresses alive;
//! COW edits cannot change a retained tile in place. Count occurrences (including aliases
//! within one document), so removing a state releases a tile only after its last reference.
use photocraft_doc::{Document, LayerContent, SmartSource, Surface};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub(crate) struct Tiles {
    refs: HashMap<usize, (u128, usize)>,
    bytes: u128,
}

pub(crate) fn surfaces(doc: &Document, visit: impl FnMut(&Surface)) {
    // Undo retains the whole clip, not only the displayed frame. Video aliases are visited
    // too: Tiles tracks every owner and counts their shared allocations only once.
    doc.for_each_surface(visit);
}

/// Every shared allocation `doc` holds that history can retain, as `visit(address, length)`:
/// the tiles of [`surfaces`] and the embedded contents of smart objects (an `Arc` that undo
/// states and duplicated layers share, replaced by Replace Contents).
pub(crate) fn blocks(doc: &Document, mut visit: impl FnMut(usize, usize)) {
    surfaces(doc, |surface| {
        for (_, tile) in surface.tiles() {
            visit(Arc::as_ptr(tile) as usize, tile.len());
        }
    });
    for (_, _, layer) in doc.walk() {
        if let LayerContent::Smart(smart) = &layer.content
            && let SmartSource::Embedded { bytes, .. } = &smart.source
        {
            visit(Arc::as_ptr(bytes) as usize, bytes.len());
        }
    }
}

impl Tiles {
    pub(crate) fn add(&mut self, doc: &Document) {
        blocks(doc, |ptr, len| {
            let count = self.refs.entry(ptr).or_insert_with(|| {
                self.bytes += len as u128;
                (0, len)
            });
            count.0 += 1;
        });
    }

    pub(crate) fn remove(&mut self, doc: &Document) {
        blocks(doc, |ptr, _| {
            if let Some(count) = self.refs.get_mut(&ptr) {
                count.0 -= 1;
                if count.0 == 0 {
                    self.bytes -= count.1 as u128;
                    self.refs.remove(&ptr);
                }
            }
        });
    }

    /// Retained bytes are incremental; only current tiles need examining below the limit.
    /// Current can change without recording (coalescing), and tile transfers can carry a
    /// different byte length, so a tile-count estimate must not underestimate its memory.
    pub(crate) fn fits(&self, current: &Document, budget: usize) -> bool {
        self.bytes <= budget as u128 && self.bytes_with(current) <= budget as u128
    }

    pub(crate) fn bytes_with(&self, current: &Document) -> u128 {
        let mut total = self.bytes;
        let mut seen = HashSet::new();
        blocks(current, |ptr, len| {
            if !self.refs.contains_key(&ptr) && seen.insert(ptr) {
                total += len as u128;
            }
        });
        total
    }
}
