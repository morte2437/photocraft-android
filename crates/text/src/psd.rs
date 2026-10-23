//! PSD type layers: the `TySh` block (transform + text descriptor with `EngineData` + warp) ⇄ the
//! typed [`TextLayer`] model.
//!
//! Units: EngineData sizes are in text-space units, which the `TySh` transform maps to document
//! pixels; Photoshop shows them as points at the document resolution, so
//! `size_pt = FontSize × 72 / dpi` (and likewise leading, tracking-independent lengths, indents).
//! Box bounds stay in text-space pixels.
//!
//! Writing starts from the layer's original `TySh` (when present) and patches only the text, runs,
//! paragraphs, fonts, shape and transform, so Photoshop-only settings survive an edit.

use photocraft_color::{Color, ColorMode};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{
    AntiAlias, Caps, CharStyle, FontBaseline, Kerning, Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextRun, TextShape, TextWarp,
};
use photocraft_geom::{Affine, Point};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D, VersionedDescriptor};

use crate::engine_data::{self as ed, Value as E};
use crate::fonts::guess_from_postscript;

/// Parsed `TySh` contents.
#[derive(Clone, Debug, PartialEq)]
pub struct TySh {
    pub transform: Affine,
    pub text: Descriptor,
    pub warp: Option<Descriptor>,
    /// The four trailing big-endian f32 values (left, top, right, bottom) after the warp
    /// descriptor. Photoshop writes zeros for unwarped type and the text-space bounds (the
    /// `bounds` descriptor's rectangle) for warped type.
    pub bounds: [f32; 4],
}

/// Whether a type transform can reach rendering or egui. Pixel coordinates use i32 throughout
/// the document, so non-finite and unrepresentable affine terms are rejected (#4758).
pub fn transform_is_safe(t: &Affine) -> bool {
    t.m.iter().all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX))
}

