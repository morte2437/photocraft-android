//! Tagged-block helpers: the preserved block-list encoding, fills, text,
//! smart objects, locks and label colors.

use photocraft_color::{Color, ColorMode};
use photocraft_doc::{AdvancedBlending, BlendIf, BlendRange, Fill, GradientStyle, Knockout, LabelColor, Locks};
use photocraft_geom::Affine;
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};
use photocraft_psd::layer::BlendingRanges;

/// `brst` (channel blending restrictions): a list of big-endian u32 channel indices left out of
/// blending → a bit mask (bit `i` = channel `i`; indices above 31 are ignored).
pub fn parse_brst(data: &[u8]) -> u32 {
    data.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).filter(|&i| i < 32).fold(0, |m, i| m | 1 << i)
}

/// Inverse of [`parse_brst`]; `None` when every channel blends (no block).
pub fn brst_data(mask: u32) -> Option<Vec<u8>> {
    (mask != 0).then(|| (0..32u32).filter(|i| mask & 1 << i != 0).flat_map(u32::to_be_bytes).collect())
}

/// The Advanced Blending blocks, as (key, value for `a`, Photoshop's default value): `knko`
/// (0 none, 1 shallow, 2 deep), `infx`, `clbl`, `tsly`, `lmgm`, `vmgm`. Each is stored as one
/// byte followed by three bytes of padding (Adobe PSD spec, "Additional Layer Information").
pub fn advanced_flags(a: &AdvancedBlending) -> [([u8; 4], u8, u8); 6] {
    [
        (*b"knko", a.knockout.to_psd(), 0),
        (*b"infx", u8::from(a.blend_interior), 0),
        (*b"clbl", u8::from(a.blend_clipped), 1),
        (*b"tsly", u8::from(a.transparency_shapes), 1),
        (*b"lmgm", u8::from(a.layer_mask_hides_effects), 0),
        (*b"vmgm", u8::from(a.vector_mask_hides_effects), 0),
    ]
}

/// Advanced Blending from a layer record's blocks (`block(key)` gives a block's data). A missing
/// or empty block keeps Photoshop's default.
pub fn advanced_from_blocks<'a>(block: impl Fn(&[u8; 4]) -> Option<&'a [u8]>) -> AdvancedBlending {
    let mut a = AdvancedBlending::default();
    let flag = |k: &[u8; 4]| block(k).and_then(|d| d.first().copied());
    if let Some(v) = flag(b"knko") {
        a.knockout = Knockout::from_psd(v);
    }
    let set = |k: &[u8; 4], f: &mut bool| {
        if let Some(v) = flag(k) {
            *f = v != 0;
        }
    };
    set(b"infx", &mut a.blend_interior);
    set(b"clbl", &mut a.blend_clipped);
    set(b"tsly", &mut a.transparency_shapes);
    set(b"lmgm", &mut a.layer_mask_hides_effects);
    set(b"vmgm", &mut a.vector_mask_hides_effects);
    a
}

/// Writes the Advanced Blending blocks into an exported layer's preserved block list: a block
/// already there is rewritten in place (keeping Photoshop's block order); a missing one is added
/// only when its value differs from the default, so plain layers gain no blocks.
pub fn put_advanced(a: &AdvancedBlending, raw: &mut Vec<([u8; 4], Vec<u8>)>) {
    for (key, v, default) in advanced_flags(a) {
        match raw.iter_mut().find(|(k, _)| *k == key) {
            Some(e) => {
                // Keep any extra bytes a reader wrote; the flag is the first byte.
                if e.1.len() < 4 {
                    e.1.resize(4, 0);
                }
                e.1[0] = v;
            }
            None if v != default => raw.push((key, vec![v, 0, 0, 0])),
            None => {}
        }
    }
}

/// Layer-record blending ranges (Adobe PSD spec, "Layer blending ranges data": the composite
/// gray source and destination ranges, then a source and destination range per channel, each
/// as black low, black high, white low, white high) → Blend If. Full ranges map to the
/// default, so files without Blend If keep an empty setting.
pub fn blend_if_from_ranges(r: &BlendingRanges) -> BlendIf {
    let mut ranges: Vec<[BlendRange; 2]> = r.ranges().iter().map(|e| [BlendRange::from_bytes(e.source), BlendRange::from_bytes(e.dest)]).collect();
    // Trailing full entries carry nothing (export pads them back), so they are dropped, as
    // `BlendIf::set` does.
    let keep = ranges.iter().rposition(|p| !p.iter().all(BlendRange::is_full)).map_or(0, |i| i + 1);
    ranges.truncate(keep);
    BlendIf { ranges }
}

/// Inverse of [`blend_if_from_ranges`] for a document with `channels` colour channels: one
/// entry for gray plus one per channel (more when the setting stores more), unset entries full.
pub fn ranges_from_blend_if(b: &BlendIf, channels: usize) -> BlendingRanges {
    if b.is_default() {
        return BlendingRanges::full(channels);
    }
    let n = b.ranges.len().max(channels + 1);
    let data = (0..n).flat_map(|i| {
        let [src, dst] = b.get(i);
        src.to_bytes().into_iter().chain(dst.to_bytes())
    });
    BlendingRanges { data: data.collect() }
}

/// `lspf` bits (Adobe spec: bit 0 transparency, 1 composite, 2 position).
/// Artboard (bit 3, `0x08`, as reference files write it) and "all" (bit 31) are undocumented.
/// Older PhotoCraft builds wrote the artboard lock at bit 4 (`0x10`); that value is still
/// accepted on read so those files keep their lock, and is rewritten at `0x08`.
pub fn locks_from_lspf(v: u32) -> Locks {
    Locks { transparency: v & 1 != 0, pixels: v & 2 != 0, position: v & 4 != 0, artboard: v & 0x08 != 0 || v & 0x10 != 0, all: v & 0x8000_0000 != 0 }
}

/// Inverse of [`locks_from_lspf`].
pub fn lspf_from_locks(l: &Locks) -> u32 {
    u32::from(l.transparency) | u32::from(l.pixels) << 1 | u32::from(l.position) << 2 | u32::from(l.artboard) << 3 | u32::from(l.all) << 31
}

