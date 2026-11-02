//! Conservative preflight for the main PSD pixel paths. This bounds decoded layer/mask/
//! merged and pattern pixels, not parser metadata or smart-filter caches (separate limits).
//!
//! Reserve the sum of all possible retained tiles and the largest workspace, without credit
//! for pruning or tile sharing. Reserving before any worker starts makes scheduling irrelevant.

use photocraft_geom::TILE_SIZE;
use photocraft_psd::{
    PsdFile,
    layer::{CHANNEL_REAL_USER_MASK, CHANNEL_USER_MASK, Rect},
};

use crate::IoError;

/// Default allowance for PSD layer, mask and merged-image pixels, including conversion
/// workspace and tile padding. Other format/parser limits still apply. Callers may lower it.
pub const DEFAULT_PSD_IMPORT_PIXEL_BYTES: u64 = if cfg!(target_arch = "wasm32") { 1 << 30 } else { 2 << 30 };

#[derive(Default)]
struct ImportBudget {
    retained: u64,
    workspace: u64,
}

fn overflow() -> IoError {
    IoError::Unsupported("PSD import pixel memory estimate overflow".into())
}

// inflate_checked uses at least 8 KiB per stream, even for a one-pixel plane.
fn inflate_scratch(plane_bytes: u64, channels: u64) -> Result<u64, IoError> {
    mul(plane_bytes.saturating_add(1).clamp(8192, 1 << 20), channels)
}

fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(overflow)
}

impl ImportBudget {
    fn retain(&mut self, bytes: u64) -> Result<(), IoError> {
        self.retained = self.retained.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    }

    fn scratch(&mut self, bytes: u64) {
        self.workspace = self.workspace.max(bytes);
    }

    fn required(&self) -> Result<u64, IoError> {
        self.retained.checked_add(self.workspace).ok_or_else(overflow)
    }

    fn check(&self, limit: u64) -> Result<(), IoError> {
        let required = self.required()?;
        if required > limit {
            return Err(IoError::PsdImportBudget { required, limit });
        }
        Ok(())
    }

    fn surface(&mut self, rect: Rect, channels: u64, sample: u64) -> Result<(), IoError> {
        if rect.is_empty() {
            return Ok(());
        }
        let tile = i64::from(TILE_SIZE);
        // Division must round down for negative offsets: even a 2x2 rect straddling the
        // origin occupies four tiles. Widen first, including the exclusive right/bottom edge.
        let nx = (i64::from(rect.right) - 1).div_euclid(tile) - i64::from(rect.left).div_euclid(tile) + 1;
        let ny = (i64::from(rect.bottom) - 1).div_euclid(tile) - i64::from(rect.top).div_euclid(tile) + 1;
        self.retain(mul(mul(mul(nx as u64, ny as u64)?, (TILE_SIZE as u64).pow(2))?, mul(channels, sample)?)?)
    }
}

