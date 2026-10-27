//! `type.*` commands: create and edit type (text) layers.
//!
//! Text offsets in parameters and results are **character** indices (Unicode scalar values),
//! so JSON clients never split a UTF-8 sequence. Sizes, leading, indents and spacing are in
//! points (converted with the document resolution); tracking is in 1/1000 em; positions and
//! boxes are document pixels. Every edit re-renders the layer with `photocraft-text` and
//! regenerates its PSD `TySh` data, so the layer saves as editable text.

use std::sync::Arc;

use photocraft_color::{Color, PixelFormat};
use photocraft_doc::text::{
    AntiAlias, Caps, CharStyle, FontBaseline, FontFeature, FontVariation, Kerning, Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextDirection,
    TextRun, TextShape,
};
use photocraft_doc::{Affine, Document, Layer, LayerContent, LayerId, TextLayer};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn layer_id(s: &Session, p: &Value) -> Result<LayerId> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(LayerId(id)),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn f32p(p: &Value, k: &str) -> Option<f32> {
    p.get(k).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite())
}

fn color(v: &Value) -> Option<Color> {
    match v {
        Value::Array(a) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            c.iter().take(4).all(|v| v.is_finite()).then(|| Color::rgba(c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)))
        }
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            let h = |i: usize| s.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match s.len() {
                6 => Some(Color::rgb(h(0)?, h(2)?, h(4)?)),
                8 => Some(Color::rgba(h(0)?, h(2)?, h(4)?, h(6)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Normalises line breaks to `\n`.
pub(crate) fn norm_text(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn byte_at(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

fn char_at(text: &str, bi: usize) -> usize {
    text[..bi.min(text.len())].chars().count()
}

/// `range: [start, end]` in characters → byte range (whole text when absent).
fn range_param(text: &str, p: &Value) -> (usize, usize) {
    match p.get("range").and_then(Value::as_array) {
        Some(r) if r.len() == 2 => {
            let a = r[0].as_u64().unwrap_or(0) as usize;
            let b = r[1].as_u64().unwrap_or(a as u64) as usize;
            let (a, b) = (a.min(b), a.max(b));
            (byte_at(text, a), byte_at(text, b))
        }
        _ => (0, text.len()),
    }
}

/// Photoshop's manual kerning range (1/1000 em).
pub const KERN_MIN: f64 = -1000.0;
pub const KERN_MAX: f64 = 10_000.0;
pub const TYPE_SIZE_MIN_PT: f64 = 0.1;
pub const TYPE_SIZE_MAX_PT: f64 = 1296.0;
pub const TRACKING_MIN: f64 = -1000.0;
pub const TRACKING_MAX: f64 = 10_000.0;
/// Photoshop's horizontal and vertical scale range, in percent.
pub const SCALE_MAX_PERCENT: f64 = 1000.0;

/// Validates the `kerning` key of character params: a number in [`KERN_MIN`]..=[`KERN_MAX`] or
/// `"metrics"`, `"optical"`, `"off"` (`"none"`, `"0"`).
pub fn check_kerning(p: &Value) -> std::result::Result<(), String> {
    match p.get("kerning") {
        None => Ok(()),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(k) if (KERN_MIN..=KERN_MAX).contains(&k) => Ok(()),
            _ => Err(format!("kerning must be between {KERN_MIN} and {KERN_MAX} (1/1000 em)")),
        },
        Some(Value::String(v)) if matches!(v.to_ascii_lowercase().as_str(), "metrics" | "optical" | "off" | "none" | "0") => Ok(()),
        Some(_) => Err("kerning must be a number (1/1000 em) or \"metrics\", \"optical\" or \"off\"".into()),
    }
}

/// Validates text metrics (size, tracking, leading, baseline shift, scales) that can otherwise
/// exceed the rasterizer's supported range or overflow `f32`, which a .pcraft manifest cannot hold,
/// and the superscript/subscript `baseline`, so a typo is an error rather than a reset to Normal.
pub fn check_size_tracking(p: &Value) -> std::result::Result<(), String> {
    if let Some(v) = p.get("size") {
        match v.as_f64() {
            Some(size) if size.is_finite() && (TYPE_SIZE_MIN_PT..=TYPE_SIZE_MAX_PT).contains(&size) => {}
            _ => return Err(format!("size must be between {TYPE_SIZE_MIN_PT} and {TYPE_SIZE_MAX_PT} pt")),
        }
    }
    if let Some(v) = p.get("tracking") {
        match v.as_f64() {
            Some(tracking) if tracking.is_finite() && (TRACKING_MIN..=TRACKING_MAX).contains(&tracking) => {}
            _ => return Err(format!("tracking must be between {TRACKING_MIN} and {TRACKING_MAX} (1/1000 em)")),
        }
    }
    if let Some(v) = p.get("leading") {
        let (min, max) = (f64::from(LEADING_MIN_PT), f64::from(LEADING_MAX_PT));
        match v {
            Value::Null => {}
            Value::String(a) if a == "auto" => {}
            Value::Number(n) if n.as_f64().is_some_and(|x| x.is_finite() && (min..=max).contains(&x)) => {}
            _ => return Err(format!("leading must be between {min} and {max} pt, or \"auto\"")),
        }
    }
    if let Some(v) = p.get("baselineShift") {
        let max = f64::from(BASELINE_SHIFT_MAX_PT);
        match v.as_f64() {
            Some(shift) if shift.is_finite() && shift.abs() <= max => {}
            _ => return Err(format!("baselineShift must be between -{max} and {max} pt")),
        }
    }
    if let Some(v) = p.get("baseline").filter(|v| !v.is_null())
        && !matches!(v.as_str(), Some("normal" | "superscript" | "subscript"))
    {
        return Err("baseline must be \"normal\", \"superscript\" or \"subscript\"".into());
    }
    for key in ["horizontalScale", "verticalScale"] {
        if let Some(v) = p.get(key) {
            match v.as_f64() {
                Some(scale) if scale.is_finite() && (0.0..=SCALE_MAX_PERCENT).contains(&scale) => {}
                _ => return Err(format!("{key} must be between 0 and {SCALE_MAX_PERCENT} %")),
            }
        }
    }
    Ok(())
}

/// The PostScript name of the installed face of `family` at exactly `weight`/`italic`. A PSD names
/// a type layer by PostScript name, and Photoshop treats a name no font has as missing (#3556).
/// Only an exact face is named: layout picks a named face before weight and italic, so naming a
/// nearby face would override the requested weight (a variable font's axis, a weight between two
/// faces) or italic. Without an exact face the style keeps no name and layout matches as before.
fn installed_postscript(family: &str, weight: u16, italic: bool) -> Option<String> {
    let mut shared = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let faces = shared.fonts.faces(family);
    let face = faces.iter().find(|f| f.italic == italic && f.weight.round() as u16 == weight)?;
    face.postscript_name.clone()
}

/// Photoshop's leading range in points.
pub const LEADING_MIN_PT: f32 = 0.01;
pub const LEADING_MAX_PT: f32 = 5000.0;
/// Photoshop's baseline shift range in points (either direction).
pub const BASELINE_SHIFT_MAX_PT: f32 = 5000.0;
/// The relative character steps `type.setStyle` accepts, with the largest step allowed.
const STEP_KEYS: [(&str, f64); 3] = [("leadingBy", 5000.0), ("baselineShiftBy", 10_000.0), ("trackingBy", 11_000.0)];

/// Validates the relative steps (`leadingBy`, `baselineShiftBy`, `trackingBy`): finite numbers
/// no larger than the whole range they move through.
pub fn check_steps(p: &Value) -> std::result::Result<(), String> {
    for (key, max) in STEP_KEYS {
        if let Some(v) = p.get(key) {
            match v.as_f64() {
                Some(by) if by.is_finite() && by.abs() <= max => {}
                _ => return Err(format!("{key} must be a number between -{max} and {max}")),
            }
        }
    }
    Ok(())
}

/// Relative character steps (Photoshop's ⌥↑/↓ leading, ⇧⌥↑/↓ baseline shift and ⌥←/→ tracking
/// over a selection): every character moves from its own value, so mixed values keep their
/// differences. Auto leading starts from the value layout uses, the paragraph's auto-leading
/// percentage of the character's size. Results are clamped to Photoshop's ranges. Returns true
/// if any step key was present.
fn step_char_metrics(t: &mut TextLayer, a: usize, b: usize, p: &Value) -> bool {
    let leading_by = f32p(p, "leadingBy").filter(|v| v.is_finite());
    let shift_by = f32p(p, "baselineShiftBy").filter(|v| v.is_finite());
    let tracking_by = f32p(p, "trackingBy").filter(|v| v.is_finite());
    if leading_by.is_none() && shift_by.is_none() && tracking_by.is_none() {
        return false;
    }
    let paragraphs = t.paragraph_runs();
    let default_auto = ParagraphStyle::default().auto_leading;
    // Each paragraph's share of `a..b`, with the auto leading it lays out with.
    let mut pieces = Vec::new();
    let mut at = 0usize;
    for pr in &paragraphs {
        let (ps, pe) = (at, at.saturating_add(pr.len));
        at = pe;
        let (pa, pb) = (a.clamp(ps, pe), b.clamp(ps, pe));
        if pa < pb {
            pieces.push((pa, pb, pr.style.auto_leading));
        }
    }
    if at < b {
        pieces.push((a.max(at), b, default_auto));
    }
    if t.text.is_empty() {
        // Empty text: the step sets the style the next typed characters get.
        pieces = vec![(0, 0, paragraphs.first().map_or(default_auto, |r| r.style.auto_leading))];
    }
    for (pa, pb, auto) in pieces {
        style_range(t, pa, pb, &|st| {
            if let Some(by) = leading_by {
                let auto = if auto.is_finite() && auto > 0.0 { auto } else { default_auto };
                let base = st.leading_pt.unwrap_or(auto * st.size_pt);
                let base = if base.is_finite() { base } else { 0.0 };
                st.leading_pt = Some((base + by).clamp(LEADING_MIN_PT, LEADING_MAX_PT));
            }
            if let Some(by) = shift_by {
                let base = if st.baseline_shift_pt.is_finite() { st.baseline_shift_pt } else { 0.0 };
                st.baseline_shift_pt = (base + by).clamp(-BASELINE_SHIFT_MAX_PT, BASELINE_SHIFT_MAX_PT);
            }
            if let Some(by) = tracking_by {
                let base = if st.tracking.is_finite() { st.tracking } else { 0.0 };
                st.tracking = (base + by).clamp(TRACKING_MIN as f32, TRACKING_MAX as f32);
            }
        });
    }
    true
}

/// Applies character-style keys from JSON. Returns true if any key was present.
pub fn apply_char_props(s: &mut CharStyle, p: &Value) -> bool {
    let mut any = false;
    let mut hit = |b: bool| any |= b;
    let font_key = ["font", "family", "fontStyle", "weight", "italic"].iter().any(|k| p.get(k).is_some());
    if let Some(v) = p.get("font").or_else(|| p.get("family")).and_then(Value::as_str) {
        s.font_family = v.to_string();
        s.postscript_name = None;
        // Photoshop accepts either a family name or a face's PostScript name in the font field.
        // Resolve only exact PostScript matches here so an unknown family remains untouched.
        let mut text = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
        if !text.fonts.has_family(v) {
            let resolved = text.fonts.resolve_postscript(v);
            if resolved.exact {
                s.variations.retain(|f| f.axis != "wght");
                s.font_family = resolved.family;
                s.weight = resolved.weight;
                s.italic = resolved.italic;
                s.postscript_name = Some(v.to_string());
            }
        }
        hit(true);
    }
    if let Some(v) = p.get("fontStyle").and_then(Value::as_str) {
        s.font_style = v.to_string();
        s.postscript_name = None;
        // The style sets the weight, so a weight slider value set earlier stops overriding it
        // (`variations` in the same call apply after this).
        s.variations.retain(|f| f.axis != "wght");
        let l = v.to_lowercase();
        s.italic = l.contains("italic") || l.contains("oblique");
        let g = photocraft_text::fonts::guess_from_postscript(&format!("X-{}", v.replace(' ', "")));
        s.weight = g.weight;
        if let Some(face) = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.named_face(&s.font_family, v) {
            s.weight = face.weight.round() as u16;
            s.italic = face.italic;
            s.postscript_name = face.postscript_name;
        }
        hit(true);
    }
    if let Some(v) = p.get("weight").and_then(Value::as_f64) {
        let weight = v.clamp(1.0, 1000.0) as u16;
        if s.weight != weight {
            s.postscript_name = None;
            s.font_style.clear();
        }
        s.variations.retain(|f| f.axis != "wght");
        s.weight = weight;
        hit(true);
    }
    if let Some(v) = p.get("italic").and_then(Value::as_bool) {
        if s.italic != v {
            s.postscript_name = None;
            s.font_style.clear();
        }
        s.italic = v;
        hit(true);
    }
    if let Some(v) = p.get("postscriptName").and_then(Value::as_str) {
        s.postscript_name = Some(v.to_string());
        hit(true);
    }
    // A font named by family (or a style that matched no face) has no PostScript name yet: fill in
    // the installed face's real name, so the PSD does not invent "<Family>-Regular" (#3556).
    if font_key && s.postscript_name.is_none() {
        s.postscript_name = installed_postscript(&s.font_family, s.weight, s.italic);
    }
    if let Some(v) = f32p(p, "size") {
        s.size_pt = v.max(0.01);
        hit(true);
    }
    if let Some(c) = p.get("color").and_then(color) {
        s.color = c;
        hit(true);
    }
    if let Some(v) = f32p(p, "tracking") {
        s.tracking = v;
        hit(true);
    }
    match p.get("leading") {
        Some(Value::Null) => {
            s.leading_pt = None;
            hit(true);
        }
        Some(Value::String(a)) if a == "auto" => {
            s.leading_pt = None;
            hit(true);
        }
        Some(v) if v.is_number() => {
            s.leading_pt = v.as_f64().map(|x| x as f32);
            hit(true);
        }
        _ => {}
    }
    for (k, f) in [("baselineShift", &mut s.baseline_shift_pt), ("horizontalScale", &mut s.horizontal_scale), ("verticalScale", &mut s.vertical_scale)] {
        if let Some(v) = f32p(p, k) {
            *f = if k == "baselineShift" { v } else { (v / 100.0).max(0.01) };
            hit(true);
        }
    }
    for (k, f) in [
        ("underline", &mut s.underline),
        ("strikethrough", &mut s.strikethrough),
        ("fauxBold", &mut s.faux_bold),
        ("fauxItalic", &mut s.faux_italic),
        ("ligatures", &mut s.ligatures),
        ("discretionaryLigatures", &mut s.discretionary_ligatures),
    ] {
        if let Some(v) = p.get(k).and_then(Value::as_bool) {
            *f = v;
            hit(true);
        }
    }
    match p.get("kerning") {
        // A number is Photoshop's manual kerning: no automatic pair kerning plus the value.
        Some(v) if v.is_number() => {
            if let Some(k) = v.as_f64().filter(|k| k.is_finite()) {
                s.kerning = Kerning::Off;
                s.kern = k.clamp(KERN_MIN, KERN_MAX) as f32;
                hit(true);
            }
        }
        Some(Value::String(v)) => {
            s.kerning = match v.to_ascii_lowercase().as_str() {
                "optical" => Kerning::Optical,
                "none" | "off" | "0" => Kerning::Off,
                _ => Kerning::Metrics,
            };
            s.kern = 0.0;
            hit(true);
        }
        _ => {}
    }
    if let Some(v) = p.get("caps").and_then(Value::as_str) {
        s.caps = match v {
            "small" | "smallCaps" => Caps::SmallCaps,
            "all" | "allCaps" => Caps::AllCaps,
            _ => Caps::Normal,
        };
        hit(true);
    }
    if let Some(v) = p.get("baseline").and_then(Value::as_str) {
        let baseline = match v {
            "superscript" => Some(FontBaseline::Superscript),
            "subscript" => Some(FontBaseline::Subscript),
            "normal" => Some(FontBaseline::Normal),
            _ => None,
        };
        // An unknown value keeps the style (the commands reject it in `check_size_tracking`).
        if let Some(b) = baseline {
            s.baseline = b;
            hit(true);
        }
    }
    if let Some(Value::Object(m)) = p.get("features") {
        for (tag, v) in m {
            let value = v.as_u64().map(|x| x as u16).or_else(|| v.as_bool().map(u16::from)).unwrap_or(1);
            s.features.retain(|f| &f.tag != tag);
            s.features.push(FontFeature { tag: tag.clone(), value });
        }
        hit(true);
    }
    if let Some(Value::Object(m)) = p.get("variations") {
        for (axis, v) in m {
            s.variations.retain(|f| &f.axis != axis);
            if let Some(x) = v.as_f64() {
                s.variations.push(FontVariation { axis: axis.clone(), value: x as f32 });
            }
        }
        hit(true);
    }
    if let Some(v) = p.get("language") {
        s.language = v.as_str().map(str::to_string);
        hit(true);
    }
    any
}

/// Applies paragraph-style keys from JSON. Returns true if any key was present.
pub fn apply_para_props(s: &mut ParagraphStyle, p: &Value) -> bool {
    let mut any = false;
    if let Some(v) = p.get("align").and_then(Value::as_str) {
        s.align = match v {
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            "justify" | "justifyLeft" => TextAlign::JustifyLeft,
            "justifyCenter" => TextAlign::JustifyCenter,
            "justifyRight" => TextAlign::JustifyRight,
            "justifyAll" => TextAlign::JustifyAll,
            _ => TextAlign::Left,
        };
        any = true;
    }
    for (k, f) in [
        ("firstLineIndent", &mut s.first_line_indent_pt),
        ("startIndent", &mut s.start_indent_pt),
        ("endIndent", &mut s.end_indent_pt),
        ("spaceBefore", &mut s.space_before_pt),
        ("spaceAfter", &mut s.space_after_pt),
        ("autoLeading", &mut s.auto_leading),
    ] {
        if let Some(v) = f32p(p, k) {
            *f = if k == "autoLeading" { v / 100.0 } else { v };
            any = true;
        }
    }
    if let Some(v) = p.get("direction").and_then(Value::as_str) {
        s.direction = match v {
            "ltr" => TextDirection::Ltr,
            "rtl" => TextDirection::Rtl,
            _ => TextDirection::Auto,
        };
        any = true;
    }
    if let Some(v) = p.get("hyphenate").and_then(Value::as_bool) {
        s.hyphenate = v;
        any = true;
    }
    any
}

fn push_merge<S: PartialEq>(out: &mut Vec<(usize, S)>, len: usize, st: S) {
    if len == 0 {
        return;
    }
    match out.last_mut() {
        Some(last) if last.1 == st => last.0 += len,
        _ => out.push((len, st)),
    }
}

/// Applies `f` to the character style of bytes `a..b` (splitting runs at the edges).
pub fn style_range(t: &mut TextLayer, a: usize, b: usize, f: &dyn Fn(&mut CharStyle)) {
    // A reversed range styles nothing; `b < a` would underflow the styled run's length (#714).
    let b = b.max(a);
    let runs = t.char_runs();
    if t.text.is_empty() {
        let mut st = runs.into_iter().next().map(|r| r.style).unwrap_or_default();
        f(&mut st);
        t.runs = vec![TextRun { len: 0, style: st }];
        t.sync_summary();
        return;
    }
    let mut out: Vec<(usize, CharStyle)> = Vec::new();
    let mut at = 0;
    for r in runs {
        let (rs, re) = (at, at + r.len);
        at = re;
        let (ca, cb) = (a.clamp(rs, re), b.clamp(rs, re));
        push_merge(&mut out, ca - rs, r.style.clone());
        let mut st = r.style.clone();
        f(&mut st);
        push_merge(&mut out, cb - ca, st);
        push_merge(&mut out, re - cb, r.style);
    }
    t.runs = out.into_iter().map(|(len, style)| TextRun { len, style }).collect();
    t.sync_summary();
}

/// Applies `f` to every paragraph touching bytes `a..b` (paragraph runs follow paragraphs).
pub fn style_paragraphs(t: &mut TextLayer, a: usize, b: usize, f: &dyn Fn(&mut ParagraphStyle)) {
    let old = t.paragraph_runs();
    let starts: Vec<usize> = old
        .iter()
        .scan(0, |acc, r| {
            let s = *acc;
            *acc += r.len;
            Some(s)
        })
        .collect();
    let style_at = |off: usize| old[starts.iter().rposition(|&s| s <= off).unwrap_or(0)].style.clone();
    let mut out: Vec<(usize, ParagraphStyle)> = Vec::new();
    let trailing_break = t.text.ends_with(['\r', '\n']);
    for pr in photocraft_text::layout::split_paragraphs(&t.text) {
        let mut st = style_at(pr.start);
        let terminal_empty = pr.start == pr.end;
        let touches =
            (pr.start < b && pr.end > a) || (a == b && a >= pr.start && (a < pr.end || terminal_empty || (pr.end == t.text.len() && !trailing_break)));
        if touches {
            f(&mut st);
        }
        if terminal_empty && trailing_break {
            // This paragraph has no bytes but must retain its own style for the
            // next typed character and for the PSD's final paragraph marker.
            out.push((0, st));
        } else {
            push_merge(&mut out, pr.len(), st.clone());
            if out.is_empty() {
                out.push((0, st));
            }
        }
    }
    t.paragraphs = out.into_iter().map(|(len, style)| ParagraphRun { len, style }).collect();
}

/// Replaces bytes `a..b` with `new`, keeping styles: replacing a selection keeps the style of its
/// first character (as Photoshop does, so retyping a smaller second line stays small); inserting
/// at a caret takes the style of the character before it (or after it at the start).
/// The part of `old` that differs from `new`: its byte range in `old` and the text that replaces
/// it, after the longest common prefix and then the longest common suffix that doesn't overlap it
/// (both on character boundaries).
fn changed_span<'a>(old: &str, new: &'a str) -> (usize, usize, &'a str) {
    let prefix = old.char_indices().zip(new.chars()).find(|((_, a), b)| a != b).map_or_else(|| old.len().min(new.len()), |((i, _), _)| i);
    let (old_rest, new_rest) = (&old[prefix..], &new[prefix..]);
    let suffix: usize = old_rest.chars().rev().zip(new_rest.chars().rev()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum();
    (prefix, old.len() - suffix, &new[prefix..new.len() - suffix])
}

pub fn replace_text(t: &mut TextLayer, a: usize, b: usize, new: &str) {
    fn adjust<S: Clone + PartialEq>(runs: Vec<(usize, S)>, a: usize, b: usize, ins: usize, trailing_paragraph: bool) -> Vec<(usize, S)> {
        let mut out: Vec<(usize, S)> = Vec::new();
        let mut at = 0;
        // Paragraph runs only: the non-empty paragraph that starts exactly at an insertion point.
        let starts_here = || {
            let mut s = 0;
            runs.iter().position(|r| {
                let here = s == a && r.0 > 0;
                s += r.0;
                here
            })
        };
        let host = if trailing_paragraph && a == b && runs.last().is_some_and(|r| r.0 == 0) && a == runs.iter().map(|r| r.0).sum::<usize>() {
            // Inserted text belongs to a styled final empty paragraph, not to
            // the preceding paragraph whose break ends at this byte offset.
            runs.len() - 1
        } else if let Some(next) = (trailing_paragraph && a == b && a > 0).then(starts_here).flatten() {
            // Text inserted at the start of a paragraph belongs to that paragraph, not to the one
            // whose break ends there (#8489).
            next
        } else {
            runs.iter()
                .scan(0, |acc, r| {
                    let s = *acc;
                    *acc += r.0;
                    Some((s, s + r.0))
                })
                .position(|(s, e)| {
                    if a == 0 {
                        e > 0 || s == 0
                    } else if a < b {
                        a >= s && a < e
                    } else {
                        a > s && a <= e
                    }
                })
                .unwrap_or(0)
        };
        for (i, (len, st)) in runs.into_iter().enumerate() {
            let (rs, re) = (at, at + len);
            at = re;
            let keep = (a.clamp(rs, re) - rs) + (re - b.clamp(rs, re));
            let add = if i == host { ins } else { 0 };
            push_merge(&mut out, keep + add, st);
        }
        out
    }
    let (a, b) = (a.min(t.text.len()), b.min(t.text.len()).max(a.min(t.text.len())));
    let new = norm_text(new);
    let cr: Vec<(usize, CharStyle)> = t.char_runs().into_iter().map(|r| (r.len, r.style)).collect();
    let pr: Vec<(usize, ParagraphStyle)> = t.paragraph_runs().into_iter().map(|r| (r.len, r.style)).collect();
    let first_c = cr.first().map(|r| r.1.clone()).unwrap_or_default();
    let first_p = pr.first().map(|r| r.1.clone()).unwrap_or_default();
    let trailing_style = t.text.ends_with(['\r', '\n']).then(|| pr.last().filter(|r| r.0 == 0).map(|r| r.1.clone())).flatten();
    let mut cr = adjust(cr, a, b, new.len(), false);
    let mut pr = adjust(pr, a, b, new.len(), true);
    t.text.replace_range(a..b, &new);
    if t.text.ends_with(['\r', '\n'])
        && let Some(style) = trailing_style
    {
        pr.push((0, style));
    }
    if cr.is_empty() {
        cr.push((t.text.len(), first_c));
    }
    if pr.is_empty() {
        pr.push((t.text.len(), first_p));
    }
    t.runs = cr.into_iter().map(|(len, style)| TextRun { len, style }).collect();
    t.paragraphs = pr.into_iter().map(|(len, style)| ParagraphRun { len, style }).collect();
    t.sync_summary();
}

/// Re-renders the layer's pixels and regenerates its PSD `TySh` data.
pub fn refresh(doc: &Document, t: &mut TextLayer) {
    refresh_at(doc.resolution_dpi, doc.pixel_format(), t);
}

/// [`refresh`] given only the two things it reads from the document: its resolution and its
/// pixel format. A caller holding the layer mutably reads them first instead of cloning the whole
/// document to pass it, once per keystroke and pointer move while typing (#7092).
pub fn refresh_at(dpi: f32, format: PixelFormat, t: &mut TextLayer) {
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let (layout, r) = eng.render(t, dpi, format);
    t.cache = Some(r.surface);
    t.psd_raw = Some(Arc::new(photocraft_text::psd::build_tysh(t, dpi, layout.bounds())));
}

/// Refuses to change a type layer under Lock All (its own or an enclosing group's), with
/// Photoshop's message. Every command that edits a type layer's text calls it first (#3132, #4188).
pub(crate) fn check_text_unlocked(doc: &Document, id: LayerId) -> Result<()> {
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if doc.effective_locks(id).all {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", layer.name)));
    }
    Ok(())
}

/// Runs `f` on a type layer as one undo step. `f` may set an explicit layer name (`type.edit`'s
/// `name`), which is applied in the same step; otherwise an auto-named layer follows its text.
/// `f` gets the document's resolution (dpi).
fn with_text_layer<R>(s: &mut Session, p: &Value, label: &str, f: impl FnOnce(&mut TextLayer, f32, &mut Option<String>) -> Result<R>) -> Result<R> {
    let id = layer_id(s, p)?;
    check_text_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let (r, damage) = s.edit(label, |doc, _| {
        let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let auto_named = is_auto_named(l);
        let LayerContent::Text(t) = &mut l.content else {
            return Err(EngineError::Other(format!("layer {} is {} {} layer, not a type layer", id.0, l.content.article(), l.content.kind_name())));
        };
        let before = t.cache.as_ref().map(|c| c.tile_bounds());
        let mut rename = None;
        let r = f(t, dpi, &mut rename)?;
        refresh_at(dpi, format, t);
        // Only this layer's pixels changed: the canvas recomposites their old and new area
        // instead of the whole document (#124). Unknown old pixels mean a full refresh.
        let damage = before.zip(t.cache.as_ref().map(|c| c.tile_bounds())).map(|(a, b)| a.union(&b));
        let name = rename.or_else(|| auto_named.then(|| layer_name(&t.text)));
        if let Some(n) = name {
            l.name = n;
        }
        Ok((r, damage))
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = damage;
    }
    Ok(r)
}

/// Direct formatting follows the selected Type layers, while an explicit layer or character
/// range keeps its single-layer scope. Selected groups are not expanded.
pub(crate) fn format_targets(s: &Session, p: &Value) -> Result<Vec<LayerId>> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let single = p.get("layer").is_some() || (p.get("range").is_some() && p.get("layers").is_none());
    let ids = if single {
        vec![layer_id(s, p)?]
    } else if let Some(value) = p.get("layers") {
        value
            .as_array()
            .ok_or_else(|| EngineError::Other("layers must be an array of layer IDs".into()))?
            .iter()
            .map(|v| v.as_u64().map(LayerId).ok_or_else(|| EngineError::Other("layers must contain layer IDs".into())))
            .collect::<Result<Vec<_>>>()?
    } else {
        st.selected_layers()
    };
    let mut targets = Vec::new();
    for id in ids {
        let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        if matches!(l.content, LayerContent::Text(_)) {
            if !targets.contains(&id) {
                targets.push(id);
            }
        } else if single {
            return Err(EngineError::Other(format!("layer {} is {} {} layer, not a type layer", id.0, l.content.article(), l.content.kind_name())));
        }
    }
    if targets.is_empty() {
        return Err(EngineError::Other("select a type layer".into()));
    }
    Ok(targets)
}

pub(crate) fn has_format_target(s: &Session) -> std::result::Result<(), String> {
    format_targets(s, &Value::Null).map(|_| ()).map_err(|e| e.to_string())
}

/// One history step for a formatting batch. Structural text edits keep their singular helper.
pub(crate) fn with_text_targets(
    s: &mut Session,
    p: &Value,
    label: &str,
    mut f: impl FnMut(&mut TextLayer, (usize, usize)) -> Result<()>,
) -> Result<Vec<LayerId>> {
    let ids = format_targets(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    for id in &ids {
        check_text_unlocked(doc, *id)?;
    }
    let damage = s.edit(label, |doc, _| {
        let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
        let mut damage = Some(photocraft_geom::Rect::EMPTY);
        for id in &ids {
            let Some(LayerContent::Text(t)) = doc.layer_mut(*id).map(|l| &mut l.content) else { return Err(EngineError::NoLayer(*id)) };
            let before = t.cache.as_ref().map(|c| c.tile_bounds());
            let range = if p.get("layers").is_none() || p.get("layer").is_some() { range_param(&t.text, p) } else { (0, t.text.len()) };
            f(t, range)?;
            refresh_at(dpi, format, t);
            // An unknown old/new cache retains Session.edit's full-refresh fallback.
            damage = damage.zip(before).zip(t.cache.as_ref().map(|c| c.tile_bounds())).map(|((all, a), b)| all.union(&a).union(&b));
        }
        Ok(damage)
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = damage;
    }
    Ok(ids)
}

/// UI size/leading are measured after the layer transform; command callers otherwise use points.
fn shown_style_params<'a>(t: &TextLayer, p: &'a Value) -> Result<std::borrow::Cow<'a, Value>> {
    if p.get("metricsAsShown").and_then(Value::as_bool) != Some(true) {
        return Ok(std::borrow::Cow::Borrowed(p));
    }
    let m = t.transform.m;
    let scale = (m[0] * m[3] - m[1] * m[2]).abs().sqrt() as f32;
    let scale = if scale.is_finite() && scale > 1e-3 { f64::from(scale) } else { 1.0 };
    let mut params = p.clone();
    for key in ["size", "leading", "leadingBy"] {
        if let Some(value) = p.get(key).and_then(Value::as_f64) {
            let value = value / scale;
            if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
                return Err(bad("type.setStyle", "shown size and leading must convert to finite point values"));
            }
            params[key] = json!(value);
        }
    }
    Ok(std::borrow::Cow::Owned(params))
}

/// Whether a type layer still has the name it got from its text, so the name follows edits to
/// the text. A layer renamed to anything else keeps its name (#483).
pub(crate) fn is_auto_named(l: &Layer) -> bool {
    matches!(&l.content, LayerContent::Text(t) if l.name == layer_name(&t.text))
}

/// The name a type layer gets from its text: its first non-empty line, at most 40 characters.
/// The Type tool names new layers with this too, so `is_auto_named` recognises them.
pub fn layer_name(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let name: String = first.chars().take(40).collect();
    if name.is_empty() { "Type Layer".into() } else { name }
}

fn info(s: &Session, p: &Value) -> Result<Value> {
    let id = layer_id(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Text(t) = &l.content else {
        return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
    };
    let text = &t.text;
    let mut at = 0;
    let runs: Vec<Value> = t
        .char_runs()
        .into_iter()
        .map(|r| {
            let v = json!({ "start": char_at(text, at), "end": char_at(text, at + r.len), "style": r.style });
            at += r.len;
            v
        })
        .collect();
    let mut at = 0;
    let paragraphs: Vec<Value> = t
        .paragraph_runs()
        .into_iter()
        .map(|r| {
            let v = json!({ "start": char_at(text, at), "end": char_at(text, at + r.len), "style": r.style });
            at += r.len;
            v
        })
        .collect();
    let layout = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).layout(t, d.doc.resolution_dpi);
    let lines: Vec<Value> = layout
        .lines
        .iter()
        .map(|ln| json!({ "start": char_at(text, ln.range.start), "end": char_at(text, ln.range.end), "baseline": ln.baseline, "x0": ln.x0, "x1": ln.x1, "ascent": ln.ascent, "descent": ln.descent }))
        .collect();
    let bounds = t.cache.as_ref().map(|c| c.content_bounds()).map(|r| json!([r.x0, r.y0, r.x1, r.y1]));
    let warnings = crate::type_extra_cmds::missing_font_warnings_for_layer(t);
    let mut out = json!({
        "layer": id.0,
        "text": text,
        "shape": t.shape,
        "transform": t.transform.m,
        "antialias": t.antialias,
        "orientation": t.orientation,
        "warp": t.warp,
        "runs": runs,
        "paragraphs": paragraphs,
        "lines": lines,
        "bounds": bounds,
        "pxPerPt": layout.px_per_pt,
    });
    if !warnings.is_empty() {
        out["warnings"] = json!(warnings);
    }
    Ok(out)
}

/// A type layer (not yet rendered or added) from `type.create`'s params: text, point `x`/`y` or
/// paragraph `box`, `orientation`, character and paragraph keys, starting from the default type
/// styles in the foreground colour. Shared with the Type Mask tools' `type.createSelection`.
pub(crate) fn new_text_layer(s: &Session, p: &Value, cmd: &str) -> Result<TextLayer> {
    crate::commands::check_f32_values(cmd, p)?;
    let orientation = match p.get("orientation") {
        None => Orientation::Horizontal,
        Some(Value::String(value)) if value == "horizontal" => Orientation::Horizontal,
        Some(Value::String(value)) if value == "vertical" => Orientation::Vertical,
        _ => return Err(bad(cmd, "orientation must be horizontal or vertical")),
    };
    check_kerning(p).map_err(|m| bad(cmd, m))?;
    check_size_tracking(p).map_err(|m| bad(cmd, m))?;
    let text = norm_text(p.get("text").and_then(Value::as_str).unwrap_or(""));
    // Type › Save Default Type Styles sets the starting styles; the colour is always
    // the foreground colour, as in Photoshop.
    let (mut style, mut para) = s
        .type_defaults
        .clone()
        .unwrap_or_else(|| (CharStyle { font_family: photocraft_text::fonts::DEFAULT_FAMILY.into(), ..Default::default() }, ParagraphStyle::default()));
    let fg = s.tools.foreground;
    style.color = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    apply_char_props(&mut style, p);
    apply_para_props(&mut para, p);
    let (shape, transform) = match p.get("box").and_then(Value::as_array) {
        Some(b) if b.len() == 4 => {
            let v: Vec<f32> = b.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            if v[2] <= 0.0 || v[3] <= 0.0 {
                return Err(bad(cmd, "box width and height must be positive"));
            }
            (TextShape::Box { x: 0.0, y: 0.0, width: v[2], height: v[3] }, Affine::translate(f64::from(v[0]), f64::from(v[1])))
        }
        _ => (TextShape::Point, Affine::translate(f32p(p, "x").unwrap_or(0.0).into(), f32p(p, "y").unwrap_or(0.0).into())),
    };
    let mut t = TextLayer {
        runs: vec![TextRun { len: text.len(), style }],
        paragraphs: vec![ParagraphRun { len: text.len(), style: para }],
        text,
        shape,
        transform,
        orientation,
        ..Default::default()
    };
    t.sync_summary();
    Ok(t)
}

const CHAR_PARAMS: &str = r##""font":str,"fontStyle":str,"weight":100..900,"italic":bool,"size":0.1..=1296 pt,"color":"#rrggbb"|[r,g,b,a],"tracking":-1000..=10000 (1/1000 em),"leading":pt|"auto","baselineShift":pt,"horizontalScale":%,"verticalScale":%,"underline":bool,"strikethrough":bool,"fauxBold":bool,"fauxItalic":bool,"kerning":1/1000em (manual, after each character)|"metrics"|"optical"|"off","caps":"normal|small|all","baseline":"normal|superscript|subscript","ligatures":bool,"discretionaryLigatures":bool,"features":{"ss01":1},"variations":{"wght":650},"language":str"##;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "type.create",
            label: "New Type Layer",
            menu: &["Layer", "New"],
            shortcut: None,
            params: r##"{"x":px,"y":px (baseline anchor of point text),"text":str,"box":[x,y,w,h]? (paragraph text),"name":str?,"align":"left|center|right|justify…"?,"orientation":"horizontal|vertical"="horizontal", …character keys: "font","size":pt=12,"color",…}"##,
            enabled: has_doc,
            journal: true,
            run: |s, p| {
                let mut t = new_text_layer(s, p, "type.create")?;
                let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| layer_name(&t.text));
                let id = s.edit("New Type Layer", |doc, active| {
                    refresh(doc, &mut t);
                    let id = doc.insert_above(*active, Layer::new(name, LayerContent::Text(t)));
                    *active = Some(id);
                    Ok(id)
                })?;
                let bounds = s.active().and_then(|d| d.doc.layer(id)).and_then(|l| l.surface()).map(|c| c.content_bounds());
                let mut out = json!({ "layer": id.0, "bounds": bounds.map(|r| [r.x0, r.y0, r.x1, r.y1]) });
                if let Some(LayerContent::Text(t)) = s.active().and_then(|d| d.doc.layer(id)).map(|l| &l.content) {
                    let warnings = crate::type_extra_cmds::missing_font_warnings_for_layer(t);
                    if !warnings.is_empty() {
                        out["warnings"] = json!(warnings);
                    }
                }
                Ok(out)
            },
        },
        CommandSpec {
            id: "type.edit",
            label: "Edit Type",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?,"text":str? (replace all, styles kept),"replace":{"start":char,"end":char,"text":str}?,"runs":[{"start":char,"end":char,…character keys}]?,"box":[x,y,w,h]? (to paragraph text),"point":[x,y]? (to point text),"move":[dx,dy]?,"transform":[a,b,c,d,e,f]?,"antialias":"none|sharp|crisp|strong|smooth"?,"name":str?,"kerning":1/1000em|"metrics"|"optical"|"off"? (with "range":[startChar,endChar]?, default all),"kernPair":{"at":caretChar,"by":1/1000em}? (Alt+←/→: the pair before the caret becomes manual, its current kerning + by)}"##,
            enabled: has_doc,
            journal: true,
            run: |s, p| {
                let id = layer_id(s, p)?;
                let name = p.get("name").and_then(Value::as_str).map(str::to_string);
                check_kerning(p).map_err(|m| bad("type.edit", m))?;
                check_size_tracking(p).map_err(|m| bad("type.edit", m))?;
                if let Some(Value::Array(runs)) = p.get("runs") {
                    for (i, r) in runs.iter().enumerate() {
                        check_kerning(r).map_err(|m| bad("type.edit", m))?;
                        // Checked before anything changes, so a bad run leaves the layer untouched (#714).
                        let start = r.get("start").and_then(Value::as_u64).unwrap_or(0);
                        if let Some(end) = r.get("end").and_then(Value::as_u64)
                            && end < start
                        {
                            return Err(bad("type.edit", format!("runs[{i}]: `end` ({end}) is before `start` ({start})")));
                        }
                        check_size_tracking(r).map_err(|m| bad("type.edit", m))?;
                    }
                }
                let kern_pair = match p.get("kernPair") {
                    None => None,
                    Some(k) => {
                        let at = k.get("at").and_then(Value::as_u64).ok_or_else(|| bad("type.edit", "kernPair.at must be a caret position (character index)"))?;
                        let by = k
                            .get("by")
                            .and_then(Value::as_f64)
                            .filter(|v| v.is_finite() && v.abs() <= KERN_MAX)
                            .ok_or_else(|| bad("type.edit", "kernPair.by must be a number of 1/1000 em"))?;
                        Some((at as usize, by))
                    }
                };
                let label = if kern_pair.is_some() { "Kerning" } else { "Edit Type" };
                with_text_layer(s, p, label, |t, dpi, rename| {
                    if let Some((at, by)) = kern_pair {
                        // Photoshop's Alt+←/→: the pair before the caret becomes manually kerned,
                        // starting from what it shows now (its metrics/optical or manual value).
                        let n = t.text.chars().count();
                        if at == 0 || at >= n {
                            return Err(bad("type.edit", "kernPair needs a caret between two characters"));
                        }
                        let a = byte_at(&t.text, at - 1);
                        let b = byte_at(&t.text, at);
                        let now = {
                            let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
                            eng.pair_kerning(t, dpi, a)
                        }
                        .ok_or_else(|| bad("type.edit", "no kerning pair at the caret (line break or line end)"))?;
                        let value = (f64::from(now.round()) + by).clamp(KERN_MIN, KERN_MAX) as f32;
                        style_range(t, a, b, &|st| {
                            st.kerning = Kerning::Off;
                            st.kern = value;
                        });
                    }
                    if p.get("kerning").is_some() {
                        let (a, b) = range_param(&t.text, p);
                        style_range(t, a, b, &|st| {
                            apply_char_props(st, &json!({ "kerning": p.get("kerning") }));
                        });
                    }
                    if let Some(v) = p.get("text").and_then(Value::as_str) {
                        // Replace only what changed, so the unchanged text keeps its character and
                        // paragraph styles (#8489) instead of the whole text taking the first one.
                        let v = norm_text(v);
                        let (a, b, new) = changed_span(&t.text, &v);
                        replace_text(t, a, b, new);
                    }
                    if let Some(r) = p.get("replace") {
                        let a = byte_at(&t.text, r.get("start").and_then(Value::as_u64).unwrap_or(0) as usize);
                        let b = byte_at(&t.text, r.get("end").and_then(Value::as_u64).map_or(usize::MAX, |x| x as usize));
                        let new = r.get("text").and_then(Value::as_str).unwrap_or("");
                        replace_text(t, a, b.max(a), new);
                    }
                    if let Some(Value::Array(runs)) = p.get("runs") {
                        for r in runs {
                            let a = byte_at(&t.text, r.get("start").and_then(Value::as_u64).unwrap_or(0) as usize);
                            let b = byte_at(&t.text, r.get("end").and_then(Value::as_u64).map_or(usize::MAX, |x| x as usize));
                            style_range(t, a, b, &|st| {
                                apply_char_props(st, r);
                            });
                        }
                    }
                    let lin = t.transform.m;
                    if let Some(b) = p.get("box").and_then(Value::as_array).filter(|b| b.len() == 4) {
                        let v: Vec<f64> = b.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
                        t.shape = TextShape::Box { x: 0.0, y: 0.0, width: v[2].max(1.0) as f32, height: v[3].max(1.0) as f32 };
                        t.transform = Affine { m: [lin[0], lin[1], lin[2], lin[3], v[0], v[1]] };
                    }
                    if let Some(pt) = p.get("point").and_then(Value::as_array).filter(|b| b.len() == 2) {
                        t.shape = TextShape::Point;
                        t.transform = Affine { m: [lin[0], lin[1], lin[2], lin[3], pt[0].as_f64().unwrap_or(0.0), pt[1].as_f64().unwrap_or(0.0)] };
                    }
                    if let Some(d) = p.get("move").and_then(Value::as_array).filter(|b| b.len() == 2) {
                        t.transform.m[4] += d[0].as_f64().unwrap_or(0.0);
                        t.transform.m[5] += d[1].as_f64().unwrap_or(0.0);
                    }
                    if let Some(m) = p.get("transform").and_then(Value::as_array).filter(|b| b.len() == 6) {
                        for (i, v) in m.iter().enumerate() {
                            t.transform.m[i] = v.as_f64().unwrap_or(t.transform.m[i]);
                        }
                    }
                    if let Some(a) = p.get("antialias").and_then(Value::as_str) {
                        t.antialias = match a {
                            "none" => AntiAlias::None,
                            "sharp" => AntiAlias::Sharp,
                            "crisp" => AntiAlias::Crisp,
                            "strong" => AntiAlias::Strong,
                            _ => AntiAlias::Smooth,
                        };
                    }
                    // The rename lands in the same undo step as the edit (#497).
                    *rename = name.clone();
                    Ok(())
                })?;
                info(s, &json!({ "layer": id.0 }))
            },
        },
        CommandSpec {
            id: "type.setStyle",
            label: "Set Type Style",
            menu: &[],
            shortcut: None,
            params: Box::leak(
                format!(r##"{{"layer":id? or "layers":[id,…]? (default selected Type layers),"range":[startChar,endChar]? (single layer; default all),"metricsAsShown":bool=false (size/leading/leadingBy include each layer's transform scale), {CHAR_PARAMS}, relative steps from each character's own value: "leadingBy":pt (auto leading starts from its laid-out value; result 0.01..=5000),"baselineShiftBy":pt (result -5000..=5000),"trackingBy":1/1000 em (result -1000..=10000), paragraph keys: "align":"left|center|right|justify|justifyCenter|justifyRight|justifyAll","firstLineIndent":pt,"startIndent":pt,"endIndent":pt,"spaceBefore":pt,"spaceAfter":pt,"autoLeading":%,"direction":"auto|ltr|rtl","hyphenate":bool}}"##)
                    .into_boxed_str(),
            ),
            enabled: has_doc,
            journal: true,
            run: |s, p| {
                check_kerning(p).map_err(|m| bad("type.setStyle", m))?;
                check_steps(p).map_err(|m| bad("type.setStyle", m))?;
                let ids = with_text_targets(s, p, "Set Type Style", |t, (a, b)| {
                    let props = shown_style_params(t, p)?;
                    check_size_tracking(&props).map_err(|m| bad("type.setStyle", m))?;
                    let mut probe = CharStyle::default();
                    if apply_char_props(&mut probe, &props) {
                        style_range(t, a, b, &|st| {
                            apply_char_props(st, &props);
                        });
                    }
                    step_char_metrics(t, a, b, &props);
                    let mut pprobe = ParagraphStyle::default();
                    if apply_para_props(&mut pprobe, p) {
                        style_paragraphs(t, a, b, &|st| {
                            apply_para_props(st, p);
                        });
                    }
                    Ok(())
                })?;
                let id = ids.first().ok_or_else(|| EngineError::Other("select a type layer".into()))?;
                info(s, &json!({ "layer": id.0 }))
            },
        },
        CommandSpec {
            id: "type.rasterize",
            label: "Rasterize Type",
            menu: &["Layer", "Rasterize"],
            shortcut: None,
            params: r##"{"layer":id?}"##,
            enabled: has_doc,
            journal: true,
            run: |s, p| {
                let id = layer_id(s, p)?;
                s.edit("Rasterize Type", |doc, _| {
                    let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                    let LayerContent::Text(t) = &mut l.content else {
                        return Err(EngineError::Other(format!("layer {} is not a type layer", id.0)));
                    };
                    if t.cache.is_none() {
                        refresh_at(dpi, format, t);
                    }
                    let surface = t.cache.take().unwrap_or_else(|| photocraft_raster::Surface::new(format));
                    let surface = if surface.format() == format { surface } else { surface.convert(format) };
                    l.content = LayerContent::Raster(surface);
                    l.psd_blocks.retain(|(k, _)| k != b"TySh");
                    Ok(())
                })?;
                Ok(json!({ "layer": id.0 }))
            },
        },
        CommandSpec {
            id: "type.info",
            label: "Type Layer Info",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → text, runs/paragraphs (char offsets + styles), shape, transform, laid-out lines (text space px), bounds"##,
            enabled: has_doc,
            journal: false,
            run: |s, p| info(s, p),
        },
        CommandSpec {
            id: "type.fonts",
            label: "List Fonts",
            menu: &[],
            shortcut: None,
            params: r##"{"family":str? (faces of one family)}"##,
            enabled: |_| Ok(()),
            journal: false,
            run: |_, p| {
                let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
                match p.get("family").and_then(Value::as_str) {
                    Some(f) => {
                        let faces: Vec<Value> = eng.fonts.faces(f).into_iter().map(|x| json!({ "family": x.family, "style": x.style, "postscriptName": x.postscript_name, "weight": x.weight, "italic": x.italic, "axes": x.axes })).collect();
                        Ok(json!({ "faces": faces }))
                    }
                    None => Ok(json!({ "families": eng.fonts.families() })),
                }
            },
        },
        size_key("type.increaseSize", "Increase Type Size", "Cmd+Shift+.", |s, p| step_size(s, p, true, 2.0)),
        size_key("type.decreaseSize", "Decrease Type Size", "Cmd+Shift+,", |s, p| step_size(s, p, false, 2.0)),
        size_key("type.increaseSizeMore", "Increase Type Size by 10", "Cmd+Alt+Shift+.", |s, p| step_size(s, p, true, 10.0)),
        size_key("type.decreaseSizeMore", "Decrease Type Size by 10", "Cmd+Alt+Shift+,", |s, p| step_size(s, p, false, 10.0)),
    ]
}