/// `lclr` index → label.
pub fn label_from_index(v: u16) -> LabelColor {
    match v {
        1 => LabelColor::Red,
        2 => LabelColor::Orange,
        3 => LabelColor::Yellow,
        4 => LabelColor::Green,
        5 => LabelColor::Blue,
        6 => LabelColor::Violet,
        7 => LabelColor::Gray,
        // Photoshop 2024 added these without renumbering the original labels.
        8 => LabelColor::Seafoam,
        9 => LabelColor::Indigo,
        10 => LabelColor::Magenta,
        11 => LabelColor::Fuchsia,
        _ => LabelColor::None,
    }
}

/// Label → `lclr` index.
pub fn label_index(l: LabelColor) -> u16 {
    match l {
        LabelColor::None => 0,
        LabelColor::Red => 1,
        LabelColor::Orange => 2,
        LabelColor::Yellow => 3,
        LabelColor::Green => 4,
        LabelColor::Blue => 5,
        LabelColor::Violet => 6,
        LabelColor::Gray => 7,
        LabelColor::Seafoam => 8,
        LabelColor::Indigo => 9,
        LabelColor::Magenta => 10,
        LabelColor::Fuchsia => 11,
    }
}

pub(crate) fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Double(d) => Some(*d),
        Value::UnitFloat { value, .. } => Some(*value),
        Value::Integer(i) => Some(f64::from(*i)),
        _ => None,
    }
}

/// Descriptor color (`RGBC`, `CMYC`, `Grsc`, `HSBC`) → [`Color`].
pub fn color_from_desc(d: &Descriptor) -> Option<Color> {
    let id = d.class_id.as_bytes();
    let g = |k: &str| num(d.get(k)).map(|v| v as f32);
    Some(match id {
        // Newer Photoshop writes 0..1 floats (`RGBC` with redFloat / greenFloat / blueFloat).
        b"RGBC" if d.get("redFloat").is_some() => Color::rgb(g("redFloat")?, g("greenFloat")?, g("blueFloat")?),
        b"RGBC" => Color::rgb(g("Rd  ")? / 255.0, g("Grn ")? / 255.0, g("Bl  ")? / 255.0),
        b"CMYC" => Color { mode: ColorMode::Cmyk, c: [g("Cyn ")? / 100.0, g("Mgnt")? / 100.0, g("Ylw ")? / 100.0, g("Blck")? / 100.0], alpha: 1.0 },
        b"Grsc" => Color::gray(1.0 - g("Gry ")? / 100.0),
        // CIE L*a*b*: L 0..100, a / b −128..127 (stored like Lab pixels: offset by 128).
        b"LbCl" => Color { mode: ColorMode::Lab, c: [g("Lmnc")? / 100.0, (g("A   ")? + 128.0) / 255.0, (g("B   ")? + 128.0) / 255.0, 0.0], alpha: 1.0 },
        // HSB: hue an angle in degrees, saturation / brightness 0..100.
        b"HSBC" => {
            let [r, gr, b] = crate::channel_map::hsb_to_rgb(g("H   ")?, g("Strt")? / 100.0, g("Brgh")? / 100.0);
            Color::rgb(r, gr, b)
        }
        // Newer Photoshop writes 0..1 floats.
        _ if d.get("redFloat").is_some() => Color::rgb(g("redFloat")?, g("greenFloat")?, g("blueFloat")?),
        _ => {
            // Fall back to any RGB-like triple.
            Color::rgb(g("Rd  ")? / 255.0, g("Grn ")? / 255.0, g("Bl  ")? / 255.0)
        }
    })
    .map(|c| with_pc_alpha(c, d))
}

/// PSD colour descriptors have no alpha. PhotoCraft writes `pcAl` (0..1) so shape fill
/// opacity round-trips (#6180). Photoshop ignores the key.
fn with_pc_alpha(mut c: Color, d: &Descriptor) -> Color {
    // A hostile file's NaN or infinity would survive the clamp: keep the colour opaque instead.
    if let Some(a) = num(d.get("pcAl")).filter(|a| a.is_finite()) {
        c.alpha = (a as f32).clamp(0.0, 1.0);
    }
    c
}

/// [`Color`] → descriptor in its own model.
pub fn color_to_desc(c: &Color) -> Descriptor {
    let d = |v: f32| Value::Double(f64::from(v));
    match c.mode {
        ColorMode::Cmyk => Descriptor::new("CMYC")
            .with("Cyn ", d(c.c[0] * 100.0))
            .with("Mgnt", d(c.c[1] * 100.0))
            .with("Ylw ", d(c.c[2] * 100.0))
            .with("Blck", d(c.c[3] * 100.0)),
        ColorMode::Grayscale => Descriptor::new("Grsc").with("Gry ", d((1.0 - c.c[0]) * 100.0)),
        ColorMode::Lab => {
            Descriptor::new("LbCl").with("Lmnc", d(c.c[0] * 100.0)).with("A   ", d(c.c[1] * 255.0 - 128.0)).with("B   ", d(c.c[2] * 255.0 - 128.0))
        }
        _ => {
            let rgb = c.to_rgb();
            Descriptor::new("RGBC").with("Rd  ", d(rgb[0] * 255.0)).with("Grn ", d(rgb[1] * 255.0)).with("Bl  ", d(rgb[2] * 255.0))
        }
    }
}

/// Solid shape fills have no other alpha slot. Write `pcAl` only here so gradient stops
/// keep using `Trns` alone (#6180).
fn color_to_desc_with_pc_alpha(c: &Color) -> Descriptor {
    let desc = color_to_desc(c);
    if c.alpha.is_finite() && (c.alpha - 1.0).abs() > 1e-6 { desc.with("pcAl", Value::Double(f64::from(c.alpha.clamp(0.0, 1.0)))) } else { desc }
}

pub(crate) fn get_desc<'a>(d: &'a Descriptor, key: &str) -> Option<&'a Descriptor> {
    match d.get(key)? {
        Value::Descriptor(x) | Value::GlobalObject(x) => Some(x),
        _ => None,
    }
}

/// Enumerated value of `key` (4-char or long id), if present.
pub(crate) fn enum_of<'a>(d: &'a Descriptor, key: &str) -> Option<&'a [u8]> {
    match d.get(key)? {
        Value::Enumerated { value, .. } => Some(value.as_bytes()),
        _ => None,
    }
}

