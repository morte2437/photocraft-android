use super::build::engine_antialias;
use super::{
    Affine, AntiAlias, Caps, CharStyle, Color, ColorMode, D, Descriptor, E, FontBaseline, Kerning, Orientation, ParagraphRun, ParagraphStyle, Point, TextAlign,
    TextLayer, TextRun, TextShape, TextWarp, TySh, dnum, ed, engine_data, enum_value, guess_from_postscript, parse_tysh,
};

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
    // The numbered object, else the older Photoshop's at `/DocumentObjects /TextObjects` (named
    // keys: run lengths under `Length`, the style under `RunData /StyleSheet /Features`, the
    // auto-kern mode under `AutoKern`).
    let numbered = txt2
        .path(&["1", "1"])
        .and_then(E::as_array)
        .and_then(|a| a.get(index))
        .and_then(|o| o.get("0"))
        .and_then(|model| Some((model.get("0").and_then(E::as_str)?, model.path(&["6", "0"]).and_then(E::as_array)?)));
    let legacy = || {
        let model = txt2.path(&["DocumentObjects", "TextObjects"]).and_then(E::as_array).and_then(|a| a.get(index))?.get("Model")?;
        Some((model.get("Text").and_then(E::as_str)?, model.path(&["StyleRun", "RunArray"]).and_then(E::as_array)?))
    };
    let (text, style_runs, is_numbered) = match numbered {
        Some((text, runs)) => (text, runs, true),
        None => {
            let Some((text, runs)) = legacy() else { return };
            (text, runs, false)
        }
    };
    if text.strip_suffix('\r').unwrap_or(text).replace('\r', "\n") != layer.text {
        return;
    }
    let (len_key, style_path, kern_key): (&str, &[&str], &str) =
        if is_numbered { ("1", &["0", "0", "6"], "11") } else { ("Length", &["RunData", "StyleSheet", "Features"], "AutoKern") };
    let styles: Vec<(f64, Option<&E>)> = style_runs.iter().map(|r| (r.get(len_key).and_then(E::as_f64).unwrap_or(0.0), r.path(style_path))).collect();
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
            match txt2_style.and_then(|st| st.get(kern_key)).and_then(E::as_i64) {
                Some(0) => style.kerning = Kerning::Off,
                Some(1) => style.kerning = Kerning::Metrics,
                Some(2) => style.kerning = Kerning::Optical,
                _ => {}
            }
            if let Some(st) = txt2_style.filter(|_| is_numbered) {
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

/// The frame (`Txt2 /0 /8 /0` entry) of text object `index`: its rendered frame's `/10` names it
/// (with `named`, else the frame its view names; see [`object_frame`]).
fn txt2_frame(txt2: &E, index: usize, named: bool) -> Option<&E> {
    let object = txt2.path(&["1", "1"]).and_then(E::as_array)?.get(index)?;
    txt2.path(&["0", "8", "0"]).and_then(E::as_array)?.get(object_frame(object, named)?)?.get("0")
}

/// The number of the frame the text object `object` is drawn in: its rendered frame's `/10`, else
/// (with `named`) the frame its view names (`/1 /0 [ << /0 n >> ]`). Photoshop leaves the rendered
/// strikes out for type it did not lay out (`text/path-wave-open.psd`, whose EngineData text is
/// blank).
pub(super) fn object_frame(object: &E, named: bool) -> Option<usize> {
    let rendered = object
        .path(&["1", "2"])
        .and_then(E::as_array)
        .and_then(|s| s.first())
        .and_then(|s| s.get("6"))
        .and_then(E::as_array)
        .and_then(|parts| parts.iter().find_map(|f| f.get("10").and_then(E::as_i64)));
    let view = || named.then(|| object.path(&["1", "0"]).and_then(E::as_array).and_then(|f| f.first()).and_then(|f| f.get("0")).and_then(E::as_i64)).flatten();
    usize::try_from(rendered.or_else(view)?).ok()
}

/// The `TySh` of `tysh` and the `Txt2` frame of its text object (`named`: see [`object_frame`]),
/// when that object still holds `layer`'s text (an edit elsewhere leaves `Txt2` stale).
fn layer_frame<'a>(layer: &TextLayer, tysh: &[u8], txt2: &'a E, named: bool) -> Option<(TySh, &'a E)> {
    let t = parse_tysh(tysh)?;
    let index = match t.text.get("TextIndex") {
        Some(D::Integer(i)) => usize::try_from(*i).ok()?,
        _ => return None,
    };
    let text = txt2.path(&["1", "1"]).and_then(E::as_array).and_then(|a| a.get(index)).and_then(|o| o.path(&["0", "0"])).and_then(E::as_str)?;
    if text.strip_suffix('\r').unwrap_or(text).replace('\r', "\n") != layer.text {
        return None;
    }
    let frame = txt2_frame(txt2, index, named)?;
    Some((t, frame))
}