/// Parses a `TySh` block.
pub fn parse_tysh(data: &[u8]) -> Option<TySh> {
    let f = |at: usize| data.get(at..at + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
    let mut m = [0.0; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f(2 + i * 8)?;
    }
    if !transform_is_safe(&Affine { m }) {
        return None;
    }
    let (text, used) = VersionedDescriptor::parse_prefix(data.get(52..)?).ok()?;
    let rest = data.get(52 + used..).unwrap_or(&[]);
    let (warp, wused) = match rest.get(2..).and_then(|r| VersionedDescriptor::parse_prefix(r).ok()) {
        Some((w, n)) => (Some(w.descriptor), n + 2),
        None => (None, 0),
    };
    let tail = rest.get(wused..).unwrap_or(&[]);
    let mut bounds = [0f32; 4];
    for (i, b) in bounds.iter_mut().enumerate() {
        if let Some(x) = tail.get(i * 4..i * 4 + 4) {
            *b = f32::from_be_bytes(x.try_into().unwrap_or([0; 4]));
        }
    }
    Some(TySh { transform: Affine { m }, text: text.descriptor, warp, bounds })
}

/// `tysh` moved by (`dx`, `dy`): only the transform's translation changes. The text, its styles,
/// the warp and the trailing bounds (which are in text space) stay byte for byte, so a moved
/// Photoshop type layer keeps everything Photoshop wrote. `None` when the block is too short.
pub fn translate_tysh(tysh: &[u8], dx: f64, dy: f64) -> Option<Vec<u8>> {
    let mut out = tysh.to_vec();
    // Version (2 bytes), then xx, xy, yx, yy, tx, ty as big-endian f64: tx at 34, ty at 42.
    for (at, d) in [(34, dx), (42, dy)] {
        let b = out.get_mut(at..at + 8)?;
        let v = f64::from_be_bytes(b.try_into().ok()?) + d;
        b.copy_from_slice(&v.to_be_bytes());
    }
    Some(out)
}

/// Serializes a `TySh` block.
pub fn write_tysh(t: &TySh) -> Vec<u8> {
    let mut v = 1u16.to_be_bytes().to_vec();
    for x in t.transform.m {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&50u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.text.clone()).to_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend(VersionedDescriptor::new(t.warp.clone().unwrap_or_else(default_warp)).to_bytes());
    for b in t.bounds {
        v.extend_from_slice(&b.to_be_bytes());
    }
    v
}

fn default_warp() -> Descriptor {
    Descriptor::new("warp")
        .with("warpStyle", enumv("warpStyle", "warpNone"))
        .with("warpValue", D::Double(0.0))
        .with("warpPerspective", D::Double(0.0))
        .with("warpPerspectiveOther", D::Double(0.0))
        .with("warpRotate", enumv("Ornt", "Hrzn"))
}

fn enumv(t: &str, v: &str) -> D {
    D::Enumerated { type_id: Id::new(t), value: Id::new(v) }
}

fn enum_value(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key) {
        Some(D::Enumerated { value, .. }) => Some(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        _ => None,
    }
}

fn dnum(v: Option<&D>) -> Option<f64> {
    match v {
        Some(D::Double(x)) if x.is_finite() => Some(*x),
        Some(D::UnitFloat { value, .. }) if value.is_finite() => Some(*value),
        Some(D::Integer(i)) => Some(f64::from(*i)),
        _ => None,
    }
}

/// The EngineData of a text descriptor.
pub fn engine_data(text: &Descriptor) -> Option<E> {
    match text.get("EngineData") {
        Some(D::RawData(b)) => ed::parse(b).ok(),
        _ => None,
    }
}

/// Whether the text engine of a `TySh` block has nothing to draw: its EngineData text
/// (`EngineDict/Editor/Text`, what Photoshop lays out) is present and only whitespace. Some
/// Photoshop layers carry their characters only in the descriptor's `Txt ` and render nothing.
pub fn engine_text_is_blank(tysh: &[u8]) -> bool {
    parse_tysh(tysh)
        .and_then(|t| engine_data(&t.text))
        .and_then(|e| e.path(&["EngineDict", "Editor", "Text"]).and_then(E::as_str).map(|s| s.trim().is_empty()))
        .unwrap_or(false)
}

/// Parses the document's `Txt2` block (Photoshop's text engine data for all type layers).
pub fn parse_txt2(data: &[u8]) -> Option<E> {
    // Txt2 is a bare sequence of `/key value` pairs (no enclosing `<< >>`).
    let mut wrapped = Vec::with_capacity(data.len() + 4);
    wrapped.extend_from_slice(b"<<");
    wrapped.extend_from_slice(data);
    wrapped.extend_from_slice(b">>");
    ed::parse(&wrapped).ok().filter(|v| matches!(v, E::Dict(_)))
}

/// Applies what Photoshop keeps only in `Txt2` to `layer`, read from `tysh`: the automatic
/// kerning modes (EngineData writes Optical and "0" alike as `AutoKerning true`) and the
/// OpenType settings besides (discretionary) ligatures: stylistic sets, contextual alternates,
/// swashes and the rest ([`TXT2_FEATURES`]), which Photoshop leaves out of EngineData. Only when
/// the layer's `TextIndex` entry still holds the layer's text (an edit elsewhere leaves `Txt2`
/// stale).
///
/// Observed layout: `Txt2 /1` is the list of text objects; an object's `/0` is its text model
/// with `/0` the text (`\r` breaks, trailing `\r`) and `/6 /0` its style runs, each
/// `<< /0 << /0 << … /6 <<style>> >> >> /1 utf16-length >>`; style key `/11` is the auto-kern
/// mode of the pair after each character: 0 manual, 1 metrics, 2 optical.
pub fn apply_txt2(layer: &mut TextLayer, tysh: &[u8], txt2: &E) {
    let Some(index) = parse_tysh(tysh).and_then(|t| match t.text.get("TextIndex") {
        Some(D::Integer(i)) => usize::try_from(*i).ok(),
        _ => None,
    }) else {
        return;
    };
    let Some(model) = txt2.path(&["1", "1"]).and_then(E::as_array).and_then(|a| a.get(index)).and_then(|o| o.get("0")) else {
        return;
    };
    let Some(text) = model.get("0").and_then(E::as_str) else {
        return;
    };
    if text.strip_suffix('\r').unwrap_or(text).replace('\r', "\n") != layer.text {
        return;
    }
    let Some(style_runs) = model.path(&["6", "0"]).and_then(E::as_array) else {
        return;
    };
    let styles: Vec<(f64, Option<&E>)> = style_runs.iter().map(|r| (r.get("1").and_then(E::as_f64).unwrap_or(0.0), r.path(&["0", "0", "6"]))).collect();
    let lens = utf16_to_byte_lengths(text, &styles.iter().map(|m| m.0).collect::<Vec<_>>());
    // Txt2 style per byte segment, then split the model runs at segment boundaries.
    let mut segs: Vec<(usize, Option<&E>)> = lens.into_iter().zip(styles.iter().map(|m| m.1)).collect();
    segs.retain(|s| s.0 > 0);
    let runs = layer.char_runs();
    let mut out: Vec<TextRun> = Vec::new();
    let (mut si, mut seg_end) = (0usize, segs.first().map_or(usize::MAX, |s| s.0));
    let mut at = 0usize;
    for r in runs {
        let end = at + r.len;
        while at < end {
            while at >= seg_end && si + 1 < segs.len() {
                si += 1;
                seg_end = seg_end.saturating_add(segs.get(si).map_or(0, |s| s.0));
            }
            let piece_end = if at < seg_end { end.min(seg_end) } else { end };
            let mut style = r.style.clone();
            let txt2_style = segs.get(si).and_then(|s| s.1).filter(|_| at < seg_end);
            // Txt2 has the exact mode (EngineData's reading also marks neighbours of manual
            // kerns manual, like Photoshop's legacy reader).
            match txt2_style.and_then(|st| st.get("11")).and_then(E::as_i64) {
                Some(0) => style.kerning = Kerning::Off,
                Some(1) => style.kerning = Kerning::Metrics,
                Some(2) => style.kerning = Kerning::Optical,
                _ => {}
            }
            if let Some(st) = txt2_style {
                add_txt2_features(&mut style, st);
            }
            let len = piece_end - at;
            match out.last_mut() {
                Some(last) if last.style == style => last.len += len,
                _ => out.push(TextRun { len, style }),
            }
            at = piece_end;
        }
    }
    layer.runs = out;
    layer.sync_summary();
}

/// The frame (`Txt2 /0 /8 /0` entry) of text object `index`: its rendered frame's `/10` names it.
fn txt2_frame(txt2: &E, index: usize) -> Option<&E> {
    let object = txt2.path(&["1", "1"]).and_then(E::as_array)?.get(index)?;
    let frame =
        object.path(&["1", "2"]).and_then(E::as_array)?.first()?.get("6").and_then(E::as_array)?.iter().find_map(|f| f.get("10").and_then(E::as_i64))?;
    txt2.path(&["0", "8", "0"]).and_then(E::as_array)?.get(usize::try_from(frame).ok()?)?.get("0")
}

/// Places `layer` where Photoshop draws it when the text object's frame in `Txt2` is moved
/// (#1317). Recent Photoshop keeps a type layer's position partly in its `Txt2` frame: a matrix
/// (`/2 /2`) from the frame's space to the `TySh` text space and, for point text, the anchor
/// (`/0`), while EngineData's `PointBase` stays at the origin and `BoxBounds` stays in the
/// frame's space. Ignoring them put the text far from Photoshop's pixels (off the canvas, so
/// it "disappeared" as soon as an edit started), and point text a line too high and off its
/// alignment anchor. Only when the layer's `TextIndex` entry still holds the layer's text, as
/// for [`apply_txt2`], and the frame agrees with the `TySh` `bounds`.
pub fn apply_txt2_frame(layer: &mut TextLayer, tysh: &[u8], txt2: &E) {
    let Some(t) = parse_tysh(tysh) else { return };
    let Some(index) = (match t.text.get("TextIndex") {
        Some(D::Integer(i)) => usize::try_from(*i).ok(),
        _ => None,
    }) else {
        return;
    };
    let text = txt2.path(&["1", "1"]).and_then(E::as_array).and_then(|a| a.get(index)).and_then(|o| o.path(&["0", "0"])).and_then(E::as_str);
    if text.is_none_or(|t| t.strip_suffix('\r').unwrap_or(t).replace('\r', "\n") != layer.text) {
        return;
    }
    let Some(frame) = txt2_frame(txt2, index) else { return };
    let m = arr_f(frame.path(&["2", "2"]));
    let matrix = if m.len() == 6 && m.iter().all(|v| v.is_finite()) { Affine { m: [m[0], m[1], m[2], m[3], m[4], m[5]] } } else { Affine::IDENTITY };
    let at = match layer.shape {
        TextShape::Point | TextShape::Path { .. } => {
            let p = arr_f(frame.get("0"));
            if p.len() == 2 && p.iter().all(|v| v.is_finite()) { Affine::translate(p[0], p[1]) } else { Affine::IDENTITY }
        }
        TextShape::Box { .. } => Affine::IDENTITY,
    };
    let to_text = matrix.mul(&at);
    if to_text == Affine::IDENTITY || to_text.determinant().abs() <= 1e-9 {
        return;
    }
    // The frame's origin (point text: the anchor; box text: the box's top left) must land in
    // the text-space `bounds` Photoshop wrote beside it, and not already be there without it. A `TySh` PhotoCraft rewrote (an edited
    // or moved layer whose text is unchanged keeps its old `Txt2` object and frame) already has
    // the frame folded into its transform and its `bounds` near the origin: applying the frame
    // again would move it twice.
    let origin = match layer.shape {
        TextShape::Point | TextShape::Path { .. } => Point::new(0.0, 0.0),
        TextShape::Box { .. } => {
            let b = engine_data(&t.text)
                .map(|e| {
                    arr_f(
                        e.path(&["EngineDict", "Rendered", "Shapes", "Children"])
                            .and_then(E::as_array)
                            .and_then(|c| c.first())
                            .and_then(|s| s.path(&["Cookie", "Photoshop", "BoxBounds"])),
                    )
                })
                .unwrap_or_default();
            let [x, y, ..] = b[..] else { return };
            Point::new(x, y)
        }
    };
    let Some([l, top, r, bottom]) = descriptor_rect(&t.text, "bounds") else { return };
    let inside = |p: Point| p.x >= l - 1.0 && p.x <= r + 1.0 && p.y >= top - 1.0 && p.y <= bottom + 1.0;
    if inside(origin) || !inside(to_text.apply(origin)) {
        return;
    }
    {
        // From the file's own transform: the frame's anchor is exact, so it replaces the
        // ink-box estimate `legacy_point_origin` made for such point text (#1469), which put
        // the baseline at the ink's top and the anchor at the ink's left whatever the alignment.
        layer.transform = t.transform.mul(&to_text);
    }
}

/// OpenType settings of a `Txt2` style that are on/off flags, by style key, as Photoshop
/// writes them (each turned on alone in the Character panel's OpenType settings and read back):
/// `/20` Contextual Alternates, `/23` Fractions, `/24` Ordinals, `/25` Swash, `/26` Titling
/// Alternates, `/28` Stylistic Alternates, `/29` Ornaments. (`/18` and `/19` are the standard and
/// discretionary ligatures, which EngineData carries too; `/27` is Arabic connection forms.)
pub const TXT2_FEATURES: [(&str, &str); 7] = [("20", "calt"), ("23", "frac"), ("24", "ordn"), ("25", "swsh"), ("26", "titl"), ("28", "salt"), ("29", "ornm")];

/// The `Txt2` style key holding the Stylistic Sets as a bit mask: bit n is set n + 1
/// (`ss01`..`ss20`).
pub const TXT2_STYLISTIC_SETS: &str = "85";

/// Sets in `style` the OpenType features a `Txt2` style dictionary turns on ([`TXT2_FEATURES`]
/// and the Stylistic Sets mask). A feature the style already sets keeps its value. A flag that is
/// off changes nothing, except Contextual Alternates: shapers apply `calt` by default, Photoshop
/// only when it is on (its default is off), so off is written as `calt` 0.
fn add_txt2_features(style: &mut CharStyle, txt2_style: &E) {
    let mut set = |tag: String, value: u16| {
        if !style.features.iter().any(|f| f.tag == tag) {
            style.features.push(photocraft_doc::text::FontFeature { tag, value });
        }
    };
    for (key, tag) in TXT2_FEATURES {
        match txt2_style.get(key).and_then(E::as_bool) {
            Some(true) => set(tag.to_string(), 1),
            Some(false) if tag == "calt" => set(tag.to_string(), 0),
            _ => {}
        }
    }
    // A mask beyond the 20 sets is not Photoshop's: ignore it.
    let sets = txt2_style.get(TXT2_STYLISTIC_SETS).and_then(E::as_i64).filter(|m| (0..1 << 20).contains(m)).unwrap_or(0);
    for n in 0..20 {
        if sets & (1 << n) != 0 {
            set(format!("ss{:02}", n + 1), 1);
        }
    }
}

/// Builds a [`TextLayer`] (model, text and transform; no cache, no `psd_raw`) from `TySh` data.
pub fn text_layer_from_tysh(data: &[u8], dpi: f32) -> Option<TextLayer> {
    let t = parse_tysh(data)?;
    let mut layer = TextLayer { transform: t.transform, ..Default::default() };
    let txt = match t.text.get("Txt ") {
        Some(D::Text(s)) => Some(s.to_string_lossy().trim_end_matches('\0').to_string()),
        _ => None,
    };
    layer.orientation = if enum_value(&t.text, "Ornt").as_deref() == Some("Vrtc") { Orientation::Vertical } else { Orientation::Horizontal };
    layer.antialias = match enum_value(&t.text, "AntA").as_deref() {
        Some("Anno" | "antiAliasNone") => AntiAlias::None,
        Some("antiAliasSharp" | "AnSh") => AntiAlias::Sharp,
        Some("AnCr" | "antiAliasCrisp") => AntiAlias::Crisp,
        Some("AnSt" | "antiAliasStrong") => AntiAlias::Strong,
        Some("antiAliasPlatformGray") => AntiAlias::Windows,
        Some("antiAliasPlatformLCD") => AntiAlias::WindowsLcd,
        Some(_) => AntiAlias::Smooth,
        // No `AntA`: the engine data's mode says it.
        None => {
            engine_data(&t.text).and_then(|e| e.path(&["EngineDict", "AntiAlias"]).and_then(E::as_f64)).and_then(engine_antialias).unwrap_or(AntiAlias::Smooth)
        }
    };
    if let Some(w) = &t.warp {
        let style = enum_value(w, "warpStyle").unwrap_or_else(|| "warpNone".into());
        if style != "warpNone" {
            // Bend and distortion are percentages in -100..100 (as `type.warpText` clamps them);
            // clamping before the cast keeps a value beyond f32 from becoming inf.
            let pct = |k: &str| dnum(w.get(k)).unwrap_or(0.0).clamp(-100.0, 100.0) as f32;
            layer.warp = Some(TextWarp {
                style,
                value: pct("warpValue"),
                horizontal_distortion: pct("warpPerspective"),
                vertical_distortion: pct("warpPerspectiveOther"),
                horizontal: enum_value(w, "warpRotate").as_deref() != Some("Vrtc"),
            });
        }
    }
    let k = 72.0 / if dpi.is_finite() && dpi > 0.0 { dpi } else { 72.0 };
    if let Some(e) = engine_data(&t.text) {
        // Without an `Ornt` key, the engine data's writing direction says it (2 = vertical).
        if enum_value(&t.text, "Ornt").is_none() && e.path(&["EngineDict", "Rendered", "Shapes", "WritingDirection"]).and_then(E::as_f64) == Some(2.0) {
            layer.orientation = Orientation::Vertical;
        }
        apply_engine_data(&mut layer, &e, txt.as_deref(), k);
    } else {
        layer.text = txt.unwrap_or_default().replace('\r', "\n");
    }
    // Some legacy PSDs store point text at a large local origin even though PointBase is zero.
    // Their transform maps that origin onto the canvas, but maps our zero-based layout far away.
    // Move the imported layout to the descriptor's ink origin so editing, engine commands, and
    // writing the TySh all agree on the visible position (#1469).
    if let Some([x, y]) = legacy_point_origin(&t) {
        let [a, b, c, d, e, f] = layer.transform.m;
        layer.transform.m = [a, b, c, d, a * x + c * y + e, b * x + d * y + f];
    }
    layer.sync_summary();
    Some(layer)
}

/// The local origin to fold into a legacy point-text transform, if present.
///
/// Photoshop normally keeps point text close to its PointBase. The affected legacy files instead
/// use a zero PointBase plus a very large `boundingBox` origin, with an equally remote transform
/// translation that cancels it out. Do not apply this to ordinary point text: its transform origin
/// is already the layout origin, and shifting it by the ink bounds would move the layer.
fn legacy_point_origin(t: &TySh) -> Option<[f64; 2]> {
    let e = engine_data(&t.text)?;
    let point_base = e
        .path(&["EngineDict", "Rendered", "Shapes", "Children"])
        .and_then(E::as_array)
        .and_then(|children| children.first())
        .and_then(|shape| shape.path(&["Cookie", "Photoshop", "PointBase"]))
        .and_then(E::as_array)?;
    let [px, py] = point_base else { return None };
    if px.as_f64() != Some(0.0) || py.as_f64() != Some(0.0) {
        return None;
    }
    let bounds = descriptor_rect(&t.text, "boundingBox")?;
    let [a, b, c, d, e, f] = t.transform.m;
    let values = [a, b, c, d, e, f, bounds[0], bounds[1]];
    if !values.iter().all(|v| v.is_finite()) {
        return None;
    }
    let shifted = [a * bounds[0] + c * bounds[1] + e, b * bounds[0] + d * bounds[1] + f];
    let origin_distance = e.abs().max(f.abs());
    let shifted_distance = shifted[0].abs().max(shifted[1].abs());
    // A local bounding-box origin in the thousands and a transform origin beyond a normal canvas
    // are the signature of this old writer. Requiring that the bounding-box origin is closer to
    // the document prevents applying the correction again after export/import.
    (bounds[0].abs().max(bounds[1].abs()) > 1024.0 && origin_distance > 4096.0 && shifted_distance < origin_distance).then_some([bounds[0], bounds[1]])
}

fn descriptor_rect(text: &Descriptor, key: &str) -> Option<[f64; 4]> {
    let D::Descriptor(rect) = text.get(key)? else { return None };
    let bounds = [dnum(rect.get("Left"))?, dnum(rect.get("Top "))?, dnum(rect.get("Rght"))?, dnum(rect.get("Btom"))?];
    (bounds.iter().all(|v| v.is_finite()) && bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
}

const MAX_ENGINE_RUNS: usize = 65_536;

pub(crate) fn arr_f(v: Option<&E>) -> Vec<f64> {
    v.and_then(E::as_array).map(|a| a.iter().take(MAX_ENGINE_RUNS).filter_map(E::as_f64).collect()).unwrap_or_default()
}

/// Converts UTF-16 run lengths over `text` into UTF-8 byte lengths, stopping after the first run
/// once `text` is exhausted so trailing runs past the text aren't processed.
pub(crate) fn utf16_to_byte_lengths(text: &str, lens: &[f64]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lens.len().min(text.len().saturating_add(1)));
    let mut chars = text.chars().peekable();
    let mut exhausted = false;
    for &l in lens.iter().take(MAX_ENGINE_RUNS) {
        if exhausted {
            break;
        }
        let mut units = l.max(0.0) as usize;
        let mut bytes = 0;
        while units > 0 {
            match chars.next() {
                Some(c) => {
                    units = units.saturating_sub(c.len_utf16());
                    bytes += c.len_utf8();
                }
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        if chars.peek().is_none() {
            exhausted = true;
        }
        out.push(bytes);
    }
    out
}

fn apply_engine_data(layer: &mut TextLayer, e: &E, txt: Option<&str>, k: f32) {
    let ed_text = e.path(&["EngineDict", "Editor", "Text"]).and_then(E::as_str).unwrap_or("").to_string();
    // Photoshop always ends the engine text with a paragraph break; the descriptor text doesn't.
    let text = txt.map(str::to_string).unwrap_or_else(|| ed_text.strip_suffix('\r').unwrap_or(&ed_text).to_string());
    layer.text = text.replace('\r', "\n");
    let res = e.get("ResourceDict").or_else(|| e.get("DocumentResources"));
    let fonts: Vec<String> = res
        .and_then(|r| r.get("FontSet"))
        .and_then(E::as_array)
        .map(|a| a.iter().map(|f| f.get("Name").and_then(E::as_str).unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    // Style resolution (#3795). A run's own `StyleSheetData` / `Properties` hold only what it
    // overrides; the rest comes from its sheets, as in Photoshop:
    // - a paragraph run: its `Properties` → the `ParagraphSheetSet` entry its `Name` names (a
    //   paragraph style; or its `Parent` index) → `TheNormalParagraphSheet`;
    // - a character run: its `StyleSheetData` → the `StyleSheetSet` entry its `Parent` or `Name`
    //   names (a character style) → the default character style of its paragraph (the paragraph run's
    //   `DefaultStyleSheet`, else its paragraph style's) → `TheNormalStyleSheet`.
    // Missing or out-of-range indices fall through to the next level, then to the defaults.
    let style_set: &[E] = res.and_then(|r| r.get("StyleSheetSet")).and_then(E::as_array).unwrap_or(&[]);
    let para_set: &[E] = res.and_then(|r| r.get("ParagraphSheetSet")).and_then(E::as_array).unwrap_or(&[]);
    let index = |key: &str| res.and_then(|r| r.get(key)).and_then(E::as_i64).and_then(|i| usize::try_from(i).ok());
    let normal_style = index("TheNormalStyleSheet").and_then(|i| style_set.get(i)).or(style_set.first());
    let normal_para = index("TheNormalParagraphSheet").and_then(|i| para_set.get(i)).or(para_set.first());
    // A run names its sheet by a `Parent` index or by `Name` (see `psd_styles::reference`).
    let named = |set: &'_ [E], sheet: &E| -> Option<usize> {
        if let Some(i) = sheet.get("Parent").and_then(E::as_i64).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < set.len()) {
            return Some(i);
        }
        let name = sheet.get("Name").and_then(E::as_str)?;
        set.iter().position(|s| s.get("Name").and_then(E::as_str) == Some(name))
    };

    // Paragraph runs first: a character run's defaults depend on its paragraph.
    let prun = e.path(&["EngineDict", "ParagraphRun"]);
    let plens16 = arr_f(prun.and_then(|r| r.get("RunLengthArray")));
    let plens = utf16_to_byte_lengths(&ed_text, &plens16);
    let max_pruns = plens.len().max(1);
    let psheets: Vec<&E> = prun
        .and_then(|r| r.get("RunArray"))
        .and_then(E::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("ParagraphSheet")).take(max_pruns).collect())
        .unwrap_or_default();
    let plens16 = &plens16[..plens16.len().min(psheets.len())];
    let empty = E::Dict(Vec::new());
    // Per paragraph run: its borrowed base property layers and its default character style.
    let para_bases: Vec<([Option<&E>; 2], Option<&E>)> = psheets
        .iter()
        .map(|sheet| {
            let style = named(para_set, sheet).and_then(|i| para_set.get(i));
            let base = [style.and_then(|s| s.get("Properties")), normal_para.and_then(|s| s.get("Properties"))];
            let char_default = [Some(*sheet), style]
                .into_iter()
                .flatten()
                .find_map(|s| s.get("DefaultStyleSheet").and_then(E::as_i64).and_then(|i| usize::try_from(i).ok()).and_then(|i| style_set.get(i)));
            (base, char_default)
        })
        .collect();
    layer.paragraphs = psheets
        .iter()
        .zip(&para_bases)
        .zip(plens)
        .map(|((sheet, (base, _)), len)| ParagraphRun { len, style: para_style(base, sheet.get("Properties").unwrap_or(&empty), k) })
        .collect();

    // Character runs.
    let srun = e.path(&["EngineDict", "StyleRun"]);
    let slens16 = arr_f(srun.and_then(|r| r.get("RunLengthArray")));
    let slens = utf16_to_byte_lengths(&ed_text, &slens16);
    let max_sruns = slens.len().max(1);
    let ssheets: Vec<&E> = srun
        .and_then(|r| r.get("RunArray"))
        .and_then(E::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("StyleSheet")).take(max_sruns).collect())
        .unwrap_or_default();
    // The paragraph run each character run starts in (UTF-16 offsets).
    let para_of = |at: f64| -> Option<usize> {
        let mut end = 0.0;
        for (i, n) in plens16.iter().enumerate() {
            end += n;
            if at < end {
                return Some(i);
            }
        }
        plens16.len().checked_sub(1)
    };
    let mut at16 = 0.0;
    let bases: Vec<[Option<&E>; 3]> = ssheets
        .iter()
        .enumerate()
        .map(|(i, sheet)| {
            let own = named(style_set, sheet).and_then(|i| style_set.get(i));
            let para_default = para_of(at16).and_then(|p| para_bases.get(p)).and_then(|(_, c)| *c);
            at16 += slens16.get(i).copied().unwrap_or(0.0);
            [own, para_default, normal_style].map(|s| s.and_then(|s| s.get("StyleSheetData")))
        })
        .collect();
    let sdata: Vec<&E> = ssheets.iter().map(|s| s.get("StyleSheetData").unwrap_or(&empty)).collect();
    let incoming: Vec<(usize, bool, f32)> = sdata
        .iter()
        .zip(&bases)
        .zip(&slens)
        .map(|((d, base), len)| {
            let auto = lookup(d, base, "AutoKerning").and_then(E::as_bool) != Some(false);
            let kern = lookup(d, base, "Kerning").and_then(E::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0) as f32;
            (*len, auto, kern)
        })
        .collect();
    layer.runs = sdata.iter().zip(&bases).zip(slens).map(|((d, base), len)| TextRun { len, style: char_style(base, d, &fonts, k) }).collect();
    layer.runs = kerning_from_pairs(&layer.text, layer.char_runs(), &incoming);

    // Shape.
    let shape = e.path(&["EngineDict", "Rendered", "Shapes", "Children"]).and_then(E::as_array).and_then(|a| a.first());
    if let Some(sh) = shape
        && sh.get("ShapeType").and_then(E::as_i64) == Some(1)
    {
        let b = arr_f(sh.path(&["Cookie", "Photoshop", "BoxBounds"]));
        if b.len() == 4 {
            layer.shape = TextShape::Box { x: b[0] as f32, y: b[1] as f32, width: (b[2] - b[0]) as f32, height: (b[3] - b[1]) as f32 };
        }
    }
}

fn lookup<'a>(run: &'a E, base: &[Option<&'a E>], key: &str) -> Option<&'a E> {
    run.get(key).or_else(|| base.iter().flatten().find_map(|b| b.get(key)))
}

pub(crate) fn char_style(base: &[Option<&E>], d: &E, fonts: &[String], k: f32) -> CharStyle {
    let g = |key: &str| lookup(d, base, key);
    let num = |key: &str| g(key).and_then(E::as_f64);
    // An f32 that a huge finite f64 overflowed to infinity is dropped too.
    let numk = |key: &str, k: f32| num(key).map(|v| v as f32 * k).filter(|v| v.is_finite());
    let flag = |key: &str| g(key).and_then(E::as_bool);
    let mut s = CharStyle::default();
    if let Some(ps) = g("Font").and_then(E::as_i64).and_then(|i| usize::try_from(i).ok()).and_then(|i| fonts.get(i)) {
        let r = guess_from_postscript(ps);
        s.font_family = r.family;
        s.weight = r.weight;
        s.italic = r.italic;
        s.postscript_name = Some(ps.clone());
    }
    if let Some(v) = numk("FontSize", k) {
        s.size_pt = v;
    }
    if flag("AutoLeading") == Some(false)
        && let Some(v) = numk("Leading", k)
    {
        s.leading_pt = Some(v);
    }
    s.tracking = numk("Tracking", 1.0).unwrap_or(0.0);
    s.baseline_shift_pt = numk("BaselineShift", k).unwrap_or(0.0);
    s.horizontal_scale = numk("HorizontalScale", 1.0).unwrap_or(1.0);
    s.vertical_scale = numk("VerticalScale", 1.0).unwrap_or(1.0);
    s.faux_bold = flag("FauxBold").unwrap_or(false);
    s.faux_italic = flag("FauxItalic").unwrap_or(false);
    s.underline = flag("Underline").unwrap_or(false);
    s.strikethrough = flag("Strikethrough").unwrap_or(false);
    s.ligatures = flag("Ligatures").unwrap_or(true);
    s.discretionary_ligatures = flag("DLigatures").unwrap_or(false);
    for (key, tag) in OPENTYPE_KEYS {
        if flag(key) == Some(true) {
            s.features.push(photocraft_doc::text::FontFeature { tag: tag.to_string(), value: 1 });
        }
    }
    s.kerning = if flag("AutoKerning") == Some(false) { Kerning::Off } else { Kerning::Metrics };
    s.kern = numk("Kerning", 1.0).unwrap_or(0.0);
    s.caps = match g("FontCaps").and_then(E::as_i64) {
        Some(1) => Caps::SmallCaps,
        Some(2) => Caps::AllCaps,
        _ => Caps::Normal,
    };
    s.baseline = match g("FontBaseline").and_then(E::as_i64) {
        Some(1) => FontBaseline::Superscript,
        Some(2) => FontBaseline::Subscript,
        _ => FontBaseline::Normal,
    };
    if let Some(c) = g("FillColor") {
        // Positions matter (alpha, then the channels): a non-finite entry voids the colour.
        let v: Vec<f64> = c.get("Values").and_then(E::as_array).map(|a| a.iter().filter_map(E::as_f64).collect()).unwrap_or_default();
        let n_raw = c.get("Values").and_then(E::as_array).map_or(0, <[E]>::len);
        let v = if v.len() == n_raw && v.iter().all(|x| (*x as f32).is_finite()) { v } else { Vec::new() };
        let a = v.first().copied().unwrap_or(1.0) as f32;
        s.color = match v.len() {
            5 => Color { mode: ColorMode::Cmyk, c: [v[1] as f32, v[2] as f32, v[3] as f32, v[4] as f32], alpha: a },
            2 => Color { mode: ColorMode::Grayscale, c: [v[1] as f32, 0.0, 0.0, 0.0], alpha: a },
            4 => Color::rgba(v[1] as f32, v[2] as f32, v[3] as f32, a),
            _ => Color::BLACK,
        };
    }
    s
}

pub(crate) fn para_style(base: &[Option<&E>], d: &E, k: f32) -> ParagraphStyle {
    let num = |key: &str| lookup(d, base, key).and_then(E::as_f64);
    let numk = |key: &str, k: f32| num(key).map(|v| v as f32 * k).filter(|v| v.is_finite());
    ParagraphStyle {
        align: match num("Justification").map(|v| v as i64) {
            Some(1) => TextAlign::Right,
            Some(2) => TextAlign::Center,
            Some(3) => TextAlign::JustifyLeft,
            Some(4) => TextAlign::JustifyRight,
            Some(5) => TextAlign::JustifyCenter,
            Some(6) => TextAlign::JustifyAll,
            _ => TextAlign::Left,
        },
        first_line_indent_pt: numk("FirstLineIndent", k).unwrap_or(0.0),
        start_indent_pt: numk("StartIndent", k).unwrap_or(0.0),
        end_indent_pt: numk("EndIndent", k).unwrap_or(0.0),
        space_before_pt: numk("SpaceBefore", k).unwrap_or(0.0),
        space_after_pt: numk("SpaceAfter", k).unwrap_or(0.0),
        auto_leading: numk("AutoLeading", 1.0).unwrap_or(1.2),
        hyphenate: lookup(d, base, "AutoHyphenate").and_then(E::as_bool).unwrap_or(false),
        ..Default::default()
    }
}

// ---------- writing ----------

fn real(v: f32) -> E {
    E::Real(f64::from(v))
}

/// Manual kerning as Photoshop stores it: a whole number of 1/1000 em (its UI range is ±1000;
/// anything non-finite writes 0).
pub(crate) fn kern_units(v: f32) -> i64 {
    if v.is_finite() { v.round().clamp(-10_000.0, 10_000.0) as i64 } else { 0 }
}

// Kerning in EngineData, as observed in Photoshop 2026 (writing, and reading files without
// `Txt2`): a manual kern k (1/1000 em) after character c is written on character c + 1 as
// `AutoKerning false` + `Kerning k`; everything else is `true` + 0. Photoshop's reader takes
// `Kerning k` on character j as the kern after j - 1 (making j - 1 manual), and
// `AutoKerning false` on j as "character j - 2 is manually kerned" (no automatic kerning). The
// automatic mode itself (Metrics or Optical) isn't in EngineData: Photoshop keeps it in the
// document's `Txt2` (see [`apply_txt2`]). We write and read the same way, so Photoshop reads
// our kerning as we do and Photoshop's own fields round-trip unchanged. (The mode of the last
// character has no slot and reads back as Metrics.)

fn manual(s: &CharStyle) -> bool {
    s.kerning == Kerning::Off || kern_units(s.kern) != 0
}

/// EngineData style runs for `ed_text` (the layer text with `\r` breaks and the trailing `\r`):
/// (model run index, `(AutoKerning, Kerning)`, UTF-16 length), split where those change.
fn pair_runs(ed_text: &str, runs: &[TextRun]) -> Vec<(usize, (bool, i64), usize)> {
    // Model run of each character (the trailing `\r` takes the last run).
    let mut owner: Vec<(usize, usize)> = Vec::with_capacity(ed_text.len());
    let (mut ri, mut run_end) = (0usize, runs.first().map_or(0, |r| r.len));
    for (b, ch) in ed_text.char_indices() {
        while b >= run_end && ri + 1 < runs.len() {
            ri += 1;
            run_end = run_end.saturating_add(runs.get(ri).map_or(0, |r| r.len));
        }
        owner.push((ri, ch.len_utf16()));
    }
    let style = |i: usize| owner.get(i).and_then(|o| runs.get(o.0)).map(|r| &r.style);
    let mut out: Vec<(usize, (bool, i64), usize)> = Vec::new();
    for (j, &(ri, n)) in owner.iter().enumerate() {
        let k = j.checked_sub(1).and_then(style).map_or(0, |s| kern_units(s.kern));
        // `false` marks character j - 2 manual: written when it is and its own kern (on j - 1)
        // doesn't say so already; harmless (and Photoshop's own form) next to a kern on j.
        let off = match j.checked_sub(2) {
            Some(p) => style(p).is_some_and(|s| manual(s) && (kern_units(s.kern) == 0 || k != 0)),
            None => k != 0,
        };
        let fields = (!off, k);
        match out.last_mut() {
            Some(last) if last.0 == ri && last.1 == fields => last.2 += n,
            _ => out.push((ri, fields, n)),
        }
    }
    out
}

/// Reads EngineData's kerning fields back into the model (see the notes above). `incoming` is
/// per EngineData style run: (UTF-8 length in the engine text, `AutoKerning`, `Kerning`).
fn kerning_from_pairs(text: &str, runs: Vec<TextRun>, incoming: &[(usize, bool, f32)]) -> Vec<TextRun> {
    if text.is_empty() || runs.is_empty() {
        return runs;
    }
    // Byte offset of every character, plus the trailing break.
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).chain(std::iter::once(text.len())).collect();
    let n = starts.len().saturating_sub(1);
    let mut kern = vec![0.0f32; n];
    let mut off = vec![false; n];
    let (mut si, mut seg_end) = (0usize, incoming.first().map_or(0, |s| s.0));
    for (j, &b) in starts.iter().enumerate() {
        while b >= seg_end && si < incoming.len() {
            si += 1;
            seg_end = seg_end.saturating_add(incoming.get(si).map_or(0, |s| s.0));
        }
        let Some(&(_, auto, k)) = incoming.get(si) else { break };
        if k != 0.0
            && let Some(p) = j.checked_sub(1)
            && let (Some(kp), Some(op)) = (kern.get_mut(p), off.get_mut(p))
        {
            *kp = k;
            *op = true;
        }
        if !auto && let Some(op) = j.checked_sub(2).and_then(|p| off.get_mut(p)) {
            *op = true;
        }
    }
    let (mut ri, mut run_end) = (0usize, runs.first().map_or(0, |r| r.len));
    let mut out: Vec<TextRun> = Vec::new();
    for (c, w) in starts.windows(2).enumerate() {
        let (b, len) = (w[0], w[1] - w[0]);
        while b >= run_end && ri + 1 < runs.len() {
            ri += 1;
            run_end = run_end.saturating_add(runs.get(ri).map_or(0, |r| r.len));
        }
        let Some(r) = runs.get(ri) else { break };
        let k = kern.get(c).copied().unwrap_or(0.0);
        let kerning = if off.get(c).copied().unwrap_or(false) { Kerning::Off } else { Kerning::Metrics };
        match out.last_mut() {
            Some(last) if last.style.kern == k && last.style.kerning == kerning && same_but_kerning(&last.style, &r.style) => last.len += len,
            _ => {
                let mut style = r.style.clone();
                style.kerning = kerning;
                style.kern = k;
                out.push(TextRun { len, style });
            }
        }
    }
    out
}

/// Whether two styles differ at most in their kerning.
fn same_but_kerning(a: &CharStyle, b: &CharStyle) -> bool {
    CharStyle { kerning: b.kerning, kern: b.kern, ..a.clone() } == *b
}

/// PostScript name for a style (the PSD `FontSet` stores these).
pub(crate) fn postscript_for(s: &CharStyle) -> String {
    if let Some(ps) = &s.postscript_name {
        return ps.clone();
    }
    let fam = if s.font_family.is_empty() { crate::fonts::DEFAULT_FAMILY } else { &s.font_family };
    let w = match s.weight {
        0..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    };
    let style = match (w, s.italic) {
        ("Regular", true) => "Italic".to_string(),
        (w, true) => format!("{w}Italic"),
        (w, false) => w.to_string(),
    };
    format!("{}-{style}", fam.replace(' ', ""))
}

/// EngineData style keys for the Type › OpenType toggles and the OpenType feature each one turns
/// on (the standard and discretionary ligature keys map to dedicated [`CharStyle`] fields).
pub const OPENTYPE_KEYS: [(&str, &str); 8] = [
    ("ContextualLigatures", "calt"),
    ("Swash", "swsh"),
    ("OldStyle", "onum"),
    ("StylisticAlternates", "salt"),
    ("Titling", "titl"),
    ("Ornaments", "ornm"),
    ("Ordinals", "ordn"),
    ("Fractions", "frac"),
];

/// A fill colour as EngineData and `Txt2` store it: the colour type (0 gray, 1 RGB, 2 CMYK) and
/// its values, alpha first.
fn fill_color(c: &Color) -> (i64, Vec<E>) {
    match c.mode {
        ColorMode::Cmyk => (2, vec![real(c.alpha), real(c.c[0]), real(c.c[1]), real(c.c[2]), real(c.c[3])]),
        ColorMode::Grayscale => (0, vec![real(c.alpha), real(c.c[0])]),
        _ => {
            let [r, g, b] = c.to_rgb();
            (1, vec![real(c.alpha), real(r), real(g), real(b)])
        }
    }
}

pub(crate) fn style_sheet_data(s: &CharStyle, font: usize, k: f32) -> E {
    let (color_type, values) = fill_color(&s.color);
    let mut opentype: Vec<(String, E)> = OPENTYPE_KEYS
        .iter()
        .filter(|(_, tag)| s.features.iter().any(|f| f.tag == *tag && f.value > 0))
        .map(|(key, _)| (key.to_string(), E::Bool(true)))
        .collect();
    let mut dict = vec![
        ("Font".into(), E::Int(font as i64)),
        ("FontSize".into(), real(s.size_pt / k)),
        ("FauxBold".into(), E::Bool(s.faux_bold)),
        ("FauxItalic".into(), E::Bool(s.faux_italic)),
        ("AutoLeading".into(), E::Bool(s.leading_pt.is_none())),
        ("Leading".into(), real(s.leading_pt.unwrap_or(0.0) / k)),
        ("HorizontalScale".into(), real(s.horizontal_scale)),
        ("VerticalScale".into(), real(s.vertical_scale)),
        ("Tracking".into(), E::Int(s.tracking.round() as i64)),
        ("AutoKerning".into(), E::Bool(s.kerning != Kerning::Off)),
        ("Kerning".into(), E::Int(kern_units(s.kern))),
        ("BaselineShift".into(), real(s.baseline_shift_pt / k)),
        (
            "FontCaps".into(),
            E::Int(match s.caps {
                Caps::Normal => 0,
                Caps::SmallCaps => 1,
                Caps::AllCaps => 2,
            }),
        ),
        (
            "FontBaseline".into(),
            E::Int(match s.baseline {
                FontBaseline::Normal => 0,
                FontBaseline::Superscript => 1,
                FontBaseline::Subscript => 2,
            }),
        ),
        ("Underline".into(), E::Bool(s.underline)),
        ("Strikethrough".into(), E::Bool(s.strikethrough)),
        ("Ligatures".into(), E::Bool(s.ligatures)),
        ("DLigatures".into(), E::Bool(s.discretionary_ligatures)),
        // Photopea lays out edited text without painting it unless its fill is enabled.
        ("FillFlag".into(), E::Bool(true)),
        ("FillColor".into(), E::Dict(vec![("Type".into(), E::Int(color_type)), ("Values".into(), E::Array(values))])),
    ];
    dict.append(&mut opentype);
    E::Dict(dict)
}

pub(crate) fn paragraph_properties(p: &ParagraphStyle, k: f32) -> E {
    let j = match p.align {
        TextAlign::Left => 0,
        TextAlign::Right => 1,
        TextAlign::Center => 2,
        TextAlign::JustifyLeft => 3,
        TextAlign::JustifyRight => 4,
        TextAlign::JustifyCenter => 5,
        TextAlign::JustifyAll => 6,
    };
    E::Dict(vec![
        ("Justification".into(), E::Int(j)),
        ("FirstLineIndent".into(), real(p.first_line_indent_pt / k)),
        ("StartIndent".into(), real(p.start_indent_pt / k)),
        ("EndIndent".into(), real(p.end_indent_pt / k)),
        ("SpaceBefore".into(), real(p.space_before_pt / k)),
        ("SpaceAfter".into(), real(p.space_after_pt / k)),
        ("AutoHyphenate".into(), E::Bool(p.hyphenate)),
        ("AutoLeading".into(), real(p.auto_leading)),
        ("LeadingType".into(), E::Int(0)),
    ])
}

fn utf16_len(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

pub(crate) fn font_entry(name: &str) -> E {
    E::Dict(vec![("Name".into(), E::String(name.into())), ("Script".into(), E::Int(0)), ("FontType".into(), E::Int(1)), ("Synthetic".into(), E::Int(0))])
}

/// Keep imported font metadata while retaining the requested font indices. New fonts use
/// the normal defaults; same-name duplicates retain their own positional records.
pub(crate) fn font_set(names: &[String], original: Option<&E>) -> E {
    let entries = original.and_then(E::as_array).unwrap_or_default();
    // Resolve fallback names once, retaining the first match just as the original scan did.
    // Missing names and reordered resource dictionaries must not rescan all entries per font.
    let mut first_by_name = std::collections::HashMap::new();
    for entry in entries {
        if let Some(name) = entry.get("Name").and_then(E::as_str) {
            first_by_name.entry(name).or_insert(entry);
        }
    }
    E::Array(
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let matches = |entry: &&E| entry.get("Name").and_then(E::as_str) == Some(name.as_str());
                entries.get(i).filter(matches).or_else(|| first_by_name.get(name.as_str()).copied()).cloned().unwrap_or_else(|| font_entry(name))
            })
            .collect(),
    )
}

