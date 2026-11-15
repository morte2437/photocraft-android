// Camera Raw's per-pixel develop stages (`photocraft_algo::camera_raw::develop`, constants from
// `camera_raw::per_pixel`), one fragment per pixel. Each function mirrors the CPU code it is
// named after, in the same order of operations; `camera_raw.rs` holds the bindings. The spatial
// stages' filters (`camera_raw_tone.wgsl`, `camera_raw_filters.wgsl`) start from passes here
// (`fs_log`, `fs_luma`, `fs_points`; Dehaze from a copy the CPU makes) and end here: Dehaze in `fs_dehaze`, Shadows
// and Highlights as a gain per pixel, Clarity and Texture in `tonal`.

struct Params {
    size: vec2<u32>,
    flags: u32,
    // The document's transfer curve: 0 for sRGB's, else a power-law gamma.
    encoding: f32,
    // White balance gains (rgb), Contrast's strength (w).
    gains: vec4<f32>,
    // Vibrance, Saturation.
    presence: vec4<f32>,
    // Color Mixer bands 0-3 and 4-7: hue, saturation, luminance.
    hsl: array<vec4<f32>, 6>,
    // Color Grading wheels: tint (rgb), luminance (w).
    tints: array<vec4<f32>, 4>,
    // Blending, Balance.
    grade: vec4<f32>,
    // [amount, highlights, style, power], [centre, axes], [edges].
    vignette: array<vec4<f32>, 3>,
    // Cell, roughness, amount, full-resolution pixels per pixel.
    grain: vec4<f32>,
    // The two octaves' seeds as (low, high) words.
    seeds: vec4<u32>,
    // Shadows and Highlights: the last pass's statistics slots (input, output); the Whites and
    // Blacks cells' edge.
    tone: vec4<u32>,
    // The log luminance's floor.
    floor_: vec4<f32>,
    // Dehaze, Clarity and Texture's strengths, Texture's limit.
    spatial: vec4<f32>,
    // Dehaze's and Clarity's subsampling, the transmission's floor.
    spatial2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
// R32Float rows of `LUT_WIDTH`: parametric, master, red, green, blue curves; Whites, Blacks rows.
@group(0) @binding(2) var luts: texture_2d<f32>;
// Shadows and Highlights: the log luminance before and after, and the order statistics.
@group(0) @binding(3) var log_in: texture_2d<f32>;
@group(0) @binding(4) var log_out: texture_2d<f32>;
@group(0) @binding(5) var stats: texture_2d<u32>;
// Dehaze: stages 1 and 2 (read instead of `src` once rendered), the transmission (red) on the
// copy, the atmospheric light (one texel).
@group(0) @binding(6) var base: texture_2d<f32>;
@group(0) @binding(7) var haze: texture_2d<f32>;
@group(0) @binding(8) var air: texture_2d<f32>;
// Clarity's coefficients (a, b) on its copy; Texture's blur (red).
@group(0) @binding(9) var clarity: texture_2d<f32>;
@group(0) @binding(10) var blur: texture_2d<f32>;

const GAINS: u32 = 0u;
// The luminance stage: Contrast, Clarity or Texture.
const LUMINANCE: u32 = 1u;
const WHITES: u32 = 2u;
const BLACKS: u32 = 3u;
const PER_CHANNEL: u32 = 4u;
const PRESENCE: u32 = 5u;
const PARAMETRIC: u32 = 6u;
const MASTER: u32 = 7u;
const RED: u32 = 8u;
const HSL: u32 = 11u;
const GRADE: u32 = 12u;
const VIGNETTE: u32 = 13u;
const GRAIN: u32 = 14u;
const CLAMP: u32 = 15u;
const LOCAL_TONE: u32 = 16u;
const DEHAZE: u32 = 17u;
const CLARITY: u32 = 18u;
const TEXTURE: u32 = 19u;

const CURVE_LUT: i32 = 1024;
const WB_STEPS: i32 = 33;
const TAU: f32 = 6.2831855;

struct VOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}

