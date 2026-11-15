// Shadows and Highlights for Camera Raw's GPU preview: `photocraft_algo::camera_raw::local_tone`
// (the fast local Laplacian filter of Paris, Hasinoff and Kautz, SIGGRAPH 2011, in Aubry et al.'s
// form, ACM TOG 2014) as fragment passes, and the order statistics it and Whites and Blacks read.
// `camera_raw_tone.rs` holds the bindings and the order of the passes.
//
// - Pyramid levels are textures. The remapped pyramids of all references sit in one atlas per
//   level: reference k is channel k % 4 of tile k / 4.
// - A quantile is found by splitting an interval of the values' order-preserving u32 keys: each
//   round counts the values at or below each of `splits()` thresholds across the interval (per
//   tile, then summed) and keeps the part that holds the rank, until one key is left: the exact
//   order statistic. Two banks of four channels select eight at once. The results are texels of
//   `stats`: column = slot, row = bank.

struct Step {
    // The output's size (or tile size, in an atlas), then the source's (or tile size).
    sizes: vec4<u32>,
    // A second source's size, the atlas's columns, a tile or cell edge.
    sizes2: vec4<u32>,
    // The tone pass (0 or 1), the statistics slot it reads (or selects into), a mode, a second slot.
    misc: vec4<u32>,
    // A selection's ranks per bank.
    ranks: array<vec4<u32>, 2>,
}

struct Tone {
    // Per pass: kind (0 Highlights, 1 Shadows), amount, sigma, unused.
    passes: array<vec4<f32>, 2>,
    // Per pass, two rows of five knots: pass p, row r, knot i at p * 10 + r * 5 + i.
    knots: array<vec4<f32>, 5>,
    // REF_STEP, MAX_REFS.
    refs: vec4<f32>,
    // The pyramid's five taps.
    taps: vec4<f32>,
    taps2: vec4<f32>,
    // Whites, Blacks (sliders), the model's row step and least Blacks adaptation on photographs.
    wb: vec4<f32>,
    // The internal amount of the first Whites row and of the first Blacks row.
    wb2: vec4<f32>,
    // The internal amounts Whites and Blacks reach: [lo, hi] each.
    wb3: vec4<f32>,
    // Model rows: Whites curves, Blacks curves, Whites adaptation, Blacks adaptation.
    model: vec4<u32>,
    // Model rows: black points, white points; their counts.
    model2: vec4<u32>,
    // Counts of the Whites and Blacks curves.
    model3: vec4<u32>,
}

@group(0) @binding(0) var<uniform> S: Step;
@group(0) @binding(1) var<uniform> T: Tone;
@group(0) @binding(2) var plane_a: texture_2d<f32>;
@group(0) @binding(3) var plane_b: texture_2d<f32>;
@group(0) @binding(4) var atlas_fine: texture_2d<f32>;
@group(0) @binding(5) var atlas_coarse: texture_2d<f32>;
@group(0) @binding(6) var stats: texture_2d<u32>;
@group(0) @binding(7) var uint_a: texture_2d<u32>;
@group(0) @binding(8) var uint_b: texture_2d<u32>;
@group(0) @binding(9) var uint_c: texture_2d<u32>;
@group(0) @binding(10) var model: texture_2d<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}

fn pixel(pos: vec4<f32>) -> vec2<i32> {
    return vec2<i32>(floor(pos.xy));
}

fn dst_size() -> vec2<i32> {
    return vec2<i32>(S.sizes.xy);
}

fn src_size() -> vec2<i32> {
    return vec2<i32>(S.sizes.zw);
}

fn tap(k: i32) -> f32 {
    if k < 4 {
        return T.taps[k];
    }
    return T.taps2.x;
}

// `local_tone::mirror`: `i` mirrored into 0..n, the edge sample not repeated.
fn mirror(i: i32, n: i32) -> i32 {
    if n == 1 {
        return 0;
    }
    let p = 2 * (n - 1);
    var m = i % p;
    if m < 0 {
        m += p;
    }
    return select(m, p - m, m >= n);
}