/// The `Txt2` auto-kern mode of a style: 0 manual, 1 metrics, 2 optical — the values
/// [`apply_txt2`] maps back to [`Kerning`]. EngineData's `AutoKerning` can't say Metrics from
/// Optical, so the mode only lives in `Txt2`.
fn txt2_mode(s: &CharStyle) -> i64 {
    if manual(s) {
        0
    } else if s.kerning == Kerning::Optical {
        2
    } else {
        1
    }
}

/// `Txt2` style runs for `layer`: `(mode, UTF-16 length)` over the engine text (`\r` breaks and
/// the trailing `\r`, which takes the last run's mode), split where the auto-kern mode changes.
fn txt2_mode_runs(layer: &TextLayer) -> Vec<(i64, usize)> {
    let runs = layer.char_runs();
    let ed_text = format!("{}\r", layer.text.replace('\n', "\r"));
    let mut out: Vec<(i64, usize)> = Vec::new();
    let (mut ri, mut run_end) = (0usize, runs.first().map_or(0, |r| r.len));
    for (b, ch) in ed_text.char_indices() {
        while b >= run_end && ri + 1 < runs.len() {
            ri += 1;
            run_end = run_end.saturating_add(runs.get(ri).map_or(0, |r| r.len));
        }
        let mode = runs.get(ri).map_or(1, |r| txt2_mode(&r.style));
        match out.last_mut() {
            Some(last) if last.0 == mode => last.1 += ch.len_utf16(),
            _ => out.push((mode, ch.len_utf16())),
        }
    }
    out
}