fn has_type_layer(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) if matches!(l.content, LayerContent::Text(_)) => Ok(()),
        _ => Err("the active layer is not a type layer".into()),
    }
}

/// Photoshop's ⇧⌘> / ⇧⌘< (⌥ for bigger steps). On a US keyboard > and < share the . and , keys.
fn size_key(id: &'static str, label: &'static str, shortcut: &'static str, run: fn(&mut Session, &Value) -> Result<Value>) -> CommandSpec {
    CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: Some(shortcut),
        params: r##"{"layer":id?,"range":[startChar,endChar]? (default all),"by":pt? (as the layer appears)}"##,
        enabled: has_type_layer,
        journal: true,
        run,
    }
}

/// Grows or shrinks every character in the range by `by` points as the layer appears, so mixed
/// sizes keep their differences and a scaled layer steps in the points its size field shows.
fn step_size(s: &mut Session, p: &Value, up: bool, default_by: f32) -> Result<Value> {
    let id = layer_id(s, p)?;
    let by = f32p(p, "by").filter(|v| v.is_finite()).unwrap_or(default_by).abs().min(TYPE_SIZE_MAX_PT as f32);
    with_text_layer(s, p, "Change Type Size", |t, _, _| {
        let m = t.transform.m;
        let scale = (m[0] * m[3] - m[1] * m[2]).abs().sqrt() as f32;
        let scale = if scale.is_finite() && scale > 1e-3 { scale } else { 1.0 };
        let delta = if up { by } else { -by } / scale;
        let (a, b) = range_param(&t.text, p);
        style_range(t, a, b, &|st| st.size_pt = (st.size_pt + delta).clamp(TYPE_SIZE_MIN_PT as f32, TYPE_SIZE_MAX_PT as f32));
        Ok(())
    })?;
    info(s, &json!({ "layer": id.0 }))
}

#[cfg(test)]
mod tests;
