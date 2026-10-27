use super::*;

/// An embedded smart-object source as written to `lnk2`.
#[derive(Clone, Debug)]
pub(super) struct Source {
    pub(super) uuid: String,
    /// Pixel size of the source document.
    pub(super) size: (f64, f64),
    pub(super) dpi: f64,
}

pub(super) fn contents_uuid(contents_id: SmartContentsId, bytes: &[u8]) -> String {
    let mut identity = blake3::Hasher::new();
    identity.update(&contents_id.0.to_le_bytes());
    identity.update(bytes);
    crate::smart_map::uuid_from(identity.finalize().as_bytes())
}

/// Smart-object state collected while emitting layers, written into the global blocks at the end.
#[derive(Default)]
pub(super) struct SmartOut {
    /// Nesting depth of this export (embedded documents are exported recursively).
    pub(super) depth: u32,
    /// The document's preserved global blocks (embedded files and filter caches from PSD import).
    pub(super) globals: Vec<photocraft_doc::PsdGlobalBlock>,
    /// uuids of the files in the preserved linked-layer blocks.
    pub(super) known: std::collections::HashSet<String>,
    /// uuids the layers reference.
    pub(super) used: std::collections::HashSet<String>,
    /// Unreferenced preserved files may be dropped (every smart layer's file id is known).
    pub(super) prune_ok: bool,
    /// Files to embed.
    pub(super) add: Vec<crate::linked::LinkedFile>,
    /// Sources converted so far, by contents identity, bytes and whether they were read from a
    /// link (`Err` = can't be embedded).
    pub(super) converted: std::collections::HashMap<(SmartContentsId, [u8; 32], bool), Result<Source, String>>,
    /// Each placed layer has its own id, even when it duplicates an imported layer.
    pub(super) placed_used: std::collections::HashSet<String>,
    /// Original instances reserve their imported placement regardless of layer order.
    pub(super) placed_owners: std::collections::HashMap<String, photocraft_doc::LayerId>,
    /// Legacy records without a placed id stay unchanged for the original instance.
    pub(super) unplaced_owners: std::collections::HashMap<SmartContentsId, photocraft_doc::LayerId>,
    /// The imported identity that keeps each original linked UUID.
    pub(super) linked_contents: std::collections::HashMap<String, SmartContentsId>,
    /// Independent copies of preserved linked items, including unavailable external sources.
    pub(super) linked_copies: std::collections::BTreeMap<(String, SmartContentsId), photocraft_doc::PsdGlobalBlock>,
    /// Embedded documents decoded while converting them, by uuid (for the filter caches).
    pub(super) docs: std::collections::HashMap<String, Document>,
    /// Source composites (document pixel format, source space) by uuid; `None` = unreadable.
    pub(super) composites: std::collections::HashMap<String, Option<(std::sync::Arc<Surface>, photocraft_geom::Rect)>>,
    /// Preserved `FEid` items (by placed id).
    pub(super) old_fx: Vec<photocraft_psd::filter_effects::FilterEffectsItem>,
    /// Placed ids whose preserved `FEid` item stays.
    pub(super) keep_fx: std::collections::HashSet<String>,
    /// Regenerated `FEid` items.
    pub(super) new_fx: Vec<photocraft_psd::filter_effects::FilterEffectsItem>,
    /// The unfiltered pixels of each smart object whose filter mask shows: its layer record holds
    /// the filtered pixels without the mask, as Photoshop's do (`compose::filter_mask`).
    pub(super) unfiltered: std::collections::HashMap<photocraft_doc::LayerId, Surface>,
}

/// Whether a smart object's filter mask shows: enabled over at least one visible, enabled filter.
pub(super) fn masked(sm: &photocraft_doc::SmartObject) -> bool {
    sm.filter_mask.as_ref().is_some_and(|m| m.enabled) && sm.filters_enabled && sm.smart_filters.iter().any(|f| f.visible)
}

/// `FEid` data padded to 4 bytes inside the block length: Photoshop counts the padding in the
/// length and can't open a file whose `FEid` is padded outside it.
fn padded_fx(fx: &photocraft_psd::filter_effects::FilterEffects) -> Vec<u8> {
    let mut d = fx.to_bytes();
    d.resize(d.len().next_multiple_of(4), 0);
    d
}

pub(super) const LINK_KEYS: [&[u8; 4]; 4] = [b"lnk2", b"lnk3", b"lnkD", b"lnkE"];

