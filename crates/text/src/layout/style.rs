use super::paragraphs::quote;
use super::*;

/// Photoshop synthesizes small caps for faces without an OpenType `smcp` table. Keep the same
/// readable hierarchy for every font instead of silently rendering lowercase text unchanged.
pub(super) const SYNTHETIC_SMALL_CAPS_SCALE: f32 = 0.7;

/// Photoshop's default superscript / subscript size and position (Type preferences, stored in a
/// PSD's text resources as `SuperscriptSize` .583, `SuperscriptPosition` .333 and their
/// `Subscript` twins): the run is drawn at 58.3% of its size and moved by 33.3% of its size.
const SCRIPT_SIZE: f32 = 0.583;

const SCRIPT_POSITION: f32 = 0.333;

/// The style a run is drawn with: superscript and subscript shrink it and shift its baseline.
pub(super) fn drawn_style(s: &mut CharStyle) {
    let dir = match s.baseline {
        FontBaseline::Normal => return,
        FontBaseline::Superscript => 1.0,
        FontBaseline::Subscript => -1.0,
    };
    s.baseline_shift_pt += dir * SCRIPT_POSITION * s.size_pt;
    s.size_pt *= SCRIPT_SIZE;
}

/// The rendering path for a `SmallCaps` character style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SmallCapsMode {
    None,
    OpenType,
    Synthetic,
}

/// Whether the style's selected face has an `smcp` feature (a font load and GSUB parse).
pub(super) fn probe_smcp(fonts: &mut FontDb, style: &CharStyle) -> bool {
    #[cfg(test)]
    SMCP_PROBES.with(|c| c.set(c.get() + 1));
    fonts.selected_face_has_feature(&style.font_family, style.weight, style.italic, Tag::new(b"smcp"))
}

#[cfg(test)]
thread_local! {
    pub(super) static SMCP_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn small_caps_mode(caps: Caps, face_has_smcp: bool) -> SmallCapsMode {
    match caps {
        Caps::SmallCaps if face_has_smcp => SmallCapsMode::OpenType,
        Caps::SmallCaps => SmallCapsMode::Synthetic,
        _ => SmallCapsMode::None,
    }
}

/// A style's horizontal scale; unset (zero or negative) is 100%.
pub(super) fn effective_horizontal_scale(st: &CharStyle) -> f32 {
    if st.horizontal_scale > 0.0 && st.horizontal_scale.is_finite() { st.horizontal_scale } else { 1.0 }
}

/// OpenType feature settings of a character style (CSS `font-feature-settings` items).
pub(super) fn feature_list(st: &CharStyle, small_caps: SmallCapsMode) -> Vec<String> {
    let mut feats: Vec<String> = Vec::new();
    // Optical and manual kerning replace the font's kerning table; a character with a manual
    // kern is manually kerned whatever its mode (as in Photoshop).
    if st.kerning != Kerning::Metrics || st.kern != 0.0 {
        feats.push("\"kern\" 0".into());
    }
    if !st.ligatures {
        feats.push("\"liga\" 0".into());
        feats.push("\"clig\" 0".into());
    }
    if st.discretionary_ligatures {
        feats.push("\"dlig\" 1".into());
    }
    if small_caps == SmallCapsMode::OpenType {
        feats.push("\"smcp\" 1".into());
    }
    for f in &st.features {
        if f.tag.len() == 4 && f.tag.is_ascii() {
            feats.push(format!("\"{}\" {}", f.tag, f.value));
        }
    }
    feats
}

/// The font variations for a run: the explicit axis values, plus `opsz` at the type size when the
/// style doesn't set it (as CSS `font-optical-sizing: auto`). A font without an `opsz` axis
/// ignores it.
pub(super) fn variation_list(st: &CharStyle) -> Vec<String> {
    let mut vars: Vec<String> =
        st.variations.iter().filter(|v| v.axis.len() == 4 && v.axis.is_ascii() && v.value.is_finite()).map(|v| format!("\"{}\" {}", v.axis, v.value)).collect();
    if st.size_pt.is_finite() && !st.variations.iter().any(|v| v.axis == "opsz") {
        vars.push(format!("\"opsz\" {}", st.size_pt.max(0.01)));
    }
    vars
}

pub(super) fn style_props(st: &CharStyle, k: f32, fallback: &[String], idx: u32, small_caps: SmallCapsMode) -> Vec<StyleProperty<'static, RunBrush>> {
    let px = (st.size_pt * k).max(0.01);
    let mut fam: Vec<String> = Vec::new();
    if !st.font_family.is_empty() {
        fam.push(quote(&st.font_family));
    }
    // Serif runs fall back to a Mincho face for Japanese (craft-fonts), others to a Gothic one.
    if crate::craft_fonts::is_serif_family(&st.font_family) {
        fam.extend(crate::craft_fonts::mincho_first(fallback).iter().map(|f| quote(f)));
    } else {
        fam.extend(fallback.iter().map(|f| quote(f)));
    }
    fam.push("sans-serif".into());
    let feats = feature_list(st, small_caps);
    let vars = variation_list(st);
    vec![
        StyleProperty::FontFamily(FontFamily::Source(Cow::Owned(fam.join(", ")))),
        StyleProperty::FontSize(px),
        StyleProperty::FontWeight(FontWeight::new(st.weight.clamp(1, 1000) as f32)),
        StyleProperty::FontStyle(if st.italic { FontStyle::Italic } else { FontStyle::Normal }),
        StyleProperty::FontFeatures(FontFeatures::Source(Cow::Owned(feats.join(", ")))),
        StyleProperty::FontVariations(FontVariations::Source(Cow::Owned(vars.join(", ")))),
        StyleProperty::LetterSpacing(st.tracking / 1000.0 * px),
        StyleProperty::Underline(st.underline),
        StyleProperty::Strikethrough(st.strikethrough),
        StyleProperty::Brush(RunBrush(idx, 0)),
        StyleProperty::Locale(st.language.as_deref().and_then(|l| parley::fontique::Language::parse(l).ok())),
    ]
}