/// `Txt2` style runs for `layer`, one per character run: `base` (one of the file's style runs,
/// `<< /0 << /0 << /6 style >> >> /1 length >>`, every key Photoshop wrote) with its style set
/// from the run's own: font (`/0`, an index into `fonts`, the block's `/0 /1 /0` font list, which
/// a missing font is appended to), size (`/1`), faux bold and italic, leading, scales and tracking
/// (`/2`–`/8`, EngineData's order), fill colour (`/53`), the OpenType keys and the auto-kern mode
/// (`/11`). Photoshop reads the character style from here rather than from EngineData, and quit
/// on a run holding only the auto-kern mode. `None` when `base` has no style dictionary or is
/// larger than a template may be (it is copied once per run).
fn txt2_style_runs(layer: &TextLayer, base: &E, fonts: &mut Vec<E>, dpi: f32) -> Option<Vec<E>> {
    if !matches!(base.path(&["0", "0", "6"]), Some(E::Dict(_))) || !fits(base, &mut { TEMPLATE_NODES }) {
        return None;
    }
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let runs = layer.char_runs();
    let ed_text = format!("{}\r", layer.text.replace('\n', "\r"));
    let mut spans: Vec<(usize, i64)> = Vec::new();
    let (mut ri, mut run_end) = (0usize, runs.first().map_or(0, |r| r.len));
    for (b, ch) in ed_text.char_indices() {
        while b >= run_end && ri + 1 < runs.len() {
            ri += 1;
            run_end = run_end.saturating_add(runs.get(ri).map_or(0, |r| r.len));
        }
        let n = ch.len_utf16() as i64;
        match spans.last_mut() {
            Some(last) if last.0 == ri => last.1 += n,
            _ => spans.push((ri, n)),
        }
    }
    let default = CharStyle::default();
    let mut out = Vec::with_capacity(spans.len());
    let mut looked_up = std::collections::HashMap::new();
    for (ri, n) in spans {
        let s = runs.get(ri).map_or(&default, |r| &r.style);
        let font = *looked_up.entry(postscript_for(s)).or_insert_with_key(|name| txt2_font(fonts, name));
        let mut run = base.clone();
        let style = run.get_mut("0")?.get_mut("0")?.get_mut("6")?;
        if let Some(font) = font {
            style.set("0", E::Int(font));
        }
        style.set("1", real(s.size_pt * k));
        style.set("2", E::Bool(s.faux_bold));
        style.set("3", E::Bool(s.faux_italic));
        style.set("4", E::Bool(s.leading_pt.is_none()));
        style.set("5", real(s.leading_pt.unwrap_or(0.0) * k));
        style.set("6", real(s.horizontal_scale));
        style.set("7", real(s.vertical_scale));
        style.set("8", E::Int(s.tracking.round() as i64));
        if let Some(paint @ E::Dict(_)) = style.get_mut("53").and_then(|p| p.get_mut("0")) {
            let (kind, values) = fill_color(&s.color);
            paint.set("0", E::Int(kind));
            paint.set("1", E::Array(values));
        }
        let feature = |tag: &str| s.features.iter().find(|f| f.tag == tag).map(|f| f.value > 0);
        for (key, tag) in TXT2_FEATURES {
            style.set(key, E::Bool(feature(tag).unwrap_or(tag == "calt")));
        }
        let sets = (0..20).filter(|n| feature(&format!("ss{:02}", n + 1)) == Some(true)).fold(0i64, |m, n| m | 1 << n);
        style.set(TXT2_STYLISTIC_SETS, E::Int(sets));
        style.set("11", E::Int(txt2_mode(s)));
        run.set("1", E::Int(n));
        out.push(run);
    }
    Some(out)
}