/// Puts `layer` on its path when its `Txt2` frame is type on a path (#7962): Photoshop keeps the
/// path only there, and EngineData describes the layer as a text box the size of the path's
/// bounds. The path is read into the `TySh` text space, the type starting where Photoshop starts
/// it (see `psd_path`). `Ok(true)` when the layer is now path text, `Ok(false)` when its frame is
/// no path (or `Txt2` holds another text for it), `Err` with the reason when the path can't be
/// read: the layer then stays the box or point text EngineData describes.
pub fn apply_txt2_path(layer: &mut TextLayer, tysh: &[u8], txt2: &E) -> Result<bool, String> {
    let Some((_, frame)) = layer_frame(layer, tysh, txt2, true) else { return Ok(false) };
    if crate::psd_path::frame_kind(frame) != crate::psd_path::PATH_FRAME {
        return Ok(false);
    }
    let path = crate::psd_path::read(frame)?;
    layer.shape = TextShape::Path { path, start: 0.0, flip: false };
    Ok(true)
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
    // Type on a path: [`apply_txt2_path`] already read the path through the frame's matrix.
    if matches!(layer.shape, TextShape::Path { .. }) {
        return;
    }
    let Some((t, frame)) = layer_frame(layer, tysh, txt2, false) else { return };
    let m = arr_f(frame.path(&["2", "2"]));
    let matrix = if m.len() == 6 && m.iter().all(|v| v.is_finite()) { Affine { m: [m[0], m[1], m[2], m[3], m[4], m[5]] } } else { Affine::IDENTITY };
    let at = match layer.shape {
        TextShape::Point | TextShape::Path { .. } => {
            let p = arr_f(frame.get("0"));
            if p.len() == 2 && p.iter().all(|v| v.is_finite()) { Affine::translate(p[0], p[1]) } else { Affine::IDENTITY }
        }
        TextShape::Box { .. } | TextShape::Area { .. } => Affine::IDENTITY,
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
        TextShape::Box { .. } | TextShape::Area { .. } => {
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
        let corrected = [a, b, c, d, a * x + c * y + e, b * x + d * y + f];
        if corrected.iter().all(|v| v.is_finite()) {
            layer.transform.m = corrected;
        }
    }
    layer.sync_summary();
    Some(layer)
}

/// Refines a legacy point-text estimate after imported fonts and styles have been resolved.
/// Uses the rendering engine's font context; an accepted `Txt2` frame stays authoritative.
pub fn refine_legacy_point_origin(layer: &mut TextLayer, dpi: f32, engine: &mut crate::TextEngine, txt2: Option<&E>) {
    let Some(raw) = layer.psd_raw.clone() else { return };
    let Some(t) = parse_tysh(&raw) else { return };
    let Some([x, y]) = legacy_point_origin(&t) else { return };
    if !matches!(layer.shape, TextShape::Point) {
        return;
    }
    let guessed = t.transform.mul(&Affine::translate(x, y));
    if layer.transform != guessed {
        return;
    }
    // `boundingBox` is the tight ink box (Photoshop writes the layout box as `bounds`), so match
    // it with the layout's ink, not its logical box: text whose ink starts below the ascent
    // line (`...`, `_`, lowercase) would otherwise land too low.
    let Some(ink) = crate::render::ink_bounds(&engine.layout(layer, dpi)) else { return };
    if !ink.iter().all(|v| v.is_finite()) {
        return;
    }
    let corrected = t.transform.mul(&Affine::translate(x - ink[0], y - ink[1]));
    if corrected.m.iter().all(|v| v.is_finite()) {
        layer.transform = corrected;
        // A valid frame can coincide with the initial estimate; numeric equality cannot tell
        // whether it was used. Reapply it after the fallback so it always wins.
        if let Some(txt2) = txt2 {
            apply_txt2_frame(layer, &raw, txt2);
        }
    }
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
    if !(bounds[0].abs().max(bounds[1].abs()) > 1024.0 && origin_distance > 4096.0 && shifted_distance < origin_distance) {
        return None;
    }

    Some([bounds[0], bounds[1]])
}

fn descriptor_rect(text: &Descriptor, key: &str) -> Option<[f64; 4]> {
    let D::Descriptor(rect) = text.get(key)? else { return None };
    let bounds = [dnum(rect.get("Left"))?, dnum(rect.get("Top "))?, dnum(rect.get("Rght"))?, dnum(rect.get("Btom"))?];
    (bounds.iter().all(|v| v.is_finite()) && bounds[2] > bounds[0] && bounds[3] > bounds[1]).then_some(bounds)
}

pub(super) const MAX_ENGINE_RUNS: usize = 65_536;

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
    // Vertical type's roman text: 2 rotated along the column, 1 upright (#9470). A run without
    // its own value takes the style sheet's, which Photoshop writes as 2.
    s.rotate_roman = g("BaselineDirection").and_then(E::as_i64) == Some(2);
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

pub(super) fn real(v: f32) -> E {
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

pub(super) fn manual(s: &CharStyle) -> bool {
    s.kerning == Kerning::Off || kern_units(s.kern) != 0
}

/// EngineData style runs for `ed_text` (the layer text with `\r` breaks and the trailing `\r`):
/// (model run index, `(AutoKerning, Kerning)`, UTF-16 length), split where those change.
pub(super) fn pair_runs(ed_text: &str, runs: &[TextRun]) -> Vec<(usize, (bool, i64), usize)> {
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
