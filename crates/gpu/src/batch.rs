use super::convert::{W, words};
use super::textures::Resident;
use super::*;

/// The uniform record of a pass.
pub(crate) fn op_words(p: &plan::Pass<'_>, tex: Option<[i32; 4]>, mask: Option<[i32; 4]>, map: Option<Rect>) -> Vec<u8> {
    words(&op_values(p, tex, mask, map))
}

/// The uniform record of the `fs_layerbatch` pass that draws `p` (whose texture and mask cover
/// `tex` and `mask`) and then `extras`, each an unmasked plain layer's `LayerBlend` pass with the
/// area its texture covers. Layout: see `batch_layer` in compose.wgsl.
pub(crate) fn batch_op_words(p: &plan::Pass<'_>, tex: Option<[i32; 4]>, mask: Option<[i32; 4]>, extras: &[(&plan::Pass<'_>, Option<[i32; 4]>)]) -> Vec<u8> {
    let mut v = op_values(p, tex, mask, None);
    // Word offsets of the record's vec4s holding the extra layers, three each: p0, p1, p2 | p3,
    // q0, q1 | q2, q3, q4.
    const AT: [usize; 9] = [20, 24, 28, 32, 44, 48, 52, 56, 60];
    if let Some(kind) = v.get_mut(1) {
        *kind = I(extras.len().min(AT.len() / 3) as i32);
    }
    for (e, (x, area)) in extras.iter().take(AT.len() / 3).enumerate() {
        let r = area.unwrap_or([0; 4]);
        let geo = [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32];
        let prm = [x.opacity, x.fill, plan::mode_index(x.mode) as f32, if area.is_some() { 1.0 } else { 0.0 }];
        for (k, vals) in [geo, x.color, prm].into_iter().enumerate() {
            let at = AT[e * 3 + k];
            for (c, f) in vals.into_iter().enumerate() {
                if let Some(w) = v.get_mut(at + c) {
                    *w = F(f);
                }
            }
        }
    }
    words(&v)
}

/// The words of a pass's uniform record (`Op` in compose.wgsl).
fn op_values(p: &plan::Pass<'_>, tex: Option<[i32; 4]>, mask: Option<[i32; 4]>, map: Option<Rect>) -> Vec<W> {
    let mut flags = p.flags;
    let (mut to, mut ts, mut mo, mut ms) = ([0; 2], [0; 2], [0; 2], [0; 2]);
    if let Some(r) = tex {
        flags |= 4;
        to = [r[0], r[1]];
        ts = [r[2], r[3]];
    }
    let (mut density, mut mdefault) = (0.0, 1.0);
    if let Some(m) = &p.mask {
        flags |= 1;
        density = m.density;
        mdefault = m.default;
        if let Some(r) = mask {
            flags |= 2;
            mo = [r[0], r[1]];
            ms = [r[2], r[3]];
        }
    }
    if p.gradient {
        flags |= 8;
    }
    let (mpo, mps) = map.map_or(([0, 0], [0, 0]), |r| ([r.x0, r.y0], [r.width() as i32, r.height() as i32]));
    let mut v = vec![
        I(plan::mode_index(p.mode)),
        I(p.adjust_kind),
        U(flags),
        U(0),
        F(p.opacity),
        F(density),
        F(mdefault),
        F(p.fill),
        I(to[0]),
        I(to[1]),
        I(ts[0]),
        I(ts[1]),
        I(mo[0]),
        I(mo[1]),
        I(ms[0]),
        I(ms[1]),
    ];
    v.extend(p.color.iter().map(|f| F(*f)));
    for row in &p.params {
        v.extend(row.iter().map(|f| F(*f)));
    }
    v.extend([I(mpo[0]), I(mpo[1]), I(mps[0]), I(mps[1])]);
    v.extend(p.extra.iter().map(|f| F(*f)));
    // q0..q4: only `fs_layerbatch` reads them (`batch_op_words`).
    v.extend((0..20).map(|_| F(0.0)));
    v
}

/// Most layers one `fs_layerbatch` pass blends: its own and the three more its record holds.
pub(crate) const BATCH: usize = 4;