/// No decoding here: even a compact ZIP stream must be refused before inflation. The sum
/// deliberately counts full decoded planes for every compression method, plus their row
/// buffers, interleave output and float conversion workspace. This is an upper estimate,
/// rather than a promise that an allocator or OS can satisfy an accepted import.
pub(crate) fn check(file: &PsdFile, limit: u64) -> Result<(), IoError> {
    let mut budget = ImportBudget::default();
    let h = &file.header;
    h.validate()?;
    let sample = u64::from(h.depth.div_ceil(8));
    let cc = match h.color_mode {
        // Unknown modes use the RGB fallback, including its alpha channel.
        photocraft_psd::ColorMode::Unknown(_) => 3,
        mode => u64::from(mode.color_channels().unwrap_or(3)),
    };
    if super::psd_import::can_recover_layers(file) {
        for rec in file.layers() {
            if !rec.rect.is_empty() && rec.channels.iter().any(|c| c.id >= 0 || c.id == -1) {
                let (w, hh) = rec.rect.size()?;
                let pixels = mul(w as u64, hh as u64)?;
                budget.surface(rec.rect, cc + 1, sample)?;
                // Whole ZIP planes + band planes + interleave + temporary float pixels.
                // Counting a full layer for each band buffer also covers a single very wide row.
                let workspace = mul(pixels, mul(cc + 1, sample * 3 + 4)?)?.checked_add(inflate_scratch(mul(pixels, sample)?, cc + 1)?).ok_or_else(overflow)?;
                // RLE row indexes live beside decoded planes.
                budget.scratch(workspace.checked_add(mul(hh as u64 + 1, (cc + 1) * 8)?).ok_or_else(overflow)?);
            }
            if let Some(mask) = rec.layer_mask() {
                for (id, rect) in [(CHANNEL_USER_MASK, mask.rect), (CHANNEL_REAL_USER_MASK, mask.real.map_or(mask.rect, |m| m.rect))] {
                    if rec.channel(id).is_none() || rect.is_empty() {
                        continue;
                    }
                    let (w, hh) = rect.size()?;
                    budget.surface(rect, 1, sample)?;
                    // The layer's ZIP decoders have been dropped before record_mask runs.
                    let pixels = mul(w as u64, hh as u64)?;
                    budget.scratch(mul(pixels, sample * 3 + 4)?.checked_add(inflate_scratch(mul(pixels, sample)?, 1)?).ok_or_else(overflow)?);
                }
            }
        }
    }
    if super::psd_import::merged_needed(file) {
        let pixels = mul(u64::from(h.width), u64::from(h.height))?;
        let channels = u64::from(h.channels);
        let canvas = Rect::from_xywh(0, 0, h.width, h.height);
        // Multichannel and extra channels become individual grayscale surfaces. Reserving
        // their tiles separately matters for images smaller than a tile.
        budget.surface(canvas, channels.max(cc + 1), sample.max(1))?;
        // A fallback surface can coexist with the layers when none produced usable pixels.
        budget.surface(canvas, cc + 1, sample.max(1))?;
        // The merged planes stay alive while layers are built.
        let merged = mul(mul(h.row_bytes() as u64, u64::from(h.height))?, channels)?;
        budget.retain(merged)?;
        // Include float expansion/readback and RGBA fallback, even for packed bitmap input.
        let converted = mul(pixels, mul(channels.max(cc + 1), sample * 3 + 12)?)?;
        budget.scratch(converted.checked_add(inflate_scratch(merged, 1)?).ok_or_else(overflow)?);
    }
    let mut patterns = photocraft_psd::patterns::PatternBudget::default();
    for block in file.global_blocks.iter().filter(|b| crate::pattern_map::PATTERN_KEYS.contains(&b.key)) {
        // Earlier records can decode before a malformed later record discards the block.
        let items = photocraft_psd::patterns::pattern_block_memory(&block.data, &mut patterns);
        for p in items {
            let pixels = mul(u64::from(p.width), u64::from(p.height))?;
            let sample = u64::from(p.depth.div_ceil(8));
            let color = if p.mode == 2 { 3 } else { photocraft_psd::patterns::mode_channels(p.mode) as u64 };
            let alpha = u64::from(p.planes > photocraft_psd::patterns::mode_channels(p.mode));
            budget.surface(Rect::from_xywh(0, 0, p.width, p.height), color + alpha, sample)?;
            // The block parser returns all planes together. Count them as retained so
            // previous converted patterns and later blocks cannot reuse this allowance.
            budget.retain(p.decoded_bytes)?;
            // Channel clones/bitmap expansion, indexed RGB expansion and interleave.
            // Include a source plane during padded channel placement as well.
            let converted = mul(pixels, mul((p.planes as u64).max(color + alpha) + color + alpha + 3, sample)?)?;
            budget.scratch(converted.checked_add(p.decoded_bytes).ok_or_else(overflow)?);
        }
    }
    budget.check(limit)
}

#[cfg(test)]
mod tests;