/// A gradient's interpolation method code. Photoshop (27.11, checked by saving gradient fill
/// layers) stores it as the enum `gradientsInterpolationMethod` (`Perc`, `Lnr `, `Smoo`, `Gcls`);
/// `gs99` is kept as a fallback.
pub(crate) fn gradient_method(d: &Descriptor) -> Option<&[u8]> {
    enum_of(d, "gradientsInterpolationMethod").or_else(|| enum_of(d, "gs99"))
}

pub(crate) fn bool_of(d: &Descriptor, key: &str) -> bool {
    matches!(d.get(key), Some(Value::Boolean(true)))
}

/// `GrdT` gradient type.
pub(crate) fn gradient_style(d: &Descriptor) -> GradientStyle {
    match enum_of(d, "Type") {
        Some(b"Rdl ") => GradientStyle::Radial,
        Some(b"Angl") => GradientStyle::Angle,
        Some(b"Rflc") => GradientStyle::Reflected,
        Some(b"Dmnd") => GradientStyle::Diamond,
        _ => GradientStyle::Linear,
    }
}

pub(crate) fn gradient_style_value(s: GradientStyle) -> Value {
    let v = match s {
        GradientStyle::Linear => "Lnr ",
        GradientStyle::Radial => "Rdl ",
        GradientStyle::Angle => "Angl",
        GradientStyle::Reflected => "Rflc",
        GradientStyle::Diamond => "Dmnd",
    };
    Value::Enumerated { type_id: Id::new("GrdT"), value: Id::new(v) }
}

/// Colour and opacity stops of a `Grdn` descriptor (locations 0..4096 → 0..1).
/// Colour stops and opacity stops of a gradient.
pub(crate) type Stops = (Vec<(f32, Color)>, Vec<(f32, f32)>);

/// Stops of a `Grdn` descriptor, baked to Photoshop's interpolation (smoothness `Intr`, stop
/// midpoints, and the parent's interpolation method when given, see [`gradient_method`]); the
/// opacity stops as [`baked_opacity_stops`].
pub(crate) fn gradient_stops_with(grad: &Descriptor, method: Option<&[u8]>) -> Stops {
    let raw = gradient_stops_raw(grad);
    let stops = if raw.stops.len() >= 2 {
        crate::gradient_bake::bake(raw.stops, &raw.mids, raw.smooth, crate::gradient_bake::Method::from_code(method))
    } else {
        raw.stops
    };
    (stops, baked_opacity_stops(grad, method))
}

/// A `Grdn` descriptor's stops as stored: colour stops (sorted), the midpoint of each segment
/// between them, opacity stops and the smoothness (`Intr`, `0..=1`).
pub(crate) struct RawStops {
    pub stops: Vec<(f32, Color)>,
    pub mids: Vec<f32>,
    pub opacity: Vec<(f32, f32)>,
    pub smooth: f32,
}

/// Stops of a gradient fill layer, kept editable: when Photoshop's interpolation is plain
/// (no smoothness, Classic method) the stops stay as they are with their midpoints, else they
/// are baked like [`gradient_stops_with`] (midpoints folded in). Opacity stops that are all
/// 100 % come back empty.
pub(crate) fn gradient_stops_editable(grad: &Descriptor, method: Option<&[u8]>) -> RawStops {
    let mut raw = gradient_stops_raw(grad);
    let m = crate::gradient_bake::Method::from_code(method);
    // Lab stops (Lab documents) interpolate in L*a*b*, which only baking reproduces.
    let lab = raw.stops.iter().all(|(_, c)| c.mode == photocraft_color::ColorMode::Lab);
    if raw.stops.len() >= 2 && (raw.smooth > 0.0 || m != crate::gradient_bake::Method::Classic || lab) {
        raw.stops = crate::gradient_bake::bake(std::mem::take(&mut raw.stops), &raw.mids, raw.smooth, m);
        raw.mids.clear();
    }
    if raw.mids.iter().all(|m| (m - 0.5).abs() < 1e-3) {
        raw.mids.clear();
    }
    if raw.opacity.iter().all(|o| (o.1 - 1.0).abs() < 1e-4) {
        raw.opacity.clear();
    }
    raw
}

/// The exact, constant-RGB subset of Photoshop noise gradients. If every RGB component has the
/// same minimum and maximum, the generated field is constant regardless of seed or roughness.
/// Other models, transparency, and varying ranges are not approximated.
fn constant_rgb_noise_color(grad: &Descriptor) -> Option<Color> {
    if enum_of(grad, "GrdF")? != b"ClNs" || enum_of(grad, "ClrS")? != b"RGBC" || bool_of(grad, "ShTr") {
        return None;
    }
    let components = |key: &str| -> Option<[f32; 4]> {
        let Some(Value::List(values)) = grad.get(key) else { return None };
        if values.len() != 4 {
            return None;
        }
        let converted: Vec<f32> = values.iter().map(|value| num(Some(value)).map(|v| v as f32)).collect::<Option<_>>()?;
        converted.try_into().ok()
    };
    let min = components("Mnm ")?;
    let max = components("Mxm ")?;
    if min.iter().zip(max).take(3).any(|(a, b)| !a.is_finite() || *a < 0.0 || *a > 100.0 || *a != b) {
        return None;
    }
    Some(Color::rgb(min[0] / 100.0, min[1] / 100.0, min[2] / 100.0))
}

fn gradient_stops_raw(grad: &Descriptor) -> RawStops {
    let mut stops = Vec::new();
    let mut mids = Vec::new();
    let mut opacity = Vec::new();
    if let Some(Value::List(items)) = grad.get("Clrs") {
        for it in items {
            if let Value::Descriptor(s) = it {
                let loc = num(s.get("Lctn")).unwrap_or(0.0) as f32 / 4096.0;
                if let Some(c) = get_desc(s, "Clr ").and_then(color_from_desc) {
                    stops.push((loc, c));
                    mids.push(num(s.get("Mdpn")).unwrap_or(50.0) as f32 / 100.0);
                }
            }
        }
    }
    // Midpoint k applies to the segment after stop k.
    let mut order: Vec<usize> = (0..stops.len()).collect();
    order.sort_by(|a, b| stops[*a].0.total_cmp(&stops[*b].0));
    let mids: Vec<f32> = order.iter().skip(1).map(|i| mids.get(*i).copied().unwrap_or(0.5)).collect();
    let smooth = num(grad.get("Intr")).map_or(0.0, |v| (v / 4096.0) as f32);
    let mut sorted: Vec<(f32, Color)> = order.iter().filter_map(|i| stops.get(*i).cloned()).collect();
    if sorted.len() != stops.len() {
        sorted = stops;
    }
    if let Some(Value::List(items)) = grad.get("Trns") {
        for it in items {
            if let Value::Descriptor(s) = it {
                let loc = num(s.get("Lctn")).unwrap_or(0.0) as f32 / 4096.0;
                opacity.push((loc, num(s.get("Opct")).unwrap_or(100.0) as f32 / 100.0));
            }
        }
    }
    RawStops { stops: sorted, mids, opacity, smooth }
}

