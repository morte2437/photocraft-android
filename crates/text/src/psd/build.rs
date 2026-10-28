use super::edit::{TEMPLATE_NODES, edited_object, fits, is_stub, names_one_frame, new_object, pad_frames, td_edited_object, with_modes};
use super::import::{OPENTYPE_KEYS, TXT2_FEATURES, TXT2_STYLISTIC_SETS, kern_units, manual, object_frame, pair_runs, parse_txt2, postscript_for, real};
use super::{
    AntiAlias, Caps, CharStyle, Color, ColorMode, D, Descriptor, E, FontBaseline, Id, Kerning, Orientation, ParagraphStyle, TextAlign, TextLayer, TextShape,
    TySh, UnicodeString, ed, engine_data, enum_value, enumv, parse_tysh, write_tysh,
};

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

pub(super) fn utf16_len(s: &str) -> i64 {
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
    // The frame each written object is drawn in, with its layer's shape (type on a path lives
    // only in the frame, #7962).
    let mut frame_shapes: Vec<(usize, &TextShape)> = Vec::new();
    // A new or rebuilt object must not be drawn in a frame another object is drawn in: the two
    // layers would share one shape. The frames the file's objects name are taken; a free one is
    // found past the frame list and the slots (`pad_frames` adds it).
    let frame_count = prev.as_ref().and_then(|p| p.path(&["0", "8", "0"])).and_then(E::as_array).map(<[E]>::len);
    let mut taken: std::collections::HashSet<usize> = prev_objects.iter().filter_map(|o| object_frame(o, true)).collect();
    let slot_count =
        objects.iter().filter_map(|(i, _)| usize::try_from(*i).ok().filter(|_| (0..=MAX_TEXT_INDEX).contains(i))).map(|i| i + 1).max().unwrap_or(0);
    let mut next_free = frame_count.unwrap_or(0).max(prev_objects.len()).max(slot_count);
    // The frame list grows only for the frames handed out here, never for a frame number an
    // object the file kept names past its list.
    let mut frames_needed = 0usize;
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
        // frame in `/0 /8 /0`, made Photoshop and Photopea refuse the file. A slot whose object
        // can't be refitted keeps the frame that object was drawn in.
        let fresh = (kept.is_none() && edited.is_none())
            .then(|| {
                let template = template?;
                let own = slot.and_then(|o| object_frame(o, true)).filter(|f| frame_count.is_some_and(|n| *f < n));
                let frame = match own {
                    Some(f) => f,
                    None if frame_count.is_some() && taken.contains(&at) => {
                        while taken.contains(&next_free) {
                            next_free += 1;
                        }
                        if next_free > MAX_TEXT_INDEX as usize { at } else { next_free }
                    }
                    None => at,
                };
                let object = new_object(template, frame, &text)?;
                taken.insert(frame);
                frames_needed = frames_needed.max(frame.saturating_add(1));
                Some(object)
            })
            .flatten();
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
        if let Some(frame) = object_frame(&object, true) {
            frame_shapes.push((frame, &layer.shape));
        }
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
    pad_frames(&mut outer, slots.len().max(frames_needed));
    // A path layer's frame holds its path; a layer taken off its path loses its path frame. An
    // unchanged path leaves the file's frame as it was, and so does a frame the file already had
    // more than one object drawn in (which layer's shape it should hold is unknown).
    let mut claims: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (at, _) in &frame_shapes {
        *claims.entry(*at).or_default() += 1;
    }
    if let Some(E::Array(frames)) = outer.get_mut("0").and_then(|v| v.get_mut("8")).and_then(|v| v.get_mut("0")) {
        for (at, shape) in frame_shapes {
            if claims.get(&at) != Some(&1) {
                continue;
            }
            if let Some(entry) = frames.get_mut(at) {
                crate::psd_path::update_frame(entry, shape);
            }
        }
    }
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
    let (shape_type, box_bounds) = match &layer.shape {
        // Type on a path: the path goes into the layer's `Txt2` frame ([`build_txt2`], #7962),
        // which only a file that already has a `Txt2` object for the layer gets; EngineData
        // keeps it as point text, one line for readers (and files) without that frame.
        TextShape::Point | TextShape::Path { .. } => (0, None),
        TextShape::Box { x, y, width, height } => (1, Some([*x, *y, x + width, y + height])),
        // PSD area type is not written yet: the layer saves as paragraph text over the path's
        // bounds, so Photoshop opens it with the same text wrapping in a rectangle.
        TextShape::Area { path } => match crate::area::path_bounds(path) {
            Some((x0, y0, x1, y1)) => (1, Some([x0 as f32, y0 as f32, x1 as f32, y1 as f32])),
            None => (0, None),
        },
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
pub(super) fn engine_antialias(n: f64) -> Option<AntiAlias> {
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
    let area = match &layer.shape {
        TextShape::Area { path } => crate::area::path_bounds(path).map(|(x0, y0, x1, y1)| [x0 as f32, y0 as f32, x1 as f32, y1 as f32]),
        _ => None,
    };
    let [l, t, r, b] = match (&layer.shape, area, ink) {
        (&TextShape::Box { x, y, width, height }, _, _) => [x, y, x + width, y + height],
        (_, Some(a), _) => a,
        (_, _, Some(i)) => i,
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