const FX_KEYS: [&[u8; 4]; 2] = [b"FEid", b"FXid"];

impl SmartOut {
    pub(super) fn new(doc: &Document, depth: u32) -> Self {
        let globals = doc.metadata.psd_global_blocks.clone();
        let known = globals.iter().filter(|(_, k, _)| LINK_KEYS.contains(&k)).flat_map(|(_, _, d)| crate::linked::block_uuids(d)).collect();
        let old_fx = globals
            .iter()
            .filter(|(_, k, _)| FX_KEYS.contains(&k))
            .filter_map(|(_, _, d)| photocraft_psd::filter_effects::FilterEffects::parse(d).ok())
            .flat_map(|fx| fx.items)
            .collect();
        let mut linked_contents = std::collections::HashMap::new();
        let mut placed_owners = std::collections::HashMap::new();
        let mut unplaced_owners = std::collections::HashMap::new();
        let mut layers = doc.walk();
        // Duplicating a layer clears psd_id and gives it a newer runtime id.
        layers.sort_by_key(|(_, _, layer)| (layer.psd_id.is_none(), layer.id));
        for (_, _, layer) in layers {
            if let LayerContent::Smart(sm) = &layer.content
                && let photocraft_doc::SmartSource::Linked { path } = &sm.source
                && let Some(placed) = sm.psd_raw.as_ref().and_then(|d| crate::smart_map::parse_sold(d))
                && placed.idnt == *path
            {
                linked_contents.entry(path.clone()).or_insert(sm.contents_id);
                if placed.placed.is_empty() {
                    unplaced_owners.entry(sm.contents_id).or_insert(layer.id);
                } else {
                    placed_owners.entry(placed.placed).or_insert(layer.id);
                }
            }
        }
        SmartOut { depth, globals, known, prune_ok: true, old_fx, linked_contents, placed_owners, unplaced_owners, ..Default::default() }
    }

    /// The document's global blocks with the linked-layer files and filter caches brought up to
    /// date: unreferenced files dropped (when every reference is known), new files appended,
    /// filter caches kept or regenerated per smart object. Unchanged blocks stay byte-identical.
    pub(super) fn finish(&self, blocks: Vec<photocraft_doc::PsdGlobalBlock>) -> Vec<photocraft_doc::PsdGlobalBlock> {
        use std::sync::Arc;
        let keep = |u: &str| !self.prune_ok || self.used.contains(u);
        let mut out = Vec::with_capacity(blocks.len() + 2);
        let mut added = false;
        let mut fx_placed = false;
        for (sig, key, data) in blocks {
            if LINK_KEYS.contains(&&key) {
                let add: &[crate::linked::LinkedFile] = if !added && &key == b"lnk2" { &self.add } else { &[] };
                added |= &key == b"lnk2";
                match crate::linked::rebuild_block(&data, &keep, add) {
                    None => out.push((sig, key, data)),
                    Some(d) if d.is_empty() => {}
                    Some(d) => out.push((sig, key, Arc::new(d))),
                }
            } else if FX_KEYS.contains(&&key) {
                let Ok(mut fx) = photocraft_psd::filter_effects::FilterEffects::parse(&data) else {
                    out.push((sig, key, data));
                    continue;
                };
                let before = fx.items.len();
                fx.items.retain(|i| self.keep_fx.contains(&i.id));
                let fresh = !fx_placed && !self.new_fx.is_empty();
                if fresh {
                    fx.items.extend(self.new_fx.iter().cloned());
                    fx_placed = true;
                }
                if fx.items.len() == before && !fresh {
                    out.push((sig, key, data));
                } else if !fx.items.is_empty() {
                    out.push((sig, key, Arc::new(padded_fx(&fx))));
                }
            } else {
                out.push((sig, key, data));
            }
        }
        if !added && !self.add.is_empty() {
            let data: Vec<u8> = self.add.iter().flat_map(crate::linked::encode_linked_file).collect();
            out.push((*b"8BIM", *b"lnk2", Arc::new(data)));
        }
        for (sig, key, data) in self.linked_copies.values() {
            if let Some((_, _, block)) = out.iter_mut().find(|(s, k, _)| s == sig && k == key) {
                Arc::make_mut(block).extend_from_slice(data);
            } else {
                out.push((*sig, *key, data.clone()));
            }
        }
        if !fx_placed && !self.new_fx.is_empty() {
            let fx = photocraft_psd::filter_effects::FilterEffects { version: 3, items: self.new_fx.clone() };
            out.push((*b"8BIM", *b"FEid", Arc::new(padded_fx(&fx))));
            if !out.iter().any(|(_, k, _)| k == b"FMsk") {
                // Filter mask overlay: RGB red at 50 %, Photoshop's default.
                out.push((*b"8BIM", *b"FMsk", Arc::new(vec![0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 50])));
            }
        }
        out
    }
}