/// The index of the PostScript font `name` in a `Txt2` font list (`<< /0 << /99 /CoolTypeFont
/// /0 << /0 name /2 type >> >> >>` entries), appending it (shaped like the list's first entry)
/// when missing. `None` for an empty list: there is no entry to shape a new one on.
fn txt2_font(fonts: &mut Vec<E>, name: &str) -> Option<i64> {
    let named = |f: &E| f.path(&["0", "0", "0"]).and_then(E::as_str) == Some(name);
    if let Some(i) = fonts.iter().position(named) {
        return i64::try_from(i).ok();
    }
    let mut entry = fonts.first()?.clone();
    let font = entry.get_mut("0")?.get_mut("0")?;
    let kind = font.get("2").cloned();
    *font = E::Dict(vec![("0".into(), E::String(name.to_string()))]);
    if let Some(kind) = kind {
        font.set("2", kind);
    }
    fonts.push(entry);
    i64::try_from(fonts.len() - 1).ok()
}

/// The highest text-object number honoured here (`TextIndex`, and the slot array [`build_txt2`]
/// sizes). PSD text objects are numbered densely from 0; a crafted file can name object
/// `i32::MAX`, and sizing the array by it would make every save allocate gigabytes.
pub const MAX_TEXT_INDEX: i32 = 0xFFFF;

/// Whether the document `Txt2` block is needed for `objects`: only a non-Metrics auto-kern mode
/// (`Optical`) is exclusive to `Txt2` — EngineData's `AutoKerning` cannot distinguish Metrics from
/// Optical — so a fresh document of ordinary Metrics text has nothing to put in the block.
/// Writing Photoshop's empty `<< /0 << >> >>` stub anyway makes some readers (Photopea) fail
/// (#3514); the export omits the block for such a document.
pub fn needs_txt2(objects: &[(i32, &TextLayer)]) -> bool {
    objects.iter().any(|(_, l)| l.char_runs().iter().any(|r| r.style.kerning == Kerning::Optical))
}

/// The document's `Txt2` block for `objects`: `(TextIndex, layer)` pairs, one text object each
/// at the slot its `TextIndex` names (see [`set_text_index`] and [`text_index`]; unused slots are
/// empty dictionaries), carrying the automatic kerning mode of every pair (`/11`: 0 manual,
/// 1 metrics, 2 optical) — the one thing EngineData can't say (`AutoKerning true` covers Metrics
/// and Optical alike). PSD export writes this block for the type layers it writes `TySh` data
/// for; [`apply_txt2`] reads it back. Numbers outside `0..=`[`MAX_TEXT_INDEX`] are skipped: the
/// slot array is sized by real text objects, never by a file's numbers.
///
/// `previous` is the block this replaces (the file's own). A kept object — one whose text is
/// unchanged — keeps everything the file held beyond the two regenerated keys (its model's text
/// and style runs): Photoshop stores glyph pen positions under `/21 /1` there, and a save with
/// no text edit must not drop them. Its own style runs keep every style key too: only their
/// auto-kern mode is set (split where a mode changes inside a run). An edited object — the
/// file's object at that slot, its text changed — keeps the object too, with its text replaced
/// and its run arrays refitted to the new text (#6968: a bare stub crashed Photoshop); only its
/// glyph pen positions, which belong to the old text, are dropped. Everything
/// else the file's block held — the other keys beside the text objects, objects no layer names —
/// is kept, and a block whose content comes out unchanged is returned byte for byte (#2374:
/// Photoshop refused files whose `Txt2` an unedited save had stripped).
pub fn build_txt2(objects: &[(i32, &TextLayer)], previous: Option<&[u8]>) -> Vec<u8> {
    build_txt2_at(objects, previous, 72.0)
}