// Order-preserving keys of f32 values (negative values below positive ones, -0 below +0).
fn key(v: f32) -> u32 {
    let b = bitcast<u32>(v);
    return select(b ^ 0x80000000u, ~b, (b & 0x80000000u) != 0u);
}

fn key4(v: vec4<f32>) -> vec4<u32> {
    return vec4(key(v.x), key(v.y), key(v.z), key(v.w));
}

fn unkey(k: u32) -> f32 {
    return bitcast<f32>(select(~k, k ^ 0x80000000u, (k & 0x80000000u) != 0u));
}

fn stat(slot: u32, bank: i32) -> vec4<u32> {
    return textureLoad(stats, vec2(i32(slot), bank), 0);
}

// ---- The filter's parameters (`local_tone::llf` and `TonePass::run`) ----

struct Llf {
    lo: f32,
    hi: f32,
    step: f32,
    refs: i32,
    // `hi - lo` is at most 1e-6 (or NaN): the filter leaves its input as it is.
    flat: bool,
    // The image range the strengths are placed in.
    range_lo: f32,
    span: f32,
}

// From a pass's input selection: bank 0 holds the REF_LOW_QUANTILE quantile, the kept one and the
// maximum; bank 1 the RANGE_QUANTILES of the cells.
fn llf(slot: u32) -> Llf {
    let a = stat(slot, 0);
    let b = stat(slot, 1);
    var l: Llf;
    l.hi = unkey(a.z);
    l.lo = min(unkey(a.x), l.hi);
    let d = l.hi - l.lo;
    l.flat = !(d > 1e-6);
    l.refs = i32(clamp(ceil(d / T.refs.x) + 1.0, 2.0, T.refs.y));
    l.step = d / f32(l.refs - 1);
    l.range_lo = unkey(b.x);
    l.span = max(unkey(b.y) - l.range_lo, 1.0);
    return l;
}

fn knot_at(p: u32, row: u32, i: u32) -> f32 {
    let k = p * 10u + row * 5u + i;
    return T.knots[k / 4u][k % 4u];
}

// `local_tone::knot`.
fn knot(t: f32, p: u32, row: u32) -> f32 {
    let x = clamp(t, 0.0, 1.0) * 4.0;
    let i = min(u32(x), 3u);
    let a = knot_at(p, row, i);
    return a + (knot_at(p, row, i + 1u) - a) * (x - f32(i));
}

// tanh that stays finite for large arguments (excess() reaches |x| of several hundred).
fn tanh_(x: f32) -> f32 {
    let e = exp(-2.0 * abs(x));
    return sign(x) * (1.0 - e) / (1.0 + e);
}

fn excess(d: f32, sigma: f32) -> f32 {
    return d - sigma * tanh_(d / sigma);
}

// `TonePass::remap` of pass `p`.
fn remap(x: f32, g: f32, p: u32, l: Llf) -> f32 {
    let pp = T.passes[p];
    if pp.x == 0.0 {
        return x + knot((g - l.range_lo) / l.span, p, 0u) * pp.y * excess(max(x - g, 0.0), pp.z);
    }
    let below = clamp((g - x) / pp.z, 0.0, 1.0);
    if below == 0.0 {
        return x;
    }
    let w = below * below * (3.0 - 2.0 * below);
    return x + pp.y * knot((x - l.range_lo) / l.span, p, 0u) * knot((g - l.range_lo) / l.span, p, 1u) * w;
}

fn remap4(x: f32, r: vec4<f32>, p: u32, l: Llf) -> vec4<f32> {
    return vec4(remap(x, r.x, p, l), remap(x, r.y, p, l), remap(x, r.z, p, l), remap(x, r.w, p, l));
}

// ---- The pyramids (`local_tone::down`, `up`) ----