/// A `Grdn` descriptor for colour and opacity stops.
pub(crate) fn gradient_desc(stops: &[(f32, Color)], opacity: &[(f32, f32)]) -> Descriptor {
    gradient_desc_with(stops, opacity, &[])
}

/// [`gradient_desc`] with colour midpoints (one per segment between the sorted stops; each is
/// stored on the stop that ends its segment, as [`gradient_stops_with`] reads it).
pub(crate) fn gradient_desc_with(stops: &[(f32, Color)], opacity: &[(f32, f32)], mids: &[f32]) -> Descriptor {
    let mut stops = stops.to_vec();
    if !mids.is_empty() {
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let clrs = stops
        .iter()
        .enumerate()
        .map(|(i, (t, c))| {
            let mid = i.checked_sub(1).and_then(|k| mids.get(k)).copied().unwrap_or(0.5);
            Value::Descriptor(
                Descriptor::new("Clrt")
                    .with("Clr ", Value::Descriptor(color_to_desc(c)))
                    .with("Type", Value::Enumerated { type_id: Id::new("Clry"), value: Id::new("UsrS") })
                    .with("Lctn", Value::Integer((t * 4096.0).round() as i32))
                    .with("Mdpn", Value::Integer((mid.clamp(0.05, 0.95) * 100.0).round() as i32)),
            )
        })
        .collect();
    let op: Vec<(f32, f32)> = if opacity.is_empty() { stops.iter().map(|(t, c)| (*t, c.alpha)).collect() } else { opacity.to_vec() };
    let trns = op
        .iter()
        .map(|(t, a)| {
            Value::Descriptor(
                Descriptor::new("TrnS")
                    .with("Opct", Value::UnitFloat { unit: *b"#Prc", value: f64::from(a * 100.0) })
                    .with("Lctn", Value::Integer((t * 4096.0).round() as i32))
                    .with("Mdpn", Value::Integer(50)),
            )
        })
        .collect();
    Descriptor::new("Grdn")
        .with("Nm  ", Value::Text(UnicodeString::new_nul("Custom")))
        .with("GrdF", Value::Enumerated { type_id: Id::new("GrdF"), value: Id::new("CstS") })
        // Our stops interpolate linearly (Photoshop smoothness is baked into them on import).
        .with("Intr", Value::Double(0.0))
        .with("Clrs", Value::List(clrs))
        .with("Trns", Value::List(trns))
}

/// Opacity stops of a `Grdn` descriptor with Photoshop's smoothness, opacity midpoints and
/// interpolation method applied (densely sampled; as stored when the interpolation is plain).
/// Gradient fill layers and the gradient layer effects share it: Photoshop renders their
/// transparency identically.
fn baked_opacity_stops(grad: &Descriptor, method: Option<&[u8]>) -> Vec<(f32, f32)> {
    let mut stops = Vec::new();
    let mut mids = Vec::new();
    if let Some(Value::List(items)) = grad.get("Trns") {
        for it in items {
            if let Value::Descriptor(s) = it {
                stops.push((num(s.get("Lctn")).unwrap_or(0.0) as f32 / 4096.0, num(s.get("Opct")).unwrap_or(100.0) as f32 / 100.0));
                mids.push(num(s.get("Mdpn")).unwrap_or(50.0) as f32 / 100.0);
            }
        }
    }
    // Midpoint k applies to the segment after stop k (stored on the stop that ends it).
    let mut order: Vec<usize> = (0..stops.len()).collect();
    order.sort_by(|a, b| stops[*a].0.total_cmp(&stops[*b].0));
    let mids: Vec<f32> = order.iter().skip(1).filter_map(|i| mids.get(*i).copied()).collect();
    let smooth = num(grad.get("Intr")).map_or(0.0, |v| (v / 4096.0) as f32);
    crate::gradient_bake::bake_opacity(stops, &mids, smooth, crate::gradient_bake::Method::from_code(method))
}

/// Parses a fill block (`SoCo`, `GdFl`, `PtFl`).
pub fn parse_fill(key: &[u8; 4], data: &[u8]) -> Option<Fill> {
    // Block data may carry trailing padding after the descriptor.
    let d = parse_prefix_versioned(data)?;
    fill_from_desc(key, &d)
}

/// A fill from its descriptor (`SoCo`/`GdFl`/`PtFl` layout; also the `solidColorLayer`,
/// `gradientLayer` and `patternLayer` contents of shape strokes).
pub fn fill_from_desc(key: &[u8; 4], d: &Descriptor) -> Option<Fill> {
    match key {
        b"SoCo" => Some(Fill::Solid(color_from_desc(get_desc(d, "Clr ")?)?)),
        b"GdFl" => {
            let angle = num(d.get("Angl")).unwrap_or(90.0) as f32;
            let scale = num(d.get("Scl ")).map_or(1.0, |v| v as f32 / 100.0);
            let gradient = get_desc(d, "Grad");
            let raw = gradient.map(|g| gradient_stops_editable(g, gradient_method(d)));
            let (mut stops, midpoints) = raw.map(|r| (r.stops, r.mids)).unwrap_or_default();
            // Opacity stops with Photoshop's smoothness, opacity midpoints and method applied
            // (raw when the interpolation is plain); none when fully opaque.
            let mut opacity_stops = get_desc(d, "Grad").map(|g| baked_opacity_stops(g, gradient_method(d))).unwrap_or_default();
            if opacity_stops.iter().all(|o| o.1 >= 1.0 - 1e-4) {
                opacity_stops.clear();
            }
            if stops.is_empty() {
                // A provably constant-RGB noise gradient renders one exact colour. Any other
                // noise gradient (variable ranges, transparency, other models) has no stops to
                // read: keep the placeholder ramp rather than failing the whole fill — the
                // layer stays a fill layer, Photoshop's own pixels ride along as `fill_cache`,
                // and the raw `GdFl` block survives the round trip byte for byte. Failing
                // here would import the layer as pixels and drop `GdFl` on save (#6272).
                stops = gradient
                    .filter(|g| enum_of(g, "GrdF") == Some(b"ClNs"))
                    .and_then(constant_rgb_noise_color)
                    .map(|color| vec![(0.0, color), (1.0, color)])
                    .unwrap_or_else(|| vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
            }
            let pct = |o: &Descriptor, k: &str| num(o.get(k)).map_or(0.0, |v| v as f32 / 100.0);
            let offset = get_desc(d, "Ofst").map_or((0.0, 0.0), |o| (pct(o, "Hrzn"), pct(o, "Vrtc")));
            Some(Fill::Gradient {
                stops,
                angle,
                scale,
                style: gradient_style(d),
                reverse: bool_of(d, "Rvrs"),
                opacity_stops,
                midpoints,
                offset,
                dither: bool_of(d, "Dthr"),
                align: !matches!(d.get("Algn"), Some(Value::Boolean(false))),
            })
        }
        b"PtFl" => {
            let (name, id) = pattern_ref(d);
            let scale = num(d.get("Scl ")).map_or(1.0, |v| v as f32 / 100.0);
            let (angle, link, phase) = pattern_placement(d);
            Some(Fill::Pattern { name, scale, id, angle, link, phase })
        }
        _ => None,
    }
}

/// Serializes a fill to its block.
pub fn write_fill(f: &Fill) -> ([u8; 4], Vec<u8>) {
    let (k, d) = fill_to_desc(f);
    (k, VersionedDescriptor::new(d).to_bytes())
}

/// A fill as (block key, descriptor with class `null`).
pub fn fill_to_desc(f: &Fill) -> ([u8; 4], Descriptor) {
    match f {
        Fill::Solid(c) => (*b"SoCo", Descriptor::new("null").with("Clr ", Value::Descriptor(color_to_desc_with_pc_alpha(c)))),
        Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither, align } => {
            let prc = |v: f32| Value::UnitFloat { unit: *b"#Prc", value: f64::from(v * 100.0) };
            let mut d = Descriptor::new("null")
                .with("Angl", Value::UnitFloat { unit: *b"#Ang", value: f64::from(*angle) })
                .with("Type", gradient_style_value(*style))
                .with("Scl ", prc(*scale))
                .with("Grad", Value::Descriptor(gradient_desc_with(stops, opacity_stops, midpoints)));
            if *reverse {
                d = d.with("Rvrs", Value::Boolean(true));
            }
            if *dither {
                d = d.with("Dthr", Value::Boolean(true));
            }
            if !*align {
                d = d.with("Algn", Value::Boolean(false));
            }
            if *offset != (0.0, 0.0) {
                d = d.with("Ofst", Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", prc(offset.0)).with("Vrtc", prc(offset.1))));
            }
            (*b"GdFl", d)
        }
        Fill::Pattern { name, scale, id, angle, link, phase } => {
            let d = Descriptor::new("null")
                .with("Scl ", Value::UnitFloat { unit: *b"#Prc", value: f64::from(scale * 100.0) })
                .with("Ptrn", Value::Descriptor(pattern_ref_desc(name, id)));
            (*b"PtFl", with_pattern_placement(d, *angle, *link, *phase))
        }
    }
}

/// Pattern name and id (`Ptrn` → `Nm  `, `Idnt`) of a pattern fill / overlay descriptor.
pub(crate) fn pattern_ref(d: &Descriptor) -> (String, String) {
    let p = get_desc(d, "Ptrn");
    let text = |k: &str| match p.and_then(|p| p.get(k)) {
        Some(Value::Text(t)) => t.to_string_lossy(),
        _ => String::new(),
    };
    (text("Nm  "), text("Idnt"))
}

/// The `Ptrn` reference descriptor (the id is omitted when unknown).
pub(crate) fn pattern_ref_desc(name: &str, id: &str) -> Descriptor {
    let p = Descriptor::new("Ptrn").with("Nm  ", Value::Text(UnicodeString::new_nul(name)));
    if id.is_empty() { p } else { p.with("Idnt", Value::Text(UnicodeString::new_nul(id))) }
}

/// Angle (`Angl`, degrees), link with layer (`Algn`, default on) and phase (`phase` point) of a
/// pattern fill / overlay descriptor.
pub(crate) fn pattern_placement(d: &Descriptor) -> (f32, bool, (f32, f32)) {
    let angle = num(d.get("Angl")).unwrap_or(0.0) as f32;
    let link = !matches!(d.get("Algn"), Some(Value::Boolean(false)));
    let phase = get_desc(d, "phase").map_or((0.0, 0.0), |p| (num(p.get("Hrzn")).unwrap_or(0.0) as f32, num(p.get("Vrtc")).unwrap_or(0.0) as f32));
    (angle, link, phase)
}

/// Adds the keys read by [`pattern_placement`] (angle only when non-zero).
pub(crate) fn with_pattern_placement(mut d: Descriptor, angle: f32, link: bool, phase: (f32, f32)) -> Descriptor {
    if angle != 0.0 {
        d = d.with("Angl", Value::UnitFloat { unit: *b"#Ang", value: f64::from(angle) });
    }
    d.with("Algn", Value::Boolean(link)).with(
        "phase",
        Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", Value::Double(f64::from(phase.0))).with("Vrtc", Value::Double(f64::from(phase.1)))),
    )
}

/// The warp of a placed layer (`SoLd`/`SoLE` `quiltWarp` or `warp` descriptor): style, bend,
/// distortions, axis, bounds and, for `warpCustom`, the `customEnvelopeWarp` mesh: one 4 × 4
/// patch, or the multi-patch grid Photoshop's split warp writes as `quiltWarp`; for
/// `warpCylinder` its seven `warpValues`. Read-only: the raw block is what gets written back.
/// `None` when absent or `warpNone` with no mesh. A `PlLd` record keeps its warp elsewhere: see
/// [`parse_plld_warp`].
pub fn parse_placed_warp(key: &[u8; 4], data: &[u8]) -> Option<photocraft_geom::warp::Warp> {
    if key != b"SoLd" && key != b"SoLE" {
        return None;
    }
    let d = data.get(8..).and_then(parse_prefix_versioned)?;
    // A split (quilt) warp is stored under `quiltWarp`; `warp` then holds no usable mesh.
    warp_of(get_desc(&d, "quiltWarp").or_else(|| get_desc(&d, "warp"))?).flatten()
}

/// A `warp` or `quiltWarp` descriptor (the `SoLd` key, or the whole descriptor after a `PlLd`
/// record's corners) as a warp: `Some(None)` for no warp (`warpNone`, or a warp that moves
/// nothing), `None` when it can't be read (an unknown style, missing bounds, a mesh that is
/// neither one 4 × 4 patch nor a consistent quilt grid, a cylinder that isn't a horizontal one
/// without perspective, the only kind measured).
pub(crate) fn warp_of(w: &Descriptor) -> Option<Option<photocraft_geom::warp::Warp>> {
    use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
    let style = enum_of(w, "warpStyle").and_then(|v| WarpStyle::parse(&String::from_utf8_lossy(v)))?;
    if style == WarpStyle::None {
        return Some(None);
    }
    let b = get_desc(w, "bounds")?;
    let g = |k: &str| num(b.get(k));
    let bounds = [g("Left")?, g("Top ")?, g("Rght")?, g("Btom")?];
    let mut warp = Warp::preset(style, num(w.get("warpValue")).unwrap_or(0.0), bounds);
    warp.h_distort = num(w.get("warpPerspective")).unwrap_or(0.0);
    warp.v_distort = num(w.get("warpPerspectiveOther")).unwrap_or(0.0);
    warp.vertical = enum_of(w, "warpRotate") == Some(b"Vrtc");
    if style == WarpStyle::Cylinder {
        if warp.vertical || warp.h_distort != 0.0 || warp.v_distort != 0.0 {
            return None;
        }
        let values: Vec<f64> = match w.get("warpValues")? {
            Value::List(l) => l.iter().map(|v| num(Some(v))).collect::<Option<_>>()?,
            Value::UnitFloats { values, .. } => values.clone(),
            _ => return None,
        };
        return Some(Some(Warp::cylinder(&values, bounds)?));
    }
    if style == WarpStyle::Custom {
        let env = get_desc(w, "customEnvelopeWarp")?;
        let Some(Value::ObjectArray(arr)) = env.get("meshPoints") else { return None };
        let vals = |k: &str| match arr.body.get(k) {
            Some(Value::UnitFloats { values, .. }) => Some(values.clone()),
            _ => None,
        };
        let (xs, ys) = (vals("Hrzn")?, vals("Vrtc")?);
        if xs.len() != ys.len() {
            return None;
        }
        let points: Vec<[f64; 2]> = xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect();
        let mesh = if points.len() == 16 && env.get("quiltSliceX").is_none() {
            BezierMesh { us: vec![0.0, 1.0], vs: vec![0.0, 1.0], points }
        } else {
            quilt_mesh(w, env, points)?
        };
        warp.mesh = mesh.is_valid().then_some(mesh);
        warp.mesh.as_ref()?;
    }
    Some((!warp.is_identity()).then_some(warp))
}

/// The most patches a `quiltWarp` is read with along each side.
pub(crate) const QUILT_MAX_PATCHES: usize = 64;

/// The control-point grid of a split ("quilt") custom warp. `deformNumCols` × `deformNumRows`
/// control points, three per patch side plus one, in row-major order; `quiltSliceX` and
/// `quiltSliceY` are the patch boundaries in the mesh's own units, first and last at its edges,
/// which become the knots on the unit square. `None` for anything inconsistent.
fn quilt_mesh(w: &Descriptor, env: &Descriptor, points: Vec<[f64; 2]>) -> Option<photocraft_geom::warp::BezierMesh> {
    // Photoshop's split grid tops out far below this; it only bounds what a file can claim.
    const MAX_SIDE: usize = 3 * QUILT_MAX_PATCHES + 1;
    let count = |k: &str| -> Option<usize> {
        let n = num(w.get(k))?;
        (n.is_finite() && n >= 4.0 && n <= MAX_SIDE as f64 && n.fract() == 0.0).then_some(n as usize)
    };
    let (cols, rows) = (count("deformNumCols")?, count("deformNumRows")?);
    if (cols - 1) % 3 != 0 || (rows - 1) % 3 != 0 || points.len() != cols.checked_mul(rows)? {
        return None;
    }
    let us = quilt_knots(env, "quiltSliceX", (cols - 1) / 3)?;
    let vs = quilt_knots(env, "quiltSliceY", (rows - 1) / 3)?;
    Some(photocraft_geom::warp::BezierMesh { us, vs, points })
}

/// The knots of a quilt's patch boundaries `key` (`quiltSliceX` or `quiltSliceY` in its
/// `customEnvelopeWarp`): `patches + 1` strictly increasing slices, mapped onto `0..=1` with the
/// first and last at the ends. `None` for anything else.
pub(crate) fn quilt_knots(env: &Descriptor, key: &str, patches: usize) -> Option<Vec<f64>> {
    // Stored as an object array holding one unit-float list under the same key; a plain list
    // of numbers is taken too.
    let s: Vec<f64> = match env.get(key)? {
        Value::ObjectArray(arr) => match arr.body.get(key)? {
            Value::UnitFloats { values, .. } => values.clone(),
            Value::List(l) => l.iter().map(|v| num(Some(v))).collect::<Option<_>>()?,
            _ => return None,
        },
        Value::List(l) => l.iter().map(|v| num(Some(v))).collect::<Option<_>>()?,
        _ => return None,
    };
    let (first, last) = (*s.first()?, *s.last()?);
    let span = last - first;
    if s.len() != patches + 1 || !(span.is_finite() && span > 0.0) || !s.windows(2).all(|p| p[1] > p[0]) {
        return None;
    }
    let n = s.len() - 1;
    Some(
        s.iter()
            .enumerate()
            .map(|(i, v)| {
                if i == 0 {
                    0.0
                } else if i == n {
                    1.0
                } else {
                    (v - first) / span
                }
            })
            .collect(),
    )
}

/// Text content and transform from a `TySh` block (best effort).
pub fn parse_tysh(data: &[u8]) -> Option<(String, Affine)> {
    let f = |at: usize| data.get(at..at + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
    let mut m = [0.0; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f(2 + i * 8)?;
    }
    // text version (2) at 50, descriptor version (4) at 52, descriptor at 56.
    let rest = data.get(52..)?;
    let text = parse_prefix_versioned(rest).and_then(|d| match d.get("Txt ") {
        Some(Value::Text(t)) => Some(t.to_string_lossy()),
        _ => None,
    });
    Some((text.unwrap_or_default(), Affine { m }))
}

/// Parses a version-16 descriptor at the start of `data` (trailing bytes allowed).
pub(crate) fn parse_prefix_versioned(data: &[u8]) -> Option<Descriptor> {
    VersionedDescriptor::parse_prefix(data).ok().map(|(v, _)| v.descriptor)
}

/// The source-space box Photoshop maps onto a placed layer's `Trnf` quad: the bounding box of a
/// custom warp's control points (one patch or a split grid) or of a cylinder's mesh (#4112),
/// which may spill past the source, else the source rect `0..w × 0..h`. Mapping the source rect
/// instead counts the overhang twice and stretches the placement. A degenerate or non-finite box
/// falls back to the source rect.
pub(crate) fn placed_frame(warp: Option<&photocraft_geom::warp::Warp>, w: f64, h: f64) -> [f64; 4] {
    use photocraft_geom::warp::WarpStyle;
    let rect = [0.0, 0.0, w, h];
    let Some(m) = warp.filter(|wp| matches!(wp.style, WarpStyle::Custom | WarpStyle::Cylinder)).and_then(|wp| wp.mesh.as_ref()).filter(|m| m.is_valid()) else {
        return rect;
    };
    let inf = f64::INFINITY;
    let b = m.points.iter().fold([inf, inf, -inf, -inf], |b, [x, y]| [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)]);
    let ok = b.iter().all(|v| v.is_finite()) && b[2] - b[0] > 1e-9 && b[3] - b[1] > 1e-9;
    if ok { b } else { rect }
}

/// The source-space box a projective (Distort / Perspective) placement maps onto its corners: a
/// cylinder's control-point box, as for an affine placement (#4112), else the source rect. A
/// custom warp keeps the source rect here: there is no Photoshop reference for that combination.
pub(crate) fn perspective_frame(warp: Option<&photocraft_geom::warp::Warp>, w: f64, h: f64) -> [f64; 4] {
    match warp {
        Some(wp) if wp.style == photocraft_geom::warp::WarpStyle::Cylinder => placed_frame(warp, w, h),
        _ => [0.0, 0.0, w, h],
    }
}

/// Smart object identifier and transform from `SoLd`/`SoLE` (best effort). A `PlLd` block
/// stores no source size, so its transform needs the embedded file: see [`parse_plld`].
pub fn parse_smart(key: &[u8; 4], data: &[u8]) -> (String, Affine) {
    // SoLd: 'soLD' + version(4) + versioned descriptor.
    let desc = if key == b"SoLd" || key == b"SoLE" { data.get(8..).and_then(parse_prefix_versioned) } else { None };
    let Some(d) = desc else { return (String::new(), Affine::IDENTITY) };
    let id = match d.get("Idnt") {
        Some(Value::Text(t)) => t.to_string_lossy(),
        _ => String::new(),
    };
    let mut affine = Affine::IDENTITY;
    if let (Some(Value::List(pts)), Some(sz)) = (d.get("Trnf"), get_desc(&d, "Sz  ")) {
        let p: Vec<f64> = pts.iter().filter_map(|v| num(Some(v))).collect();
        let (w, h) = (num(sz.get("Wdth")).unwrap_or(0.0), num(sz.get("Hght")).unwrap_or(0.0));
        if let (Ok(p), true) = (<[f64; 8]>::try_from(p), w > 0.0 && h > 0.0) {
            affine = corners_affine(&p, placed_frame(parse_placed_warp(key, data).as_ref(), w, h)).unwrap_or(Affine::IDENTITY);
        }
    }
    (id, affine)
}

/// The affine that takes the source-space box `frame` (left, top, right, bottom; see
/// [`placed_frame`]) onto placed corners `p` (top-left, top-right, bottom-right, bottom-left as
/// x, y pairs). `None` for an empty or non-finite frame, or non-finite corners.
pub fn corners_affine(p: &[f64; 8], frame: [f64; 4]) -> Option<Affine> {
    let [x0, y0, x1, y1] = frame;
    let (bw, bh) = (x1 - x0, y1 - y0);
    if !(frame.iter().chain(p).all(|v| v.is_finite()) && bw > 0.0 && bh > 0.0) {
        return None;
    }
    let (a, b, c, d) = ((p[2] - p[0]) / bw, (p[3] - p[1]) / bw, (p[6] - p[0]) / bh, (p[7] - p[1]) / bh);
    let m = [a, b, c, d, p[0] - a * x0 - c * y0, p[1] - b * x0 - d * y0];
    m.iter().all(|v| v.is_finite()).then_some(Affine { m })
}

/// The transform and (Distort/Perspective) projective placement of a `PlLd` record's corners `p`
/// for a source of `size` warped by `warp`: the inverse of [`crate::smart_map::plld_bytes`]'s
/// quad. As for `SoLd`'s `Trnf` (#3677), affine corners frame a custom warp's or a cylinder's
/// control-point box (which may spill past the source), else the source rect; projective corners
/// frame [`perspective_frame`], as `parse_smart_perspective` reads them. `PlLd` stores no source
/// size: without the source file (`None`) the layer is taken as placed at 100 %, the edge lengths
/// being its size (a custom warp's box needs no size); position and rotation stay right, only
/// Replace Contents and re-renders depend on the scale. Import and export (which keeps an unedited
/// record verbatim) both place a `PlLd` layer with this.
pub fn plld_transform(p: &[f64; 8], size: Option<(f64, f64)>, warp: Option<&photocraft_geom::warp::Warp>) -> (Affine, Option<[f64; 9]>) {
    let (w, h) = size.unwrap_or_else(|| ((p[2] - p[0]).hypot(p[3] - p[1]), (p[6] - p[0]).hypot(p[7] - p[1])));
    let perspective = corners_perspective_in([[p[0], p[1]], [p[2], p[3]], [p[4], p[5]], [p[6], p[7]]], perspective_frame(warp, w, h));
    let frame = if perspective.is_some() { perspective_frame(warp, w, h) } else { placed_frame(warp, w, h) };
    (corners_affine(p, frame).unwrap_or(Affine::IDENTITY), perspective)
}

/// The source identifier and placed corners of a `PlLd` block, the placed-layer record Photoshop
/// CS2 and older write with no `SoLd` next to it (#2465). Adobe PSD spec, "Placed Layer": `plcL`,
/// version, Pascal unique id (unpadded), page, total pages, anti-alias policy, layer type, then
/// 8 doubles: the x, y of each corner (top-left, top-right, bottom-right, bottom-left). The warp
/// that follows is read by [`parse_plld_warp`]. `None` for anything else or a truncated record.
pub fn parse_plld(data: &[u8]) -> Option<(String, [f64; 8])> {
    plld_record(data).map(|(id, corners, _)| (id, corners))
}

/// [`parse_plld`] and the offset just past the corners, where the warp starts.
fn plld_record(data: &[u8]) -> Option<(String, [f64; 8], usize)> {
    if data.get(..4)? != b"plcL" {
        return None;
    }
    let n = usize::from(*data.get(8)?);
    let id = String::from_utf8_lossy(data.get(9..9 + n)?).into_owned();
    // Page number, total pages, anti-alias policy and layer type (4 × u32).
    let at = 9 + n + 16;
    let mut corners = [0.0; 8];
    for (i, c) in corners.iter_mut().enumerate() {
        let b = data.get(at + i * 8..at + i * 8 + 8)?;
        *c = f64::from_be_bytes(b.try_into().ok()?);
    }
    corners.iter().all(|v| v.is_finite()).then_some((id, corners, at + 64))
}

/// The warp of a placed layer's `PlLd` record (Photoshop CS2 can warp a smart object).
#[derive(Debug, Clone, PartialEq)]
pub enum PlldWarp {
    /// No warp: `warpNone`, a warp that moves nothing, or a record that ends at its corners.
    None,
    /// The warp, in source pixels like the `SoLd` one.
    Warp(photocraft_geom::warp::Warp),
    /// A warp is stored but can't be read (or the record itself can't be): re-rendering the
    /// layer from its source would drop it.
    Unreadable,
}

/// The warp after a `PlLd` record's corners: a u32 warp version, then a versioned `warp`
/// descriptor (Adobe PSD spec, "Placed Layer"; the layout [`crate::smart_map::plld_bytes`]
/// writes).
pub fn parse_plld_warp(data: &[u8]) -> PlldWarp {
    let Some(rest) = plld_record(data).and_then(|(_, _, end)| data.get(end..)) else { return PlldWarp::Unreadable };
    // Only padding after the corners: no warp stored.
    if rest.iter().all(|b| *b == 0) {
        return PlldWarp::None;
    }
    match rest.get(4..).and_then(parse_prefix_versioned).as_ref().and_then(warp_of) {
        Some(Some(w)) => PlldWarp::Warp(w),
        Some(None) => PlldWarp::None,
        None => PlldWarp::Unreadable,
    }
}

/// The projective placement of a `SoLd` / `SoLE` block whose corners aren't a parallelogram
/// (Distort, Perspective): source pixels → document pixels, row-major 3×3. Read from
/// `nonAffineTransform` (the placed corners), else `Trnf`. `None` for affine placements, which
/// [`parse_smart`]'s affine holds exactly.
pub fn parse_smart_perspective(key: &[u8; 4], data: &[u8]) -> Option<[f64; 9]> {
    if key != b"SoLd" && key != b"SoLE" {
        return None;
    }
    let d = data.get(8..).and_then(parse_prefix_versioned)?;
    let sz = get_desc(&d, "Sz  ")?;
    let (w, h) = (num(sz.get("Wdth"))?, num(sz.get("Hght"))?);
    let quad = |key: &str| -> Option<[[f64; 2]; 4]> {
        let Some(Value::List(pts)) = d.get(key) else { return None };
        let p: Vec<f64> = pts.iter().filter_map(|v| num(Some(v))).collect();
        (p.len() == 8 && p.iter().all(|v| v.is_finite())).then(|| [[p[0], p[1]], [p[2], p[3]], [p[4], p[5]], [p[6], p[7]]])
    };
    let q = quad("nonAffineTransform").or_else(|| quad("Trnf"))?;
    // The warp `parse_placed_warp` reads: a split warp's `quiltWarp` comes first.
    let warp = get_desc(&d, "quiltWarp").or_else(|| get_desc(&d, "warp")).and_then(warp_of).flatten();
    corners_perspective_in(q, perspective_frame(warp.as_ref(), w, h))
}

/// The projective placement of a `w` × `h` source onto corners `q` that aren't a parallelogram;
/// `None` for affine corners (see [`corners_affine`]) or an empty size.
pub fn corners_perspective(q: [[f64; 2]; 4], (w, h): (f64, f64)) -> Option<[f64; 9]> {
    corners_perspective_in(q, [0.0, 0.0, w, h])
}

/// [`corners_perspective`] for the source-space box `frame` (left, top, right, bottom; see
/// [`perspective_frame`]).
pub(crate) fn corners_perspective_in(q: [[f64; 2]; 4], frame: [f64; 4]) -> Option<[f64; 9]> {
    // A parallelogram (top-left + bottom-right = top-right + bottom-left) is affine.
    let tol = 1e-6 * (1.0 + q.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs())));
    let ok = frame.iter().all(|v| v.is_finite()) && frame[2] > frame[0] && frame[3] > frame[1];
    if (0..2).all(|i| (q[0][i] + q[2][i] - q[1][i] - q[3][i]).abs() <= tol) || !ok {
        return None;
    }
    photocraft_algo::transform::Homography::rect_to_quad(frame, q).map(|m| m.0)
}

/// `masterFXSwitch` from an `lfx2` block (defaults to `true`).
pub fn effects_enabled(lfx2: &[u8]) -> bool {
    lfx2.get(4..)
        .and_then(parse_prefix_versioned)
        .and_then(|d| match d.get("masterFXSwitch") {
            Some(Value::Boolean(b)) => Some(*b),
            _ => None,
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod more_tests;
