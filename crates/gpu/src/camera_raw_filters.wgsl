// Dehaze, Clarity and Texture for Camera Raw's GPU preview: develop's `box_mean`, `guided`,
// `guided_fast` and `dehaze` (constants from `photocraft_algo::camera_raw::spatial`) as fragment
// passes. Appended to `camera_raw_tone.wgsl`, whose bindings and helpers it uses;
// `camera_raw_tone.rs` orders the passes and `camera_raw.wgsl` applies their results.

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

// `box_mean` along one axis (misc.x: 0 rows, 1 columns) at radius misc.y, edges clamped. Mode
// misc.z: 0 the means; 1 then the guided filter's coefficients (a, b) from the means of
// (I, p, I·p, I·I), regularised by misc.w's bits; 2 then the guided filter's output a·I + b, I
// the red channel of `plane_b`.
@fragment
fn fs_box(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let n = src_size();
    let r = i32(S.misc.y);
    var acc = vec4(0.0);
    if S.misc.x == 0u {
        for (var k = -r; k <= r; k++) {
            acc += textureLoad(plane_a, vec2(clamp(p.x + k, 0, n.x - 1), p.y), 0);
        }
    } else {
        for (var k = -r; k <= r; k++) {
            acc += textureLoad(plane_a, vec2(p.x, clamp(p.y + k, 0, n.y - 1)), 0);
        }
    }
    let m = acc / f32(2 * r + 1);
    if S.misc.z == 1u {
        let a = (m.z - m.x * m.y) / (m.w - m.x * m.x + bitcast<f32>(S.misc.w));
        return vec4(a, m.y - a * m.x, 0.0, 1.0);
    }
    if S.misc.z == 2u {
        return vec4(m.x * textureLoad(plane_b, p, 0).r + m.y, 0.0, 0.0, 1.0);
    }
    return m;
}

// `guided_fast`'s subsampled copy of `plane_a`'s red channel (`edge × edge` block means), as the
// guided filter's (I, p, I·p, I·I) with I = p.
@fragment
fn fs_block(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let t = pixel(pos);
    let f = i32(S.sizes2.w);
    let end = min((t + vec2(1)) * f, src_size());
    var sum = 0.0;
    for (var y = t.y * f; y < end.y; y++) {
        for (var x = t.x * f; x < end.x; x++) {
            sum += textureLoad(plane_a, vec2(x, y), 0).r;
        }
    }
    let m = sum / f32((end.x - t.x * f) * (end.y - t.y * f));
    return vec4(m, m, m * m, m * m);
}

// Dehaze's minimum filter along one axis (misc.x) at radius misc.y, the window cut at the edges.
// Mode misc.z: 0 the red channel; 1 the darkest channel of `plane_a`; 2 the darkest channel over
// the atmospheric light (`plane_b`'s one texel); 3 the red channel, then the guided filter's
// (I, p, I·p, I·I) for the transmission: I the luminance of the cells (`plane_b`), p = 1 − w·min
// with w the red of `atlas_fine`'s one texel; 4 the maximum of the red channel (the opening).
@fragment
fn fs_min(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let n = src_size();
    let r = i32(S.misc.y);
    let mode = S.misc.z;
    let air = textureLoad(plane_b, vec2(0, 0), 0).rgb;
    var axis = vec2(1, 0);
    var at = p.x;
    var len = n.x;
    if S.misc.x == 1u {
        axis = vec2(0, 1);
        at = p.y;
        len = n.y;
    }
    // The window is never empty, so these never show.
    var m = 3.0e38;
    if mode == 4u {
        m = -3.0e38;
    }
    for (var k = max(at - r, 0); k <= min(at + r, len - 1); k++) {
        let v = textureLoad(plane_a, p + axis * (k - at), 0);
        var d = v.r;
        if mode == 1u {
            d = min(min(v.r, v.g), v.b);
        } else if mode == 2u {
            d = min(min(v.r / air.r, v.g / air.g), v.b / air.b);
        }
        if mode == 4u {
            m = max(m, d);
        } else {
            m = min(m, d);
        }
    }
    if mode == 3u {
        let i = luma(textureLoad(plane_b, p, 0).rgb);
        let t = 1.0 - textureLoad(atlas_fine, vec2(0, 0), 0).r * m;
        return vec4(i, t, i * t, i * i);
    }
    return vec4(m, 0.0, 0.0, 1.0);
}

// The cells in the order `dehaze` sorts them, for selecting the atmospheric light's: -1 where the
// dark channel (`plane_a`) is above the selected value (slot misc.y), the cell's index where it
// is that value (ties go in index order), and above every index elsewhere.
@fragment
fn fs_haze_rank(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pixel(pos);
    let k = key(textureLoad(plane_a, p, 0).r);
    let v = stat(S.misc.y, 0).x;
    var out = 3.0e38;
    if k > v {
        out = -1.0;
    } else if k == v {
        out = f32(p.y * src_size().x + p.x);
    }
    return vec4(out, 0.0, 0.0, 1.0);
}

// The atmospheric light. Mode misc.x 0: per `edge × edge` tile, the cells (`plane_b`) whose order
// (`plane_a`) is at or below the selected one (slot misc.y), each divided by their count misc.w,
// summed. Mode 1: every tile (`plane_a`) summed, each channel at least misc.w's bits.
// The transmission's weight. Mode 2: per tile, the dark channel (`plane_a`) and its opening
// (`plane_b`) summed into red and green. Mode 3: every tile (`plane_a`) summed, and the weight
// ω·mean(dark) / mean(opened), ω misc.w's bits, the cells' count misc.y.
@fragment
fn fs_air(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let n = src_size();
    var sum = vec3(0.0);
    if S.misc.x == 1u || S.misc.x == 3u {
        for (var y = 0; y < n.y; y++) {
            for (var x = 0; x < n.x; x++) {
                sum += textureLoad(plane_a, vec2(x, y), 0).rgb;
            }
        }
        if S.misc.x == 3u {
            let count = f32(S.misc.y);
            return vec4(bitcast<f32>(S.misc.w) * (sum.r / count) / max(sum.g / count, 1e-6), 0.0, 0.0, 1.0);
        }
        return vec4(max(sum, vec3(bitcast<f32>(S.misc.w))), 1.0);
    }
    let t = pixel(pos);
    let f = i32(S.sizes2.w);
    let end = min((t + vec2(1)) * f, n);
    if S.misc.x == 2u {
        for (var y = t.y * f; y < end.y; y++) {
            for (var x = t.x * f; x < end.x; x++) {
                sum += vec3(textureLoad(plane_a, vec2(x, y), 0).r, textureLoad(plane_b, vec2(x, y), 0).r, 0.0);
            }
        }
        return vec4(sum, 1.0);
    }
    let cut = stat(S.misc.y, 0).x;
    let top = f32(S.misc.w);
    for (var y = t.y * f; y < end.y; y++) {
        for (var x = t.x * f; x < end.x; x++) {
            if key(textureLoad(plane_a, vec2(x, y), 0).r) <= cut {
                sum += textureLoad(plane_b, vec2(x, y), 0).rgb / top;
            }
        }
    }
    return vec4(sum, 1.0);
}
