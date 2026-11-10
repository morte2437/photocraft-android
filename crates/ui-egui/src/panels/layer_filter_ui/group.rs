//! The Layers panel's Group filter (#10883): every group with its whole subtree; layers in no
//! group are hidden. An artboard row is not a group here, but the groups inside it are.

use std::collections::HashSet;

use photocraft_doc::{Document, Layer, LayerContent, LayerId};

/// The ids of the layers the Group filter lists. The tree is walked once, with a stack rather
/// than recursion, so a deep document can't overflow it.
pub fn listed(doc: &Document) -> HashSet<LayerId> {
    let mut out = HashSet::new();
    let mut pending: Vec<(&Layer, bool)> = doc.layers.iter().map(|l| (l, false)).collect();
    while let Some((l, inside)) = pending.pop() {
        let artboard = matches!(&l.content, LayerContent::Group(g) if g.artboard.is_some());
        let inside = inside || (l.is_group() && !artboard);
        if inside {
            out.insert(l.id);
        }
        pending.extend(l.children().unwrap_or_default().iter().map(|c| (c, inside)));
    }
    out
}
