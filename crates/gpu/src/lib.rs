//! GPU compositor (architecture §7.2, milestone M5).
//!
//! Renders a [`Document`] with wgpu, producing the same straight-alpha composite as the CPU
//! reference (`photocraft-compose`) within ~1/255:
//!
//! - **Residency.** Layer and mask surfaces live on the GPU in square *pages* (up to [`PAGE`]²,
//!   never above the device's texture limit) covering their allocated tiles. A page is uploaded
//!   only when a chunk inside it is drawn, and surfaces are copy-on-write `Arc` tiles, so a tile
//!   is re-uploaded only when its `Arc` changed (a brush stroke uploads just the touched 256²
//!   tiles; undo swaps pointers back and uploads only what differs). RGBA8 tiles upload with
//!   zero conversion. Pages over a byte budget are evicted least recently used first.
//! - **Planner** ([`plan`]). The layer tree becomes a linear list of passes over abstract
//!   chunk-sized buffers (blend with every Photoshop mode, opacity × fill, masks, clipping groups,
//!   pass-through vs isolated groups, adjustments, solid / gradient fills, layer effects).
//! - **Layer effects** (`fx.rs`). Each effect layer's maps (shadow, glow, satin, bevel, stroke
//!   bands) are built by fragment passes over its effect region and cached on the GPU per layer
//!   state; a brush stroke rebuilds only the damaged tiles grown by the effect reach. A region
//!   larger than the texture limit is built per page cell, over the cell grown by the effects'
//!   reach, so every map is exact inside its cell.
//! - **Execution.** The canvas is processed page cell by page cell, each in chunks (1024²
//!   RGBA32F accumulators, reused) that bind only that cell's pages, and each finished chunk is
//!   handed to a caller-supplied sink, e.g. to encode it straight into a display texture — no
//!   readback. Documents of any size composite on the GPU; a huge refresh submits as it goes, so
//!   staged uploads and evicted pages stay bounded.
//! - **Stack reuse.** A complete root stack unit can checkpoint its backdrop into bounded chunk
//!   textures. Exact COW snapshots invalidate the lower prefix; edits above it restore those
//!   pixels and execute only the remaining passes, capturing on the way the highest checkpoint
//!   below the edit, so repeating it (a duplicate or paste and its undo) skips that much more.
//!   Groups and clipping units stay indivisible.
//!
//! Vector masks (rasterised once per mask state into a combined mask texture), layers clipped to
//! pass-through groups, stroked shapes with clipped layers (fill and stroke split once per shape
//! state), pattern fills and artboards are planned like everything else. What remains
//! (Multichannel documents, patterns larger than the texture limit, uncached gradients with
//! coincident colour or opacity stops) returns [`Unsupported`];
//! callers fall back to the CPU compositor.
//!
//! [`CameraRawDevelop`] (`camera_raw.rs`) is a separate pass, not part of the compositor: Camera
//! Raw's per-pixel develop stages for the dialog's preview, built the first time the dialog opens.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bounds;
mod camera_raw;
mod fx;
pub mod health;
pub mod plan;
mod prefix;
pub mod timing;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_compose::effects::FieldKind;
use photocraft_doc::{DocId, Document, LayerContent, LayerId, Pattern};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

pub use camera_raw::{CAMERA_RAW_SHADER, CameraRawDevelop};
pub use health::{DeviceHealth, Fault};
pub use plan::{Kernel, Plan, Role, Unsupported, plan};
pub use timing::{GpuTimer, GpuTiming};