/// Closing CJK punctuation: its full-width cell has blank space after the mark.
fn is_closing(c: char) -> bool {
    matches!(c, '、' | '。' | '，' | '．' | '」' | '』' | '）' | '〕' | '】' | '〉' | '》' | '〙' | '〗' | '］' | '｝')
}

/// Opening CJK brackets: blank space before the mark.
fn is_opening(c: char) -> bool {
    matches!(c, '「' | '『' | '（' | '〔' | '【' | '〈' | '《' | '〘' | '〖' | '［' | '｛')
}

/// Basic Japanese punctuation squeeze (JIS X 4051 style) for one line in line space: between a
/// closing mark and a following opening or closing mark, the closing mark's trailing half-em
/// blank is removed; between two opening brackets, the second one's leading half-em. Clusters and
/// glyphs from the squeezed point on move back. Returns the total length removed.
pub(crate) fn squeeze_punctuation(text: &str, clusters: &mut [ClusterInfo], glyphs: &mut [PlacedGlyph]) -> f32 {
    let first = |c: &ClusterInfo| text.get(c.range.clone()).and_then(|s| s.chars().next());
    let last = |c: &ClusterInfo| text.get(c.range.clone()).and_then(|s| s.chars().next_back());
    // Shift to apply from each cluster on.
    let mut shifts = vec![0.0f32; clusters.len()];
    let mut total = 0.0f32;
    for i in 1..clusters.len() {
        let (Some(prev), Some(cur)) = (clusters.get(i - 1), clusters.get(i)) else { continue };
        if prev.rtl || cur.rtl {
            continue;
        }
        let (Some(a), Some(b)) = (last(prev), first(cur)) else { continue };
        let cut = if is_closing(a) && (is_opening(b) || is_closing(b)) {
            prev.advance * 0.5
        } else if is_opening(a) && is_opening(b) {
            cur.advance * 0.5
        } else {
            0.0
        };
        if cut.is_finite() && cut > 0.0 {
            total += cut;
        }
        if let Some(s) = shifts.get_mut(i) {
            *s = total;
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    let starts: Vec<f32> = clusters.iter().map(|c| c.x).collect();
    for (c, s) in clusters.iter_mut().zip(&shifts) {
        c.x -= s;
    }
    for g in glyphs.iter_mut() {
        let i = starts.iter().rposition(|&s| s <= g.x + 1e-3).unwrap_or(0);
        g.x -= shifts.get(i).copied().unwrap_or(0.0);
    }
    total
}

/// Whether to embolden a face synthetically. Font matching (fontique) asks for it whenever the
/// requested weight is above the face's, so Regular (400) from a family with only W3 (300) and W6
/// (600), like Hiragino Mincho ProN, came out as a smeared bold W3. Like Photoshop and CSS, only
/// a bold request (≥ 600) on a face that isn't bold (≤ 500) is emboldened; anything else uses the
/// nearest face as it is (Faux Bold stays a separate, explicit style).
pub(crate) fn synthetic_bold(matcher_says: bool, requested: u16, face_weight: f32) -> bool {
    matcher_says && requested >= 600 && face_weight <= 500.0
}

/// Height of the lowercase ascender ('d') of the tallest run on the line.
pub(super) fn first_ascent(line: &parley::Line<'_, RunBrush>) -> Option<f32> {
    use skrifa::MetadataProvider;
    let mut best: Option<f32> = None;
    for run in line.runs() {
        let fd = run.font();
        let Ok(font) = skrifa::FontRef::from_index(fd.data.as_ref(), fd.index) else {
            continue;
        };
        let coords: Vec<skrifa::instance::NormalizedCoord> = run.normalized_coords().iter().map(|&c| skrifa::instance::NormalizedCoord::from_bits(c)).collect();
        let loc = skrifa::instance::LocationRef::new(&coords);
        let size = skrifa::instance::Size::new(run.font_size());
        let h = font.charmap().map('d').and_then(|g| font.glyph_metrics(size, loc).bounds(g)).map(|b| b.y_max).or_else(|| {
            let m = font.metrics(size, loc);
            m.cap_height.or(Some(m.ascent * 0.75))
        });
        if let Some(h) = h {
            best = Some(best.map_or(h, |b: f32| b.max(h)));
        }
    }
    best
}

/// Hangul (syllables and jamo).
fn is_hangul(c: char) -> bool {
    matches!(c, '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7AF}' | '\u{D7B0}'..='\u{D7FF}')
}

/// The byte ranges of `text` made of Hangul, each as long as it runs.
pub(crate) fn hangul_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, c) in text.char_indices().filter(|&(_, c)| is_hangul(c)) {
        match runs.last_mut() {
            Some(r) if r.end == i => r.end = i + c.len_utf8(),
            _ => runs.push(i..i + c.len_utf8()),
        }
    }
    runs
}