// The input's Gaussian pyramid: the next coarser level of `plane_a`.
@fragment
fn fs_down(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let n = src_size();
    var acc = 0.0;
    for (var kx = 0; kx < 5; kx++) {
        let x = mirror(2 * p.x + kx - 2, n.x);
        var col = 0.0;
        for (var ky = 0; ky < 5; ky++) {
            col += tap(ky) * textureLoad(plane_a, vec2(x, mirror(2 * p.y + ky - 2, n.y)), 0).r;
        }
        acc += tap(kx) * col;
    }
    return vec4(acc, 0.0, 0.0, 1.0);
}

// The tile and its references of an atlas pixel: (group, local x, local y).
fn atlas_cell(p: vec2<i32>, tile: vec2<i32>) -> vec3<i32> {
    let t = p / tile;
    return vec3(t.y * i32(S.sizes2.z) + t.x, p - t * tile);
}

fn tile_origin(group: i32, tile: vec2<i32>) -> vec2<i32> {
    let cols = i32(S.sizes2.z);
    return vec2(group % cols, group / cols) * tile;
}

// Level 1 of the remapped pyramids: `plane_a` (the input) remapped at the tile's four
// references, then blurred and subsampled.
@fragment
fn fs_down_remap(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let tile = dst_size();
    let c = atlas_cell(pixel(pos), tile);
    let l = llf(S.misc.y);
    if l.flat || 4 * c.x >= l.refs {
        return vec4(0.0);
    }
    let k = f32(4 * c.x);
    let r = vec4(l.lo + l.step * k, l.lo + l.step * (k + 1.0), l.lo + l.step * (k + 2.0), l.lo + l.step * (k + 3.0));
    let n = src_size();
    var acc = vec4(0.0);
    for (var kx = 0; kx < 5; kx++) {
        let x = mirror(2 * c.y + kx - 2, n.x);
        var col = vec4(0.0);
        for (var ky = 0; ky < 5; ky++) {
            let v = textureLoad(plane_a, vec2(x, mirror(2 * c.z + ky - 2, n.y)), 0).r;
            col += tap(ky) * remap4(v, r, S.misc.x, l);
        }
        acc += tap(kx) * col;
    }
    return acc;
}

// The next level of the remapped pyramids, tile by tile.
@fragment
fn fs_down_atlas(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let tile = dst_size();
    let c = atlas_cell(pixel(pos), tile);
    let l = llf(S.misc.y);
    if l.flat || 4 * c.x >= l.refs {
        return vec4(0.0);
    }
    let n = src_size();
    let o = tile_origin(c.x, n);
    var acc = vec4(0.0);
    for (var kx = 0; kx < 5; kx++) {
        let x = mirror(2 * c.y + kx - 2, n.x);
        var col = vec4(0.0);
        for (var ky = 0; ky < 5; ky++) {
            col += tap(ky) * textureLoad(atlas_fine, o + vec2(x, mirror(2 * c.z + ky - 2, n.y)), 0);
        }
        acc += tap(kx) * col;
    }
    return acc;
}

// `local_tone::up`'s source sample for index `i` of a zero-stuffed axis of length `n` holding
// `lim` samples, or -1 where the stuffed sample is zero.
fn up_src(i: i32, n: i32, lim: i32) -> i32 {
    let m = mirror(i, n);
    if m % 2 == 0 && m / 2 < lim {
        return m / 2;
    }
    return -1;
}

// `plane_b` (`coarse` texels) brought up to the output level, at `p`.
fn up_plane(p: vec2<i32>, fine: vec2<i32>, coarse: vec2<i32>) -> f32 {
    let lim = min(coarse, (fine + vec2(1)) / 2);
    var acc = 0.0;
    for (var kx = 0; kx < 5; kx++) {
        let jx = up_src(p.x + kx - 2, fine.x, lim.x);
        if jx < 0 {
            continue;
        }
        var col = 0.0;
        for (var ky = 0; ky < 5; ky++) {
            let jy = up_src(p.y + ky - 2, fine.y, lim.y);
            if jy >= 0 {
                col += tap(ky) * (textureLoad(plane_b, vec2(jx, jy), 0).r * 4.0);
            }
        }
        acc += tap(kx) * col;
    }
    return acc;
}