fn on(flag: u32) -> bool {
    return (P.flags & (1u << flag)) != 0u;
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

fn smoothstep_(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn rem_euclid(x: f32, m: f32) -> f32 {
    let r = x % m;
    return select(r, r + m, r < 0.0);
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn decode(v: f32) -> f32 {
    if P.encoding > 0.0 {
        return pow(max(v, 0.0), P.encoding);
    }
    return srgb_to_linear(v);
}

fn encode(v: f32) -> f32 {
    if P.encoding > 0.0 {
        return pow(max(v, 0.0), 1.0 / P.encoding);
    }
    return linear_to_srgb(v);
}

fn lut_at(row: i32, i: i32) -> f32 {
    return textureLoad(luts, vec2(i, row), 0).r;
}

// `lut_eval`: linear inside 0..1, extended linearly outside (float documents).
fn lut_eval(row: i32, v: f32) -> f32 {
    if v < 0.0 {
        return v + lut_at(row, 0);
    }
    if v > 1.0 {
        return lut_at(row, CURVE_LUT - 1) + (v - 1.0);
    }
    let x = v * f32(CURVE_LUT - 1);
    let i = min(i32(x), CURVE_LUT - 2);
    let t = x - f32(i);
    let a = lut_at(row, i);
    return a + (lut_at(row, i + 1) - a) * t;
}

// `whites_blacks::curve` on one row.
fn wb_curve(row: i32, v: f32) -> f32 {
    if v >= 1.0 {
        return 1.0;
    }
    let p = max(v, 0.0) * f32(WB_STEPS - 1);
    let j = min(i32(p), WB_STEPS - 2);
    let s = p - f32(j);
    let a = lut_at(row, j);
    return a + (lut_at(row, j + 1) - a) * s;
}

fn grey(v: f32) -> f32 {
    var g = v;
    if on(WHITES) {
        g = wb_curve(5, g);
    }
    if on(BLACKS) {
        g = wb_curve(6, g);
    }
    return g;
}

// `whites_blacks::apply`.
fn whites_blacks(q: vec3<f32>) -> vec3<f32> {
    let lin = vec3(decode(q.x), decode(q.y), decode(q.z));
    var out = q;
    if on(PER_CHANNEL) {
        for (var c = 0; c < 3; c++) {
            let over = max(lin[c] - 1.0, 0.0);
            out[c] = encode(srgb_to_linear(grey(linear_to_srgb(min(lin[c], 1.0)))) + over);
        }
        return out;
    }
    let y = max(luma(lin), 0.0);
    if y <= 1e-7 {
        return q;
    }
    let y2 = srgb_to_linear(grey(linear_to_srgb(min(y, 1.0)))) + max(y - 1.0, 0.0);
    let k = y2 / y;
    for (var c = 0; c < 3; c++) {
        out[c] = encode(lin[c] * k);
    }
    return out;
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let mx = max(max(c.x, c.y), c.z);
    let mn = min(min(c.x, c.y), c.z);
    let d = mx - mn;
    var h = 0.0;
    if d <= 1e-9 {
        h = 0.0;
    } else if mx == c.x {
        h = 60.0 * rem_euclid((c.y - c.z) / d, 6.0);
    } else if mx == c.y {
        h = 60.0 * ((c.z - c.x) / d + 2.0);
    } else {
        h = 60.0 * ((c.x - c.y) / d + 4.0);
    }
    return vec3(h, select(0.0, d / mx, mx > 1e-9), mx);
}

fn hsv_to_rgb(hue: f32, s: f32, v: f32) -> vec3<f32> {
    let h = rem_euclid(hue, 360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - abs((h % 2.0) - 1.0));
    var rgb: vec3<f32>;
    switch u32(h) {
        case 0u: { rgb = vec3(c, x, 0.0); }
        case 1u: { rgb = vec3(x, c, 0.0); }
        case 2u: { rgb = vec3(0.0, c, x); }
        case 3u: { rgb = vec3(0.0, x, c); }
        case 4u: { rgb = vec3(x, 0.0, c); }
        default: { rgb = vec3(c, 0.0, x); }
    }
    let m = v - c;
    return rgb + vec3(m);
}

const HSL_BANDS = array<f32, 8>(0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0);

fn band(k: i32, row: i32) -> f32 {
    return P.hsl[row * 2 + k / 4][k % 4];
}

// `band_weights` folded into the three sums.
fn hsl_shift(h: f32) -> vec3<f32> {
    var bands = HSL_BANDS;
    var d = vec3(0.0);
    for (var k = 0; k < 8; k++) {
        let c = bands[k];
        let prev = bands[(k + 7) % 8];
        let next = bands[(k + 1) % 8];
        let diff = ((h - c + 540.0) % 360.0) - 180.0;
        let span = select(rem_euclid(c - prev, 360.0), rem_euclid(next - c, 360.0), diff >= 0.0);
        let w = max(1.0 - abs(diff) / max(span, 1e-3), 0.0);
        d += w * vec3(band(k, 0), band(k, 1), band(k, 2));
    }
    return d;
}

// `photo_util::hash01`: 64-bit words as (low, high).
fn mul_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2((p00 & 0xffffu) | (mid << 16u), p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let w = mul_wide(a.x, b.x);
    return vec2(w.x, w.y + a.x * b.y + a.y * b.x);
}

fn shr64(z: vec2<u32>, n: u32) -> vec2<u32> {
    return vec2((z.x >> n) | (z.y << (32u - n)), z.y >> n);
}

fn i64_(v: i32) -> vec2<u32> {
    return vec2(bitcast<u32>(v), select(0u, 0xffffffffu, v < 0));
}

fn hash01(x: i32, y: i32, seed: vec2<u32>) -> f32 {
    var z = mul64(i64_(x), vec2(0x7f4a7c15u, 0x9e3779b9u)) ^ mul64(i64_(y), vec2(0x27d4eb4fu, 0xc2b2ae3du)) ^ mul64(seed, vec2(0x9e3779f9u, 0x165667b1u));
    z = mul64(z ^ shr64(z, 30u), vec2(0x1ce4e5b9u, 0xbf58476du));
    z = mul64(z ^ shr64(z, 27u), vec2(0x133111ebu, 0x94d049bbu));
    z = z ^ shr64(z, 31u);
    return f32(z.y >> 8u) / 16777216.0;
}

fn value_noise(x: f32, y: f32, seed: vec2<u32>) -> f32 {
    let xi = floor(x);
    let yi = floor(y);
    let tx = x - xi;
    let ty = y - yi;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let ix = i32(xi);
    let iy = i32(yi);
    let h00 = hash01(ix, iy, seed);
    let h10 = hash01(ix + 1, iy, seed);
    let h01 = hash01(ix, iy + 1, seed);
    let h11 = hash01(ix + 1, iy + 1, seed);
    let top = h00 + (h10 - h00) * sx;
    let bot = h01 + (h11 - h01) * sx;
    return top + (bot - top) * sy;
}

fn gained(v: f32, g: f32) -> f32 {
    return linear_to_srgb(max(srgb_to_linear(v) * g, 0.0));
}

// 1. White balance and exposure. (Whole vectors: FXC unrolls any loop around a component written
// by index, and `fs_points` loops over this.)
fn balance(q: vec3<f32>) -> vec3<f32> {
    if on(GAINS) {
        return vec3(gained(q.x, P.gains.x), gained(q.y, P.gains.y), gained(q.z, P.gains.z));
    }
    return q;
}

// `camera_raw_tone.wgsl`'s `unkey`.
fn unkey(k: u32) -> f32 {
    return bitcast<f32>(select(~k, k ^ 0x80000000u, (k & 0x80000000u) != 0u));
}

// `photo_util::bilinear` of texture `t` at (x, y), edges clamped.
fn bilinear(t: texture_2d<f32>, x_: f32, y_: f32) -> vec4<f32> {
    let n = vec2<i32>(textureDimensions(t));
    let x = clamp(x_, 0.0, f32(n.x - 1));
    let y = clamp(y_, 0.0, f32(n.y - 1));
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, n.x - 1);
    let y1 = min(y0 + 1, n.y - 1);
    let tx = x - f32(x0);
    let ty = y - f32(y0);
    let p00 = textureLoad(t, vec2(x0, y0), 0);
    let p10 = textureLoad(t, vec2(x1, y0), 0);
    let p01 = textureLoad(t, vec2(x0, y1), 0);
    let p11 = textureLoad(t, vec2(x1, y1), 0);
    let top = p00 + (p10 - p00) * tx;
    let bot = p01 + (p11 - p01) * tx;
    return top + (bot - top) * ty;
}

// Where pixel `p` of a copy subsampled `s` times sits: develop's `(x + 0.5) / s - 0.5`.
fn subsampled(p: vec2<i32>, s: f32) -> vec2<f32> {
    return (vec2<f32>(p) + vec2(0.5)) / s - vec2(0.5);
}

// Stages 1 and 2 at `p`: the dehazed frame once `fs_dehaze` rendered it.
fn source(p: vec2<i32>) -> vec3<f32> {
    if on(DEHAZE) {
        return textureLoad(base, p, 0).rgb;
    }
    return balance(textureLoad(src, p, 0).rgb);
}

// Stages 1 to 3 up to Shadows and Highlights at `p`.
fn lit(p: vec2<i32>) -> vec3<f32> {
    var c = source(p);
    // 3. Shadows and Highlights: the light scaled by the change of the log luminance, the last
    // pass's kept quantile put back.
    if on(LOCAL_TONE) {
        let shift = unkey(textureLoad(stats, vec2(i32(P.tone.x), 0), 0).y) - unkey(textureLoad(stats, vec2(i32(P.tone.y), 0), 0).x);
        let change = (textureLoad(log_out, p, 0).r + shift) - textureLoad(log_in, p, 0).r;
        let gain = exp2(change);
        c = vec3(encode(decode(c.x) * gain), encode(decode(c.y) * gain), encode(decode(c.z) * gain));
    }
    return c;
}

// Stages 1 to 3 on the pixel at `p`: what Whites and Blacks measure and apply to.
fn tonal(p: vec2<i32>) -> vec3<f32> {
    var c = lit(p);
    // 3. Clarity, Texture and Contrast on the luminance (develop's `tone`).
    if on(LUMINANCE) {
        let l0 = max(luma(c), 0.0);
        let lc = min(l0, 1.0);
        var lv = lc;
        if on(CLARITY) {
            let at = subsampled(p, P.spatial2.y);
            let ab = bilinear(clarity, at.x, at.y);
            let base_ = ab.x * lc + ab.y;
            let d = 2.0 * clamp(lv, 0.0, 1.0) - 1.0;
            let mw = 1.0 - d * d;
            lv += P.spatial.y * (lc - base_) * mw;
        }
        if on(TEXTURE) {
            lv += P.spatial.z * clamp(lc - textureLoad(blur, p, 0).r, -P.spatial.w, P.spatial.w);
        }
        if P.gains.w != 0.0 {
            let xx = clamp(lv, 0.0, 1.0);
            lv += -P.gains.w * sin(TAU * xx) / TAU;
        }
        lv = max(lv, 0.0) + (l0 - lc);
        if l0 > 1e-5 {
            c = c * (lv / l0);
        } else {
            c = c + vec3(lv - l0);
        }
    }
    return c;
}

// 2. Dehaze: each pixel moved away from the atmospheric light by the transmission (brought up
// from the copy), or toward it for negative amounts.
@fragment
fn fs_dehaze(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let q = textureLoad(src, p, 0);
    let i = balance(q.rgb);
    let at = subsampled(p, P.spatial2.x);
    let tv = clamp(bilinear(haze, at.x, at.y).r, P.spatial2.z, 1.0);
    let a = textureLoad(air, vec2(0, 0), 0).rgb;
    let amount = P.spatial.x;
    if amount >= 0.0 {
        let j = (i - a) / tv + a;
        return vec4(i + (j - i) * amount, q.a);
    }
    return vec4(i + (a - i) * (-amount) * 0.5 * (1.0 - tv * 0.5), q.a);
}

// The log luminance Shadows and Highlights filter (`local_tone::apply`).
@fragment
fn fs_log(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = source(vec2<i32>(floor(pos.xy)));
    let l = max(luma(vec3(decode(c.x), decode(c.y), decode(c.z))), 0.0);
    return vec4(log2(l + P.floor_.x), 0.0, 0.0, 1.0);
}

// Clarity and Texture's input: the luminance after Shadows and Highlights, clipped to 0 … 1.
@fragment
fn fs_luma(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = lit(vec2<i32>(floor(pos.xy)));
    return vec4(min(max(luma(c), 0.0), 1.0), 0.0, 0.0, 1.0);
}

// `luminance_points`' cells after Shadows and Highlights: the luminance and the brightest
// channel of each `edge × edge` block's mean light (clipped to 0 … 1).
@fragment
fn fs_points(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let t = vec2<i32>(floor(pos.xy));
    let f = i32(P.tone.z);
    let end = min((t + vec2(1)) * f, vec2<i32>(P.size));
    var sum = vec3(0.0);
    for (var y = t.y * f; y < end.y; y++) {
        for (var x = t.x * f; x < end.x; x++) {
            let c = tonal(vec2(x, y));
            sum += clamp(vec3(decode(c.x), decode(c.y), decode(c.z)), vec3(0.0), vec3(1.0));
        }
    }
    let m = sum / f32((end.x - t.x * f) * (end.y - t.y * f));
    return vec4(luma(m), max(max(m.x, m.y), m.z), 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let q = textureLoad(src, p, 0);
    var c = tonal(p);
    // 3b. Whites and Blacks.
    if on(WHITES) || on(BLACKS) {
        c = whites_blacks(c);
    }
    // 4. Vibrance and Saturation.
    if on(PRESENCE) {
        let l = luma(c);
        let mx = max(max(c.x, c.y), c.z);
        let mn = min(min(c.x, c.y), c.z);
        let sat = select(0.0, (mx - mn) / mx, mx > 1e-6);
        let hue = rgb_to_hsv(c).x;
        let skin = clamp(1.0 - (abs(hue - 25.0) / 25.0), 0.0, 1.0) * 0.5;
        let k = 1.0 + P.presence.y + P.presence.x * (1.0 - sat) * (1.0 - skin);
        c = vec3(l) + (c - vec3(l)) * max(k, 0.0);
    }
    // 5. Tone curves.
    if on(PARAMETRIC) {
        c = vec3(lut_eval(0, c.x), lut_eval(0, c.y), lut_eval(0, c.z));
    }
    if on(MASTER) {
        c = vec3(lut_eval(1, c.x), lut_eval(1, c.y), lut_eval(1, c.z));
    }
    for (var i = 0; i < 3; i++) {
        if on(RED + u32(i)) {
            c[i] = lut_eval(2 + i, c[i]);
        }
    }
    // 6. Color Mixer.
    if on(HSL) {
        let hsv = rgb_to_hsv(max(c, vec3(0.0)));
        let s = hsv.y;
        if s > 1e-4 {
            let d = hsl_shift(hsv.x);
            let nh = hsv.x + d.x / 100.0 * 30.0;
            let ns = clamp(s * (1.0 + d.y / 100.0), 0.0, 1.0);
            var out = hsv_to_rgb(nh, ns, hsv.z);
            let l0 = luma(c);
            let l1 = luma(out);
            if l1 > 1e-6 {
                out = out * l0 / l1;
            }
            let k = exp2(d.z / 100.0 * 1.0 * s);
            c = out * k;
        }
    }
    // 7. Color Grading.
    if on(GRADE) {
        let l = clamp(luma(c), 0.0, 1.0);
        let blend = P.grade.x;
        let bal = P.grade.y;
        let sh = 1.0 - smoothstep_(0.0, 0.5 + bal + blend, l);
        let hi = smoothstep_(0.5 + bal - blend, 1.0, l);
        let mid = max(1.0 - sh - hi, 0.0);
        let masks = vec4(sh, mid, hi, 1.0);
        for (var k = 0; k < 4; k++) {
            c += masks[k] * P.tints[k].xyz;
            if P.tints[k].w != 0.0 {
                c = c * exp2(masks[k] * P.tints[k].w / 100.0 * 0.6);
            }
        }
    }
    // 9. Vignette.
    if on(VIGNETTE) {
        let amt = P.vignette[0].x;
        let hl = P.vignette[0].y;
        let style = u32(P.vignette[0].z);
        let pw = P.vignette[0].w;
        let centre = P.vignette[1].xy;
        let axes = P.vignette[1].zw;
        let dx = abs((f32(p.x) + 0.5 - centre.x) / axes.x);
        let dy = abs((f32(p.y) + 0.5 - centre.y) / axes.y);
        let r = pow(pow(dx, pw) + pow(dy, pw), 1.0 / pw);
        let m = smoothstep_(P.vignette[2].x, P.vignette[2].y, r);
        if m > 0.0 {
            let l = clamp(luma(c), 0.0, 1.0);
            if style == 2u {
                let goal = select(1.0, 0.0, amt < 0.0);
                let k = m * abs(amt);
                c += (vec3(goal) - c) * k;
            } else {
                var k = exp2(amt * 2.0 * m);
                if amt < 0.0 && hl > 0.0 {
                    let protect = smoothstep_(0.6, 1.0, l) * hl;
                    k += (1.0 - k) * protect;
                }
                if style == 1u {
                    let nl = l * k;
                    c += vec3(nl - l);
                } else {
                    c = c * k;
                }
            }
        }
    }
    // 9. Grain, in full-resolution coordinates.
    if on(GRAIN) {
        let cell = P.grain.x;
        let rough = P.grain.y;
        let fx = f32(p.x) * P.grain.w / cell;
        let fy = f32(p.y) * P.grain.w / cell;
        let n = value_noise(fx, fy, P.seeds.xy) * (1.0 - rough) + (value_noise(fx * 2.3, fy * 2.3, P.seeds.zw) - 0.0) * rough;
        let l = clamp(luma(c), 0.0, 1.0);
        let g = (n - 0.5) * 2.0 * P.grain.z * (0.4 + 2.4 * l * (1.0 - l));
        c += vec3(g);
    }
    if on(CLAMP) {
        c = clamp(c, vec3(0.0), vec3(1.0));
    }
    return vec4(c, q.a);
}
