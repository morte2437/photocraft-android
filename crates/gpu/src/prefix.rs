//! One lower-stack composite, held in bounded chunk textures. Exact COW snapshots deliberately
//! invalidate on more than rendering needs: completeness matters more than a clever render key.
//!
//! Entries are **whole grid chunks** (chunk-aligned, chunk-sized document rectangles), not the
//! damage rectangles the render happens to need: a brush stroke paints a new partial rectangle
//! every frame, and caching those would fill the budget with entries that never hit again.
//! A partial rectangle is served as a sub-rect copy of the whole chunk it falls into, and a
//! capture merges into the chunk it overlaps, so every hit lands on the same stable entry.
//!
//! Each chunk remembers the checkpoint it holds. A render captures at the highest checkpoint
//! below its edit, also where it resumed from a lower cached one, so the cache follows the edit
//! up the stack chunk by chunk: repeating an edit high in the stack (a duplicate or paste and its
//! undo) then skips everything below it, and chunks a refresh doesn't reach keep serving their
//! own, lower prefix meanwhile (#5286). A chunk moved by a partial capture holds only that
//! rectangle afterwards: the rest of it misses once, like any fresh entry.
use std::collections::HashMap;

use photocraft_doc::{Document, Layer};
use photocraft_geom::Rect;

use crate::plan::Checkpoint;

pub(crate) const MAX_BYTES: u64 = 512 << 20;

/// One whole grid chunk, with the document rectangles that hold valid pixels: a whole-chunk
/// capture covers the cell in one entry, partial captures accumulate alongside each other.
/// The texture is only copied to and from, so it has no view: Vulkan rejects a view on an image
/// without a sampled/storage/attachment usage (VUID-VkImageViewCreateInfo-image-04441).
pub(crate) struct Chunk {
    tex: wgpu::Texture,
    covered: Vec<Rect>,
    /// The checkpoint the covered pixels hold.
    checkpoint: Checkpoint,
    bytes: u64,
}

pub(crate) struct Prefix {
    context: Option<Document>,
    /// The root layers of the last prepared render, all of them: a chunk stays valid while every
    /// render since its capture left the layers below its checkpoint as they were.
    layers: Vec<Layer>,
    /// Where renders capture: the highest useful checkpoint below the latest edit.
    checkpoint: Option<Checkpoint>,
    chunks: HashMap<Rect, Chunk>,
    bytes: u64,
    budget: u64,
    step: i32,
}

impl Default for Prefix {
    fn default() -> Self {
        Self { context: None, layers: Vec::new(), checkpoint: None, chunks: HashMap::new(), bytes: 0, budget: MAX_BYTES, step: 512 }
    }
}

/// The whole grid chunk a rectangle falls into (chunk-aligned origin, full chunk size).
fn cell_of(rect: Rect, step: i32) -> Rect {
    let x0 = rect.x0.div_euclid(step) * step;
    let y0 = rect.y0.div_euclid(step) * step;
    Rect::new(x0, y0, x0.saturating_add(step), y0.saturating_add(step))
}

impl Prefix {
    pub(crate) fn clear(&mut self) {
        self.context = None;
        self.layers.clear();
        self.checkpoint = None;
        self.chunks.clear();
        self.bytes = 0;
    }

    pub(crate) fn forget(&mut self, doc: photocraft_doc::DocId) {
        if self.context.as_ref().is_some_and(|d| d.id == doc) {
            self.clear();
        }
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn budget(&self) -> u64 {
        self.budget
    }

    pub(crate) fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes.min(MAX_BYTES);
        if self.bytes > self.budget || self.budget == 0 {
            self.clear();
        }
    }

    /// The grid step the entries are aligned to; the compositor reports its chunk size.
    pub(crate) fn set_step(&mut self, step: u32) {
        let step = step.max(1) as i32;
        if step != self.step {
            self.step = step;
            self.clear();
        }
    }

    /// The document the cached composite belongs to, if any.
    pub(crate) fn doc(&self) -> Option<photocraft_doc::DocId> {
        self.context.as_ref().map(|d| d.id)
    }