/// The source size of a smart object whose file we can't decode: its rendered bounds taken back
/// through the inverse transform (the source's top-left is the origin).
pub(super) fn size_from_layer(sm: &photocraft_doc::SmartObject) -> (f64, f64) {
    let r = sm.cache.as_ref().map(Surface::content_bounds).filter(|r| !r.is_empty());
    let (Some(r), Some(inv)) = (r, sm.transform.inverse()) else { return (1.0, 1.0) };
    let [a, b, c, d, e, f] = inv.m;
    let (mut w, mut h) = (1.0f64, 1.0f64);
    for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
        let (x, y) = (f64::from(x), f64::from(y));
        w = w.max((a * x + c * y + e).round());
        h = h.max((b * x + d * y + f).round());
    }
    (w, h)
}

pub(super) fn upsert(raw: &mut Vec<([u8; 4], Vec<u8>)>, key: &[u8; 4], data: Vec<u8>, before: Option<&[u8; 4]>) {
    if let Some(e) = raw.iter_mut().find(|(k, _)| k == key) {
        e.1 = data;
        return;
    }
    match before.and_then(|b| raw.iter().position(|(k, _)| k == b)) {
        Some(i) => raw.insert(i, (*key, data)),
        None => raw.push((*key, data)),
    }
}

/// Resource 1026 (Layer › Link Layers): one u16 group id per layer record, in the order
/// [`Ex::emit`] writes records (a group's bounding divider, its children, then the group).
/// Imported ids that fit in a u16 are kept, so unchanged files write the same bytes; otherwise
/// groups are renumbered by first appearance. `None` when no layer is linked.
pub(super) fn link_group_resource(layers: &[Layer]) -> Option<Vec<u8>> {
    fn walk(layers: &[Layer], out: &mut Vec<Option<u64>>) {
        for l in layers {
            if let LayerContent::Group(g) = &l.content {
                out.push(None);
                walk(&g.children, out);
            }
            out.push(l.link_group);
        }
    }
    let mut per = Vec::new();
    walk(layers, &mut per);
    if per.iter().all(Option::is_none) {
        return None;
    }
    let fits = per.iter().flatten().all(|&g| (1..=u64::from(u16::MAX)).contains(&g));
    let mut seen: Vec<u64> = Vec::new();
    let ids = per.iter().map(|g| match *g {
        None => 0u16,
        Some(g) if fits => g as u16,
        Some(g) => {
            let i = seen.iter().position(|&s| s == g).unwrap_or_else(|| {
                seen.push(g);
                seen.len() - 1
            });
            u16::try_from(i + 1).unwrap_or(u16::MAX)
        }
    });
    Some(ids.flat_map(u16::to_be_bytes).collect())
}

/// Whether the layer's effects still equal what its preserved `lmfx`/`lfx2`
/// decodes to. `None` when there is nothing preserved or it can't be decoded.
/// Block keys a smart object's `psd_raw` may be written under, in priority order. The data says
/// which it is: `PlLd` holds a `plcL` structure, `SoLd`/`SoLE` a `soLD` one (Adobe PSD spec,
/// "Placed Layer" and "Placed Layer Data"). Writing one under the other's key makes the block
/// unreadable.
pub(super) fn smart_keys(data: Option<&[u8]>) -> &'static [&'static [u8; 4]] {
    match data.and_then(|d| d.get(..4)) {
        Some(b"plcL") => &[b"PlLd"],
        Some(b"soLD") => &[b"SoLd", b"SoLE"],
        _ => &[b"SoLd", b"PlLd", b"SoLE"],
    }
}

pub(super) fn effects_unchanged(l: &Layer) -> Option<bool> {
    let src = l.psd_blocks.iter().find(|(k, _)| k == b"lmfx").map(|(_, d)| d.clone()).or_else(|| l.effects.psd_raw.clone())?;
    let (m, items) = crate::effects_map::parse_lfx2(&src)?;
    Some(m == l.effects.enabled && items == l.effects.items)
}