/// [`build_txt2`] for a document of `dpi` (the style runs of new and edited objects carry sizes
/// in the document's points, as EngineData does).
pub fn build_txt2_at(objects: &[(i32, &TextLayer)], previous: Option<&[u8]>, dpi: f32) -> Vec<u8> {
    let prev = previous.and_then(parse_txt2);
    let prev_objects: &[E] = prev.as_ref().and_then(|p| p.path(&["1", "1"]).and_then(E::as_array)).unwrap_or(&[]);
    let mut slots: Vec<E> = prev_objects.to_vec();
    // The older Photoshop's objects (#6971): named keys at `/DocumentObjects /TextObjects`
    // instead of the numbered `/1 /1`, one entry per layer by `TextIndex` position (the same
    // numbering the slots take), its text under `Model /Text` and its run lengths under `/Length`.
    // Photoshop re-renders a type layer from this copy when the file has one, so an edit that
    // left the old text here showed the old text back on reopen. The entries are only ever edited
    // in place, never grown: a layer the block has no entry for has no stale copy to fix.
    let mut td: Option<Vec<E>> = prev.as_ref().and_then(|p| p.path(&["DocumentObjects", "TextObjects"])).and_then(E::as_array).map(<[E]>::to_vec);
    let mut td_changed = false;
    let mut fonts: Vec<E> = prev.as_ref().and_then(|p| p.path(&["0", "1", "0"]).and_then(E::as_array)).map(<[E]>::to_vec).unwrap_or_default();
    let fonts_before = fonts.len();
    // Looked up once per save, not per layer: a crafted block with many objects must not cost
    // a pass over all of them for every type layer.
    let all_stubs = prev_objects.iter().all(is_stub);
    // A template is copied once per new layer, so a large one is skipped (the stub is written).
    let template = prev_objects.iter().rev().find(|o| names_one_frame(o) && fits(o, &mut { TEMPLATE_NODES }) && edited_object(o, "\r").is_some());
    for (index, layer) in objects {
        if !(0..=MAX_TEXT_INDEX).contains(index) {
            continue;
        }
        let text = format!("{}\r", layer.text.replace('\n', "\r"));
        let modes = txt2_mode_runs(layer);
        let at = *index as usize;
        // The older Photoshop's copy of this object (#6971), refitted like the numbered one
        // below. The entry's own text is the check, so a file a previous save left with a stale
        // copy beside a current `/1 /1` object is repaired here too.
        if let Some(entries) = td.as_mut() {
            let fixed = match entries.get(at) {
                Some(entry) if entry.path(&["Model", "Text"]).and_then(E::as_str) != Some(text.as_str()) => td_edited_object(entry, &text),
                _ => None,
            };
            if let Some(fixed) = fixed
                && let Some(slot) = entries.get_mut(at)
            {
                *slot = fixed;
                td_changed = true;
            }
        }
        // A stub an earlier save wrote beside Photoshop's objects counts as no object at all, so
        // it is replaced like a missing one (below) and re-saving such a file repairs it, whether
        // or not the layer's text changed.
        let slot = prev_objects.get(at).filter(|o| !is_stub(o) || all_stubs);
        let kept = slot.filter(|o| o.get("0").and_then(|m| m.get("0")).and_then(E::as_str) == Some(text.as_str()));
        // An edited object (#6968): the file's object at this slot with its text changed. It keeps
        // the file's structure — paragraph runs, the view naming its frame, the rest — with every
        // run array refitted to the new text. A bare `<< /0 << /0 text /6 runs >> >>` stub in
        // Photoshop's block (no paragraph runs, no view) made Photoshop quit while opening the file.
        let edited = kept.is_none().then(|| slot.and_then(|o| edited_object(o, &text))).flatten();
        // A new object (#7698): a layer the file's block has no object for (a type layer added
        // here) takes the shape of one of the file's own objects, refitted the same way, with its
        // view naming its own frame. A bare stub beside Photoshop's objects, its slot without a
        // frame in `/0 /8 /0`, made Photoshop and Photopea refuse the file.
        let fresh = (kept.is_none() && edited.is_none()).then(|| template.and_then(|t| new_object(t, at, &text))).flatten();
        let reuse = kept.is_some() || edited.is_some();
        let unchanged = kept.is_some();
        let mut object = kept.cloned().or(edited).or(fresh).unwrap_or_else(E::dict);
        let mut model = object.get("0").cloned().filter(|m| matches!(m, E::Dict(_))).unwrap_or_else(E::dict);
        // A new or edited object takes the layer's own style: Photoshop reads the character style
        // from these runs, so the template's (or the old text's) would show, and come back on
        // reopen. They are built on one of the object's own runs, every key Photoshop wrote kept.
        let styled = (!unchanged && !is_stub(&object))
            .then(|| model.path(&["6", "0"]).and_then(E::as_array).and_then(<[E]>::first).cloned())
            .flatten()
            .and_then(|base| txt2_style_runs(layer, &base, &mut fonts, dpi));
        let file_runs = reuse.then(|| model.path(&["6", "0"]).and_then(E::as_array)).flatten().and_then(|runs| with_modes(runs, &modes));
        let runs = styled.or(file_runs).unwrap_or_else(|| {
            modes
                .iter()
                .map(|&(mode, len)| {
                    // `<< /0 << /0 << /0 () /6 << /0 0 /11 mode >> >> >> /1 len >>`.
                    let style = E::Dict(vec![("0".into(), E::Int(0)), ("11".into(), E::Int(mode))]);
                    let data = E::Dict(vec![("0".into(), E::String(String::new())), ("6".into(), style)]);
                    E::Dict(vec![("0".into(), E::Dict(vec![("0".into(), data)])), ("1".into(), E::Int(len as i64))])
                })
                .collect()
        });
        model.set("0", E::String(text));
        match model.get_mut("6") {
            Some(style_run @ E::Dict(_)) => style_run.set("0", E::Array(runs)),
            _ => model.set("6", E::Dict(vec![("0".into(), E::Array(runs))])),
        }
        object.set("0", model);
        if slots.len() <= at {
            slots.resize(at + 1, E::dict());
        }
        if let Some(slot) = slots.get_mut(at) {
            *slot = object;
        }
    }
    // The outer dictionary keeps whatever the block held (`/98` version, block extras, …), and
    // `/1` whatever it holds beside the text objects.
    let mut outer = prev.clone().unwrap_or_else(|| E::Dict(vec![("98".into(), E::Dict(vec![("0".into(), E::Int(14))])), ("0".into(), E::dict())]));
    pad_frames(&mut outer, slots.len());
    if fonts.len() != fonts_before
        && let Some(list @ E::Dict(_)) = outer.get_mut("0").and_then(|v| v.get_mut("1"))
    {
        list.set("0", E::Array(fonts));
    }
    match outer.get_mut("1") {
        Some(objs @ E::Dict(_)) => objs.set("1", E::Array(slots)),
        _ => outer.set("1", E::Dict(vec![("1".into(), E::Array(slots))])),
    }
    // The older Photoshop's objects, whatever the loop fixed in them (an untouched list is left
    // exactly as the file has it, so the byte-for-byte return below still holds).
    if td_changed
        && let Some(entries) = td
        && let Some(objects @ E::Dict(_)) = outer.get_mut("DocumentObjects")
    {
        objects.set("TextObjects", E::Array(entries));
    }
    // Nothing changed: the file's own bytes (writing them back would re-encode strings and numbers).
    if let (Some(bytes), Some(p)) = (previous, &prev)
        && *p == outer
    {
        return bytes.to_vec();
    }
    match outer {
        E::Dict(items) => ed::write_bare(&items),
        // Unreachable: `parse_txt2` and the fallback above both yield a dictionary.
        _ => ed::write_bare(&[]),
    }
}

/// The most values a `Txt2` object may hold to serve as the template for new objects. Photoshop's
/// own objects hold a few hundred (611 at most in the file from #7698).
const TEMPLATE_NODES: usize = 4096;

/// Cumulative node and byte budgets across all run dictionaries cloned while refitting one `Txt2`
/// object (`runs_per` and `runs_to_length`), so an edited object with a large preserved run and
/// many output paragraphs falls back to a fresh object instead of amplifying memory.
const MAX_REFIT_RUN_NODES: usize = 16_384;
const MAX_REFIT_RUN_BYTES: usize = 1 << 20;

/// Whether `v` holds at most `budget` values, counting down `budget` (stops at the first one over).
fn fits(v: &E, budget: &mut usize) -> bool {
    let Some(left) = budget.checked_sub(1) else { return false };
    *budget = left;
    match v {
        E::Dict(items) => items.iter().all(|(_, x)| fits(x, budget)),
        E::Array(items) => items.iter().all(|x| fits(x, budget)),
        _ => true,
    }
}

/// Whether `v` fits within the remaining `(nodes, bytes)` budget, deducting its nodes and string
/// bytes as it walks.
fn fits_budget(v: &E, nodes: &mut usize, bytes: &mut usize) -> bool {
    let Some(left) = nodes.checked_sub(1) else { return false };
    *nodes = left;
    match v {
        E::Dict(items) => items.iter().all(|(k, x)| {
            let Some(rem) = bytes.checked_sub(k.len()) else { return false };
            *bytes = rem;
            fits_budget(x, nodes, bytes)
        }),
        E::Array(items) => items.iter().all(|x| fits_budget(x, nodes, bytes)),
        E::String(s) | E::Name(s) => {
            let Some(rem) = bytes.checked_sub(s.len()) else { return false };
            *bytes = rem;
            true
        }
        _ => true,
    }
}

/// Whether the text object `object` has a view naming one frame (`/1 /0 [ << /0 n >> ]`), as
/// Photoshop writes it.
fn names_one_frame(object: &E) -> bool {
    object.path(&["1", "0"]).and_then(E::as_array).is_some_and(|f| f.len() == 1 && f.first().and_then(|x| x.get("0")).is_some())
}

/// A text object for the slot `at`, which the file's block has none for: the file's object
/// `template` refitted to `text` by [`edited_object`], its view now naming frame `at` and its view
/// runs (`/1 /1 /0`, one per paragraph) remade for the new paragraphs. `None` when the template
/// can't be refitted.
fn new_object(template: &E, at: usize, text: &str) -> Option<E> {
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let frame = template.path(&["1", "0"]).and_then(E::as_array).and_then(|f| f.first()).and_then(|f| f.get("0")).cloned();
    let at = i64::try_from(at).ok()?;
    let mut object = edited_object(template, text)?;
    let view = object.get_mut("1")?;
    view.set("0", E::Array(vec![E::Dict(vec![("0".into(), E::Int(at))])]));
    // The rendered strikes' frame strike (`/F`) names the template's frame too.
    if let Some(E::Array(strikes)) = view.get_mut("2") {
        for s in strikes.iter_mut() {
            let Some(E::Array(parts)) = s.get_mut("6") else { continue };
            for p in parts.iter_mut().filter(|p| p.get("99").and_then(E::as_str) == Some("F") && p.get("10") == frame.as_ref()) {
                p.set("10", E::Int(at));
            }
        }
    }
    if let Some(runs) = view.path(&["1", "0"]).and_then(E::as_array) {
        let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
        let fitted = runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?;
        view.get_mut("1")?.set("0", E::Array(fitted));
    }
    Some(object)
}

/// Whether `object` is the bare `<< /0 << /0 text /6 runs >> >>` object [`build_txt2`] writes
/// when it has nothing to take a shape from: no view, no paragraph runs.
fn is_stub(object: &E) -> bool {
    let only = |v: &E, keys: &[&str]| matches!(v, E::Dict(items) if items.iter().all(|(k, _)| keys.contains(&k.as_str())));
    only(object, &["0"]) && object.get("0").is_some_and(|m| only(m, &["0", "6"]))
}

/// Gives the block `outer` a frame (`/0 /8 /0`) for each of its `count` text objects: Photoshop
/// writes one per object, and a reader indexes them by object number. Missing frames are the
/// small point-text frame Photoshop writes (`<< /0 << /2 << /6 [ -1.0 -1.0 ] /11 << /4 -1 /7
/// false >> >> >> >>`), never copies of a file's frame: the slot count reaches
/// [`MAX_TEXT_INDEX`], and a large frame copied that often would blow up the save. A block
/// without a frame list is left as it is.
fn pad_frames(outer: &mut E, count: usize) {
    let Some(E::Array(frames)) = outer.get_mut("0").and_then(|v| v.get_mut("8")).and_then(|v| v.get_mut("0")) else { return };
    let count = count.min(MAX_TEXT_INDEX as usize + 1);
    if frames.len() < count {
        let kind = E::Dict(vec![("4".into(), E::Int(-1)), ("7".into(), E::Bool(false))]);
        let data = E::Dict(vec![("6".into(), E::Array(vec![E::Real(-1.0), E::Real(-1.0)])), ("11".into(), kind)]);
        let frame = E::Dict(vec![("0".into(), E::Dict(vec![("2".into(), data)]))]);
        frames.resize(count, frame);
    }
}

/// The file's `Txt2` text object `object` (`<< /0 model /1 view … >>`) for the changed engine
/// text `text` (`\r` breaks, trailing `\r`): the model's text replaced, its paragraph runs (`/5`)
/// one per paragraph of the new text (each keeping the style of the file's paragraph at that
/// position, else its last), and every other run array of the model (style runs `/6`, …) cut or
/// extended at its end to the new length. The view and the other keys are kept, except the glyph
/// pen positions (`/21`), which belong to the old text. `None` when the object or one of its run
/// arrays isn't shaped as Photoshop writes it: the caller writes a fresh object instead.
fn edited_object(object: &E, text: &str) -> Option<E> {
    let mut object = object.clone();
    let E::Dict(items) = &mut object else { return None };
    items.retain(|(k, _)| k != "21");
    let model = object.get_mut("0").filter(|m| matches!(m, E::Dict(_)))?;
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let E::Dict(entries) = model else { return None };
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "0" {
            *value = E::String(text.to_string());
            continue;
        }
        let Some(runs) = value.get("0").and_then(E::as_array).filter(|r| r.iter().all(|x| matches!(x, E::Dict(_)) && x.get("1").is_some())) else {
            continue;
        };
        let fitted = if key.as_str() == "5" {
            runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?
        } else {
            runs_to_length(runs, total, "1", &mut nodes, &mut bytes)?
        };
        value.set("0", E::Array(fitted));
    }
    Some(object)
}