// Reference `k`'s coarse level (`atlas_coarse`, `coarse` per tile) brought up, at `p`.
fn up_ref(p: vec2<i32>, fine: vec2<i32>, coarse: vec2<i32>, k: i32) -> f32 {
    let lim = min(coarse, (fine + vec2(1)) / 2);
    let o = tile_origin(k / 4, coarse);
    let c = k % 4;
    var acc = 0.0;
    for (var kx = 0; kx < 5; kx++) {
        let jx = up_src(p.x + kx - 2, fine.x, lim.x);
        if jx < 0 {
            continue;
        }
        var col = 0.0;
        for (var ky = 0; ky < 5; ky++) {
            let jy = up_src(p.y + ky - 2, fine.y, lim.y);
            if jy >= 0 {
                col += tap(ky) * (textureLoad(atlas_coarse, o + vec2(jx, jy), 0)[c] * 4.0);
            }
        }
        acc += tap(kx) * col;
    }
    return acc;
}

// One level of the output, coarse to fine: the coarser output (`plane_b`) brought up plus this
// level's Laplacian coefficients, each interpolated between the two references around the
// input's level (`plane_a`) there. Mode 1 is level 0, whose remapped levels are the remapping
// itself; above it they are tiles of `atlas_fine`.
@fragment
fn fs_lap(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let l = llf(S.misc.y);
    let gl = textureLoad(plane_a, p, 0).r;
    if l.flat {
        return vec4(gl, 0.0, 0.0, 1.0);
    }
    let fine = dst_size();
    let coarse = src_size();
    let gc = clamp(gl, l.lo, l.hi);
    let k0 = i32(floor((gc - l.lo) / l.step));
    var lap = 0.0;
    for (var k = max(k0 - 1, 0); k <= min(k0 + 2, l.refs - 1); k++) {
        let r = l.lo + l.step * f32(k);
        let wgt = max(1.0 - abs(gc - r) / l.step, 0.0);
        if wgt > 0.0 {
            var f: f32;
            if S.misc.z == 1u {
                f = remap(gl, r, S.misc.x, l);
            } else {
                f = textureLoad(atlas_fine, tile_origin(k / 4, fine) + p, 0)[k % 4];
            }
            lap += wgt * (f - up_ref(p, fine, coarse, k));
        }
    }
    return vec4(up_plane(p, fine, coarse) + lap, 0.0, 0.0, 1.0);
}

// The second pass's input: the first one's output with its kept quantile put back.
@fragment
fn fs_shift(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let shift = unkey(stat(S.misc.y, 0).y) - unkey(stat(S.misc.w, 0).x);
    return vec4(textureLoad(plane_a, pixel(pos), 0).r + shift, 0.0, 0.0, 1.0);
}

// `local_tone::range`'s cells: `plane_a` averaged over `edge × edge` blocks.
@fragment
fn fs_cells(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let f = i32(S.sizes2.w);
    let n = src_size();
    let end = min((p + vec2(1)) * f, n);
    var sum = 0.0;
    for (var y = p.y * f; y < end.y; y++) {
        for (var x = p.x * f; x < end.x; x++) {
            sum += textureLoad(plane_a, vec2(x, y), 0).r;
        }
    }
    let count = (end.x - p.x * f) * (end.y - p.y * f);
    return vec4(sum / f32(count), 0.0, 0.0, 1.0);
}

// ---- Order statistics ----

struct Banks {
    @location(0) a: vec4<u32>,
    @location(1) b: vec4<u32>,
}

