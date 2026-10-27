/// Accumulator format for intermediate buffers. Full float: discontinuous operations
/// (Posterize, Threshold, Hard Mix, Dissolve) must land on the same side of their thresholds as
/// the CPU reference, and Color Burn/Dodge amplify input error.
pub const ACC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

// default accumulation format; `preferred_acc_format` downgrades to Rgba16Float where 32-bit float isn't renderable
/// Effect intermediates (distances, blurs) and layer shapes.
pub(crate) const MAP32: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Final effect coverage maps (0..1; half precision is ~1/4000).
pub(crate) const MAP16: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Side of the square chunks the canvas is processed in.
pub const CHUNK: u32 = 1024;

pub(crate) const STRIDE: u64 = 256;

pub(crate) const CHUNK_UNIFORM: u64 = 16;

pub(crate) const OP_UNIFORM: u64 = 256;

/// GPU memory the effect-map cache may hold before evicting layers not drawn this frame.
pub const FX_BUDGET: usize = 1536 << 20;

/// Largest side of the square pages layer surfaces are stored in on the GPU (smaller when the
/// device's texture limit is). A chunk only binds the pages of the cell it lies in, so layers of
/// any size composite on the GPU.
pub const PAGE: u32 = 2048;

/// GPU memory resident layer pages may hold before the least recently used are evicted.
pub const RESIDENT_BUDGET: u64 = 4 << 30;

/// GPU memory pattern textures may hold before the least recently painted are dropped (those the
/// current frame paints with always stay).
pub const PATTERN_BUDGET: u64 = 256 << 20;

/// A pattern texture no frame has painted with for this many frames is dropped.
pub const PATTERN_IDLE_FRAMES: u64 = 240;

/// Uploads (bytes) [`Compositor::render`] stages before submitting the work recorded so far, so
/// a refresh of a huge document never holds all its uploads in staging memory at once.
pub(crate) const FLUSH_BYTES: u64 = 512 << 20;

/// Pass pixels (chunk pixels × passes) [`Compositor::render`] records before submitting: one
/// command buffer holding a whole huge refresh can run for seconds, long enough for the OS to
/// reset the GPU.
pub(crate) const FLUSH_WORK: u64 = 1 << 30;

/// Copies and render passes [`Compositor::render`] records before submitting. wgpu opens up to
/// two backend command buffers for each (a render pass and its barrier pre-pass), and Metal
/// treats more than 4096 outstanding ones as a lost device: a refresh of a document with a few
/// hundred layers reached that inside a single page cell (P12, "pc_compose_uniforms is
/// invalid"), where the work and byte limits above never trigger.
pub(crate) const FLUSH_OPS: u32 = 1000;