/// The older Photoshop's `Txt2` text object `object` (`<< /Model << /Text … /ParagraphRun … >>
/// /View … >>`) for the changed engine text `text` (`\r` breaks, trailing `\r`): the model's text
/// replaced and every run array of the model refitted the way [`edited_object`] refits the
/// numbered objects' — the paragraph runs (`/ParagraphRun`) one per paragraph of the new text, the
/// others (`/StyleRun`, `/KernRun`, …) cut or extended to the new total. The model's other keys,
/// the view and the rest are kept: these blocks (the objects at `/DocumentObjects /TextObjects`
/// that older Photoshop versions write and re-render from, #6971) name their keys and number
/// their runs with `/Length` in UTF-16 units instead of `/1`. `None` when the object or one of
/// its run arrays isn't shaped as Photoshop writes it: the caller then leaves the entry as the
/// file has it, rather than write text its runs don't cover (#6968).
fn td_edited_object(object: &E, text: &str) -> Option<E> {
    let mut object = object.clone();
    let model = object.get_mut("Model").filter(|m| matches!(m, E::Dict(_)))?;
    let E::Dict(entries) = model else { return None };
    // The text must be there to replace: refitted runs over a text the model doesn't hold would
    // be a half-edited object.
    if !entries.iter().any(|(k, v)| k == "Text" && matches!(v, E::String(_))) {
        return None;
    }
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "Text" {
            *value = E::String(text.to_string());
            continue;
        }
        if !matches!(value, E::Dict(_)) {
            continue;
        }
        // A model dict without `RunArray` isn't a run array (the view's keys live elsewhere), so
        // it is skipped; a named `RunArray` is unambiguous — malformed, the whole entry is left to
        // the file rather than half-updated.
        let Some(runs) = value.get("RunArray") else { continue };
        let fitted = match runs {
            E::Array(runs) if runs.iter().all(|x| matches!(x, E::Dict(_)) && x.get("Length").is_some()) => {
                if key.as_str() == "ParagraphRun" {
                    runs_per(runs, &paragraphs, "Length", &mut nodes, &mut bytes)?
                } else {
                    runs_to_length(runs, total, "Length", &mut nodes, &mut bytes)?
                }
            }
            _ => return None,
        };
        value.set("RunArray", E::Array(fitted));
    }
    Some(object)
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) remade with the lengths `lens`:
/// run i keeps the content of the file's run i, else of its last one. `None` for an empty `runs`
/// or when the cumulative `nodes` / `bytes` budget is exceeded.
fn runs_per(runs: &[E], lens: &[usize], len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let last = runs.last()?;
    lens.iter()
        .enumerate()
        .map(|(i, &len)| {
            let src = runs.get(i).unwrap_or(last);
            if !fits_budget(src, nodes, bytes) {
                return None;
            }
            let mut r = src.clone();
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) covering `total` UTF-16 units:
/// the leading runs as they were, cut where `total` ends, the last kept run extended when `total`
/// is longer. Empty runs are dropped. `None` when `runs` is empty, a length isn't a count, or the
/// cumulative `nodes` / `bytes` budget is exceeded.
fn runs_to_length(runs: &[E], total: usize, len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let mut out: Vec<(E, usize)> = Vec::new();
    let mut left = total;
    for r in runs {
        let len = r.get(len_key).and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())?;
        let take = len.min(left);
        if take > 0 {
            if !fits_budget(r, nodes, bytes) {
                return None;
            }
            out.push((r.clone(), take));
            left -= take;
        }
    }
    if left > 0 {
        match out.last_mut() {
            Some(last) => last.1 = last.1.checked_add(left)?,
            None => {
                let last = runs.last()?;
                if !fits_budget(last, nodes, bytes) {
                    return None;
                }
                out.push((last.clone(), left));
            }
        }
    }
    out.into_iter()
        .map(|(mut r, len)| {
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// The file's own `Txt2` style runs `runs` with the auto-kern modes `modes` (`(mode, UTF-16
/// length)`, see [`txt2_mode_runs`]) set at `/0 /0 /6 /11`, keeping every other style key. A
/// run a mode change falls inside is split there. `None` when the runs don't cover the text or
/// lack a style dictionary: the caller writes its own runs instead.
fn with_modes(runs: &[E], modes: &[(i64, usize)]) -> Option<Vec<E>> {
    let lens: Vec<usize> = runs.iter().map(|r| r.get("1").and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())).collect::<Option<_>>()?;
    let total = modes.iter().try_fold(0usize, |a, m| a.checked_add(m.1))?;
    if lens.iter().try_fold(0usize, |a, l| a.checked_add(*l))? != total {
        return None;
    }
    let mut out = Vec::with_capacity(runs.len());
    let (mut mi, mut mode_end) = (0usize, modes.first()?.1);
    let mut at = 0usize;
    for (run, len) in runs.iter().zip(lens) {
        if len == 0 {
            out.push(run.clone());
            continue;
        }
        let end = at + len;
        while at < end {
            while at >= mode_end && mi + 1 < modes.len() {
                mi += 1;
                mode_end = mode_end.saturating_add(modes.get(mi)?.1);
            }
            let piece_end = end.min(mode_end);
            if piece_end <= at {
                return None;
            }
            let mut piece = run.clone();
            if piece_end - at != len {
                piece.set("1", E::Int(i64::try_from(piece_end - at).ok()?));
            }
            let style = piece.get_mut("0").and_then(|v| v.get_mut("0")).and_then(|v| v.get_mut("6")).filter(|v| matches!(v, E::Dict(_)))?;
            let mode = modes.get(mi)?.0;
            // Keep the file's value (and its number type) when the mode is the same.
            if style.get("11").and_then(E::as_i64) != Some(mode) {
                style.set("11", E::Int(mode));
            }
            out.push(piece);
            at = piece_end;
        }
    }
    Some(out)
}

/// The layer's text-object number in the document's `Txt2` block, when its `TySh` names one
/// within `0..=`[`MAX_TEXT_INDEX`]. PSD export keeps a preserved number — its regenerated `Txt2`
/// object sits at that slot, and [`apply_txt2`] looks the slot up — and numbers layers without
/// one (or with a file-controlled out-of-range one) afresh.
pub fn text_index(tysh: &[u8]) -> Option<i32> {
    let t = parse_tysh(tysh)?;
    match t.text.get("TextIndex") {
        Some(D::Integer(i)) if (0..=MAX_TEXT_INDEX).contains(i) => Some(*i),
        _ => None,
    }
}

/// How many text objects the document `Txt2` block `data` holds (`/1 /1`), named by a layer or
/// not; 0 when it doesn't parse. PSD export numbers new type layers past them, so a new layer
/// never lands on a deleted layer's object.
pub fn txt2_object_count(data: &[u8]) -> usize {
    parse_txt2(data).and_then(|t| t.path(&["1", "1"]).and_then(E::as_array).map(<[E]>::len)).unwrap_or(0)
}

/// Rewrites a `TySh`'s `TextIndex`: the layer's text-object number in the document's `Txt2`
/// block ([`build_txt2`] writes them in this order). `None` when `tysh` doesn't parse.
pub fn set_text_index(tysh: &[u8], index: i32) -> Option<Vec<u8>> {
    let mut t = parse_tysh(tysh)?;
    match t.text.items.iter_mut().find(|(k, _)| k.is("TextIndex")) {
        Some(e) => e.1 = D::Integer(index),
        // Where Photoshop writes it: right before `EngineData`.
        None => {
            let at = t.text.items.iter().position(|(k, _)| k.is("EngineData")).unwrap_or(t.text.items.len());
            t.text.items.insert(at, (Id::new("TextIndex"), D::Integer(index)));
        }
    }
    Some(write_tysh(&t))
}

/// Updates (or creates) an EngineData tree for `layer`.
pub fn build_engine_data(layer: &TextLayer, template: Option<E>, dpi: f32) -> E {
    let k = 72.0 / if dpi.is_finite() && dpi > 0.0 { dpi } else { 72.0 };
    let fresh = || E::Dict(vec![("EngineDict".into(), E::dict()), ("ResourceDict".into(), E::dict())]);
    // The template comes from the PSD: it may not be a dictionary or may lack `EngineDict`.
    let mut e = match template {
        Some(t @ E::Dict(_)) => t,
        _ => fresh(),
    };
    if !matches!(e.get("EngineDict"), Some(E::Dict(_))) {
        e.set("EngineDict", E::dict());
    }
    let runs = layer.char_runs();
    let paras = layer.paragraph_runs();
    let text = layer.text.replace('\n', "\r");
    let ed_text = format!("{text}\r");

    // Fonts: keep the template's font list and append new PostScript names.
    let mut fonts: Vec<String> = e
        .path(&["ResourceDict", "FontSet"])
        .and_then(E::as_array)
        .map(|a| a.iter().map(|f| f.get("Name").and_then(E::as_str).unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let mut font_index = |ps: String| match fonts.iter().position(|f| *f == ps) {
        Some(i) => i,
        None => {
            fonts.push(ps);
            fonts.len() - 1
        }
    };
    let mut sruns = Vec::new();
    let mut slens = Vec::new();
    for (ri, pair, n) in pair_runs(&ed_text, &runs) {
        let Some(r) = runs.get(ri) else { continue };
        let fi = font_index(postscript_for(&r.style));
        let mut data = style_sheet_data(&r.style, fi, k);
        data.set("AutoKerning", E::Bool(pair.0));
        data.set("Kerning", E::Int(pair.1));
        sruns.push(E::Dict(vec![("StyleSheet".into(), E::Dict(vec![("StyleSheetData".into(), data)]))]));
        slens.push(E::Int(n as i64));
    }
    let mut pruns = Vec::new();
    let mut plens = Vec::new();
    let mut at = 0usize;
    for (i, p) in paras.iter().enumerate() {
        let piece = &text[at..at + p.len];
        at += p.len;
        let mut n = utf16_len(piece);
        if i + 1 == paras.len() {
            n += 1;
        }
        pruns.push(E::Dict(vec![(
            "ParagraphSheet".into(),
            E::Dict(vec![("DefaultStyleSheet".into(), E::Int(0)), ("Properties".into(), paragraph_properties(&p.style, k))]),
        )]));
        plens.push(E::Int(n));
    }

    let Some(dict) = e.get_mut("EngineDict") else {
        return fresh();
    };
    let mut editor = dict.get("Editor").cloned().unwrap_or_else(E::dict);
    editor.set("Text", E::String(ed_text));
    dict.set("Editor", editor);
    let mut pr = dict.get("ParagraphRun").cloned().unwrap_or_else(E::dict);
    pr.set("RunArray", E::Array(pruns));
    pr.set("RunLengthArray", E::Array(plens));
    if pr.get("IsJoinable").is_none() {
        pr.set("IsJoinable", E::Int(1));
    }
    dict.set("ParagraphRun", pr);
    let mut sr = dict.get("StyleRun").cloned().unwrap_or_else(E::dict);
    sr.set("RunArray", E::Array(sruns));
    sr.set("RunLengthArray", E::Array(slens));
    if sr.get("IsJoinable").is_none() {
        sr.set("IsJoinable", E::Int(2));
    }
    dict.set("StyleRun", sr);
    dict.set(
        "AntiAlias",
        E::Int(match layer.antialias {
            AntiAlias::None => 0,
            AntiAlias::Crisp => 1,
            AntiAlias::Strong => 2,
            AntiAlias::Smooth => 3,
            AntiAlias::Sharp => 4,
            // No Photoshop file confirms the platform modes' numbers here; the `AntA` descriptor
            // carries the exact mode, and Smooth is the closest.
            AntiAlias::Windows | AntiAlias::WindowsLcd => 3,
        }),
    );
    if dict.get("UseFractionalGlyphWidths").is_none() {
        dict.set("UseFractionalGlyphWidths", E::Bool(true));
    }
    let (shape_type, box_bounds) = match layer.shape {
        // PSD path type is not written yet: the layer saves as point text.
        TextShape::Point | TextShape::Path { .. } => (0, None),
        TextShape::Box { x, y, width, height } => (1, Some([x, y, x + width, y + height])),
    };
    let mut photoshop = E::Dict(vec![("ShapeType".into(), E::Int(shape_type))]);
    match box_bounds {
        None => photoshop.set("PointBase", E::Array(vec![real(0.0), real(0.0)])),
        Some(b) => photoshop.set("BoxBounds", E::Array(b.iter().map(|v| real(*v)).collect())),
    }
    photoshop.set(
        "Base",
        E::Dict(vec![
            ("ShapeType".into(), E::Int(shape_type)),
            ("TransformPoint0".into(), E::Array(vec![real(1.0), real(0.0)])),
            ("TransformPoint1".into(), E::Array(vec![real(0.0), real(1.0)])),
            ("TransformPoint2".into(), E::Array(vec![real(0.0), real(0.0)])),
        ]),
    );
    // EngineData writing direction and procession, as Photoshop writes vertical type: 2 and 1
    // (horizontal: 0 and 0). The descriptor's `Ornt` agrees.
    let writing = if layer.orientation == Orientation::Vertical { 2 } else { 0 };
    let child = E::Dict(vec![
        ("ShapeType".into(), E::Int(shape_type)),
        ("Procession".into(), E::Int(i64::from(writing == 2))),
        ("Lines".into(), E::Dict(vec![("WritingDirection".into(), E::Int(writing)), ("Children".into(), E::Array(vec![]))])),
        ("Cookie".into(), E::Dict(vec![("Photoshop".into(), photoshop)])),
    ]);
    dict.set(
        "Rendered",
        E::Dict(vec![
            ("Version".into(), E::Int(1)),
            ("Shapes".into(), E::Dict(vec![("WritingDirection".into(), E::Int(writing)), ("Children".into(), E::Array(vec![child]))])),
        ]),
    );

    for key in ["ResourceDict", "DocumentResources"] {
        if key == "DocumentResources" && e.get(key).is_none() {
            let rd = e.get("ResourceDict").cloned().unwrap_or_else(E::dict);
            e.set(key, rd);
            continue;
        }
        let Some(rd) = e.get_mut(key) else { continue };
        let fontset = font_set(&fonts, rd.get("FontSet"));
        rd.set("FontSet", fontset);
        if rd.get("StyleSheetSet").is_none() {
            let base = runs.first().map(|r| r.style.clone()).unwrap_or_default();
            rd.set(
                "StyleSheetSet",
                E::Array(vec![E::Dict(vec![("Name".into(), E::String("Normal RGB".into())), ("StyleSheetData".into(), style_sheet_data(&base, 0, k))])]),
            );
        }
        if rd.get("ParagraphSheetSet").is_none() {
            rd.set(
                "ParagraphSheetSet",
                E::Array(vec![E::Dict(vec![
                    ("Name".into(), E::String("Normal RGB".into())),
                    ("DefaultStyleSheet".into(), E::Int(0)),
                    ("Properties".into(), paragraph_properties(&ParagraphStyle::default(), k)),
                ])]),
            );
        }
        for (k2, v) in [("TheNormalStyleSheet", 0), ("TheNormalParagraphSheet", 0)] {
            if rd.get(k2).is_none() {
                rd.set(k2, E::Int(v));
            }
        }
    }
    e
}

/// The anti-aliasing mode for EngineData's `AntiAlias` number, as Photoshop writes it: 0 None,
/// 1 Crisp, 2 Strong, 3 Smooth, 4 Sharp. Other numbers (platform modes) are left to `AntA`.
fn engine_antialias(n: f64) -> Option<AntiAlias> {
    Some(match n as i64 {
        0 => AntiAlias::None,
        1 => AntiAlias::Crisp,
        2 => AntiAlias::Strong,
        3 => AntiAlias::Smooth,
        4 => AntiAlias::Sharp,
        _ => return None,
    })
}

/// Builds the `TySh` block for `layer`, patching its current `psd_raw` when present.
/// `ink` is the layer's rendered bounds in text space (for `bounds`/`boundingBox`).
pub fn build_tysh(layer: &TextLayer, dpi: f32, ink: Option<[f32; 4]>) -> Vec<u8> {
    let old = layer.psd_raw.as_deref().and_then(|d| parse_tysh(d));
    let template = old.as_ref().and_then(|t| engine_data(&t.text));
    let e = build_engine_data(layer, template, dpi);
    let mut text = old.as_ref().map(|t| t.text.clone()).unwrap_or_else(|| Descriptor::new("TxLr"));
    let set = |d: &mut Descriptor, key: &str, v: D| match d.items.iter_mut().find(|(k, _)| k.is(key)) {
        Some(e) => e.1 = v,
        None => d.items.push((Id::new(key), v)),
    };
    set(&mut text, "Txt ", D::Text(UnicodeString::new_nul(&layer.text.replace('\n', "\r"))));
    if text.get("textGridding").is_none() {
        set(&mut text, "textGridding", enumv("textGridding", "None"));
    }
    set(&mut text, "Ornt", enumv("Ornt", if layer.orientation == Orientation::Vertical { "Vrtc" } else { "Hrzn" }));
    set(
        &mut text,
        "AntA",
        enumv(
            "Annt",
            match layer.antialias {
                AntiAlias::None => "Anno",
                AntiAlias::Sharp => "antiAliasSharp",
                AntiAlias::Crisp => "AnCr",
                AntiAlias::Strong => "AnSt",
                AntiAlias::Smooth => "AnSm",
                AntiAlias::Windows => "antiAliasPlatformGray",
                AntiAlias::WindowsLcd => "antiAliasPlatformLCD",
            },
        ),
    );
    let [l, t, r, b] = match (&layer.shape, ink) {
        (&TextShape::Box { x, y, width, height }, _) => [x, y, x + width, y + height],
        (_, Some(i)) => i,
        _ => [0.0; 4],
    };
    let rect = |cls: &str| {
        D::Descriptor(
            Descriptor::new(cls)
                .with("Left", D::UnitFloat { unit: *b"#Pnt", value: f64::from(l) })
                .with("Top ", D::UnitFloat { unit: *b"#Pnt", value: f64::from(t) })
                .with("Rght", D::UnitFloat { unit: *b"#Pnt", value: f64::from(r) })
                .with("Btom", D::UnitFloat { unit: *b"#Pnt", value: f64::from(b) }),
        )
    };
    set(&mut text, "bounds", rect("bounds"));
    set(&mut text, "boundingBox", rect("boundingBox"));
    set(&mut text, "EngineData", D::RawData(ed::write(&e)));
    let warp = match (&layer.warp, old.as_ref().and_then(|o| o.warp.clone())) {
        (Some(w), _) => Some(
            Descriptor::new("warp")
                .with("warpStyle", enumv("warpStyle", &w.style))
                .with("warpValue", D::Double(f64::from(w.value)))
                .with("warpPerspective", D::Double(f64::from(w.horizontal_distortion)))
                .with("warpPerspectiveOther", D::Double(f64::from(w.vertical_distortion)))
                .with("warpRotate", enumv("Ornt", if w.horizontal { "Hrzn" } else { "Vrtc" })),
        ),
        (None, Some(old)) if enum_value(&old, "warpStyle").as_deref() == Some("warpNone") => Some(old),
        _ => None,
    };
    // Photoshop's trailing bounds: zeros for unwarped type, the text-space bounds when warped.
    let warped = warp.as_ref().is_some_and(|w| enum_value(w, "warpStyle").is_some_and(|s| s != "warpNone"));
    let bounds = if warped { [l, t, r, b] } else { [0.0; 4] };
    write_tysh(&TySh { transform: layer.transform, text, warp, bounds })
}

#[cfg(test)]
mod run_length_safety_tests {
    use super::*;

    #[test]
    fn utf16_lengths_stop_after_text_is_consumed() {
        assert_eq!(utf16_to_byte_lengths("é😀", &[1.0, 2.0, 1.0, 1.0e300]), [2, 4]);
        assert_eq!(utf16_to_byte_lengths("", &[1.0, 1.0e300]), [0]);
        assert_eq!(utf16_to_byte_lengths("abc", &[0.0, 1.0, 0.0, 1.0, 1.0, 1.0e300]), [0, 1, 0, 1, 1]);
        let zero_runs = vec![0.0; MAX_ENGINE_RUNS + 1];
        assert_eq!(utf16_to_byte_lengths("a", &zero_runs).len(), MAX_ENGINE_RUNS);
    }

    #[test]
    fn engine_run_length_array_conversion_is_bounded() {
        let values = E::Array(vec![E::Real(0.0); MAX_ENGINE_RUNS + 1]);
        assert_eq!(arr_f(Some(&values)).len(), MAX_ENGINE_RUNS);
    }
}

#[cfg(test)]
mod transform_safety_tests {
    use super::*;

    #[test]
    fn unsafe_affine_terms_are_not_editable_type() {
        let mut t = TySh { transform: Affine::IDENTITY, text: Descriptor::new("TxLr"), warp: None, bounds: [0.0; 4] };
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.315776645269349e62, 1.4759871577491642e307] {
            for i in 0..6 {
                t.transform = Affine::IDENTITY;
                t.transform.m[i] = bad;
                assert!(parse_tysh(&write_tysh(&t)).is_none(), "term {i}: {bad}");
            }
        }
        t.transform = Affine { m: [0.0, -2.01242653341062, 2.286521576443544, 0.0, 586.63, 2452.60] };
        assert_eq!(parse_tysh(&write_tysh(&t)).unwrap().transform, t.transform);
    }
}

#[cfg(test)]
mod nonfinite_tests {
    use super::*;
    use crate::engine_data::parse;

    #[test]
    fn non_finite_style_numbers_fall_back_to_defaults() {
        let d = parse(b"<< /FontSize 1e999 /Tracking nan /HorizontalScale inf /Kerning -inf /FillColor << /Values [1.0 nan 0.0 0.0] >> >>").unwrap();
        let s = char_style(&[], &d, &[], 1.0);
        let dflt = CharStyle::default();
        assert_eq!(s.size_pt, dflt.size_pt);
        assert_eq!((s.tracking, s.horizontal_scale, s.kern), (0.0, 1.0, 0.0));
        assert_eq!(s.color, Color::BLACK);
        // A finite f64 too big for f32 is dropped as well.
        let d = parse(b"<< /FontSize 1e300 /StartIndent inf >> ").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).size_pt, dflt.size_pt);
        assert_eq!(para_style(&[], &d, 1.0).start_indent_pt, 0.0);
        let d = parse(b"<< /FontSize 24.0 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).size_pt, 24.0);
    }

    /// #5645: `FontBaseline` (1 superscript, 2 subscript) is read and written back.
    #[test]
    fn font_baseline_round_trips() {
        use photocraft_doc::text::FontBaseline;
        let d = parse(b"<< /FontBaseline 2 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).baseline, FontBaseline::Subscript);
        let d = parse(b"<< /FontBaseline 1 >>").unwrap();
        assert_eq!(char_style(&[], &d, &[], 1.0).baseline, FontBaseline::Superscript);
        let s = CharStyle { baseline: FontBaseline::Subscript, ..Default::default() };
        let back = char_style(&[], &style_sheet_data(&s, 0, 1.0), &[], 1.0);
        assert_eq!(back.baseline, FontBaseline::Subscript);
    }

    #[test]
    fn warp_values_beyond_f32_clamp_to_the_warp_range() {
        let warp = Descriptor::new("warp")
            .with("warpStyle", enumv("warpStyle", "warpArc"))
            .with("warpValue", D::Double(1e39))
            .with("warpPerspective", D::Double(-1e39))
            .with("warpPerspectiveOther", D::Double(250.0))
            .with("warpRotate", enumv("Ornt", "Hrzn"));
        let tysh = write_tysh(&TySh { transform: Affine::IDENTITY, text: Descriptor::new("TxLr"), warp: Some(warp), bounds: [0.0; 4] });
        let w = text_layer_from_tysh(&tysh, 72.0).and_then(|l| l.warp).unwrap();
        assert_eq!((w.value, w.horizontal_distortion, w.vertical_distortion), (100.0, -100.0, 100.0));
    }

    #[test]
    fn infinite_dpi_does_not_write_a_zero_size() {
        let layer =
            TextLayer { text: "x".into(), runs: vec![TextRun { len: 1, style: CharStyle { size_pt: 12.0, ..Default::default() } }], ..Default::default() };
        let out = String::from_utf8_lossy(&crate::engine_data::write(&build_engine_data(&layer, None, f32::INFINITY))).into_owned();
        assert!(out.contains("/FontSize 12.0"), "{out}");
    }
}