// Every key: the selection's starting interval.
@fragment
fn fs_init(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    return select(vec4(0xffffffffu), vec4(0u), pixel(pos).x == 0);
}

fn splits() -> u32 {
    return S.misc.w;
}

// Threshold `t` (0 … splits − 1) of the interval [lo, hi]: the last key of its part `t`, the parts
// as even as integers allow (later thresholds clamp to `hi` on a narrow interval).
fn threshold(lo: vec4<u32>, hi: vec4<u32>, t: u32) -> vec4<u32> {
    let d = hi - lo;
    let q = d / (splits() + 1u) + vec4(1u);
    return lo + min(q * (t + 1u) - vec4(1u), d);
}

fn interval(bank: i32) -> array<vec4<u32>, 2> {
    return array(textureLoad(uint_a, vec2(0, bank), 0), textureLoad(uint_a, vec2(1, bank), 0));
}

// Per `edge × edge` tile and threshold (a column per threshold): how many keys are at or below
// it. Bank 0 counts `plane_a` (its red channel in all four, or mode 1 its four channels), bank 1
// `plane_b`'s red channel (`sizes2.xy` texels; none when 0).
@fragment
fn fs_count(@builtin(position) pos: vec4<f32>) -> Banks {
    let g = pixel(pos);
    let t = vec2(g.x / i32(splits()), g.y);
    let split = u32(g.x) % splits();
    let f = i32(S.sizes2.w);
    var out: Banks;
    out.a = vec4(0u);
    out.b = vec4(0u);
    let i0 = interval(0);
    let m0 = threshold(i0[0], i0[1], split);
    let end = min((t + vec2(1)) * f, src_size());
    for (var y = t.y * f; y < end.y; y++) {
        for (var x = t.x * f; x < end.x; x++) {
            var v = textureLoad(plane_a, vec2(x, y), 0);
            if S.misc.z == 0u {
                v = vec4(v.r);
            }
            out.a += select(vec4(0u), vec4(1u), key4(v) <= m0);
        }
    }
    let end2 = min((t + vec2(1)) * f, vec2<i32>(S.sizes2.xy));
    if end2.x > t.x * f && end2.y > t.y * f {
        let i1 = interval(1);
        let m1 = threshold(i1[0], i1[1], split);
        for (var y = t.y * f; y < end2.y; y++) {
            for (var x = t.x * f; x < end2.x; x++) {
                out.b += select(vec4(0u), vec4(1u), vec4(key(textureLoad(plane_b, vec2(x, y), 0).r)) <= m1);
            }
        }
    }
    return out;
}

// Counts of both banks (`uint_a`, `uint_b`) summed over `edge × edge` tiles, per threshold.
// Sizes are in tiles.
@fragment
fn fs_reduce(@builtin(position) pos: vec4<f32>) -> Banks {
    let g = pixel(pos);
    let n = i32(splits());
    let t = vec2(g.x / n, g.y);
    let split = g.x % n;
    let f = i32(S.sizes2.w);
    let end = min((t + vec2(1)) * f, src_size());
    var out: Banks;
    out.a = vec4(0u);
    out.b = vec4(0u);
    for (var y = t.y * f; y < end.y; y++) {
        for (var x = t.x * f; x < end.x; x++) {
            out.a += textureLoad(uint_a, vec2(x * n + split, y), 0);
            out.b += textureLoad(uint_b, vec2(x * n + split, y), 0);
        }
    }
    return out;
}