/// For each pass, how many passes from it on the executor draws as one `fs_layerbatch` pass
/// (1: the pass alone). A run starts at a `LayerBlend` pass over the whole chunk (no clip) that
/// isn't a gradient (whose parameters use the words the extra layers go in), and continues while
/// the next pass is a `LayerBlend` of an unmasked plain layer (no text gamma, no Dissolve) onto
/// exactly what the previous pass wrote, a slot nothing else reads before it is written again.
/// A run never ends in the slot it reads from.
///
/// The runs depend on the plan alone, never on the prefix cache: a compiler may give a layer
/// other bits batched than drawn alone (FXC on D3D12 does), and reusing a prefix must stay exact
/// (#5286). The cache captures and resumes only at `run_starts` instead.
pub(crate) fn batch_runs(plan: &Plan<'_>) -> Vec<usize> {
    let passes = &plan.passes;
    let n = passes.len();
    let reads = |p: &plan::Pass<'_>, s: plan::Slot| [p.a, p.b, p.c, p.d].contains(&Some(s));
    // feeds_next[i]: what pass i writes is read by pass i + 1 alone.
    let feeds_next: Vec<bool> = (0..n)
        .map(|i| {
            let (Some(this), Some(next)) = (passes.get(i), passes.get(i + 1)) else { return false };
            let dst = this.dst;
            if next.a != Some(dst) || [next.b, next.c, next.d].contains(&Some(dst)) {
                return false;
            }
            for later in passes.iter().skip(i + 2) {
                // A copy into a slot keeps what lies outside its rect: that is a read too.
                if reads(later, dst) || (later.dst == dst && later.kernel == Kernel::CopyRect) {
                    return false;
                }
                if later.dst == dst {
                    return true;
                }
            }
            dst != plan.root
        })
        .collect();
    let lead = |p: &plan::Pass<'_>| p.kernel == Kernel::LayerBlend && p.clip.is_none() && !p.gradient;
    let extra = |p: &plan::Pass<'_>| lead(p) && p.mask.is_none() && p.flags & plan::F_TEXT_GAMMA == 0 && p.mode != photocraft_color::BlendMode::Dissolve;
    (0..n)
        .map(|i| {
            let Some(first) = passes.get(i).filter(|p| lead(p)) else { return 1 };
            let (mut k, mut best) = (1, 1);
            while k < BATCH && feeds_next.get(i + k - 1).copied().unwrap_or(false) && passes.get(i + k).is_some_and(extra) {
                k += 1;
                if passes.get(i + k - 1).is_some_and(|p| Some(p.dst) != first.a) {
                    best = k;
                }
            }
            best
        })
        .collect()
}

/// For each pass and the end of the plan, whether the executor draws `runs` (`batch_runs`) so
/// that a pass starts there rather than inside a run.
pub(crate) fn run_starts(runs: &[usize]) -> Vec<bool> {
    let mut starts = vec![false; runs.len() + 1];
    let mut i = 0;
    while let Some(s) = starts.get_mut(i) {
        *s = true;
        i += runs.get(i).copied().unwrap_or(1).max(1);
    }
    starts
}

/// The pipeline that draws `run` (passes `batch_runs` joined): `fs_layerbatch_normal` when every
/// layer blends Normal without text gamma outside Lab documents, else `fs_layerbatch`.
pub(crate) fn batch_kernel(run: &[plan::Pass<'_>]) -> Kernel {
    let normal = |p: &plan::Pass<'_>| p.mode == photocraft_color::BlendMode::Normal && p.flags & (plan::F_TEXT_GAMMA | plan::F_LAB) == 0;
    if run.iter().all(normal) { Kernel::LayerBatchNormal } else { Kernel::LayerBatch }
}

/// Canvas rounded out to whole tiles.
pub(crate) fn tile_grid(canvas: Rect) -> Rect {
    let t = TILE_SIZE;
    Rect::new(canvas.x0.div_euclid(t) * t, canvas.y0.div_euclid(t) * t, (canvas.x1 + t - 1).div_euclid(t) * t, (canvas.y1 + t - 1).div_euclid(t) * t)
}

pub(crate) fn write_tile(queue: &wgpu::Queue, r: &Resident, c: TileCoord, bytes: &[u8]) {
    let tr = c.rect();
    let bpp = r.kind.bytes_per_pixel() as u32;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &r.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: (tr.x0 - r.region.x0) as u32, y: (tr.y0 - r.region.y0) as u32, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(TILE_SIZE as u32 * bpp), rows_per_image: Some(TILE_SIZE as u32) },
        wgpu::Extent3d { width: TILE_SIZE as u32, height: TILE_SIZE as u32, depth_or_array_layers: 1 },
    );
}

/// Submit what `encoder` recorded and continue in a fresh one, then wait for the previous
/// intermediate submit (`last`), so at most two batches of uploads and evicted pages are in
/// flight. The web has no blocking wait and relies on the browser's own scheduling.
pub(crate) fn submit_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    health: Option<&DeviceHealth>,
    timer: Option<&mut timing::GpuTimer>,
    last: &mut Option<wgpu::SubmissionIndex>,
) {
    let mut done = std::mem::replace(encoder, device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_compose") }));
    if let Some(t) = timer {
        // One refresh, several command buffers: the measurement spans all of them.
        t.split(&mut done, encoder);
    }
    let prev = last.replace(queue.submit([done.finish()]));
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(prev) = prev {
        match health {
            Some(h) => {
                h.wait(device, Some(prev));
            }
            None => {
                let _ = device.poll(wgpu::PollType::Wait { submission_index: Some(prev), timeout: None });
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (prev, health);
}