/// GPU compositor. Create once per device; call [`Compositor::render`] per refresh.
pub struct Compositor {
    kit: Kit,
    pool: Vec<(wgpu::Texture, wgpu::TextureView)>,
    residents: HashMap<ResKey, Resident>,
    fx: HashMap<FxKey, FxEntry>,
    prefix: prefix::Prefix,
    /// Effect temporaries (R32F, region sized) and the frame they were last used.
    temps: Vec<(Tex, u64)>,
    patterns: HashMap<String, CachedPattern>,
    /// The colour mode of the document being encoded: patterns paint in it
    /// (`photocraft_compose::pattern::pattern_rgba`).
    pattern_mode: photocraft_color::ColorMode,
    /// The document being encoded, which owns the patterns it paints with.
    pattern_doc: DocId,
    /// GPU memory pattern textures may hold ([`PATTERN_BUDGET`] by default).
    pattern_budget: u64,
    /// Texture side limit the paging is planned for (the device's, or smaller in tests).
    max_dim: u32,
    /// The device's real texture side limit.
    device_max: u32,
    /// Page side (a power of two, multiple of the tile size, at most `max_dim`).
    page: u32,
    /// Chunk side (divides `page`).
    chunk: u32,
    resident_budget: u64,
    /// GPU memory the effect-map cache may hold ([`FX_BUDGET`] or less).
    fx_budget: u64,
    /// Layer-page bytes the last full fit check asked for ([`Compositor::fit_layers`]).
    layer_need: u64,
    /// Bytes of `fx_budget` lent to `resident_budget` so those pages fit.
    lent: u64,
    /// `resident_budget` was set exactly ([`Compositor::set_resident_budget`]): nothing is lent.
    exact_resident: bool,
    /// What the view shows: a full refresh draws these cells last, so their pages are the
    /// ones still resident for the edits that follow.
    focus: Option<Rect>,
    frame: u64,
    /// Cell visits so far (LRU clock of pages).
    stamp: u64,
    /// Bytes staged by uploads since the last submit.
    staged: u64,
    /// Copies and render passes recorded since the last submit.
    recorded: u32,
    /// Why a layer page texture couldn't be created this refresh (it then ends on the CPU).
    layer_alloc: Option<String>,
    /// Bytes of pages dropped to admit new ones, not yet counted towards an intermediate submit.
    admit_freed: u64,
    /// Test hook ([`Compositor::fail_layer_allocs`]): how many more layer page textures fail as
    /// out of memory.
    fail_allocs: u32,
    acc_format: wgpu::TextureFormat,
    /// Whether the effect-map pipelines could be built (else documents with layer effects are
    /// [`Unsupported`] and use the CPU compositor).
    effect_maps: bool,
    /// `CmykSpace::id` of the document being encoded (0: built-in coated CMYK).
    cmyk: u64,
    /// The device's health (see [`Compositor::set_health`]); `None` = assumed healthy.
    health: Option<DeviceHealth>,
    /// Timestamp queries measuring how long the GPU spends on a refresh (see
    /// [`Compositor::measure_gpu_time`]); `None` = not measured.
    timer: Option<timing::GpuTimer>,
}
use W::{F, I, U};

mod batch;
mod cells;
mod consts;
mod convert;
mod encode;
mod fx_passes;
mod kit;
mod paging;
#[cfg(not(target_arch = "wasm32"))]
mod readback;
mod setup;
mod textures;

#[cfg(test)]
use batch::{BATCH, batch_kernel, batch_op_words, batch_runs, op_words, run_starts, tile_grid};
#[cfg(test)]
use consts::OP_UNIFORM;
pub use consts::{ACC_FORMAT, CHUNK, FX_BUDGET, PAGE, PATTERN_BUDGET, PATTERN_IDLE_FRAMES, RESIDENT_BUDGET};
use convert::W;
#[cfg(test)]
use convert::{CONVERT_BATCH, colour_managed, convert_tile, convert_tiles, rgb_gray16_to_f16, rgb_gray32_to_f16};
pub use convert::{f16_to_f32, f32_to_f16};
pub use kit::SHADER;
use kit::{Kit, tex_entry};
pub use paging::OVER_BUDGET;
use paging::first_error;
#[cfg(test)]
use paging::{lru_victims, uniform_buffer_len};
#[cfg(not(target_arch = "wasm32"))]
pub use readback::{render_to_vec, render_to_vec_stats};
#[cfg(test)]
use textures::TexKind;
use textures::{CachedPattern, FxEntry, FxKey, ResKey, Resident, Tex};
pub use textures::{ChunkOut, Stats};

#[cfg(test)]
mod tests;