// One round: texel (0, bank) is the interval's low end, (1, bank) its high end. `uint_a` is the
// interval, `uint_b` and `uint_c` the two banks' counts (all their tiles summed here). The new
// interval is the part ending at the first threshold with more than `rank` keys at or below it.
@fragment
fn fs_update(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    let p = pixel(pos);
    let n = src_size();
    let i = interval(p.y);
    let rank = S.ranks[p.y];
    var lo = i[0];
    var hi = i[1];
    var found = vec4(false);
    var prev = i[0];
    for (var s = 0u; s < splits(); s++) {
        var count = vec4(0u);
        for (var y = 0; y < n.y; y++) {
            for (var x = 0; x < n.x; x++) {
                let at = vec2(x * i32(splits()) + i32(s), y);
                if p.y == 0 {
                    count += textureLoad(uint_b, at, 0);
                } else {
                    count += textureLoad(uint_c, at, 0);
                }
            }
        }
        let thr = threshold(i[0], i[1], s);
        let take = !found & (count > rank);
        hi = select(hi, thr, take);
        if s > 0u {
            lo = select(lo, prev + vec4(1u), take);
        }
        found = found | take;
        prev = thr;
    }
    // Past the last threshold (below `hi`, or it would have been taken).
    lo = select(prev + vec4(1u), lo, found);
    return select(hi, lo, p.x == 0);
}

// ---- Whites and Blacks after Shadows and Highlights (`whites_blacks::amounts`, `row_at`) ----

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn model_at(x: i32, row: u32) -> f32 {
    return textureLoad(model, vec2(x, i32(row)), 0).r;
}

// `whites_blacks::adapt`'s index into a grid of `n` points on model row `row`: (index, fraction).
fn grid_index(row: u32, n: i32, v: f32) -> vec2<f32> {
    let w = clamp(v, model_at(0, row), model_at(n - 1, row));
    var i = n - 2;
    for (var k = 0; k < n - 1; k++) {
        if w <= model_at(k + 1, row) {
            i = k;
            break;
        }
    }
    let g = model_at(i, row);
    return vec2(f32(i), (w - g) / (model_at(i + 1, row) - g));
}

// `whites_blacks::adapt` of the table starting at model row `table`.
fn adapt(table: u32, bp: f32, wp: f32) -> f32 {
    let bi = grid_index(T.model2.x, i32(T.model2.z), bp);
    let wi = grid_index(T.model2.y, i32(T.model2.w), wp);
    let i = u32(bi.x);
    let j = i32(wi.x);
    let t = wi.y;
    let r0 = model_at(j, table + i) * (1.0 - t) + model_at(j + 1, table + i) * t;
    let r1 = model_at(j, table + i + 1u) * (1.0 - t) + model_at(j + 1, table + i + 1u) * t;
    return r0 * (1.0 - bi.y) + r1 * bi.y;
}

// Rows 5 (Whites) and 6 (Blacks) of `camera_raw.wgsl`'s tables, at the internal amounts for the
// image's points (slot `misc.y`: the black point's luminance, the white point's channel, linear).
@fragment
fn fs_wb_rows(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let pts = stat(S.misc.y, 0);
    let bp = linear_to_srgb(unkey(pts.x));
    let wp = linear_to_srgb(unkey(pts.y));
    let whites = p.y == i32(S.misc.z);
    var x: f32;
    var table: u32;
    var rows: u32;
    var row0: f32;
    if whites {
        x = T.wb.x;
        if x > 0.0 {
            x = x * adapt(T.model.z, bp, wp);
        }
        x = clamp(x, T.wb3.x, T.wb3.y);
        table = T.model.x;
        rows = T.model3.x;
        row0 = T.wb2.x;
    } else {
        x = T.wb.y;
        if x < 0.0 {
            x = x * max(adapt(T.model.w, bp, wp), T.wb.w);
        }
        x = clamp(x, T.wb3.z, T.wb3.w);
        table = T.model.y;
        rows = T.model3.y;
        row0 = T.wb2.y;
    }
    let r = clamp((x - row0) / T.wb.z, 0.0, f32(rows - 1u));
    let i = min(u32(r), rows - 2u);
    let s = r - f32(i);
    return vec4(model_at(p.x, table + i) * (1.0 - s) + model_at(p.x, table + i + 1u) * s, 0.0, 0.0, 1.0);
}