    /// Drops the chunks `doc` invalidated and returns the checkpoint this render captures at, one
    /// of `checkpoints` (the plan's that a batch run starts at). A chunk held at any other point
    /// is dropped too: resuming there would group the layers above it differently.
    pub(crate) fn prepare(&mut self, doc: &Document, checkpoints: &[Checkpoint]) -> Option<Checkpoint> {
        if self.budget == 0 {
            return None;
        }
        // Keeping every non-layer field makes new document inputs invalidate automatically.
        // Tile data is Arc-shared; shared Tile equality does not scan its pixel bytes.
        let mut context = doc.clone();
        let layers = std::mem::take(&mut context.layers);
        let same_context = self.context.as_ref() == Some(&context);
        // The lowest root layer this render changes since the last one (a layer added or removed
        // at the top changes from there on).
        let changed = self
            .layers
            .iter()
            .zip(&layers)
            .position(|(a, b)| a != b)
            .or_else(|| (layers.len() != self.layers.len()).then_some(layers.len().min(self.layers.len())));
        if same_context {
            let mut freed = 0u64;
            self.chunks.retain(|_, c| {
                let keep = changed.is_none_or(|ch| c.checkpoint.layer_end <= ch) && checkpoints.contains(&c.checkpoint);
                if !keep {
                    freed = freed.saturating_add(c.bytes);
                }
                keep
            });
            self.bytes = self.bytes.saturating_sub(freed);
        } else {
            self.clear();
        }
        let kept = self.checkpoint.filter(|c| same_context && changed.is_none() && checkpoints.contains(c));
        let checkpoint = kept.or_else(|| match changed {
            // Just below the edit; nothing when it is at the bottom.
            Some(changed) if same_context => checkpoints.iter().rev().find(|c| c.layer_end <= changed && c.pass_end > 1).copied(),
            _ => {
                let mut useful = checkpoints.iter().rev().filter(|c| c.pass_end > 1);
                let last = useful.next().copied();
                useful.next().copied().or(last)
            }
        });
        // Remember bottom-layer edits even when no useful prefix survives: otherwise every other
        // edit would populate and immediately discard the default checkpoint.
        self.context = Some(context);
        self.layers = layers;
        self.checkpoint = checkpoint;
        checkpoint
    }

    /// The checkpoint the cached pixels of `rect` hold, when every one of them is cached in its
    /// whole grid chunk.
    pub(crate) fn restore_point(&self, rect: Rect) -> Option<Checkpoint> {
        self.covered_by(rect).map(|(chunk, _)| chunk.checkpoint)
    }

    /// The whole chunk and the source offset that serve `rect`, when it is fully covered.
    fn covered_by(&self, rect: Rect) -> Option<(&Chunk, (i32, i32))> {
        let cell = cell_of(rect, self.step);
        let chunk = self.chunks.get(&cell)?;
        let offset = (rect.x0 - cell.x0, rect.y0 - cell.y0);
        chunk.covered.iter().any(|c| c.contains_rect(&rect)).then_some((chunk, offset))
    }

    /// Copies the cached pixels of `rect` into `dst` (its accumulator slot). Only whole-chunk
    /// and sub-rect copies out of the whole chunk; a partial hit is not pretended to be full.
    pub(crate) fn copy_out(&self, encoder: &mut wgpu::CommandEncoder, rect: Rect, dst: &wgpu::Texture) -> bool {
        let Some((chunk, (ox, oy))) = self.covered_by(rect) else { return false };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &chunk.tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x: ox.unsigned_abs(), y: oy.unsigned_abs(), z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: rect.width(), height: rect.height(), depth_or_array_layers: 1 },
        );
        true
    }

    /// Merges the rendered `rect` (document coordinates) into its whole grid chunk, admitting
    /// the chunk while space remains. Replacing entries on every miss would make a full
    /// refresh larger than the cache evict its own next hits.
    /// A chunk holding another (lower) checkpoint starts over with this capture.
    pub(crate) fn capture(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, rect: Rect, src: &wgpu::Texture, checkpoint: Checkpoint) {
        if rect.is_empty() {
            return;
        }
        let cell = cell_of(rect, self.step);
        let bpp = if src.format() == wgpu::TextureFormat::Rgba32Float { 16 } else { 8 };
        let cell_bytes = u64::from(cell.width()).saturating_mul(u64::from(cell.height())).saturating_mul(bpp);
        if !self.chunks.contains_key(&cell) {
            if cell_bytes == 0 || cell_bytes > self.budget.saturating_sub(self.bytes) {
                return;
            }
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pc_compose_prefix"),
                size: wgpu::Extent3d { width: cell.width().max(1), height: cell.height().max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: src.format(),
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.bytes = self.bytes.saturating_add(cell_bytes);
            self.chunks.insert(cell, Chunk { tex, covered: Vec::new(), checkpoint, bytes: cell_bytes });
        }
        let Some(chunk) = self.chunks.get_mut(&cell) else { return };
        if chunk.checkpoint != checkpoint {
            // The render restored this chunk's old pixels earlier in the same encoder, so
            // overwriting them in place is ordered after that copy.
            chunk.covered.clear();
            chunk.checkpoint = checkpoint;
        }
        // A whole-chunk render replaces the coverage; a partial rectangle lands at its offset
        // inside the cell, beside whatever earlier strokes put there.
        if rect == cell {
            chunk.covered.clear();
        } else if chunk.covered.iter().any(|c| c.contains_rect(&rect)) {
            return; // already cached from an earlier frame
        }
        let (ox, oy) = (rect.x0 - cell.x0, rect.y0 - cell.y0);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo {
                texture: &chunk.tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x: ox.unsigned_abs(), y: oy.unsigned_abs(), z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d { width: rect.width(), height: rect.height(), depth_or_array_layers: 1 },
        );
        chunk.covered.push(rect);
        if chunk.covered.iter().any(|c| c == &cell) {
            chunk.covered = vec![cell];
        }
    }
}
